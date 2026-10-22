//! Acceptance tests at the executor/provider seam, including completed tool replay.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::{self, BoxStream};

use super::*;
use crate::llm::provider_turn::{AnthropicThinkingBlock, ProviderReplayPayload, RouteSnapshot};
use crate::llm::reasoning_profile::ReasoningApiStyle;
use crate::llm::{CompletionResponse, FinishReason, StreamChunk};
use crate::tools::{Tool, ToolExecutionContext, ToolResult};

const ACTIVE_REQUEST: &str =
    "Inspect every fixture batch. Preserve the exact acceptance criterion: ACCEPTANCE_314159.";
const TOOL_ROUNDS: usize = 12;

struct ContextFixtureTool {
    steering: Option<mpsc::UnboundedSender<AgentSteeringMessage>>,
}

#[async_trait]
impl Tool for ContextFixtureTool {
    fn name(&self) -> &str {
        "context_fixture_batch"
    }

    fn description(&self) -> &str {
        "Return the requested batch of fixture evidence."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "batch": { "type": "integer" },
                "rows": { "type": "integer" }
            },
            "required": ["batch"]
        })
    }

    async fn execute(&self, ctx: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        let args: serde_json::Value = serde_json::from_str(ctx.arguments).unwrap();
        let batch = args["batch"].as_u64().unwrap();
        let rows = args["rows"].as_u64().unwrap_or(160);
        if batch == 2 {
            if let Some(steering) = &self.steering {
                steering
                    .send(AgentSteeringMessage::text(
                        "Keep the additional STEERING_271828 constraint.",
                    ))
                    .unwrap();
            }
        }
        let content = (0..rows)
            .map(|row| format!("BATCH_{batch}_EVIDENCE row_{row} accepted_source_{row}\n"))
            .collect::<String>();
        Ok(ToolResult {
            call_id: ctx.call_id.to_string(),
            content,
            is_error: false,
            artifacts: None,
        })
    }
}

struct ContextFixtureProvider {
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
    rounds: usize,
    tools_per_round: usize,
    rows: usize,
    opaque_replay: bool,
    usage: Option<Usage>,
}

#[async_trait]
impl LlmProvider for ContextFixtureProvider {
    fn name(&self) -> &str {
        "context-window-fixture"
    }

    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec!["gpt-4o".into()])
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        if let Some(usage) = self.usage.clone() {
            self.requests.lock().unwrap().push(request.clone());
            return Ok(CompletionResponse {
                content: "Fixture complete.".into(),
                tool_calls: None,
                finish_reason: FinishReason::Stop,
                usage,
                thinking: None,
                provider_replay: None,
            });
        }
        Err(CoreError::Llm(
            "fixture expects a streaming model step".into(),
        ))
    }

    fn reasoning_replay_policy(&self, _: &str) -> ReasoningReplayPolicy {
        if self.opaque_replay {
            ReasoningReplayPolicy::OpaqueSignature
        } else {
            ReasoningReplayPolicy::NotRequired
        }
    }

    fn route_snapshot(&self, request: &CompletionRequest) -> RouteSnapshot {
        let mut route = RouteSnapshot::unknown(
            self.name(),
            &request.model,
            self.reasoning_replay_policy(&request.model),
        );
        if self.opaque_replay {
            route.api_style = ReasoningApiStyle::AnthropicMessages;
        } else if self.usage.is_some() {
            route.api_style = ReasoningApiStyle::OpenAiChatCompletions;
        }
        route
    }

    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let mut requests = self.requests.lock().unwrap();
        let batch = requests.len();
        requests.push(request.clone());
        drop(requests);
        let mut events = Vec::new();
        if batch < self.rounds {
            for index in 0..self.tools_per_round {
                let evidence_batch = batch * self.tools_per_round + index;
                events.push(ProviderStreamEvent::Chunk {
                    chunk: Box::new(StreamChunk {
                        delta: String::new(),
                        tool_call_delta: Some(ToolCallDelta {
                            id: format!("fixture-call-{batch}-{index}"),
                            name: Some("context_fixture_batch".into()),
                            arguments_delta: format!(
                                r#"{{"batch":{evidence_batch},"rows":{}}}"#,
                                self.rows
                            )
                            .into(),
                            index: Some(u32::try_from(index).unwrap()),
                            thought_signature: None,
                        }),
                        finish_reason: (index + 1 == self.tools_per_round)
                            .then_some(FinishReason::ToolCalls),
                        usage: (index + 1 == self.tools_per_round)
                            .then(|| self.usage.clone())
                            .flatten(),
                        thinking_delta: None,
                    }),
                });
            }
            if self.opaque_replay {
                events.push(ProviderStreamEvent::ReplayState {
                    replay: Box::new(ProviderReplayPayload::AnthropicThinkingBlocks(vec![
                        AnthropicThinkingBlock::Thinking {
                            thinking: format!("Inspect fixture batch {batch}"),
                            signature: format!("opaque-fixture-signature-{batch}"),
                        },
                    ])),
                });
            }
        } else {
            events.push(ProviderStreamEvent::Chunk {
                chunk: Box::new(StreamChunk {
                    delta: "All fixture evidence was checked; ACCEPTANCE_314159 is satisfied."
                        .into(),
                    tool_call_delta: None,
                    finish_reason: Some(FinishReason::Stop),
                    usage: self.usage.clone(),
                    thinking_delta: None,
                }),
            });
        }
        Ok(Box::pin(stream::iter(events)))
    }

    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

