//! Connection-owned MCP bridge. The subprocess is only a stdio transport;
//! all effects go through Nexa's existing scope, approval and receipt dispatcher.
use super::{error, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use nexa_core::{
    agent::{CancellationToken, ExternalToolSession},
    llm::{ContentPart, ToolCallRequest, ToolDefinition},
};
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};

#[derive(Clone)]
struct BridgeState {
    call_namespace: uuid::Uuid,
    token: String,
    definitions: Arc<Vec<ToolDefinition>>,
    active: Arc<RwLock<Option<Arc<ExternalToolSession>>>>,
    slots: Arc<tokio::sync::Semaphore>,
    cancellation: CancellationToken,
}

pub(super) struct Bridge {
    state: BridgeState,
    url: String,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.deactivate();
        self.stop.cancel();
        self.task.abort();
    }
}
impl Bridge {
    pub(super) async fn start(
        definitions: Vec<ToolDefinition>,
        cancellation: CancellationToken,
    ) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(error)?;
        let url = format!("http://{}/mcp", listener.local_addr().map_err(error)?);
        let state = BridgeState {
            call_namespace: uuid::Uuid::new_v4(),
            token: uuid::Uuid::new_v4().to_string(),
            definitions: Arc::new(definitions),
            active: Arc::new(RwLock::new(None)),
            slots: Arc::new(tokio::sync::Semaphore::new(16)),
            cancellation,
        };
        let router = Router::new()
            .route("/mcp", post(handle))
            .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
            .with_state(state.clone());
        let stop = CancellationToken::new();
        let shutdown = stop.clone();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await;
        });
        Ok(Self {
            state,
            url,
            stop,
            task,
        })
    }
    pub(super) fn activate(&self, tools: Arc<ExternalToolSession>) {
        *self.state.active.write().unwrap_or_else(|e| e.into_inner()) = Some(tools);
    }
    pub(super) fn deactivate(&self) {
        if let Some(tools) = self
            .state
            .active
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            tools.cancel_tools();
        }
    }
    pub(super) fn server(&self) -> Result<nexa_core::mcp::McpServer> {
        // Stdio is mandatory in ACP v1; no HTTP capability assumption or helper
        // runtime dependency. Credentials are passed in env, never argv/logs.
        Ok(nexa_core::mcp::McpServer {
            id: "host".into(),
            name: "Nexa host tools".into(),
            transport: "stdio".into(),
            command: Some(
                std::env::current_exe()
                    .map_err(error)?
                    .to_string_lossy()
                    .into_owned(),
            ),
            args: Some(json!(["--nexa-mcp-stdio"]).to_string()),
            url: None,
            env_json: Some(
                json!({"NEXA_MCP_BRIDGE_URL":self.url,"NEXA_MCP_BRIDGE_TOKEN":self.state.token})
                    .to_string(),
            ),
            headers_json: None,
            enabled: true,
            builtin_id: None,
            oauth_epoch: 0,
            created_at: String::new(),
            updated_at: String::new(),
        })
    }
}

async fn handle(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(message): Json<Value>,
) -> Response {
    if headers.contains_key("origin")
        || headers.get("authorization").and_then(|h| h.to_str().ok())
            != Some(&format!("Bearer {}", state.token))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(_permit) = state.slots.try_acquire() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let id = message.get("id").cloned();
    let method = message["method"].as_str().unwrap_or_default();
    if id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let id = id.unwrap();
    let tools = state
        .active
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let supported = nexa_core::mcp::client::SUPPORTED_PROTOCOL_VERSIONS;
    let version = message["params"]["protocolVersion"]
        .as_str()
        .filter(|v| supported.contains(v))
        .unwrap_or(supported[0]);
    let result = match (method, tools) {
        ("initialize", _) => Ok(
            json!({"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"nexa-host","version":env!("CARGO_PKG_VERSION")}}),
        ),
        ("ping", _) => Ok(json!({})),
        ("tools/list", _) if !state.cancellation.is_cancelled() => Ok(
            json!({"tools":state.definitions.iter().map(|tool| json!({"name":tool.name,"description":tool.description,"inputSchema":tool.parameters})).collect::<Vec<_>>()}),
        ),
        ("tools/call", Some(tools)) => {
            let params = &message["params"];
            let name = params["name"].as_str().unwrap_or_default();
            if !state.definitions.iter().any(|tool| tool.name == name) {
                Err("Unknown Nexa tool".to_string())
            } else {
                let call = ToolCallRequest {
                    id: format!("acp-host:{}:{}", state.call_namespace, id),
                    name: name.into(),
                    arguments: params
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({}))
                        .to_string(),
                    thought_signature: None,
                };
                tools.execute(call).await.map(|output| {
                    let mut content = vec![json!({"type":"text","text":output.result.content})];
                    content.extend(output.visual_parts.into_iter().filter_map(|part| match part {
                        ContentPart::Image { media_type, data } => Some(json!({"type":"image","mimeType":media_type,"data":data})),
                        ContentPart::Text { text } => Some(json!({"type":"text","text":text})),
                        ContentPart::Document { document } => Some(json!({"type":"text","text":document.fallback("ACP document protocol unavailable")})), _ => None,
                    }));
                    json!({"content":content,"isError":output.result.is_error})
                }).map_err(|cause| cause.to_string())
            }
        }
        ("tools/list" | "tools/call", None) => Err("Nexa turn is no longer active".into()),
        _ => Err("Unsupported MCP method".into()),
    };
    Json(match result {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(message) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":message}}),
    })
    .into_response()
}

/// Invoked before any GUI, database or single-instance initialization.
pub(crate) fn stdio_proxy() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let url = url::Url::parse(&std::env::var("NEXA_MCP_BRIDGE_URL")?)?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || url.path() != "/mcp"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Invalid Nexa MCP endpoint".into());
    }
    let token = std::env::var("NEXA_MCP_BRIDGE_TOKEN")?;
    proxy_io(
        url,
        token,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
}

