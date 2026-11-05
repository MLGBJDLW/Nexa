//! Native transport for the existing desktop protocol. Tokens never cross IPC.
use crate::platform;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};
use tokio_util::sync::CancellationToken;

// Serialize read-modify-write updates to the encrypted host collection.
static HOST_STORE: Mutex<()> = Mutex::const_new(());

#[derive(Default)]
pub struct RemoteHosts {
    connections: Mutex<HashMap<String, (String, CancellationToken)>>,
}

pub fn origin(input: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(input)
        .map_err(|_| "Enter the HTTPS pairing URL from your computer.")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err("Use an HTTPS endpoint with a trusted certificate.".into());
    }
    Ok(url.origin().ascii_serialization())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}
fn request(
    client: &reqwest::Client,
    method: reqwest::Method,
    endpoint: &str,
    path: &str,
) -> reqwest::RequestBuilder {
    client
        .request(method, format!("{endpoint}{path}"))
        .header("Origin", endpoint)
        .header("X-Pinggy-No-Screen", "1")
}

async fn response(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|size| size > 32 * 1024 * 1024)
    {
        return Err("The response is too large to display on this phone.".into());
    }
    let mut bytes = Vec::new();
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|_| "The connection was interrupted.")?;
        if bytes.len() + chunk.len() > 32 * 1024 * 1024 {
            return Err("The response is too large to display on this phone.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| format!("The endpoint did not return Nexa data ({status})."))?;
    if !status.is_success() {
        return Err(value["error"]
            .as_str()
            .unwrap_or("The computer rejected this request.")
            .into());
    }
    Ok(value)
}

fn public_host(saved: &Value) -> Value {
    json!({"id":saved["manifest"]["serverId"],"token":"","device":saved["device"],"manifest":saved["manifest"],"endpoint":saved["endpoint"]})
}

pub async fn configure_phone_control(
    app: &AppHandle,
    id: &str,
    enabled: bool,
) -> Result<(), String> {
    let saved = host(app, id).await?;
    if enabled
        && !saved["manifest"]["capabilities"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item == "phone-control-v1"))
    {
        return Err("Update the paired computer to a version supporting phone control.".into());
    }
    platform::call(
        app,
        "remoteControl",
        json!({"action":if enabled {"enable"} else {"disable"},"hostId":id}),
    )
    .await?;
    Ok(())
}

async fn phone_control_enabled(app: &AppHandle, id: &str) -> bool {
    tokio::time::timeout(
        Duration::from_secs(2),
        platform::call(app, "remoteControl", json!({"action":"status","hostId":id})),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .is_some_and(|value| value["enabled"] == true)
}

struct PhoneRequestGuard {
    app: AppHandle,
    host: String,
    id: String,
    active: bool,
}
impl Drop for PhoneRequestGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let app = self.app.clone();
        let host = self.host.clone();
        let id = self.id.clone();
        tauri::async_runtime::spawn(async move {
            let _ = platform::call(
                &app,
                "remoteControl",
                json!({"action":"cancel","hostId":host,"requestId":id}),
            )
            .await;
        });
    }
}

pub async fn hosts(app: &AppHandle) -> Result<Vec<Value>, String> {
    let saved = platform::read_secret(app, "remote-hosts")
        .await?
        .unwrap_or(json!([]));
    Ok(saved
        .as_array()
        .ok_or("Invalid saved host list.")?
        .iter()
        .map(public_host)
        .collect())
}

async fn host(app: &AppHandle, id: &str) -> Result<Value, String> {
    let saved = platform::read_secret(app, "remote-hosts")
        .await?
        .unwrap_or(json!([]));
    saved
        .as_array()
        .and_then(|list| list.iter().find(|host| host["manifest"]["serverId"] == id))
        .cloned()
        .ok_or("Pair this computer first.".into())
}

pub(crate) async fn supports_model_sync(app: &AppHandle, id: &str) -> Result<bool, String> {
    let saved = host(app, id).await?;
    Ok([
        &saved["manifest"]["capabilities"],
        &saved["device"]["capabilities"],
    ]
    .iter()
    .all(|capabilities| {
        capabilities
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item == "model-connection-sync-v1"))
    }))
}

