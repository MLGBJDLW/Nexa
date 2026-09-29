//! Sonnet 5.5's bound assistant turns and between-tools request contract.
use super::*;
use crate::llm::provider_turn::{AnthropicAssistantReplay, ProviderReplayPayload};

pub(super) fn is_model(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case("claude-sonnet-5-5")
}

pub(super) fn between_tools(request: &CompletionRequest) -> bool {
    is_model(&request.model)
        && (request.reasoning_enabled == Some(false)
            || request.reasoning_effort == Some(ReasoningEffort::None))
}

fn remove_cache_hints(value: &mut serde_json::Value) {
    // Only protocol annotations are excluded. A user/tool argument or schema
    // property also named cache_control remains part of the bound input.
    for field in ["system", "tools", "content"] {
        if let Some(blocks) = value
            .get_mut(field)
            .and_then(serde_json::Value::as_array_mut)
        {
            for block in blocks {
                if let Some(fields) = block.as_object_mut() {
                    fields.remove("cache_control");
                }
            }
        }
    }
}

fn extend_prefix(prefix: &mut blake3::Hasher, value: impl Serialize) {
    let mut value = serde_json::to_value(value).expect("native prompt fields are serializable");
    remove_cache_hints(&mut value);
    let encoded = serde_json::to_vec(&value).expect("JSON prompt fields are serializable");
    prefix.update(&(encoded.len() as u64).to_le_bytes());
    prefix.update(&encoded);
}

fn prefix_seed(body: &AnthropicRequest) -> blake3::Hasher {
    let mut prefix = blake3::Hasher::new();
    extend_prefix(
        &mut prefix,
        serde_json::json!({"system":&body.system,"tools":&body.tools}),
    );
    prefix
}

pub(super) fn request_prefix(body: &AnthropicRequest) -> Option<String> {
    if !is_model(&body.model) {
        return None;
    }
    let mut prefix = prefix_seed(body);
    for message in &body.messages {
        extend_prefix(&mut prefix, message);
    }
    Some(prefix.finalize().to_hex().to_string())
}

/// A linear pass compares each saved turn with the prompt that produced it.
/// On edits, remove only bound thinking from the affected turn onward. Keep
/// visible text, tool calls and results. Unchanged signed blocks stay verbatim.
pub(super) fn reconcile_between_tools(body: &mut AnthropicRequest) {
    if !is_model(&body.model)
        || !body
            .thinking
            .as_ref()
            .is_some_and(|thinking| thinking.r#type == "between_tools")
    {
        return;
    }
    let mut prefix = prefix_seed(body);
    let mut changed = false;
    body.messages.retain_mut(|message| {
        if let Some(expected) = &message.replay_prefix {
            // A turn produced after the edit is bound to this corrected prefix.
            // It starts a valid suffix; do not keep discarding new thinking.
            changed = expected != prefix.finalize().to_hex().as_str();
        }
        if changed {
            match &mut message.content {
                AnthropicContent::RawBlocks(blocks) => blocks.retain(|block| {
                    !matches!(
                        block["type"].as_str(),
                        Some("thinking" | "redacted_thinking")
                    )
                }),
                AnthropicContent::Blocks(blocks) => blocks.retain(|block| {
                    !matches!(
                        block,
                        AnthropicContentBlock::Thinking { .. }
                            | AnthropicContentBlock::RedactedThinking { .. }
                    )
                }),
                AnthropicContent::Text(_) => {}
            }
        }
        let empty = match &message.content {
            AnthropicContent::RawBlocks(blocks) => blocks.is_empty(),
            AnthropicContent::Blocks(blocks) => blocks.is_empty(),
            AnthropicContent::Text(_) => false,
        };
        if !empty {
            extend_prefix(&mut prefix, message);
        }
        !empty
    });
}

pub(super) fn replay_payload(
    finish: Option<&FinishReason>,
    mut content: Vec<serde_json::Value>,
    request_prefix: Option<&str>,
) -> Option<ProviderReplayPayload> {
    let Some(request_prefix) = request_prefix else {
        return anthropic_pause_replay_payload(finish, content);
    };
    if !matches!(
        finish,
        Some(FinishReason::Stop | FinishReason::ToolCalls | FinishReason::ProviderPause)
    ) {
        return None;
    }
    let paused = matches!(finish, Some(FinishReason::ProviderPause));
    if paused {
        content.retain(|block| block["type"] != "tool_use");
    }
    // Retain even malformed material so the dispatch boundary rejects it;
    // dropping it here would turn an invalid optional replay into no replay.
    Some(ProviderReplayPayload::AnthropicAssistantBlocks(
        AnthropicAssistantReplay {
            content,
            request_prefix: request_prefix.to_string(),
            paused,
        },
    ))
}

#[cfg(test)]
#[path = "anthropic_sonnet55_tests.rs"]
mod tests;
