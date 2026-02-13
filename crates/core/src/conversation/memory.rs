//! Context window management for conversation history.

use crate::llm::{Message, Role};

/// Approximate token count: ~4 chars per token for English.
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as f64 / 4.0).ceil() as u32
}

/// Known model context windows, mapped by exact API model ID.
/// Falls back to prefix/substring matching for unknown variants.
///
/// IMPORTANT: Only verified model IDs are listed here. For unlisted models,
/// users can set context_window in AgentConfig. Update this list by checking
/// provider API documentation.
pub fn model_context_window(model: &str) -> u32 {
    let m = model.to_lowercase();
    let m = m.as_str();

    // ── Exact matches for verified model IDs (highest priority) ────
    match m {
        // OpenAI GPT-5 series (400K)
        "gpt-5.2" | "gpt-5.2-codex" | "gpt-5.2-pro" => 400_000,
        "gpt-5.1" | "gpt-5.1-codex" => 400_000,
        "gpt-5" | "gpt-5-mini" | "gpt-5-nano" => 400_000,
        // OpenAI GPT-4.1 series (1,047,576)
        "gpt-4.1" | "gpt-4.1-2025-04-14" => 1_047_576,
        "gpt-4.1-mini" | "gpt-4.1-mini-2025-04-14" => 1_047_576,
        "gpt-4.1-nano" | "gpt-4.1-nano-2025-04-14" => 1_047_576,
        // OpenAI GPT-4.5 (128K)
        "gpt-4.5-preview" | "gpt-4.5-preview-2025-02-27" => 128_000,
        // OpenAI GPT-4o (128K)
        "gpt-4o" | "gpt-4o-mini" | "gpt-4o-audio-preview" | "chatgpt-4o-latest" => 128_000,
        "gpt-4o-2024-11-20" | "gpt-4o-2024-08-06" | "gpt-4o-2024-05-13" => 128_000,
        "gpt-4o-mini-2024-07-18" => 128_000,
        // OpenAI Legacy
        "gpt-4-turbo" | "gpt-4-turbo-2024-04-09" => 128_000,
        "gpt-4-0613" => 8_192,
        "gpt-3.5-turbo" | "gpt-3.5-turbo-0125" => 16_385,
        // OpenAI o-series reasoning (200K)
        "o4-mini" | "o4-mini-2025-04-16" => 200_000,
        "o3" | "o3-2025-04-16" => 200_000,
        "o3-mini" | "o3-mini-2025-01-31" => 200_000,
        "o3-pro" | "o3-pro-2025-06-10" => 200_000,
        "o1" | "o1-2024-12-17" => 200_000,
        "o1-mini" | "o1-mini-2024-09-12" => 128_000,
        "o1-pro" | "o1-pro-2025-03-19" => 200_000,
        // OpenAI Codex
        "codex-mini-latest" => 200_000,

        // Anthropic Claude 4.x (200K)
        "claude-opus-4-6" => 200_000,
        "claude-sonnet-4-5" | "claude-sonnet-4-5-20250929" => 200_000,
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => 200_000,
        "claude-opus-4-20250514" => 200_000,
        "claude-sonnet-4-20250514" => 200_000,
        // Anthropic Claude 3.x
        "claude-3-7-sonnet-20250219" | "claude-3-7-sonnet-latest" => 200_000,
        "claude-3-5-sonnet-20241022" | "claude-3-5-sonnet-latest" => 200_000,
        "claude-3-5-haiku-20241022" | "claude-3-5-haiku-latest" => 200_000,
        "claude-3-opus-20240229" | "claude-3-opus-latest" => 200_000,
        "claude-3-sonnet-20240229" => 200_000,
        "claude-3-haiku-20240307" => 200_000,
        // Anthropic Claude 2.x
        "claude-2.1" => 200_000,
        "claude-2.0" => 100_000,

        // Google Gemini 3.x (preview)
        "gemini-3-pro-preview" => 1_048_576,
        "gemini-3-flash" => 1_048_576,
        // Google Gemini 2.5
        "gemini-2.5-pro" | "gemini-2.5-pro-preview-05-06" => 1_048_576,
        "gemini-2.5-flash" | "gemini-2.5-flash-preview-04-17" => 1_048_576,
        "gemini-2.5-flash-lite" => 1_048_576,
        // Google Gemini 2.0
        "gemini-2.0-flash" | "gemini-2.0-flash-lite" => 1_048_576,
        // Google Gemini 1.5
        "gemini-1.5-pro" | "gemini-1.5-pro-latest" => 2_097_152,
        "gemini-1.5-flash" | "gemini-1.5-flash-latest" => 1_048_576,

        // DeepSeek
        "deepseek-chat" | "deepseek-reasoner" => 128_000,

        // xAI Grok
        "grok-4" | "grok-4-0709" => 256_000,
        "grok-4-1-fast" | "grok-4-1-fast-reasoning" => 2_000_000,
        "grok-3" | "grok-3-latest" | "grok-3-mini" => 131_072,
        "grok-2" | "grok-2-latest" => 131_072,

        // Mistral
        "mistral-large-2512" => 256_000,
        "magistral-medium-2509" | "mistral-small-2506" => 128_000,
        "codestral-2508" | "devstral-2-2512" => 256_000,

        _ => return prefix_model_context_window(m),
    }
}

