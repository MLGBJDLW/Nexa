//! Resolve references to saved model accounts only on execution copies.
use std::sync::OnceLock;

use crate::{
    app_settings::ImageGenerationConfig, conversation::AgentConfig, db::Database,
    embed::EmbedderConfig, error::CoreError, provider_credentials::feature_endpoint,
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoutePreset {
    id: String,
    provider: String,
    api_style: String,
    base_url: String,
}

fn presets(feature: &str) -> &'static [RoutePreset] {
    static IMAGE: OnceLock<Vec<RoutePreset>> = OnceLock::new();
    static STT: OnceLock<Vec<RoutePreset>> = OnceLock::new();
    static TTS: OnceLock<Vec<RoutePreset>> = OnceLock::new();
    let (cache, json) = match feature {
        "image" => (
            &IMAGE,
            include_str!("../../../shared/image-provider-presets.json"),
        ),
        "stt" => (
            &STT,
            include_str!("../../../shared/stt-provider-presets.json"),
        ),
        "tts" => (
            &TTS,
            include_str!("../../../shared/tts-provider-presets.json"),
        ),
        _ => return &[],
    };
    cache.get_or_init(|| serde_json::from_str(json).expect("bundled capability route presets"))
}

pub(crate) fn matches_route(
    source: &AgentConfig,
    feature: &str,
    preset_id: &str,
    preset_endpoint: &str,
    target: Option<&str>,
) -> bool {
    let Some(target) = target.and_then(|url| url::Url::parse(url.trim()).ok()) else {
        return false;
    };
    feature_endpoint(
        &source.provider,
        source.base_url.as_deref(),
        feature,
        preset_id,
        preset_endpoint,
    )
    .and_then(|value| url::Url::parse(&value).ok())
    .is_some_and(|expected| {
        expected.as_str().trim_end_matches('/') == target.as_str().trim_end_matches('/')
    })
}

pub(crate) fn matches_media(
    source: &AgentConfig,
    feature: &str,
    provider: &str,
    style: &str,
    target: Option<&str>,
) -> bool {
    presets(feature).iter().any(|preset| {
        preset.provider == provider
            && preset.api_style == style
            && matches_route(source, feature, &preset.id, &preset.base_url, target)
    })
}

pub(crate) fn matches_embedding(source: &AgentConfig, config: &EmbedderConfig) -> bool {
    config.provider == "api"
        && crate::embedding_provider_catalog::find_embedding_provider(&config.api_base_url)
            .is_some_and(|preset| {
                matches_route(
                    source,
                    "embedding",
                    &preset.id,
                    &preset.base_url,
                    Some(&config.api_base_url),
                )
            })
}

pub(crate) fn matches_system_one(
    source: &AgentConfig,
    config: &crate::system_one::SystemOneConfig,
) -> bool {
    crate::system_one::base_endpoint(config).is_ok()
        && crate::system_one::presets()
            .iter()
            .find(|p| p.id == config.provider)
            .is_some_and(|preset| {
                matches_route(
                    source,
                    "systemOne",
                    &preset.id,
                    &preset.base_url,
                    config.base_url.as_deref(),
                )
            })
}

