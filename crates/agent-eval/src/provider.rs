use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Instant;

use async_trait::async_trait;
use futures::{stream, stream::BoxStream, Stream};
use nexa_core::error::CoreError;
use nexa_core::llm::{
    prompt_cache::PromptCacheProfile, provider_turn::RouteSnapshot,
    reasoning_profile::ReasoningReplayPolicy, CompletionRequest, CompletionResponse, FinishReason,
    LlmProvider, ProviderStreamEvent, ReplayHistoryProjection, StreamChunk, ToolCallDelta, Usage,
};
use serde::Serialize;

use crate::Task;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InvocationOutcome {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInvocation {
    pub elapsed_ms: u64,
    pub first_output_ms: Option<u64>,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cache_read_tokens: Option<u32>,
    pub cache_creation_tokens: Option<u32>,
    pub usage_reported: bool,
    pub request_messages: usize,
    pub input_image_digests: Vec<String>,
    pub failed: bool,
    pub outcome: Option<InvocationOutcome>,
}

pub type Calls = Arc<Mutex<Vec<ProviderInvocation>>>;

pub struct MeteredProvider {
    pub inner: Box<dyn LlmProvider>,
    pub calls: Calls,
}

impl MeteredProvider {
    fn begin(&self, request: &CompletionRequest) -> InvocationGuard {
        let started = Instant::now();
        let mut calls = self.calls.lock().unwrap();
        let index = calls.len();
        calls.push(ProviderInvocation {
            request_messages: request.messages.len(),
            input_image_digests: request_image_digests(request),
            ..Default::default()
        });
        InvocationGuard {
            calls: self.calls.clone(),
            index,
            started,
            terminal: None,
            finalized: false,
        }
    }
}

pub fn image_digest(media_type: &str, data: &str) -> String {
    let mut hash = blake3::Hasher::new();
    hash.update(media_type.as_bytes());
    hash.update(&[0]);
    hash.update(data.as_bytes());
    hash.finalize().to_hex().to_string()
}

fn request_image_digests(request: &CompletionRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part {
            nexa_core::llm::ContentPart::Image { media_type, data } => {
                Some(image_digest(media_type, data))
            }
            _ => None,
        })
        .collect()
}

fn apply_usage(call: &mut ProviderInvocation, usage: &Usage) {
    // Provider chunks report a cumulative snapshot for a provider invocation.
    // Replace it; never add repeated usage chunks or AgentEvent totals.
    call.prompt_tokens = usage.prompt_tokens;
    call.completion_tokens = usage.completion_tokens;
    call.cache_read_tokens = usage.cache_read_tokens;
    call.cache_creation_tokens = usage.cache_creation_tokens;
    call.usage_reported = true;
}

/// Owns one measurement from before the provider await until its final result.
/// Dropping an opening/completion future must account for cancellation too.
struct InvocationGuard {
    calls: Calls,
    index: usize,
    started: Instant,
    terminal: Option<InvocationOutcome>,
    finalized: bool,
}

impl InvocationGuard {
    fn observe(&mut self, event: &ProviderStreamEvent) {
        let mut calls = self.calls.lock().unwrap();
        let call = &mut calls[self.index];
        match event {
            ProviderStreamEvent::Chunk { chunk } => {
                if (!chunk.delta.is_empty()
                    || chunk
                        .thinking_delta
                        .as_deref()
                        .is_some_and(|thinking| !thinking.is_empty())
                    || chunk.tool_call_delta.is_some())
                    && call.first_output_ms.is_none()
                {
                    call.first_output_ms = Some(self.started.elapsed().as_millis() as u64);
                }
                if let Some(usage) = &chunk.usage {
                    apply_usage(call, usage);
                }
            }
            ProviderStreamEvent::TerminalError { .. }
            | ProviderStreamEvent::RecoverableError { .. } => {
                self.terminal.get_or_insert(InvocationOutcome::Failed);
            }
            ProviderStreamEvent::Cancelled { .. } => {
                self.terminal.get_or_insert(InvocationOutcome::Cancelled);
            }
            _ => {}
        }
    }

    fn finish(&mut self, fallback: InvocationOutcome) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let outcome = self.terminal.unwrap_or(fallback);
        let mut calls = self.calls.lock().unwrap();
        let call = &mut calls[self.index];
        call.elapsed_ms = self.started.elapsed().as_millis() as u64;
        call.failed = outcome != InvocationOutcome::Completed;
        call.outcome = Some(outcome);
    }
}

impl Drop for InvocationGuard {
    fn drop(&mut self) {
        self.finish(InvocationOutcome::Cancelled);
    }
}

