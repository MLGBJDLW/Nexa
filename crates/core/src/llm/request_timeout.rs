//! Implicit model patience is shared by HTTP adapters and unmanaged providers.
//! Explicit timeouts, including zero, always take precedence.
use super::{configured_request_timeout, ProviderConfig, ProviderType};
use std::time::Duration;

pub(crate) fn model_default(provider: Option<ProviderType>, model: &str) -> Duration {
    if matches!(
        provider,
        Some(ProviderType::Ollama | ProviderType::LmStudio)
    ) {
        return Duration::from_secs(300);
    }
    let normalized = model.trim().to_ascii_lowercase();
    let limits = provider
        .and_then(|provider| crate::provider_catalog::model_limits_from_catalog(provider, model));
    let reasoning = provider
        .and_then(|provider| {
            crate::provider_catalog::model_capabilities_from_catalog(provider, model)
        })
        .and_then(|capabilities| capabilities.reasoning);
    let long_reasoner = limits.as_ref().is_some_and(|limits| {
        limits
            .context_tokens
            .is_some_and(|tokens| tokens >= 500_000)
            || limits
                .max_output_tokens
                .is_some_and(|tokens| tokens >= 65_536)
    }) || reasoning.as_ref().is_some_and(|reasoning| {
        matches!(reasoning.default_effort.as_deref(), Some("xhigh" | "max"))
            || reasoning
                .thinking_budget
                .as_ref()
                .and_then(|budget| budget.max_tokens)
                .is_some_and(|tokens| tokens >= 65_536)
    }) || normalized.contains("kimi-k3")
        || normalized.contains("qwen3.8-max")
        || normalized.contains("qwen3.8_max");
    Duration::from_secs(if long_reasoner { 900 } else { 300 })
}

pub(crate) fn for_model(
    config: &ProviderConfig,
    model: &str,
    adapter_default: u64,
) -> Option<Duration> {
    configured_request_timeout(config.timeout_secs.unwrap_or_else(|| {
        adapter_default.max(model_default(Some(config.provider_type), model).as_secs())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_model_patience_preserves_explicit_and_adapter_defaults() {
        for (provider_type, model, baseline, expected) in [
            (ProviderType::Google, "gemini-4-argon", 300, 900),
            (ProviderType::OpenAi, "ordinary-model", 600, 600),
            (ProviderType::DeepSeek, "deepseek-flash", 600, 900),
            (ProviderType::Ollama, "qwen3.8-max", 300, 300),
        ] {
            let mut config = ProviderConfig {
                provider_type,
                base_url: None,
                api_key: None,
                org_id: None,
                timeout_secs: None,
                streaming: Default::default(),
            };
            assert_eq!(
                for_model(&config, model, baseline),
                Some(Duration::from_secs(expected))
            );
            for explicit in [0, 30, 1200] {
                config.timeout_secs = Some(explicit);
                assert_eq!(
                    for_model(&config, model, baseline),
                    configured_request_timeout(explicit)
                );
            }
        }
    }
}
