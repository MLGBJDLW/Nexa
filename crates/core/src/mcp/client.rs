//! MCP client for stdio, legacy SSE, and Streamable HTTP transports.

use std::collections::{HashMap, HashSet};
use std::env;
#[cfg(windows)]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::{Client as HttpClient, StatusCode, Url};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use super::events::McpClientEvents;
use crate::error::CoreError;
use crate::mcp::McpToolInfo;

const JSONRPC_VERSION: &str = "2.0";
const METHOD_NOT_FOUND_CODE: i64 = -32601;
const CONTENT_TYPE_JSON: &str = "application/json";
const CONTENT_TYPE_SSE: &str = "text/event-stream";
const HEADER_MCP_PROTOCOL_VERSION: &str = "mcp-protocol-version";
const HEADER_MCP_SESSION_ID: &str = "mcp-session-id";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const SSE_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_DIAGNOSTIC_BYTES: usize = 16 * 1024;
const MAX_CATALOG_PAGES: usize = 128;
const MAX_CATALOG_TOOLS: usize = 4096;
const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] =
    ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

fn content_catalog_part(
    result: Result<Vec<Value>, CoreError>,
    complete: &mut bool,
    diagnostics: &mut Vec<String>,
) -> Result<Vec<Value>, CoreError> {
    match result {
        Ok(items) => Ok(items),
        Err(error @ CoreError::McpTransport(_)) => Err(error),
        Err(error) => {
            *complete = false;
            diagnostics.push(error.to_string());
            Ok(Vec::new())
        }
    }
}

struct StdioTransport {
    child: Child,
    stdin: tokio::io::BufWriter<tokio::process::ChildStdin>,
    stdout_rx: mpsc::Receiver<Value>,
    reader_handle: tokio::task::JoinHandle<()>,
    stderr_buf: Arc<Mutex<String>>,
    stderr_handle: tokio::task::JoinHandle<()>,
    events: Arc<McpClientEvents>,
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Failed discovery and cancelled connection futures do not reach the
        // manager's explicit shutdown path. The transport still owns its I/O
        // tasks and child in those cases.
        self.reader_handle.abort();
        self.stderr_handle.abort();
        let _ = self.child.start_kill();
    }
}

struct LegacySseTransport {
    client: HttpClient,
    message_url: Url,
    custom_headers: HeaderMap,
    events_rx: mpsc::Receiver<Value>,
    diagnostics: Arc<Mutex<String>>,
    stream_handle: tokio::task::JoinHandle<()>,
    events: Arc<McpClientEvents>,
}

impl Drop for LegacySseTransport {
    fn drop(&mut self) {
        self.stream_handle.abort();
    }
}

struct StreamableHttpTransport {
    client: HttpClient,
    endpoint_url: Url,
    custom_headers: HeaderMap,
    session_id: Option<String>,
    events: Arc<McpClientEvents>,
    notification_reader: Option<tokio::task::JoinHandle<()>>,
    notification_rx: Option<mpsc::Receiver<Value>>,
}

impl Drop for StreamableHttpTransport {
    fn drop(&mut self) {
        if let Some(reader) = self.notification_reader.take() {
            reader.abort();
        }
    }
}

enum Transport {
    Stdio(StdioTransport),
    LegacySse(LegacySseTransport),
    StreamableHttp(StreamableHttpTransport),
}

/// MCP client communicating with a server over one of the supported transports.
pub struct McpClient {
    transport: Transport,
    request_id: AtomicI64,
    server_name: String,
    protocol_version: String,
    server_capabilities: Value,
    /// Request timeout; tools/call renews its idle deadline with correlated
    /// advancing progress. Defaults to [`DEFAULT_TIMEOUT`].
    call_timeout: Duration,
    auth: Option<super::oauth::RequestAuth>,
}

enum StreamablePostOutcome {
    Accepted,
    Json(Value),
    Sse(reqwest::Response),
}

enum StreamablePostError {
    Core(CoreError),
    SessionExpired,
}

impl From<CoreError> for StreamablePostError {
    fn from(value: CoreError) -> Self {
        Self::Core(value)
    }
}