struct ContextFixtureSummarizer(Arc<AtomicUsize>);

#[async_trait]
impl LlmProvider for ContextFixtureSummarizer {
    fn name(&self) -> &str {
        "context-summary-fixture"
    }

    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec!["gpt-4o".into()])
    }

    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(CompletionResponse {
            content: "Earlier fixture batches were inspected. Continue checking the remaining batches and preserve ACCEPTANCE_314159.".into(),
            tool_calls: None,
            finish_reason: FinishReason::Stop,
            usage: Usage::default(),
            thinking: None,
            provider_replay: None,
        })
    }

    async fn stream_events(
        &self,
        _: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        Err(CoreError::Llm("summary uses completion".into()))
    }

    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

async fn run_context_fixture(
    rounds: usize,
    tools_per_round: usize,
    rows: usize,
    opaque_replay: bool,
    with_steering: bool,
) -> (
    Result<Message, CoreError>,
    Vec<CompletionRequest>,
    usize,
    u32,
) {
    let (steering_tx, steering_rx) = mpsc::unbounded_channel();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ContextFixtureTool {
        steering: with_steering.then_some(steering_tx),
    }));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let summaries = Arc::new(AtomicUsize::new(0));
    let executor = AgentExecutor::new(
        Box::new(ContextFixtureProvider {
            requests: Arc::clone(&requests),
            rounds,
            tools_per_round,
            rows,
            opaque_replay,
            usage: None,
        }),
        registry,
        AgentConfig {
            system_prompt: "Check fixture evidence and report the observed result.".into(),
            model: Some("gpt-4o".into()),
            context_window: Some(8_192),
            max_tokens: Some(512),
            auto_compact_percent: Some(60),
            tool_approval_mode: ToolApprovalMode::AllowAll,
            ..AgentConfig::default()
        },
    )
    .with_skills_override(Vec::new())
    .with_auto_loaded_skills_override(Vec::new())
    .with_steering_receiver(steering_rx)
    .with_summarization_provider(Box::new(ContextFixtureSummarizer(Arc::clone(&summaries))));
    let db = Database::open_memory().unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    let drain = tokio::spawn(async move {
        let mut latest = 0;
        while let Some(event) = rx.recv().await {
            if let AgentEvent::UsageUpdate {
                last_prompt_tokens, ..
            } = event
            {
                latest = last_prompt_tokens;
            }
        }
        latest
    });
    let answer = executor
        .run(
            Vec::new(),
            vec![ContentPart::Text {
                text: ACTIVE_REQUEST.into(),
            }],
            &db,
            None,
            None,
            tx,
            0,
        )
        .await;
    let latest_occupancy = drain.await.unwrap();
    assert_eq!(
        executor.context_metrics.lock().unwrap().analyzed_tool_message_count(),
        rounds * tools_per_round,
        "budgeting, cache diagnostics, compaction, and usage must share each completed tool result analysis"
    );
    let requests = requests.lock().unwrap().clone();
    (
        answer,
        requests,
        summaries.load(Ordering::SeqCst),
        latest_occupancy,
    )
}