/// Fallback matching for model variants not in the exact list.
fn prefix_model_context_window(m: &str) -> u32 {
    match m {
        // OpenAI
        _ if m.starts_with("gpt-5") => 400_000,
        _ if m.starts_with("gpt-4.1") => 1_047_576,
        _ if m.starts_with("gpt-4.5") => 128_000,
        _ if m.starts_with("gpt-4o") => 128_000,
        _ if m.starts_with("gpt-4") => 128_000,
        _ if m.starts_with("gpt-3.5") => 16_385,
        _ if m.starts_with("o1")
            || m.starts_with("o3")
            || m.starts_with("o4") =>
        {
            200_000
        }
        _ if m.starts_with("codex") => 200_000,

        // Anthropic
        _ if m.contains("claude") => 200_000,

        // Google
        _ if m.contains("gemini") => 1_048_576,

        // DeepSeek
        _ if m.contains("deepseek") => 128_000,

        // xAI Grok
        _ if m.starts_with("grok-4") => 256_000,
        _ if m.contains("grok") => 131_072,

        // Mistral
        _ if m.contains("codestral") => 256_000,
        _ if m.contains("devstral") => 256_000,
        _ if m.contains("magistral") => 128_000,
        _ if m.contains("mistral") || m.contains("mixtral") => 128_000,

        // Meta Llama
        _ if m.contains("llama") => 128_000,

        // Qwen
        _ if m.contains("qwen") => 128_000,

        // Others
        _ if m.contains("phi") => 128_000,
        _ if m.contains("command") => 128_000,
        _ if m.contains("yi") => 128_000,
        _ if m.contains("starcoder") => 16_384,

        // Default for completely unknown models
        _ => 32_000,
    }
}