impl McpClient {
    /// Connect to an MCP server via stdio transport.
    pub async fn connect_stdio(
        command: &str,
        args: &[String],
        env: Option<&HashMap<String, String>>,
        server_name: &str,
    ) -> Result<Self, CoreError> {
        let transport = Self::build_stdio_transport(command, args, env).await?;
        let mut client = Self {
            transport: Transport::Stdio(transport),
            request_id: AtomicI64::new(1),
            server_name: server_name.to_string(),
            protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].to_string(),
            server_capabilities: serde_json::json!({}),
            call_timeout: DEFAULT_TIMEOUT,
            auth: None,
        };
        client.initialize_handshake().await?;
        Ok(client)
    }

    /// Connect to an MCP server via legacy SSE transport.
    pub async fn connect_sse(
        url: &str,
        headers: Option<&HashMap<String, String>>,
        server_name: &str,
    ) -> Result<Self, CoreError> {
        Self::connect_remote_authorized(url, headers, server_name, true, None).await
    }

    /// Connect to an MCP server via Streamable HTTP transport.
    pub async fn connect_streamable_http(
        url: &str,
        headers: Option<&HashMap<String, String>>,
        server_name: &str,
    ) -> Result<Self, CoreError> {
        Self::connect_remote_authorized(url, headers, server_name, false, None).await
    }

    pub(crate) async fn connect_remote_authorized(
        url: &str,
        headers: Option<&HashMap<String, String>>,
        server_name: &str,
        legacy: bool,
        auth: Option<super::oauth::RequestAuth>,
    ) -> Result<Self, CoreError> {
        let transport = if legacy {
            Transport::LegacySse(
                Self::build_legacy_sse_transport(url, headers, auth.as_ref()).await?,
            )
        } else {
            Transport::StreamableHttp(Self::build_streamable_http_transport(url, headers)?)
        };
        let mut client = Self {
            transport,
            request_id: AtomicI64::new(1),
            server_name: server_name.to_string(),
            protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].to_string(),
            server_capabilities: serde_json::json!({}),
            call_timeout: DEFAULT_TIMEOUT,
            auth,
        };
        client.initialize_handshake().await?;
        Ok(client)
    }

    /// Override the call timeout for this client.
    pub fn set_call_timeout(&mut self, timeout: Duration) {
        self.call_timeout = timeout;
    }

    /// List tools available on the connected MCP server.
    pub async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>, CoreError> {
        // Keep the historical fallback for peers with an empty capability object,
        // while honoring explicit resource-only or prompt-only capabilities.
        if self
            .server_capabilities
            .as_object()
            .is_some_and(|caps| !caps.is_empty() && !caps.contains_key("tools"))
        {
            return Ok(Vec::new());
        }
        let deadline = self.call_timeout;
        tokio::time::timeout(deadline, async {
            let mut tools = Vec::new();
            let mut cursor: Option<String> = None;
            let mut cursors = HashSet::new();
            let mut names = HashSet::new();
            let mut catalog_bytes = 0usize;
            for _ in 0..MAX_CATALOG_PAGES {
                let params = cursor.as_ref().map_or_else(
                    || serde_json::json!({}),
                    |cursor| serde_json::json!({"cursor":cursor}),
                );
                let response = self.send_request("tools/list", Some(params)).await?;
                let page: Vec<McpToolInfo> =
                    serde_json::from_value(response.get("tools").cloned().ok_or_else(|| {
                        CoreError::Mcp("tools/list response missing 'tools' field".into())
                    })?)
                    .map_err(|error| {
                        CoreError::Mcp(format!("Failed to parse tools list: {error}"))
                    })?;
                for tool in page {
                    if tool.name.is_empty() || !names.insert(tool.name.clone()) {
                        return Err(CoreError::Mcp(
                            "MCP catalog contains an empty or duplicate exact tool name".into(),
                        ));
                    }
                    catalog_bytes = catalog_bytes
                        .saturating_add(serde_json::to_vec(&tool).map_err(CoreError::from)?.len());
                    if catalog_bytes > MAX_CATALOG_BYTES {
                        return Err(CoreError::Mcp(format!(
                            "MCP catalog exceeds the {MAX_CATALOG_BYTES}-byte budget"
                        )));
                    }
                    tools.push(tool);
                    if tools.len() > MAX_CATALOG_TOOLS {
                        return Err(CoreError::Mcp(format!(
                            "MCP catalog exceeds {MAX_CATALOG_TOOLS} tools"
                        )));
                    }
                }
                cursor = match response.get("nextCursor") {
                    None | Some(Value::Null) => return Ok(tools),
                    Some(Value::String(cursor)) if !cursor.is_empty() && cursor.len() <= 4096 => {
                        Some(cursor.clone())
                    }
                    _ => return Err(CoreError::Mcp("Invalid tools/list nextCursor".into())),
                };
                if !cursors.insert(cursor.clone().expect("next cursor exists")) {
                    return Err(CoreError::Mcp(
                        "MCP tools/list repeated a pagination cursor; catalog is incomplete".into(),
                    ));
                }
            }
            Err(CoreError::Mcp(format!(
                "MCP catalog exceeded {MAX_CATALOG_PAGES} pages; catalog is incomplete"
            )))
        })
        .await
        .map_err(|_| CoreError::McpTransport("MCP full catalog discovery timed out".into()))?
    }

    pub(crate) fn events(&self) -> Arc<McpClientEvents> {
        match &self.transport {
            Transport::Stdio(transport) => Arc::clone(&transport.events),
            Transport::LegacySse(transport) => Arc::clone(&transport.events),
            Transport::StreamableHttp(transport) => Arc::clone(&transport.events),
        }
    }

    pub async fn list_content(&mut self) -> Result<super::McpContentCatalog, CoreError> {
        let deadline = self.call_timeout;
        tokio::time::timeout(deadline, async {
            let mut content = super::McpContentCatalog {
                capabilities: self.server_capabilities.clone(),
                resources_complete: true,
                resource_templates_complete: true,
                prompts_complete: true,
                ..Default::default()
            };
            if self.server_capabilities.get("resources").is_some() {
                content.resources = content_catalog_part(
                    self.list_content_pages("resources/list", "resources", "uri")
                        .await,
                    &mut content.resources_complete,
                    &mut content.content_diagnostics,
                )?;
                content.resource_templates = content_catalog_part(
                    self.list_content_pages(
                        "resources/templates/list",
                        "resourceTemplates",
                        "uriTemplate",
                    )
                    .await,
                    &mut content.resource_templates_complete,
                    &mut content.content_diagnostics,
                )?;
            }
            if self.server_capabilities.get("prompts").is_some() {
                content.prompts = content_catalog_part(
                    self.list_content_pages("prompts/list", "prompts", "name")
                        .await,
                    &mut content.prompts_complete,
                    &mut content.content_diagnostics,
                )?;
            }
            Ok(content)
        })
        .await
        .map_err(|_| CoreError::McpTransport("MCP content catalog discovery timed out".into()))?
    }

    async fn list_content_pages(
        &mut self,
        method: &str,
        field: &str,
        key: &str,
    ) -> Result<Vec<Value>, CoreError> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        let mut cursors = HashSet::new();
        let mut identities = HashSet::new();
        let mut bytes = 0usize;
        for _ in 0..MAX_CATALOG_PAGES {
            let params = cursor.as_ref().map_or_else(
                || serde_json::json!({}),
                |cursor| serde_json::json!({"cursor":cursor}),
            );
            let response = match self.send_request(method, Some(params)).await {
                Ok(response) => response,
                Err(CoreError::McpRpc {
                    code: METHOD_NOT_FOUND_CODE,
                    ..
                }) if method == "resources/templates/list" && cursor.is_none() => {
                    return Ok(Vec::new())
                }
                Err(error) => return Err(error),
            };
            let page = response
                .get(field)
                .and_then(Value::as_array)
                .ok_or_else(|| CoreError::Mcp(format!("{method} response is missing {field}")))?;
            for item in page {
                if item
                    .get("name")
                    .and_then(Value::as_str)
                    .is_none_or(|name| name.is_empty())
                {
                    return Err(CoreError::Mcp(format!(
                        "Invalid {method} entry: missing name"
                    )));
                }
                if method == "prompts/list" {
                    if let Some(arguments) = item.get("arguments") {
                        let arguments = arguments.as_array().ok_or_else(|| {
                            CoreError::Mcp("Prompt arguments must be an array".into())
                        })?;
                        let mut names = HashSet::new();
                        for argument in arguments {
                            let name = argument
                                .get("name")
                                .and_then(Value::as_str)
                                .filter(|name| !name.is_empty())
                                .ok_or_else(|| {
                                    CoreError::Mcp("Prompt argument has no name".into())
                                })?;
                            if !names.insert(name)
                                || argument
                                    .get("required")
                                    .is_some_and(|value| !value.is_boolean())
                            {
                                return Err(CoreError::Mcp("Prompt arguments contain duplicate names or invalid required flags".into()));
                            }
                        }
                    }
                }
                let identity = item
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty() && value.len() <= 16_384)
                    .ok_or_else(|| {
                        CoreError::Mcp(format!("Invalid {method} entry: missing {key}"))
                    })?;
                if !identities.insert(identity.to_string()) {
                    return Err(CoreError::Mcp(format!("Duplicate {key} in {method}")));
                }
                bytes = bytes.saturating_add(serde_json::to_vec(item)?.len());
                if bytes > MAX_CATALOG_BYTES || items.len() >= MAX_CATALOG_TOOLS {
                    return Err(CoreError::Mcp(format!(
                        "{method} catalog exceeds its size budget"
                    )));
                }
                items.push(item.clone());
            }
            cursor = match response.get("nextCursor") {
                None | Some(Value::Null) => return Ok(items),
                Some(Value::String(value)) if !value.is_empty() && value.len() <= 4096 => {
                    Some(value.clone())
                }
                _ => return Err(CoreError::Mcp(format!("Invalid {method} nextCursor"))),
            };
            if !cursors.insert(cursor.clone().expect("cursor exists")) {
                return Err(CoreError::Mcp(format!(
                    "{method} repeated a pagination cursor"
                )));
            }
        }
        Err(CoreError::Mcp(format!("{method} exceeded the page budget")))
    }

    pub async fn read_resource(
        &mut self,
        uri: &str,
    ) -> Result<super::result::McpCallOutcome, CoreError> {
        if self.server_capabilities.get("resources").is_none() {
            return Err(CoreError::Mcp(
                "This connector does not advertise resources".into(),
            ));
        }
        if uri.is_empty() || uri.len() > 16_384 {
            return Err(CoreError::InvalidInput("Invalid MCP resource URI".into()));
        }
        let response = self
            .send_request("resources/read", Some(serde_json::json!({"uri":uri})))
            .await?;
        let contents = response
            .get("contents")
            .and_then(Value::as_array)
            .ok_or_else(|| CoreError::Mcp("resources/read response is missing contents".into()))?;
        super::result::McpCallOutcome::from_response(serde_json::json!({
            "content":contents.iter().map(|resource| serde_json::json!({"type":"resource","resource":resource})).collect::<Vec<_>>(),
            "_meta":response.get("_meta"),
        }))
    }

    pub async fn get_prompt(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<super::result::McpCallOutcome, CoreError> {
        if self.server_capabilities.get("prompts").is_none() {
            return Err(CoreError::Mcp(
                "This connector does not advertise prompts".into(),
            ));
        }
        let response = self
            .send_request(
                "prompts/get",
                Some(serde_json::json!({"name":name,"arguments":arguments})),
            )
            .await?;
        let messages = response
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| CoreError::Mcp("prompts/get response is missing messages".into()))?;
        let mut content = Vec::new();
        for message in messages {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .filter(|role| matches!(*role, "user" | "assistant"))
                .ok_or_else(|| {
                    CoreError::Mcp("MCP prompt contains an invalid message role".into())
                })?;
            content.push(
                serde_json::json!({"type":"text","text":format!("Prompt message ({role}):")}),
            );
            content.push(
                message.get("content").cloned().ok_or_else(|| {
                    CoreError::Mcp("MCP prompt message is missing content".into())
                })?,
            );
        }
        // Template messages are returned as evidence; never install their roles in model history.
        super::result::McpCallOutcome::from_response(
            serde_json::json!({"content":content,"structuredContent":response}),
        )
    }

    fn start_streamable_notification_reader(&mut self) {
        let Transport::StreamableHttp(transport) = &mut self.transport else {
            return;
        };
        if transport.notification_reader.is_some() {
            return;
        }
        let mut headers = transport.custom_headers.clone();
        headers.insert(ACCEPT, HeaderValue::from_static(CONTENT_TYPE_SSE));
        let Ok(protocol) = HeaderValue::from_str(&self.protocol_version) else {
            return;
        };
        headers.insert(
            HeaderName::from_static(HEADER_MCP_PROTOCOL_VERSION),
            protocol,
        );
        if let Some(session_id) = &transport.session_id {
            let Ok(session_id) = HeaderValue::from_str(session_id) else {
                return;
            };
            headers.insert(HeaderName::from_static(HEADER_MCP_SESSION_ID), session_id);
        }
        let client = transport.client.clone();
        let endpoint = transport.endpoint_url.clone();
        let auth = self.auth.clone();
        let events = Arc::clone(&transport.events);
        let (sender, receiver) = mpsc::channel(64);
        transport.notification_rx = Some(receiver);
        transport.notification_reader = Some(tokio::spawn(async move {
            // GET is optional. A 405 or closed notification stream never causes
            // a tools/call retry or disables a server with working POST RPCs.
            let Ok(Ok(response)) = tokio::time::timeout(
                SSE_CONNECT_TIMEOUT,
                send_authorized(&client, &endpoint, headers, None, auth.as_ref()),
            )
            .await
            else {
                return;
            };
            if !response.status().is_success()
                || !response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.contains(CONTENT_TYPE_SSE))
            {
                return;
            }
            let mut stream = response.bytes_stream();
            let mut buffer = Vec::new();
            while let Some(Ok(chunk)) = stream.next().await {
                if buffer.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return;
                }
                buffer.extend_from_slice(&chunk);
                while let Some(raw) = drain_sse_event(&mut buffer) {
                    let (event, data) = parse_sse_event(&raw);
                    if data.trim().is_empty() || event.as_deref() == Some("ping") {
                        continue;
                    }
                    let Ok(message) = serde_json::from_str::<Value>(data.trim()) else {
                        continue;
                    };
                    if events.observe(&message) {
                        continue;
                    }
                    if sender.send(message).await.is_err() {
                        return;
                    }
                }
            }
        }));
    }

    /// Call a tool on the MCP server.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<super::result::McpCallOutcome, CoreError> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments,
        });
        // Effectful requests are never reposted after session expiry. The
        // connector slot may establish a fresh session for a subsequent call.
        let response = self
            .send_request_inner("tools/call", Some(params), false)
            .await?;
        // isError belongs to the typed result. Business failures can carry
        // useful structured content or attachments and never request reconnect.
        super::result::McpCallOutcome::from_response(response)
    }

    /// Gracefully shut down the MCP server connection.
    pub async fn shutdown(&mut self) -> Result<(), CoreError> {
        if let Transport::Stdio(transport) = &mut self.transport {
            let _ = transport.child.kill().await;
            transport.reader_handle.abort();
            transport.stderr_handle.abort();
            return Ok(());
        }

        if let Transport::LegacySse(transport) = &mut self.transport {
            transport.stream_handle.abort();
            return Ok(());
        }

        let _ = self.reset_streamable_http_session().await;
        Ok(())
    }

    async fn initialize_handshake(&mut self) -> Result<(), CoreError> {
        let mut last_error = None;

        for version in SUPPORTED_PROTOCOL_VERSIONS {
            self.protocol_version = version.to_string();
            let init_params = serde_json::json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": {
                    "name": "nexa",
                    "version": "0.1.0"
                }
            });

            match self
                .send_request_inner("initialize", Some(init_params), false)
                .await
            {
                Ok(result) => {
                    self.server_capabilities = result
                        .get("capabilities")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));
                    if !self.server_capabilities.is_object() {
                        return Err(CoreError::Mcp("Invalid MCP server capabilities".into()));
                    }
                    if let Some(negotiated) = result
                        .get("protocolVersion")
                        .and_then(|value| value.as_str())
                    {
                        self.protocol_version = negotiated.to_string();
                    }
                    self.send_notification("notifications/initialized", None)
                        .await?;
                    // Progress can arrive on GET while a POST response is still
                    // pending, independently of catalog listChanged support.
                    // Peers without GET support may reject this optional stream.
                    self.start_streamable_notification_reader();
                    return Ok(());
                }
                Err(error @ CoreError::McpTransport(_)) => return Err(error),
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            CoreError::Mcp("Failed to negotiate an MCP protocol version".into())
        }))
    }

    async fn send_request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, CoreError> {
        self.send_request_inner(method, params, true).await
    }

    async fn send_request_inner(
        &mut self,
        method: &str,
        params: Option<Value>,
        allow_reinitialize: bool,
    ) -> Result<Value, CoreError> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let mut request = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "method": method,
        });
        if let Some(p) = params {
            request
                .as_object_mut()
                .expect("request object")
                .insert("params".to_string(), p);
        }

        let progress = if method == "tools/call" {
            let params = request["params"].as_object_mut().ok_or_else(|| {
                CoreError::Mcp("MCP tools/call requires object parameters".into())
            })?;
            params.insert("_meta".into(), serde_json::json!({"progressToken":id}));
            Some(self.events().track_call(id))
        } else {
            None
        };
        let timeout = self.call_timeout;
        // Discovery keeps a full-operation deadline, even on heartbeat-heavy
        // streams. Tool execution instead renews its idle deadline only when
        // its own progress token advances; cancellation still drops the future.
        let operation = async {
            match &self.transport {
                Transport::StreamableHttp(_) => {
                    self.send_streamable_http_request(request, id, method, allow_reinitialize)
                        .await
                }
                _ => {
                    self.send_transport_message(&request).await?;
                    self.wait_for_response(id, method).await
                }
            }
        };
        let result = if let Some(progress) = progress {
            progress.wait(timeout, operation).await
        } else {
            tokio::time::timeout(timeout, operation)
                .await
                .map_err(|_| ())
        };
        match result {
            Ok(result) => result,
            Err(_) => Err(self.transport_timeout_error(method).await),
        }
    }

    async fn send_notification(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), CoreError> {
        let mut notification = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": method,
        });
        if let Some(p) = params {
            notification
                .as_object_mut()
                .expect("notification object")
                .insert("params".to_string(), p);
        }
        match tokio::time::timeout(
            self.call_timeout,
            self.send_transport_message(&notification),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(self.transport_timeout_error(method).await),
        }
    }

    async fn send_transport_message(&mut self, payload: &Value) -> Result<(), CoreError> {
        if let Transport::Stdio(transport) = &mut self.transport {
            let mut msg = serde_json::to_string(payload)
                .map_err(|e| CoreError::Mcp(format!("Failed to serialize request: {e}")))?;
            msg.push('\n');
            let writing = tokio::time::timeout(self.call_timeout, async {
                transport
                    .stdin
                    .write_all(msg.as_bytes())
                    .await
                    .map_err(|e| {
                        CoreError::McpTransport(format!("Failed to write to MCP server stdin: {e}"))
                    })?;
                transport.stdin.flush().await.map_err(|e| {
                    CoreError::McpTransport(format!("Failed to flush MCP server stdin: {e}"))
                })
            })
            .await;
            return match writing {
                Ok(result) => result,
                Err(_) => Err(self.transport_timeout_error("stdio write").await),
            };
        }

        if matches!(&self.transport, Transport::LegacySse(_)) {
            self.post_legacy_sse_message(payload).await?;
            return Ok(());
        }

        match self.post_streamable_http(payload).await.map_err(|err| {
            streamable_post_error_into_core(err, &self.server_name, "notification")
        })? {
            StreamablePostOutcome::Accepted => Ok(()),
            StreamablePostOutcome::Json(value) => {
                self.process_http_json_payload(value, None, "notification")
                    .await?;
                Ok(())
            }
            StreamablePostOutcome::Sse(response) => {
                self.process_streamable_http_sse(response, None, "notification")
                    .await?;
                Ok(())
            }
        }
    }

    async fn wait_for_response(&mut self, id: i64, method: &str) -> Result<Value, CoreError> {
        // send_request_inner owns the response deadline for every transport.
        // A second absolute timeout here would truncate progressing tools/call.
        loop {
            let next = match &mut self.transport {
                Transport::Stdio(transport) => transport.stdout_rx.recv().await,
                Transport::LegacySse(transport) => transport.events_rx.recv().await,
                Transport::StreamableHttp(_) => {
                    unreachable!("queue wait only used for stdio/SSE")
                }
            };

            match next {
                Some(message) => {
                    if let Some(result) = self
                        .process_incoming_message(message, Some(id), method)
                        .await?
                    {
                        return Ok(result);
                    }
                }
                None => return Err(self.transport_closed_error().await),
            }
        }
    }

    async fn send_streamable_http_request(
        &mut self,
        request: Value,
        request_id: i64,
        method: &str,
        allow_reinitialize: bool,
    ) -> Result<Value, CoreError> {
        let mut can_reinitialize = allow_reinitialize;

        loop {
            match self.post_streamable_http(&request).await {
                Ok(StreamablePostOutcome::Accepted) => {
                    if matches!(&self.transport, Transport::StreamableHttp(transport) if transport.notification_rx.is_some())
                    {
                        loop {
                            let message = match &mut self.transport {
                                Transport::StreamableHttp(transport) => {
                                    transport
                                        .notification_rx
                                        .as_mut()
                                        .expect("notification receiver exists")
                                        .recv()
                                        .await
                                }
                                _ => unreachable!(),
                            };
                            let Some(message) = message else {
                                break;
                            };
                            if let Some(result) = self
                                .process_incoming_message(message, Some(request_id), method)
                                .await?
                            {
                                return Ok(result);
                            }
                        }
                        if let Transport::StreamableHttp(transport) = &mut self.transport {
                            transport.notification_rx = None;
                        }
                    }
                    let response = self.open_streamable_http_get().await.map_err(|err| {
                        streamable_post_error_into_core(err, &self.server_name, method)
                    })?;
                    if let Some(result) = self
                        .process_streamable_http_sse(response, Some(request_id), method)
                        .await?
                    {
                        return Ok(result);
                    }
                    return Err(CoreError::McpTransport(format!(
                        "MCP server '{}' accepted {method} but no matching response arrived.",
                        self.server_name
                    )));
                }
                Ok(StreamablePostOutcome::Json(value)) => {
                    if let Some(result) = self
                        .process_http_json_payload(value, Some(request_id), method)
                        .await?
                    {
                        return Ok(result);
                    }
                    return Err(CoreError::Mcp(format!(
                        "MCP server '{}' returned JSON for {method} without a matching response id.",
                        self.server_name
                    )));
                }
                Ok(StreamablePostOutcome::Sse(response)) => {
                    if let Some(result) = self
                        .process_streamable_http_sse(response, Some(request_id), method)
                        .await?
                    {
                        return Ok(result);
                    }
                    return Err(CoreError::McpTransport(format!(
                        "MCP server '{}' closed the SSE response for {method} before replying.",
                        self.server_name
                    )));
                }
                Err(StreamablePostError::SessionExpired) if can_reinitialize => {
                    let _ = self.reset_streamable_http_session().await;
                    Box::pin(self.initialize_handshake()).await?;
                    can_reinitialize = false;
                }
                Err(err) => {
                    return Err(streamable_post_error_into_core(
                        err,
                        &self.server_name,
                        method,
                    ));
                }
            }
        }
    }

    async fn process_http_json_payload(
        &mut self,
        payload: Value,
        expected_id: Option<i64>,
        context: &str,
    ) -> Result<Option<Value>, CoreError> {
        match payload {
            Value::Array(items) => {
                for item in items {
                    if let Some(result) = self
                        .process_incoming_message(item, expected_id, context)
                        .await?
                    {
                        return Ok(Some(result));
                    }
                }
                Ok(None)
            }
            Value::Object(_) => {
                self.process_incoming_message(payload, expected_id, context)
                    .await
            }
            other => Err(CoreError::Mcp(format!(
                "MCP server '{}' returned an invalid JSON-RPC payload for {context}: {other}",
                self.server_name
            ))),
        }
    }

    async fn process_streamable_http_sse(
        &mut self,
        response: reqwest::Response,
        expected_id: Option<i64>,
        context: &str,
    ) -> Result<Option<Value>, CoreError> {
        let mut stream = response.bytes_stream();
        let mut buffer = Vec::new();
        let idle_timeout = if expected_id.is_some() {
            self.call_timeout
        } else {
            Duration::from_secs(2)
        };

        loop {
            let next_chunk = if context == "tools/call" {
                // Correlated progress can also arrive on the GET notification
                // stream; the outer request owns tool-idle accounting.
                Ok(stream.next().await)
            } else {
                tokio::time::timeout(idle_timeout, stream.next()).await
            };
            let chunk = match next_chunk {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(Some(Err(err))) => {
                    return Err(CoreError::McpTransport(format!(
                        "Failed to read SSE response from MCP server '{}': {err}",
                        self.server_name
                    )))
                }
                Ok(None) => {
                    return Ok(None);
                }
                Err(_) if expected_id.is_none() => return Ok(None),
                Err(_) => return Err(self.transport_timeout_error(context).await),
            };

            if buffer.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(CoreError::Mcp(
                    "MCP SSE event exceeded the response byte limit".into(),
                ));
            }
            buffer.extend_from_slice(&chunk);
            while let Some(raw_event) = drain_sse_event(&mut buffer) {
                if let Some(result) = self
                    .handle_sse_event(&raw_event, expected_id, context)
                    .await?
                {
                    return Ok(Some(result));
                }
            }
        }
    }

    async fn handle_sse_event(
        &mut self,
        raw_event: &str,
        expected_id: Option<i64>,
        context: &str,
    ) -> Result<Option<Value>, CoreError> {
        let (event_name, data) = parse_sse_event(raw_event);
        let trimmed = data.trim();
        if trimmed.is_empty() || matches!(event_name.as_deref(), Some("ping")) {
            return Ok(None);
        }

        let payload: Value = serde_json::from_str(trimmed).map_err(|e| {
            CoreError::Mcp(format!(
                "Failed to parse SSE event from MCP server '{}': {e}",
                self.server_name
            ))
        })?;
        self.process_http_json_payload(payload, expected_id, context)
            .await
    }

    async fn process_incoming_message(
        &mut self,
        message: Value,
        expected_id: Option<i64>,
        context: &str,
    ) -> Result<Option<Value>, CoreError> {
        if !message.is_object() {
            return Err(CoreError::Mcp(format!(
                "MCP server '{}' returned a non-object JSON-RPC message for {context}.",
                self.server_name
            )));
        }

        if is_server_request(&message) {
            self.respond_method_not_found(&message).await?;
            return Ok(None);
        }

        if message.get("method").is_some() {
            self.events().observe(&message);
            return Ok(None);
        }

        let Some(id_value) = message.get("id") else {
            return Ok(None);
        };

        if !matches_request_id(id_value, expected_id) {
            return Ok(None);
        }

        if let Some(error) = message.get("error") {
            return Err(CoreError::McpRpc {
                code: error.get("code").and_then(Value::as_i64).unwrap_or(-32000),
                message: format!(
                    "MCP {context} failed on server '{}': {}",
                    self.server_name,
                    format_json_rpc_error(error)
                ),
            });
        }

        if let Some(result) = message.get("result") {
            return Ok(Some(result.clone()));
        }

        Err(CoreError::Mcp(format!(
            "MCP server '{}' returned a response for {context} without result or error.",
            self.server_name
        )))
    }

    async fn respond_method_not_found(&mut self, message: &Value) -> Result<(), CoreError> {
        let Some(id) = message.get("id") else {
            return Ok(());
        };

        let response = serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id.clone(),
            "error": {
                "code": METHOD_NOT_FOUND_CODE,
                "message": "Method not found"
            }
        });
        match tokio::time::timeout(
            self.call_timeout,
            Box::pin(self.send_transport_message(&response)),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(self.transport_timeout_error("server request reply").await),
        }
    }

    async fn transport_closed_error(&self) -> CoreError {
        match &self.transport {
            Transport::Stdio(transport) => {
                let stderr = transport.stderr_buf.lock().await;
                if stderr.trim().is_empty() {
                    CoreError::McpTransport(format!(
                        "MCP stdio server '{}' closed unexpectedly.",
                        self.server_name
                    ))
                } else {
                    CoreError::McpTransport(format!(
                        "MCP stdio server '{}' closed unexpectedly. stderr:\n{}",
                        self.server_name,
                        stderr.trim()
                    ))
                }
            }
            Transport::LegacySse(transport) => {
                let diagnostics = transport.diagnostics.lock().await;
                if diagnostics.trim().is_empty() {
                    CoreError::McpTransport(format!(
                        "MCP SSE server '{}' closed its event stream unexpectedly.",
                        self.server_name
                    ))
                } else {
                    CoreError::McpTransport(format!(
                        "MCP SSE server '{}' closed its event stream unexpectedly. Details:\n{}",
                        self.server_name,
                        diagnostics.trim()
                    ))
                }
            }
            Transport::StreamableHttp(_) => CoreError::McpTransport(format!(
                "MCP Streamable HTTP server '{}' closed unexpectedly.",
                self.server_name
            )),
        }
    }

    async fn transport_timeout_error(&self, method: &str) -> CoreError {
        CoreError::McpTransport(format!(
            "Timed out waiting for MCP server '{}' to finish {method}.",
            self.server_name
        ))
    }

    async fn build_stdio_transport(
        command: &str,
        args: &[String],
        env: Option<&HashMap<String, String>>,
    ) -> Result<StdioTransport, CoreError> {
        let normalized_command = normalize_stdio_command_name(command);
        let resolved_command = resolve_stdio_command_path(&normalized_command)
            .unwrap_or_else(|| PathBuf::from(&normalized_command));
        let mut process = Command::new(&resolved_command);
        process
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut has_port = false;
        if let Some(env_map) = env {
            for (key, value) in env_map {
                if key.eq_ignore_ascii_case("PORT") {
                    has_port = true;
                }
                process.env(key, value);
            }
        }
        if !has_port {
            process.env("PORT", "0");
        }
        crate::background_process::configure_tokio_background(&mut process);

        let mut child = process.spawn().map_err(|e| {
            CoreError::Mcp(format!(
                "Failed to spawn MCP server command '{command}' (resolved to '{}'): {e}",
                resolved_command.display()
            ))
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CoreError::Mcp("MCP stdio child process has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CoreError::Mcp("MCP stdio child process has no stdout".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CoreError::Mcp("MCP stdio child process has no stderr".into()))?;

        let diagnostics = Arc::new(Mutex::new(String::new()));
        let events = Arc::new(McpClientEvents::default());
        let reader_events = Arc::clone(&events);
        let (stdout_tx, stdout_rx) = mpsc::channel(64);
        let stdout_diagnostics = diagnostics.clone();
        let reader_handle = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout);
            loop {
                match read_bounded_line(&mut lines, MAX_RESPONSE_BYTES).await {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        match serde_json::from_str::<Value>(trimmed) {
                            Ok(value) => {
                                if reader_events.observe(&value) {
                                    continue;
                                }
                                if stdout_tx.send(value).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                append_diagnostics(
                                    &stdout_diagnostics,
                                    &format!("Ignored non-JSON stdout line from MCP server: {line} ({err})"),
                                )
                                .await;
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        append_diagnostics(
                            &stdout_diagnostics,
                            &format!("Failed to read MCP stdout: {err}"),
                        )
                        .await;
                        break;
                    }
                }
            }
        });

        let stderr_diagnostics = diagnostics.clone();
        let stderr_handle = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => append_diagnostics(&stderr_diagnostics, &line).await,
                    Ok(None) => break,
                    Err(err) => {
                        append_diagnostics(
                            &stderr_diagnostics,
                            &format!("Failed to read MCP stderr: {err}"),
                        )
                        .await;
                        break;
                    }
                }
            }
        });

        Ok(StdioTransport {
            child,
            stdin: tokio::io::BufWriter::new(stdin),
            stdout_rx,
            reader_handle,
            stderr_buf: diagnostics,
            stderr_handle,
            events,
        })
    }

    async fn build_legacy_sse_transport(
        url: &str,
        headers: Option<&HashMap<String, String>>,
        auth: Option<&super::oauth::RequestAuth>,
    ) -> Result<LegacySseTransport, CoreError> {
        let client = build_http_client()?;
        let base_url = parse_url(url, "legacy SSE")?;
        let custom_headers = build_header_map(headers)?;

        let mut request_headers = HeaderMap::new();
        apply_custom_headers(&mut request_headers, &custom_headers);
        request_headers.insert(ACCEPT, HeaderValue::from_static(CONTENT_TYPE_SSE));
        request_headers.insert(
            HeaderName::from_static(HEADER_MCP_PROTOCOL_VERSION),
            HeaderValue::from_static(SUPPORTED_PROTOCOL_VERSIONS[0]),
        );

        let response = tokio::time::timeout(
            SSE_CONNECT_TIMEOUT,
            send_authorized(&client, &base_url, request_headers, None, auth),
        )
        .await
        .map_err(|_| {
            CoreError::McpTransport(format!(
                "Timed out connecting to legacy SSE MCP server at {}",
                base_url
            ))
        })??;

        let status = response.status();
        if let Some(error) = http_auth_error(&response) {
            return Err(error);
        }
        if !status.is_success() {
            let body = tokio::time::timeout(
                SSE_CONNECT_TIMEOUT,
                read_bounded_response(response, MAX_DIAGNOSTIC_BYTES),
            )
            .await
            .map_err(|_| {
                CoreError::McpTransport(format!(
                    "Timed out reading legacy SSE MCP error response from {base_url}"
                ))
            })?
            .unwrap_or_default();
            return Err(CoreError::Mcp(format!(
                "Legacy SSE MCP server at {} returned {status}: {}",
                base_url,
                body.trim()
            )));
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !content_type.contains(CONTENT_TYPE_SSE) {
            return Err(CoreError::Mcp(format!(
                "Legacy SSE MCP server at {} did not return an SSE stream.",
                base_url
            )));
        }

        let (events_tx, events_rx) = mpsc::channel(64);
        let diagnostics = Arc::new(Mutex::new(String::new()));
        let (endpoint_tx, endpoint_rx) = oneshot::channel::<Result<Url, CoreError>>();
        let stream_diagnostics = diagnostics.clone();
        let events = Arc::new(McpClientEvents::default());
        let reader_events = Arc::clone(&events);
        let pending_message_url = base_url.clone();
        let stream_handle = tokio::spawn(async move {
            read_legacy_sse_stream(
                response,
                base_url,
                events_tx,
                Some(endpoint_tx),
                stream_diagnostics,
                reader_events,
            )
            .await;
        });
        // Own the reader before waiting for the endpoint. Cancelling connection
        // setup must drop the transport even before the server is initialized.
        let mut transport = LegacySseTransport {
            client,
            message_url: pending_message_url,
            custom_headers,
            events_rx,
            diagnostics,
            stream_handle,
            events,
        };
        transport.message_url = match tokio::time::timeout(SSE_CONNECT_TIMEOUT, endpoint_rx).await {
            Ok(Ok(Ok(url))) => url,
            Ok(Ok(Err(err))) => return Err(err),
            Ok(Err(_)) => {
                return Err(CoreError::McpTransport(
                    "Legacy SSE MCP connection closed before publishing a message endpoint.".into(),
                ));
            }
            Err(_) => {
                return Err(CoreError::McpTransport(
                    "Timed out waiting for a legacy SSE MCP endpoint event.".into(),
                ));
            }
        };
        Ok(transport)
    }

    fn build_streamable_http_transport(
        url: &str,
        headers: Option<&HashMap<String, String>>,
    ) -> Result<StreamableHttpTransport, CoreError> {
        Ok(StreamableHttpTransport {
            client: build_http_client()?,
            endpoint_url: parse_url(url, "Streamable HTTP")?,
            custom_headers: build_header_map(headers)?,
            session_id: None,
            events: Arc::new(McpClientEvents::default()),
            notification_reader: None,
            notification_rx: None,
        })
    }

    async fn open_streamable_http_get(&mut self) -> Result<reqwest::Response, StreamablePostError> {
        let (client, endpoint_url, custom_headers, session_id) = match &self.transport {
            Transport::StreamableHttp(transport) => (
                transport.client.clone(),
                transport.endpoint_url.clone(),
                transport.custom_headers.clone(),
                transport.session_id.clone(),
            ),
            _ => {
                return Err(StreamablePostError::Core(CoreError::Internal(
                    "open_streamable_http_get called for non-HTTP transport".into(),
                )))
            }
        };

        let mut headers = HeaderMap::new();
        apply_custom_headers(&mut headers, &custom_headers);
        headers.insert(ACCEPT, HeaderValue::from_static(CONTENT_TYPE_SSE));
        headers.insert(
            HeaderName::from_static(HEADER_MCP_PROTOCOL_VERSION),
            HeaderValue::from_str(&self.protocol_version).map_err(|e| {
                StreamablePostError::Core(CoreError::Mcp(format!(
                    "Invalid MCP protocol version header '{}': {e}",
                    self.protocol_version
                )))
            })?,
        );
        if let Some(session_id) = session_id.as_deref() {
            headers.insert(
                HeaderName::from_static(HEADER_MCP_SESSION_ID),
                HeaderValue::from_str(session_id).map_err(|e| {
                    StreamablePostError::Core(CoreError::Mcp(format!(
                        "Invalid MCP session id '{session_id}': {e}",
                    )))
                })?,
            );
        }

        let response = tokio::time::timeout(
            self.call_timeout,
            send_authorized(&client, &endpoint_url, headers, None, self.auth.as_ref()),
        )
        .await
        .map_err(|_| {
            StreamablePostError::Core(CoreError::McpTransport(format!(
                "Timed out opening the Streamable HTTP event stream at {endpoint_url}"
            )))
        })?
        .map_err(StreamablePostError::Core)?;

        let status = response.status();
        if status == StatusCode::NOT_FOUND && session_id.is_some() {
            return Err(StreamablePostError::SessionExpired);
        }
        if let Some(error) = http_auth_error(&response) {
            return Err(StreamablePostError::Core(error));
        }
        if !status.is_success() {
            let body = read_bounded_response(response, MAX_DIAGNOSTIC_BYTES)
                .await
                .unwrap_or_else(|error| error.to_string());
            return Err(StreamablePostError::Core(mcp_http_status_error(
                status,
                format!(
                    "Streamable HTTP GET {} returned {status}: {}",
                    endpoint_url,
                    body.trim()
                ),
            )));
        }

        self.update_session_id(response.headers())?;
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !content_type.contains(CONTENT_TYPE_SSE) {
            return Err(StreamablePostError::Core(CoreError::Mcp(format!(
                "Streamable HTTP GET {} did not return an SSE stream.",
                endpoint_url
            ))));
        }

        Ok(response)
    }

    async fn post_legacy_sse_message(&mut self, payload: &Value) -> Result<(), CoreError> {
        let (client, message_url, custom_headers) = match &self.transport {
            Transport::LegacySse(transport) => (
                transport.client.clone(),
                transport.message_url.clone(),
                transport.custom_headers.clone(),
            ),
            _ => {
                return Err(CoreError::Internal(
                    "post_legacy_sse_message called for non-SSE transport".into(),
                ))
            }
        };

        let mut headers = HeaderMap::new();
        apply_custom_headers(&mut headers, &custom_headers);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_JSON));
        headers.insert(ACCEPT, HeaderValue::from_static(CONTENT_TYPE_JSON));
        headers.insert(
            HeaderName::from_static(HEADER_MCP_PROTOCOL_VERSION),
            HeaderValue::from_str(&self.protocol_version).map_err(|e| {
                CoreError::Mcp(format!(
                    "Invalid MCP protocol version header '{}': {e}",
                    self.protocol_version
                ))
            })?,
        );

        let sending = send_authorized(
            &client,
            &message_url,
            headers,
            Some(payload),
            self.auth.as_ref(),
        );
        let response = if payload["method"] == "tools/call" {
            // A legacy server may report progress before acknowledging its POST.
            // The correlated-progress idle owner covers the response wait.
            Ok(sending.await)
        } else {
            tokio::time::timeout(self.call_timeout, sending).await
        }
        .map_err(|_| {
            CoreError::McpTransport(format!(
                "Timed out sending a request to legacy SSE MCP server at {message_url}"
            ))
        })??;

        if let Some(error) = http_auth_error(&response) {
            return Err(error);
        }
        if !response.status().is_success() {
            let status = response.status();
            let body = read_bounded_response(response, MAX_DIAGNOSTIC_BYTES)
                .await
                .unwrap_or_else(|error| error.to_string());
            return Err(mcp_http_status_error(
                status,
                format!(
                    "Legacy SSE MCP server at {message_url} returned {status}: {}",
                    body.trim()
                ),
            ));
        }

        Ok(())
    }

    async fn post_streamable_http(
        &mut self,
        payload: &Value,
    ) -> Result<StreamablePostOutcome, StreamablePostError> {
        let (client, endpoint_url, custom_headers, session_id) = match &self.transport {
            Transport::StreamableHttp(transport) => (
                transport.client.clone(),
                transport.endpoint_url.clone(),
                transport.custom_headers.clone(),
                transport.session_id.clone(),
            ),
            _ => {
                return Err(StreamablePostError::Core(CoreError::Internal(
                    "post_streamable_http called for non-HTTP transport".into(),
                )))
            }
        };

        let mut headers = HeaderMap::new();
        apply_custom_headers(&mut headers, &custom_headers);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_JSON));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/event-stream"),
        );
        headers.insert(
            HeaderName::from_static(HEADER_MCP_PROTOCOL_VERSION),
            HeaderValue::from_str(&self.protocol_version).map_err(|e| {
                StreamablePostError::Core(CoreError::Mcp(format!(
                    "Invalid MCP protocol version header '{}': {e}",
                    self.protocol_version
                )))
            })?,
        );
        if let Some(session_id) = session_id.as_deref() {
            headers.insert(
                HeaderName::from_static(HEADER_MCP_SESSION_ID),
                HeaderValue::from_str(session_id).map_err(|e| {
                    StreamablePostError::Core(CoreError::Mcp(format!(
                        "Invalid MCP session id '{session_id}': {e}",
                    )))
                })?,
            );
        }

        let sending = send_authorized(
            &client,
            &endpoint_url,
            headers,
            Some(payload),
            self.auth.as_ref(),
        );
        let response = if payload["method"] == "tools/call" {
            // Progress can arrive on GET while a server computes its JSON POST response.
            // The correlated-progress idle owner covers the response wait.
            Ok(sending.await)
        } else {
            tokio::time::timeout(self.call_timeout, sending).await
        }
        .map_err(|_| {
            StreamablePostError::Core(CoreError::McpTransport(format!(
                "Timed out sending a Streamable HTTP request to {endpoint_url}"
            )))
        })?
        .map_err(StreamablePostError::Core)?;

        let status = response.status();
        if status == StatusCode::NOT_FOUND && session_id.is_some() {
            return Err(StreamablePostError::SessionExpired);
        }
        if let Some(error) = http_auth_error(&response) {
            return Err(StreamablePostError::Core(error));
        }
        if !status.is_success() {
            let body = read_bounded_response(response, MAX_DIAGNOSTIC_BYTES)
                .await
                .unwrap_or_else(|error| error.to_string());
            return Err(StreamablePostError::Core(mcp_http_status_error(
                status,
                format!(
                    "Streamable HTTP endpoint {} returned {status}: {}",
                    endpoint_url,
                    body.trim()
                ),
            )));
        }

        self.update_session_id(response.headers())?;
        if status == StatusCode::ACCEPTED || status == StatusCode::NO_CONTENT {
            return Ok(StreamablePostOutcome::Accepted);
        }

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if content_type.contains(CONTENT_TYPE_SSE) {
            return Ok(StreamablePostOutcome::Sse(response));
        }

        let body = read_bounded_response(response, MAX_RESPONSE_BYTES).await?;
        if body.trim().is_empty() {
            return Ok(StreamablePostOutcome::Accepted);
        }

        let json = serde_json::from_str(&body).map_err(|e| {
            StreamablePostError::Core(CoreError::Mcp(format!(
                "Failed to parse Streamable HTTP response from {} as JSON: {e}",
                endpoint_url
            )))
        })?;
        Ok(StreamablePostOutcome::Json(json))
    }

    async fn reset_streamable_http_session(&mut self) -> Result<(), CoreError> {
        if let Transport::StreamableHttp(transport) = &mut self.transport {
            if let Some(reader) = transport.notification_reader.take() {
                reader.abort();
            }
            transport.notification_rx = None;
        }
        let (client, endpoint_url, custom_headers, session_id) = match &self.transport {
            Transport::StreamableHttp(transport) => (
                transport.client.clone(),
                transport.endpoint_url.clone(),
                transport.custom_headers.clone(),
                transport.session_id.clone(),
            ),
            _ => return Ok(()),
        };

        if let Some(session_id) = session_id.as_deref() {
            let mut headers = HeaderMap::new();
            apply_custom_headers(&mut headers, &custom_headers);
            headers.insert(
                HeaderName::from_static(HEADER_MCP_PROTOCOL_VERSION),
                HeaderValue::from_str(&self.protocol_version).map_err(|e| {
                    CoreError::Mcp(format!(
                        "Invalid MCP protocol version header '{}': {e}",
                        self.protocol_version
                    ))
                })?,
            );
            headers.insert(
                HeaderName::from_static(HEADER_MCP_SESSION_ID),
                HeaderValue::from_str(session_id).map_err(|e| {
                    CoreError::Mcp(format!("Invalid MCP session id '{session_id}': {e}"))
                })?,
            );

            let _ = tokio::time::timeout(self.call_timeout, async {
                let response = client
                    .delete(endpoint_url)
                    .headers(headers)
                    .send()
                    .await
                    .map_err(|error| CoreError::McpTransport(error.to_string()))?;
                read_bounded_response(response, MAX_DIAGNOSTIC_BYTES).await
            })
            .await;
        }

        if let Transport::StreamableHttp(transport) = &mut self.transport {
            transport.session_id = None;
        }
        Ok(())
    }

    fn update_session_id(&mut self, headers: &HeaderMap) -> Result<(), CoreError> {
        let Some(session_id) = headers.get(HEADER_MCP_SESSION_ID) else {
            return Ok(());
        };
        let session_id = session_id.to_str().map_err(|e| {
            CoreError::Mcp(format!(
                "MCP server returned an invalid session id header: {e}"
            ))
        })?;
        if session_id.trim().is_empty() {
            return Ok(());
        }

        if let Transport::StreamableHttp(transport) = &mut self.transport {
            transport.session_id = Some(session_id.to_string());
        }
        Ok(())
    }
}

