//! Request recovery controls and per-attempt connection policy.
//! Transport adapters alone own stream liveness (including provider-hosted
//! actions). Heartbeats must not race a second semantic-progress watchdog.

use std::time::Duration;

use crate::llm::{ProviderType, ReasoningEffort};
use crate::provider_catalog::model_capabilities_from_catalog;

const LONG_REASONER_CONNECT_DEADLINE: Duration = Duration::from_secs(900);
const DEFAULT_CONNECT_DEADLINE: Duration = Duration::from_secs(300);
const LOCAL_MODEL_CONNECT_DEADLINE: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ModelProgressPolicy {
    pub(super) connect_deadline: Duration,
}

impl ModelProgressPolicy {
    pub(super) fn for_model(provider: Option<ProviderType>, model: &str) -> Self {
        let normalized = model.trim().to_ascii_lowercase();
        let catalog_limits = provider.and_then(|provider| {
            crate::provider_catalog::model_limits_from_catalog(provider, model)
        });
        let catalog_reasoning = provider
            .and_then(|provider| model_capabilities_from_catalog(provider, model))
            .and_then(|capabilities| capabilities.reasoning);
        let capability_long_reasoner = catalog_limits.as_ref().is_some_and(|limits| {
            limits
                .context_tokens
                .is_some_and(|tokens| tokens >= 500_000)
                || limits
                    .max_output_tokens
                    .is_some_and(|tokens| tokens >= 65_536)
        }) || catalog_reasoning.as_ref().is_some_and(|reasoning| {
            matches!(reasoning.default_effort.as_deref(), Some("xhigh" | "max"))
                || reasoning
                    .thinking_budget
                    .as_ref()
                    .and_then(|budget| budget.max_tokens)
                    .is_some_and(|tokens| tokens >= 65_536)
        });
        let long_reasoner = capability_long_reasoner
            || normalized.contains("kimi-k3")
            || normalized.contains("qwen3.8-max")
            || normalized.contains("qwen3.8_max");

        Self {
            connect_deadline: if matches!(
                provider,
                Some(ProviderType::Ollama | ProviderType::LmStudio)
            ) {
                LOCAL_MODEL_CONNECT_DEADLINE
            } else if long_reasoner {
                LONG_REASONER_CONNECT_DEADLINE
            } else {
                DEFAULT_CONNECT_DEADLINE
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelProgressRecoveryControls {
    pub(super) reasoning_enabled: Option<bool>,
    pub(super) reasoning_effort: Option<ReasoningEffort>,
    pub(super) thinking_budget: Option<u32>,
    pub(super) description: &'static str,
}

/// Request-side controls used by explicit user recovery and replay-safety
/// recovery. They are never applied merely because an active stream is slow.
pub(super) fn recovery_controls(
    provider: Option<ProviderType>,
    model: &str,
) -> ModelProgressRecoveryControls {
    let Some(reasoning) = provider
        .and_then(|provider| model_capabilities_from_catalog(provider, model))
        .and_then(|capabilities| capabilities.reasoning)
    else {
        return ModelProgressRecoveryControls {
            reasoning_enabled: Some(false),
            reasoning_effort: None,
            thinking_budget: None,
            description: "reasoning disabled",
        };
    };
    let always_on = reasoning.mode.as_deref() == Some("always");
    if !always_on {
        let disabled_effort = (provider == Some(ProviderType::OpenRouter)
            && reasoning.effort_levels.iter().any(|level| level == "none"))
        .then_some(ReasoningEffort::None);
        return ModelProgressRecoveryControls {
            reasoning_enabled: Some(false),
            reasoning_effort: disabled_effort,
            thinking_budget: reasoning
                .thinking_budget
                .filter(|budget| budget.enabled && budget.allow_zero == Some(true))
                .map(|_| 0),
            description: "reasoning disabled",
        };
    }
    let effort = reasoning
        .effort_levels
        .iter()
        .filter_map(|effort| ReasoningEffort::from_wire(effort))
        .find(|effort| *effort != ReasoningEffort::None);
    ModelProgressRecoveryControls {
        reasoning_enabled: Some(true),
        reasoning_effort: effort.clone(),
        thinking_budget: None,
        description: if matches!(
            effort,
            Some(ReasoningEffort::Minimal | ReasoningEffort::Low)
        ) {
            "reasoning reduced to the lowest supported effort"
        } else {
            "always-on reasoning reduced to the lowest supported effort"
        },
    }
}

/// Keep DeepSeek's same-turn reasoning replay valid when continuing a real
/// output limit. Disabling it and later enabling it again invalidates the
/// native tool history and makes the model repeat already completed work.
pub(super) fn continuation_controls(
    provider: Option<ProviderType>,
    model: &str,
) -> ModelProgressRecoveryControls {
    if provider == Some(ProviderType::DeepSeek)
        && model_capabilities_from_catalog(ProviderType::DeepSeek, model)
            .and_then(|capabilities| capabilities.reasoning)
            .is_some_and(|reasoning| reasoning.effort_levels.iter().any(|effort| effort == "low"))
    {
        return ModelProgressRecoveryControls {
            reasoning_enabled: Some(true),
            reasoning_effort: Some(ReasoningEffort::Low),
            thinking_budget: None,
            description: "reasoning reduced while preserving native continuation state",
        };
    }
    recovery_controls(provider, model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complex_requests_have_time_to_open_without_shortening_local_deadlines() {
        assert_eq!(
            ModelProgressPolicy::for_model(Some(ProviderType::Google), "gemini-4-argon")
                .connect_deadline,
            Duration::from_secs(900)
        );
        assert_eq!(
            ModelProgressPolicy::for_model(Some(ProviderType::OpenAi), "ordinary-model")
                .connect_deadline,
            Duration::from_secs(300)
        );
        assert_eq!(
            ModelProgressPolicy::for_model(Some(ProviderType::Ollama), "qwen3.8-max")
                .connect_deadline,
            Duration::from_secs(300)
        );
    }

    #[test]
    fn recovery_controls_are_request_side_only() {
        let qwen = recovery_controls(Some(ProviderType::Qwen), "qwen3.8-max");
        assert_eq!(qwen.reasoning_enabled, Some(false));
        let kimi = recovery_controls(Some(ProviderType::Moonshot), "kimi-k3");
        assert_eq!(kimi.reasoning_enabled, Some(true));
        assert_eq!(kimi.reasoning_effort, Some(ReasoningEffort::Low));
    }
}
