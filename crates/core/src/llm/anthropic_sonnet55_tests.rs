use super::*;
use crate::llm::provider_turn::{ProviderTurnEnvelope, RouteSnapshot};
use crate::llm::ProviderType;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn request() -> CompletionRequest {
    CompletionRequest {
        model: "claude-sonnet-5-5".into(),
        messages: vec![
            Message::text(Role::System, "Stable policy"),
            Message::text(Role::User, "Inspect"),
        ],
        temperature: Some(0.4),
        max_tokens: Some(128_000),
        tools: Some(vec![ToolDefinition {
            name: "read_file".into(),
            description: "Read".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"},"cache_control":{"type":"string"}}}),
        }]),
        stop: None,
        thinking_budget: None,
        reasoning_enabled: None,
        reasoning_effort: None,
        provider_type: Some(ProviderType::Anthropic),
        routing_session_id: None,
        parallel_tool_calls: true,
    }
}

fn body(request: &CompletionRequest) -> AnthropicRequest {
    let (system, messages) = convert_messages(&request.messages);
    build_request_body(request, system, messages, true)
}

fn native_blocks(id: &str) -> Vec<Value> {
    vec![
        json!({"type":"text","text":"Checking the file"}),
        json!({"type":"thinking","thinking":"","signature":"signed-empty-progress"}),
        json!({"type":"tool_use","id":id,"name":"read_file","input":{"path":"a.rs","cache_control":"literal-user-argument"}}),
    ]
}

#[test]
fn haiku55_implicit_adaptive_route_matches_an_explicitly_enabled_turn() {
    let provider = AnthropicProvider::new(ProviderConfig {
        provider_type: ProviderType::Anthropic,
        api_key: Some("fixture".into()),
        base_url: None,
        org_id: None,
        timeout_secs: Some(10),
        streaming: Default::default(),
    })
    .unwrap();
    let mut input = request();
    input.model = "claude-haiku-5-5".into();
    let default = provider.route_snapshot(&input);
    assert_eq!(
        default.replay_policy,
        crate::llm::reasoning_profile::ReasoningReplayPolicy::OpaqueSignature
    );
    input.reasoning_enabled = Some(true);
    assert_eq!(provider.route_snapshot(&input), default);
    input.reasoning_enabled = Some(false);
    for effort in [ReasoningEffort::XHigh, ReasoningEffort::Max] {
        input.reasoning_effort = Some(effort);
        assert!(provider.resolve_output_capacity(&input).is_err());
    }
}

#[test]
fn haiku55_adaptive_default_disabled_and_bound_replay_are_distinct() {
    let mut input = request();
    input.model = "claude-haiku-5-5".into();
    input.thinking_budget = Some(4096);
    let adaptive = serde_json::to_value(body(&input)).unwrap();
    assert_eq!(adaptive["thinking"]["type"], "adaptive");
    assert_eq!(adaptive["output_config"]["effort"], "medium");
    assert_eq!(
        adaptive["thinking"]["block_binding"]["prefix_mismatch_behavior"],
        "drop_block"
    );
    assert!(adaptive["thinking"].get("budget_tokens").is_none());
    assert!(adaptive.get("temperature").is_none());
    assert!(anthropic_beta_headers(&input.model).contains("thinking-binding-controls-2026-08-01"));
    for effort in [
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
    ] {
        input.reasoning_enabled = Some(false);
        input.reasoning_effort = Some(effort);
        let disabled = serde_json::to_value(body(&input)).unwrap();
        assert_eq!(disabled["thinking"], json!({"type":"disabled"}));
    }
    input.reasoning_effort = None;
    let captured = captured_message(&input, "first");
    input.messages.push(captured);
    input.messages.push(Message::text_with_name(
        Role::Tool,
        "File contents",
        "first",
    ));
    assert_eq!(
        serde_json::to_value(body(&input)).unwrap()["messages"][1]["content"],
        json!(native_blocks("first"))
    );
    input.messages[0] = Message::text(Role::System, "Updated system policy");
    let changed = serde_json::to_value(body(&input)).unwrap();
    assert_eq!(
        changed["messages"][1]["content"].as_array().unwrap().len(),
        2
    );
    assert_eq!(changed["messages"][1]["content"][1]["id"], "first");
}