async fn send_authorized(
    client: &HttpClient,
    url: &Url,
    mut headers: HeaderMap,
    payload: Option<&Value>,
    auth: Option<&super::oauth::RequestAuth>,
) -> Result<reqwest::Response, CoreError> {
    let safe_retry = payload.is_none()
        || matches!(
            payload
                .and_then(|p| p.get("method"))
                .and_then(Value::as_str),
            Some(
                "initialize"
                    | "tools/list"
                    | "resources/list"
                    | "resources/templates/list"
                    | "prompts/list"
                    | "resources/read"
                    | "prompts/get"
            )
        );
    for attempt in 0..=1 {
        if let Some(auth) = auth {
            auth.validate_target(url)?;
            auth.apply(&mut headers, attempt == 1).await?;
        }
        let request = if let Some(payload) = payload {
            client.post(url.clone()).json(payload)
        } else {
            client.get(url.clone())
        };
        let response = request.headers(headers.clone()).send().await.map_err(|_| {
            CoreError::McpTransport(
                "MCP HTTP request failed; its effect may be unknown. No operation was replayed."
                    .into(),
            )
        })?;
        if response.status() == StatusCode::UNAUTHORIZED
            && auth.is_some()
            && safe_retry
            && attempt == 0
        {
            continue;
        }
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            if let Some(auth) = auth {
                auth.note_rejection(
                    response.status().as_u16(),
                    super::oauth::bearer_challenge(response.headers()).as_ref(),
                );
            }
        }
        return Ok(response);
    }
    unreachable!("bounded authorization retry")
}