#[tokio::test]
async fn provider_usage_counts_anthropic_cache_in_occupancy_without_changing_billing() {
    check_provider_usage(ProviderType::Anthropic, "anthropic", true, 100).await;
}

struct UnavailableContextProvider;
#[async_trait]
impl LlmProvider for UnavailableContextProvider {
    fn name(&self) -> &str {
        "unavailable-context-fixture"
    }
    fn reasoning_replay_policy(&self, _: &str) -> ReasoningReplayPolicy {
        ReasoningReplayPolicy::NotRequired
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }
    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        Err(CoreError::TransientLlm("fixture unavailable".into()))
    }
    async fn stream_events(
        &self,
        _: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        Err(CoreError::TransientLlm("fixture unavailable".into()))
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

#[tokio::test]
async fn context_hud_measures_the_same_replay_projection_as_each_model_request() {
    for routed in 0..3 {
        check_replay_projected_hud(routed).await;
    }
}

async fn check_replay_projected_hud(routed: u8) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(ContextFixtureTool { steering: None }));
    let provider: Box<dyn LlmProvider> = Box::new(ContextFixtureProvider {
        requests: Arc::clone(&requests),
        rounds: 2,
        tools_per_round: 1,
        rows: 1,
        opaque_replay: true,
        usage: Some(Usage {
            prompt_tokens: 1000,
            completion_tokens: 20,
            total_tokens: 1020,
            ..Default::default()
        }),
    });
    let provider = if routed == 2 {
        Box::new(
            crate::llm::fallback::AutomaticFallbackProvider::new(
                0,
                Box::new(UnavailableContextProvider),
                "gpt-4o".into(),
                ProviderType::OpenAi,
                vec![crate::llm::fallback::AutomaticFallbackCandidate {
                    fallback_index: 1,
                    provider,
                    model: "gpt-4o".into(),
                    provider_type: ProviderType::OpenAi,
                }],
                Arc::new(|_, _, _| Ok(())),
            )
            .unwrap(),
        ) as Box<dyn LlmProvider>
    } else if routed == 1 {
        Box::new(
            crate::llm::fallback::AutomaticFallbackProvider::new(
                0,
                provider,
                "gpt-4o".into(),
                ProviderType::OpenAi,
                vec![],
                Arc::new(|_, _, _| Ok(())),
            )
            .unwrap(),
        ) as Box<dyn LlmProvider>
    } else {
        provider
    };
    let executor = AgentExecutor::new(
        provider,
        tools,
        AgentConfig {
            model: Some("gpt-4o".into()),
            context_window: Some(1_000_000),
            max_tokens: Some(512),
            tool_approval_mode: ToolApprovalMode::AllowAll,
            ..Default::default()
        },
    )
    .with_skills_override(Vec::new())
    .with_auto_loaded_skills_override(Vec::new());
    let mut legacy = Message::text(Role::Assistant, "");
    legacy.tool_calls = Some(vec![crate::llm::ToolCallRequest {
        id: "legacy".into(),
        name: "read_file".into(),
        arguments: "{}".into(),
        thought_signature: None,
    }]);
    let history = vec![
        Message::text(Role::User, "Earlier completed task"),
        legacy,
        Message::text_with_name(
            Role::Tool,
            "old detailed evidence ".repeat(20_000),
            "legacy",
        ),
        Message::text(Role::Assistant, "Earlier task finished."),
    ];
    let db = Database::open_memory().unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    let drain = tokio::spawn(async move {
        let mut occupied = Vec::new();
        while let Some(event) = rx.recv().await {
            if matches!(
                &event,
                AgentEvent::ToolRunStarted { .. } | AgentEvent::TextDelta { .. }
            ) {
                assert!(
                    occupied.last().is_some_and(|tokens| *tokens < 20_000),
                    "fallback must refresh occupancy before streaming visible output: {occupied:?}"
                );
            }
            if let AgentEvent::UsageUpdate {
                last_prompt_tokens, ..
            } = event
            {
                occupied.push(last_prompt_tokens);
            }
        }
        occupied
    });
    executor
        .run(
            history,
            vec![ContentPart::Text {
                text: ACTIVE_REQUEST.into(),
            }],
            &db,
            None,
            None,
            tx,
            0,
        )
        .await
        .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 3);
    assert!(requests.lock().unwrap().iter().all(|request| request
        .messages
        .iter()
        .all(|message| !message.text_content().contains("old detailed evidence"))));
    let occupied = drain.await.unwrap();
    assert!(occupied.iter().skip(if routed == 2 { 2 } else { 1 }).all(|tokens| *tokens < 1_200),
        "the same projected prefix must retain provider calibration, including recreated conclusions: {occupied:?}");
    assert!(occupied.iter().skip(usize::from(routed == 2)).all(|tokens| *tokens < 20_000),
        "the HUD must not alternate between canonical archives and the transmitted window: {occupied:?}");
}