fn captured_message(input: &CompletionRequest, id: &str) -> Message {
    let blocks = native_blocks(id);
    let replay = replay_payload(
        Some(&FinishReason::ToolCalls),
        blocks.clone(),
        request_prefix(&body(input)).as_deref(),
    )
    .unwrap();
    let profile = resolve_reasoning_profile(
        ProviderType::Anthropic,
        None,
        ReasoningApiStyle::AnthropicMessages,
        &input.model,
    );
    let calls = vec![ToolCallRequest {
        id: id.into(),
        name: "read_file".into(),
        arguments: blocks[2]["input"].to_string(),
        thought_signature: None,
    }];
    let envelope = ProviderTurnEnvelope::capture_with_replay_payload(
        id,
        id,
        RouteSnapshot::from_profile_for_request(&profile, input),
        "Checking the file",
        None,
        None,
        calls.clone(),
        true,
        Some(replay),
    );
    assert!(envelope.authorizes_tool_dispatch());
    let mut message = Message::text(Role::Assistant, "Checking the file");
    message.tool_calls = Some(calls);
    message.set_provider_turn(envelope);
    message
}

#[test]
fn sonnet55_request_modes_never_send_removed_thinking_fields() {
    let mut input = request();
    for effort in [
        None,
        Some(ReasoningEffort::Low),
        Some(ReasoningEffort::Medium),
        Some(ReasoningEffort::High),
        Some(ReasoningEffort::XHigh),
        Some(ReasoningEffort::Max),
    ] {
        input.reasoning_effort = effort.clone();
        input.thinking_budget = Some(4096);
        let value = serde_json::to_value(body(&input)).unwrap();
        assert_eq!(value["thinking"]["type"], "adaptive");
        assert_eq!(value["thinking"]["display"], "updates");
        assert_eq!(
            value["thinking"]["block_binding"]["prefix_mismatch_behavior"],
            "drop_block"
        );
        assert!(value["thinking"].get("budget_tokens").is_none());
        assert!(value.get("temperature").is_none());
        assert_eq!(
            value["output_config"]["effort"],
            effort.map_or("high".to_string(), |effort| effort.to_string())
        );
        assert_eq!(value["max_tokens"], 128_000);
        assert_eq!(value["tool_choice"]["type"], "auto");
    }
    for effort in [
        None,
        Some(ReasoningEffort::None),
        Some(ReasoningEffort::Low),
        Some(ReasoningEffort::XHigh),
        Some(ReasoningEffort::Max),
    ] {
        input.reasoning_enabled = Some(false);
        input.reasoning_effort = effort;
        let value = serde_json::to_value(body(&input)).unwrap();
        assert_eq!(value["thinking"], json!({"type":"between_tools"}));
        assert!(matches!(
            value["output_config"]["effort"].as_str(),
            Some("low" | "medium" | "high")
        ));
    }
    input.reasoning_enabled = None;
    input.reasoning_effort = Some(ReasoningEffort::None);
    assert_eq!(body(&input).thinking.unwrap().r#type, "between_tools");
    assert!(anthropic_beta_headers(&input.model).contains("thinking-display-updates-2026-08-18"));
    for model in ["claude-sonnet-5", "private-sonnet-5-5", "claude-opus-5-5"] {
        input.model = model.into();
        assert!(!between_tools(&input));
        assert!(!anthropic_beta_headers(model).contains("thinking-display-updates"));
        assert!(request_prefix(&body(&input)).is_none());
    }
}

#[test]
fn sonnet55_replays_ordered_empty_thinking_and_drops_only_after_changed_prefix() {
    let mut input = request();
    input.reasoning_enabled = Some(false);
    let first = captured_message(&input, "first");
    input.messages.push(first);
    input
        .messages
        .push(Message::text_with_name(Role::Tool, "First result", "first"));
    let unchanged = serde_json::to_value(body(&input)).unwrap();
    assert_eq!(
        unchanged["messages"][1]["content"],
        json!(native_blocks("first"))
    );
    let second = captured_message(&input, "second");
    input.messages.push(second);
    input.messages.push(Message::text_with_name(
        Role::Tool,
        "Second result",
        "second",
    ));
    let unchanged = serde_json::to_value(body(&input)).unwrap();
    assert_eq!(
        unchanged["messages"][3]["content"],
        json!(native_blocks("second"))
    );
    input.messages[3] = Message::text_with_name(Role::Tool, "Edited first result", "first");
    let edited = serde_json::to_value(body(&input)).unwrap();
    assert_eq!(
        edited["messages"][1]["content"],
        json!(native_blocks("first"))
    );
    assert_eq!(
        edited["messages"][3]["content"].as_array().unwrap().len(),
        2
    );
    assert_eq!(edited["messages"][3]["content"][0]["type"], "text");
    assert_eq!(edited["messages"][3]["content"][1]["type"], "tool_use");
    assert_eq!(
        edited["messages"][2]["content"][0]["content"],
        "Edited first result"
    );
    let third = captured_message(&input, "after-edit");
    input.messages.push(third);
    input.messages.push(Message::text_with_name(
        Role::Tool,
        "Recovered result",
        "after-edit",
    ));
    let recovered = serde_json::to_value(body(&input)).unwrap();
    assert_eq!(
        recovered["messages"][3]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        recovered["messages"][5]["content"],
        json!(native_blocks("after-edit"))
    );
    input.messages[0] = Message::text(Role::System, "Edited policy");
    let changed_system = serde_json::to_value(body(&input)).unwrap();
    assert_eq!(
        changed_system["messages"][1]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn sonnet55_prefix_ignores_only_protocol_cache_hints() {
    let input = request();
    let mut native = body(&input);
    let original = request_prefix(&native);
    native.system.as_mut().unwrap()[0].cache_control = Some(CacheControl {
        r#type: "ephemeral".into(),
    });
    assert_eq!(request_prefix(&native), original);
    native.tools.as_mut().unwrap()[0]["input_schema"]["properties"]["cache_control"]
        ["description"] = json!("changed schema");
    assert_ne!(request_prefix(&native), original);
    let mut input = input;
    input.messages.push(captured_message(&request(), "first"));
    let mut native = body(&input);
    let original = request_prefix(&native);
    if let AnthropicContent::RawBlocks(blocks) = &mut native.messages[1].content {
        blocks[2]["input"]["cache_control"] = json!("edited argument");
    }
    assert_ne!(request_prefix(&native), original);
}

#[test]
fn sonnet55_full_replay_cannot_authorize_tampered_calls_or_persist_sensitive_input() {
    let mut input = request();
    input.reasoning_enabled = Some(false);
    let message = captured_message(&input, "first");
    let envelope = message.provider_turn().unwrap();
    let restored: ProviderTurnEnvelope =
        serde_json::from_str(&serde_json::to_string(envelope).unwrap()).unwrap();
    assert!(restored.authorizes_tool_dispatch());
    assert_eq!(restored.replay_payload, envelope.replay_payload);
    let mut envelope = message.provider_turn().unwrap().clone();
    envelope.tool_calls[0].name = "write_file".into();
    assert!(!envelope.authorizes_tool_dispatch());
    let mut envelope = message.provider_turn().unwrap().clone();
    if let ProviderReplayPayload::AnthropicAssistantBlocks(payload) = &mut envelope.replay_payload {
        payload.content[1]
            .as_object_mut()
            .unwrap()
            .remove("signature");
    }
    assert!(!envelope.authorizes_tool_dispatch());
    let mut envelope = message.provider_turn().unwrap().clone();
    if let ProviderReplayPayload::AnthropicAssistantBlocks(payload) = &mut envelope.replay_payload {
        payload.content[2]["name"] = json!("computer_control");
        payload.content[2]["input"] = json!({"action":"type_text","text":"do-not-persist-secret"});
    }
    assert!(
        !serde_json::to_string(&envelope.audit_safe_for_persistence())
            .unwrap()
            .contains("do-not-persist-secret")
    );
}

#[test]
fn sonnet55_provider_pause_resumes_native_server_state_without_client_dispatch() {
    let input = request();
    let mut content = native_blocks("uncommitted");
    content.push(json!({"type":"server_tool_use","id":"search","name":"web_search","input":{"query":"Nexa"}}));
    let replay = replay_payload(
        Some(&FinishReason::ProviderPause),
        content,
        request_prefix(&body(&input)).as_deref(),
    )
    .unwrap();
    assert!(replay.resumes_provider_pause());
    let ProviderReplayPayload::AnthropicAssistantBlocks(payload) = replay else {
        panic!("missing native state")
    };
    assert!(payload
        .content
        .iter()
        .all(|block| block["type"] != "tool_use"));
    assert_eq!(payload.content[2]["type"], "server_tool_use");
    assert!(replay_payload(
        Some(&FinishReason::Length),
        native_blocks("incomplete"),
        Some("prefix")
    )
    .is_none());
}

async fn serve(
    body: String,
    mime: &'static str,
) -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sent, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..position]);
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map(|(_, length)| length.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= position + 4 + length {
                    break;
                }
            }
        }
        sent.send(String::from_utf8(bytes).unwrap()).unwrap();
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    (base, received, server)
}