pub(crate) async fn paired_guard(
    app: &AppHandle,
    id: &str,
) -> Result<tokio::sync::MutexGuard<'static, ()>, String> {
    let guard = HOST_STORE.lock().await;
    host(app, id).await?;
    Ok(guard)
}

pub async fn pair(
    app: &AppHandle,
    pairing_url: String,
    name: String,
    client_nonce: String,
) -> Result<Value, String> {
    uuid::Uuid::parse_str(&client_nonce).map_err(|_| "A stable pairing request ID is required.")?;
    let url = reqwest::Url::parse(&pairing_url).map_err(|_| "Invalid pairing URL.")?;
    let fields: HashMap<_, _> = reqwest::Url::parse(&format!(
        "https://pair.invalid/?{}",
        url.fragment().unwrap_or("")
    ))
    .map_err(|_| "Invalid pairing link.")?
    .query_pairs()
    .into_owned()
    .collect();
    let code = fields
        .get("pair")
        .ok_or("Paste the complete pairing link, including its pairing code.")?;
    let expected = fields
        .get("server")
        .ok_or("The pairing link needs its computer identity.")?;
    let endpoint = origin(&pairing_url)?;
    let http = client()?;
    let health = response(
        request(&http, reqwest::Method::GET, &endpoint, "/api/health")
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| "The computer is unreachable or its certificate is not trusted.")?,
    )
    .await?;
    if health["version"] != 1 || health["serverId"].as_str() != Some(expected) {
        return Err("This pairing link points to a different or incompatible computer.".into());
    }
    let mut request_body = json!({"code":code,"name":name,"clientNonce":client_nonce});
    // Older hosts reject unknown fields, and existing pairings retain their original grants.
    if health["capabilities"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item == "model-connection-sync-v1"))
    {
        request_body["capabilities"] = json!(["model-connection-sync-v1"]);
    }
    let mut paired = response(
        request(&http, reqwest::Method::POST, &endpoint, "/api/pair")
            .json(&request_body)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|_| "Pairing was interrupted. Retry the same link.")?,
    )
    .await?;
    if paired["manifest"]["serverId"].as_str() != Some(expected)
        || paired["token"].as_str().is_none()
    {
        return Err("The computer identity changed during pairing.".into());
    }
    paired["endpoint"] = json!(endpoint);
    let _guard = HOST_STORE.lock().await;
    let mut saved = platform::read_secret(app, "remote-hosts")
        .await?
        .unwrap_or(json!([]));
    let list = saved.as_array_mut().ok_or("Invalid saved host list.")?;
    list.retain(|item| item["manifest"]["serverId"].as_str() != Some(expected));
    list.push(paired.clone());
    platform::save_secret(app, "remote-hosts", Some(&saved)).await?;
    Ok(public_host(&paired))
}

pub async fn forget(app: &AppHandle, id: &str) -> Result<(), String> {
    let _ = platform::call(
        app,
        "remoteControl",
        json!({"action":"disable","hostId":id}),
    )
    .await;
    let _guard = HOST_STORE.lock().await;
    let mut saved = platform::read_secret(app, "remote-hosts")
        .await?
        .unwrap_or(json!([]));
    saved
        .as_array_mut()
        .ok_or("Invalid saved host list.")?
        .retain(|item| item["manifest"]["serverId"] != id);
    platform::save_secret(app, "remote-hosts", Some(&saved)).await
}

fn apply_manifest(saved: &mut Value, manifest: &Value) -> Result<bool, String> {
    if manifest["serverId"] != saved["manifest"]["serverId"] {
        return Err("The computer identity changed.".into());
    }
    let routes = manifest["endpoints"]
        .as_array()
        .ok_or("Invalid computer routes.")?;
    let endpoints: Vec<Value> = routes
        .iter()
        .filter(|route| route["url"].as_str().is_some_and(|url| origin(url).is_ok()))
        .cloned()
        .collect();
    let current_retained = endpoints
        .iter()
        .any(|route| route["url"] == saved["endpoint"]);
    saved["manifest"] = manifest.clone();
    saved["manifest"]["endpoints"] = json!(endpoints);
    if !current_retained {
        saved["endpoint"] = endpoints
            .first()
            .map(|route| route["url"].clone())
            .unwrap_or(Value::Null);
    }
    Ok(!current_retained)
}

