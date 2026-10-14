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
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::{Child, ChildStdin},
    sync::mpsc,
};

pub(super) const RPC_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_FRAME: usize = 8 * 1024 * 1024;

fn inherited_diagnostic_secrets(
    environment: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<String> {
    environment
        .into_iter()
        .filter(|(key, _)| {
            let key = key.to_string_lossy().to_ascii_lowercase();
            ["key", "token", "secret", "password", "credential"]
                .iter()
                .any(|part| key.contains(part))
        })
        // stderr is decoded lossily too, so preserve the same representation
        // for redaction without requiring the inherited environment to be UTF-8.
        .map(|(_, value)| value.to_string_lossy().into_owned())
        .collect()
}

fn with_bundled_runtime(
    preset: &ExternalAgentPreset,
    launch: &ExternalAgentLaunch,
    installed: Option<nexa_core::external_agent::launch::LaunchPlan>,
    resolve_copilot: impl FnOnce() -> std::result::Result<std::path::PathBuf, String>,
) -> Result<Option<nexa_core::external_agent::launch::LaunchPlan>> {
    if preset.provider != "github_copilot_acp"
        || launch.executable.is_some()
        || launch
            .env
            .keys()
            .any(|key| key.eq_ignore_ascii_case("PATH"))
        || installed.as_ref().is_some_and(|plan| !plan.prepared)
    {
        return Ok(installed);
    }
    match resolve_copilot() {
        Ok(executable) => {
            let mut bundled = launch.clone();
            bundled.executable = Some(executable.to_string_lossy().into_owned());
            nexa_core::external_agent::launch::installed(preset, &bundled)
        }
        Err(reason) if reason == "copilot_runtime_not_found" => Ok(installed),
        Err(reason) => Err(error(reason)),
    }
}

pub(super) async fn prepare_plan(
    preset: &ExternalAgentPreset,
    launch: &ExternalAgentLaunch,
    cache_root: Option<&Path>,
) -> Result<nexa_core::external_agent::launch::LaunchPlan> {
    let installed = nexa_core::external_agent::launch::installed(preset, launch)?;
    let preset_owned = preset.clone();
    let launch_owned = launch.clone();
    let installed = tokio::task::spawn_blocking(move || {
        with_bundled_runtime(
            &preset_owned,
            &launch_owned,
            installed,
            crate::commands::subscription_accounts::resolve_copilot_binary,
        )
    })
    .await
    .map_err(error)??;
    if let Some(plan) = installed {
        return Ok(plan);
    }
    nexa_core::external_agent::launch::prepare(preset, launch, cache_root).await
}

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
        let plan = prepare_plan(preset, launch, cache_root).await?;
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
        let secrets = inherited_diagnostic_secrets(std::env::vars_os())
            .into_iter()
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
                        let _ = tx
                            .send(Err(nexa_core::error::CoreError::StreamIncomplete(
                                cause.to_string(),
                            )))
                            .await;
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
        super::super::write_runtime_message(&mut self.stdin, &bytes, RPC_TIMEOUT, "ACP").await
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
            None => Err(nexa_core::error::CoreError::StreamIncomplete(
                self.closed_error("External agent exited before completing the request")
                    .await
                    .to_string(),
            )),
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
        let detail = self.stderr_detail();
        error(if detail.is_empty() {
            format!("{context}{status}")
        } else {
            format!("{context}{status}\n{detail}")
        })
    }

    fn stderr_detail(&self) -> String {
        let bytes = self.diagnostics.lock().unwrap_or_else(|e| e.into_inner());
        nexa_core::external_agent::launch::redact_diagnostic(
            &String::from_utf8_lossy(&bytes),
            &self.secrets,
        )
    }

    pub(super) async fn unsupported(&mut self, message: &Value) -> Result<()> {
        self.write(json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32601,"message":"This ACP client capability is not supported"}})).await
    }

    pub(super) async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let response = self.request_response(method, params).await?;
        self.rpc_result(response, method)
    }

    pub(super) fn rpc_result(&self, message: Value, method: &str) -> Result<Value> {
        rpc_result(message, method, &self.secrets).map_err(|cause| {
            let detail = self.stderr_detail();
            match cause {
                nexa_core::error::CoreError::Agent(context) if !detail.is_empty() => {
                    nexa_core::error::CoreError::Agent(format!("{context}\n{detail}"))
                }
                cause => cause,
            }
        })
    }

    pub(super) async fn request_response(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.send(method, params).await?;
        // A reviewed package runner may perform its first installation before
        // initialization. Ordinary RPCs keep the short liveness timeout.
        let timeout = match method {
            "initialize" => Duration::from_secs(600),
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

fn rpc_result(message: Value, method: &str, secrets: &[String]) -> Result<Value> {
    if let Some(failure) = message.get("error") {
        if failure["code"] == -32000 {
            return Err(error("The external agent requires authentication. Sign in through its official CLI, then check the connection again."));
        }
        // Preserve actionable setup failures (missing runtimes, configuration)
        // under the same bounded credential redaction as stderr. Never dump
        // arbitrary RPC data, which may include prompts or credential objects.
        let detail = nexa_core::external_agent::launch::redact_diagnostic(
            &[
                failure["message"].as_str(),
                failure.pointer("/data/message").and_then(Value::as_str),
                failure.pointer("/data/error").and_then(Value::as_str),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n"),
            secrets,
        );
        return Err(error(format!(
            "ACP {method} failed (code {}). {}",
            failure["code"]
                .as_i64()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "invalid".into()),
            if detail.is_empty() {
                "Check the agent's local configuration."
            } else {
                &detail
            }
        )));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| error("ACP response has no result"))
}

#[cfg(test)]
mod launch_tests {
    use super::*;

    #[test]
    fn rpc_failures_preserve_setup_diagnostics_without_disclosing_credentials_or_payloads() {
        let error = rpc_result(json!({"error": {
            "code": -32603,
            "message": "Pi executable was not found; token=opaque-private-value",
            "data": {"error": "Install Pi, then retry", "credentials": "private-data", "prompt": "private-prompt"}
        }}), "session/new", &["opaque-private-value".into()]).unwrap_err().to_string();
        assert!(error.contains("Pi executable was not found"));
        assert!(error.contains("Install Pi"));
        assert!(!error.contains("opaque-private-value"));
        assert!(!error.contains("private-data"));
        assert!(!error.contains("private-prompt"));
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_environment_remains_redacted_without_panicking() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let secrets = inherited_diagnostic_secrets([
            (
                OsString::from_vec(b"API_TOKEN\xff".to_vec()),
                OsString::from_vec(b"private\xfevalue".to_vec()),
            ),
            (OsString::from("UNRELATED"), OsString::from_vec(vec![0xff])),
        ]);
        assert_eq!(secrets.len(), 1);
        let stderr = String::from_utf8_lossy(b"failed with private\xfevalue");
        let redacted = nexa_core::external_agent::launch::redact_diagnostic(&stderr, &secrets);
        assert!(!redacted.contains("private"));
        assert!(redacted.contains("[REDACTED]"));
    }
    #[test]
    fn bundled_copilot_fills_a_missing_default_runtime_without_overriding_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("copilot.exe");
        std::fs::write(&executable, "fixture").unwrap();
        let preset = nexa_core::external_agent::preset("github_copilot_acp").unwrap();
        let mut launch = ExternalAgentLaunch {
            working_directory: directory.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        let plan = with_bundled_runtime(preset, &launch, None, || Ok(executable.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(plan.executable, executable);
        assert_eq!(plan.args, vec!["--acp"]);
        launch.env.insert("PATH".into(), String::new());
        assert!(with_bundled_runtime(preset, &launch, None, || panic!(
            "explicit PATH must be authoritative"
        ))
        .unwrap()
        .is_none());
        launch.env.clear();
        launch.executable = Some(executable.to_string_lossy().into_owned());
        let custom = nexa_core::external_agent::launch::installed(preset, &launch).unwrap();
        assert!(with_bundled_runtime(preset, &launch, custom, || panic!(
            "explicit executable must be authoritative"
        ))
        .unwrap()
        .is_some());
    }
}
