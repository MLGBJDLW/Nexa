//! Actual MCP/registry/approval contract tests using local JSON-RPC peers.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;
use crate::approval::{
    ApprovalDecision, ApprovalRequest, ApprovalRisk, SessionApprovalStore, ToolPermissionKey,
};
use crate::tools::{ToolExecutionContext, ToolRegistry};

pub(super) struct TestPeer {
    pub(super) server: McpServer,
    calls: Arc<StdMutex<Vec<String>>>,
    pub(super) tools: Arc<RwLock<Vec<Value>>>,
    result: Arc<RwLock<Option<Value>>>,
    capabilities: Arc<RwLock<Value>>,
    content_responses: Arc<RwLock<HashMap<String, Value>>>,
    content_calls: Arc<StdMutex<Vec<Value>>>,
    pages: Arc<RwLock<HashMap<String, Value>>>,
    list_gate: Arc<RwLock<Option<Arc<tokio::sync::Semaphore>>>>,
    listing_started: Arc<tokio::sync::Semaphore>,
    list_count: Arc<AtomicUsize>,
    notifications: tokio::sync::broadcast::Sender<Value>,
    stream_ready: Arc<tokio::sync::Semaphore>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestPeer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(stream: &mut TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0u8; 2048];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "peer closed before HTTP headers");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let method = headers.split_whitespace().next().unwrap().to_owned();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let mut chunk = [0u8; 2048];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "peer closed before HTTP body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    let payload = if content_length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap()
    };
    (method, payload)
}

async fn reply(stream: &mut TcpStream, status: &str, result: Option<Value>) {
    let body = result.map(|value| value.to_string()).unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

pub(super) async fn peer(id: &str, name: &str, tool_names: &[&str]) -> TestPeer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let tools = Arc::new(RwLock::new(
        tool_names
            .iter()
            .map(|name| {
                json!({
                    "name": name,
                    "description": format!("Exact tool {name}"),
                    "inputSchema": {"type":"object","properties":{}}
                })
            })
            .collect::<Vec<_>>(),
    ));
    let recorded = Arc::clone(&calls);
    let discovered = Arc::clone(&tools);
    let result = Arc::new(RwLock::new(None::<Value>));
    let response_result = Arc::clone(&result);
    let capabilities = Arc::new(RwLock::new(json!({"tools":{"listChanged":true}})));
    let response_capabilities = Arc::clone(&capabilities);
    let content_responses = Arc::new(RwLock::new(HashMap::<String, Value>::new()));
    let response_content = Arc::clone(&content_responses);
    let content_calls = Arc::new(StdMutex::new(Vec::new()));
    let recorded_content = Arc::clone(&content_calls);
    let pages = Arc::new(RwLock::new(HashMap::<String, Value>::new()));
    let list_gate = Arc::new(RwLock::new(None::<Arc<tokio::sync::Semaphore>>));
    let listing_started = Arc::new(tokio::sync::Semaphore::new(0));
    let list_count = Arc::new(AtomicUsize::new(0));
    let stream_ready = Arc::new(tokio::sync::Semaphore::new(0));
    let (notifications, _) = tokio::sync::broadcast::channel::<Value>(128);
    let response_pages = Arc::clone(&pages);
    let response_gate = Arc::clone(&list_gate);
    let response_started = Arc::clone(&listing_started);
    let response_count = Arc::clone(&list_count);
    let response_notifications = notifications.clone();
    let response_stream_ready = Arc::clone(&stream_ready);
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let recorded = Arc::clone(&recorded);
            let discovered = Arc::clone(&discovered);
            let response_result = Arc::clone(&response_result);
            let response_capabilities = Arc::clone(&response_capabilities);
            let response_content = Arc::clone(&response_content);
            let recorded_content = Arc::clone(&recorded_content);
            let response_pages = Arc::clone(&response_pages);
            let response_gate = Arc::clone(&response_gate);
            let response_started = Arc::clone(&response_started);
            let response_count = Arc::clone(&response_count);
            let response_notifications = response_notifications.clone();
            let response_stream_ready = Arc::clone(&response_stream_ready);
            tokio::spawn(async move {
                let (method, request) = read_request(&mut stream).await;
                if method == "GET" {
                    let mut receiver = response_notifications.subscribe();
                    if stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.is_err() { return; }
                    response_stream_ready.add_permits(1);
                    while let Ok(notification) = receiver.recv().await {
                        let data = format!("data: {notification}\n\n");
                        let chunk = format!("{:x}\r\n{}\r\n", data.len(), data);
                        if stream.write_all(chunk.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    return;
                }
                if method == "DELETE" {
                    reply(&mut stream, "204 No Content", None).await;
                    return;
                }
                if request["method"] == "notifications/initialized" {
                    reply(&mut stream, "202 Accepted", None).await;
                    return;
                }
                let result = match request["method"].as_str().unwrap() {
                    "initialize" => json!({
                        "protocolVersion":"2025-11-25",
                        "capabilities":response_capabilities.read().unwrap().clone(),
                        "serverInfo":{"name":"identity-test-peer","version":"1"}
                    }),
                    "tools/list" => {
                        response_count.fetch_add(1, Ordering::SeqCst);
                        response_started.add_permits(1);
                        let gate = response_gate.read().unwrap().clone();
                        if let Some(gate) = gate {
                            gate.acquire().await.unwrap().forget();
                        }
                        let cursor = request["params"]["cursor"].as_str().unwrap_or("");
                        response_pages
                            .read()
                            .unwrap()
                            .get(cursor)
                            .cloned()
                            .unwrap_or_else(|| json!({"tools":discovered.read().unwrap().clone()}))
                    }
                    "tools/call" => {
                        let tool = request["params"]["name"].as_str().unwrap().to_owned();
                        recorded.lock().unwrap().push(tool.clone());
                        response_result
                            .read()
                            .unwrap()
                            .clone()
                            .unwrap_or_else(|| json!({"content":[{"type":"text","text":tool}]}))
                    }
                    method @ ("resources/list"
                    | "resources/templates/list"
                    | "prompts/list"
                    | "resources/read"
                    | "prompts/get") => {
                        recorded_content.lock().unwrap().push(request.clone());
                        let cursor = request["params"]["cursor"].as_str().unwrap_or("");
                        let responses = response_content.read().unwrap();
                        responses.get(&format!("{method}:{cursor}")).or_else(|| responses.get(method)).cloned().unwrap_or_else(|| match method {
                            "resources/list" => json!({"resources":[]}),
                            "resources/templates/list" => json!({"resourceTemplates":[]}),
                            "prompts/list" => json!({"prompts":[]}),
                            "resources/read" => json!({"contents":[{"uri":request["params"]["uri"],"mimeType":"text/plain","text":"RESOURCE_EVIDENCE"}]}),
                            _ => json!({"messages":[{"role":"assistant","content":{"type":"text","text":"TEMPLATE_EVIDENCE"}}]}),
                        })
                    }
                    other => panic!("unexpected method {other}"),
                };
                reply(
                    &mut stream,
                    "200 OK",
                    Some(if let Some(error) = result.get("_rpc_error") {
                        json!({"jsonrpc":"2.0","id":request["id"],"error":error})
                    } else {
                        json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                    }),
                )
                .await;
            });
        }
    });
    TestPeer {
        server: McpServer {
            id: id.into(),
            name: name.into(),
            transport: "streamable_http".into(),
            command: None,
            args: None,
            url: Some(format!("http://{address}/mcp")),
            env_json: None,
            headers_json: None,
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
            builtin_id: None,
        },
        calls,
        tools,
        result,
        capabilities,
        content_responses,
        content_calls,
        pages,
        list_gate,
        listing_started,
        list_count,
        notifications,
        stream_ready,
        task,
    }
}

