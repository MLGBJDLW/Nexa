use log::warn;
use nexa_core::agent::{AgentEvent, StreamBlockChannel, ToolRunItem};
use nexa_core::agent_run::{
    AgentRunDisplayKind, AgentRunEvent, AgentRunEventImportance, AgentRunEventVisibility,
};
use nexa_core::llm::{ContentPart, Message};
use nexa_core::runtime::AgentRunEventOutbox;
pub(crate) use nexa_core::stream_projection::agent_event_rotates_stream_blocks;
#[cfg(test)]
pub(crate) use nexa_core::stream_projection::split_text_by_utf8_bytes;
use nexa_core::stream_projection::StreamBlockProjector;
use serde::Serialize;
use tauri::AppHandle;

use crate::app_events::emit_main_window_event;

/// Envelope for agent stream events sent to frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentFrontendEvent {
    conversation_id: String,
    #[serde(rename = "runEvent")]
    run_event: AgentRunEvent,
}

pub(crate) fn emit_agent_frontend_event(
    event_outbox: &AgentRunEventOutbox,
    conversation_id: &str,
    task_run_id: &str,
    turn_id: Option<&str>,
    event: AgentEvent,
) {
    let (_, run_event) = prepare_agent_run_event_for_frontend(task_run_id, turn_id, event);
    if let Err(error) = publish_run_event(event_outbox, run_event) {
        warn!("Failed to submit RunEvent for {conversation_id}: {error}");
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_agent_frontend_event_with_presentation(
    event_outbox: &AgentRunEventOutbox,
    conversation_id: &str,
    task_run_id: &str,
    turn_id: Option<&str>,
    event: AgentEvent,
    visibility: AgentRunEventVisibility,
    display_kind: AgentRunDisplayKind,
    importance: AgentRunEventImportance,
) {
    let (_, run_event) = prepare_agent_run_event_for_frontend(task_run_id, turn_id, event);
    let run_event = run_event.with_presentation(visibility, display_kind, importance);
    if let Err(error) = publish_run_event(event_outbox, run_event) {
        warn!("Failed to submit RunEvent for {conversation_id}: {error}");
    }
}

pub(crate) fn emit_agent_run_frontend_event(
    handle: &AppHandle,
    conversation_id: &str,
    run_event: &AgentRunEvent,
) {
    let payload = AgentFrontendEvent {
        conversation_id: conversation_id.to_string(),
        run_event: run_event.clone(),
    };
    emit_main_window_event(handle, "agent://run-event", &payload);
}

pub(crate) enum PendingStreamDelta {
    Text(String),
    Thinking(String),
}

pub(crate) const MAX_FRONTEND_TOOL_CONTENT_CHARS: usize = 128 * 1024;
const MAX_FRONTEND_TOOL_ARGUMENT_CHARS: usize = 128 * 1024;
const MAX_FRONTEND_MESSAGE_TEXT_CHARS: usize = 64 * 1024;
pub(crate) const MAX_FRONTEND_ARTIFACT_STRING_CHARS: usize = 32 * 1024;
const MAX_FRONTEND_ARTIFACT_ITEMS: usize = 512;
const MAX_FRONTEND_ARTIFACT_DEPTH: usize = 8;
pub(crate) const MAX_TASK_EVENT_TEXT_CHARS: usize = 4_000;

fn compact_json_value_for_frontend(value: &serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= MAX_FRONTEND_ARTIFACT_DEPTH {
        return match value {
            serde_json::Value::String(text) => serde_json::Value::String(truncate_task_event_text(
                text,
                MAX_FRONTEND_ARTIFACT_STRING_CHARS,
            )),
            other => serde_json::Value::String(truncate_task_event_text(
                &other.to_string(),
                MAX_FRONTEND_ARTIFACT_STRING_CHARS,
            )),
        };
    }

    match value {
        serde_json::Value::String(text) => serde_json::Value::String(truncate_task_event_text(
            text,
            MAX_FRONTEND_ARTIFACT_STRING_CHARS,
        )),
        serde_json::Value::Array(items) => {
            let mut out = items
                .iter()
                .take(MAX_FRONTEND_ARTIFACT_ITEMS)
                .map(|item| compact_json_value_for_frontend(item, depth + 1))
                .collect::<Vec<_>>();
            if items.len() > MAX_FRONTEND_ARTIFACT_ITEMS {
                out.push(serde_json::json!({
                    "truncated": true,
                    "omittedItems": items.len() - MAX_FRONTEND_ARTIFACT_ITEMS,
                }));
            }
            serde_json::Value::Array(out)
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (idx, (key, item)) in map.iter().enumerate() {
                if idx >= MAX_FRONTEND_ARTIFACT_ITEMS {
                    out.insert("_truncated".to_string(), serde_json::Value::Bool(true));
                    out.insert(
                        "_omittedKeys".to_string(),
                        serde_json::Value::Number(serde_json::Number::from(
                            map.len() - MAX_FRONTEND_ARTIFACT_ITEMS,
                        )),
                    );
                    break;
                }
                out.insert(
                    key.clone(),
                    compact_json_value_for_frontend(item, depth + 1),
                );
            }
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    }
}

fn compact_tool_run_for_frontend(mut run: ToolRunItem) -> ToolRunItem {
    run.arguments = run
        .arguments
        .map(|arguments| truncate_task_event_text(&arguments, MAX_FRONTEND_TOOL_ARGUMENT_CHARS));
    run.content = run
        .content
        .map(|content| truncate_task_event_text(&content, MAX_FRONTEND_TOOL_CONTENT_CHARS));
    run.progress_note = run
        .progress_note
        .map(|note| truncate_task_event_text(&note, MAX_TASK_EVENT_TEXT_CHARS));
    run.artifacts = run
        .artifacts
        .map(|artifacts| compact_json_value_for_frontend(&artifacts, 0));
    run
}

fn done_message_will_truncate(event: &AgentEvent) -> bool {
    let AgentEvent::Done { message, .. } = event else {
        return false;
    };
    message.parts.iter().any(|part| {
        matches!(part, ContentPart::Text { text } if text.chars().count() > MAX_FRONTEND_MESSAGE_TEXT_CHARS)
    })
}

fn compact_message_for_frontend(mut message: Message) -> Message {
    message.parts.retain_mut(|part| {
        match part {
            ContentPart::Text { text } => {
                *text = truncate_task_event_text(text, MAX_FRONTEND_MESSAGE_TEXT_CHARS);
                true
            }
            ContentPart::Image { data, .. } => {
                if data.len() > MAX_TASK_EVENT_TEXT_CHARS {
                    *data = "[image data omitted from stream event]".to_string();
                }
                true
            }
            // Provider replay envelopes can contain opaque signatures or
            // encrypted state. They are durable backend protocol state, never
            // a frontend stream payload.
            ContentPart::Document { document } => {
                *part = ContentPart::Text {
                    text: document.fallback("display extraction"),
                };
                true
            }
            ContentPart::ProviderTurn { .. } => false,
        }
    });
    message.reasoning_content = message
        .reasoning_content
        .as_deref()
        .map(|text| truncate_task_event_text(text, MAX_TASK_EVENT_TEXT_CHARS));
    message
}

fn mark_truncated_done_message(run_event: &mut AgentRunEvent, message_truncated: bool) {
    if !message_truncated {
        return;
    }
    if let Some(payload) = run_event.payload.as_object_mut() {
        payload.insert(
            "messageTruncated".to_string(),
            serde_json::Value::Bool(true),
        );
    }
}

pub(crate) fn prepare_agent_run_event_for_frontend(
    task_run_id: &str,
    turn_id: Option<&str>,
    event: AgentEvent,
) -> (AgentEvent, AgentRunEvent) {
    let message_truncated = done_message_will_truncate(&event);
    let event = compact_agent_event_for_frontend(event);
    let mut run_event =
        AgentRunEvent::from_agent_event(&event).with_context(Some(task_run_id), turn_id, Some(0));
    mark_truncated_done_message(&mut run_event, message_truncated);
    (event, run_event)
}

pub(crate) fn compact_agent_event_for_frontend(event: AgentEvent) -> AgentEvent {
    match event {
        AgentEvent::ToolCallStart {
            call_id,
            tool_name,
            arguments,
        } => AgentEvent::ToolCallStart {
            call_id,
            tool_name,
            arguments: truncate_task_event_text(&arguments, MAX_FRONTEND_TOOL_ARGUMENT_CHARS),
        },
        AgentEvent::ToolCallArgsDelta {
            call_id,
            tool_name,
            arguments_delta,
            index,
        } => AgentEvent::ToolCallArgsDelta {
            call_id,
            tool_name,
            arguments_delta: truncate_task_event_text(
                &arguments_delta,
                MAX_FRONTEND_TOOL_ARGUMENT_CHARS,
            ),
            index,
        },
        AgentEvent::ToolCallResult {
            call_id,
            tool_name,
            content,
            is_error,
            artifacts,
        } => AgentEvent::ToolCallResult {
            call_id,
            tool_name,
            content: truncate_task_event_text(&content, MAX_FRONTEND_TOOL_CONTENT_CHARS),
            is_error,
            artifacts: artifacts.map(|value| compact_json_value_for_frontend(&value, 0)),
        },
        AgentEvent::ToolRunStarted { run } => AgentEvent::ToolRunStarted {
            run: compact_tool_run_for_frontend(run),
        },
        AgentEvent::ToolRunUpdated { run } => AgentEvent::ToolRunUpdated {
            run: compact_tool_run_for_frontend(run),
        },
        AgentEvent::ToolRunCompleted { run } => AgentEvent::ToolRunCompleted {
            run: compact_tool_run_for_frontend(run),
        },
        AgentEvent::Thinking { content } => AgentEvent::Thinking {
            content: truncate_task_event_text(&content, MAX_FRONTEND_TOOL_CONTENT_CHARS),
        },
        AgentEvent::Status { content, tone } => AgentEvent::Status {
            content: truncate_task_event_text(&content, MAX_TASK_EVENT_TEXT_CHARS),
            tone,
        },
        AgentEvent::Steering { content } => AgentEvent::Steering {
            content: truncate_task_event_text(&content, MAX_TASK_EVENT_TEXT_CHARS),
        },
        AgentEvent::PlanUpdated {
            plan,
            phase,
            summary,
        } => AgentEvent::PlanUpdated {
            plan: compact_json_value_for_frontend(&plan, 0),
            phase,
            summary: summary.map(|text| truncate_task_event_text(&text, MAX_TASK_EVENT_TEXT_CHARS)),
        },
        AgentEvent::Done {
            message,
            assistant_message_id,
            usage_total,
            last_prompt_tokens,
            context_breakdown,
            cached,
            finish_reason,
        } => AgentEvent::Done {
            message: compact_message_for_frontend(message),
            assistant_message_id,
            usage_total,
            last_prompt_tokens,
            context_breakdown,
            cached,
            finish_reason,
        },
        other => other,
    }
}

pub(crate) struct StreamBlockEmitter {
    event_outbox: AgentRunEventOutbox,
    projector: StreamBlockProjector,
}

impl StreamBlockEmitter {
    pub(crate) fn new(event_outbox: AgentRunEventOutbox) -> Self {
        Self {
            event_outbox,
            projector: StreamBlockProjector::default(),
        }
    }

    pub(crate) fn rotate_blocks(&mut self) {
        self.projector.rotate();
    }

    pub(crate) fn emit_event(&self, conversation_id: &str, run_event: AgentRunEvent) {
        if let Err(error) = publish_run_event(&self.event_outbox, run_event) {
            warn!("Failed to submit RunEvent for {conversation_id}: {error}");
        }
    }

    pub(crate) fn flush_pending(
        &mut self,
        pending: &mut Option<PendingStreamDelta>,
        conversation_id: &str,
        task_run_id: &str,
        turn_id: Option<&str>,
    ) {
        let Some(delta) = pending.take() else {
            return;
        };
        match delta {
            PendingStreamDelta::Text(delta) if !delta.is_empty() => self.emit_block_delta(
                conversation_id,
                task_run_id,
                turn_id,
                StreamBlockChannel::Answer,
                delta,
            ),
            PendingStreamDelta::Thinking(content) if !content.is_empty() => self.emit_block_delta(
                conversation_id,
                task_run_id,
                turn_id,
                StreamBlockChannel::Thinking,
                content,
            ),
            _ => {}
        }
    }

    fn emit_block_delta(
        &mut self,
        conversation_id: &str,
        task_run_id: &str,
        turn_id: Option<&str>,
        channel: StreamBlockChannel,
        delta: String,
    ) {
        if let Err(error) =
            self.projector
                .submit(&self.event_outbox, task_run_id, turn_id, channel, &delta)
        {
            warn!("Failed to submit stream block for {conversation_id}: {error}");
        }
    }
}

fn publish_run_event(
    event_outbox: &AgentRunEventOutbox,
    run_event: AgentRunEvent,
) -> Result<(), nexa_core::runtime::AgentRunEventSubmitError> {
    if run_event.is_durable() {
        event_outbox.submit(run_event)
    } else {
        event_outbox.publish_ephemeral(run_event)
    }
}

pub(crate) fn truncate_task_event_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out = value
        .chars()
        .take(max_chars.saturating_sub(32))
        .collect::<String>();
    out.push_str("\n[truncated]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_core::llm::provider_turn::{ProviderTurnEnvelope, RouteSnapshot};
    use nexa_core::llm::reasoning_profile::{ReasoningApiStyle, ReasoningReplayPolicy};
    use nexa_core::llm::Role;

    #[test]
    fn frontend_messages_drop_provider_replay_envelopes() {
        let mut message = Message::text(Role::Assistant, "visible answer");
        message.set_provider_turn(ProviderTurnEnvelope::capture(
            "turn-item",
            "sample",
            RouteSnapshot {
                provider_endpoint_id: "openai-public".to_string(),
                provider_family: "openai".to_string(),
                api_style: ReasoningApiStyle::OpenAiResponses,
                model_id: "gpt-5.6".to_string(),
                reasoning_profile_id: "openai-responses-reasoning-v1".to_string(),
                reasoning_profile_version: 1,
                replay_policy: ReasoningReplayPolicy::RequiredOnToolCall,
            },
            "visible answer",
            None,
            None,
            Vec::new(),
            true,
        ));

        let compacted = compact_message_for_frontend(message);

        assert_eq!(compacted.text_content(), "visible answer");
        assert!(compacted.provider_turn().is_none());
    }

    #[test]
    fn frontend_done_event_marks_truncated_message_text() {
        let event = AgentEvent::Done {
            message: Message::text(
                Role::Assistant,
                "x".repeat(MAX_FRONTEND_MESSAGE_TEXT_CHARS + 1),
            ),
            usage_total: nexa_core::llm::Usage::default(),
            last_prompt_tokens: 0,
            context_breakdown: None,
            assistant_message_id: None,
            cached: false,
            finish_reason: Some("stop".to_string()),
        };
        let (compacted, run_event) =
            prepare_agent_run_event_for_frontend("run-1", Some("turn-1"), event);

        assert!(matches!(
            compacted,
            AgentEvent::Done { message, .. } if message.text_content().ends_with("[truncated]")
        ));
        assert_eq!(
            run_event.payload.get("messageTruncated"),
            Some(&serde_json::Value::Bool(true))
        );
    }
}