fn http_auth_error(response: &reqwest::Response) -> Option<CoreError> {
    matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    )
    .then(|| CoreError::McpHttpAuth {
        status: response.status().as_u16(),
        challenge: super::oauth::bearer_challenge(response.headers()),
    })
}

fn build_http_client() -> Result<HttpClient, CoreError> {
    HttpClient::builder()
        .connect_timeout(SSE_CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| CoreError::Mcp(format!("Failed to build HTTP client for MCP: {e}")))
}

fn mcp_http_status_error(status: StatusCode, message: String) -> CoreError {
    if status.is_server_error() {
        CoreError::McpTransport(message)
    } else {
        CoreError::Mcp(message)
    }
}

fn build_header_map(headers: Option<&HashMap<String, String>>) -> Result<HeaderMap, CoreError> {
    let mut map = HeaderMap::new();
    if let Some(headers) = headers {
        for (name, value) in headers {
            let header_name = HeaderName::from_bytes(name.trim().as_bytes()).map_err(|e| {
                CoreError::InvalidInput(format!("Invalid HTTP header name '{name}': {e}"))
            })?;
            let header_value = HeaderValue::from_str(value).map_err(|e| {
                CoreError::InvalidInput(format!("Invalid HTTP header value for '{name}': {e}"))
            })?;
            map.insert(header_name, header_value);
        }
    }
    Ok(map)
}

