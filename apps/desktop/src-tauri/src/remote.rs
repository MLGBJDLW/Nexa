//! The paired phone surface delegates to canonical desktop services and policy.
use crate::commands::{
    self,
    live::{self, LiveState},
    AppState, ApprovalState,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use nexa_remote::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Manager, State};

#[derive(Default)]
pub struct RemoteState {
    runtime: Mutex<Option<RemoteRuntime>>,
    lifecycle: tokio::sync::Mutex<()>,
    preparing: Mutex<Option<Arc<tokio::sync::Notify>>>,
    revision: std::sync::atomic::AtomicU64,
}
struct RemoteRuntime {
    server: Arc<RemoteServer>,
    _listeners: Vec<RunningListener>,
    tunnel: Option<tunnel::ManagedTunnel>,
    certificate_path: Option<PathBuf>,
    warning: Option<String>,
}
impl Drop for RemoteRuntime {
    fn drop(&mut self) {
        self.server.shutdown.cancel();
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteOptions {
    pub lan: bool,
    pub quick_tunnel: bool,
    pub public_url: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    enabled: bool,
    preparing: bool,
    connected_device_ids: Vec<String>,
    pairing_device_id: Option<String>,
    manifest: Option<ConnectionManifest>,
    devices: Vec<Device>,
    certificate_path: Option<String>,
    warning: Option<String>,
    tunnel_running: bool,
}

impl RemoteState {
    pub fn publish<T: Serialize + ?Sized>(&self, event: &str, payload: &T) {
        if event != "agent://run-event" {
            return;
        }
        let server = self
            .runtime
            .lock()
            .ok()
            .and_then(|state| state.as_ref().map(|runtime| runtime.server.clone()));
        if let Some(server) = server {
            if !server.has_subscribers() {
                return;
            }
            if let Ok(mut value) = serde_json::to_value(payload) {
                if let Some(run_event) = value.get_mut("runEvent") {
                    *run_event = project_run_event(run_event.take());
                }
                server.publish(None, event, value);
            }
        }
    }
    pub fn publish_live<T: Serialize + ?Sized>(&self, owner: &str, event: &T) {
        if let Some(server) = self
            .runtime
            .lock()
            .ok()
            .and_then(|state| state.as_ref().map(|runtime| runtime.server.clone()))
        {
            if let Ok(value) = serde_json::to_value(event) {
                server.publish(Some(owner), "live:event", value);
            }
        }
    }
    pub fn shutdown(&self) {
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(cancel) = self
            .preparing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            cancel.notify_one();
        }
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.take();
        }
    }
    fn server(&self) -> Result<Arc<RemoteServer>, String> {
        self.runtime
            .lock()
            .map_err(|_| "Remote state is unavailable")?
            .as_ref()
            .map(|runtime| runtime.server.clone())
            .ok_or("Remote access is not enabled".into())
    }
    fn status(&self) -> RemoteStatus {
        let preparing = self
            .preparing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let state = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        match state.as_ref() {
            None => RemoteStatus {
                enabled: false,
                preparing,
                connected_device_ids: vec![],
                pairing_device_id: None,
                manifest: None,
                devices: vec![],
                certificate_path: None,
                warning: None,
                tunnel_running: false,
            },
            Some(runtime) => RemoteStatus {
                enabled: true,
                preparing,
                connected_device_ids: runtime.server.connected_device_ids(),
                pairing_device_id: runtime.server.pairing_device_id(),
                manifest: Some(runtime.server.manifest()),
                devices: runtime.server.devices(),
                certificate_path: runtime
                    .certificate_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                warning: if runtime
                    .tunnel
                    .as_ref()
                    .is_some_and(|tunnel| !tunnel.is_running())
                {
                    Some("The temporary public tunnel has stopped. Restart remote access to obtain a new QR code.".into())
                } else {
                    runtime.warning.clone()
                },
                tunnel_running: runtime
                    .tunnel
                    .as_ref()
                    .is_some_and(|tunnel| tunnel.is_running()),
            },
        }
    }
}
#[tauri::command]
pub fn remote_status_cmd(state: State<'_, RemoteState>) -> RemoteStatus {
    state.status()
}
#[tauri::command]
pub async fn start_remote_cmd(
    app: AppHandle,
    state: State<'_, RemoteState>,
    options: RemoteOptions,
) -> Result<RemoteStatus, String> {
    let revision = state.revision.load(std::sync::atomic::Ordering::SeqCst);
    let _gate = state.lifecycle.lock().await;
    if revision != state.revision.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("Remote setup was cancelled".into());
    }
    if state
        .runtime
        .lock()
        .map_err(|_| "Remote state is unavailable")?
        .is_some()
    {
        return Ok(state.status());
    }
    let cancel = Arc::new(tokio::sync::Notify::new());
    *state.preparing.lock().unwrap_or_else(|e| e.into_inner()) = Some(cancel.clone());
    crate::app_events::emit_main_window_event(&app, "remote:status-changed", &());
    let result = tokio::select! { biased; _ = cancel.notified() => Err("Remote setup was cancelled".to_string()), result = build_remote_runtime(&app, options) => result };
    state
        .preparing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    let outcome = match result {
        Ok(runtime) => {
            *state.runtime.lock().unwrap_or_else(|e| e.into_inner()) = Some(runtime);
            Ok(state.status())
        }
        Err(error) => Err(error),
    };
    crate::app_events::emit_main_window_event(&app, "remote:status-changed", &());
    outcome
}

