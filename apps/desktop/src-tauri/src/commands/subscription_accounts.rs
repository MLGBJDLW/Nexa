use std::env;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;
use url::Url;

use github_copilot_sdk::rpc::AccountGetQuotaRequest;
use github_copilot_sdk::{install_bundled_cli, Client};

use super::AppState;

const CODEX_BINARY_OVERRIDE: &str = "NEXA_CODEX_BIN";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const LOGIN_MAX_AGE: Duration = Duration::from_secs(15 * 60);
const COPILOT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Explicit account import for the API provider. Never reads another editor's
/// credentials or changes the SDK/ACP authentication store.
#[tauri::command]
pub async fn import_github_copilot_api_token_cmd() -> Result<String, String> {
    let mut command = tokio::process::Command::new("gh");
    command
        .args(["auth", "token", "--hostname", "github.com"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .map_err(|_| "GitHub CLI account lookup timed out".to_string())?
        .map_err(|_| {
            "GitHub CLI is unavailable. Sign in with gh auth login or paste an account token."
                .to_string()
        })?;
    if !output.status.success() {
        return Err(
            "GitHub CLI has no signed-in github.com account. Run gh auth login first.".into(),
        );
    }
    let token = String::from_utf8(output.stdout)
        .map_err(|_| "Invalid GitHub CLI account token".to_string())?
        .trim()
        .to_owned();
    if token.is_empty() {
        return Err("GitHub CLI returned an empty account token".into());
    }
    Ok(token)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRateLimitWindow {
    used_percent: Option<f64>,
    window_duration_mins: Option<u64>,
    resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRateLimitBucket {
    id: String,
    name: Option<String>,
    plan_type: Option<String>,
    primary: Option<CodexRateLimitWindow>,
    secondary: Option<CodexRateLimitWindow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountIdentity {
    account_type: String,
    email: Option<String>,
    plan_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountUsage {
    lifetime_tokens: Option<u64>,
    current_streak_days: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLoginDescriptor {
    login_id: String,
    kind: String,
    auth_url: Option<String>,
    verification_url: Option<String>,
    user_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLoginCompletion {
    success: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccountSnapshot {
    available: bool,
    runtime_version: Option<String>,
    error_code: Option<String>,
    requires_openai_auth: Option<bool>,
    account: Option<CodexAccountIdentity>,
    rate_limits: Vec<CodexRateLimitBucket>,
    usage: Option<CodexAccountUsage>,
    pending_login: Option<CodexLoginDescriptor>,
    last_login: Option<CodexLoginCompletion>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopilotModelSummary {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) context_tiers: Vec<nexa_core::context_policy::NativeContextTier>,
    pub(crate) id: String,
    pub(super) name: String,
    pub(super) reasoning_efforts: Vec<String>,
    pub(super) supports_reasoning_effort: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) context_window: Option<u32>,
}

pub(crate) fn copilot_context_tiers(
    model: &github_copilot_sdk::rpc::Model,
) -> Vec<nexa_core::context_policy::NativeContextTier> {
    use nexa_core::context_policy::NativeContextTier;
    let positive =
        |value: Option<i64>| value.and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
    let prices = model
        .billing
        .as_ref()
        .and_then(|billing| billing.token_prices.as_ref());
    let mut tiers = vec![NativeContextTier {
        id: "default".into(),
        prompt_token_limit: positive(prices.and_then(|price| price.max_prompt_tokens).or_else(
            || {
                model
                    .capabilities
                    .limits
                    .as_ref()
                    .and_then(|limits| limits.max_prompt_tokens)
            },
        )),
    }];
    if let Some(long) = prices.and_then(|price| price.long_context.as_ref()) {
        tiers.push(NativeContextTier {
            id: "long_context".into(),
            prompt_token_limit: positive(long.max_prompt_tokens),
        });
    }
    tiers
}

pub(crate) fn copilot_reasoning_efforts(model: &github_copilot_sdk::rpc::Model) -> Vec<String> {
    if model
        .capabilities
        .supports
        .as_ref()
        .and_then(|supports| supports.reasoning_effort)
        == Some(false)
    {
        return Vec::new();
    }
    model
        .supported_reasoning_efforts
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .collect()
}

fn copilot_model_summary(model: github_copilot_sdk::rpc::Model) -> CopilotModelSummary {
    CopilotModelSummary {
        context_tiers: copilot_context_tiers(&model),
        context_window: model
            .capabilities
            .limits
            .as_ref()
            .and_then(|limits| limits.max_context_window_tokens)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0),
        id: model.id.clone(),
        name: bounded_field(&model.name, 160),
        reasoning_efforts: copilot_reasoning_efforts(&model),
        supports_reasoning_effort: model
            .capabilities
            .supports
            .as_ref()
            .and_then(|supports| supports.reasoning_effort),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopilotQuotaSnapshot {
    id: String,
    remaining_percent: Option<f64>,
    reset_date: Option<String>,
    unlimited: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopilotAccountSnapshot {
    available: bool,
    runtime_version: Option<String>,
    error_code: Option<String>,
    authenticated: bool,
    entitlement_verified: bool,
    auth_type: Option<String>,
    login: Option<String>,
    host: Option<String>,
    models: Vec<CopilotModelSummary>,
    quotas: Vec<CopilotQuotaSnapshot>,
    login_pending: bool,
    login_error: Option<String>,
}

impl CopilotAccountSnapshot {
    fn unavailable(error_code: impl Into<String>, login: CopilotLoginStatus) -> Self {
        Self {
            available: false,
            runtime_version: None,
            error_code: Some(error_code.into()),
            authenticated: false,
            entitlement_verified: false,
            auth_type: None,
            login: None,
            host: None,
            models: Vec::new(),
            quotas: Vec::new(),
            login_pending: login.pending,
            login_error: login.error,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct CopilotLoginStatus {
    pending: bool,
    error: Option<String>,
}

#[derive(Default)]
struct CopilotAccountRuntimeInner {
    login: Option<Child>,
    login_error: Option<String>,
}

#[derive(Clone, Default)]
pub struct CopilotAccountRuntime {
    inner: Arc<Mutex<CopilotAccountRuntimeInner>>,
}

impl CopilotAccountRuntime {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, CopilotAccountRuntimeInner>, String> {
        self.inner
            .lock()
            .map_err(|_| "copilot_runtime_state_poisoned".to_string())
    }

    fn login_status(&self) -> CopilotLoginStatus {
        let mut inner = match self.lock() {
            Ok(inner) => inner,
            Err(error) => {
                return CopilotLoginStatus {
                    pending: false,
                    error: Some(error),
                }
            }
        };
        let mut status_error = false;
        if let Some(login) = inner.login.as_mut() {
            match login.try_wait() {
                Ok(None) => {
                    return CopilotLoginStatus {
                        pending: true,
                        error: None,
                    }
                }
                Ok(Some(status)) if status.success() => {
                    inner.login_error = None;
                }
                Ok(Some(_)) => {
                    inner.login_error = Some("copilot_login_failed".to_string());
                }
                Err(_) => {
                    status_error = true;
                }
            }
            if let Some(mut finished) = inner.login.take() {
                if status_error {
                    let _ = finished.kill();
                    let _ = finished.wait();
                    inner.login_error = Some("copilot_login_status_failed".to_string());
                }
            }
        }
        CopilotLoginStatus {
            pending: false,
            error: inner.login_error.clone(),
        }
    }

    fn start_login(&self) -> Result<(), String> {
        let mut inner = self.lock()?;
        if let Some(login) = inner.login.as_mut() {
            match login.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(_)) => {}
                Err(_) => {
                    if let Some(mut stale) = inner.login.take() {
                        let _ = stale.kill();
                        let _ = stale.wait();
                    }
                }
            }
            inner.login.take();
        }
        let binary = resolve_copilot_binary()?;
        let mut command = Command::new(binary);
        command
            .args(["login", "--web-flow", "--no-auto-update"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_background_command(&mut command);
        inner.login = Some(
            command
                .spawn()
                .map_err(|_| "copilot_login_start_failed".to_string())?,
        );
        inner.login_error = None;
        Ok(())
    }

    fn cancel_login(&self) -> Result<(), String> {
        let mut inner = self.lock()?;
        if let Some(mut login) = inner.login.take() {
            let _ = login.kill();
            let _ = login.wait();
        }
        inner.login_error = None;
        Ok(())
    }
}

impl Drop for CopilotAccountRuntimeInner {
    fn drop(&mut self) {
        if let Some(login) = self.login.as_mut() {
            let _ = login.kill();
            let _ = login.wait();
        }
    }
}

impl CodexAccountSnapshot {
    fn unavailable(error_code: impl Into<String>) -> Self {
        Self {
            available: false,
            runtime_version: None,
            error_code: Some(error_code.into()),
            requires_openai_auth: None,
            account: None,
            rate_limits: Vec::new(),
            usage: None,
            pending_login: None,
            last_login: None,
        }
    }
}

#[derive(Default)]
struct CodexAccountRuntimeInner {
    pending_login: Option<PendingCodexLogin>,
    last_login: Option<CodexLoginCompletion>,
}

struct PendingCodexLogin {
    client: CodexAppServerClient,
    descriptor: CodexLoginDescriptor,
    started_at: Instant,
}

#[derive(Clone, Default)]
pub struct CodexAccountRuntime {
    inner: Arc<Mutex<CodexAccountRuntimeInner>>,
}

impl CodexAccountRuntime {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, CodexAccountRuntimeInner>, String> {
        self.inner
            .lock()
            .map_err(|_| "codex_runtime_state_poisoned".to_string())
    }

    fn snapshot(&self) -> CodexAccountSnapshot {
        let mut inner = match self.lock() {
            Ok(inner) => inner,
            Err(error) => return CodexAccountSnapshot::unavailable(error),
        };

        if inner
            .pending_login
            .as_ref()
            .is_some_and(|pending| pending.started_at.elapsed() > LOGIN_MAX_AGE)
        {
            if let Some(mut expired) = inner.pending_login.take() {
                let _ = expired.client.request(
                    "account/login/cancel",
                    Some(json!({ "loginId": expired.descriptor.login_id })),
                );
            }
            inner.last_login = Some(CodexLoginCompletion {
                success: false,
                error: Some("login_expired".to_string()),
            });
        }

        if inner.pending_login.is_some() {
            let (mut snapshot, completion, descriptor) = {
                let pending = inner.pending_login.as_mut().expect("checked above");
                let snapshot = match pending.client.account_snapshot() {
                    Ok(snapshot) => snapshot,
                    Err(error) => CodexAccountSnapshot::unavailable(error),
                };
                (
                    snapshot,
                    pending.client.take_login_completion(),
                    pending.descriptor.clone(),
                )
            };
            if let Some(completion) = completion {
                inner.last_login = Some(completion);
            }
            snapshot.pending_login = Some(descriptor);
            snapshot.last_login = inner.last_login.clone();

            let login_finished = snapshot.account.is_some()
                || snapshot
                    .last_login
                    .as_ref()
                    .is_some_and(|completion| !completion.success || snapshot.account.is_some());
            if login_finished {
                snapshot.pending_login = None;
                inner.pending_login.take();
                inner.last_login.take();
            }
            return snapshot;
        }

        let last_login = inner.last_login.take();
        drop(inner);
        match CodexAppServerClient::start().and_then(|mut client| client.account_snapshot()) {
            Ok(mut snapshot) => {
                snapshot.last_login = last_login;
                snapshot
            }
            Err(error) => CodexAccountSnapshot::unavailable(error),
        }
    }

    fn start_login(&self, kind: CodexLoginKind) -> Result<CodexLoginDescriptor, String> {
        let mut inner = self.lock()?;
        if let Some(pending) = inner.pending_login.as_ref() {
            return Ok(pending.descriptor.clone());
        }

        let mut client = CodexAppServerClient::start()?;
        let (method, params) = match kind {
            CodexLoginKind::Browser => (
                "chatgpt",
                json!({
                    "type": "chatgpt",
                    "appBrand": "codex",
                    "codexStreamlinedLogin": false,
                    "useHostedLoginSuccessPage": true
                }),
            ),
            CodexLoginKind::DeviceCode => {
                ("chatgptDeviceCode", json!({ "type": "chatgptDeviceCode" }))
            }
        };
        let response = client.request("account/login/start", Some(params))?;
        let descriptor = parse_login_descriptor(method, &response)?;
        inner.last_login = None;
        inner.pending_login = Some(PendingCodexLogin {
            client,
            descriptor: descriptor.clone(),
            started_at: Instant::now(),
        });
        Ok(descriptor)
    }

    fn cancel_login(&self, login_id: &str) -> Result<(), String> {
        let mut inner = self.lock()?;
        let Some(mut pending) = inner.pending_login.take() else {
            return Ok(());
        };
        if pending.descriptor.login_id != login_id {
            inner.pending_login = Some(pending);
            return Err("codex_login_id_mismatch".to_string());
        }
        pending
            .client
            .request("account/login/cancel", Some(json!({ "loginId": login_id })))?;
        inner.last_login = None;
        Ok(())
    }

    fn logout(&self) -> Result<CodexAccountSnapshot, String> {
        let mut inner = self.lock()?;
        if let Some(mut pending) = inner.pending_login.take() {
            let _ = pending.client.request(
                "account/login/cancel",
                Some(json!({ "loginId": pending.descriptor.login_id })),
            );
        }
        inner.last_login = None;
        drop(inner);

        let mut client = CodexAppServerClient::start()?;
        client.request("account/logout", None)?;
        client.account_snapshot()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CodexLoginKind {
    Browser,
    DeviceCode,
}

impl<'de> serde::Deserialize<'de> for CodexLoginKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        match value.as_str() {
            "browser" => Ok(Self::Browser),
            "deviceCode" => Ok(Self::DeviceCode),
            _ => Err(serde::de::Error::custom("unsupported Codex login kind")),
        }
    }
}

struct CodexAppServerClient {
    child: Child,
    process_tree: Option<nexa_core::managed_process::ProcessTree>,
    stdin: ChildStdin,
    messages: Receiver<Result<Value, String>>,
    reader: Option<JoinHandle<()>>,
    next_id: u64,
    runtime_version: String,
    last_login: Option<CodexLoginCompletion>,
}

impl CodexAppServerClient {
    fn start() -> Result<Self, String> {
        let binary = resolve_codex_binary()?;
        let mut command = Command::new(&binary.program);
        command
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        configure_background_command(&mut command);
        let (mut child, process_tree) = nexa_core::managed_process::spawn_standard(&mut command)
            .map_err(|_| "codex_runtime_start_failed".to_string())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "codex_runtime_stdin_unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "codex_runtime_stdout_unavailable".to_string())?;
        let (sender, messages) = mpsc::channel();
        let reader = thread::Builder::new()
            .name("nexa-codex-account-reader".to_string())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let message = match line {
                        Ok(line) if line.trim().is_empty() => continue,
                        Ok(line) => serde_json::from_str::<Value>(&line)
                            .map_err(|_| "codex_runtime_invalid_json".to_string()),
                        Err(_) => Err("codex_runtime_read_failed".to_string()),
                    };
                    if sender.send(message).is_err() {
                        break;
                    }
                }
            })
            .map_err(|_| "codex_runtime_reader_start_failed".to_string())?;

        let mut client = Self {
            child,
            process_tree: Some(process_tree),
            stdin,
            messages,
            reader: Some(reader),
            next_id: 1,
            runtime_version: binary.version,
            last_login: None,
        };
        client.request(
            "initialize",
            Some(json!({
                "clientInfo": {
                    "name": "nexa-desktop",
                    "title": "Nexa",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": { "experimentalApi": false }
            })),
        )?;
        client.notify("initialized", None)?;
        Ok(client)
    }

    fn notify(&mut self, method: &str, params: Option<Value>) -> Result<(), String> {
        let mut message = json!({ "method": method });
        if let Some(params) = params {
            message["params"] = params;
        }
        self.write_message(&message)
    }

    fn request(&mut self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let request_id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let mut message = json!({ "method": method, "id": request_id });
        if let Some(params) = params {
            message["params"] = params;
        }
        self.write_message(&message)?;

        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("codex_runtime_request_timeout".to_string());
            }
            let message = self
                .messages
                .recv_timeout(remaining)
                .map_err(|_| "codex_runtime_request_timeout".to_string())??;
            self.observe_notification(&message);
            if message.get("id").and_then(Value::as_u64) != Some(request_id) {
                continue;
            }
            if message.get("error").is_some() {
                return Err("codex_runtime_request_failed".to_string());
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| "codex_runtime_missing_result".to_string());
        }
    }

    fn write_message(&mut self, message: &Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.stdin, message)
            .map_err(|_| "codex_runtime_write_failed".to_string())?;
        self.stdin
            .write_all(b"\n")
            .and_then(|_| self.stdin.flush())
            .map_err(|_| "codex_runtime_write_failed".to_string())
    }

    fn observe_notification(&mut self, message: &Value) {
        if message.get("method").and_then(Value::as_str) != Some("account/login/completed") {
            return;
        }
        let params = message.get("params").unwrap_or(&Value::Null);
        let success = params
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.last_login = Some(CodexLoginCompletion {
            success,
            error: (!success).then(|| "codex_login_failed".to_string()),
        });
    }

    fn take_login_completion(&mut self) -> Option<CodexLoginCompletion> {
        self.last_login.take()
    }

    fn account_snapshot(&mut self) -> Result<CodexAccountSnapshot, String> {
        let account_response =
            self.request("account/read", Some(json!({ "refreshToken": false })))?;
        let account = parse_account(&account_response);
        let mut snapshot = CodexAccountSnapshot {
            available: true,
            runtime_version: Some(self.runtime_version.clone()),
            error_code: None,
            requires_openai_auth: account_response
                .get("requiresOpenaiAuth")
                .and_then(Value::as_bool),
            account,
            rate_limits: Vec::new(),
            usage: None,
            pending_login: None,
            last_login: self.last_login.clone(),
        };
        if snapshot.account.is_some() {
            if let Ok(rate_limits) = self.request("account/rateLimits/read", None) {
                snapshot.rate_limits = parse_rate_limits(&rate_limits);
            }
            // Token-activity totals are not subscription quota. Optional
            // account/usage/read support must not delay a successful quota read.
        }
        Ok(snapshot)
    }
}

impl Drop for CodexAppServerClient {
    fn drop(&mut self) {
        self.process_tree.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            // A wrapper's descendant can retain stdout after its parent exits.
            // Never hold an account snapshot (or login lock) waiting on that pipe.
            if reader.is_finished() {
                let _ = reader.join();
            }
        }
    }
}

pub(crate) struct CodexBinary {
    pub(crate) program: OsString,
    pub(crate) version: String,
}

pub(crate) fn resolve_codex_binary() -> Result<CodexBinary, String> {
    if let Some(raw_override) = env::var_os(CODEX_BINARY_OVERRIDE) {
        let path = PathBuf::from(raw_override);
        if !path.is_absolute() || !path.is_file() {
            return Err("codex_runtime_invalid_override".to_string());
        }
        return probe_codex_binary(path.into_os_string());
    }
    let environment = nexa_core::external_agent::launch::process_environment(&Default::default());
    let path = nexa_core::external_agent::launch::find_executable("codex", &environment)
        .ok_or_else(|| "codex_runtime_not_found".to_string())?;
    probe_codex_binary(path.into_os_string())
}

fn probe_codex_binary(program: OsString) -> Result<CodexBinary, String> {
    let mut command = Command::new(&program);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    configure_background_command(&mut command);
    let (mut child, tree) = nexa_core::managed_process::spawn_standard(&mut command)
        .map_err(|_| "codex_runtime_not_found".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "codex_runtime_stdout_unavailable".to_string())?;
    let (sender, received) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(4096).read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => break Err("codex_runtime_version_timeout".to_string()),
        }
    };
    drop(tree);
    let _ = child.kill();
    let _ = child.wait();
    let output = received.recv_timeout(Duration::from_secs(1));
    if reader.is_finished() {
        let _ = reader.join();
    }
    if !status?.success() {
        return Err("codex_runtime_version_failed".to_string());
    }
    let bytes = output
        .map_err(|_| "codex_runtime_version_timeout".to_string())?
        .map_err(|_| "codex_runtime_version_failed".to_string())?;
    let version = safe_detail(&String::from_utf8_lossy(&bytes));
    if version.is_empty() {
        return Err("codex_runtime_version_failed".to_string());
    }
    Ok(CodexBinary { program, version })
}

fn parse_account(response: &Value) -> Option<CodexAccountIdentity> {
    let account = response.get("account")?.as_object()?;
    Some(CodexAccountIdentity {
        account_type: bounded_field(account.get("type")?.as_str()?, 40),
        email: account
            .get("email")
            .and_then(Value::as_str)
            .map(|value| bounded_field(value, 254)),
        plan_type: account
            .get("planType")
            .and_then(Value::as_str)
            .map(|value| bounded_field(value, 64)),
    })
}

fn parse_rate_window(value: Option<&Value>) -> Option<CodexRateLimitWindow> {
    let value = value?.as_object()?;
    // Missing/malformed usage is unknown, never a fabricated 100% remaining.
    let used_percent = value
        .get("usedPercent")
        .and_then(Value::as_f64)
        .filter(|used| used.is_finite())
        .map(|used| used.clamp(0.0, 100.0));
    Some(CodexRateLimitWindow {
        used_percent,
        window_duration_mins: value.get("windowDurationMins").and_then(Value::as_u64),
        resets_at: value.get("resetsAt").and_then(Value::as_i64),
    })
}

fn parse_rate_bucket(fallback_id: &str, value: &Value) -> Option<CodexRateLimitBucket> {
    let value = value.as_object()?;
    Some(CodexRateLimitBucket {
        id: value
            .get("limitId")
            .and_then(Value::as_str)
            .map(|value| bounded_field(value, 80))
            .unwrap_or_else(|| bounded_field(fallback_id, 80)),
        name: value
            .get("limitName")
            .and_then(Value::as_str)
            .map(|value| bounded_field(value, 100)),
        plan_type: value
            .get("planType")
            .and_then(Value::as_str)
            .map(|value| bounded_field(value, 64)),
        primary: parse_rate_window(value.get("primary")),
        secondary: parse_rate_window(value.get("secondary")),
    })
}

fn parse_rate_limits(response: &Value) -> Vec<CodexRateLimitBucket> {
    if let Some(by_id) = response
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
    {
        let mut buckets = by_id
            .iter()
            .filter_map(|(id, value)| parse_rate_bucket(id, value))
            .collect::<Vec<_>>();
        buckets.sort_by(|left, right| left.id.cmp(&right.id));
        return buckets;
    }
    response
        .get("rateLimits")
        .and_then(|value| parse_rate_bucket("codex", value))
        .into_iter()
        .collect()
}

fn parse_login_descriptor(kind: &str, response: &Value) -> Result<CodexLoginDescriptor, String> {
    if response.get("type").and_then(Value::as_str) != Some(kind) {
        return Err("codex_login_response_type_mismatch".to_string());
    }
    let login_id = response
        .get("loginId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .ok_or_else(|| "codex_login_id_missing".to_string())?
        .to_string();
    let auth_url = response
        .get("authUrl")
        .and_then(Value::as_str)
        .map(validate_openai_login_url)
        .transpose()?;
    let verification_url = response
        .get("verificationUrl")
        .and_then(Value::as_str)
        .map(validate_openai_login_url)
        .transpose()?;
    let user_code = response
        .get("userCode")
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
        .map(str::to_string);
    if kind == "chatgpt" && auth_url.is_none() {
        return Err("codex_login_url_missing".to_string());
    }
    if kind == "chatgptDeviceCode" && (verification_url.is_none() || user_code.is_none()) {
        return Err("codex_device_login_details_missing".to_string());
    }
    Ok(CodexLoginDescriptor {
        login_id,
        kind: if kind == "chatgpt" {
            "browser".to_string()
        } else {
            "deviceCode".to_string()
        },
        auth_url,
        verification_url,
        user_code,
    })
}

fn validate_openai_login_url(value: &str) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| "codex_login_url_invalid".to_string())?;
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "codex_login_url_invalid".to_string())?;
    let trusted_host = host == "openai.com"
        || host.ends_with(".openai.com")
        || host == "chatgpt.com"
        || host.ends_with(".chatgpt.com");
    if url.scheme() != "https" || !trusted_host || url.username() != "" || url.password().is_some()
    {
        return Err("codex_login_url_untrusted".to_string());
    }
    Ok(url.into())
}

fn safe_detail(value: &str) -> String {
    bounded_field(&value.split_whitespace().collect::<Vec<_>>().join(" "), 240)
}

fn bounded_field(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn resolve_copilot_binary() -> Result<PathBuf, String> {
    for variable in ["NEXA_COPILOT_BIN", "COPILOT_CLI_PATH"] {
        if let Some(raw_path) = env::var_os(variable) {
            let path = PathBuf::from(raw_path);
            if !path.is_absolute() || !path.is_file() {
                return Err("copilot_runtime_invalid_override".to_string());
            }
            return Ok(path);
        }
    }
    install_bundled_cli().ok_or_else(|| "copilot_runtime_not_found".to_string())
}

#[cfg(windows)]
fn configure_background_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_background_command(_command: &mut Command) {}

async fn read_copilot_account_snapshot(runtime: CopilotAccountRuntime) -> CopilotAccountSnapshot {
    let login = runtime.login_status();
    if login.pending {
        return CopilotAccountSnapshot {
            available: true,
            runtime_version: None,
            error_code: None,
            authenticated: false,
            entitlement_verified: false,
            auth_type: None,
            login: None,
            host: None,
            models: Vec::new(),
            quotas: Vec::new(),
            login_pending: true,
            login_error: None,
        };
    }
    let login_for_snapshot = login.clone();
    let result = tokio::time::timeout(COPILOT_REQUEST_TIMEOUT, async {
        let binary = tokio::task::spawn_blocking(resolve_copilot_binary)
            .await
            .map_err(|_| "copilot_runtime_task_failed".to_string())??;
        let options = crate::agent_runtime::copilot_options::client_options(binary);
        let client = Client::start(options)
            .await
            .map_err(|_| "copilot_runtime_start_failed".to_string())?;
        let status = client
            .get_status()
            .await
            .map_err(|_| "copilot_runtime_status_failed".to_string())?;
        let auth = client
            .get_auth_status()
            .await
            .map_err(|_| "copilot_auth_status_failed".to_string())?;

        let mut models = Vec::new();
        let mut quotas = Vec::new();
        let mut entitlement_verified = false;
        let mut error_code = None;
        if auth.is_authenticated {
            // A slow optional catalog must not consume the quota request's
            // entire deadline. These independent reads share one native client.
            let (catalog, quota) = tokio::join!(
                tokio::time::timeout(Duration::from_secs(10), client.list_models()),
                tokio::time::timeout(Duration::from_secs(10), async {
                    client
                        .rpc()
                        .account()
                        .get_quota_with_params(AccountGetQuotaRequest {
                            git_hub_token: None,
                        })
                        .await
                }),
            );
            match catalog {
                Ok(Ok(catalog)) => {
                    models = catalog.into_iter().map(copilot_model_summary).collect();
                    entitlement_verified = !models.is_empty();
                    if !entitlement_verified {
                        error_code = Some("copilot_entitlement_unverified".to_string());
                    }
                }
                _ => {
                    error_code = Some("copilot_entitlement_unverified".to_string());
                }
            }
            if let Ok(Ok(quota)) = quota {
                quotas = quota
                    .quota_snapshots
                    .into_iter()
                    .map(|(id, quota)| CopilotQuotaSnapshot {
                        id: bounded_field(&id, 80),
                        remaining_percent: quota
                            .remaining_percentage
                            .is_finite()
                            .then(|| quota.remaining_percentage.clamp(0.0, 100.0)),
                        reset_date: quota.reset_date.map(|date| bounded_field(&date, 80)),
                        unlimited: quota.is_unlimited_entitlement,
                    })
                    .collect::<Vec<_>>();
                quotas.sort_by(|left, right| left.id.cmp(&right.id));
            }
        }
        let snapshot = CopilotAccountSnapshot {
            available: true,
            runtime_version: Some(bounded_field(&status.version, 120)),
            error_code,
            authenticated: auth.is_authenticated,
            entitlement_verified,
            auth_type: auth.auth_type.map(|value| bounded_field(&value, 40)),
            login: auth.login.map(|value| bounded_field(&value, 100)),
            host: auth.host.map(|value| bounded_field(&value, 200)),
            models,
            quotas,
            login_pending: login_for_snapshot.pending,
            login_error: login_for_snapshot.error,
        };
        let _ = tokio::time::timeout(Duration::from_secs(2), client.stop()).await;
        Ok::<_, String>(snapshot)
    })
    .await;

    match result {
        Ok(Ok(snapshot)) => snapshot,
        Ok(Err(error)) => CopilotAccountSnapshot::unavailable(error, login),
        Err(_) => CopilotAccountSnapshot::unavailable("copilot_runtime_timeout", login),
    }
}

#[tauri::command]
pub async fn get_codex_account_snapshot_cmd(
    state: State<'_, AppState>,
) -> Result<CodexAccountSnapshot, String> {
    let runtime = state.codex_account_runtime.clone();
    tokio::task::spawn_blocking(move || runtime.snapshot())
        .await
        .map_err(|_| "codex_runtime_task_failed".to_string())
}

#[tauri::command]
pub async fn start_codex_account_login_cmd(
    state: State<'_, AppState>,
    kind: CodexLoginKind,
) -> Result<CodexLoginDescriptor, String> {
    let runtime = state.codex_account_runtime.clone();
    tokio::task::spawn_blocking(move || runtime.start_login(kind))
        .await
        .map_err(|_| "codex_runtime_task_failed".to_string())?
}

#[tauri::command]
pub async fn cancel_codex_account_login_cmd(
    state: State<'_, AppState>,
    login_id: String,
) -> Result<(), String> {
    let runtime = state.codex_account_runtime.clone();
    tokio::task::spawn_blocking(move || runtime.cancel_login(&login_id))
        .await
        .map_err(|_| "codex_runtime_task_failed".to_string())?
}

#[tauri::command]
pub async fn logout_codex_account_cmd(
    state: State<'_, AppState>,
) -> Result<CodexAccountSnapshot, String> {
    let runtime = state.codex_account_runtime.clone();
    tokio::task::spawn_blocking(move || runtime.logout())
        .await
        .map_err(|_| "codex_runtime_task_failed".to_string())?
}

#[tauri::command]
pub async fn get_copilot_account_snapshot_cmd(
    state: State<'_, AppState>,
) -> Result<CopilotAccountSnapshot, String> {
    Ok(read_copilot_account_snapshot(state.copilot_account_runtime.clone()).await)
}

#[tauri::command]
pub async fn list_subscription_models_cmd(
    state: State<'_, AppState>,
    provider: String,
    agent_config_id: Option<String>,
) -> Result<Vec<CopilotModelSummary>, String> {
    if nexa_core::external_agent::preset(&provider).is_some() {
        let id = agent_config_id.ok_or_else(|| {
            "Select an external-agent profile to discover its models.".to_string()
        })?;
        let config = state
            .db
            .get_agent_config(&id)
            .map_err(|error| error.to_string())?;
        if config.provider != provider {
            return Err("The profile belongs to a different external agent.".into());
        }
        let launch = state
            .db
            .external_agent_launch(&id)
            .map_err(|error| error.to_string())?;
        return crate::agent_runtime::acp::probe(
            &provider,
            &launch,
            &state.db,
            Some(&config.model),
        )
        .await
        .map_err(|error| error.to_string())
        .and_then(super::external_agents::connected_models);
    }
    if provider == "github_copilot" && state.copilot_account_runtime.login_status().pending {
        return Err("Finish signing in to GitHub Copilot, then refresh models.".into());
    }
    native_subscription_models(&provider).await
}

pub(crate) async fn native_subscription_models(
    provider: &str,
) -> Result<Vec<CopilotModelSummary>, String> {
    match provider {
        "github_copilot" => {
            // Chat needs only the entitled model list, not status, quotas or
            // runtime-version probes. The UI shares and caches this request.
            tokio::time::timeout(COPILOT_REQUEST_TIMEOUT, async {
                let binary = tokio::task::spawn_blocking(resolve_copilot_binary).await
                    .map_err(|_| "copilot_runtime_task_failed".to_string())??;
                let client = Client::start(crate::agent_runtime::copilot_options::client_options(binary)).await
                    .map_err(|_| "Unable to start Copilot. Check its installation and retry.".to_string())?;
                let result = async {
                    let auth = client.get_auth_status().await.map_err(|_| "Unable to verify the Copilot account.".to_string())?;
                    if !auth.is_authenticated { return Err("Sign in to GitHub Copilot, then refresh models.".to_string()); }
                    client.list_models().await.map_err(|_| "Unable to load Copilot models. Check the connection and account, then retry.".to_string())
                }.await;
                let _ = client.stop().await;
                result.map(|models| models.into_iter().map(copilot_model_summary).collect())
            }).await.map_err(|_| "Copilot model loading timed out. Please retry.".to_string())?
        }
        "openai_codex" => tokio::task::spawn_blocking(|| {
            let mut client = CodexAppServerClient::start()?;
            let account = client.request("account/read", Some(json!({"refreshToken":false})))?;
            if account.pointer("/account/type").and_then(Value::as_str) != Some("chatgpt") {
                return Err(
                    "Sign in with your ChatGPT subscription, then refresh the model list.".into(),
                );
            }
            let mut models = Vec::new();
            let mut cursor: Option<String> = None;
            let mut cursors = std::collections::HashSet::new();
            loop {
                let page = client.request(
                    "model/list",
                    Some(json!({"limit":100,"includeHidden":false,"cursor":cursor})),
                )?;
                let entries = page
                    .get("data")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "Codex returned an invalid model catalog".to_string())?;
                for item in entries {
                    if item.get("hidden").and_then(Value::as_bool) == Some(true) {
                        continue;
                    }
                    let Some(id) = item
                        .get("model")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                    else {
                        continue;
                    };
                    models.push(CopilotModelSummary {
                        supports_reasoning_effort: None,
                        context_tiers: Vec::new(),
                        context_window: item
                            .get("contextWindow")
                            .and_then(Value::as_u64)
                            .and_then(|v| u32::try_from(v).ok())
                            .filter(|v| *v > 0),
                        id: id.to_string(),
                        name: item
                            .get("displayName")
                            .and_then(Value::as_str)
                            .unwrap_or(id)
                            .to_string(),
                        reasoning_efforts: item
                            .get("supportedReasoningEfforts")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|effort| {
                                effort
                                    .get("reasoningEffort")
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned)
                            })
                            .collect(),
                    });
                }
                cursor = page
                    .get("nextCursor")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                let Some(next) = cursor.as_ref() else {
                    break;
                };
                if models.len() > 1000 || !cursors.insert(next.clone()) {
                    return Err("Codex model pagination did not converge".into());
                }
            }
            Ok(models)
        })
        .await
        .map_err(|_| "Codex model discovery task failed".to_string())?,
        _ => Err("Unknown subscription runtime".into()),
    }
}

#[tauri::command]
pub async fn start_copilot_account_login_cmd(state: State<'_, AppState>) -> Result<(), String> {
    let runtime = state.copilot_account_runtime.clone();
    tokio::task::spawn_blocking(move || runtime.start_login())
        .await
        .map_err(|_| "copilot_runtime_task_failed".to_string())?
}

#[tauri::command]
pub async fn cancel_copilot_account_login_cmd(state: State<'_, AppState>) -> Result<(), String> {
    let runtime = state.copilot_account_runtime.clone();
    tokio::task::spawn_blocking(move || runtime.cancel_login())
        .await
        .map_err(|_| "copilot_runtime_task_failed".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_projection_keeps_identity_but_never_projects_tokens() {
        let response = json!({
            "account": {
                "type": "chatgpt",
                "email": "person@example.com",
                "planType": "pro",
                "accessToken": "must-not-escape"
            },
            "requiresOpenaiAuth": true
        });

        let account = parse_account(&response).expect("account");
        let serialized = serde_json::to_string(&account).expect("serialize");
        assert_eq!(account.account_type, "chatgpt");
        assert_eq!(account.plan_type.as_deref(), Some("pro"));
        assert!(!serialized.contains("must-not-escape"));
        assert!(!serialized.contains("accessToken"));
    }

    #[test]
    fn rate_limit_projection_prefers_multi_bucket_response() {
        let response = json!({
            "rateLimits": { "limitId": "legacy", "primary": { "usedPercent": 90 } },
            "rateLimitsByLimitId": {
                "spark": {
                    "limitId": "spark",
                    "limitName": "Fast",
                    "planType": "plus",
                    "primary": { "usedPercent": 12, "windowDurationMins": 300 },
                    "secondary": { "usedPercent": 3, "windowDurationMins": 10080 }
                },
                "codex": { "limitId": "codex", "primary": { "usedPercent": 101 } }
            }
        });

        let buckets = parse_rate_limits(&response);
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].id, "codex");
        assert_eq!(
            buckets[0].primary.as_ref().unwrap().used_percent,
            Some(100.0)
        );
        assert_eq!(buckets[1].name.as_deref(), Some("Fast"));
        assert_eq!(
            buckets[1].secondary.as_ref().unwrap().window_duration_mins,
            Some(10080)
        );
    }

    #[test]
    fn rate_windows_preserve_fractional_usage_and_unknown_percentages() {
        for value in [
            json!({}),
            json!({"usedPercent": null}),
            json!({"usedPercent": "12"}),
        ] {
            assert!(parse_rate_window(Some(&value))
                .unwrap()
                .used_percent
                .is_none());
        }
        assert_eq!(
            parse_rate_window(Some(&json!({"usedPercent": 0.5})))
                .unwrap()
                .used_percent,
            Some(0.5)
        );
        let unknown = parse_rate_window(Some(
            &json!({"usedPercent":null,"windowDurationMins":300,"resetsAt":1791741600}),
        ))
        .unwrap();
        assert_eq!(unknown.window_duration_mins, Some(300));
        assert_eq!(unknown.resets_at, Some(1791741600));
    }

    #[test]
    fn login_links_are_https_and_owned_by_openai() {
        assert!(validate_openai_login_url("https://auth.openai.com/oauth/authorize").is_ok());
        assert!(validate_openai_login_url("https://chatgpt.com/codex/device").is_ok());
        assert!(validate_openai_login_url("http://auth.openai.com/oauth/authorize").is_err());
        assert!(validate_openai_login_url("https://openai.com.example.test/login").is_err());
        assert!(validate_openai_login_url("https://user@auth.openai.com/login").is_err());
    }

    #[test]
    fn device_login_requires_bounded_human_code() {
        let descriptor = parse_login_descriptor(
            "chatgptDeviceCode",
            &json!({
                "type": "chatgptDeviceCode",
                "loginId": "login-1",
                "userCode": "ABCD-EFGH",
                "verificationUrl": "https://auth.openai.com/codex/device"
            }),
        )
        .expect("valid descriptor");
        assert_eq!(descriptor.kind, "deviceCode");
        assert_eq!(descriptor.user_code.as_deref(), Some("ABCD-EFGH"));

        assert!(parse_login_descriptor(
            "chatgptDeviceCode",
            &json!({
                "type": "chatgptDeviceCode",
                "loginId": "login-2",
                "userCode": "<script>",
                "verificationUrl": "https://auth.openai.com/codex/device"
            }),
        )
        .is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires the bundled Copilot runtime and optional local GitHub authentication"]
    async fn installed_copilot_sdk_projects_only_safe_account_metadata() {
        let snapshot = read_copilot_account_snapshot(CopilotAccountRuntime::default()).await;
        assert!(snapshot.available, "{:?}", snapshot.error_code);
        eprintln!(
            "authenticated={} entitlement_verified={} models={} quotas={}",
            snapshot.authenticated,
            snapshot.entitlement_verified,
            snapshot.models.len(),
            snapshot.quotas.len()
        );
        let serialized = serde_json::to_string(&snapshot).expect("serialize snapshot");
        assert!(!serialized.contains("github_pat_"));
        assert!(!serialized.contains("ghu_"));
        assert!(!serialized.contains("gho_"));
        assert!(!serialized.contains("accessToken"));
    }

    #[test]
    fn copilot_reasoning_uses_native_capabilities_instead_of_model_name() {
        let mut model: github_copilot_sdk::rpc::Model = serde_json::from_value(json!({
            "id":"gemini-4-argon", "name":"Gemini 4 Argon", "capabilities":{"supports":{"reasoningEffort":true}},
            "supportedReasoningEfforts":["low","high"]
        })).unwrap();
        assert_eq!(copilot_reasoning_efforts(&model), vec!["low", "high"]);
        model
            .capabilities
            .supports
            .as_mut()
            .unwrap()
            .reasoning_effort = Some(false);
        assert!(copilot_reasoning_efforts(&model).is_empty());
        let summary = copilot_model_summary(model);
        assert_eq!(summary.supports_reasoning_effort, Some(false));
        assert!(summary.reasoning_efforts.is_empty());
    }
}