#[tokio::test]
async fn provider_usage_uses_config_route_ids_without_double_counting_openai_cache() {
    for (provider, id) in [
        (ProviderType::OpenAi, "open_ai"),
        (ProviderType::OpenRouter, "openrouter"),
        (ProviderType::SiliconFlow, "siliconflow"),
    ] {
        check_provider_usage(provider, id, false, 5_400).await;
    }
}

async fn check_provider_usage(provider: ProviderType, id: &str, anthropic: bool, billed: u32) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let executor = AgentExecutor::new(
        Box::new(ContextFixtureProvider {
            requests: Arc::clone(&requests),
            rounds: 0,
            tools_per_round: 0,
            rows: 0,
            opaque_replay: anthropic,
            usage: Some(Usage {
                prompt_tokens: billed,
                completion_tokens: 20,
                total_tokens: billed + 20,
                cache_read_tokens: Some(5_000),
                cache_creation_tokens: Some(300),
                ..Usage::default()
            }),
        }),
        ToolRegistry::new(),
        AgentConfig {
            provider_type: Some(provider),
            model: Some("gpt-4o".into()),
            context_window: Some(8_192),
            max_tokens: Some(512),
            auto_compact_percent: Some(60),
            ..AgentConfig::default()
        },
    )
    .with_skills_override(Vec::new())
    .with_auto_loaded_skills_override(Vec::new());
    let db = Database::open_memory().unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    let drain = tokio::spawn(async move {
        let mut usage_events = Vec::new();
        while let Some(event) = rx.recv().await {
            if let AgentEvent::UsageUpdate {
                usage_total,
                last_prompt_tokens,
                context_breakdown: Some(breakdown),
            } = event
            {
                usage_events.push((usage_total, last_prompt_tokens, breakdown));
            }
        }
        usage_events
    });
    executor
        .run(
            Vec::new(),
            vec![ContentPart::Text {
                text: "Finish the fixture".into(),
            }],
            &db,
            None,
            None,
            tx,
            0,
        )
        .await
        .unwrap();
    let events = drain.await.unwrap();
    let (usage, displayed, breakdown) = events
        .iter()
        .find(|(usage, _, _)| usage.completion_tokens == 20)
        .unwrap();
    assert_eq!(
        *displayed, 5_400,
        "all occupied input counts, including cached input"
    );
    assert_eq!(breakdown.total_tokens, 5_400);
    assert_eq!(breakdown.runtime_provider.as_deref(), Some(id));
    assert_eq!(
        usage.prompt_tokens, billed,
        "context normalization must not change billing counters"
    );
    assert_eq!(usage.total_tokens, billed + 20);
    let request = requests.lock().unwrap()[0].clone();
    let window = context_window::ContextWindow::new("gpt-4o", Some(8_192), 512)
        .with_compact_percent(Some(60));
    let calibrated = executor.context_budget_usage(
        "gpt-4o",
        &request.messages,
        request.tools.as_deref().unwrap_or_default(),
        window,
    );
    assert_eq!(
        calibrated.total_tokens, 5_400,
        "next-request calibration must retain cached occupancy"
    );
    assert!(
        window
            .budget_decision(calibrated.total_tokens)
            .should_compact
    );
    assert_eq!(calibrated.runtime_provider.as_deref(), Some(id));
}

