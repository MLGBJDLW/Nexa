//! ACP terminals have a session lifetime and bounded output. Empty argv carries
//! a shell command (as used by Goose); nonempty argv preserves literal arguments.
//! Waiting for one terminal must not block other JSON-RPC requests or Stop.
use super::{error, Result};
use nexa_core::agent::CancellationToken;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::watch,
    task::JoinHandle,
};

const MAX_TERMINALS: usize = 32;
const MAX_OUTPUT: usize = 1024 * 1024;

#[derive(Default)]
pub(super) struct Terminals {
    entries: Mutex<HashMap<String, Arc<Terminal>>>,
    released: Mutex<Option<nexa_core::runtime_receipts::RuntimeReceipts>>,
}
struct Terminal {
    output: Arc<Mutex<Output>>,
    exit: watch::Receiver<Option<Value>>,
    kill: CancellationToken,
    task: JoinHandle<()>,
}
impl Drop for Terminal {
    fn drop(&mut self) {
        self.kill.cancel();
        self.task.abort();
    }
}
struct Output {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}
impl Output {
    fn push(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
        let overflow = self.bytes.len().saturating_sub(self.limit);
        if overflow > 0 {
            self.bytes.drain(..overflow);
            while self.bytes.first().is_some_and(|byte| byte & 0xc0 == 0x80) {
                self.bytes.remove(0);
            }
            self.truncated = true;
        }
    }
}
async fn capture(mut pipe: impl AsyncRead + Unpin, output: Arc<Mutex<Output>>) {
    let mut bytes = [0; 4096];
    loop {
        match pipe.read(&mut bytes).await {
            Ok(0) | Err(_) => break,
            Ok(count) => output
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(&bytes[..count]),
        }
    }
}

impl Terminals {
    fn terminal_snapshot(terminal: &Terminal) -> Value {
        let output = terminal.output.lock().unwrap_or_else(|e| e.into_inner());
        let mut result =
            json!({"output":String::from_utf8_lossy(&output.bytes),"truncated":output.truncated});
        if let Some(status) = terminal.exit.borrow().clone() {
            result["exitStatus"] = status;
        }
        result
    }
    /// UI receipts outlive release; protocol requests still reject released IDs.
    pub(super) fn snapshot(&self, id: &str) -> Result<Option<Value>> {
        if let Some(terminal) = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
        {
            return Ok(Some(Self::terminal_snapshot(terminal)));
        }
        let released = self.released.lock().unwrap_or_else(|e| e.into_inner());
        released
            .as_ref()
            .map(|receipts| {
                receipts
                    .get(id)
                    .map(|record| record.map(|(_, result)| result))
            })
            .transpose()
            .map(Option::flatten)
    }
    pub(super) fn create(
        &self,
        params: &Value,
        cwd: &Path,
        provider: Option<&str>,
    ) -> Result<Value> {
        let command = params["command"]
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 32768)
            .ok_or_else(|| error("ACP terminal command is required"))?;
        let args = if let Some(args) = params.get("args") {
            args.as_array()
                .filter(|args| args.len() <= 1024)
                .ok_or_else(|| error("Invalid ACP terminal arguments"))?
                .iter()
                .map(|arg| {
                    arg.as_str()
                        .ok_or_else(|| error("ACP terminal arguments must be strings"))
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            vec![]
        };
        let mut process = if args.is_empty() {
            let shell = command_shell(provider)?;
            let (program, args) = shell.invocation(Some(command), cwd).map_err(error)?;
            let mut process = tokio::process::Command::new(program);
            process.args(args);
            process
        } else {
            let mut process = tokio::process::Command::new(command);
            process.args(args);
            process
        };
        if let Some(env) = params.get("env") {
            let env = env
                .as_array()
                .filter(|env| env.len() <= 128)
                .ok_or_else(|| error("Invalid ACP terminal environment"))?;
            for item in env {
                let name = item["name"]
                    .as_str()
                    .filter(|name| !name.is_empty() && !name.contains(['=', '\0']))
                    .ok_or_else(|| error("Invalid ACP environment variable name"))?;
                let value = item["value"]
                    .as_str()
                    .filter(|value| value.len() <= 65536)
                    .ok_or_else(|| error("Invalid ACP environment variable value"))?;
                process.env(name, value);
            }
        }
        let limit = match params.get("outputByteLimit") {
            Some(value) => value
                .as_u64()
                .ok_or_else(|| error("Invalid ACP output byte limit"))?
                .min(MAX_OUTPUT as u64) as usize,
            None => MAX_OUTPUT,
        };
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if entries.len() >= MAX_TERMINALS {
            return Err(error(
                "ACP terminal limit reached; release an existing terminal",
            ));
        }
        process
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (mut child, tree) = nexa_core::managed_process::spawn(&mut process).map_err(error)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| error("ACP terminal stdout unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| error("ACP terminal stderr unavailable"))?;
        let output = Arc::new(Mutex::new(Output {
            bytes: Vec::new(),
            limit,
            truncated: false,
        }));
        let (exit_tx, exit) = watch::channel(None);
        let kill = CancellationToken::new();
        let kill_task = kill.clone();
        let capture_output = output.clone();
        let task = tokio::spawn(async move {
            // Reader futures stay in this task so abort drops the pipes, child
            // and process tree together. No orphaned output workers survive it.
            let readers = async {
                tokio::join!(
                    capture(stdout, capture_output.clone()),
                    capture(stderr, capture_output)
                );
            };
            let process = async {
                let status = tokio::select! {
                    status = child.wait() => status.ok(),
                    _ = kill_task.cancelled() => { let _ = child.kill().await; child.wait().await.ok() },
                };
                drop(tree);
                status.map(|status| {
                    #[cfg(unix)]
                    { use std::os::unix::process::ExitStatusExt; json!({"exitCode":status.code(),"signal":status.signal().map(|value| value.to_string())}) }
                    #[cfg(not(unix))]
                    { json!({"exitCode":status.code()}) }
                }).unwrap_or_else(|| json!({"signal":"terminated"}))
            };
            let (status, ()) = tokio::join!(process, readers);
            let _ = exit_tx.send(Some(status));
        });
        let id = uuid::Uuid::new_v4().to_string();
        entries.insert(
            id.clone(),
            Arc::new(Terminal {
                output,
                exit,
                kill,
                task,
            }),
        );
        Ok(json!({"terminalId":id}))
    }

    pub(super) async fn request(&self, method: &str, id: &str) -> Result<Value> {
        let terminal = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| error("Unknown or released ACP terminal"))?;
        match method {
            "terminal/output" => Ok(Self::terminal_snapshot(&terminal)),
            "terminal/wait_for_exit" => {
                let mut exit = terminal.exit.clone();
                loop {
                    if let Some(status) = exit.borrow_and_update().clone() {
                        return Ok(status);
                    }
                    exit.changed()
                        .await
                        .map_err(|_| error("ACP terminal was released before exit"))?;
                }
            }
            "terminal/kill" | "terminal/release" => {
                terminal.kill.cancel();
                let mut exit = terminal.exit.clone();
                if exit.borrow().is_none() {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(3),
                        exit.wait_for(Option::is_some),
                    )
                    .await;
                }
                if method == "terminal/release" {
                    let mut released = self.released.lock().unwrap_or_else(|e| e.into_inner());
                    if released.is_none() {
                        *released = Some(nexa_core::runtime_receipts::RuntimeReceipts::new()?);
                    }
                    released.as_ref().expect("initialized receipts").insert(
                        id,
                        "",
                        &Self::terminal_snapshot(&terminal),
                    )?;
                    self.entries
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(id);
                }
                Ok(json!({}))
            }
            _ => Err(error("Unknown ACP terminal method")),
        }
    }
}

