//! Speech inherits keys at the same endpoint boundary as the provider UI.
//! Only exact reviewed endpoints may share keys across API paths or aliases.
use crate::{app_settings::AppConfig, db::Database, error::CoreError};
use std::{collections::HashMap, sync::OnceLock};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scopes {
    endpoints: HashMap<String, String>,
    provider_aliases: HashMap<String, String>,
}

fn scope(provider: &str, endpoint: Option<&str>) -> Option<String> {
    static SCOPES: OnceLock<Scopes> = OnceLock::new();
    let scopes = SCOPES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../shared/provider-credential-scopes.json"
        ))
        .expect("reviewed credential scopes")
    });
    let Some(endpoint) = endpoint.filter(|endpoint| !endpoint.trim().is_empty()) else {
        let provider = provider.trim().to_lowercase();
        return Some(
            scopes
                .provider_aliases
                .get(&provider)
                .cloned()
                .unwrap_or(provider),
        );
    };
    let url = url::Url::parse(endpoint.trim()).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let normalized = url.as_str().trim_end_matches('/');
    Some(
        scopes
            .endpoints
            .get(normalized)
            .cloned()
            .unwrap_or_else(|| format!("endpoint:{normalized}")),
    )
}

impl Database {
    /// Materialize inherited credentials for execution only. Settings load/save
    /// uses the raw config so rotating or deleting a source key takes effect.
    pub fn load_speech_app_config(&self) -> Result<AppConfig, CoreError> {
        let mut config = self.load_app_config()?;
        self.resolve_speech_credentials(&mut config)?;
        Ok(config)
    }

    /// Fill only absent keys; an explicit speech key always wins. This is also
    /// used before mobile validates a newly selected cloud speech provider.
    pub fn resolve_speech_credentials(&self, config: &mut AppConfig) -> Result<(), CoreError> {
        if !config.speech_to_text.api_key.trim().is_empty()
            && !config.text_to_speech.api_key.trim().is_empty()
        {
            return Ok(());
        }
        let mut configs = self.list_agent_configs()?;
        configs.sort_by_key(|source| !source.is_default);
        config.inherit_speech_credentials(configs.iter().map(|source| {
            (
                source.provider.as_str(),
                source.base_url.as_deref(),
                source.api_key.as_str(),
            )
        }));
        Ok(())
    }
}

impl AppConfig {
    /// Host-specific credential stores supply candidates in preference order.
    pub fn inherit_speech_credentials<'a>(
        &mut self,
        connections: impl Iterator<Item = (&'a str, Option<&'a str>, &'a str)> + Clone,
    ) {
        for (provider, endpoint, key) in [
            (
                &self.speech_to_text.provider,
                &self.speech_to_text.base_url,
                &mut self.speech_to_text.api_key,
            ),
            (
                &self.text_to_speech.provider,
                &self.text_to_speech.base_url,
                &mut self.text_to_speech.api_key,
            ),
        ] {
            if !key.trim().is_empty() {
                continue;
            }
            let Some(target) = scope(provider, endpoint.as_deref()) else {
                continue;
            };
            if let Some((_, _, shared_key)) =
                connections.clone().find(|(provider, endpoint, key)| {
                    !key.trim().is_empty() && scope(provider, *endpoint).as_ref() == Some(&target)
                })
            {
                *key = shared_key.to_owned();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_speech_credentials_follow_endpoint_trust_boundaries() {
        let db = Database::open_memory().unwrap();
        // Exercise the actual saved connection path without external credentials.
        db.conn().execute("INSERT INTO agent_configs (id,name,provider,api_key,base_url,model,is_default) VALUES ('voice-test','voice-test','alibaba_model_studio','fixture-key','https://dashscope.aliyuncs.com/compatible-mode/v1','qwen3.8-max',1)", []).unwrap();
        let mut config = AppConfig::default();
        config.speech_to_text.base_url =
            Some("https://dashscope.aliyuncs.com/api-ws/v1/inference".into());
        config.text_to_speech.base_url =
            Some("https://dashscope.aliyuncs.com/api/v1/services/audio/tts".into());
        db.resolve_speech_credentials(&mut config).unwrap();
        assert_eq!(config.speech_to_text.api_key, "fixture-key");
        assert_eq!(config.text_to_speech.api_key, "fixture-key");
        config.speech_to_text.api_key.clear();
        config.text_to_speech.api_key.clear();
        db.save_app_config(&config).unwrap();
        let mut unrelated = db.load_app_config().unwrap();
        assert!(unrelated.speech_to_text.api_key.is_empty());
        unrelated.default_search_limit = 42;
        db.save_app_config(&unrelated).unwrap();
        db.conn()
            .execute(
                "UPDATE agent_configs SET api_key='rotated-key' WHERE id='voice-test'",
                [],
            )
            .unwrap();
        assert_eq!(
            db.load_speech_app_config().unwrap().speech_to_text.api_key,
            "rotated-key"
        );
        db.conn()
            .execute("DELETE FROM agent_configs WHERE id='voice-test'", [])
            .unwrap();
        assert!(db
            .load_speech_app_config()
            .unwrap()
            .speech_to_text
            .api_key
            .is_empty());
        db.conn().execute("INSERT INTO agent_configs (id,name,provider,api_key,base_url,model,is_default) VALUES ('voice-test','voice-test','alibaba_model_studio','fixture-key','https://dashscope.aliyuncs.com/compatible-mode/v1','qwen3.8-max',1)", []).unwrap();
        for endpoint in [
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
            "https://coding.dashscope.aliyuncs.com/v1",
            "https://dashscope.aliyuncs.com/evil",
            "https://dashscope.aliyuncs.com/api-ws/v1?redirect=evil",
        ] {
            config.speech_to_text.api_key.clear();
            config.speech_to_text.base_url = Some(endpoint.into());
            db.resolve_speech_credentials(&mut config).unwrap();
            assert!(config.speech_to_text.api_key.is_empty(), "{endpoint}");
        }
        config.text_to_speech.api_key = "explicit-voice-key".into();
        db.resolve_speech_credentials(&mut config).unwrap();
        assert_eq!(config.text_to_speech.api_key, "explicit-voice-key");
    }
}