fn error_outcome(error: &CoreError) -> InvocationOutcome {
    if matches!(error, CoreError::Cancelled(_)) {
        InvocationOutcome::Cancelled
    } else {
        InvocationOutcome::Failed
    }
}

struct MeasuredStream<'a> {
    inner: BoxStream<'a, ProviderStreamEvent>,
    invocation: InvocationGuard,
}

impl Stream for MeasuredStream<'_> {
    type Item = ProviderStreamEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let result = self.inner.as_mut().poll_next(cx);
        match &result {
            Poll::Ready(Some(event)) => self.invocation.observe(event),
            Poll::Ready(None) => self.invocation.finish(InvocationOutcome::Completed),
            Poll::Pending => {}
        }
        result
    }
}

#[async_trait]
impl LlmProvider for MeteredProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn stream_max_retries(&self) -> Option<u32> {
        self.inner.stream_max_retries()
    }
    fn prompt_cache_profile(&self, model: &str) -> PromptCacheProfile {
        self.inner.prompt_cache_profile(model)
    }
    fn reasoning_replay_policy(&self, model: &str) -> ReasoningReplayPolicy {
        self.inner.reasoning_replay_policy(model)
    }
    fn reasoning_replay_history_policy(&self, model: &str) -> ReasoningReplayPolicy {
        self.inner.reasoning_replay_history_policy(model)
    }
    fn replay_history_projection(&self, request: &CompletionRequest) -> ReplayHistoryProjection {
        self.inner.replay_history_projection(request)
    }
    fn route_snapshot(&self, request: &CompletionRequest) -> RouteSnapshot {
        self.inner.route_snapshot(request)
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        self.inner.list_models().await
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        self.inner.health_check().await
    }
    async fn runtime_metadata(&self) -> Option<serde_json::Value> {
        self.inner.runtime_metadata().await
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        let mut invocation = self.begin(request);
        let result = self.inner.complete(request).await;
        if let Ok(response) = &result {
            apply_usage(
                &mut self.calls.lock().unwrap()[invocation.index],
                &response.usage,
            );
        }
        invocation.finish(match &result {
            Ok(_) => InvocationOutcome::Completed,
            Err(error) => error_outcome(error),
        });
        result
    }

    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let mut invocation = self.begin(request);
        match self.inner.stream_events(request).await {
            Ok(inner) => Ok(Box::pin(MeasuredStream { inner, invocation })),
            Err(error) => {
                invocation.finish(error_outcome(&error));
                Err(error)
            }
        }
    }
}

/// A deterministic transport fixture. This exercises the real executor and
/// real file tools, but is deliberately never labelled a model-quality score.
pub struct ScriptedProvider {
    task: Task,
    next_action: Mutex<usize>,
}

