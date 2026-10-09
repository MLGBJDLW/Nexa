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
}

#[async_trait]
impl LlmProvider for ContextFixtureProvider {
    fn name(&self) -> &str {
        "context-window-fixture"
    }

    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec!["gpt-4o".into()])
    }

    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
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
                        usage: None,
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
                    usage: None,
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
) -> (Result<Message, CoreError>, Vec<CompletionRequest>, usize) {
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
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
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
    drain.await.unwrap();
    assert_eq!(
        executor.context_metrics.lock().unwrap().analyzed_tool_message_count(),
        rounds * tools_per_round,
        "budgeting, cache diagnostics, compaction, and usage must share each completed tool result analysis"
    );
    let requests = requests.lock().unwrap().clone();
    (answer, requests, summaries.load(Ordering::SeqCst))
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
    let (answer, requests, summaries) =
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
    let (answer, requests, _) = run_context_fixture(1, 1, 2_000, false, false).await;
    let error = answer.expect_err("an indivisible oversized exchange must not be silently dropped");
    assert!(error.to_string().contains("cannot fit"), "{error}");
    assert_eq!(
        requests.len(),
        1,
        "no request may follow silent evidence eviction"
    );
}

#[tokio::test]
async fn compaction_preserves_parallel_calls_opaque_replay_and_user_steering() {
    let (answer, requests, summaries) = run_context_fixture(TOOL_ROUNDS, 2, 70, true, true).await;
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
