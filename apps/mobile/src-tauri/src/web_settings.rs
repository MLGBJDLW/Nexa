use nexa_core::{
    app_settings::WebSearchConfig,
    connection_sync::SharedModelConnection,
    web_search::providers::deepseek::{self, AccountKeys},
};

pub fn use_deepseek_connection(
    config: &mut WebSearchConfig,
    connection: &SharedModelConnection,
) -> Result<(), String> {
    if !deepseek::supports_connection(&connection.provider, connection.base_url.as_deref())
        || connection.api_key.trim().is_empty()
    {
        return Err("Choose a configured official DeepSeek connection.".into());
    }
    deepseek::bind_connection(config, &connection.id);
    Ok(())
}

pub fn native_search_plan(
    account: &SharedModelConnection,
    config: &WebSearchConfig,
) -> Result<nexa_core::llm::native_search::NativeSearchPlan, String> {
    let provider = nexa_core::provider_registry::provider_type_from_key(&account.provider)
        .ok_or("Unsupported provider")?;
    Ok(
        nexa_core::llm::native_search::NativeSearchPlan::resolve_with_engine(
            config.execution_mode,
            provider,
            account.base_url.as_deref(),
            &account.model,
            config.provider_native_engine,
        ),
    )
}

pub struct PhoneAccounts(pub tauri::AppHandle);

#[async_trait::async_trait]
impl AccountKeys for PhoneAccounts {
    async fn deepseek_key(&self, id: &str) -> Option<String> {
        let account = crate::connections::get(&self.0, Some(id)).await.ok()?;
        deepseek::supports_connection(&account.provider, account.base_url.as_deref())
            .then_some(account.api_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_core::{app_settings::WebSearchProviderMode, llm::native_search::SearchExecutionMode};
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
        assert!(config.custom_providers[0].api_key.is_empty());
        assert_eq!(
            config.custom_providers[0].connection_id.as_deref(),
            Some("deepseek-account")
        );
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
        assert!(config.custom_providers[0].api_key.is_empty());
    }
}