async fn update_manifest(app: &AppHandle, id: &str, manifest: &Value) -> Result<(), String> {
    let _guard = HOST_STORE.lock().await;
    let mut saved = platform::read_secret(app, "remote-hosts")
        .await?
        .unwrap_or(json!([]));
    let item = saved
        .as_array_mut()
        .and_then(|list| {
            list.iter_mut()
                .find(|item| item["manifest"]["serverId"] == id)
        })
        .ok_or("This computer is no longer paired.")?;
    apply_manifest(item, manifest)?;
    platform::save_secret(app, "remote-hosts", Some(&saved)).await
}

async fn reachable(http: &reqwest::Client, saved: &Value) -> Result<String, String> {
    let mut endpoints = vec![saved["endpoint"].as_str().unwrap_or_default().to_string()];
    if let Some(routes) = saved["manifest"]["endpoints"].as_array() {
        endpoints.extend(
            routes
                .iter()
                .filter_map(|route| route["url"].as_str().map(str::to_owned)),
        );
    }
    endpoints.dedup();
    for endpoint in endpoints {
        let Ok(endpoint) = origin(&endpoint) else {
            continue;
        };
        if let Ok(reply) = request(http, reqwest::Method::GET, &endpoint, "/api/health")
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            if let Ok(health) = response(reply).await {
                if health["version"] == 1 && health["serverId"] == saved["manifest"]["serverId"] {
                    return Ok(endpoint);
                }
            }
        }
    }
    Err(
        "The paired computer is unreachable. Check its remote access and network connection."
            .into(),
    )
}

pub async fn rpc(
    app: &AppHandle,
    id: &str,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    // The server retains its typed command allowlist; this bridge has no desktop
    // IPC dispatch and sends only to a freshly identity-checked paired host.
    let saved = host(app, id).await?;
    let http = client()?;
    let endpoint = reachable(&http, &saved).await?;
    let reply = request(&http, reqwest::Method::POST, &endpoint, "/api/rpc")
        .bearer_auth(
            saved["token"]
                .as_str()
                .ok_or("Pairing token unavailable.")?,
        )
        .json(&json!({"method":method,"params":params}))
        .timeout(Duration::from_secs(if method == "device.control.result" {
            15
        } else if method == "sync.connections" {
            30
        } else {
            145
        }))
        .send()
        .await
        .map_err(|_| "The connection changed. Retry to recover the task.")?;
    if reply.status() == reqwest::StatusCode::UNAUTHORIZED {
        emit_revoked(app, id);
    }
    let result = response(reply).await?;
    Ok(result["result"].clone())
}

fn emit_revoked(app: &AppHandle, id: &str) {
    let control_app = app.clone();
    let control_id = id.to_owned();
    tauri::async_runtime::spawn(async move {
        let _ = platform::call(
            &control_app,
            "remoteControl",
            json!({"action":"disable","hostId":control_id}),
        )
        .await;
    });
    let _ = app.emit("mobile:remote", json!({"hostId":id,"event":"connection:state","payload":{"phase":"revoked","endpoint":null}}));
}

