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
    let mut trimmed = trim_to_context_window(&messages, max_context, max_tokens_response);

    // If messages were evicted, inject an extractive recap into the system prompt
    // so the LLM retains awareness of earlier conversation topics.
    let original_non_system = messages.iter().filter(|m| m.role != Role::System).count();
    let kept_non_system = trimmed.iter().filter(|m| m.role != Role::System).count();
    let evicted_count = original_non_system.saturating_sub(kept_non_system);

    if evicted_count > 0 {
        let evicted: Vec<&Message> = messages
            .iter()
            .filter(|m| m.role != Role::System)
            .take(evicted_count)
            .collect();

        let recap = build_evicted_recap(&evicted);
        if !recap.is_empty() {
            if let Some(sys) = trimmed.iter_mut().find(|m| m.role == Role::System) {
                sys.content = format!("{}\n\n{}", sys.content, recap);
            }
        }
    }

    trimmed
}

/// Build an extractive recap of evicted conversation messages.
///
/// Only includes `User` and `Assistant` messages (skips tool-call
/// intermediaries). The output is capped at ~800 characters (~200 tokens)
/// to avoid eating too much context budget.
fn build_evicted_recap(evicted: &[&Message]) -> String {
    const MAX_RECAP_CHARS: usize = 800;
    let mut parts: Vec<String> = Vec::new();
    let mut total_chars: usize = 0;

    for msg in evicted {
        if total_chars >= MAX_RECAP_CHARS {
            break;
        }

        match msg.role {
            Role::User => {
                let summary = truncate_text(&msg.content, 100);
                let line = format!("- User asked: {}", summary);
                total_chars += line.len();
                parts.push(line);
            }
            Role::Assistant => {
                // Skip tool-call intermediary messages.
                if msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty()) {
                    continue;
                }
                let summary = truncate_text(&msg.content, 80);
                let line = format!("- You answered: {}", summary);
                total_chars += line.len();
                parts.push(line);
            }
            _ => {} // Skip Tool and System messages.
        }
    }

    if parts.is_empty() {
        return String::new();
    }

    format!(
        "## Earlier conversation context (summarized)\n\
         These topics were discussed earlier but trimmed for context space:\n{}",
        parts.join("\n")
    )
}

/// Truncate text to `max_chars` on a word boundary, appending "..." if truncated.
fn truncate_text(text: &str, max_chars: usize) -> String {
    let clean = text.replace('\n', " ");
    let clean = clean.trim();
    if clean.len() <= max_chars {
        return clean.to_string();
    }
    let truncated = &clean[..max_chars];
    let cut = truncated.rfind(' ').unwrap_or(max_chars);
    format!("{}...", &clean[..cut])
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
        // Build a history that exceeds a small context window.
        // Alternate User/Assistant so the recap has both sides.
        // Each message: ~220 ASCII chars ≈ 59 tokens. 200 messages ≈ 11800 tokens.
        let history: Vec<Message> = (0..200)
            .map(|i| {
                let role = if i % 2 == 0 { Role::User } else { Role::Assistant };
                let padding = "x".repeat(200);
                msg(role, &format!("Message number {i} {padding}"))
            })
            .collect();
        // Force a small context window (8192) so trimming is guaranteed.
        let result = prepare_messages("Sys", &history, "New", "some-model", 512, Some(8192));

        // System message must survive.
        assert_eq!(result[0].role, Role::System);
        // Total input is 202 messages. With 7680 token budget and ~59 tok/msg, only ~130 fit.
        assert!(result.len() < 202, "expected trimming, got {} messages", result.len());
        assert!(result.len() > 2, "expected more than just sys+user");
        // Last message is the new user input.
        assert_eq!(result.last().unwrap().content, "New");

        // System message should contain the evicted recap.
        assert!(
            result[0].content.contains("Earlier conversation context"),
            "System message should contain evicted recap"
        );
    }

    #[test]
    fn test_no_recap_when_nothing_evicted() {
        let history = vec![
            msg(Role::User, "Hi"),
            msg(Role::Assistant, "Hello!"),
        ];
        let result = prepare_messages("Sys", &history, "What's up?", "gpt-4o", 4096, None);

        // No trimming happened, so no recap.
        assert!(!result[0].content.contains("Earlier conversation context"));
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