#[tokio::test]
async fn history_compaction_publishes_its_current_budget_before_the_first_request() {
    let summaries = Arc::new(AtomicUsize::new(0));
    let executor = AgentExecutor::new(
        Box::new(ContextFixtureProvider {
            requests: Arc::new(Mutex::new(Vec::new())),
            rounds: 0,
            tools_per_round: 0,
            rows: 0,
            opaque_replay: false,
            usage: None,
        }),
        ToolRegistry::new(),
        AgentConfig {
            model: Some("gpt-4o".into()),
            context_window: Some(8_192),
            max_tokens: Some(512),
            auto_compact_percent: Some(60),
            ..Default::default()
        },
    )
    .with_summarization_provider(Box::new(ContextFixtureSummarizer(Arc::clone(&summaries))));
    let history = vec![
        Message::text(Role::User, "Earlier request"),
        Message::text(Role::Assistant, "earlier evidence ".repeat(6_000)),
        Message::text(Role::User, "Recent request"),
        Message::text(Role::Assistant, "Recent answer"),
        Message::text(Role::User, "Latest request"),
        Message::text(Role::Assistant, "Latest answer"),
    ];
    let db = Database::open_memory().unwrap();
    let (tx, mut rx) = mpsc::channel(8);
    let (reduced, _) = executor
        .summarize_if_needed(
            history.clone(),
            "gpt-4o",
            512,
            &tx,
            context_compaction::CompactionRunContext {
                db: &db,
                conversation_id: None,
                turn_id: None,
                active_request: None,
            },
            None,
        )
        .await
        .unwrap();
    assert!(summaries.load(Ordering::SeqCst) > 0);
    assert_ne!(
        prompt_cache::message_sequence_fingerprint(&history),
        prompt_cache::message_sequence_fingerprint(&reduced)
    );
    let AgentEvent::UsageUpdate {
        usage_total,
        last_prompt_tokens,
        context_breakdown: Some(breakdown),
    } = rx.try_recv().unwrap()
    else {
        panic!("history gate must publish current occupancy");
    };
    assert_eq!(
        usage_total.total_tokens, 0,
        "an estimate is not newly billed usage"
    );
    assert_eq!(last_prompt_tokens, breakdown.total_tokens);
    assert_eq!(
        breakdown.measurement,
        Some(context::ContextMeasurement::Estimated)
    );
    let budget = breakdown.budget.unwrap();
    assert_eq!(budget.capacity_tokens, 8_192);
    assert!(last_prompt_tokens > budget.compact_threshold);
}

