//! ACP v1: one UTF-8 JSON-RPC 2.0 message per line on stdio.
use super::{error, Result};
use nexa_core::{
    external_agent::{ExternalAgentLaunch, ExternalAgentPreset},
    managed_process::ProcessTree,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
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
    stderr_reader: Option<tokio::task::JoinHandle<()>>,
    diagnostics: Arc<Mutex<Vec<u8>>>,
    secrets: Vec<String>,
    queued: VecDeque<Value>,
    next_id: u64,
}

impl Drop for Wire {
    fn drop(&mut self) {
        self.reader.abort();
        if let Some(reader) = &self.stderr_reader {
            reader.abort();
        }
        let _ = self.child.start_kill();
    }
}

// The ACP session and read-only account probe share executable, environment,
// cwd and Windows shim validation, so they cannot silently use different homes.
pub(super) fn command(
    preset: &ExternalAgentPreset,
    launch: &ExternalAgentLaunch,
    args: &[String],
) -> Result<tokio::process::Command> {
    launch.validate_resolved()?;
    let mut probe_launch = launch.clone();
    probe_launch.args = Some(args.to_vec());
    let plan =
        nexa_core::external_agent::launch::installed(preset, &probe_launch)?.ok_or_else(|| {
            error(format!(
                "{} ACP executable was not found; check its connection in Settings.",
                preset.name
            ))
        })?;
    command_from_plan(&plan, launch)
}

pub(super) fn command_from_plan(
    plan: &nexa_core::external_agent::launch::LaunchPlan,
    launch: &ExternalAgentLaunch,
) -> Result<tokio::process::Command> {
    let executable = &plan.executable;
    let args = &plan.args;
    if cfg!(windows)
        && executable.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
        && args
            .iter()
            .any(|argument| argument.contains(['%', '!', '^', '&', '|', '<', '>', '\r', '\n']))
    {
        return Err(error("These arguments require a native executable, not a Windows command shim. Select the agent's executable (or Node/Python executable) directly."));
    }
    let mut command = tokio::process::Command::new(executable);
    command
        .args(args)
        .envs(&plan.env)
        .current_dir(&launch.working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8");
    Ok(command)
}

impl Wire {
    pub(super) async fn prepare(
        preset: &ExternalAgentPreset,
        launch: &ExternalAgentLaunch,
        cache_root: Option<&Path>,
    ) -> Result<Self> {
        let plan = nexa_core::external_agent::launch::prepare(preset, launch, cache_root).await?;
        Self::spawn(command_from_plan(&plan, launch)?)
    }
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
        let plan = nexa_core::external_agent::launch::installed(preset, launch)?
            .ok_or_else(|| error("ACP executable was not found"))?;
        Self::spawn(command_from_plan(&plan, launch)?)
    }

    pub(super) fn spawn(mut command: tokio::process::Command) -> Result<Self> {
        let secrets = std::env::vars()
            .filter(|(key, _)| {
                let key = key.to_ascii_lowercase();
                ["key", "token", "secret", "password", "credential"]
                    .iter()
                    .any(|part| key.contains(part))
            })
            .map(|(_, value)| value)
            .chain(
                command
                    .as_std()
                    .get_envs()
                    .filter(|(key, _)| !key.eq_ignore_ascii_case("PATH"))
                    .filter_map(|(_, value)| value.map(|v| v.to_string_lossy().into_owned())),
            )
            .filter(|value| value.len() >= 4)
            .collect();
        let (mut child, tree) = nexa_core::managed_process::spawn(&mut command).map_err(error)?;
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let stderr_reader = child.stderr.take().map(|mut stderr| {
            let diagnostics = diagnostics.clone();
            tokio::spawn(async move {
                let mut buffer = [0u8; 2048];
                while let Ok(length) = stderr.read(&mut buffer).await {
                    if length == 0 {
                        break;
                    }
                    let mut tail = diagnostics.lock().unwrap_or_else(|e| e.into_inner());
                    tail.extend_from_slice(&buffer[..length]);
                    let excess = tail.len().saturating_sub(8192);
                    tail.drain(..excess);
                }
            })
        });
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
                    // Empty lines are harmless transport whitespace, commonly
                    // emitted by Windows package runners before initialization.
                    if frame.iter().all(u8::is_ascii_whitespace) {
                        frame.clear();
                        continue;
                    }
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
            stderr_reader,
            diagnostics,
            secrets,
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
        match self.messages.recv().await {
            Some(result) => result,
            None => Err(self
                .closed_error("External agent exited before completing the request")
                .await),
        }
    }

    async fn closed_error(&mut self, context: &str) -> nexa_core::error::CoreError {
        if let Some(reader) = self.stderr_reader.as_mut() {
            let _ = tokio::time::timeout(Duration::from_millis(100), reader).await;
        }
        if self
            .stderr_reader
            .as_ref()
            .is_some_and(|reader| reader.is_finished())
        {
            self.stderr_reader.take();
        }
        self.diagnostic_error(context)
    }

    fn diagnostic_error(&mut self, context: &str) -> nexa_core::error::CoreError {
        let status = self
            .child
            .try_wait()
            .ok()
            .flatten()
            .map(|status| format!(" ({status})"))
            .unwrap_or_default();
        let bytes = self.diagnostics.lock().unwrap_or_else(|e| e.into_inner());
        let detail = nexa_core::external_agent::launch::redact_diagnostic(
            &String::from_utf8_lossy(&bytes),
            &self.secrets,
        );
        error(if detail.is_empty() {
            format!("{context}{status}")
        } else {
            format!("{context}{status}\n{detail}")
        })
    }

    pub(super) async fn unsupported(&mut self, message: &Value) -> Result<()> {
        self.write(json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32601,"message":"This ACP client capability is not supported"}})).await
    }

    pub(super) async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        rpc_result(self.request_response(method, params).await?, method)
    }

    pub(super) async fn request_response(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send(method, params).await?;
        // A reviewed package runner may perform its first installation before
        // initialization. Ordinary RPCs keep the short liveness timeout.
        let timeout = match method {
            "initialize" => Duration::from_secs(180),
            "session/new" => Duration::from_secs(120),
            "authenticate" => Duration::from_secs(300),
            _ => RPC_TIMEOUT,
        };
        tokio::time::timeout(timeout, async {
            loop {
                let message = match self.messages.recv().await {
                    Some(result) => result?,
                    None => return Err(self.closed_error("External agent exited during connection").await),
                };
                if message.get("method").is_none() && message["id"].as_u64() == Some(id) {
                    return Ok(message);
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
        }).await.map_err(|_| self.diagnostic_error(&format!("ACP {method} timed out")))?
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