impl Database {
    pub(crate) fn bound_capability_key(
        &self,
        id: &str,
        matches: impl FnOnce(&AgentConfig) -> bool,
    ) -> Result<String, CoreError> {
        match self.get_agent_config(id) {
            Ok(source) if !source.api_key.trim().is_empty() && matches(&source) => {
                Ok(source.api_key)
            }
            Ok(_) | Err(CoreError::NotFound(_)) => Ok(String::new()),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn bound_media_key(
        &self,
        id: &str,
        feature: &str,
        provider: &str,
        style: &str,
        target: Option<&str>,
    ) -> Result<String, CoreError> {
        self.bound_capability_key(id, |source| {
            matches_media(source, feature, provider, style, target)
        })
    }

    pub fn resolve_image_credential(
        &self,
        config: &mut ImageGenerationConfig,
    ) -> Result<(), CoreError> {
        if let Some(id) = &config.connection_id {
            config.api_key = self.bound_media_key(
                id,
                "image",
                &config.provider,
                &config.api_style,
                config.base_url.as_deref(),
            )?;
        }
        Ok(())
    }

    pub fn resolve_embedder_credential(
        &self,
        config: &mut EmbedderConfig,
    ) -> Result<(), CoreError> {
        let Some(id) = &config.connection_id else {
            return Ok(());
        };
        config.api_key.clear();
        if config.provider == "api" {
            config.api_key =
                self.bound_capability_key(id, |source| matches_embedding(source, config))?;
            if config.api_key.is_empty() {
                return Err(CoreError::InvalidInput("The selected embedding connection is unavailable or has changed endpoint. Select a connection in Settings.".into()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_settings::AppConfig;

    #[test]
    fn linked_capabilities_follow_one_account_across_rotation_and_revocation() {
        let db = Database::open_memory().unwrap();
        let endpoint = "https://api.openai.com/v1";
        for (id, key, default) in [("other", "other-key", true), ("selected", "first", false)] {
            db.conn().execute("INSERT INTO agent_configs(id,name,provider,api_key,base_url,model,is_default) VALUES(?1,?1,'open_ai',?2,?3,'fixture',?4)", rusqlite::params![id,key,endpoint,default]).unwrap();
        }
        let id = Some("selected".to_owned());
        let mut app = AppConfig::default();
        app.image_generation.connection_id = id.clone();
        app.image_generation.provider = "open_ai".into();
        app.image_generation.api_style = "openai_images".into();
        app.image_generation.base_url = Some(endpoint.into());
        app.text_to_speech.connection_id = id.clone();
        app.text_to_speech.provider = "open_ai".into();
        app.text_to_speech.api_style = "openai_speech".into();
        app.text_to_speech.base_url = Some(endpoint.into());
        app.speech_to_text.connection_id = id.clone();
        app.speech_to_text.provider = "open_ai".into();
        app.speech_to_text.api_style = "openai_transcription".into();
        app.speech_to_text.base_url = Some(endpoint.into());
        app.system_one.connection_id = id.clone();
        app.system_one.enabled = true;
        app.system_one.provider = "openai".into();
        app.system_one.base_url = Some(endpoint.into());
        let mut embedding = EmbedderConfig {
            provider: "api".into(),
            connection_id: id,
            api_key: "old-snapshot".into(),
            ..Default::default()
        };
        app.image_generation.api_key = "old-snapshot".into();
        db.save_app_config(&app).unwrap();
        db.save_embedder_config(&embedding).unwrap();
        assert!(db
            .load_app_config()
            .unwrap()
            .image_generation
            .api_key
            .is_empty());
        assert!(db.get_embedder_config().unwrap().api_key.is_empty());

        let assert_app = |expected: &str| {
            let mut config = db.load_speech_app_config().unwrap();
            db.resolve_image_credential(&mut config.image_generation)
                .unwrap();
            db.resolve_system_one_credential(&mut config.system_one)
                .unwrap();
            for key in [
                &config.speech_to_text.api_key,
                &config.text_to_speech.api_key,
                &config.image_generation.api_key,
                &config.system_one.api_key,
            ] {
                assert_eq!(key, expected);
            }
        };
        assert_app("first");
        assert_eq!(db.runtime_embedder_config().unwrap().api_key, "first");
        db.conn()
            .execute(
                "UPDATE agent_configs SET api_key='rotated' WHERE id='selected'",
                [],
            )
            .unwrap();
        assert_app("rotated");
        assert_eq!(db.runtime_embedder_config().unwrap().api_key, "rotated");
        db.conn().execute("UPDATE agent_configs SET base_url='https://api.example.org/v1' WHERE id='selected'", []).unwrap();
        assert_app("");
        assert!(db.resolve_embedder_credential(&mut embedding).is_err());
        assert!(embedding.api_key.is_empty());
        db.conn()
            .execute("DELETE FROM agent_configs WHERE id='selected'", [])
            .unwrap();
        assert_app("");
        assert!(db.runtime_embedder_config().is_err());
    }
}