/// Trim conversation history to fit within context window.
///
/// Keeps the system prompt (first message if role == System) plus the
/// newest messages that fit within `max_tokens - reserved_for_response`.
pub fn trim_to_context_window(
    messages: &[Message],
    max_tokens: u32,
    reserved_for_response: u32,
) -> Vec<Message> {
    if messages.is_empty() {
        return Vec::new();
    }

    let budget = max_tokens.saturating_sub(reserved_for_response);
    let mut result: Vec<Message> = Vec::new();
    let mut used: u32 = 0;

    // Separate system message (if present) from the rest.
    let (system_msgs, conversation): (Vec<&Message>, Vec<&Message>) =
        messages.iter().partition(|m| m.role == Role::System);

    // Always include system messages first — they are non-negotiable.
    for msg in &system_msgs {
        let cost = estimate_tokens(&msg.content);
        used = used.saturating_add(cost);
        result.push((*msg).clone());
    }

    // If system messages alone exceed budget, return just them.
    if used >= budget {
        return result;
    }

    // Walk conversation from newest to oldest, accumulating until budget.
    let remaining_budget = budget.saturating_sub(used);
    let mut tail: Vec<Message> = Vec::new();
    let mut tail_tokens: u32 = 0;

    for msg in conversation.iter().rev() {
        let cost = estimate_tokens(&msg.content);
        if tail_tokens.saturating_add(cost) > remaining_budget {
            break;
        }
        tail_tokens = tail_tokens.saturating_add(cost);
        tail.push((*msg).clone());
    }

    // Reverse so oldest-kept message comes first.
    tail.reverse();
    result.extend(tail);

    result
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
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1); // 4 chars = 1 token
        assert_eq!(estimate_tokens("abcde"), 2); // ceil(5/4) = 2
    }

    #[test]
    fn test_model_context_window_exact_match() {
        // OpenAI GPT-5
        assert_eq!(model_context_window("gpt-5.2"), 400_000);
        assert_eq!(model_context_window("gpt-5"), 400_000);
        // OpenAI GPT-4.1
        assert_eq!(model_context_window("gpt-4.1"), 1_047_576);
        assert_eq!(model_context_window("gpt-4.1-mini"), 1_047_576);
        assert_eq!(model_context_window("gpt-4.1-nano"), 1_047_576);
        // OpenAI GPT-4.5
        assert_eq!(model_context_window("gpt-4.5-preview"), 128_000);
        assert_eq!(model_context_window("gpt-4.5-preview-2025-02-27"), 128_000);
        // OpenAI GPT-4o
        assert_eq!(model_context_window("gpt-4o"), 128_000);
        assert_eq!(model_context_window("chatgpt-4o-latest"), 128_000);
        // OpenAI Legacy
        assert_eq!(model_context_window("gpt-4-turbo"), 128_000);
        assert_eq!(model_context_window("gpt-4-0613"), 8_192);
        assert_eq!(model_context_window("gpt-3.5-turbo"), 16_385);
        // OpenAI o-series
        assert_eq!(model_context_window("o1"), 200_000);
        assert_eq!(model_context_window("o1-mini"), 128_000);
        assert_eq!(model_context_window("o3-mini"), 200_000);
        assert_eq!(model_context_window("o3-pro"), 200_000);
        assert_eq!(model_context_window("o4-mini"), 200_000);
        assert_eq!(model_context_window("codex-mini-latest"), 200_000);
        // Anthropic
        assert_eq!(model_context_window("claude-opus-4-6"), 200_000);
        assert_eq!(model_context_window("claude-sonnet-4-5"), 200_000);
        assert_eq!(model_context_window("claude-3-7-sonnet-20250219"), 200_000);
        assert_eq!(model_context_window("claude-2.1"), 200_000);
        assert_eq!(model_context_window("claude-2.0"), 100_000);
        // Google Gemini
        assert_eq!(model_context_window("gemini-3-pro-preview"), 1_048_576);
        assert_eq!(model_context_window("gemini-2.0-flash"), 1_048_576);
        assert_eq!(model_context_window("gemini-1.5-pro"), 2_097_152);
        assert_eq!(model_context_window("gemini-1.5-flash"), 1_048_576);
        // DeepSeek
        assert_eq!(model_context_window("deepseek-chat"), 128_000);
        assert_eq!(model_context_window("deepseek-reasoner"), 128_000);
        // xAI Grok
        assert_eq!(model_context_window("grok-4"), 256_000);
        assert_eq!(model_context_window("grok-4-1-fast"), 2_000_000);
        assert_eq!(model_context_window("grok-3"), 131_072);
        assert_eq!(model_context_window("grok-2"), 131_072);
        // Mistral
        assert_eq!(model_context_window("mistral-large-2512"), 256_000);
        assert_eq!(model_context_window("codestral-2508"), 256_000);
    }

    #[test]
    fn test_model_context_window_prefix_fallback() {
        // These hit prefix/substring matching, not exact match
        assert_eq!(model_context_window("gpt-5-future"), 400_000);
        assert_eq!(model_context_window("gpt-4o-something"), 128_000);
        assert_eq!(model_context_window("claude-3-opus"), 200_000);
        assert_eq!(model_context_window("gemini-2.5-future"), 1_048_576);
        assert_eq!(model_context_window("deepseek-something"), 128_000);
        assert_eq!(model_context_window("grok-4-future"), 256_000);
        assert_eq!(model_context_window("grok-3-beta"), 131_072);
        assert_eq!(model_context_window("llama-3-70b"), 128_000);
        assert_eq!(model_context_window("codex-future"), 200_000);
    }

    #[test]
    fn test_model_context_window_case_insensitive() {
        assert_eq!(model_context_window("GPT-5.2"), 400_000);
        assert_eq!(model_context_window("Claude-Opus-4-6"), 200_000);
        assert_eq!(model_context_window("GEMINI-2.5-PRO"), 1_048_576);
    }

    #[test]
    fn test_model_context_window_unknown() {
        assert_eq!(model_context_window("some-custom-model"), 32_000);
    }

    #[test]
    fn test_trim_empty() {
        let result = trim_to_context_window(&[], 4096, 512);
        assert!(result.is_empty());
    }

    #[test]
    fn test_trim_keeps_system_and_newest() {
        let messages = vec![
            msg(Role::System, "You are helpful."),
            msg(Role::User, "Hello"),
            msg(Role::Assistant, "Hi there!"),
            msg(Role::User, "How are you?"),
        ];
        // Budget is tiny: system + only newest should fit
        let result = trim_to_context_window(&messages, 20, 5);
        assert_eq!(result[0].role, Role::System);
        // Last message should be present
        assert_eq!(result.last().unwrap().content, "How are you?");
    }

    #[test]
    fn test_trim_all_fit() {
        let messages = vec![
            msg(Role::System, "Sys"),
            msg(Role::User, "Hi"),
            msg(Role::Assistant, "Hey"),
        ];
        let result = trim_to_context_window(&messages, 100_000, 512);
        assert_eq!(result.len(), 3);
    }
}
