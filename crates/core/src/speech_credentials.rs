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

/// Return the speech preset's endpoint for a specific saved connection.
pub fn connection_speech_endpoint(
    source_provider: &str,
    source_base_url: Option<&str>,
    preset_provider: &str,
    preset_base_url: &str,
    api_style: &str,
) -> Option<String> {
    let source_scope = scope(source_provider, source_base_url)?;
    let preset_scope = scope(preset_provider, Some(preset_base_url))?;
    if !preset_base_url.is_empty() && source_scope == preset_scope {
        return Some(preset_base_url.to_owned());
    }
    let mut source = url::Url::parse(source_base_url?.trim()).ok()?;
    let host = source.host_str()?;
    // A regional or workspace chat key stays on that exact authority. Only
    // substitute paths from the reviewed Beijing speech catalog.
    if matches!(
        api_style,
        "dashscope_asr" | "dashscope_audio_asr" | "dashscope_speech"
    ) && preset_scope == "alibaba-model-studio:beijing"
        && source.scheme() == "https"
        && source.port().is_none()
        && source.path().trim_end_matches('/') == "/compatible-mode/v1"
    {
        let workspace =
            crate::provider_catalog::alibaba_workspace_preset_id(source.as_str()).is_some();
        source.set_path(url::Url::parse(preset_base_url).ok()?.path());
        if workspace
            || (source_scope.starts_with("alibaba-model-studio:")
                && source_scope == scope(preset_provider, Some(source.as_str()))?)
        {
            return Some(source.into());
        }
        return None;
    }
    // An explicitly custom OpenAI-compatible server may expose audio routes.
    // Keep its full base path. A hosted chat provider alone implies no audio API.
    let gateway_audio = matches!(
        (preset_provider, api_style),
        ("custom", "openai_transcription") | ("open_ai", "openai_speech")
    );
    if gateway_audio
        && matches!(source_provider, "custom" | "open_ai")
        && source_scope.starts_with("endpoint:")
        && !host.ends_with(".maas.aliyuncs.com")
        && !host.ends_with(".dashscope.aliyuncs.com")
        && crate::provider_catalog::provider_for_preset_endpoint(source_base_url).is_none()
        && (source.scheme() == "https"
            || (source.scheme() == "http" && matches!(host, "localhost" | "127.0.0.1" | "[::1]")))
    {
        return Some(source.as_str().trim_end_matches('/').to_owned());
    }
    None
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
        for (provider, endpoint, key, binding) in [
            (
                &self.speech_to_text.provider,
                &self.speech_to_text.base_url,
                &mut self.speech_to_text.api_key,
                &self.speech_to_text.connection_id,
            ),
            (
                &self.text_to_speech.provider,
                &self.text_to_speech.base_url,
                &mut self.text_to_speech.api_key,
                &self.text_to_speech.connection_id,
            ),
        ] {
            if binding.is_some() || !key.trim().is_empty() {
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
    fn speech_routes_keep_the_selected_connection_authority() {
        fn cosy(source: &str) -> Option<String> {
            connection_speech_endpoint(
                "alibaba_model_studio",
                Some(source),
                "qwen",
                "https://dashscope.aliyuncs.com/api-ws/v1/inference",
                "dashscope_speech",
            )
        }
        for host in [
            "dashscope.aliyuncs.com",
            "dashscope-intl.aliyuncs.com",
            "demo.cn-beijing.maas.aliyuncs.com",
            "demo.ap-southeast-1.maas.aliyuncs.com",
        ] {
            let source = format!("https://{host}/compatible-mode/v1/");
            let endpoint = format!("https://{host}/api-ws/v1/inference");
            assert_eq!(cosy(&source), Some(endpoint));
        }
        for source in [
            "https://trial.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
            "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
            "https://coding.dashscope.aliyuncs.com/v1",
            "https://demo.cn-beijing.maas.aliyuncs.com.evil.test/compatible-mode/v1",
            "https://demo.cn-beijing.maas.aliyuncs.com:444/compatible-mode/v1",
            "https://demo.cn-beijing.maas.aliyuncs.com/compatible-mode/v1?token=fixture",
        ] {
            assert_eq!(cosy(source), None);
            assert!(connection_speech_endpoint(
                "qwen",
                Some(source),
                "alibaba_model_studio",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "dashscope_asr"
            )
            .is_none());
        }
        let custom = "https://gateway.example.test/tenant-a/v1";
        assert_eq!(
            connection_speech_endpoint(
                "open_ai",
                Some(custom),
                "open_ai",
                "https://api.openai.com/v1",
                "openai_speech"
            )
            .as_deref(),
            Some(custom)
        );
        assert!(connection_speech_endpoint(
            "custom",
            Some("https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1"),
            "custom",
            "",
            "openai_transcription"
        )
        .is_none());
        assert_eq!(
            connection_speech_endpoint(
                "custom",
                Some(custom),
                "custom",
                "",
                "openai_transcription"
            )
            .as_deref(),
            Some(custom)
        );
        assert_eq!(
            connection_speech_endpoint(
                "custom",
                Some(custom),
                "open_ai",
                "https://api.openai.com/v1",
                "openai_speech"
            )
            .as_deref(),
            Some(custom)
        );
        assert!(connection_speech_endpoint(
            "open_ai",
            Some("https://api.anthropic.com"),
            "open_ai",
            "https://api.openai.com/v1",
            "openai_speech"
        )
        .is_none());
        assert!(
            connection_speech_endpoint("qwen", None, "qwen", "", "dashscope_audio_generation")
                .is_none()
        );
    }
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