fn apply_custom_headers(target: &mut HeaderMap, custom_headers: &HeaderMap) {
    for (name, value) in custom_headers {
        target.insert(name.clone(), value.clone());
    }
}

fn parse_url(url: &str, transport_name: &str) -> Result<Url, CoreError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidInput(format!(
            "{transport_name} MCP transport requires a URL"
        )));
    }

    let parsed = Url::parse(trimmed).map_err(|e| {
        CoreError::InvalidInput(format!(
            "Invalid URL for {transport_name} MCP transport: {e}"
        ))
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        other => Err(CoreError::InvalidInput(format!(
            "{transport_name} MCP transport only supports http/https URLs, got '{other}'"
        ))),
    }
}

fn resolve_sse_endpoint(base_url: &Url, endpoint: &str) -> Result<Url, CoreError> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return Err(CoreError::Mcp(
            "Legacy SSE MCP endpoint event was empty".into(),
        ));
    }

    base_url.join(trimmed).or_else(|_| {
        Url::parse(trimmed).map_err(|e| {
            CoreError::Mcp(format!(
                "Legacy SSE MCP endpoint '{trimmed}' is invalid: {e}"
            ))
        })
    })
}

async fn read_legacy_sse_stream(
    response: reqwest::Response,
    base_url: Url,
    sender: mpsc::Sender<Value>,
    mut endpoint_tx: Option<oneshot::Sender<Result<Url, CoreError>>>,
    diagnostics: Arc<Mutex<String>>,
    events: Arc<McpClientEvents>,
) {
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(chunk) => chunk,
            Err(err) => {
                let error = CoreError::Mcp(format!(
                    "Failed to read legacy SSE stream from {base_url}: {err}"
                ));
                append_diagnostics(&diagnostics, &error.to_string()).await;
                if let Some(tx) = endpoint_tx.take() {
                    let _ = tx.send(Err(error));
                }
                return;
            }
        };

        if buffer.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            append_diagnostics(
                &diagnostics,
                "Legacy SSE MCP event exceeded the response byte limit",
            )
            .await;
            return;
        }
        buffer.extend_from_slice(&chunk);
        while let Some(raw_event) = drain_sse_event(&mut buffer) {
            if let Err(err) =
                process_legacy_sse_event(&raw_event, &base_url, &sender, &mut endpoint_tx, &events)
                    .await
            {
                append_diagnostics(&diagnostics, &err.to_string()).await;
                if let Some(tx) = endpoint_tx.take() {
                    let _ = tx.send(Err(err));
                }
                return;
            }
        }
    }

    if let Some(tx) = endpoint_tx.take() {
        let _ = tx.send(Err(CoreError::Mcp(
            "Legacy SSE MCP stream closed before publishing a message endpoint.".into(),
        )));
    }
}

async fn process_legacy_sse_event(
    raw_event: &str,
    base_url: &Url,
    sender: &mpsc::Sender<Value>,
    endpoint_tx: &mut Option<oneshot::Sender<Result<Url, CoreError>>>,
    events: &McpClientEvents,
) -> Result<(), CoreError> {
    let (event_name, data) = parse_sse_event(raw_event);
    let trimmed = data.trim();
    if trimmed.is_empty() || matches!(event_name.as_deref(), Some("ping")) {
        return Ok(());
    }

    if matches!(event_name.as_deref(), Some("endpoint")) {
        let endpoint = resolve_sse_endpoint(base_url, trimmed)?;
        if let Some(tx) = endpoint_tx.take() {
            let _ = tx.send(Ok(endpoint));
        }
        return Ok(());
    }

    let message = serde_json::from_str::<Value>(trimmed).map_err(|e| {
        CoreError::Mcp(format!(
            "Failed to parse legacy SSE message '{trimmed}' as JSON: {e}"
        ))
    })?;
    if events.observe(&message) {
        return Ok(());
    }
    sender
        .send(message)
        .await
        .map_err(|_| CoreError::Mcp("Legacy SSE MCP receiver was dropped".into()))
}

async fn read_bounded_response(
    response: reqwest::Response,
    limit: usize,
) -> Result<String, CoreError> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            CoreError::McpTransport(format!("Failed to read MCP response: {error}"))
        })?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(CoreError::Mcp(format!(
                "MCP response exceeded the {limit}-byte limit; no tool call was retried"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map_err(|error| CoreError::Mcp(format!("MCP response was not valid UTF-8: {error}")))
}

async fn read_bounded_line(
    reader: &mut (impl AsyncBufRead + Unpin),
    limit: usize,
) -> std::io::Result<Option<String>> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            break;
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(count) > limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "MCP stdout line exceeded the response byte limit",
            ));
        }
        let ended = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if ended {
            break;
        }
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

async fn append_diagnostics(buffer: &Arc<Mutex<String>>, line: &str) {
    let mut guard = buffer.lock().await;
    if line.len() >= MAX_DIAGNOSTIC_BYTES {
        let mut start = line.len() - MAX_DIAGNOSTIC_BYTES;
        while !line.is_char_boundary(start) {
            start += 1;
        }
        guard.clear();
        guard.push_str(&line[start..]);
        return;
    }
    let separator_bytes = usize::from(!guard.is_empty());
    let excess = (guard.len() + separator_bytes + line.len()).saturating_sub(MAX_DIAGNOSTIC_BYTES);
    if excess > 0 {
        let mut start = excess.min(guard.len());
        while !guard.is_char_boundary(start) {
            start += 1;
        }
        guard.drain(..start);
    }
    if !guard.is_empty() {
        guard.push('\n');
    }
    guard.push_str(line);
}

