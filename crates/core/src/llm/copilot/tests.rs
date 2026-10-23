use super::*;
use crate::llm::{
    provider_turn::ProviderTurnEnvelope, Message, ReasoningEffort, ToolCallRequest, ToolDefinition,
};
use futures::StreamExt;

fn config() -> ProviderConfig {
    ProviderConfig {
        provider_type: ProviderType::GithubCopilotApi,
        api_key: Some("test-account".to_string()),
        base_url: Some(DEFAULT_BASE_URL.into()),
        org_id: None,
        timeout_secs: Some(60),
        streaming: Default::default(),
    }
}

fn model(id: &str, endpoints: &[&str], adaptive: bool) -> Model {
    let mut model: Model = serde_json::from_value(json!({
        "id":id, "name":id, "supported_endpoints":endpoints, "model_picker_enabled":true,
        "capabilities":{"type":"chat", "supports":{"reasoning_effort":["low","medium","high"], "adaptive_thinking":adaptive},
        "limits":{"max_output_tokens":32000,"max_prompt_tokens":128000}}
    })).unwrap();
    model.account_scope = "account-a".to_string();
    model
}

#[test]
fn copilot_validates_hosts_and_preserves_exact_effort() {
    for base_url in [
        None,
        Some(String::new()),
        Some(format!("{DEFAULT_BASE_URL}/")),
    ] {
        let mut omitted_url = config();
        omitted_url.base_url = base_url;
        assert_eq!(
            CopilotProvider::new(omitted_url)
                .unwrap()
                .config
                .base_url
                .as_deref(),
            Some(DEFAULT_BASE_URL)
        );
    }
    for endpoint in [
        "http://api.githubcopilot.com",
        "https://api.githubcopilot.com.evil.test",
        "https://api.githubcopilot.com:8443",
        "https://api.githubcopilot.com/path",
        "https://token@api.githubcopilot.com",
        "https://api.githubcopilot.com/?q=1",
    ] {
        let mut cfg = config();
        cfg.base_url = Some(endpoint.into());
        assert!(CopilotProvider::new(cfg).is_err(), "{endpoint}");
    }
    for (id, paths, adaptive, style) in [
        (
            "claude-opus-5.5",
            vec!["/chat/completions", "/v1/messages"],
            true,
            ReasoningApiStyle::AnthropicMessages,
        ),
        (
            "gpt-6.1-sol",
            vec!["/chat/completions", "/responses"],
            false,
            ReasoningApiStyle::OpenAiResponses,
        ),
        (
            "gemini-4-argon",
            vec!["/chat/completions"],
            false,
            ReasoningApiStyle::OpenAiChatCompletions,
        ),
    ] {
        let model = model(id, &paths, adaptive);
        assert!(model.available());
        let mut disabled = model.clone();
        disabled.policy = Some(ModelPolicy {
            state: "disabled".into(),
        });
        assert!(!disabled.available());
        assert_eq!(model.api_style(), style);
        for effort in [
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ] {
            let request = model
                .resolve_request(&CompletionRequest {
                    model: id.into(),
                    reasoning_enabled: Some(true),
                    reasoning_effort: Some(effort.clone()),
                    ..Default::default()
                })
                .unwrap();
            let wire: Value = serde_json::from_slice(
                &model
                    .wire_body(&json!({"temperature":0.4}), &request, &config())
                    .unwrap(),
            )
            .unwrap();
            let actual = match style {
                ReasoningApiStyle::AnthropicMessages => {
                    assert_eq!(wire["thinking"]["display"], "summarized");
                    &wire["output_config"]["effort"]
                }
                ReasoningApiStyle::OpenAiResponses => {
                    assert_eq!(wire["reasoning"]["summary"], "auto");
                    &wire["reasoning"]["effort"]
                }
                _ => &wire["reasoning_effort"],
            };
            assert_eq!(actual, &serde_json::to_value(effort).unwrap());
            assert!(wire.get("temperature").is_none());
        }
        assert!(model
            .resolve_request(&CompletionRequest {
                model: id.into(),
                reasoning_effort: Some(ReasoningEffort::Ultra),
                ..Default::default()
            })
            .is_err());
    }
}