struct InspectingProvider {
    alias: String,
    requests: Arc<StdMutex<Vec<crate::llm::CompletionRequest>>>,
}

#[async_trait::async_trait]
impl crate::llm::LlmProvider for InspectingProvider {
    fn name(&self) -> &str {
        "mcp-acceptance-provider"
    }
    fn reasoning_replay_policy(
        &self,
        _model: &str,
    ) -> crate::llm::reasoning_profile::ReasoningReplayPolicy {
        crate::llm::reasoning_profile::ReasoningReplayPolicy::NotRequired
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec!["gpt-4.1".into()])
    }
    async fn complete(
        &self,
        _request: &crate::llm::CompletionRequest,
    ) -> Result<crate::llm::CompletionResponse, CoreError> {
        Err(CoreError::Llm(
            "Acceptance fixture expects streamed inference".into(),
        ))
    }
    async fn stream_events(
        &self,
        request: &crate::llm::CompletionRequest,
    ) -> Result<futures::stream::BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError> {
        let first = {
            let mut requests = self.requests.lock().unwrap();
            let first = requests.is_empty();
            requests.push(request.clone());
            first
        };
        let chunk = crate::llm::StreamChunk {
            delta: if first {
                String::new()
            } else {
                "MCP evidence received and retained.".into()
            },
            tool_call_delta: first.then(|| crate::llm::ToolCallDelta {
                id: "mcp-mixed-call".into(),
                name: Some(self.alias.clone()),
                arguments_delta: "{}".into(),
                index: Some(0),
                thought_signature: None,
            }),
            finish_reason: Some(if first {
                crate::llm::FinishReason::ToolCalls
            } else {
                crate::llm::FinishReason::Stop
            }),
            usage: None,
            thinking_delta: None,
        };
        crate::llm::provider_events_from_chunk_stream(Box::pin(futures::stream::iter(vec![Ok(
            chunk,
        )])))
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

fn png_data() -> String {
    use base64::Engine as _;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([20, 40, 60])))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
}