#[tokio::test]
async fn long_single_turn_keeps_active_request_and_latest_tool_evidence() {
    let (answer, requests, summaries, _) =
        run_context_fixture(TOOL_ROUNDS, 1, 160, false, false).await;
    let answer = answer.expect("a long single user turn must continue through safe compaction");
    assert!(answer.text_content().contains("ACCEPTANCE_314159"));
    assert_eq!(requests.len(), TOOL_ROUNDS + 1);
    for (index, request) in requests.iter().enumerate() {
        assert!(
            request.messages.iter().any(|message| {
                message.role == Role::User && message.text_content() == ACTIVE_REQUEST
            }),
            "model request {index} lost the original user's exact acceptance criterion"
        );
        if index > 0 {
            let latest = format!("BATCH_{}_EVIDENCE", index - 1);
            assert!(
                request.messages.iter().any(|message| {
                    message.role == Role::Tool && message.text_content().contains(&latest)
                }),
                "model request {index} lost the latest completed tool evidence"
            );
        }
        crate::llm::message_validation::validate_provider_request(
            &request.messages,
            "context-window-fixture",
            "gpt-4o",
        )
        .expect("compaction must preserve every tool call/result pair");
    }
    assert!(summaries > 0);
}

#[tokio::test]
async fn oversized_active_exchange_stops_before_another_provider_request() {
    let (answer, requests, _, occupied) = run_context_fixture(1, 1, 2_000, false, false).await;
    let error = answer.expect_err("an indivisible oversized exchange must not be silently dropped");
    assert!(error.to_string().contains("cannot fit"), "{error}");
    assert!(
        occupied > 8_192,
        "the rejected oversized request must replace the previous small occupancy: {occupied}"
    );
    assert_eq!(
        requests.len(),
        1,
        "no request may follow silent evidence eviction"
    );
}

#[tokio::test]
async fn compaction_preserves_parallel_calls_opaque_replay_and_user_steering() {
    let (answer, requests, summaries, _) =
        run_context_fixture(TOOL_ROUNDS, 2, 70, true, true).await;
    answer.expect("complete signed tool batches must survive long-turn compaction");
    assert_eq!(requests.len(), TOOL_ROUNDS + 1);
    assert!(summaries > 0);
    for (index, request) in requests.iter().enumerate() {
        assert!(request
            .messages
            .iter()
            .any(|message| message.role == Role::User && message.text_content() == ACTIVE_REQUEST));
        crate::llm::message_validation::validate_provider_request(
            &request.messages,
            "context-window-fixture",
            "gpt-4o",
        )
        .unwrap();
        if index > 0 {
            for evidence in [(index - 1) * 2, (index - 1) * 2 + 1] {
                assert!(
                    request
                        .messages
                        .iter()
                        .any(|message| message.role == Role::Tool
                            && message
                                .text_content()
                                .contains(&format!("BATCH_{evidence}_EVIDENCE"))),
                    "missing complete parallel result at request {index}"
                );
            }
        }
        for message in request
            .messages
            .iter()
            .filter(|message| message.tool_calls.is_some())
        {
            let envelope = message
                .provider_turn()
                .expect("retained assistant must keep typed replay");
            assert_eq!(envelope.tool_calls.len(), 2);
            let batch = envelope.tool_calls[0]
                .id
                .strip_prefix("fixture-call-")
                .unwrap()
                .split('-')
                .next()
                .unwrap();
            let ProviderReplayPayload::AnthropicThinkingBlocks(blocks) = &envelope.replay_payload
            else {
                panic!("opaque payload was replaced");
            };
            assert_eq!(
                blocks,
                &[AnthropicThinkingBlock::Thinking {
                    thinking: format!("Inspect fixture batch {batch}"),
                    signature: format!("opaque-fixture-signature-{batch}"),
                }]
            );
        }
    }
    assert!(requests
        .last()
        .unwrap()
        .messages
        .iter()
        .any(|message| message.role == Role::User
            && message.text_content().contains("STEERING_271828")));
}