#[test]
fn copilot_opaque_replay_is_account_scoped_and_never_display_text() {
    let model = model("gemini-4-argon", &["/chat/completions"], false);
    let mut request = CompletionRequest {
        model: model.id.clone(),
        ..Default::default()
    };
    let payload = crate::llm::provider_turn::ProviderReplayPayload::CopilotReasoning {
        opaque: Some("private-signature/==".to_string()),
        text: Some("Visible summary".to_string()),
    };
    assert!(payload.reasoning_content_ref().is_none());
    let envelope = ProviderTurnEnvelope::capture_with_replay_payload(
        "turn",
        "sample",
        model.route(&config(), &request),
        "",
        Some("Visible summary"),
        None,
        vec![],
        true,
        Some(payload),
    );
    let mut message = Message::text(Role::Assistant, "".to_string());
    message.set_provider_turn(envelope);
    request.messages.push(message);
    let body = json!({"messages":[{"role":"assistant","content":""}]});
    let wire: Value =
        serde_json::from_slice(&model.wire_body(&body, &request, &config()).unwrap()).unwrap();
    assert_eq!(
        wire["messages"][0]["reasoning_opaque"],
        "private-signature/=="
    );
    assert_eq!(wire["messages"][0]["reasoning_text"], "Visible summary");
    let mut other = model.clone();
    other.account_scope = "account-b".to_string();
    let wire: Value =
        serde_json::from_slice(&other.wire_body(&body, &request, &config()).unwrap()).unwrap();
    assert!(wire["messages"][0].get("reasoning_opaque").is_none());
    let client = reqwest::Client::new();
    let first = headers(
        client.post(DEFAULT_BASE_URL),
        Some(&CompletionRequest {
            messages: vec![Message::text(Role::User, "hello".to_string())],
            ..Default::default()
        }),
    )
    .build()
    .unwrap();
    let continuation = headers(
        client.post(DEFAULT_BASE_URL),
        Some(&CompletionRequest {
            messages: vec![Message::text(Role::Tool, "result".to_string())],
            ..Default::default()
        }),
    )
    .build()
    .unwrap();
    assert_eq!(first.headers()["X-Initiator"], "user");
    assert_eq!(continuation.headers()["X-Initiator"], "agent");
}

async fn read_sample(
    provider: &dyn LlmProvider,
    request: &CompletionRequest,
) -> CompletionResponse {
    let mut stream = provider.stream_events(request).await.unwrap();
    let mut response = CompletionResponse {
        content: String::new(),
        tool_calls: None,
        finish_reason: crate::llm::FinishReason::ProtocolIncomplete,
        usage: Default::default(),
        thinking: None,
        provider_replay: None,
    };
    let mut calls = std::collections::BTreeMap::<u32, ToolCallRequest>::new();
    while let Some(event) = stream.next().await {
        match event {
            ProviderStreamEvent::Chunk { chunk } => {
                response.content.push_str(&chunk.delta);
                if let Some(thinking) = chunk.thinking_delta {
                    response
                        .thinking
                        .get_or_insert_default()
                        .push_str(&thinking);
                }
                if let Some(finish) = chunk.finish_reason {
                    response.finish_reason = finish;
                }
                if let Some(usage) = chunk.usage {
                    response.usage = usage;
                }
                if let Some(delta) = chunk.tool_call_delta {
                    let call =
                        calls
                            .entry(delta.index.unwrap_or(0))
                            .or_insert_with(|| ToolCallRequest {
                                id: String::new(),
                                name: String::new(),
                                arguments: String::new(),
                                thought_signature: None,
                            });
                    if !delta.id.is_empty() {
                        call.id = delta.id;
                    }
                    if let Some(name) = delta.name {
                        call.name = name;
                    }
                    if delta.arguments_delta.is_snapshot() {
                        call.arguments = delta.arguments_delta.to_string();
                    } else {
                        call.arguments.push_str(&delta.arguments_delta);
                    }
                    if delta.thought_signature.is_some() {
                        call.thought_signature = delta.thought_signature;
                    }
                }
            }
            ProviderStreamEvent::ReplayState { replay } => response.provider_replay = Some(*replay),
            ProviderStreamEvent::TerminalError { failure } => {
                panic!("Copilot stream failed: {failure:?}")
            }
            _ => {}
        }
    }
    if !calls.is_empty() {
        // The executor seals parameterless provider calls as an empty object.
        for call in calls.values_mut() {
            if call.arguments.trim().is_empty() {
                call.arguments = "{}".into();
            }
        }
        response.tool_calls = Some(calls.into_values().collect());
    }
    response
}