#[tokio::test]
async fn sonnet55_http_and_sse_preserve_the_same_native_turn() {
    for streaming in [false, true] {
        let content = native_blocks("wire");
        let response = if streaming {
            let events = vec![
                json!({"type":"message_start","message":{"usage":{"input_tokens":10}}}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Checking the file"}}),
                json!({"type":"content_block_stop","index":0}),
                json!({"type":"content_block_start","index":1,"content_block":{"type":"thinking","thinking":""}}),
                json!({"type":"content_block_delta","index":1,"delta":{"type":"signature_delta","signature":"signed-empty-progress"}}),
                json!({"type":"content_block_stop","index":1}),
                json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"wire","name":"read_file","input":{}}}),
                json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":content[2]["input"].to_string()}}),
                json!({"type":"content_block_stop","index":2}),
                json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
                json!({"type":"message_stop"}),
            ];
            events
                .iter()
                .map(|event| {
                    format!(
                        "event: {}\ndata: {event}\n\n",
                        event["type"].as_str().unwrap()
                    )
                })
                .collect::<String>()
        } else {
            json!({"content":content,"stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":5}}).to_string()
        };
        let (base, received, server) = serve(
            response,
            if streaming {
                "text/event-stream"
            } else {
                "application/json"
            },
        )
        .await;
        let provider = AnthropicProvider::new(ProviderConfig {
            provider_type: ProviderType::Anthropic,
            base_url: Some(base),
            api_key: Some("fixture-key".into()),
            org_id: None,
            timeout_secs: Some(10),
            streaming: Default::default(),
        })
        .unwrap();
        let input = request();
        let replay = if streaming {
            let mut stream = provider.stream_events(&input).await.unwrap();
            let mut replay = None;
            while let Some(event) = stream.next().await {
                match event {
                    ProviderStreamEvent::ReplayState { replay: value } => replay = Some(*value),
                    error @ (ProviderStreamEvent::TerminalError { .. }
                    | ProviderStreamEvent::RecoverableError { .. }
                    | ProviderStreamEvent::Cancelled { .. }) => {
                        panic!("unexpected stream error: {error:?}")
                    }
                    _ => {}
                }
            }
            replay.unwrap()
        } else {
            provider
                .complete(&input)
                .await
                .unwrap()
                .provider_replay
                .unwrap()
        };
        let ProviderReplayPayload::AnthropicAssistantBlocks(payload) = replay else {
            panic!("native replay missing");
        };
        assert_eq!(payload.content, content);
        assert!(!payload.request_prefix.is_empty());
        let wire = received.await.unwrap();
        assert!(wire.contains("thinking-display-updates-2026-08-18"));
        let sent: Value = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(sent["model"], "claude-sonnet-5-5");
        assert_eq!(sent["thinking"]["display"], "updates");
        server.await.unwrap();
    }
}