#[cfg(feature = "document-processing")]
struct DocumentBudgetProvider {
    private_endpoint: bool,
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
}
#[cfg(feature = "document-processing")]
#[async_trait]
impl LlmProvider for DocumentBudgetProvider {
    fn name(&self) -> &str {
        "native-document-budget-fixture"
    }
    fn supports_document_input(
        &self,
        request: &CompletionRequest,
        document: &crate::llm::document::DocumentInput,
    ) -> bool {
        crate::llm::document::native_semantics(
            ProviderType::Anthropic,
            self.private_endpoint
                .then_some("https://private.example/v1"),
            ReasoningApiStyle::AnthropicMessages,
            &request.model,
            document,
        )
        .is_some()
    }
    fn route_snapshot(&self, request: &CompletionRequest) -> RouteSnapshot {
        let contract = crate::llm::model_contract::resolve_model_contract(
            ProviderType::Anthropic,
            self.private_endpoint
                .then_some("https://private.example/v1"),
            ReasoningApiStyle::AnthropicMessages,
            &request.model,
        );
        RouteSnapshot::from_profile_for_request(&contract.reasoning, request)
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }
    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        unreachable!()
    }
    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let wire = crate::llm::document::project_request(
            request,
            ProviderType::Anthropic,
            self.private_endpoint
                .then_some("https://private.example/v1"),
            ReasoningApiStyle::AnthropicMessages,
        )
        .into_owned();
        self.requests.lock().unwrap().push(wire);
        Ok(Box::pin(stream::iter(vec![ProviderStreamEvent::Chunk {
            chunk: Box::new(StreamChunk {
                delta: "Read extracted evidence.".into(),
                finish_reason: Some(FinishReason::Stop),
                tool_call_delta: None,
                usage: None,
                thinking_delta: None,
            }),
        }])))
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

#[tokio::test]
#[cfg(feature = "document-processing")]
async fn native_document_extraction_fallback_reaches_provider_before_large_pdf_budget_rejection() {
    for private_endpoint in [false, true] {
        let mut input = crate::llm::document::tests::pdf_request_with_pages(101);
        let parts = input.messages.remove(0).into_data().parts;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let executor = AgentExecutor::new(
            Box::new(DocumentBudgetProvider {
                private_endpoint,
                requests: Arc::clone(&requests),
            }),
            ToolRegistry::new(),
            AgentConfig {
                model: Some("claude-sonnet-4-5".into()),
                provider_type: Some(ProviderType::Anthropic),
                context_window: Some(128_000),
                max_tokens: Some(1024),
                system_prompt: "Read the supplied document.".into(),
                ..Default::default()
            },
        )
        .with_skills_override(vec![])
        .with_auto_loaded_skills_override(vec![]);
        let db = Database::open_memory().unwrap();
        let mut privacy = db.load_privacy_config().unwrap();
        privacy.enabled = false;
        db.save_privacy_config(&privacy).unwrap();
        let (tx, mut rx) = mpsc::channel(64);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = executor.run(vec![], parts, &db, None, None, tx, 0).await;
        drain.await.unwrap();
        assert!(result.is_ok(), "private={private_endpoint}: {result:?}");
        let seen = requests.lock().unwrap();
        assert!(!seen.is_empty());
        assert!(!seen[0].messages.iter().any(|m| m.has_documents()));
        assert!(seen[0]
            .messages
            .iter()
            .any(|m| m.text_content().contains("PDF fallback")));
    }
}

struct ForkingContextProvider {
    inner: ContextFixtureProvider,
    fork: Mutex<Option<crate::context_maintenance::ContextForkRequest>>,
    steering: mpsc::UnboundedSender<AgentSteeringMessage>,
}
#[async_trait]
impl LlmProvider for ForkingContextProvider {
    fn name(&self) -> &str {
        "manual-fork-fixture"
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }
    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        unreachable!()
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let events = self.inner.stream_events(request).await?;
        if let Some(fork) = self.fork.lock().unwrap().take() {
            self.steering
                .send(AgentSteeringMessage::fork(fork))
                .unwrap();
        }
        Ok(events)
    }
}