/// Opt-in account probe. Only the fixed synthetic problem leaves this process;
/// the account token is neither persisted nor included in diagnostic output.
#[tokio::test]
#[ignore = "requires an explicitly authorized GitHub CLI Copilot account"]
async fn live_copilot_api_tool_round_and_summary() {
    let output = std::process::Command::new("gh")
        .args(["auth", "token", "--hostname", "github.com"])
        .output()
        .unwrap();
    assert!(output.status.success(), "GitHub CLI login required");
    let mut cfg = config();
    cfg.api_key = Some(String::from_utf8(output.stdout).unwrap().trim().into());
    cfg.base_url = std::env::var("NEXA_COPILOT_API_PROBE_ENDPOINT")
        .ok()
        .or(cfg.base_url);
    let provider = super::super::create_provider(cfg).unwrap();
    let model = std::env::var("NEXA_COPILOT_API_PROBE_MODEL")
        .unwrap_or_else(|_| "claude-opus-5.5".to_string());
    let effort =
        std::env::var("NEXA_COPILOT_API_PROBE_EFFORT").unwrap_or_else(|_| "low".to_string());
    let mut request = CompletionRequest {
        model:model.clone(), reasoning_enabled:Some(true), reasoning_effort:Some(serde_json::from_value(json!(effort)).unwrap()),
        messages:vec![Message::text(Role::User, "Call read_problem exactly once, then solve the returned problem carefully. Give the numeric answer and a concise explanation.".to_string())],
        tools:Some(vec![ToolDefinition {name:"read_problem".to_string(),description:"Read the synthetic arithmetic problem".to_string(),parameters:json!({"type":"object","properties":{},"required":[],"additionalProperties":false})}]),
        ..Default::default()
    };
    let catalog = provider.model_catalog_snapshot().await.unwrap().unwrap();
    let descriptor = catalog
        .descriptors
        .iter()
        .find(|d| d.id == model)
        .expect("account model missing");
    assert!(descriptor.capabilities.tool_calling);
    assert!(descriptor.limits.context_tokens.is_some());
    let first = if std::env::var_os("NEXA_COPILOT_API_PROBE_NONSTREAM").is_some() {
        provider.complete(&request).await.unwrap()
    } else {
        read_sample(provider.as_ref(), &request).await
    };
    let calls = first.tool_calls.clone().expect("missing tool call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "read_problem");
    assert!(first.finish_reason.allows_completed_client_tools());
    let route = provider.route_snapshot(&request);
    let envelope = ProviderTurnEnvelope::capture_with_replay_payload(
        "turn",
        "sample",
        route.clone(),
        &first.content,
        first.thinking.as_deref(),
        None,
        calls.clone(),
        true,
        first.provider_replay.clone(),
    );
    assert!(
        envelope.authorizes_tool_dispatch(),
        "native replay did not authorize tool"
    );
    let mut assistant = Message::text(Role::Assistant, first.content.clone());
    assistant.tool_calls = Some(calls.clone());
    assistant.reasoning_content = first.thinking.clone();
    assistant.set_provider_turn(envelope);
    request.messages.push(assistant);
    let mut result = Message::text(
        Role::Tool,
        "Count nonnegative integer triples x+2y+3z=47, x<=17, y+z<=20.".to_string(),
    );
    result.name = Some(calls[0].id.clone());
    request.messages.push(result);
    let second = read_sample(provider.as_ref(), &request).await;
    assert!(matches!(
        second.finish_reason,
        crate::llm::FinishReason::Stop
    ));
    assert!(!second.content.trim().is_empty());
    assert!(second.tool_calls.is_none());
    let thinking_chars = first.thinking.as_ref().map_or(0, |s| s.chars().count())
        + second.thinking.as_ref().map_or(0, |s| s.chars().count());
    eprintln!("copilot_api model={model} effort={effort} protocol={:?} summary_chars={thinking_chars} answer_chars={} tool_round=passed",route.api_style,second.content.chars().count());
    if std::env::var_os("NEXA_COPILOT_API_PROBE_EXPECT_SUMMARY").is_some() {
        assert!(thinking_chars > 0, "provider omitted readable summary");
    }
}
