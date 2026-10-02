//! One immutable capability decision for a concrete provider route and model.
//!
//! Wire schema, reasoning/replay, cache diagnostics and input/output budgeting
//! consume this same decision. Request-specific effort and explicit budgets
//! remain caller overrides; account discovery never mutates the built-in facts.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use crate::conversation::memory::{ContextWindowAuthority, ResolvedContextWindow};
use crate::provider_catalog::find_endpoint_model_preset;
use crate::provider_registry::{
    canonical_provider_key, provider_adapter_for_type, provider_type_for_parts, ProviderAdapterKind,
};

use super::prompt_cache::{resolve_prompt_cache_profile, PromptCacheApiStyle, PromptCacheProfile};
use super::reasoning_profile::{resolve_reasoning_profile, ReasoningApiStyle, ReasoningProfile};
use super::ProviderType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSchemaDialect {
    JsonSchema,
    Moonshot,
}

#[derive(Debug)]
pub struct ResolvedModelContract {
    pub provider_type: ProviderType,
    pub reasoning: ReasoningProfile,
    pub cache: PromptCacheProfile,
    pub tool_schema: ToolSchemaDialect,
    pub catalog_authoritative: bool,
    pub context_tokens: Option<u32>,
    pub max_output_tokens: Option<u32>,
}

impl ResolvedModelContract {
    pub fn catalog_limits(&self) -> Option<crate::model_catalog::ModelLimits> {
        self.catalog_authoritative
            .then(|| crate::model_catalog::ModelLimits {
                context_tokens: self.context_tokens.map(u64::from),
                max_output_tokens: self.max_output_tokens.map(u64::from),
                ..Default::default()
            })
    }

    pub fn context_window(&self, override_tokens: Option<u32>) -> ResolvedContextWindow {
        ResolvedContextWindow {
            capacity_tokens: override_tokens.or(self.context_tokens),
            authority: if override_tokens.is_some() {
                ContextWindowAuthority::UserOverride
            } else if self.context_tokens.is_some() {
                ContextWindowAuthority::Catalog
            } else {
                ContextWindowAuthority::ProviderManaged
            },
        }
    }
}

#[derive(PartialEq, Eq)]
struct ContractKey {
    provider: ProviderType,
    // Kept in memory only. Do not normalize away credentials, query, path or
    // port distinctions before the trust predicates have examined the route.
    endpoint: Option<String>,
    api_style: ReasoningApiStyle,
    model: String,
}

type ContractCache = VecDeque<(ContractKey, Arc<ResolvedModelContract>)>;
const MAX_CACHED_CONTRACTS: usize = 128;

/// Resolve the configured adapter's ordinary API style. Route-selecting wire
/// adapters pass their selected style to `resolve_model_contract` directly.
pub fn resolve_configured_model_contract(
    provider: ProviderType,
    base_url: Option<&str>,
    model: &str,
) -> Arc<ResolvedModelContract> {
    let provider = provider_type_for_parts(canonical_provider_key(provider), base_url);
    let api_style = if super::provider_boundary::is_deepseek_anthropic_endpoint(provider, base_url)
    {
        ReasoningApiStyle::AnthropicMessages
    } else {
        match provider_adapter_for_type(provider) {
            ProviderAdapterKind::OpenAiCompatible => ReasoningApiStyle::OpenAiChatCompletions,
            ProviderAdapterKind::Anthropic => ReasoningApiStyle::AnthropicMessages,
            ProviderAdapterKind::Google => ReasoningApiStyle::GeminiGenerateContent,
            ProviderAdapterKind::Ollama => ReasoningApiStyle::Local,
        }
    };
    resolve_model_contract(provider, base_url, api_style, model)
}

