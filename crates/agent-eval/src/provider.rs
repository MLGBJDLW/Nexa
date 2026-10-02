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
}

pub type Calls = Arc<Mutex<Vec<ProviderInvocation>>>;

pub struct MeteredProvider {
    pub inner: Box<dyn LlmProvider>,
    pub calls: Calls,
}

impl MeteredProvider {
    fn begin(&self, request: &CompletionRequest) -> usize {
        let mut calls = self.calls.lock().unwrap();
        let index = calls.len();
        calls.push(ProviderInvocation {
            request_messages: request.messages.len(),
            input_image_digests: request_image_digests(request),
            ..Default::default()
        });
        index
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

struct MeasuredStream<'a> {
    inner: BoxStream<'a, ProviderStreamEvent>,
    calls: Calls,
    index: usize,
    started: Instant,
}

impl Stream for MeasuredStream<'_> {
    type Item = ProviderStreamEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let result = self.inner.as_mut().poll_next(cx);
        if let Poll::Ready(Some(event)) = &result {
            let mut calls = self.calls.lock().unwrap();
            let call = &mut calls[self.index];
            match event {
                ProviderStreamEvent::Chunk { chunk } => {
                    if (!chunk.delta.is_empty()
                        || chunk.thinking_delta.is_some()
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
                | ProviderStreamEvent::RecoverableError { .. }
                | ProviderStreamEvent::Cancelled { .. } => call.failed = true,
                _ => {}
            }
        }
        result
    }
}

impl Drop for MeasuredStream<'_> {
    fn drop(&mut self) {
        self.calls.lock().unwrap()[self.index].elapsed_ms =
            self.started.elapsed().as_millis() as u64;
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
        let index = self.begin(request);
        let started = Instant::now();
        let result = self.inner.complete(request).await;
        let mut calls = self.calls.lock().unwrap();
        let call = &mut calls[index];
        call.elapsed_ms = started.elapsed().as_millis() as u64;
        call.failed = result.is_err();
        if let Ok(response) = &result {
            apply_usage(call, &response.usage);
        }
        result
    }

    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let index = self.begin(request);
        let started = Instant::now();
        match self.inner.stream_events(request).await {
            Ok(inner) => Ok(Box::pin(MeasuredStream {
                inner,
                calls: self.calls.clone(),
                index,
                started,
            })),
            Err(error) => {
                let mut calls = self.calls.lock().unwrap();
                calls[index].elapsed_ms = started.elapsed().as_millis() as u64;
                calls[index].failed = true;
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