fn proxy_io(
    url: url::Url,
    token: String,
    mut input: impl std::io::BufRead,
    mut output: impl std::io::Write,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::io::{BufRead, Read};
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(None)
        .build()?;
    loop {
        let mut frame = Vec::new();
        // Bound malformed unterminated lines without imposing a tool deadline.
        let read = std::io::Read::by_ref(&mut input)
            .take(8 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut frame)?;
        if read == 0 {
            return Ok(());
        }
        if frame.len() > 8 * 1024 * 1024 {
            return Err("MCP frame exceeds limit".into());
        }
        let request: Value = serde_json::from_slice(&frame)?;
        let response = client
            .post(url.clone())
            .bearer_auth(&token)
            .json(&request)
            .send()?
            .error_for_status()?;
        if request.get("id").is_some() {
            serde_json::to_writer(&mut output, &response.json::<Value>()?)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[tokio::test]
    async fn acp_host_bridge_lists_calls_and_revokes_real_nexa_tools() {
        let (request, mut events, effects, nonce) = crate::agent_runtime::tests::fixture(
            crate::agent_runtime::AgentRuntimeKind::Acp("gemini_cli".into()),
            "native",
        );
        let drain = tokio::spawn(async move { while events.recv().await.is_some() {} });
        let tools = request
            .host_tools(false, request.cancellation.clone())
            .unwrap();
        let bridge = Bridge::start(tools.definitions(), request.cancellation.clone())
            .await
            .unwrap();
        bridge.activate(tools.clone());
        // ACP's mandatory stdio transport needs no optional HTTP capability.
        let projected =
            super::super::mcp::project(&[bridge.server().unwrap()], &json!({})).unwrap();
        assert_eq!(projected[0]["args"][0], "--nexa-mcp-stdio");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let post = |payload| {
            client
                .post(&bridge.url)
                .bearer_auth(&bridge.state.token)
                .json(&payload)
        };
        let list: Value = post(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let name = list["result"]["tools"][0]["name"].as_str().unwrap();
        let call = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":{}}});
        let endpoint = bridge.url.parse().unwrap();
        let token = bridge.state.token.clone();
        let frames = format!(
            "{}\n{}\n{}\n{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            call,
            call
        );
        let replies = tokio::task::spawn_blocking(move || {
            let mut bytes = vec![];
            proxy_io(endpoint, token, std::io::Cursor::new(frames), &mut bytes).unwrap();
            String::from_utf8(bytes)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        })
        .await
        .unwrap();
        assert_eq!(replies.len(), 3, "notifications have no response frame");
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], "nexa-host");
        for result in &replies[1..] {
            assert_eq!(
                result["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .lines()
                    .next(),
                Some(nonce.as_str())
            );
            assert_eq!(result["result"]["isError"], false);
        }
        let mut contradictory = call.clone();
        contradictory["params"]["arguments"] = json!({"changed":true});
        let conflict: Value = post(contradictory)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(conflict.get("error").is_some());
        assert_eq!(
            effects.load(Ordering::SeqCst),
            1,
            "duplicate MCP delivery must not repeat effects"
        );
        assert_eq!(
            client
                .post(&bridge.url)
                .json(&call)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            post(call.clone())
                .header("Origin", "https://untrusted.example")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        bridge.deactivate();
        let result: Value = post(call).send().await.unwrap().json().await.unwrap();
        assert!(result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no longer active"));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert!(
            !request.cancellation.is_cancelled(),
            "revoking tools must not cancel completion hooks"
        );
        tools.persist_answer("Verified").await.unwrap();
        drop(bridge);
        drop(tools);
        drop(request);
        drain.await.unwrap();
    }
}