pub fn resolve_model_contract(
    provider: ProviderType,
    base_url: Option<&str>,
    api_style: ReasoningApiStyle,
    model: &str,
) -> Arc<ResolvedModelContract> {
    let provider = provider_type_for_parts(canonical_provider_key(provider), base_url);
    let key = ContractKey {
        provider,
        endpoint: base_url.map(str::to_owned),
        api_style,
        model: model.to_string(),
    };
    static CACHE: OnceLock<Mutex<ContractCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    {
        let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(index) = cache.iter().position(|(candidate, _)| candidate == &key) {
            let entry = cache.remove(index).expect("position came from the cache");
            let value = Arc::clone(&entry.1);
            cache.push_back(entry);
            return value;
        }
    }

    // Static computation only; the shared cache lock never surrounds I/O or
    // catalog parsing. Multiple simultaneous misses may compute equivalent
    // facts, and the second short lock publishes exactly one shared value.
    let reasoning = resolve_reasoning_profile(provider, base_url, api_style, model);
    let cache_style = match api_style {
        ReasoningApiStyle::OpenAiChatCompletions | ReasoningApiStyle::OpenAiResponses => {
            PromptCacheApiStyle::OpenAiCompatible
        }
        ReasoningApiStyle::AnthropicMessages => PromptCacheApiStyle::AnthropicMessages,
        ReasoningApiStyle::GeminiGenerateContent => PromptCacheApiStyle::Gemini,
        ReasoningApiStyle::Local => PromptCacheApiStyle::Local,
    };
    let preset = find_endpoint_model_preset(canonical_provider_key(provider), base_url, model);
    let value = Arc::new(ResolvedModelContract {
        provider_type: provider,
        reasoning,
        cache: resolve_prompt_cache_profile(provider, base_url, cache_style, model),
        tool_schema: if api_style == ReasoningApiStyle::OpenAiChatCompletions
            && super::moonshot_schema::uses_moonshot_schema(provider, base_url, model)
        {
            ToolSchemaDialect::Moonshot
        } else {
            ToolSchemaDialect::JsonSchema
        },
        catalog_authoritative: preset.is_some(),
        context_tokens: preset
            .and_then(|model| model.context_tokens)
            .and_then(|n| n.try_into().ok()),
        max_output_tokens: preset
            .and_then(|model| model.max_output_tokens)
            .and_then(|n| n.try_into().ok()),
    });
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((_, existing)) = cache.iter().find(|(candidate, _)| candidate == &key) {
        return Arc::clone(existing);
    }
    if cache.len() == MAX_CACHED_CONTRACTS {
        cache.pop_front();
    }
    cache.push_back((key, Arc::clone(&value)));
    value
}

#[cfg(test)]
mod tests {
    use super::super::reasoning_profile::ThinkingModeControl;
    use super::*;

    #[test]
    fn public_alias_schema_reasoning_replay_and_limits_use_one_identity() {
        let url = Some("https://api.moonshot.cn/v1");
        let style = ReasoningApiStyle::OpenAiChatCompletions;
        let custom = resolve_model_contract(ProviderType::Custom, url, style, "kimi-k3");
        let direct = resolve_model_contract(ProviderType::Moonshot, url, style, "kimi-k3");
        assert!(Arc::ptr_eq(&custom, &direct));
        assert_eq!(custom.tool_schema, ToolSchemaDialect::Moonshot);
        assert_eq!(custom.reasoning.mode_control, ThinkingModeControl::AlwaysOn);
        assert!(custom.reasoning.preserve_reasoning_history);
        assert!(custom.catalog_authoritative);
        assert!(custom.context_tokens.is_some());
        assert_eq!(
            custom.context_window(Some(12345)).capacity_tokens,
            Some(12345)
        );
    }

    #[test]
    fn unknown_routes_do_not_inherit_capabilities_from_model_names() {
        for endpoint in [
            "https://private.example/v1",
            "http://api.moonshot.cn/v1",
            "https://api.moonshot.cn:8443/v1",
            "https://api.moonshot.cn/v1?tenant=other",
        ] {
            let value = resolve_model_contract(
                ProviderType::Custom,
                Some(endpoint),
                ReasoningApiStyle::OpenAiChatCompletions,
                "kimi-k3",
            );
            assert_eq!(value.provider_type, ProviderType::Custom);
            assert_eq!(value.tool_schema, ToolSchemaDialect::JsonSchema);
            assert_eq!(
                value.reasoning.mode_control,
                ThinkingModeControl::Unsupported
            );
            assert!(!value.catalog_authoritative);
            assert_eq!(value.context_tokens, None);
            assert_eq!(value.max_output_tokens, None);
        }
    }

    #[test]
    fn api_style_and_changed_route_invalidate_the_contract() {
        let direct = resolve_model_contract(
            ProviderType::Moonshot,
            Some("https://api.moonshot.cn/v1"),
            ReasoningApiStyle::OpenAiChatCompletions,
            "kimi-k3",
        );
        let responses = resolve_model_contract(
            ProviderType::Moonshot,
            Some("https://api.moonshot.cn/v1"),
            ReasoningApiStyle::OpenAiResponses,
            "kimi-k3",
        );
        let private = resolve_model_contract(
            ProviderType::Moonshot,
            Some("https://private.example/v1"),
            ReasoningApiStyle::OpenAiChatCompletions,
            "kimi-k3",
        );
        assert!(!Arc::ptr_eq(&direct, &responses));
        assert_eq!(responses.tool_schema, ToolSchemaDialect::JsonSchema);
        assert!(!private.catalog_authoritative);
        assert_eq!(private.context_tokens, None);
    }
}
