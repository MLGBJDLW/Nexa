use crate::{
    AuthStore, ConnectionManifest, Device, Endpoint, EndpointKind, PairingCode, RemoteEvent,
    RemoteHost, RpcRequest,
};
use axum::{
    body::Body,
    extract::{
        ws::{close_code, CloseFrame, Message, WebSocket},
        DefaultBodyLimit, Request, State, WebSocketUpgrade,
    },
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::sync::{broadcast, Semaphore};
use tokio_util::sync::CancellationToken;

const MAX_BODY: usize = 768 * 1024;
pub const RECONNECT_GRACE: Duration = Duration::from_secs(30);

pub struct RemoteServer {
    auth: Mutex<AuthStore>,
    endpoints: RwLock<Vec<Endpoint>>,
    host: Arc<dyn RemoteHost>,
    events: broadcast::Sender<RemoteEvent>,
    pub shutdown: CancellationToken,
    devices_changed: tokio::sync::Notify,
    connections: Mutex<HashMap<String, (usize, u64)>>,
    sockets: Arc<Semaphore>,
    requests: Arc<Semaphore>,
}
pub struct RunningListener {
    pub address: SocketAddr,
    pub task: tokio::task::JoinHandle<Result<(), String>>,
    handle: axum_server::Handle<SocketAddr>,
}
impl RunningListener {
    pub fn stop(&self) {
        self.handle.graceful_shutdown(Some(Duration::from_secs(1)));
    }
}
impl Drop for RunningListener {
    fn drop(&mut self) {
        self.handle.shutdown();
        self.task.abort();
    }
}

impl RemoteServer {
    pub fn new(auth: AuthStore, host: Arc<dyn RemoteHost>) -> Arc<Self> {
        let (events, _) = broadcast::channel(128);
        Arc::new(Self {
            auth: Mutex::new(auth),
            endpoints: RwLock::new(vec![]),
            host,
            events,
            shutdown: CancellationToken::new(),
            devices_changed: tokio::sync::Notify::new(),
            connections: Mutex::new(HashMap::new()),
            sockets: Arc::new(Semaphore::new(24)),
            requests: Arc::new(Semaphore::new(32)),
        })
    }
    pub fn server_id(&self) -> String {
        self.auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .server_id()
            .to_string()
    }
    pub fn devices(&self) -> Vec<Device> {
        self.auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .devices()
    }
    pub fn pairing(&self) -> PairingCode {
        self.auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .rotate_pairing()
    }
    pub fn pairing_device_id(&self) -> Option<String> {
        self.auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pairing_device_id()
    }
    pub fn connected_device_ids(&self) -> Vec<String> {
        self.connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, (count, _))| *count > 0)
            .map(|(id, _)| id.clone())
            .collect()
    }
    pub fn manifest(&self) -> ConnectionManifest {
        ConnectionManifest {
            server_id: self.server_id(),
            endpoints: self
                .endpoints
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            reconnect_grace_seconds: RECONNECT_GRACE.as_secs(),
        }
    }
    pub fn add_endpoint(&self, endpoint: Endpoint) -> Result<(), String> {
        validate_endpoint(&endpoint)?;
        let mut endpoints = self.endpoints.write().unwrap_or_else(|e| e.into_inner());
        if !endpoints.iter().any(|entry| entry.url == endpoint.url) {
            endpoints.push(endpoint);
        }
        drop(endpoints);
        self.publish(
            None,
            "connection:manifest",
            serde_json::to_value(self.manifest())
                .map_err(|_| "Unable to encode connection manifest")?,
        );
        Ok(())
    }
    pub fn publish(&self, owner: Option<&str>, event: &str, payload: Value) {
        if self.shutdown.is_cancelled() {
            return;
        }
        let _ = self.events.send(RemoteEvent {
            event: event.into(),
            payload,
            owner: owner.map(str::to_owned),
        });
    }
    pub fn has_subscribers(&self) -> bool {
        self.events.receiver_count() > 0
    }
    pub async fn revoke(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let server = self.clone();
        let device_id = id.to_string();
        tokio::task::spawn_blocking(move || {
            server
                .auth
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .revoke(&device_id)
        })
        .await
        .map_err(|_| "Device revocation failed")??;
        self.devices_changed.notify_waiters();
        self.connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.host.disconnected(id).await;
        Ok(())
    }
    pub async fn stop(&self) {
        self.shutdown.cancel();
        for device in self.devices() {
            self.host.disconnected(&device.id).await;
        }
    }
    pub fn router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/api/health", get(health))
            .route("/api/pair", post(pair))
            .route("/api/manifest", get(manifest))
            .route("/api/rpc", post(rpc))
            .route("/api/events", get(events))
            .fallback(get(asset))
            .layer(DefaultBodyLimit::max(MAX_BODY))
            .layer(tower_http::compression::CompressionLayer::new())
            .layer(middleware::from_fn_with_state(self.clone(), boundary))
            .with_state(self.clone())
    }
    pub async fn listen(
        self: &Arc<Self>,
        address: SocketAddr,
        tls: Option<axum_server::tls_rustls::RustlsConfig>,
    ) -> Result<RunningListener, String> {
        let socket = std::net::TcpListener::bind(address)
            .map_err(|e| format!("Unable to listen on {address}: {e}"))?;
        socket
            .set_nonblocking(true)
            .map_err(|_| "Unable to configure remote listener")?;
        let address = socket
            .local_addr()
            .map_err(|_| "Unable to read listener address")?;
        if tls.is_none() && !address.ip().is_loopback() {
            return Err("Plain HTTP remote access is restricted to loopback".into());
        }
        let endpoint = Endpoint {
            url: format!(
                "{}://{address}",
                if tls.is_some() { "https" } else { "http" }
            ),
            kind: if address.ip().is_loopback() {
                EndpointKind::Ssh
            } else {
                EndpointKind::Lan
            },
        };
        self.add_endpoint(endpoint)?;
        let app = self.router();
        let handle = axum_server::Handle::new();
        let server_handle = handle.clone();
        let task = tokio::spawn(async move {
            match tls {
                Some(config) => axum_server::from_tcp_rustls(socket, config)
                    .map_err(|e| e.to_string())?
                    .handle(server_handle)
                    .serve(app.into_make_service())
                    .await
                    .map_err(|e| e.to_string()),
                None => axum_server::from_tcp(socket)
                    .map_err(|e| e.to_string())?
                    .handle(server_handle)
                    .serve(app.into_make_service())
                    .await
                    .map_err(|e| e.to_string()),
            }
        });
        Ok(RunningListener {
            address,
            task,
            handle,
        })
    }
    fn authenticate(&self, headers: &HeaderMap) -> Option<Device> {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))?;
        self.auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .authenticate(token)
    }
    async fn revoked(&self, id: &str) {
        loop {
            let changed = self.devices_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self
                .auth
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .has_device(id)
            {
                return;
            }
            changed.await;
        }
    }
    fn allows_origin(&self, origin: &str) -> bool {
        self.endpoints
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|endpoint| endpoint.url == origin)
    }
    fn allows_host(&self, host: &str) -> bool {
        self.endpoints
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|endpoint| {
                url::Url::parse(&endpoint.url).is_ok_and(|url| {
                    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
                    authority.eq_ignore_ascii_case(host)
                        || (endpoint.kind == EndpointKind::Ssh
                            && host
                                == format!(
                                    "localhost:{}",
                                    url.port_or_known_default().unwrap_or(80)
                                ))
                })
            })
    }
}
fn failure(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"error":message.into()}))).into_response()
}
fn unauthorized() -> Response {
    failure(
        StatusCode::UNAUTHORIZED,
        "Device is not paired or has been revoked",
    )
}
pub fn validate_endpoint(endpoint: &Endpoint) -> Result<(), String> {
    let url = url::Url::parse(&endpoint.url).map_err(|_| "Invalid connection address")?;
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
        || endpoint.url.ends_with('/')
    {
        return Err(
            "Use a server origin without paths, credentials, query, or trailing slash".into(),
        );
    }
    if url.scheme() != "https"
        && !(endpoint.kind == EndpointKind::Ssh
            && url.scheme() == "http"
            && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")))
    {
        return Err("Use HTTPS, or a localhost SSH forwarding address".into());
    }
    Ok(())
}
async fn boundary(
    State(server): State<Arc<RemoteServer>>,
    request: Request,
    next: Next,
) -> Response {
    if server.shutdown.is_cancelled() {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "Remote access has stopped");
    }
    let Some(host) = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return failure(StatusCode::BAD_REQUEST, "Missing host");
    };
    if !server.allows_host(host) {
        return failure(StatusCode::FORBIDDEN, "Unknown remote host");
    }
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let immutable_asset = request.uri().path().starts_with("/assets/");
    if origin
        .as_deref()
        .is_some_and(|origin| !server.allows_origin(origin))
    {
        return failure(
            StatusCode::FORBIDDEN,
            "Origin is not an authorized Nexa address",
        );
    }
    if request.method() == Method::POST && origin.is_none() {
        return failure(
            StatusCode::FORBIDDEN,
            "An authorized browser origin is required",
        );
    }
    let Ok(_permit) = server.requests.clone().try_acquire_owned() else {
        return failure(
            StatusCode::TOO_MANY_REQUESTS,
            "Remote request queue is full",
        );
    };
    let mut response = if request.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    if let Some(origin) = origin.and_then(|value| HeaderValue::from_str(&value).ok()) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Authorization, Content-Type"),
        );
        headers.insert(
            "access-control-allow-private-network",
            HeaderValue::from_static("true"),
        );
        headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if immutable_asset {
            "public, max-age=31536000, immutable"
        } else {
            "no-store"
        }),
    );
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    let origins = server
        .endpoints
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .flat_map(|endpoint| [endpoint.url.clone(), endpoint.url.replacen("http", "ws", 1)])
        .collect::<Vec<_>>()
        .join(" ");
    if let Ok(csp)=HeaderValue::from_str(&format!("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self'; connect-src 'self' {origins}; worker-src 'self' blob:; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")) {headers.insert(header::CONTENT_SECURITY_POLICY,csp);}
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(self), microphone=(self), geolocation=()"),
    );
    response
}
async fn health(State(server): State<Arc<RemoteServer>>) -> Json<Value> {
    Json(json!({"serverId":server.server_id(),"version":1}))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PairRequest {
    code: String,
    name: String,
    client_nonce: String,
}
async fn pair(State(server): State<Arc<RemoteServer>>, Json(input): Json<PairRequest>) -> Response {
    let pairing = server.clone();
    match tokio::task::spawn_blocking(move || {
        pairing.auth.lock().unwrap_or_else(|e| e.into_inner()).pair(
            &input.code,
            &input.name,
            &input.client_nonce,
        )
    })
    .await
    {
        Ok(Ok(device)) => {
            server.publish(None, "devices:changed", json!({}));
            Json(json!({"device":device.device,"token":device.token,"manifest":server.manifest()}))
                .into_response()
        }
        Ok(Err(message)) => failure(StatusCode::FORBIDDEN, message),
        Err(_) => failure(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to pair this device",
        ),
    }
}
async fn manifest(State(server): State<Arc<RemoteServer>>, headers: HeaderMap) -> Response {
    if server.authenticate(&headers).is_none() {
        return unauthorized();
    }
    Json(server.manifest()).into_response()
}
async fn rpc(
    State(server): State<Arc<RemoteServer>>,
    headers: HeaderMap,
    Json(input): Json<RpcRequest>,
) -> Response {
    let device = match server.authenticate(&headers) {
        Some(device) => device,
        None => return unauthorized(),
    };
    if let Err(error) = input.command.validate() {
        return failure(StatusCode::BAD_REQUEST, error);
    }
    let request = server.host.execute(&device.id, input.command);
    let result = tokio::select! {biased; _ = server.shutdown.cancelled()=>Err("Remote access has stopped".into()),_=server.revoked(&device.id)=>Err("Device was revoked".into()),result=tokio::time::timeout(Duration::from_secs(150),request)=>result.unwrap_or_else(|_|Err("Remote request timed out".into()))};
    if !server
        .auth
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .has_device(&device.id)
    {
        return unauthorized();
    }
    match result {
        Ok(value) => Json(json!({"id":input.id,"result":value})).into_response(),
        Err(error) => failure(StatusCode::BAD_REQUEST, error),
    }
}
async fn events(
    State(server): State<Arc<RemoteServer>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if headers.get(header::ORIGIN).is_none() {
        return failure(StatusCode::FORBIDDEN, "WebSocket origin is required");
    }
    let Ok(permit) = server.sockets.clone().try_acquire_owned() else {
        return failure(StatusCode::TOO_MANY_REQUESTS, "Too many remote connections");
    };
    upgrade
        .max_message_size(MAX_BODY)
        .max_frame_size(MAX_BODY)
        .write_buffer_size(0)
        .max_write_buffer_size(2 * MAX_BODY)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            socket_session(server, socket).await
        })
        .into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authenticate {
    token: String,
}
async fn socket_session(server: Arc<RemoteServer>, mut socket: WebSocket) {
    let first = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await;
    let device = match first {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Authenticate>(&text)
            .ok()
            .and_then(|input| {
                server
                    .auth
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .authenticate(&input.token)
            }),
        _ => None,
    };
    let Some(device) = device else {
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: close_code::POLICY,
                reason: "Device is not paired or was revoked".into(),
            })))
            .await;
        return;
    };
    let resumed = {
        let mut connections = server.connections.lock().unwrap_or_else(|e| e.into_inner());
        let entry = connections.entry(device.id.clone()).or_default();
        let resumed = entry.0 == 0;
        entry.0 += 1;
        entry.1 += 1;
        resumed
    };
    if resumed {
        server.host.connection_changed(&device.id, true).await;
    }
    let (mut sink, mut source) = socket.split();
    let mut events = server.events.subscribe();
    if sink.send(Message::Text(json!({"event":"connection:ready","payload":{"device":device,"manifest":server.manifest()}}).to_string().into())).await.is_ok() {
        let work=async {
            let mut heartbeat=tokio::time::interval(Duration::from_secs(5));
            loop {
                let outbound:Option<Value>=tokio::select! {
                    message=source.next()=>match message {
                        Some(Ok(Message::Text(text)))=>{
                            if !server.auth.lock().unwrap_or_else(|e|e.into_inner()).has_device(&device.id){break;}
                            if text=="ping" {Some(json!({"event":"connection:pong","payload":{}}))}
                            else {
                                match serde_json::from_str::<RpcRequest>(&text) {
                                    Ok(input) if input.command.is_audio()=>{
                                        let result=match input.command.validate(){Ok(())=>server.host.execute(&device.id,input.command).await,Err(error)=>Err(error)};
                                        Some(match result {Err(error)=>json!({"event":"connection:input-error","payload":{"id":input.id,"message":error}}),Ok(_)=>json!({"event":"connection:ack","payload":{"id":input.id}})})
                                    }
                                    _=>Some(json!({"event":"connection:input-error","payload":{"message":"Only bounded Live audio is accepted on the event channel"}})),
                                }
                            }
                        }
                        Some(Ok(Message::Ping(bytes)))=>{if sink.send(Message::Pong(bytes)).await.is_err(){break;}None},
                        Some(Ok(Message::Pong(_)))=>None,
                        _=>break,
                    },
                    event=events.recv()=>match event {
                        Ok(event) if event.owner.as_deref().is_none_or(|owner|owner==device.id)=>Some(serde_json::to_value(event).unwrap_or(Value::Null)),
                        Ok(_)=>None,
                        Err(broadcast::error::RecvError::Lagged(_))=>Some(json!({"event":"connection:resync","payload":{}})),
                        Err(_)=>break,
                    },
                    _=server.devices_changed.notified()=>{if !server.auth.lock().unwrap_or_else(|e|e.into_inner()).has_device(&device.id){break;}None},
                    _=heartbeat.tick()=>{if !server.auth.lock().unwrap_or_else(|e|e.into_inner()).has_device(&device.id){break;}Some(json!({"event":"connection:heartbeat","payload":{}}))},
                };
                if let Some(value)=outbound {if !matches!(tokio::time::timeout(Duration::from_secs(5),sink.send(Message::Text(value.to_string().into()))).await,Ok(Ok(()))){break;}}
            }
        };
        tokio::select!{biased;_=server.shutdown.cancelled()=>{},_=server.revoked(&device.id)=>{},_=work=>{}}
    }
    if !server
        .auth
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .has_device(&device.id)
    {
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            sink.send(Message::Close(Some(CloseFrame {
                code: close_code::POLICY,
                reason: "Device was revoked".into(),
            }))),
        )
        .await;
    }
    let (generation, last_connection) = {
        let mut connections = server.connections.lock().unwrap_or_else(|e| e.into_inner());
        let entry = connections.entry(device.id.clone()).or_default();
        entry.0 = entry.0.saturating_sub(1);
        entry.1 += 1;
        (entry.1, entry.0 == 0)
    };
    if last_connection {
        server.host.connection_changed(&device.id, false).await;
    }
    // Network changes are allowed to reconnect without destroying an active Live session.
    tokio::spawn(async move {
        tokio::select! {_=server.shutdown.cancelled()=>{},_=tokio::time::sleep(RECONNECT_GRACE)=>{}}
        if server
            .connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&device.id)
            .is_some_and(|entry| entry.0 == 0 && entry.1 == generation)
        {
            server.host.disconnected(&device.id).await;
        }
    });
}
async fn asset(State(server): State<Arc<RemoteServer>>, request: Request) -> Response {
    let path = request.uri().path();
    let target = if matches!(path, "/" | "/phone.html") {
        "phone.html"
    } else if path.starts_with("/assets/")
        && !path.contains('%')
        && !path.contains("..")
        && !path.contains('\\')
    {
        &path[1..]
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let target = target.to_owned();
    let host = server.host.clone();
    match tokio::task::spawn_blocking(move || host.asset(&target)).await {
        Ok(Some(asset)) => Response::builder()
            .header(header::CONTENT_TYPE, asset.mime_type)
            .body(Body::from(asset.bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