fn drain_sse_event(buffer: &mut Vec<u8>) -> Option<String> {
    let (index, delimiter_len) = find_sse_delimiter(buffer)?;
    let drained = buffer.drain(..index + delimiter_len).collect::<Vec<_>>();
    let payload = &drained[..index];
    Some(String::from_utf8_lossy(payload).into_owned())
}

fn find_sse_delimiter(buffer: &[u8]) -> Option<(usize, usize)> {
    if buffer.len() < 2 {
        return None;
    }

    for index in 0..buffer.len() - 1 {
        if buffer[index] == b'\n' && buffer[index + 1] == b'\n' {
            return Some((index, 2));
        }
        if index + 3 < buffer.len()
            && buffer[index] == b'\r'
            && buffer[index + 1] == b'\n'
            && buffer[index + 2] == b'\r'
            && buffer[index + 3] == b'\n'
        {
            return Some((index, 4));
        }
    }

    None
}

fn parse_sse_event(raw_event: &str) -> (Option<String>, String) {
    let mut event_name = None;
    let mut data_lines = Vec::new();

    for raw_line in raw_event.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }

        if let Some(rest) = line.strip_prefix("event:") {
            event_name = Some(rest.trim().to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }

    (event_name, data_lines.join("\n"))
}

fn is_server_request(message: &Value) -> bool {
    message.get("method").is_some() && message.get("id").is_some()
}

fn matches_request_id(id_value: &Value, expected_id: Option<i64>) -> bool {
    let Some(expected_id) = expected_id else {
        return false;
    };

    id_value
        .as_i64()
        .map(|value| value == expected_id)
        .or_else(|| {
            id_value
                .as_str()
                .and_then(|value| value.parse::<i64>().ok())
                .map(|value| value == expected_id)
        })
        .unwrap_or(false)
}

fn format_json_rpc_error(error: &Value) -> String {
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_string());
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown MCP error");
    let data = error.get("data").and_then(|value| {
        if value.is_null() {
            None
        } else if let Some(text) = value.as_str() {
            Some(text.to_string())
        } else {
            Some(value.to_string())
        }
    });

    match data {
        Some(data) if !data.is_empty() => format!("code {code}: {message} ({data})"),
        _ => format!("code {code}: {message}"),
    }
}

fn streamable_post_error_into_core(
    error: StreamablePostError,
    server_name: &str,
    method: &str,
) -> CoreError {
    match error {
        StreamablePostError::Core(error) => error,
        StreamablePostError::SessionExpired => CoreError::McpTransport(format!(
            "Streamable HTTP session for MCP server '{server_name}' expired while processing {method}.",
        )),
    }
}

