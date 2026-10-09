//! ACP v1: one UTF-8 JSON-RPC 2.0 message per line on stdio.
use super::{error, Result};
use nexa_core::{
    external_agent::{ExternalAgentLaunch, ExternalAgentPreset},
    managed_process::ProcessTree,
};
use serde_json::{json, Value};
use std::{collections::VecDeque, path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::mpsc,
};

pub(super) const RPC_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_FRAME: usize = 8 * 1024 * 1024;

pub(super) struct Wire {
    child: Child,
    _tree: ProcessTree,
    stdin: ChildStdin,
    messages: mpsc::Receiver<Result<Value>>,
    reader: tokio::task::JoinHandle<()>,
    queued: VecDeque<Value>,
    next_id: u64,
}

impl Drop for Wire {
    fn drop(&mut self) {
        self.reader.abort();
        let _ = self.child.start_kill();
    }
}

fn executable(preset: &ExternalAgentPreset, launch: &ExternalAgentLaunch) -> Result<PathBuf> {
    if let Some(path) = launch.executable.as_deref().filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    // Windows npm shims have .cmd; Command quotes its fixed argv, not a shell
    // command assembled from user input. Neither cwd nor prompts become argv.
    let names = if cfg!(windows) {
        vec![
            format!("{}.exe", preset.command),
            format!("{}.cmd", preset.command),
            format!("{}.bat", preset.command),
        ]
    } else {
        vec![preset.command.clone()]
    };
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if !directory.is_absolute() {
            continue;
        }
        for name in &names {
            let path = directory.join(name);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    if preset.provider == "github_copilot_acp" {
        return crate::commands::subscription_accounts::resolve_copilot_binary().map_err(error);
    }
    Err(error(format!("{} is not installed on PATH. Install and sign in with its official CLI, or choose its executable in Settings.", preset.name)))
}

impl Wire {
    pub(super) fn take_pending(&mut self) -> Result<VecDeque<Value>> {
        let mut pending = std::mem::take(&mut self.queued);
        while let Ok(value) = self.messages.try_recv() {
            if pending.len() >= 256 {
                return Err(error("ACP queued event budget exceeded"));
            }
            pending.push_back(value?);
        }
        Ok(pending)
    }
    pub(super) fn requeue(&mut self, pending: VecDeque<Value>) {
        self.queued = pending;
    }

    pub(super) fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None)) && !self.messages.is_closed()
    }

    pub(super) fn start(
        preset: &ExternalAgentPreset,
        launch: &ExternalAgentLaunch,
    ) -> Result<Self> {
        launch.validate_resolved()?;
        let mut command = tokio::process::Command::new(executable(preset, launch)?);
        command
            .args(&preset.args)
            .envs(&preset.env)
            .current_dir(&launch.working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8");
        Self::spawn(command)
    }

    pub(super) fn spawn(mut command: tokio::process::Command) -> Result<Self> {
        let (mut child, tree) = nexa_core::managed_process::spawn(&mut command).map_err(error)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| error("ACP stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| error("ACP stdout unavailable"))?;
        let (tx, messages) = mpsc::channel(16);
        let reader = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut frame = Vec::new();
            loop {
                let chunk = match reader.fill_buf().await {
                    Ok([]) => {
                        if !frame.is_empty() {
                            let _ = tx.send(Err(error("Truncated ACP frame"))).await;
                        }
                        break;
                    }
                    Ok(chunk) => chunk,
                    Err(cause) => {
                        let _ = tx.send(Err(error(cause))).await;
                        break;
                    }
                };
                let length = chunk
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(chunk.len(), |index| index + 1);
                if frame.len() + length > MAX_FRAME {
                    let _ = tx.send(Err(error("ACP frame too large"))).await;
                    break;
                }
                let complete = chunk[length - 1] == b'\n';
                frame.extend_from_slice(&chunk[..length]);
                reader.consume(length);
                if complete {
                    let parsed = serde_json::from_slice::<Value>(&frame)
                        .map_err(|_| error("Invalid ACP UTF-8 JSON"))
                        .and_then(|value| {
                            if value["jsonrpc"] == "2.0" {
                                Ok(value)
                            } else {
                                Err(error("Invalid ACP JSON-RPC version"))
                            }
                        });
                    let invalid = parsed.is_err();
                    if tx.send(parsed).await.is_err() || invalid {
                        break;
                    }
                    frame.clear();
                }
            }
        });
        Ok(Self {
            child,
            _tree: tree,
            stdin,
            messages,
            reader,
            queued: VecDeque::new(),
            next_id: 1,
        })
    }

    pub(super) async fn write(&mut self, message: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&message).map_err(error)?;
        if bytes.len() > MAX_FRAME {
            return Err(error("ACP request too large"));
        }
        bytes.push(b'\n');
        tokio::time::timeout(RPC_TIMEOUT, self.stdin.write_all(&bytes))
            .await
            .map_err(|_| error("ACP write timed out"))?
            .map_err(error)
    }

    pub(super) async fn send(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        Ok(id)
    }

    pub(super) async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.write(json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await
    }

    pub(super) async fn receive(&mut self) -> Result<Value> {
        if let Some(message) = self.queued.pop_front() {
            return Ok(message);
        }
        self.messages
            .recv()
            .await
            .ok_or_else(|| error("External agent exited before completing the request"))?
    }

    pub(super) async fn unsupported(&mut self, message: &Value) -> Result<()> {
        self.write(json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32601,"message":"This ACP client capability is not supported"}})).await
    }

    pub(super) async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send(method, params).await?;
        tokio::time::timeout(RPC_TIMEOUT, async {
            loop {
                let message = self.messages.recv().await.ok_or_else(|| error("External agent exited during connection"))??;
                if message.get("method").is_none() && message["id"].as_u64() == Some(id) {
                    return rpc_result(message, method);
                }
                if message.get("method").is_some() && message.get("id").is_some() {
                    // A connection/model probe cannot authorize side effects.
                    if message["method"] == "session/request_permission" {
                        self.write(json!({"jsonrpc":"2.0","id":message["id"],"result":{"outcome":{"outcome":"cancelled"}}})).await?;
                    } else { self.unsupported(&message).await?; }
                } else {
                    if self.queued.len() >= 256 { return Err(error("ACP startup event budget exceeded")); }
                    self.queued.push_back(message);
                }
            }
        }).await.map_err(|_| error(format!("ACP {method} timed out")))?
    }
}

pub(super) fn rpc_result(message: Value, method: &str) -> Result<Value> {
    if let Some(failure) = message.get("error") {
        if failure["code"] == -32000 {
            return Err(error("The external agent requires authentication. Sign in through its official CLI, then check the connection again."));
        }
        // RPC errors may contain environment/secrets; show only the method/code.
        return Err(error(format!(
            "ACP {method} failed (code {}). Check the agent's local configuration.",
            failure["code"]
                .as_i64()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "invalid".into())
        )));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| error("ACP response has no result"))
}
