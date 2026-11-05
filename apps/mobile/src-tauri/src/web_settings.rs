use nexa_core::{
    app_settings::{WebSearchConfig, WebSearchCustomProviderPreset, WebSearchProviderMode},
    connection_sync::SharedModelConnection,
    llm::native_search::SearchExecutionMode,
};

/// Explicitly requested credential reuse, restricted to the official account
/// origin. Hosted DeepSeek models do not grant access to DeepSeek's web API.
pub fn use_deepseek_connection(
    config: &mut WebSearchConfig,
    connection: &SharedModelConnection,
) -> Result<(), String> {
    let base = connection.base_url.as_deref().unwrap_or("");
    let url = reqwest::Url::parse(base)
        .map_err(|_| "Choose an official DeepSeek connection.".to_string())?;
    if url.scheme() != "https"
        || url.host_str() != Some("api.deepseek.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || connection.api_key.trim().is_empty()
    {
        return Err("Choose a configured official DeepSeek connection.".into());
    }
    let mut provider = config
        .custom_providers
        .iter()
        .find(|provider| provider.preset == WebSearchCustomProviderPreset::Deepseek)
        .cloned()
        .unwrap_or_else(|| {
            WebSearchConfig::default()
                .custom_providers
                .into_iter()
                .find(|provider| provider.preset == WebSearchCustomProviderPreset::Deepseek)
                .unwrap()
        });
    provider.enabled = true;
    provider.api_key = connection.api_key.clone();
    provider.base_url = Some("https://api.deepseek.com/v1/tools/search".into());
    provider.priority = 0;
    config
        .custom_providers
        .retain(|provider| provider.preset != WebSearchCustomProviderPreset::Deepseek);
    config.custom_providers.insert(0, provider);
    config.provider_mode = WebSearchProviderMode::CustomFirst;
    config.execution_mode = SearchExecutionMode::NexaRouter;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn saved_official_account_enables_search_and_fetch_without_overwriting_other_sources() {
        let mut config = WebSearchConfig::default();
        let mut connection: SharedModelConnection = serde_json::from_value(json!({
            "id":"deepseek-account","name":"DeepSeek","provider":"openai","model":"deepseek-flash",
            "baseUrl":"https://api.deepseek.com/anthropic","apiKey":"fixture-key","isDefault":false
        }))
        .unwrap();
        let count = config.custom_providers.len();
        use_deepseek_connection(&mut config, &connection).unwrap();
        assert_eq!(config.custom_providers.len(), count);
        assert_eq!(config.custom_providers[0].api_key, "fixture-key");
        assert!(config.custom_providers[0].is_configured());
        assert_eq!(config.provider_mode, WebSearchProviderMode::CustomFirst);
        assert_eq!(config.execution_mode, SearchExecutionMode::NexaRouter);
        for base in [
            "https://openrouter.ai/api/v1",
            "http://api.deepseek.com/v1",
            "https://api.deepseek.com.evil.test",
            "https://api.deepseek.com:444/v1",
        ] {
            connection.base_url = Some(base.into());
            assert!(use_deepseek_connection(&mut config, &connection).is_err());
        }
        assert_eq!(config.custom_providers[0].api_key, "fixture-key");
    }
}