async fn build_remote_runtime(
    app: &AppHandle,
    options: RemoteOptions,
) -> Result<RemoteRuntime, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("remote");
    let auth_path = directory.clone();
    let auth = tokio::task::spawn_blocking(move || AuthStore::open(&auth_path))
        .await
        .map_err(|e| e.to_string())??;
    let server = RemoteServer::new(auth, Arc::new(DesktopHost { app: app.clone() }));
    let loopback = server.listen(([127, 0, 0, 1], 8790).into(), None).await?;
    let loopback_url = format!("http://{}", loopback.address);
    let mut runtime = RemoteRuntime {
        server: server.clone(),
        _listeners: vec![loopback],
        tunnel: None,
        certificate_path: None,
        warning: None,
    };
    if options.lan {
        let addresses = tls::lan_addresses()?;
        if addresses.is_empty() {
            runtime.warning = Some(
                "No LAN interface is available. Public or SSH access can still be used.".into(),
            );
        } else {
            let identity = tls::identity(&directory, &addresses).await?;
            runtime.certificate_path = Some(identity.certificate_path);
            for address in addresses {
                match server
                    .listen((address, 8791).into(), Some(identity.config.clone()))
                    .await
                {
                    Ok(listener) => runtime._listeners.push(listener),
                    Err(error) => runtime.warning = Some(error),
                }
            }
        }
    }
    if let Some(url) = options.public_url.filter(|url| !url.trim().is_empty()) {
        server.add_endpoint(Endpoint {
            url: url.trim().trim_end_matches('/').into(),
            kind: EndpointKind::Tunnel,
        })?;
    }
    if options.quick_tunnel {
        match tunnel::start(&directory.join("helper"), &loopback_url).await {
            Ok(tunnel) => {
                server.add_endpoint(Endpoint {
                    url: tunnel.url.clone(),
                    kind: EndpointKind::Tunnel,
                })?;
                runtime.tunnel = Some(tunnel);
            }
            Err(error) => runtime.warning = Some(error),
        }
    }
    Ok(runtime)
}
#[tauri::command]
pub async fn stop_remote_cmd(app: AppHandle, state: State<'_, RemoteState>) -> Result<(), String> {
    state
        .revision
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if let Some(cancel) = state
        .preparing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        cancel.notify_one();
    }
    let _gate = state.lifecycle.lock().await;
    let runtime = state
        .runtime
        .lock()
        .map_err(|_| "Remote state is unavailable")?
        .take();
    if let Some(runtime) = runtime {
        runtime.server.stop().await;
    }
    crate::app_events::emit_main_window_event(&app, "remote:status-changed", &());
    Ok(())
}
#[tauri::command]
pub async fn revoke_remote_device_cmd(
    app: AppHandle,
    state: State<'_, RemoteState>,
    device_id: String,
) -> Result<(), String> {
    state.server()?.revoke(&device_id).await?;
    crate::app_events::emit_main_window_event(&app, "remote:status-changed", &());
    Ok(())
}
#[tauri::command]
pub fn remote_pairing_cmd(app: AppHandle, state: State<'_, RemoteState>) -> Result<Value, String> {
    let server = state.server()?;
    let pairing = server.pairing();
    let manifest = server.manifest();
    let endpoint = manifest
        .endpoints
        .iter()
        .find(|endpoint| endpoint.kind == EndpointKind::Tunnel)
        .or_else(|| {
            manifest
                .endpoints
                .iter()
                .find(|endpoint| endpoint.kind == EndpointKind::Lan)
        })
        .or_else(|| manifest.endpoints.first())
        .ok_or("No remote address is ready")?;
    let url = format!(
        "{}/#pair={}&server={}",
        endpoint.url, pairing.code, manifest.server_id
    );
    let svg = nexa_remote::pairing_qr(&url)?;
    crate::app_events::emit_main_window_event(&app, "remote:status-changed", &());
    Ok(json!({"pairing":pairing,"url":url,"qrSvg":svg}))
}

