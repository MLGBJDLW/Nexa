use super::*;

fn config(base: String) -> ProviderConfig {
    ProviderConfig {
        provider_type: ProviderType::Bedrock,
        base_url: Some(base),
        api_key: Some("bedrock-test-key".into()),
        org_id: None,
        timeout_secs: Some(3),
        streaming: Default::default(),
    }
}

fn tool_events() -> Vec<(&'static str, Value)> {
    vec![
        ("messageStart", json!({"role":"assistant"})),
        (
            "contentBlockDelta",
            json!({"contentBlockIndex":0,"delta":{"reasoningContent":{"text":"analysis"}}}),
        ),
        (
            "contentBlockDelta",
            json!({"contentBlockIndex":0,"delta":{"reasoningContent":{"signature":"signed"}}}),
        ),
        ("contentBlockStop", json!({"contentBlockIndex":0})),
        (
            "contentBlockStart",
            json!({"contentBlockIndex":1,"start":{"toolUse":{"toolUseId":"tool-1","name":"read_file"}}}),
        ),
        (
            "contentBlockDelta",
            json!({"contentBlockIndex":1,"delta":{"toolUse":{"input":"{\"path\":"}}}),
        ),
        (
            "contentBlockDelta",
            json!({"contentBlockIndex":1,"delta":{"toolUse":{"input":"\"main.rs\"}"}}}),
        ),
        ("contentBlockStop", json!({"contentBlockIndex":1})),
        ("messageStop", json!({"stopReason":"tool_use"})),
        (
            "metadata",
            json!({"usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15,"cacheReadInputTokens":2}}),
        ),
    ]
}

#[test]
fn signed_tool_turn_is_atomic_and_keeps_trailing_usage() {
    let mut state = StreamState::default();
    for (kind, value) in tool_events() {
        state.event(kind, value).unwrap();
    }
    let (terminal, replay) = state.terminal("test-prefix".into(), true).unwrap();
    assert_eq!(terminal.finish_reason, Some(FinishReason::ToolCalls));
    assert_eq!(terminal.usage.unwrap().cache_read_tokens, Some(2));
    let ProviderReplayPayload::BedrockAssistantBlocks(payload) = replay else {
        panic!()
    };
    let blocks = payload.content;
    let calls = vec![ToolCallRequest {
        id: "tool-1".into(),
        name: "read_file".into(),
        arguments: r#"{"path":"main.rs"}"#.into(),
        thought_signature: None,
    }];
    assert!(valid_replay(&blocks, Some(&calls)));
    let mut mismatched = calls;
    mismatched[0].arguments = "{}".into();
    assert!(!valid_replay(&blocks, Some(&mismatched)));
    assert_eq!(
        blocks[0]["reasoningContent"]["reasoningText"]["signature"],
        "signed"
    );
    let mut broken = blocks;
    broken[0]["reasoningContent"]["reasoningText"]
        .as_object_mut()
        .unwrap()
        .remove("signature");
    assert!(!signed_reasoning_complete(&broken));
}

#[test]
fn partial_and_late_exception_streams_never_complete() {
    for count in [1, 5, 7, 8, 9] {
        let mut state = StreamState::default();
        for (kind, value) in tool_events().into_iter().take(count) {
            state.event(kind, value).unwrap();
        }
        assert!(state.terminal("test-prefix".into(), true).is_err());
    }
    let mut state = StreamState::default();
    for (kind, value) in tool_events() {
        state.event(kind, value).unwrap();
    }
    assert!(state
        .event(
            "modelStreamErrorException",
            json!({"message":"late failure"})
        )
        .is_err());
}

#[test]
fn reasoning_replay_tracks_prefix_and_cloud_thinking_controls() {
    let provider = BedrockProvider::new(config(
        "https://bedrock-runtime.us-east-1.amazonaws.com".into(),
    ))
    .unwrap();
    let mut request = CompletionRequest {
        model: "global.anthropic.claude-haiku-5-5".into(),
        messages: vec![
            Message::text(Role::System, "rules"),
            Message::text(Role::User, "task"),
        ],
        ..Default::default()
    };
    let route = provider.route_snapshot(&request);
    let initial = body(&request, &route).unwrap();
    assert_eq!(
        initial["additionalModelRequestFields"]["thinking"]["type"],
        "adaptive"
    );
    assert_eq!(
        initial["additionalModelRequestFields"]["anthropic_beta"],
        json!(["thinking-binding-controls-2026-08-01"])
    );
    let blocks = vec![
        json!({"reasoningContent":{"reasoningText":{"text":"analysis","signature":"signed"}}}),
        json!({"toolUse":{"toolUseId":"tool-1","name":"read_file","input":{}}}),
    ];
    let calls = vec![ToolCallRequest {
        id: "tool-1".into(),
        name: "read_file".into(),
        arguments: "{}".into(),
        thought_signature: None,
    }];
    let replay = ProviderReplayPayload::BedrockAssistantBlocks(BedrockAssistantReplay {
        content: blocks.clone(),
        request_prefix: request_prefix(&initial),
    });
    let envelope = super::super::provider_turn::ProviderTurnEnvelope::capture_with_replay_payload(
        "turn",
        "sample",
        route.clone(),
        "",
        Some("analysis"),
        None,
        calls.clone(),
        true,
        Some(replay),
    );
    assert!(envelope.authorizes_tool_dispatch());
    let mut assistant = Message::text(Role::Assistant, "");
    assistant.tool_calls = Some(calls);
    assistant.set_provider_turn(envelope);
    let mut result = Message::text(Role::Tool, "receipt");
    result.name = Some("tool-1".into());
    result.parts.push(ContentPart::Image {
        media_type: "image/png".into(),
        data: "cGl4ZWxz".into(),
    });
    request.messages.extend([assistant, result]);
    request.reasoning_enabled = Some(false);
    let preserved = body(&request, &route).unwrap();
    assert_eq!(preserved["messages"][1]["content"], json!(blocks));
    assert_eq!(
        preserved["additionalModelRequestFields"]["thinking"]["type"],
        "disabled"
    );
    assert_eq!(
        preserved["messages"][2]["content"][0]["toolResult"]["content"][1]["image"]["source"]
            ["bytes"],
        "cGl4ZWxz"
    );
    request.messages[0] = Message::text(Role::System, "updated rules");
    let changed = body(&request, &route).unwrap();
    assert_eq!(
        changed["messages"][1]["content"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        changed["messages"][1]["content"][0]["toolUse"]["toolUseId"],
        "tool-1"
    );
    request.reasoning_effort = Some(ReasoningEffort::Max);
    assert!(body(&request, &route).is_err());
}

#[test]
fn unsigned_summaries_and_empty_guardrail_outcomes_are_not_transport_failures() {
    let value = json!({"output":{"message":{"content":[{"reasoningContent":{"reasoningText":{"text":"summary"}}},{"text":"answer"}]}},"stopReason":"end_turn","usage":{"inputTokens":2,"outputTokens":3,"totalTokens":5}});
    assert_eq!(
        response(value.clone(), "prefix".into(), false)
            .unwrap()
            .thinking
            .as_deref(),
        Some("summary")
    );
    assert!(response(value, "prefix".into(), true).is_err());
    let filtered = response(
        json!({"output":{"message":{"content":[]}},"stopReason":"guardrail_intervened","usage":{}}),
        "prefix".into(),
        true,
    )
    .unwrap();
    assert_eq!(filtered.finish_reason, FinishReason::ContentFilter);
    assert!(filtered.provider_replay.is_none());
    let mut state = StreamState::default();
    state
        .event("messageStart", json!({"role":"assistant"}))
        .unwrap();
    state
        .event("messageStop", json!({"stopReason":"content_filtered"}))
        .unwrap();
    state.event("metadata", json!({"usage":{}})).unwrap();
    assert_eq!(
        state
            .terminal("prefix".into(), true)
            .unwrap()
            .0
            .finish_reason,
        Some(FinishReason::ContentFilter)
    );
}

#[test]
fn profile_arn_is_one_path_segment_and_tool_results_keep_ids() {
    let provider = BedrockProvider::new(config(
        "https://bedrock-runtime.us-east-1.amazonaws.com".into(),
    ))
    .unwrap();
    let url = provider
        .url("arn:aws:bedrock:us-east-1:123:inference-profile/demo", true)
        .unwrap();
    assert!(url
        .as_str()
        .ends_with("arn:aws:bedrock:us-east-1:123:inference-profile%2Fdemo/converse-stream"));
    let mut assistant = Message::text(Role::Assistant, "");
    assistant.tool_calls = Some(vec![ToolCallRequest {
        id: "call-1".into(),
        name: "read_file".into(),
        arguments: "{}".into(),
        thought_signature: None,
    }]);
    let mut result = Message::text(Role::Tool, "receipt");
    result.name = Some("call-1".into());
    let request = CompletionRequest {
        model: "global.anthropic.claude-opus-5-5".into(),
        messages: vec![
            Message::text(Role::System, "rules"),
            Message::text(Role::User, "task"),
            assistant,
            result,
        ],
        ..Default::default()
    };
    let wire = body(&request, &provider.route_snapshot(&request)).unwrap();
    assert_eq!(
        wire["messages"][2]["content"][0]["toolResult"]["toolUseId"],
        "call-1"
    );
    assert_eq!(wire["system"][0]["text"], "rules");
    assert!(wire.get("model").is_none());
    assert!(!wire.to_string().contains("cachePoint"));
}

#[tokio::test]
async fn live_stream_uses_bearer_binary_frames_and_retains_usage() {
    use futures::StreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut input = Vec::new();
        loop {
            let mut buffer = [0u8; 4096];
            let n = socket.read(&mut buffer).await.unwrap();
            input.extend_from_slice(&buffer[..n]);
            if let Some(end) = input.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&input[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                if input.len() >= end + 4 + length {
                    break;
                }
            }
            assert!(n > 0);
        }
        let request = String::from_utf8(input).unwrap();
        assert!(request.contains("POST /model/global.anthropic.claude-opus-5-5/converse-stream"));
        assert!(request.contains("authorization: Bearer bedrock-test-key"));
        assert!(!request.contains("x-api-key"));
        let output: Vec<u8> = tool_events()
            .into_iter()
            .flat_map(|(kind, value)| event_stream::frame(kind, value))
            .collect();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/vnd.amazon.eventstream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",output.len()).as_bytes()).await.unwrap();
        for piece in output.chunks(11) {
            socket.write_all(piece).await.unwrap();
        }
    });
    let provider = BedrockProvider::new(config(format!("http://{address}"))).unwrap();
    let request = CompletionRequest {
        model: "global.anthropic.claude-opus-5-5".into(),
        messages: vec![Message::text(Role::User, "task")],
        ..Default::default()
    };
    let events: Vec<_> = provider
        .stream_events(&request)
        .await
        .unwrap()
        .collect()
        .await;
    server.await.unwrap();
    assert!(
        matches!(events.last(),Some(ProviderStreamEvent::Chunk {chunk}) if chunk.usage.as_ref().is_some_and(|usage| usage.total_tokens==15))
    );
    assert!(events.iter().any(
        |event| matches!(event,ProviderStreamEvent::ReplayState{replay} if replay.is_present())
    ));
    assert_eq!(events.iter().filter(|event| matches!(event,ProviderStreamEvent::Chunk{chunk} if chunk.tool_call_delta.is_some())).count(),3);
}
