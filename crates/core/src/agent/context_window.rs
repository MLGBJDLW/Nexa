//! Context-window policy for the agent loop.

use std::collections::HashSet;

use crate::conversation::memory::{
    context_safety_buffer, resolve_context_window, ResolvedContextWindow,
};
#[cfg(test)]
use crate::conversation::memory::{estimate_message_tokens_for_model, estimate_tokens_for_model};
use crate::error::CoreError;
use crate::llm::{Message, Role};

/// Start compacting before the provider's hard limit is close enough to make
/// one large tool result turn an otherwise healthy run into an overflow retry.
const AUTO_COMPACT_THRESHOLD: u8 = crate::context_policy::DEFAULT_COMPACT_PERCENT;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ContextWindow {
    context_window: Option<u32>,
    max_response_tokens: u32,
    compact_percent: u8,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ContextBudgetDecision {
    pub(crate) usage_pct: f32,
    pub(crate) should_compact: bool,
}

impl ContextWindow {
    #[cfg(test)]
    pub(crate) fn new(
        model: &str,
        context_window_override: Option<u32>,
        max_response_tokens: u32,
    ) -> Self {
        Self::new_with_resolution(model, context_window_override, None, max_response_tokens)
    }

    pub(crate) fn new_with_resolution(
        model: &str,
        context_window_override: Option<u32>,
        endpoint_resolution: Option<ResolvedContextWindow>,
        max_response_tokens: u32,
    ) -> Self {
        Self {
            context_window: endpoint_resolution
                .unwrap_or_else(|| resolve_context_window(model, context_window_override))
                .capacity_tokens,
            max_response_tokens,
            compact_percent: AUTO_COMPACT_THRESHOLD,
        }
    }

    pub(crate) fn context_budget(self) -> Option<u32> {
        self.context_window.map(|context_window| {
            context_window
                .saturating_sub(self.max_response_tokens)
                .saturating_sub(context_safety_buffer(context_window))
        })
    }

    pub(crate) fn with_compact_percent(mut self, percent: Option<u8>) -> Self {
        self.compact_percent = percent.unwrap_or(AUTO_COMPACT_THRESHOLD).clamp(60, 95);
        self
    }

    pub(crate) fn budget_snapshot(self) -> Option<super::context::ContextBudget> {
        let capacity_tokens = self.context_window?;
        let input_budget = self.context_budget()?;
        Some(super::context::ContextBudget {
            capacity_tokens,
            input_budget,
            response_reserve: self.max_response_tokens.min(capacity_tokens),
            safety_reserve: context_safety_buffer(capacity_tokens)
                .min(capacity_tokens.saturating_sub(self.max_response_tokens)),
            compact_threshold: ((u64::from(input_budget) * u64::from(self.compact_percent)) / 100)
                as u32,
            compact_percent: self.compact_percent,
        })
    }

    pub(crate) fn budget_decision(self, prompt_tokens: u32) -> ContextBudgetDecision {
        match self.context_budget() {
            Some(budget) => ContextBudgetDecision {
                usage_pct: if budget == 0 {
                    0.0
                } else {
                    (prompt_tokens as f32 / budget as f32) * 100.0
                },
                should_compact: budget > 0
                    && u64::from(prompt_tokens) * 100
                        > u64::from(budget) * u64::from(self.compact_percent),
            },
            None => ContextBudgetDecision {
                usage_pct: 0.0,
                should_compact: false,
            },
        }
    }

    /// Reject an unsatisfied retention obligation instead of sending a lossy
    /// fallback prompt. Unknown provider capacities still preserve the request.
    pub(crate) fn validate_request(
        self,
        messages: &[Message],
        active_request: Option<&Message>,
        prompt_tokens: u32,
    ) -> Result<(), CoreError> {
        if active_request.is_some_and(|active| {
            !messages
                .iter()
                .any(|message| same_user_request(message, active))
        }) {
            return Err(CoreError::InvalidInput(
                "The active user request could not be retained in the model context. The request was stopped without discarding its evidence.".into(),
            ));
        }
        if self
            .context_budget()
            .is_some_and(|budget| prompt_tokens > budget)
        {
            return Err(CoreError::InvalidInput(
                "The active request and its latest complete tool exchanges cannot fit in the model context after safe compaction. Use a larger context window or reduce the next tool result; the retained evidence was not silently truncated.".into(),
            ));
        }
        Ok(())
    }
}

fn same_user_request(message: &Message, active: &Message) -> bool {
    message.role == Role::User
        && (message.revision() == active.revision() || message.parts == active.parts)
}

pub(super) fn is_context_checkpoint(message: &Message) -> bool {
    message.role == Role::System
        && message
            .text_content()
            .starts_with("## Earlier conversation context")
}

/// One reduction plan shared by summary and archived-history adapters. The
/// plan owns indices, never a second copy of the retained transcript/payloads.
pub(super) struct ContextReductionPlan {
    prefix_end: usize,
    evict_end: usize,
    retained_before_cut: Vec<usize>,
}

impl ContextReductionPlan {
    #[cfg(test)]
    pub(super) fn prepare(
        messages: &[Message],
        model: &str,
        target_tokens: u32,
        active_request: Option<&Message>,
    ) -> Option<Self> {
        let costs = messages
            .iter()
            .map(|message| {
                estimate_message_tokens_for_model(model, message).saturating_add(
                    message
                        .reasoning_content
                        .as_deref()
                        .map_or(0, |reasoning| estimate_tokens_for_model(model, reasoning)),
                )
            })
            .collect::<Vec<_>>();
        Self::prepare_with_costs(messages, target_tokens, active_request, &costs)
    }

    pub(super) fn prepare_with_costs(
        messages: &[Message],
        target_tokens: u32,
        active_request: Option<&Message>,
        costs: &[u32],
    ) -> Option<Self> {
        debug_assert_eq!(messages.len(), costs.len());
        let prefix_end = messages
            .iter()
            .position(|message| message.role != Role::System || is_context_checkpoint(message))?;
        let assistant_starts = messages
            .iter()
            .enumerate()
            .skip(prefix_end)
            .filter_map(|(index, message)| (message.role == Role::Assistant).then_some(index))
            .collect::<Vec<_>>();
        // Always retain the most recent two assistant exchanges. A huge single
        // exchange is an explicit cannot-fit result, never a reason to drop it.
        let last_boundary = assistant_starts
            .iter()
            .rev()
            .nth(1)
            .or_else(|| assistant_starts.first())
            .copied()
            .or_else(|| {
                messages
                    .iter()
                    .rposition(|message| message.role == Role::User)
            })?;
        let mut protected = messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                index < prefix_end
                    || (message.role == Role::System && !is_context_checkpoint(message))
                    || active_request.is_some_and(|active| same_user_request(message, active))
            })
            .collect::<Vec<_>>();
        for (index, _) in messages
            .iter()
            .enumerate()
            .filter(|(_, message)| message.role == Role::User)
            .rev()
            .take(2)
        {
            protected[index] = true;
        }
        let mut suffix = vec![0_u32; messages.len() + 1];
        for index in (0..messages.len()).rev() {
            suffix[index] = suffix[index + 1].saturating_add(costs[index]);
        }
        let mut pending = HashSet::new();
        let mut retained_cost = costs[..prefix_end].iter().copied().sum::<u32>();
        let mut removed_cost = 0_u32;
        let mut selected = None;
        for index in prefix_end..last_boundary {
            let message = &messages[index];
            if let Some(calls) = &message.tool_calls {
                pending.extend(calls.iter().map(|call| call.id.as_str()));
            }
            if message.role == Role::Tool {
                if let Some(call_id) = message.name.as_deref() {
                    pending.remove(call_id);
                }
            }
            if protected[index] {
                retained_cost = retained_cost.saturating_add(costs[index]);
            } else {
                removed_cost = removed_cost.saturating_add(costs[index]);
            }
            let boundary = index + 1;
            if pending.is_empty() && messages[boundary].role != Role::Tool && removed_cost > 0 {
                selected = Some(boundary);
                if retained_cost.saturating_add(suffix[boundary]) <= target_tokens {
                    break;
                }
            }
        }
        let evict_end = selected?;
        Some(Self {
            prefix_end,
            evict_end,
            retained_before_cut: (prefix_end..evict_end)
                .filter(|&index| protected[index])
                .collect(),
        })
    }

    pub(super) fn evicted<'a>(&self, messages: &'a [Message]) -> &'a [Message] {
        &messages[self.prefix_end..self.evict_end]
    }

    pub(super) fn replacement(&self, messages: &[Message], checkpoint: Message) -> Vec<Message> {
        let mut next = Vec::with_capacity(
            self.prefix_end + 1 + self.retained_before_cut.len() + messages.len() - self.evict_end,
        );
        next.extend_from_slice(&messages[..self.prefix_end]);
        next.push(checkpoint);
        next.extend(
            self.retained_before_cut
                .iter()
                .map(|&index| messages[index].clone()),
        );
        next.extend_from_slice(&messages[self.evict_end..]);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::Role;

    #[test]
    fn million_token_models_do_not_compact_at_half_their_window() {
        let config = crate::agent::AgentConfig {
            provider_type: Some(crate::llm::ProviderType::DeepSeek),
            context_window_resolution: Some(
                crate::provider_catalog::resolve_endpoint_model_context_window(
                    "deep_seek",
                    Some("https://api.deepseek.com"),
                    "deepseek-flash",
                    None,
                ),
            ),
            ..Default::default()
        };
        assert_eq!(
            config.context_window_resolution.unwrap().capacity_tokens,
            Some(1_000_000)
        );
        let pipeline = ContextWindow::new_with_resolution(
            "deepseek-flash",
            None,
            config.context_window_resolution,
            config.resolved_max_response_tokens("deepseek-flash"),
        );
        assert!(!pipeline.budget_decision(520_000).should_compact);
        assert!(!pipeline.budget_decision(850_000).should_compact);
        assert!(pipeline.budget_decision(950_000).should_compact);
    }

    #[test]
    fn compact_decision_tracks_budget_usage() {
        let pipeline = ContextWindow::new("test-model", Some(1_000), 100);
        let decision = pipeline.budget_decision(800);
        assert!(pipeline.context_budget().unwrap() < 1_000);
        assert!(decision.usage_pct > 80.0);
        assert!(decision.should_compact);
    }

    #[test]
    fn configured_compaction_triggers_against_input_budget_after_reserves() {
        let pipeline = ContextWindow::new("private", Some(100_000), 20_000);
        let budget = pipeline.context_budget().unwrap();
        assert_eq!(budget, 76_000);
        let early = pipeline.with_compact_percent(Some(65));
        let late = pipeline.with_compact_percent(Some(90));
        assert!(!early.budget_decision(budget * 65 / 100).should_compact);
        assert!(early.budget_decision(budget * 65 / 100 + 1).should_compact);
        assert!(early.budget_decision(60_000).should_compact);
        assert!(!late.budget_decision(60_000).should_compact);
        assert!(late.budget_decision(budget * 90 / 100 + 1).should_compact);
    }

    #[test]
    fn unsatisfied_retention_stops_instead_of_silently_dropping_evidence() {
        let window = ContextWindow::new("test-model", Some(2_000), 100);
        let active = Message::text(Role::User, "Keep this exact requirement");
        let messages = vec![Message::text(Role::System, "policy"), active.clone()];
        assert!(window
            .validate_request(&messages, Some(&active), 3_000)
            .is_err());
        assert!(window
            .validate_request(&messages[..1], Some(&active), 10)
            .is_err());
        assert!(window
            .validate_request(&messages, Some(&active), 100)
            .is_ok());
    }

    #[test]
    fn unknown_provider_managed_window_does_not_compact_or_trim_early() {
        let pipeline = ContextWindow::new("private-router-model", None, 4_096);
        let decision = pipeline.budget_decision(500_000);
        assert_eq!(pipeline.context_budget(), None);
        assert_eq!(decision.usage_pct, 0.0);
        assert!(!decision.should_compact);

        let messages = vec![
            Message::text(Role::System, "system"),
            Message::text(Role::User, "history ".repeat(20_000)),
        ];
        assert!(pipeline
            .validate_request(&messages, Some(&messages[1]), 500_000)
            .is_ok());
    }
}