struct DesktopHost {
    app: AppHandle,
}
fn value<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}
fn project_run_event(mut event: Value) -> Value {
    let original = event
        .get_mut("payload")
        .map(Value::take)
        .unwrap_or(Value::Null);
    let fields = match event["kind"].as_str().unwrap_or_default() {
        "outputDelta" => &["blockId", "channel", "offset", "delta"][..],
        "outputSnapshot" => &["blockId", "channel", "text"][..],
        "streamReset" => &["reason", "discardSample"][..],
        _ => &[][..],
    };
    let mut payload = serde_json::Map::new();
    for key in fields {
        if let Some(value) = original.get(key) {
            let value = if let Some(text) = value.as_str() {
                Value::String(text.chars().take(128_000).collect())
            } else {
                value.clone()
            };
            payload.insert((*key).into(), value);
        }
    }
    event["payload"] = Value::Object(payload);
    event
}
#[async_trait::async_trait]
impl RemoteHost for DesktopHost {
    async fn execute(&self, owner: &str, command: RemoteCommand) -> Result<Value, String> {
        let app = &self.app;
        let state = app.state::<AppState>();
        use RemoteCommand::*;
        match command {
            Connections => state.db_executor.read(|db| {
                Ok(db.list_agent_configs()?.into_iter().map(|config| {
                    json!({"id":config.id,"name":config.name,"model":config.model,"isDefault":config.is_default})
                }).collect::<Vec<_>>())
            }).await.map_err(|e| e.to_string()).and_then(|result| value(result.value)),
            Conversations { before } => state.db_executor
                .read(move |db| db.remote_conversations(before.as_deref()))
                .await.map(|result| result.value).map_err(|e| e.to_string()),
            Conversation { conversation_id, before_order } => state.db_executor
                .read(move |db| db.remote_messages(&conversation_id, before_order))
                .await.map(|result| result.value).map_err(|e| e.to_string()),
            Message { conversation_id, message_id, offset } => state.db_executor
                .read(move |db| db.remote_message_text(&conversation_id, &message_id, offset))
                .await.map(|result| result.value).map_err(|e| e.to_string()),
            CreateConversation { connection_id } => state.db_executor.write(move |db| {
                let config = db.get_agent_config(&connection_id)?;
                let conversation = db.create_conversation(&nexa_core::conversation::CreateConversationInput {
                    provider: config.provider, model: config.model, system_prompt: None,
                    collection_context: None, project_id: None, persona_id: None,
                })?;
                Ok(json!({"id":conversation.id,"title":conversation.title,"model":conversation.model}))
            }).await.map(|result| result.value).map_err(|e| e.to_string()),
            StartTurn { conversation_id, connection_id, message, idempotency_key } => {
                let request = serde_json::from_value(json!({
                    "conversationId":conversation_id,"agentConfigId":connection_id,
                    "message":message,"idempotencyKey":idempotency_key
                })).map_err(|e| e.to_string())?;
                value(commands::agent_chat_cmd(
                    app.state(), app.state(), app.state(), app.state(), app.state(), app.state(),
                    app.clone(), request,
                ).await?)
            }
            StopTurn { conversation_id } => {
                commands::agent_stop_cmd(app.state(), app.state(), app.state(), app.clone(), conversation_id).await?;
                Ok(Value::Null)
            }
            Resume { conversation_id, run_id, after_sequence } => state.db_executor.read(move |db| {
                let id = db.remote_latest_run(&conversation_id)?.or_else(|| run_id.clone());
                let Some(id) = id else {
                    return Ok(json!({"run":null,"events":[],"hasMore":false}));
                };
                let run = db.get_agent_task_run(&id)?;
                if run.conversation_id != conversation_id {
                    return Err(nexa_core::error::CoreError::NotFound("Run does not belong to this conversation".into()));
                }
                let after = if run_id.as_deref() == Some(id.as_str()) { after_sequence.unwrap_or(0) } else { 0 };
                let page = db.list_agent_run_event_page(&id, after, None, 128)?;
                let events = page.events.into_iter()
                    .map(|event| serde_json::to_value(event).map(project_run_event))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!({
                    "run":{"id":run.id,"status":run.status,"turnId":run.turn_id},
                    "events":events,"hasMore":page.has_more,"nextSequence":page.next_after_event_seq
                }))
            }).await.map(|result| result.value).map_err(|e| e.to_string()),
            Interactions { conversation_id } => state.db_executor
                .read(move |db| db.list_interaction_requests(conversation_id.as_deref(), false))
                .await.map_err(|e| e.to_string()).and_then(|result| value(result.value)),
            RespondInteraction { input } => {
                let input = serde_json::from_value(input).map_err(|e| e.to_string())?;
                state.db_executor.write(move |db| db.submit_interaction_response(&input))
                    .await.map_err(|e| e.to_string()).and_then(|result| value(result.value))
            }
            Approvals => {
                let approvals = app.state::<ApprovalState>();
                let pending = approvals.pending.lock().await;
                value(pending.values().map(|entry| {
                    json!({"runId":entry.task_run_id,"request":entry.request})
                }).collect::<Vec<_>>())
            }
            RespondApproval { request_id, decision } => {
                commands::approve_tool_call_cmd(app.state(), request_id, decision).await?;
                Ok(Value::Null)
            }
            LiveConnections => value(live::connections(app).await?),
            LiveStart { request } => {
                let request = serde_json::from_value(request).map_err(|e| e.to_string())?;
                value(live::start(app, owner, request).await?)
            }
            LiveSnapshot { session_id } => value(app.state::<LiveState>().manager.snapshot(owner, &session_id)?),
            LiveFrame { session_id, mime_type, data } => {
                app.state::<LiveState>().manager.frame(owner, &session_id, &mime_type, &data)?;
                Ok(Value::Null)
            }
            LiveAudio { session_id, data } => {
                let audio = B64.decode(data).map_err(|_| "Invalid audio encoding")?;
                live::audio(app, owner, &session_id, audio).await?;
                Ok(Value::Null)
            }
            LiveStop { session_id } => value(live::stop(app, owner, &session_id).await?),
            LiveSummarize { session_id, connection_id } => value(live::summarize(app, owner, &session_id, &connection_id).await?),
            LiveRecords => {
                let owner = owner.to_owned();
                state.db_executor.read(move |db| db.list_live_records(&owner))
                    .await.map_err(|e| e.to_string()).and_then(|result| value(result.value))
            }
            LiveRecord { session_id } => {
                let owner = owner.to_owned();
                state.db_executor.read(move |db| db.load_live_record(&owner, &session_id))
                    .await.map_err(|e| e.to_string()).and_then(|result| value(result.value))
            }
            Certificate => {
                let path = app.state::<RemoteState>().runtime.lock()
                    .map_err(|_| "Remote state is unavailable")?.as_ref()
                    .and_then(|runtime| runtime.certificate_path.clone())
                    .ok_or("LAN TLS is not enabled")?;
                value(tokio::fs::read_to_string(path).await.map_err(|e| e.to_string())?)
            }
        }
    }
    async fn disconnected(&self, owner: &str) {
        self.app.state::<LiveState>().manager.stop_owner(owner);
    }
    async fn connection_changed(&self, owner: &str, connected: bool) {
        self.app
            .state::<LiveState>()
            .manager
            .connection_notice(owner, connected);
        crate::app_events::emit_main_window_event(&self.app, "remote:status-changed", &());
    }
    fn asset(&self, path: &str) -> Option<WebAsset> {
        self.app
            .asset_resolver()
            .get(path.to_string())
            .map(|asset| WebAsset {
                bytes: asset.bytes,
                mime_type: asset.mime_type,
            })
    }
}