#[tokio::test]
async fn manual_fork_during_streaming_waits_for_tools_or_final_answer_and_accepts_short_windows() {
    for rounds in [0, 1] {
        let db = Database::open_memory().unwrap();
        let conversation = db
            .create_conversation(&crate::conversation::CreateConversationInput {
                provider: "custom".into(),
                model: "gpt-4o".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        db.add_message(&crate::conversation::ConversationMessage {
            id: "fork-user".into(),
            conversation_id: conversation.id.clone(),
            role: Role::User,
            content: ACTIVE_REQUEST.into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: None,
            token_count: 10,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        })
        .unwrap();
        let activities = crate::activity::ActivityRuntime::new();
        activities
            .start(
                crate::activity::ActivitySpec::new(
                    crate::activity::ActivitySurface::Maintenance,
                    "context_compaction",
                )
                .with_activity_id("manual-fork")
                .with_conversation_id(&conversation.id),
            )
            .unwrap();
        let (fork, completed) = crate::context_maintenance::ContextForkRequest::new(
            "manual-fork".into(),
            activities,
            CancellationToken::new(),
        );
        let (steering, receiver) = mpsc::unbounded_channel();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = ForkingContextProvider {
            inner: ContextFixtureProvider {
                requests: requests.clone(),
                rounds,
                tools_per_round: 1,
                rows: 1,
                opaque_replay: false,
                usage: None,
            },
            fork: Mutex::new(Some(fork)),
            steering,
        };
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(ContextFixtureTool { steering: None }));
        tools.register(Box::new(
            crate::tools::context_history_tool::ContextHistoryTool,
        ));
        let executor = AgentExecutor::new(
            Box::new(provider),
            tools,
            AgentConfig {
                model: Some("gpt-4o".into()),
                context_window: Some(1_000_000),
                max_tokens: Some(512),
                context_management_mode: crate::context_history::ContextManagementMode::History,
                tool_approval_mode: ToolApprovalMode::AllowAll,
                ..Default::default()
            },
        )
        .with_steering_receiver(receiver)
        .with_skills_override(vec![])
        .with_auto_loaded_skills_override(vec![]);
        let (tx, mut rx) = mpsc::channel(64);
        let drain = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                assert!(
                    !matches!(event, AgentEvent::StreamReset { .. }),
                    "Fork must not interrupt the current sample"
                );
            }
        });
        executor
            .run(
                vec![],
                vec![ContentPart::Text {
                    text: ACTIVE_REQUEST.into(),
                }],
                &db,
                Some(&conversation.id),
                None,
                tx,
                1,
            )
            .await
            .unwrap();
        drain.await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), completed)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            result.summary_kind.starts_with("history_fork"),
            "{result:?}"
        );
        let seen = requests.lock().unwrap();
        assert_eq!(
            seen.len(),
            rounds + 1,
            "Fork must not restart a completed answer"
        );
        if rounds == 1 {
            assert!(seen[1]
                .messages
                .iter()
                .any(|message| message.text_content().contains("fresh history handoff")));
            assert!(!seen[1]
                .messages
                .iter()
                .any(|message| message.text_content().contains("BATCH_0_EVIDENCE")));
        }
        let projection =
            crate::context_maintenance::load_context_projection(&db, &conversation.id).unwrap();
        assert!(projection.projected);
        if rounds == 0 {
            assert!(!projection
                .messages
                .iter()
                .any(|message| message.role == Role::Assistant));
            assert!(db
                .get_messages(&conversation.id)
                .unwrap()
                .iter()
                .any(|message| message.content.contains("ACCEPTANCE_314159 is satisfied")));
        }
    }
}