fn command_shell(provider: Option<&str>) -> Result<nexa_core::shell_environment::ShellProfile> {
    if provider != Some("goose") {
        return nexa_core::shell_environment::resolve_profile("default").map_err(error);
    }
    // Goose's tool description tells the model this exact shell dialect. Match
    // its GOOSE_SHELL/default selection rather than using the desktop preference.
    let program = std::env::var("GOOSE_SHELL").unwrap_or_else(|_| {
        if cfg!(windows) {
            "cmd".into()
        } else if nexa_core::shell_environment::resolve_profile("bash").is_ok() {
            "bash".into()
        } else {
            "sh".into()
        }
    });
    let basename = Path::new(&program)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kind = if cfg!(windows) && matches!(basename.as_str(), "cmd" | "powershell" | "pwsh") {
        basename
    } else {
        "sh".into()
    };
    Ok(nexa_core::shell_environment::ShellProfile {
        id: "goose-native".into(),
        label: "Goose native shell".into(),
        program,
        kind,
        distribution: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[tokio::test]
    async fn goose_uses_its_advertised_windows_shell_dialect() {
        let directory = tempfile::tempdir().unwrap();
        let terminals = Terminals::default();
        let shell = command_shell(Some("goose")).unwrap();
        let command = if shell.kind == "cmd" {
            "echo %NEXA_ACP_TEST% & echo complete"
        } else if matches!(shell.kind.as_str(), "powershell" | "pwsh") {
            "Write-Output $env:NEXA_ACP_TEST; Write-Output complete"
        } else {
            "printf '%s complete' \"$NEXA_ACP_TEST\""
        };
        let created = terminals.create(&json!({"command":command,"env":[{"name":"NEXA_ACP_TEST","value":"goose-dialect"}]}), directory.path(), Some("goose")).unwrap();
        let id = created["terminalId"].as_str().unwrap();
        let status = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            terminals.request("terminal/wait_for_exit", id),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(status["exitCode"], 0);
        let output = terminals.request("terminal/output", id).await.unwrap();
        let output = output["output"].as_str().unwrap();
        assert!(
            output.contains("goose-dialect") && output.contains("complete"),
            "{output}"
        );
    }
    #[tokio::test]
    async fn native_shell_command_supports_spaces_pipelines_and_terminal_receipts() {
        let directory = tempfile::tempdir().unwrap();
        let terminals = Terminals::default();
        let command = if cfg!(windows) {
            "Write-Output 'native shell ✓' | ForEach-Object { $_ }"
        } else {
            "printf 'native shell ✓\\n' | cat"
        };
        let created = terminals
            .create(&json!({"command":command}), directory.path(), None)
            .unwrap();
        let id = created["terminalId"].as_str().unwrap();
        let status = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            terminals.request("terminal/wait_for_exit", id),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(status["exitCode"], 0);
        assert!(
            terminals.request("terminal/output", id).await.unwrap()["output"]
                .as_str()
                .unwrap()
                .contains("native shell")
        );
        terminals.request("terminal/release", id).await.unwrap();
        assert!(terminals.snapshot(id).unwrap().is_some());
    }
    #[test]
    fn retained_tail_respects_utf8_boundaries_and_zero_limit() {
        let mut output = Output {
            bytes: Vec::new(),
            limit: 4,
            truncated: false,
        };
        output.push("中文🙂".as_bytes());
        assert_eq!(std::str::from_utf8(&output.bytes).unwrap(), "🙂");
        assert!(output.truncated);
        output.limit = 0;
        output.push(b"x");
        assert!(output.bytes.is_empty());
    }
}