fn normalize_stdio_command_name(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.len() >= 2 {
        let first = trimmed.as_bytes()[0];
        let last = trimmed.as_bytes()[trimmed.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return trimmed[1..trimmed.len() - 1].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn resolve_stdio_command_path(command: &str) -> Option<PathBuf> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = Path::new(trimmed);
    if path.is_absolute() || has_directory_component(trimmed) {
        return resolve_stdio_candidate(path);
    }

    let path_dirs: Vec<PathBuf> = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect())
        .unwrap_or_default();
    resolve_stdio_in_dirs(trimmed, &path_dirs, &windows_pathexts())
}

fn resolve_stdio_in_dirs(
    command: &str,
    path_dirs: &[PathBuf],
    pathexts: &[String],
) -> Option<PathBuf> {
    for dir in path_dirs {
        if let Some(candidate) = resolve_stdio_candidate_with_pathext(&dir.join(command), pathexts)
        {
            return Some(candidate);
        }
    }
    None
}

fn resolve_stdio_candidate(path: &Path) -> Option<PathBuf> {
    resolve_stdio_candidate_with_pathext(path, &windows_pathexts())
}

fn resolve_stdio_candidate_with_pathext(path: &Path, _pathexts: &[String]) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if path.extension().is_none() {
            for extension in _pathexts {
                let mut candidate = path.as_os_str().to_os_string();
                candidate.push(extension);
                let candidate = PathBuf::from(candidate);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    if path.is_file() {
        return Some(path.to_path_buf());
    }

    None
}

fn has_directory_component(command: &str) -> bool {
    command.contains(std::path::MAIN_SEPARATOR)
        || (cfg!(windows) && command.contains('/'))
        || (cfg!(windows) && command.contains('\\'))
}

fn windows_pathexts() -> Vec<String> {
    #[cfg(windows)]
    {
        let raw = env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
        raw.to_string_lossy()
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.starts_with('.') {
                    value.to_string()
                } else {
                    format!(".{value}")
                }
            })
            .collect()
    }

    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[cfg(windows)]
    use std::fs;

    use serde_json::json;
    #[cfg(windows)]
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{mpsc, Mutex};

    #[derive(Debug)]
    struct TestHttpRequest {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    #[tokio::test]
    async fn diagnostics_keep_a_bounded_utf8_tail() {
        let diagnostics = Arc::new(Mutex::new(String::new()));
        for _ in 0..256 {
            append_diagnostics(&diagnostics, &"诊断".repeat(128)).await;
        }
        append_diagnostics(&diagnostics, "latest failure").await;
        let retained = diagnostics.lock().await;
        assert!(
            retained.len() <= 16 * 1024,
            "MCP diagnostics grew without a byte limit"
        );
        assert!(retained.ends_with("latest failure"));
        drop(retained);

        append_diagnostics(
            &diagnostics,
            &format!("{}single-line-tail", "诊断".repeat(8_192)),
        )
        .await;
        let retained = diagnostics.lock().await;
        assert!(retained.len() <= 16 * 1024);
        assert!(retained.ends_with("single-line-tail"));
    }

    #[tokio::test]
    async fn request_deadline_includes_stalled_http_response_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request(&mut stream).await.unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{").await.unwrap();
            std::future::pending::<()>().await;
        });
        let transport = McpClient::build_streamable_http_transport(&url, None).unwrap();
        let mut client = McpClient {
            transport: Transport::StreamableHttp(transport),
            request_id: AtomicI64::new(1),
            server_name: "stalled-body".into(),
            protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].into(),
            server_capabilities: serde_json::json!({}),
            call_timeout: Duration::from_millis(50),
            auth: None,
        };
        let result = tokio::time::timeout(Duration::from_millis(500), client.list_tools()).await;
        server.abort();
        assert!(
            matches!(result, Ok(Err(CoreError::McpTransport(_)))),
            "the configured request deadline must include the HTTP body: {result:?}"
        );
    }

    #[tokio::test]
    async fn transport_failure_does_not_repeat_protocol_negotiation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_attempts = Arc::clone(&attempts);
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                read_http_request(&mut stream).await.unwrap();
                observed_attempts.fetch_add(1, Ordering::SeqCst);
                write_text_response(
                    &mut stream,
                    "503 Service Unavailable",
                    None,
                    "connector is unavailable",
                )
                .await
                .unwrap();
            }
        });
        let transport = McpClient::build_streamable_http_transport(&url, None).unwrap();
        let mut client = McpClient {
            transport: Transport::StreamableHttp(transport),
            request_id: AtomicI64::new(1),
            server_name: "unavailable-handshake".into(),
            protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].into(),
            server_capabilities: serde_json::json!({}),
            call_timeout: DEFAULT_TIMEOUT,
            auth: None,
        };
        let result = client.initialize_handshake().await;
        server.abort();
        assert!(matches!(result, Err(CoreError::McpTransport(message)) if message.contains("503")));
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "a transport failure was retried as a protocol version mismatch"
        );
    }

    #[tokio::test]
    async fn protocol_rejection_still_negotiates_an_older_version() {
        for http_rejection in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/mcp", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let mut attempted_versions = Vec::new();
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let request = read_http_request(&mut stream).await.unwrap();
                    let request: Value = serde_json::from_slice(&request.body).unwrap();
                    if request["method"] == "notifications/initialized" {
                        write_empty_response(&mut stream, "202 Accepted", None)
                            .await
                            .unwrap();
                        return attempted_versions;
                    }
                    assert_eq!(request["method"], "initialize");
                    attempted_versions.push(request["params"]["protocolVersion"].clone());
                    if attempted_versions.len() == 1 {
                        let status = if http_rejection {
                            "400 Bad Request"
                        } else {
                            "200 OK"
                        };
                        write_json_response(
                            &mut stream,
                            status,
                            None,
                            &json!({
                                "jsonrpc": "2.0", "id": request["id"],
                                "error": { "code": -32602, "message": "Unsupported protocol version" }
                            }),
                        )
                        .await
                        .unwrap();
                    } else {
                        write_json_response(
                            &mut stream,
                            "200 OK",
                            None,
                            &json!({
                                "jsonrpc": "2.0", "id": request["id"],
                                "result": {
                                    "protocolVersion": request["params"]["protocolVersion"],
                                    "capabilities": {},
                                    "serverInfo": { "name": "older-server", "version": "1.0" }
                                }
                            }),
                        )
                        .await
                        .unwrap();
                    }
                    // Keep the one-request socket alive until the client has
                    // consumed the response. Otherwise an immediate drop can
                    // hide accidental pooling of a connection we cannot serve.
                    let mut next_request = [0u8; 1];
                    assert_eq!(
                        tokio::time::timeout(
                            Duration::from_secs(5),
                            stream.read(&mut next_request),
                        )
                        .await
                        .expect("client must close the one-request fixture connection")
                        .unwrap(),
                        0,
                        "client reused a connection that the one-request fixture cannot serve"
                    );
                }
            });
            let client = McpClient::connect_streamable_http(&url, None, "older-server")
                .await
                .unwrap();
            assert_eq!(client.protocol_version, SUPPORTED_PROTOCOL_VERSIONS[1]);
            assert_eq!(
                server.await.unwrap(),
                vec![
                    json!(SUPPORTED_PROTOCOL_VERSIONS[0]),
                    json!(SUPPORTED_PROTOCOL_VERSIONS[1])
                ]
            );
        }
    }

    #[tokio::test]
    async fn sse_heartbeats_do_not_extend_the_request_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_http_request(&mut stream).await.unwrap();
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
            loop {
                if stream.write_all(b"8\r\n: ping\n\n\r\n").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let transport = McpClient::build_streamable_http_transport(&url, None).unwrap();
        let mut client = McpClient {
            transport: Transport::StreamableHttp(transport),
            request_id: AtomicI64::new(1),
            server_name: "heartbeat-only".into(),
            protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].into(),
            server_capabilities: serde_json::json!({}),
            call_timeout: Duration::from_millis(50),
            auth: None,
        };
        let result = tokio::time::timeout(Duration::from_millis(500), client.list_tools()).await;
        server.abort();
        assert!(
            matches!(result, Ok(Err(CoreError::McpTransport(_)))),
            "heartbeats kept a request alive past its deadline: {result:?}"
        );
    }

    #[tokio::test]
    async fn initialized_client_receives_get_progress_without_list_changed_capability() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let (token_tx, token_rx) = tokio::sync::watch::channel(None::<Value>);
        let server = tokio::spawn(async move {
            let mut handlers = Vec::new();
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let token_tx = token_tx.clone();
                let mut token_rx = token_rx.clone();
                handlers.push(tokio::spawn(async move {
                    let request = read_http_request(&mut stream).await.unwrap();
                    if request.method == "GET" {
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
                        while token_rx.borrow().is_none() { token_rx.changed().await.unwrap(); }
                        let token = token_rx.borrow().clone().unwrap();
                        for value in 0..10 {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            let event = json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":token,"progress":value}});
                            if stream.write_all(format!("data: {event}\n\n").as_bytes()).await.is_err() { return; }
                        }
                    } else {
                        let payload: Value = serde_json::from_slice(&request.body).unwrap();
                        match payload["method"].as_str().unwrap() {
                            "initialize" => {
                                let response = json!({"jsonrpc":"2.0","id":payload["id"],"result":{
                                    "protocolVersion":SUPPORTED_PROTOCOL_VERSIONS[0],
                                    "capabilities":{"tools":{}},
                                    "serverInfo":{"name":"get-progress-post-json","version":"1"}
                                }});
                                write_json_response(&mut stream, "200 OK", None, &response).await.unwrap();
                            }
                            "notifications/initialized" => {
                                write_empty_response(&mut stream, "202 Accepted", None).await.unwrap();
                            }
                            "tools/call" => {
                                let token = payload["params"]["_meta"]["progressToken"].clone();
                                assert_eq!(token, payload["id"]);
                                token_tx.send(Some(token)).unwrap();
                                tokio::time::sleep(Duration::from_millis(1100)).await;
                                let response = json!({"jsonrpc":"2.0","id":payload["id"],"result":{"content":[{"type":"text","text":"finished"}]}});
                                let _ = write_json_response(&mut stream, "200 OK", None, &response).await;
                            }
                            method => panic!("unexpected method {method}"),
                        }
                    }
                }));
            }
            for handler in handlers {
                handler.await.unwrap();
            }
        });
        let mut client = McpClient::connect_streamable_http(&url, None, "get-progress-post-json")
            .await
            .unwrap();
        client.set_call_timeout(Duration::from_millis(500));
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            client.call_tool("long_task", json!({})),
        )
        .await;
        client.shutdown().await.unwrap();
        if !matches!(result, Ok(Ok(_))) {
            server.abort();
        }
        assert!(matches!(result, Ok(Ok(_))), "{result:?}");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn tool_progress_renews_http_idle_deadlines_but_duplicate_progress_does_not() {
        for advances in [true, false] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/mcp", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut stream).await.unwrap();
                let payload: Value = serde_json::from_slice(&request.body).unwrap();
                let token = payload["params"]["_meta"]["progressToken"].clone();
                assert_eq!(token, payload["id"]);
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
                for value in 0..10 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let event = json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":token,"progress":if advances { value } else { 0 }}});
                    if stream
                        .write_all(format!("data: {event}\n\n").as_bytes())
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                let response = json!({"jsonrpc":"2.0","id":payload["id"],"result":{"content":[{"type":"text","text":"finished"}]}});
                let _ = stream
                    .write_all(format!("data: {response}\n\n").as_bytes())
                    .await;
            });
            let transport = McpClient::build_streamable_http_transport(&url, None).unwrap();
            let mut client = McpClient {
                transport: Transport::StreamableHttp(transport),
                request_id: AtomicI64::new(1),
                server_name: "tool-progress".into(),
                protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].into(),
                server_capabilities: json!({}),
                call_timeout: Duration::from_millis(500),
                auth: None,
            };
            let result = client.call_tool("long_task", json!({})).await;
            if advances {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(
                    matches!(result, Err(CoreError::McpTransport(_))),
                    "{result:?}"
                );
            }
            server.abort();
        }
    }

    #[test]
    #[ignore = "subprocess fixture invoked only by the stdio progress test"]
    fn stdio_progress_fixture() {
        use std::io::{BufRead, Write};
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).unwrap();
        let payload: Value = serde_json::from_str(&line).unwrap();
        let token = &payload["params"]["_meta"]["progressToken"];
        assert_eq!(token, &payload["id"]);
        for value in 0..10 {
            std::thread::sleep(Duration::from_millis(100));
            println!(
                "{}",
                json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":token,"progress":value}})
            );
            std::io::stdout().flush().unwrap();
        }
        println!(
            "{}",
            json!({"jsonrpc":"2.0","id":payload["id"],"result":{"content":[{"type":"text","text":"finished"}]}})
        );
        std::io::stdout().flush().unwrap();
    }

    #[tokio::test]
    async fn tool_progress_renews_stdio_idle_deadline() {
        let command = std::env::current_exe().unwrap();
        let transport = McpClient::build_stdio_transport(
            command.to_str().unwrap(),
            &[
                "--exact".into(),
                "mcp::client::tests::stdio_progress_fixture".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            None,
        )
        .await
        .unwrap();
        let mut client = McpClient {
            transport: Transport::Stdio(transport),
            request_id: AtomicI64::new(1),
            server_name: "stdio-progress".into(),
            protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].into(),
            server_capabilities: json!({}),
            call_timeout: Duration::from_millis(500),
            auth: None,
        };
        let result = client.call_tool("long_task", json!({})).await;
        client.shutdown().await.unwrap();
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn shutdown_deadline_includes_stalled_http_delete_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await.unwrap();
            assert_eq!(request.method, "DELETE");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let mut transport = McpClient::build_streamable_http_transport(&url, None).unwrap();
        transport.session_id = Some("closing-session".into());
        let mut client = McpClient {
            transport: Transport::StreamableHttp(transport),
            request_id: AtomicI64::new(1),
            server_name: "stalled-shutdown".into(),
            protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].into(),
            server_capabilities: serde_json::json!({}),
            call_timeout: Duration::from_millis(50),
            auth: None,
        };
        let result = tokio::time::timeout(Duration::from_millis(500), client.shutdown()).await;
        server.abort();
        assert!(
            matches!(result, Ok(Ok(()))),
            "MCP shutdown waited indefinitely for DELETE response body: {result:?}"
        );
        let Transport::StreamableHttp(transport) = &client.transport else {
            unreachable!()
        };
        assert!(transport.session_id.is_none());
    }

    #[tokio::test]
    async fn dropping_legacy_transport_stops_its_idle_reader() {
        let stream_handle = tokio::spawn(std::future::pending::<()>());
        let reader = stream_handle.abort_handle();
        let (_events_tx, events_rx) = mpsc::channel(1);
        let transport = LegacySseTransport {
            client: build_http_client().unwrap(),
            message_url: Url::parse("http://127.0.0.1/mcp").unwrap(),
            custom_headers: HeaderMap::new(),
            events_rx,
            diagnostics: Arc::new(Mutex::new(String::new())),
            stream_handle,
            events: Arc::new(McpClientEvents::default()),
        };
        drop(transport);
        tokio::task::yield_now().await;
        let stopped = reader.is_finished();
        reader.abort();
        assert!(
            stopped,
            "dropping a failed or cancelled connection detached its reader"
        );
    }

    #[tokio::test]
    async fn cancelling_legacy_connection_before_endpoint_closes_the_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/sse", listener.local_addr().unwrap());
        let connecting =
            tokio::spawn(
                async move { McpClient::connect_sse(&url, None, "pending-endpoint").await },
            );
        let (mut stream, _) = listener.accept().await.unwrap();
        read_http_request(&mut stream).await.unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n8\r\n: ping\n\n\r\n").await.unwrap();
        // Allow the GET response to enter its endpoint-wait stage. The server
        // intentionally never sends an endpoint event or completes its body.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!connecting.is_finished());
        connecting.abort();
        assert!(matches!(connecting.await, Err(error) if error.is_cancelled()));
        let mut byte = [0];
        let closed = tokio::time::timeout(Duration::from_millis(500), stream.read(&mut byte)).await;
        assert!(
            matches!(closed, Ok(Ok(0) | Err(_))),
            "cancelled MCP setup retained its idle SSE reader and socket: {closed:?}"
        );
    }

    #[test]
    #[ignore = "subprocess fixture invoked only by MCP transport lifecycle tests"]
    fn stdio_lifecycle_fixture() {
        use std::io::Write;

        let Ok(address) = std::env::var("NEXA_TEST_MCP_LIFECYCLE_ADDRESS") else {
            return;
        };
        let mut ready = std::net::TcpStream::connect(address).unwrap();
        ready.write_all(b"ready").unwrap();
        // The socket makes process exit observable without relying on platform
        // process enumeration. Bound the fixture lifetime even for a red test.
        std::thread::sleep(Duration::from_secs(3));
        drop(ready);
    }

    async fn idle_stdio_fixture() -> (StdioTransport, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let environment = HashMap::from([(
            "NEXA_TEST_MCP_LIFECYCLE_ADDRESS".into(),
            listener.local_addr().unwrap().to_string(),
        )]);
        let command = std::env::current_exe().unwrap();
        let transport = McpClient::build_stdio_transport(
            command.to_str().unwrap(),
            &[
                "--exact".into(),
                "mcp::client::tests::stdio_lifecycle_fixture".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            Some(&environment),
        )
        .await
        .unwrap();
        let (mut ready, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut marker = [0; 5];
        ready.read_exact(&mut marker).await.unwrap();
        assert_eq!(&marker, b"ready");
        (transport, ready)
    }

    #[tokio::test]
    async fn dropping_stdio_transport_terminates_child_and_readers() {
        let (transport, mut process_lifetime) = idle_stdio_fixture().await;
        let stdout = transport.reader_handle.abort_handle();
        let stderr = transport.stderr_handle.abort_handle();
        let dropped_at = tokio::time::Instant::now();
        drop(transport);
        let mut byte = [0];
        let exited = tokio::time::timeout(Duration::from_secs(5), process_lifetime.read(&mut byte))
            .await
            .unwrap();
        assert!(
            matches!(exited, Ok(0) | Err(_)),
            "fixture process kept its lifetime socket open"
        );
        assert!(
            dropped_at.elapsed() < Duration::from_secs(1),
            "dropping MCP transport left its child running"
        );
        tokio::task::yield_now().await;
        assert!(
            stdout.is_finished() && stderr.is_finished(),
            "MCP reader tasks outlived their transport"
        );
    }

    #[tokio::test]
    async fn request_deadline_includes_blocked_stdio_write() {
        let (transport, _process_lifetime) = idle_stdio_fixture().await;
        let mut client = McpClient {
            transport: Transport::Stdio(transport),
            request_id: AtomicI64::new(1),
            server_name: "blocked-stdin".into(),
            protocol_version: SUPPORTED_PROTOCOL_VERSIONS[0].into(),
            server_capabilities: serde_json::json!({}),
            call_timeout: Duration::from_millis(50),
            auth: None,
        };
        let events = client.events();
        let progress = tokio::spawn(async move {
            for value in 0..100 {
                tokio::time::sleep(Duration::from_millis(10)).await;
                events.observe(&json!({"method":"notifications/progress","params":{"progressToken":1,"progress":value}}));
            }
        });
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            client.call_tool("write", json!({ "body": "x".repeat(1024 * 1024) })),
        )
        .await;
        progress.abort();
        client.shutdown().await.unwrap();
        assert!(
            matches!(result, Ok(Err(CoreError::McpTransport(_)))),
            "a child which stops reading stdin escaped the request deadline: {result:?}"
        );
    }

    #[tokio::test]
    async fn streamable_http_does_not_repost_effectful_calls_after_session_expiry() {
        use std::sync::atomic::AtomicUsize;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let initializes = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let recorded_initializes = Arc::clone(&initializes);
        let recorded_calls = Arc::clone(&calls);
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let initializes = Arc::clone(&recorded_initializes);
                let calls = Arc::clone(&recorded_calls);
                tokio::spawn(async move {
                    let request = read_http_request(&mut stream).await.unwrap();
                    if request.method == "DELETE" {
                        write_empty_response(&mut stream, "204 No Content", None)
                            .await
                            .unwrap();
                        return;
                    }
                    if request.method == "GET" {
                        write_empty_response(&mut stream, "405 Method Not Allowed", None)
                            .await
                            .unwrap();
                        return;
                    }
                    let payload: Value = serde_json::from_slice(&request.body).unwrap();
                    match payload["method"].as_str().unwrap() {
                        "initialize" => {
                            initializes.fetch_add(1, Ordering::SeqCst);
                            write_json_response(&mut stream,"200 OK",Some("expired-session"),&json!({
                                "jsonrpc":"2.0","id":payload["id"],"result":{
                                    "protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"once","version":"1"}
                                }
                            })).await.unwrap();
                        }
                        "notifications/initialized" => {
                            write_empty_response(&mut stream, "202 Accepted", None)
                                .await
                                .unwrap();
                        }
                        "tools/call" => {
                            calls.fetch_add(1, Ordering::SeqCst);
                            write_text_response(
                                &mut stream,
                                "404 Not Found",
                                None,
                                "session expired",
                            )
                            .await
                            .unwrap();
                        }
                        method => panic!("unexpected method {method}"),
                    }
                });
            }
        });
        let mut client =
            McpClient::connect_streamable_http(&format!("http://{address}/mcp"), None, "once")
                .await
                .unwrap();
        let error = client.call_tool("write", json!({})).await.unwrap_err();
        assert!(matches!(error, CoreError::McpTransport(_)), "{error}");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "effectful requests must never be automatically replayed"
        );
        assert_eq!(
            initializes.load(Ordering::SeqCst),
            1,
            "the failed invocation must not reconnect inline"
        );
        client.shutdown().await.unwrap();
        task.abort();
    }

    #[tokio::test]
    async fn streamable_http_reinitializes_after_session_expiry() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let initialize_calls = Arc::new(Mutex::new(0usize));
        let tool_sessions = Arc::new(Mutex::new(Vec::<String>::new()));

        let initialize_calls_server = initialize_calls.clone();
        let tool_sessions_server = tool_sessions.clone();
        let server_task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let initialize_calls = initialize_calls_server.clone();
                let tool_sessions = tool_sessions_server.clone();

                tokio::spawn(async move {
                    let request = read_http_request(&mut stream).await.unwrap();
                    let payload: Value = if request.body.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_slice(&request.body).unwrap()
                    };

                    match (request.method.as_str(), request.path.as_str()) {
                        ("POST", "/mcp") => {
                            let method = payload
                                .get("method")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            match method {
                                "initialize" => {
                                    let mut count = initialize_calls.lock().await;
                                    *count += 1;
                                    let session_id = if *count == 1 {
                                        "session-old"
                                    } else {
                                        "session-new"
                                    };
                                    let response = json!({
                                        "jsonrpc": "2.0",
                                        "id": payload.get("id").cloned().unwrap(),
                                        "result": {
                                            "protocolVersion": "2025-11-25",
                                            "capabilities": {},
                                            "serverInfo": { "name": "remote", "version": "1.0.0" }
                                        }
                                    });
                                    write_json_response(
                                        &mut stream,
                                        "200 OK",
                                        Some(session_id),
                                        &response,
                                    )
                                    .await
                                    .unwrap();
                                }
                                "notifications/initialized" => {
                                    let session_id =
                                        request.headers.get(HEADER_MCP_SESSION_ID).cloned();
                                    write_empty_response(
                                        &mut stream,
                                        "202 Accepted",
                                        session_id.as_deref(),
                                    )
                                    .await
                                    .unwrap();
                                }
                                "tools/list" => {
                                    let session_id = request
                                        .headers
                                        .get(HEADER_MCP_SESSION_ID)
                                        .cloned()
                                        .unwrap_or_default();
                                    tool_sessions.lock().await.push(session_id.clone());

                                    if session_id == "session-old" {
                                        write_text_response(
                                            &mut stream,
                                            "404 Not Found",
                                            None,
                                            "expired",
                                        )
                                        .await
                                        .unwrap();
                                    } else {
                                        let response = json!({
                                            "jsonrpc": "2.0",
                                            "id": payload.get("id").cloned().unwrap(),
                                            "result": {
                                                "tools": [{
                                                    "name": "demo",
                                                    "description": "Demo tool",
                                                    "inputSchema": {
                                                        "type": "object",
                                                        "properties": {}
                                                    }
                                                }]
                                            }
                                        });
                                        write_json_response(
                                            &mut stream,
                                            "200 OK",
                                            Some("session-new"),
                                            &response,
                                        )
                                        .await
                                        .unwrap();
                                    }
                                }
                                _ => {
                                    write_text_response(
                                        &mut stream,
                                        "400 Bad Request",
                                        None,
                                        "unexpected method",
                                    )
                                    .await
                                    .unwrap();
                                }
                            }
                        }
                        ("DELETE", "/mcp") => {
                            write_empty_response(&mut stream, "204 No Content", None)
                                .await
                                .unwrap();
                        }
                        _ => {
                            write_text_response(&mut stream, "404 Not Found", None, "missing")
                                .await
                                .unwrap();
                        }
                    }
                });
            }
        });

        let url = format!("http://{addr}/mcp");
        let mut client = McpClient::connect_streamable_http(&url, None, "remote")
            .await
            .unwrap();
        let tools = client.list_tools().await.unwrap();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "demo");
        assert_eq!(*initialize_calls.lock().await, 2);
        assert_eq!(
            tool_sessions.lock().await.clone(),
            vec!["session-old".to_string(), "session-new".to_string()]
        );

        server_task.abort();
    }

    #[tokio::test]
    async fn legacy_sse_connects_and_lists_tools() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let sse_sender = Arc::new(Mutex::new(None::<mpsc::UnboundedSender<String>>));

        let sse_sender_server = sse_sender.clone();
        let server_task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let sse_sender = sse_sender_server.clone();

                tokio::spawn(async move {
                    let request = read_http_request(&mut stream).await.unwrap();
                    match (request.method.as_str(), request.path.as_str()) {
                        ("GET", "/sse") => {
                            let headers = concat!(
                                "HTTP/1.1 200 OK\r\n",
                                "Content-Type: text/event-stream\r\n",
                                "Cache-Control: no-cache\r\n",
                                "Connection: keep-alive\r\n",
                                "\r\n"
                            );
                            stream.write_all(headers.as_bytes()).await.unwrap();
                            stream
                                .write_all(b"event: endpoint\r\ndata: /messages\r\n\r\n")
                                .await
                                .unwrap();
                            stream.flush().await.unwrap();

                            let (tx, mut rx) = mpsc::unbounded_channel::<String>();
                            *sse_sender.lock().await = Some(tx);

                            while let Some(event) = rx.recv().await {
                                if stream.write_all(event.as_bytes()).await.is_err() {
                                    break;
                                }
                                if stream.flush().await.is_err() {
                                    break;
                                }
                            }
                        }
                        ("POST", "/messages") => {
                            let payload: Value = serde_json::from_slice(&request.body).unwrap();
                            let method = payload
                                .get("method")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let sender = sse_sender.lock().await.clone().unwrap();

                            match method {
                                "initialize" => {
                                    let response = json!({
                                        "jsonrpc": "2.0",
                                        "id": payload.get("id").cloned().unwrap(),
                                        "result": {
                                            "protocolVersion": "2025-11-25",
                                            "capabilities": {},
                                            "serverInfo": { "name": "legacy", "version": "1.0.0" }
                                        }
                                    });
                                    sender
                                        .send(format!(
                                            "data: {}\r\n\r\n",
                                            serde_json::to_string(&response).unwrap()
                                        ))
                                        .unwrap();
                                }
                                "tools/list" => {
                                    let response = json!({
                                        "jsonrpc": "2.0",
                                        "id": payload.get("id").cloned().unwrap(),
                                        "result": {
                                            "tools": [{
                                                "name": "legacy_tool",
                                                "description": "Legacy tool",
                                                "inputSchema": {
                                                    "type": "object",
                                                    "properties": {}
                                                }
                                            }]
                                        }
                                    });
                                    sender
                                        .send(format!(
                                            "data: {}\r\n\r\n",
                                            serde_json::to_string(&response).unwrap()
                                        ))
                                        .unwrap();
                                }
                                "notifications/initialized" => {}
                                _ => panic!("unexpected legacy SSE method: {method}"),
                            }

                            write_empty_response(&mut stream, "202 Accepted", None)
                                .await
                                .unwrap();
                        }
                        _ => {
                            write_text_response(&mut stream, "404 Not Found", None, "missing")
                                .await
                                .unwrap();
                        }
                    }
                });
            }
        });

        let url = format!("http://{addr}/sse");
        let mut client = McpClient::connect_sse(&url, None, "legacy").await.unwrap();
        let tools = client.list_tools().await.unwrap();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "legacy_tool");

        server_task.abort();
    }

    #[test]
    fn normalize_stdio_command_name_trims_wrapping_quotes() {
        assert_eq!(normalize_stdio_command_name("  \"npx\"  "), "npx");
        assert_eq!(
            normalize_stdio_command_name(" 'C:\\Program Files\\nodejs\\npx.cmd' "),
            "C:\\Program Files\\nodejs\\npx.cmd"
        );
        assert_eq!(normalize_stdio_command_name("npx"), "npx");
    }

    #[cfg(windows)]
    #[test]
    fn resolve_stdio_in_dirs_prefers_windows_wrappers() {
        let temp = tempdir().unwrap();
        let cmd_path = temp.path().join("npx.cmd");
        fs::write(&cmd_path, "@echo off\r\n").unwrap();

        let resolved = resolve_stdio_in_dirs(
            "npx",
            &[temp.path().to_path_buf()],
            &[
                ".COM".to_string(),
                ".EXE".to_string(),
                ".BAT".to_string(),
                ".CMD".to_string(),
            ],
        )
        .unwrap();

        assert_eq!(
            resolved.to_string_lossy().to_ascii_lowercase(),
            cmd_path.to_string_lossy().to_ascii_lowercase()
        );
    }

    async fn read_http_request(stream: &mut TcpStream) -> std::io::Result<TestHttpRequest> {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 1024];

        loop {
            let bytes_read = stream.read(&mut chunk).await?;
            if bytes_read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed before request completed",
                ));
            }

            buffer.extend_from_slice(&chunk[..bytes_read]);
            if let Some(header_end) = find_http_header_end(&buffer) {
                let header_text = String::from_utf8_lossy(&buffer[..header_end]);
                let mut lines = header_text.split("\r\n");
                let request_line = lines.next().unwrap_or_default();
                let mut request_parts = request_line.split_whitespace();
                let method = request_parts.next().unwrap_or_default().to_string();
                let path = request_parts.next().unwrap_or_default().to_string();

                let mut headers = HashMap::new();
                for line in lines {
                    if line.is_empty() {
                        continue;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
                    }
                }

                let body_start = header_end + 4;
                let content_length = headers
                    .get("content-length")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);

                while buffer.len() < body_start + content_length {
                    let bytes_read = stream.read(&mut chunk).await?;
                    if bytes_read == 0 {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "connection closed before body completed",
                        ));
                    }
                    buffer.extend_from_slice(&chunk[..bytes_read]);
                }

                return Ok(TestHttpRequest {
                    method,
                    path,
                    headers,
                    body: buffer[body_start..body_start + content_length].to_vec(),
                });
            }
        }
    }

    fn find_http_header_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(4).position(|window| window == b"\r\n\r\n")
    }

    async fn write_json_response(
        stream: &mut TcpStream,
        status: &str,
        session_id: Option<&str>,
        body: &Value,
    ) -> std::io::Result<()> {
        write_response(
            stream,
            status,
            session_id,
            CONTENT_TYPE_JSON,
            &serde_json::to_vec(body).unwrap(),
        )
        .await
    }

    async fn write_text_response(
        stream: &mut TcpStream,
        status: &str,
        session_id: Option<&str>,
        body: &str,
    ) -> std::io::Result<()> {
        write_response(stream, status, session_id, "text/plain", body.as_bytes()).await
    }

    async fn write_empty_response(
        stream: &mut TcpStream,
        status: &str,
        session_id: Option<&str>,
    ) -> std::io::Result<()> {
        write_response(stream, status, session_id, CONTENT_TYPE_JSON, &[]).await
    }

    async fn write_response(
        stream: &mut TcpStream,
        status: &str,
        session_id: Option<&str>,
        content_type: &str,
        body: &[u8],
    ) -> std::io::Result<()> {
        // Each fixture serves one request per accepted socket. Advertising
        // closure prevents a pooled follow-up from racing the socket drop.
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(session_id) = session_id {
            response.push_str(&format!("MCP-Session-Id: {session_id}\r\n"));
        }
        response.push_str("\r\n");

        stream.write_all(response.as_bytes()).await?;
        if !body.is_empty() {
            stream.write_all(body).await?;
        }
        stream.flush().await
    }
}