impl ScriptedProvider {
    pub fn new(task: Task) -> Self {
        Self {
            task,
            next_action: Mutex::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted-eval-transport"
    }
    fn reasoning_replay_policy(&self, _: &str) -> ReasoningReplayPolicy {
        ReasoningReplayPolicy::NotRequired
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec!["scripted-eval".into()])
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        let Some(summary) = self.task.compaction_summary.as_ref() else {
            return Err(CoreError::InvalidInput(
                "Scripted compaction requires the task's explicit recorded summary fixture".into(),
            ));
        };
        Ok(CompletionResponse {
            content: summary.clone(),
            tool_calls: None,
            finish_reason: FinishReason::Stop,
            usage: Usage {
                prompt_tokens: request.messages.len() as u32 * 10,
                completion_tokens: 10,
                total_tokens: request.messages.len() as u32 * 10 + 10,
                ..Default::default()
            },
            thinking: None,
            provider_replay: None,
        })
    }
    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let mut cursor = self.next_action.lock().unwrap();
        let chunk = if let Some(action) = self.task.reference_actions.get(*cursor) {
            *cursor += 1;
            StreamChunk {
                delta: String::new(),
                thinking_delta: None,
                tool_call_delta: Some(ToolCallDelta {
                    id: format!("eval-{}-{}", self.task.id, *cursor),
                    name: Some(action.tool.clone()),
                    arguments_delta: action.arguments.to_string().into(),
                    index: Some(0),
                    thought_signature: None,
                }),
                finish_reason: Some(FinishReason::ToolCalls),
                usage: Some(Usage {
                    prompt_tokens: request.messages.len() as u32 * 10,
                    completion_tokens: 10,
                    total_tokens: request.messages.len() as u32 * 10 + 10,
                    ..Default::default()
                }),
            }
        } else {
            StreamChunk {
                delta: self.task.reference_answer.clone(),
                thinking_delta: None,
                tool_call_delta: None,
                finish_reason: Some(FinishReason::Stop),
                usage: Some(Usage {
                    prompt_tokens: request.messages.len() as u32 * 10,
                    completion_tokens: 10,
                    total_tokens: request.messages.len() as u32 * 10 + 10,
                    ..Default::default()
                }),
            }
        };
        Ok(Box::pin(stream::iter([ProviderStreamEvent::Chunk {
            chunk: Box::new(chunk),
        }])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use nexa_core::llm::{ProviderRecoveryCategory, ProviderStreamFailure};
    use std::time::Duration;

    #[derive(Default)]
    struct ProbeProvider {
        pending_open: bool,
        pending_complete: bool,
        pending_tail: bool,
        error_is_cancelled: Option<bool>,
        events: Vec<ProviderStreamEvent>,
    }

    impl ProbeProvider {
        fn error(&self) -> Option<CoreError> {
            self.error_is_cancelled.map(|cancelled| {
                if cancelled {
                    CoreError::Cancelled("fixture cancellation".into())
                } else {
                    CoreError::Internal("fixture provider failure".into())
                }
            })
        }
    }

    #[async_trait]
    impl LlmProvider for ProbeProvider {
        fn name(&self) -> &str {
            "metering-probe"
        }

        async fn list_models(&self) -> Result<Vec<String>, CoreError> {
            Ok(Vec::new())
        }

        async fn health_check(&self) -> Result<(), CoreError> {
            Ok(())
        }

        async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
            if self.pending_complete {
                return futures::future::pending().await;
            }
            if let Some(error) = self.error() {
                return Err(error);
            }
            Ok(CompletionResponse {
                content: "complete".into(),
                tool_calls: None,
                finish_reason: FinishReason::Stop,
                usage: Usage {
                    prompt_tokens: 25,
                    completion_tokens: 5,
                    total_tokens: 30,
                    ..Default::default()
                },
                thinking: None,
                provider_replay: None,
            })
        }

        async fn stream_events(
            &self,
            _: &CompletionRequest,
        ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
            if self.pending_open {
                return futures::future::pending().await;
            }
            if let Some(error) = self.error() {
                return Err(error);
            }
            let events = stream::iter(self.events.clone());
            if self.pending_tail {
                Ok(Box::pin(events.chain(stream::pending())))
            } else {
                Ok(Box::pin(events))
            }
        }
    }

    fn metered(probe: ProbeProvider) -> (MeteredProvider, Calls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            MeteredProvider {
                inner: Box::new(probe),
                calls: calls.clone(),
            },
            calls,
        )
    }

    fn completed_chunk() -> ProviderStreamEvent {
        ProviderStreamEvent::Chunk {
            chunk: Box::new(StreamChunk {
                delta: "complete".into(),
                thinking_delta: None,
                tool_call_delta: None,
                finish_reason: Some(FinishReason::Stop),
                usage: Some(Usage {
                    prompt_tokens: 25,
                    completion_tokens: 5,
                    total_tokens: 30,
                    ..Default::default()
                }),
            }),
        }
    }

    #[test]
    fn empty_thinking_delta_does_not_start_first_output_timing() {
        let (provider, calls) = metered(ProbeProvider::default());
        let mut invocation = provider.begin(&CompletionRequest::default());
        let mut chunk = StreamChunk {
            delta: String::new(),
            thinking_delta: Some(String::new()),
            tool_call_delta: None,
            finish_reason: None,
            usage: None,
        };
        invocation.observe(&ProviderStreamEvent::Chunk {
            chunk: Box::new(chunk.clone()),
        });
        assert_eq!(calls.lock().unwrap()[0].first_output_ms, None);
        chunk.thinking_delta = Some("reasoning".into());
        invocation.observe(&ProviderStreamEvent::Chunk {
            chunk: Box::new(chunk),
        });
        assert!(calls.lock().unwrap()[0].first_output_ms.is_some());
        invocation.finish(InvocationOutcome::Completed);
    }

