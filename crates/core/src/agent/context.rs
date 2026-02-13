//! Context management — prepare and trim messages for LLM requests.

use chrono::Utc;

use crate::conversation::memory::{model_context_window, trim_to_context_window};
use crate::llm::{Message, Role};

/// Build a complete message list for an LLM request, trimmed to fit the
/// model's context window.
///
/// 1. Prepend the system prompt.
/// 2. Append conversation history.
/// 3. Append the new user message.
/// 4. Trim from the oldest non-system message to stay within the context window
///    minus `max_tokens_response` (reserved for the model's answer).
///
/// If `context_window_override` is `Some`, it takes priority over auto-detection.
pub fn prepare_messages(
    system_prompt: &str,
    history: &[Message],
    user_message: &str,
    model: &str,
    max_tokens_response: u32,
    context_window_override: Option<u32>,
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(history.len() + 2);

    // System message — always first, with current date/time appended.
    let system_with_datetime = format!(
        "{}\n\nCurrent date and time: {} (UTC)",
        system_prompt,
        Utc::now().format("%Y-%m-%d %H:%M UTC")
    );
    messages.push(Message {
        role: Role::System,
        content: system_with_datetime,
        name: None,
        tool_calls: None,
    });

    // Prior conversation turns.
    messages.extend_from_slice(history);

    // New user input.
    messages.push(Message {
        role: Role::User,
        content: user_message.to_string(),
        name: None,
        tool_calls: None,
    });

    // Trim to fit context window.
    let max_context = context_window_override.unwrap_or_else(|| model_context_window(model));
    trim_to_context_window(&messages, max_context, max_tokens_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            name: None,
            tool_calls: None,
        }
    }

    #[test]
    fn test_prepare_messages_basic() {
        let history = vec![
            msg(Role::User, "Hi"),
            msg(Role::Assistant, "Hello!"),
        ];
        let result = prepare_messages("System prompt", &history, "What's up?", "gpt-4o", 4096, None);

        // System is first, with datetime appended.
        assert_eq!(result[0].role, Role::System);
        assert!(result[0].content.starts_with("System prompt\n\nCurrent date and time:"));

        // Last message is the new user input.
        assert_eq!(result.last().unwrap().content, "What's up?");
        assert_eq!(result.last().unwrap().role, Role::User);
    }

    #[test]
    fn test_prepare_messages_trims_when_needed() {
        // Build a history that exceeds even a tiny context window.
        // Each message is ~220 chars ≈ 55 tokens. 200 messages ≈ 11000 tokens > 8192 - 512.
        let history: Vec<Message> = (0..200)
            .map(|i| {
                let padding = "x".repeat(200);
                msg(Role::User, &format!("Message number {i} {padding}"))
            })
            .collect();
        // Use an unknown model → 8192 token window, 512 reserved.
        let result = prepare_messages("Sys", &history, "New", "some-model", 512, None);

        // System message must survive.
        assert_eq!(result[0].role, Role::System);
        // Total input is 202 messages (1 sys + 200 history + 1 user).
        // With ~55 tokens/message and 7680 token budget, only ~140 fit.
        assert!(result.len() < 202, "expected trimming, got {} messages", result.len());
        assert!(result.len() > 2, "expected more than just sys+user");
        // Last message is the new user input.
        assert_eq!(result.last().unwrap().content, "New");
    }

    #[test]
    fn test_prepare_messages_empty_history() {
        let result = prepare_messages("Sys", &[], "Hello", "gpt-4o", 4096, None);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, Role::System);
        assert_eq!(result[1].role, Role::User);
        assert_eq!(result[1].content, "Hello");
    }
}