impl RemoteHosts {
    pub async fn close(&self, id: &str, connection_id: Option<&str>) {
        let mut connections = self.connections.lock().await;
        if connections
            .get(id)
            .is_some_and(|(owner, _)| connection_id.is_none_or(|id| id == owner))
        {
            if let Some((_, cancel)) = connections.remove(id) {
                cancel.cancel();
            }
        }
    }
    pub async fn connect(
        self: &Arc<Self>,
        app: AppHandle,
        id: String,
        connection_id: String,
    ) -> Result<(), String> {
        let mut connections = self.connections.lock().await;
        let mut saved = host(&app, &id).await?;
        let cancel = CancellationToken::new();
        if let Some((_, previous)) = connections.insert(id.clone(), (connection_id, cancel.clone()))
        {
            previous.cancel();
        }
        tokio::spawn(async move {
            let Ok(http) = client() else { return };
            let mut failures = 0u32;
            loop {
                let _ = app.emit("mobile:remote", json!({"hostId":id,"event":"connection:state","payload":{"phase":"reconnecting","endpoint":null}}));
                let attempt = async {
                    let endpoint = reachable(&http, &saved).await?;
                    let mut request =
                        format!("{}/api/events", endpoint.replacen("https:", "wss:", 1))
                            .into_client_request()
                            .map_err(|e| e.to_string())?;
                    request.headers_mut().insert(
                        "Origin",
                        HeaderValue::from_str(&endpoint).map_err(|e| e.to_string())?,
                    );
                    request
                        .headers_mut()
                        .insert("X-Pinggy-No-Screen", HeaderValue::from_static("1"));
                    let (mut socket, _) =
                        tokio::time::timeout(Duration::from_secs(15), connect_async(request))
                            .await
                            .map_err(|_| "Connection timed out.")?
                            .map_err(|e| e.to_string())?;
                    socket
                        .send(Message::Text(
                            json!({"token":saved["token"]}).to_string().into(),
                        ))
                        .await
                        .map_err(|e| e.to_string())?;
                    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
                    let mut last_message = tokio::time::Instant::now();
                    let mut control_session = String::new();
                    let mut control_advertised = None;
                    let mut control_calls = tokio::task::JoinSet::new();
                    let mut seen_control_ids = std::collections::VecDeque::<String>::new();
                    loop {
                        tokio::select! {
                            _ = cancel.cancelled() => { let _ = socket.close(None).await; return Ok::<bool,String>(false); },
                            _ = heartbeat.tick() => {
                                if last_message.elapsed() > Duration::from_secs(20) { return Err("Connection heartbeat expired.".into()); }
                                socket.send(Message::Text("ping".into())).await.map_err(|e| e.to_string())?;
                                if !control_session.is_empty() {
                                    let enabled=phone_control_enabled(&app,&id).await;
                                    if control_advertised != Some(enabled) {
                                        socket.send(Message::Text(json!({"type":"phoneControl","version":1,"enabled":enabled}).to_string().into())).await.map_err(|e|e.to_string())?;
                                        control_advertised=Some(enabled);
                                    }
                                }
                            },
                            _ = control_calls.join_next(), if !control_calls.is_empty() => {},
                            message = socket.next() => {
                                let message = message.ok_or("Connection closed.")?.map_err(|e| e.to_string())?;
                                last_message = tokio::time::Instant::now();
                                if let Message::Close(Some(frame)) = &message {
                                    if frame.code == tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy {
                                        emit_revoked(&app, &id);
                                        return Ok(false);
                                    }
                                }
                                if let Message::Text(text) = message {
                                    if text.len() > 2 * 1024 * 1024 { return Err("Remote event exceeds the display limit.".into()); }
                                    let Ok(event) = serde_json::from_str::<Value>(&text) else { continue };
                                    let ready = event["event"] == "connection:ready";
                                    if ready || event["event"] == "connection:manifest" {
                                        let manifest = if ready { &event["payload"]["manifest"] } else { &event["payload"] };
                                        saved["endpoint"] = json!(endpoint);
                                        let reconnect = apply_manifest(&mut saved, manifest)?;
                                        update_manifest(&app, &id, manifest).await?;
                                        if reconnect && !ready { return Err("The computer routes changed.".into()); }
                                    }
                                    if ready {
                                        if saved["manifest"]["capabilities"].as_array().is_some_and(|items|items.iter().any(|item|item == "phone-control-v1")) {
                                            control_session=event["payload"]["controlSessionId"].as_str().unwrap_or_default().to_owned();
                                            let enabled=phone_control_enabled(&app,&id).await;
                                            socket.send(Message::Text(json!({"type":"phoneControl","version":1,"enabled":enabled}).to_string().into())).await.map_err(|e|e.to_string())?;
                                            control_advertised=Some(enabled);
                                        }
                                        failures = 0;
                                        let _ = app.emit("mobile:remote", json!({"hostId":id,"event":"connection:state","payload":{"phase":"connected","endpoint":{"url":endpoint,"kind":"tunnel"}}}));
                                    }
                                    if event["event"] == "connection:revoked" {
                                        emit_revoked(&app, &id);
                                        return Ok(false);
                                    }
                                    if event["event"] == "device:control-cancel" {
                                        if let Some(request_id)=event["payload"]["id"].as_str() {
                                            let _=platform::call(&app,"remoteControl",json!({"action":"cancel","hostId":id,"requestId":request_id})).await;
                                        }
                                        continue;
                                    }
                                    if event["event"] == "device:control" && !control_session.is_empty() {
                                        // The broker admits one operation per phone. Do not let
                                        // a malformed peer accumulate native work or result tasks.
                                        // A completed native call can still be delivering its
                                        // HTTP acknowledgement when the next request arrives.
                                        // Permit that overlap; native execution remains serialized.
                                        if control_calls.len() >= 2 { return Err("The phone control channel is overloaded.".into()); }
                                        let Some(request_id)=event["payload"]["id"].as_str().filter(|value|uuid::Uuid::parse_str(value).is_ok()) else {continue};
                                        if seen_control_ids.iter().any(|id|id == request_id) { continue; }
                                        seen_control_ids.push_back(request_id.to_owned()); if seen_control_ids.len()>256 {seen_control_ids.pop_front();}
                                        let arguments=event["payload"]["arguments"].clone();
                                        if !arguments.is_object() {continue;}
                                        let control_app=app.clone(); let host=id.clone(); let request_id=request_id.to_owned(); let session_id=control_session.clone();
                                        control_calls.spawn(async move {
                                            let mut guard=PhoneRequestGuard{app:control_app.clone(),host:host.clone(),id:request_id.clone(),active:true};
                                            let reply=tokio::time::timeout(Duration::from_secs(25),platform::call(&control_app,"remoteControl",json!({"action":"execute","hostId":host,"requestId":request_id,"arguments":arguments}))).await;
                                            let result=match reply {
                                                Ok(Ok(value))=>{guard.active=false;value},
                                                Ok(Err(error))=>{guard.active=false;json!({"code":"phone_control_unavailable","retryable":false,"message":error})},
                                                Err(_)=>json!({"code":"phone_control_outcome_unknown","retryable":false,"message":"The native phone operation did not return. Observe before continuing; never repeat an uncertain send."}),
                                            };
                                            drop(guard);
                                            let _=rpc(&control_app,&host,"device.control.result".into(),Some(json!({"requestId":request_id,"sessionId":session_id,"result":result}))).await;
                                        });
                                        continue;
                                    }
                                    let _ = app.emit("mobile:remote", json!({"hostId":id,"event":event["event"],"payload":event["payload"]}));
                                }
                            }
                        }
                    }
                };
                tokio::select! { _ = cancel.cancelled() => break, result = attempt => { if matches!(result, Ok(false)) { break; } } }
                failures = failures.saturating_add(1);
                tokio::select! { _ = cancel.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(1u64 << failures.min(3))) => {} }
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotated_routes_replace_stale_endpoint_without_changing_identity() {
        let mut saved = json!({"endpoint":"https://old.example","manifest":{"serverId":"pc"}});
        let next = json!({"serverId":"pc","endpoints":[{"url":"http://unsafe.example"},{"url":"https://new.example"}]});
        assert!(apply_manifest(&mut saved, &next).unwrap());
        assert_eq!(saved["endpoint"], "https://new.example");
        assert_eq!(saved["manifest"]["endpoints"].as_array().unwrap().len(), 1);
        assert!(!apply_manifest(&mut saved, &next).unwrap());
        let before = saved.clone();
        assert!(apply_manifest(&mut saved, &json!({"serverId":"other","endpoints":[]})).is_err());
        assert_eq!(before, saved);
    }
}