    #[tokio::test]
    async fn cancelling_provider_open_and_completion_accounts_for_awaited_time() {
        let request = CompletionRequest::default();
        for opening_stream in [true, false] {
            let (provider, calls) = metered(ProbeProvider {
                pending_open: opening_stream,
                pending_complete: !opening_stream,
                ..Default::default()
            });
            let timeout = Duration::from_millis(20);
            let timed_out = if opening_stream {
                tokio::time::timeout(timeout, provider.stream_events(&request))
                    .await
                    .is_err()
            } else {
                tokio::time::timeout(timeout, provider.complete(&request))
                    .await
                    .is_err()
            };
            assert!(timed_out);
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].outcome, Some(InvocationOutcome::Cancelled));
            assert!(calls[0].failed);
            assert!(
                calls[0].elapsed_ms >= 10,
                "await time must not become local tool time"
            );
            assert!(!calls[0].usage_reported);
        }
    }

    #[tokio::test]
    async fn dropping_an_unfinished_stream_records_cancellation_and_elapsed_time() {
        let (provider, calls) = metered(ProbeProvider {
            pending_tail: true,
            ..Default::default()
        });
        let request = CompletionRequest::default();
        let mut events = provider.stream_events(&request).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), events.next())
                .await
                .is_err()
        );
        drop(events);
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].outcome, Some(InvocationOutcome::Cancelled));
        assert!(calls[0].failed);
        assert!(calls[0].elapsed_ms >= 10);
    }

    #[tokio::test]
    async fn completed_stream_is_finalized_at_eof_once_and_keeps_usage() {
        let (provider, calls) = metered(ProbeProvider {
            events: vec![completed_chunk()],
            ..Default::default()
        });
        let request = CompletionRequest::default();
        let mut events = provider.stream_events(&request).await.unwrap();
        assert!(events.next().await.is_some());
        assert!(events.next().await.is_none());
        let completed = calls.lock().unwrap()[0].clone();
        assert_eq!(completed.outcome, Some(InvocationOutcome::Completed));
        assert!(!completed.failed);
        assert!(completed.usage_reported);
        assert_eq!(
            (completed.prompt_tokens, completed.completion_tokens),
            (25, 5)
        );
        assert!(completed.first_output_ms.is_some());
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(events.next().await.is_none());
        drop(events);
        let final_call = calls.lock().unwrap()[0].clone();
        assert_eq!(final_call.elapsed_ms, completed.elapsed_ms);
        assert_eq!(final_call.outcome, completed.outcome);
    }

    #[tokio::test]
    async fn stream_errors_keep_their_outcome_when_the_consumer_stops_or_reaches_eof() {
        let request = CompletionRequest::default();
        let cases = [
            (
                ProviderStreamEvent::RecoverableError {
                    message: "retryable".into(),
                    category: ProviderRecoveryCategory::Network,
                },
                InvocationOutcome::Failed,
            ),
            (
                ProviderStreamEvent::TerminalError {
                    failure: ProviderStreamFailure::Internal {
                        message: "terminal".into(),
                    },
                },
                InvocationOutcome::Failed,
            ),
            (
                ProviderStreamEvent::Cancelled {
                    message: "cancelled".into(),
                },
                InvocationOutcome::Cancelled,
            ),
        ];
        for (event, expected) in cases {
            for consume_eof in [true, false] {
                let (provider, calls) = metered(ProbeProvider {
                    events: vec![event.clone()],
                    ..Default::default()
                });
                let mut events = provider.stream_events(&request).await.unwrap();
                assert!(events.next().await.is_some());
                if consume_eof {
                    assert!(events.next().await.is_none());
                }
                drop(events);
                let calls = calls.lock().unwrap();
                assert_eq!(calls[0].outcome, Some(expected));
                assert!(calls[0].failed);
            }
        }
    }

    #[tokio::test]
    async fn returned_errors_and_successful_completion_have_distinct_outcomes() {
        let request = CompletionRequest::default();
        for cancelled in [true, false] {
            let (provider, calls) = metered(ProbeProvider {
                error_is_cancelled: Some(cancelled),
                ..Default::default()
            });
            assert!(provider.complete(&request).await.is_err());
            assert!(provider.stream_events(&request).await.is_err());
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            for call in calls.iter() {
                assert!(call.failed);
                assert_eq!(
                    call.outcome,
                    Some(if cancelled {
                        InvocationOutcome::Cancelled
                    } else {
                        InvocationOutcome::Failed
                    })
                );
            }
        }
        let (provider, calls) = metered(ProbeProvider::default());
        provider.complete(&request).await.unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].outcome, Some(InvocationOutcome::Completed));
        assert!(!calls[0].failed);
        assert!(calls[0].usage_reported);
        assert_eq!(
            (calls[0].prompt_tokens, calls[0].completion_tokens),
            (25, 5)
        );
    }

    #[test]
    fn cumulative_usage_is_replaced_and_missing_usage_stays_unknown() {
        let mut call = ProviderInvocation::default();
        assert!(!call.usage_reported);
        let usage = Usage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            ..Default::default()
        };
        apply_usage(&mut call, &usage);
        apply_usage(&mut call, &usage);
        assert_eq!((call.prompt_tokens, call.completion_tokens), (100, 20));
    }
}