#[tokio::test]
async fn mcp_content_catalogs_reads_and_prompts_reach_the_registry_without_tools_capability() {
    let remote = peer("content", "Knowledge", &[]).await;
    *remote.capabilities.write().unwrap() =
        json!({"resources":{"listChanged":true},"prompts":{"listChanged":true}});
    remote.content_responses.write().unwrap().extend([
        (
            "resources/list:".into(),
            json!({"resources":[{"uri":"notes://one","name":"One"}],"nextCursor":"next"}),
        ),
        (
            "resources/list:next".into(),
            json!({"resources":[{"uri":"notes://two","name":"Two"}]}),
        ),
        (
            "resources/templates/list".into(),
            json!({"resourceTemplates":[{"uriTemplate":"notes://{id}","name":"Note"}]}),
        ),
        (
            "prompts/list".into(),
            json!({"prompts":[{"name":"review","arguments":[{"name":"subject","required":true}]}]}),
        ),
    ]);
    let manager = McpManager::new();
    assert!(manager
        .connect_server(&remote.server, Some(3))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(remote.list_count.load(Ordering::SeqCst), 0);
    let catalog = manager.catalog_snapshot(&remote.server.id).unwrap();
    assert_eq!(catalog.content.resources.len(), 2);
    assert_eq!(catalog.content.prompts.len(), 1);
    let mut registry = ToolRegistry::new();
    manager.register_tools(&mut registry).unwrap();
    assert!(registry.contains("mcp_context"));
    let invocation = registry.build_invocation(
        "identity",
        "mcp_context",
        json!({"action":"read_resource","server_id":"content","uri":"notes://one"}),
    );
    let identity = invocation
        .tool_identity
        .expect("content reads bind connector approvals");
    assert_eq!(identity.id.connector_id, "content");
    assert_eq!(identity.id.tool_name, "resources/read");
    assert_eq!(
        identity.trust_config_digest,
        super::identity::connector_trust_digest(&remote.server)
    );
    let db = Database::open_memory().unwrap();
    let bad = registry.prepare_arguments_for_scheduling(
        "mcp_context",
        "bad",
        r#"{"action":"read_resource","server_id":"content","name":"review"}"#,
    );
    assert!(bad.1.unwrap().is_error);
    let page = registry
        .execute(
            "mcp_context",
            ToolExecutionContext::new(
                "list",
                r#"{"action":"list_resources","server_id":"content","limit":1}"#,
                &db,
                &[],
            ),
        )
        .await
        .unwrap();
    let page: Value = serde_json::from_str(&page.content).unwrap();
    assert_eq!(page["nextOffset"], 1);
    let read = registry
        .execute(
            "mcp_context",
            ToolExecutionContext::new(
                "read",
                r#"{"action":"read_resource","server_id":"content","uri":"notes://one"}"#,
                &db,
                &[],
            ),
        )
        .await
        .unwrap();
    assert!(!read.is_error);
    assert!(read.llm_context_content().contains("RESOURCE_EVIDENCE"));
    assert_eq!(
        read.artifacts.as_ref().unwrap()["contentMethod"],
        "resources/read"
    );
    let before = remote.content_calls.lock().unwrap().len();
    let missing = registry
        .execute(
            "mcp_context",
            ToolExecutionContext::new(
                "missing",
                r#"{"action":"get_prompt","server_id":"content","name":"review"}"#,
                &db,
                &[],
            ),
        )
        .await
        .unwrap();
    assert!(missing.is_error);
    assert_eq!(remote.content_calls.lock().unwrap().len(), before);
    let prompt = registry.execute("mcp_context", ToolExecutionContext::new("prompt",r#"{"action":"get_prompt","server_id":"content","name":"review","arguments":{"subject":"files"}}"#,&db,&[])).await.unwrap();
    assert!(!prompt.is_error);
    assert!(prompt.llm_context_content().contains("TEMPLATE_EVIDENCE"));
    assert!(prompt.llm_context_content().contains("untrusted evidence"));
    assert!(
        registry
            .access_profile("mcp_context", &json!({"action":"get_prompt"}))
            .needs_approval
    );
    assert!(
        !registry
            .run_capabilities("mcp_context", &json!({"action":"get_prompt"}))
            .destructive
    );
    manager.disconnect_server(&remote.server.id).await.unwrap();
    let before = remote.content_calls.lock().unwrap().len();
    let stale = registry
        .execute(
            "mcp_context",
            ToolExecutionContext::new(
                "stale",
                r#"{"action":"read_resource","server_id":"content","uri":"notes://one"}"#,
                &db,
                &[],
            ),
        )
        .await
        .unwrap();
    assert!(stale.is_error);
    assert_eq!(remote.content_calls.lock().unwrap().len(), before);
}

#[tokio::test]
async fn mcp_content_optional_templates_and_each_capability_are_independent() {
    for resources in [true, false] {
        let remote = peer("content-only", "Content only", &[]).await;
        *remote.capabilities.write().unwrap() = if resources {
            json!({"resources":{}})
        } else {
            json!({"prompts":{}})
        };
        remote.content_responses.write().unwrap().insert(
            "resources/templates/list".into(),
            json!({"_rpc_error":{"code":-32601,"message":"No templates"}}),
        );
        let manager = McpManager::new();
        manager
            .connect_server(&remote.server, Some(3))
            .await
            .unwrap();
        assert!(
            manager
                .catalog_snapshot(&remote.server.id)
                .unwrap()
                .complete
        );
        assert_eq!(remote.list_count.load(Ordering::SeqCst), 0);
        let calls = remote.content_calls.lock().unwrap();
        assert!(calls.iter().all(|call| call["method"]
            .as_str()
            .unwrap()
            .starts_with(if resources { "resources/" } else { "prompts/" })));
        drop(calls);
        manager.shutdown().await;
    }
}

#[tokio::test]
async fn mcp_content_repeated_pagination_is_rejected_and_notifications_refresh_catalogs() {
    let remote = peer("content-page", "Content page", &["kept"]).await;
    *remote.capabilities.write().unwrap() = json!({"resources":{"listChanged":true},"tools":{}});
    remote.content_responses.write().unwrap().insert(
        "resources/list".into(),
        json!({"resources":[],"nextCursor":"again"}),
    );
    let manager = McpManager::new();
    manager
        .connect_server(&remote.server, Some(3))
        .await
        .unwrap();
    let partial = manager.catalog_snapshot(&remote.server.id).unwrap();
    assert!(!partial.content.resources_complete);
    assert_eq!(partial.tools[0].name, "kept");
    assert!(partial
        .content
        .content_diagnostics
        .iter()
        .any(|message| message.contains("cursor")));
    remote.content_responses.write().unwrap().insert(
        "resources/list".into(),
        json!({"resources":[{"uri":"notes://one","name":"One"}]}),
    );
    manager.refresh_server(&remote.server.id).await.unwrap();
    let before = manager.catalog_snapshot(&remote.server.id).unwrap();
    tokio::time::timeout(Duration::from_secs(2), remote.stream_ready.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    remote.content_responses.write().unwrap().insert(
        "resources/list".into(),
        json!({"resources":[{"uri":"notes://two","name":"Two"}]}),
    );
    remote
        .notifications
        .send(json!({"jsonrpc":"2.0","method":"notifications/resources/list_changed"}))
        .unwrap();
    let after = wait_for_catalog(&manager, &remote.server.id, before.catalog_revision + 1).await;
    assert_eq!(after.content.resources[0]["uri"], "notes://two");
    manager.shutdown().await;
}

#[tokio::test]
async fn mixed_mcp_result_reaches_next_model_and_reopened_history_without_replay() {
    use crate::agent::{AgentConfig, AgentExecutor};
    use crate::conversation::{ConversationMessage, CreateConversationInput};
    use crate::llm::{ContentPart, Role};

    for business_error in [false, true] {
        let remote = peer("mixed-result", "Evidence", &["capture"]).await;
        let image = png_data();
        *remote.result.write().unwrap() = Some(json!({
            "isError":business_error,
            "content":[
                {"type":"text","text":"MIXED_TEXT_SENTINEL"},
                {"type":"image","mimeType":"image/png","data":image},
                {"type":"resource","resource":{"uri":"test://notes","mimeType":"text/plain","text":"EMBEDDED_RESOURCE_SENTINEL"}},
                {"type":"resource_link","uri":"https://example.invalid/report","name":"Report"}
            ],
            "structuredContent":{"answer":42,"marker":"STRUCTURED_SENTINEL"}
        }));
        let manager = McpManager::new();
        manager
            .connect_server(&remote.server, Some(3))
            .await
            .unwrap();
        let mut registry = ToolRegistry::new();
        manager.register_tools(&mut registry).unwrap();
        let alias = CanonicalToolId::new(&remote.server.id, "capture").model_alias();
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let approvals = Arc::new(StdMutex::new(Vec::<ApprovalRequest>::new()));
        let received_approvals = Arc::clone(&approvals);
        let executor = AgentExecutor::new(
            Box::new(InspectingProvider {
                alias,
                requests: Arc::clone(&requests),
            }),
            registry,
            AgentConfig {
                model: Some("gpt-4.1".into()),
                native_vision: Some(true),
                max_iterations: 1,
                tool_approval_mode: crate::approval::ToolApprovalMode::Ask,
                context_window: Some(128_000),
                reasoning_enabled: Some(false),
                ..AgentConfig::default()
            },
        )
        .with_skills_override(Vec::new())
        .with_auto_loaded_skills_override(Vec::new())
        .with_approval_callback(Arc::new(move |request| {
            received_approvals.lock().unwrap().push(request);
            Box::pin(async { ApprovalDecision::AllowOnce })
        }));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mixed.sqlite");
        let db = Database::new(&path).unwrap();
        let conversation = db
            .create_conversation(&CreateConversationInput {
                provider: "custom".into(),
                model: "gpt-4.1".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        let user = ConversationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation.id.clone(),
            role: Role::User,
            content: "Use the connected capture tool once, then report the evidence.".into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: None,
            token_count: 16,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        };
        db.add_message(&user).unwrap();
        let turn = db
            .create_conversation_turn(&conversation.id, &user.id, None)
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let events = tokio::spawn(async move {
            let mut all = Vec::new();
            while let Some(event) = rx.recv().await {
                all.push(event);
            }
            all
        });
        executor
            .run(
                Vec::new(),
                vec![ContentPart::Text { text: user.content }],
                &db,
                Some(&conversation.id),
                Some(&turn.id),
                tx,
                1,
            )
            .await
            .unwrap();
        let _events = events.await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "exactly one tool step and one answer step"
        );
        let next = &requests[1];
        assert!(next.messages.iter().any(|message| message
            .parts
            .iter()
            .any(|part| matches!(part, ContentPart::Image {data, ..} if data == &image))));
        let text = next
            .messages
            .iter()
            .map(|message| message.text_content())
            .collect::<Vec<_>>()
            .join("\n");
        for marker in [
            "MIXED_TEXT_SENTINEL",
            "EMBEDDED_RESOURCE_SENTINEL",
            "STRUCTURED_SENTINEL",
            "https://example.invalid/report",
        ] {
            assert!(
                text.contains(marker),
                "missing {marker} in actual next request"
            );
        }
        assert_eq!(
            remote.calls.lock().unwrap().len(),
            1,
            "projection/attachment failures must never replay a side effect"
        );
        let approvals = approvals.lock().unwrap();
        assert_eq!(approvals.len(), 1);
        assert_eq!(
            approvals[0].tool_identity,
            Some(McpToolIdentity::new(&remote.server, "capture"))
        );
        assert!(approvals[0].expires_at.is_some());
        drop(approvals);
        drop(requests);
        drop(db);
        let reopened = Database::new(&path).unwrap();
        let saved = reopened.get_messages(&conversation.id).unwrap();
        let tool = saved
            .iter()
            .find(|message| message.role == Role::Tool)
            .expect("persisted MCP tool message");
        let artifact = tool.artifacts.as_ref().unwrap();
        assert_eq!(artifact["kind"], "mcpToolResult");
        assert_eq!(artifact["contentBlocks"][1]["data"], image);
        assert_eq!(artifact["structuredContent"]["answer"], 42);
        assert_eq!(artifact["isError"], business_error);
        assert!(
            artifact.pointer("/toolOutput/attachments").is_none(),
            "current-step projection must not duplicate durable binary content"
        );
        manager.shutdown().await;
    }
}

#[tokio::test]
async fn canonical_identity_routes_every_exact_tool_and_connector() {
    let peers = [
        peer(
            "user-json:first",
            "Same display name",
            &["read.file", "read_file", "READ_FILE"],
        )
        .await,
        peer("user-json:second", "Same display name", &["read.file"]).await,
        peer("user-json:third", "Same display name", &["read.file"]).await,
    ];
    let manager = McpManager::new();
    for item in &peers {
        manager.connect_server(&item.server, Some(3)).await.unwrap();
    }
    let mut registry = ToolRegistry::new();
    manager.register_tools(&mut registry).unwrap();
    assert_eq!(registry.tool_names().len(), 5);
    assert_eq!(
        registry
            .tool_names()
            .into_iter()
            .collect::<HashSet<_>>()
            .len(),
        5
    );
    let db = Database::open_memory().unwrap();
    for item in &peers {
        let names = item
            .tools
            .read()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        for name in &names {
            let identity = McpToolIdentity::new(&item.server, name);
            let alias = identity.id.model_alias();
            let invocation = registry.build_invocation("call", &alias, json!({}));
            assert_eq!(invocation.tool_identity, Some(identity));
            let output = registry
                .get(&alias)
                .unwrap()
                .execute(ToolExecutionContext::new("call", "{}", &db, &[]))
                .await
                .unwrap();
            assert!(!output.is_error, "{}", output.content);
            assert_eq!(output.content, *name);
        }
        assert_eq!(*item.calls.lock().unwrap(), names);
    }
    manager.shutdown().await;
}

#[tokio::test]
async fn canonical_approval_grants_follow_connector_and_trust_configuration() {
    let first = peer("account-a", "Shared", &["write"]).await;
    let second = peer("account-b", "Shared", &["write"]).await;
    let manager = McpManager::new();
    manager
        .connect_server(&first.server, Some(3))
        .await
        .unwrap();
    manager
        .connect_server(&second.server, Some(3))
        .await
        .unwrap();
    let mut registry = ToolRegistry::new();
    manager.register_tools(&mut registry).unwrap();
    let request = |server: &McpServer, registry: &ToolRegistry| {
        let alias = CanonicalToolId::new(&server.id, "write").model_alias();
        ApprovalRequest::from_invocation(
            "req",
            &registry.build_invocation("call", &alias, json!({})),
            ApprovalRisk::High,
            "MCP write",
        )
    };
    let grant = request(&first.server, &registry);
    let other = request(&second.server, &registry);
    let store = SessionApprovalStore::default();
    store.set(&grant.permission_key, ApprovalDecision::AllowSession);
    assert_eq!(
        store.resolve(&ToolPermissionKey::from_request(&grant)),
        Some(ApprovalDecision::AllowSession)
    );
    assert_eq!(
        store.resolve(&ToolPermissionKey::from_request(&other)),
        None
    );

    let mut renamed = first.server.clone();
    renamed.name = "Renamed".into();
    manager.connect_server(&renamed, Some(3)).await.unwrap();
    let mut renamed_registry = ToolRegistry::new();
    manager.register_tools(&mut renamed_registry).unwrap();
    let renamed_request = request(&renamed, &renamed_registry);
    assert_eq!(grant.permission_key, renamed_request.permission_key);

    let mut changed = renamed.clone();
    changed.url = second.server.url.clone();
    manager.connect_server(&changed, Some(3)).await.unwrap();
    let mut changed_registry = ToolRegistry::new();
    manager.register_tools(&mut changed_registry).unwrap();
    let changed_request = request(&changed, &changed_registry);
    assert_eq!(grant.tool_name, changed_request.tool_name);
    assert_ne!(grant.permission_key, changed_request.permission_key);
    assert_eq!(
        store.resolve(&ToolPermissionKey::from_request(&changed_request)),
        None
    );

    let stale_db = Database::open_memory().unwrap();
    let stale = registry
        .get(&grant.tool_name)
        .unwrap()
        .execute(ToolExecutionContext::new(
            "stale-grant",
            "{}",
            &stale_db,
            &[],
        ))
        .await
        .unwrap();
    assert!(stale.is_error);
    assert!(
        stale.content.contains("disabled or replaced connector"),
        "{}",
        stale.content
    );
    assert!(first.calls.lock().unwrap().is_empty());
    assert!(second.calls.lock().unwrap().is_empty());

    // No name-only or wildcard migration into the versioned connector grant.
    store.set(
        &ToolPermissionKey::new(&changed_request.tool_name, "tool", "*").permission_key(),
        ApprovalDecision::AllowSession,
    );
    store.set(
        &ToolPermissionKey::new("mcp__shared__write", "mcp_tool", "mcp__shared__write")
            .permission_key(),
        ApprovalDecision::AllowSession,
    );
    assert_eq!(
        store.resolve(&ToolPermissionKey::from_request(&changed_request)),
        None
    );
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("permissions.sqlite");
    let db = Database::new(&path).unwrap();
    for legacy in [
        ToolPermissionKey::new(&changed_request.tool_name, "tool", "*"),
        ToolPermissionKey::new("*", "tool", "*"),
        ToolPermissionKey::new("mcp__shared__write", "mcp_tool", "mcp__shared__write"),
    ] {
        db.save_tool_permission_policy(&legacy, "allow").unwrap();
    }
    db.save_tool_permission_policy(&ToolPermissionKey::from_request(&grant), "never")
        .unwrap();
    drop(db);
    let reopened = Database::new(&path).unwrap();
    assert_eq!(
        reopened
            .resolve_tool_permission_policy(&ToolPermissionKey::from_request(&grant))
            .unwrap()
            .as_deref(),
        Some("never")
    );
    assert_eq!(
        reopened
            .resolve_tool_permission_policy(&ToolPermissionKey::from_request(&changed_request))
            .unwrap(),
        None
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn canonical_alias_keeps_specialized_package_and_mcp_host_gates_after_builtin_rename() {
    use crate::package_host::{PackageHealthState, PackageLifecycleState, PackageRuntimeAssembler};
    for (builtin_id, display_name) in [
        ("computer-use", "Computer Use"),
        ("windows-computer-use", "Windows Computer Use"),
    ] {
        let remote = peer(
            &format!("fixture-{builtin_id}"),
            display_name,
            &["computer"],
        )
        .await;
        let db = Database::open_memory().unwrap();
        let saved = db
            .save_mcp_server(&SaveMcpServerInput {
                id: None,
                name: remote.server.name.clone(),
                transport: remote.server.transport.clone(),
                command: None,
                args: None,
                url: remote.server.url.clone(),
                env_json: None,
                headers_json: None,
                enabled: true,
            })
            .unwrap();
        // Install the same host-owned metadata as a builtin connector seed.
        // Public SaveMcpServerInput has no builtin_id field and cannot claim it.
        db.conn()
            .execute(
                "UPDATE mcp_servers SET builtin_id = ?1 WHERE id = ?2",
                rusqlite::params![builtin_id, saved.id],
            )
            .unwrap();
        let original = db.get_mcp_server(&saved.id).unwrap();
        let manager = McpManager::new();
        manager
            .sync_server_from_database(&db, &saved.id, Some(3))
            .await
            .unwrap();
        let mut registry = ToolRegistry::new();
        manager.register_tools(&mut registry).unwrap();
        let alias = CanonicalToolId::new(&saved.id, "computer").model_alias();
        let original_invocation = registry.build_invocation("before", &alias, json!({}));
        let original_permission = ToolPermissionKey::from_invocation(&original_invocation);
        assert_eq!(original_invocation.owner.id, "computer-use-connector");

        // Exercise the actual rename save flow, not a hand-edited in-memory
        // server. Neither package owner nor a session grant may follow a label.
        let renamed = db
            .save_mcp_server(&SaveMcpServerInput {
                id: Some(saved.id.clone()),
                name: "Presentation label only".into(),
                transport: original.transport.clone(),
                command: original.command.clone(),
                args: original.args.clone(),
                url: original.url.clone(),
                env_json: original.env_json.clone(),
                headers_json: original.headers_json.clone(),
                enabled: true,
            })
            .unwrap();
        assert_eq!(
            renamed.builtin_id.as_deref(),
            Some(builtin_id),
            "saving a name must preserve the authoritative builtin binding"
        );
        assert_eq!(renamed.transport, original.transport);
        assert_eq!(renamed.command, original.command);
        manager
            .sync_server_from_database(&db, &renamed.id, Some(3))
            .await
            .unwrap();
        let mut renamed_registry = ToolRegistry::new();
        manager.register_tools(&mut renamed_registry).unwrap();
        assert!(renamed_registry.contains(&alias));
        assert_eq!(
            renamed_registry.plugin_info(&alias).id,
            "computer-use-connector"
        );
        let renamed_invocation = renamed_registry.build_invocation("after", &alias, json!({}));
        assert_eq!(renamed_invocation.owner.id, "computer-use-connector");
        let renamed_permission = ToolPermissionKey::from_invocation(&renamed_invocation);
        assert_eq!(renamed_permission, original_permission);
        let session = SessionApprovalStore::default();
        session.set(
            &original_permission.permission_key(),
            ApprovalDecision::AllowSession,
        );
        assert_eq!(
            session.resolve(&renamed_permission),
            Some(ApprovalDecision::AllowSession)
        );

        for computer_enabled in [false, true] {
            for host_enabled in [false, true] {
                for (id, enabled) in [
                    ("computer-use-connector", computer_enabled),
                    ("mcp-connectors", host_enabled),
                ] {
                    db.upsert_package_host_state(
                        id,
                        if enabled {
                            PackageLifecycleState::Enabled
                        } else {
                            PackageLifecycleState::Disabled
                        },
                        PackageHealthState::Healthy,
                    )
                    .unwrap();
                }
                let runtime = PackageRuntimeAssembler::database_builtin(&db)
                    .unwrap()
                    .assemble_tool_registry(renamed_registry.clone())
                    .unwrap();
                assert_eq!(
                    runtime.tools.contains(&alias),
                    computer_enabled && host_enabled,
                    "renamed builtin {builtin_id} must retain both package gates"
                );
            }
        }
        manager.shutdown().await;
    }
}

#[tokio::test]
async fn custom_connector_display_names_cannot_claim_a_builtin_package_owner() {
    use crate::package_host::{PackageHealthState, PackageLifecycleState, PackageRuntimeAssembler};
    for (index, name) in ["Computer Use", "Windows Computer Use"]
        .into_iter()
        .enumerate()
    {
        let remote = peer(&format!("custom-{index}"), name, &["computer"]).await;
        assert!(remote.server.builtin_id.is_none());
        let manager = McpManager::new();
        manager
            .connect_server(&remote.server, Some(3))
            .await
            .unwrap();
        let mut registry = ToolRegistry::new();
        manager.register_tools(&mut registry).unwrap();
        let alias = CanonicalToolId::new(&remote.server.id, "computer").model_alias();
        assert_eq!(registry.plugin_info(&alias).id, "mcp-connectors");
        assert_eq!(
            registry
                .build_invocation("custom", &alias, json!({}))
                .owner
                .id,
            "mcp-connectors"
        );
        let db = Database::open_memory().unwrap();
        db.upsert_package_host_state(
            "computer-use-connector",
            PackageLifecycleState::Disabled,
            PackageHealthState::Healthy,
        )
        .unwrap();
        db.upsert_package_host_state(
            "mcp-connectors",
            PackageLifecycleState::Enabled,
            PackageHealthState::Healthy,
        )
        .unwrap();
        let runtime = PackageRuntimeAssembler::database_builtin(&db)
            .unwrap()
            .assemble_tool_registry(registry)
            .unwrap();
        assert!(
            runtime.tools.contains(&alias),
            "a custom connector belongs to the generic MCP package regardless of display label"
        );
        manager.shutdown().await;
    }
}

fn catalog_tool(name: &str) -> Value {
    json!({"name":name,"description":format!("Tool {name}"),"inputSchema":{"type":"object","properties":{}}})
}

async fn wait_for_catalog(manager: &McpManager, id: &str, revision: u64) -> McpCatalogSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(snapshot) = manager.catalog_snapshot(id) {
                if snapshot.complete && snapshot.catalog_revision >= revision {
                    return snapshot;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("catalog refresh completed")
}

#[tokio::test]
async fn complete_catalog_consumes_all_pages_and_rejects_partial_failure() {
    let remote = peer("paged", "Paged", &["fallback"]).await;
    remote.pages.write().unwrap().extend([
        (
            "".into(),
            json!({"tools":[catalog_tool("alpha")],"nextCursor":"page-2"}),
        ),
        ("page-2".into(), json!({"tools":[catalog_tool("beta")]})),
    ]);
    let manager = McpManager::new();
    let tools = manager
        .connect_server(&remote.server, Some(3))
        .await
        .unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert_eq!(remote.list_count.load(Ordering::SeqCst), 2);
    let old = manager.catalog_snapshot(&remote.server.id).unwrap();
    remote
        .pages
        .write()
        .unwrap()
        .insert("page-2".into(), json!({"invalid":"missing tools"}));
    assert!(manager.refresh_server(&remote.server.id).await.is_err());
    let failed = manager.catalog_snapshot(&remote.server.id).unwrap();
    assert!(!failed.complete);
    assert_eq!(
        failed.tools, old.tools,
        "never publish a partial first page"
    );
    assert_eq!(failed.catalog_revision, old.catalog_revision);
    let mut registry = ToolRegistry::new();
    assert!(manager.register_tools(&mut registry).is_err());
    assert!(registry.tool_names().is_empty());
    remote
        .pages
        .write()
        .unwrap()
        .insert("page-2".into(), json!({"tools":[],"nextCursor":"page-2"}));
    let error = manager
        .refresh_server(&remote.server.id)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("repeated a pagination cursor"), "{error}");
    manager.shutdown().await;
}

#[tokio::test]
async fn idle_list_changed_refreshes_schema_and_removal_without_replaying_old_tools() {
    let remote = peer("dynamic", "Dynamic", &["retained", "removed"]).await;
    let manager = McpManager::new();
    manager
        .connect_server(&remote.server, Some(3))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), remote.stream_ready.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let mut old_registry = ToolRegistry::new();
    manager.register_tools(&mut old_registry).unwrap();
    let original = manager.catalog_snapshot(&remote.server.id).unwrap();
    let mut changed = catalog_tool("retained");
    changed["inputSchema"] = json!({"type":"object","properties":{"requiredValue":{"type":"string"}},"required":["requiredValue"]});
    *remote.tools.write().unwrap() = vec![changed, catalog_tool("added")];
    for _ in 0..20 {
        remote
            .notifications
            .send(json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}))
            .unwrap();
    }
    let updated =
        wait_for_catalog(&manager, &remote.server.id, original.catalog_revision + 1).await;
    assert_eq!(
        updated.connection_epoch, original.connection_epoch,
        "catalog changes do not require reconnect"
    );
    assert_eq!(
        updated
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["added", "retained"]
    );
    assert!(
        remote.list_count.load(Ordering::SeqCst) <= 3,
        "notification burst should be coalesced"
    );
    let db = Database::open_memory().unwrap();
    for name in ["retained", "removed"] {
        let old = old_registry
            .get(&CanonicalToolId::new(&remote.server.id, name).model_alias())
            .unwrap();
        let result = old
            .execute(ToolExecutionContext::new("old", "{}", &db, &[]))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("removed or its definition changed"),
            "{}",
            result.content
        );
    }
    assert!(remote.calls.lock().unwrap().is_empty());
    let mut registry = ToolRegistry::new();
    manager.register_tools(&mut registry).unwrap();
    assert!(registry.contains(&CanonicalToolId::new(&remote.server.id, "added").model_alias()));
    assert!(!registry.contains(&CanonicalToolId::new(&remote.server.id, "removed").model_alias()));
    manager.shutdown().await;
}

#[tokio::test]
async fn idle_notification_burst_cannot_starve_the_http_catalog_reader() {
    let remote = peer("noisy", "Noisy", &["before"]).await;
    let manager = McpManager::new();
    manager
        .connect_server(&remote.server, Some(3))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), remote.stream_ready.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let original = manager.catalog_snapshot(&remote.server.id).unwrap();
    *remote.tools.write().unwrap() = vec![catalog_tool("after")];

    // The real HTTP GET reader's response queue has capacity 64. No caller
    // drains it while idle; ordinary notifications must be consumed before it.
    for sequence in 0..96 {
        remote.notifications.send(json!({
            "jsonrpc":"2.0",
            "method":if sequence % 2 == 0 { "notifications/message" } else { "notifications/progress" },
            "params":{"sequence":sequence}
        })).unwrap();
    }
    remote
        .notifications
        .send(json!({
            "jsonrpc":"2.0","method":"notifications/tools/list_changed"
        }))
        .unwrap();
    // Poll only the immutable snapshot: an unrelated RPC must not be needed to
    // release the notification queue or make the new tool visible.
    let refreshed =
        wait_for_catalog(&manager, &remote.server.id, original.catalog_revision + 1).await;
    assert_eq!(refreshed.tools[0].name, "after");
    assert_eq!(refreshed.connection_epoch, original.connection_epoch);
    assert_eq!(remote.list_count.load(Ordering::SeqCst), 2);
    manager.shutdown().await;
}

#[tokio::test]
async fn blocked_connector_discovery_does_not_block_other_calls_or_snapshots() {
    let slow = peer("slow", "Slow", &["slow"]).await;
    let ready = peer("ready", "Ready", &["ready"]).await;
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *slow.list_gate.write().unwrap() = Some(Arc::clone(&gate));
    let manager = McpManager::new();
    manager
        .connect_server(&ready.server, Some(3))
        .await
        .unwrap();
    let slow_manager = manager.clone();
    let slow_server = slow.server.clone();
    let connecting =
        tokio::spawn(async move { slow_manager.connect_server(&slow_server, Some(3)).await });
    tokio::time::timeout(Duration::from_secs(3), slow.listing_started.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let mut registry = ToolRegistry::new();
    assert!(manager.register_tools(&mut registry).is_err());
    assert!(manager.catalog_snapshot(&ready.server.id).unwrap().complete);
    let db = Database::open_memory().unwrap();
    let tool = registry
        .get(&CanonicalToolId::new(&ready.server.id, "ready").model_alias())
        .unwrap();
    let output = tokio::time::timeout(
        Duration::from_millis(500),
        tool.execute(ToolExecutionContext::new("fast", "{}", &db, &[])),
    )
    .await
    .expect("other connector must not wait for blocked discovery")
    .unwrap();
    assert!(!output.is_error, "{}", output.content);
    manager.disconnect_server(&slow.server.id).await.unwrap();
    assert!(tokio::time::timeout(Duration::from_millis(500), connecting)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    gate.add_permits(1);
    assert!(
        manager.catalog_snapshot(&slow.server.id).is_none(),
        "disabled discovery cannot publish late"
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn discovery_admission_is_bounded_across_connector_slots() {
    let mut peers = Vec::new();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    for index in 0..7 {
        let remote = peer(&format!("bounded-{index}"), "Bounded", &["demo"]).await;
        *remote.list_gate.write().unwrap() = Some(Arc::clone(&gate));
        peers.push(remote);
    }
    let manager = McpManager::new();
    let servers = peers
        .iter()
        .map(|remote| remote.server.clone())
        .collect::<Vec<_>>();
    let background = manager.clone();
    let syncing = tokio::spawn(async move { background.sync_servers(&servers, Some(5)).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while peers
            .iter()
            .map(|remote| remote.list_count.load(Ordering::SeqCst))
            .sum::<usize>()
            < 4
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        peers
            .iter()
            .map(|remote| remote.list_count.load(Ordering::SeqCst))
            .sum::<usize>(),
        4
    );
    gate.add_permits(7);
    assert!(syncing.await.unwrap().is_empty());
    assert_eq!(
        peers
            .iter()
            .map(|remote| remote.list_count.load(Ordering::SeqCst))
            .sum::<usize>(),
        7
    );
    manager.shutdown().await;
}

#[tokio::test]
async fn database_disable_during_discovery_prevents_late_publication_and_calls() {
    let remote = peer("source", "Database", &["write"]).await;
    let db = Database::open_memory().unwrap();
    let saved = db
        .save_mcp_server(&SaveMcpServerInput {
            id: None,
            name: remote.server.name.clone(),
            transport: remote.server.transport.clone(),
            command: None,
            args: None,
            url: remote.server.url.clone(),
            env_json: None,
            headers_json: None,
            enabled: true,
        })
        .unwrap();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *remote.list_gate.write().unwrap() = Some(Arc::clone(&gate));
    let manager = McpManager::new();
    let pending_manager = manager.clone();
    let pending_db = db.clone();
    let pending_id = saved.id.clone();
    let pending = tokio::spawn(async move {
        pending_manager
            .sync_server_from_database(&pending_db, &pending_id, Some(5))
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), remote.listing_started.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    db.toggle_mcp_server(&saved.id, false).unwrap();
    gate.add_permits(1);
    assert!(pending.await.unwrap().is_err());
    assert!(manager.catalog_snapshot(&saved.id).is_none());
    let mut registry = ToolRegistry::new();
    manager.register_tools(&mut registry).unwrap();
    assert!(registry.tool_names().is_empty());
    assert!(remote.calls.lock().unwrap().is_empty());
    manager.shutdown().await;
}
