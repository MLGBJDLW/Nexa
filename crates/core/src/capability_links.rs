use super::{
    decrypt_app_config_secrets, write_app_config, AppConfig, ImageGenerationConfig,
    SpeechToTextConfig, TextToSpeechConfig, APP_CONFIG_KEY,
};
use crate::{
    capability_credentials::{matches_embedding, matches_media, matches_system_one},
    conversation::agent_config_on_connection,
    db::Database,
    embed::{write_embedder_config, EmbedderConfig},
    error::CoreError,
    system_one::SystemOneConfig,
    web_search::providers::deepseek,
};
use rusqlite::{OptionalExtension, TransactionBehavior};

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityLinkPatch {
    pub image_generation: Option<ImageGenerationConfig>,
    pub speech_to_text: Option<SpeechToTextConfig>,
    pub text_to_speech: Option<TextToSpeechConfig>,
    pub embedding: Option<EmbedderConfig>,
    pub system_one: Option<SystemOneConfig>,
    #[serde(default)]
    pub deep_seek_search: bool,
    #[serde(default)]
    pub native_search: bool,
}

impl Database {
    /// Revalidate the saved account and commit all selected features together.
    /// Only the chosen slots are replaced. Credentials stay in the saved account.
    pub fn link_provider_capabilities(
        &self,
        id: &str,
        patch: CapabilityLinkPatch,
    ) -> Result<(), CoreError> {
        let mut conn = self.conn();
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source = agent_config_on_connection(&transaction, id)?;
        let invalid = || {
            CoreError::InvalidInput(
                "The connection or feature endpoint has changed. Review the current settings."
                    .into(),
            )
        };
        if source.api_key.trim().is_empty() {
            return Err(invalid());
        }
        let valid = |bound: Option<&str>, key: &str, matches: bool| {
            if bound == Some(id) && key.is_empty() && matches {
                Ok(())
            } else {
                Err(invalid())
            }
        };
        let changes_app = patch.image_generation.is_some()
            || patch.speech_to_text.is_some()
            || patch.text_to_speech.is_some()
            || patch.system_one.is_some()
            || patch.deep_seek_search
            || patch.native_search;
        if changes_app {
            let raw: Option<String> = transaction
                .query_row(
                    "SELECT value FROM app_config WHERE key=?1",
                    [APP_CONFIG_KEY],
                    |row| row.get(0),
                )
                .optional()?;
            let mut app = match raw {
                Some(json) => decrypt_app_config_secrets(serde_json::from_str(&json)?)?,
                None => AppConfig::default(),
            };
            if let Some(config) = patch.image_generation {
                valid(
                    config.connection_id.as_deref(),
                    &config.api_key,
                    matches_media(
                        &source,
                        "image",
                        &config.provider,
                        &config.api_style,
                        config.base_url.as_deref(),
                    ),
                )?;
                app.image_generation = config;
            }
            if let Some(config) = patch.speech_to_text {
                valid(
                    config.connection_id.as_deref(),
                    &config.api_key,
                    matches_media(
                        &source,
                        "stt",
                        &config.provider,
                        &config.api_style,
                        config.base_url.as_deref(),
                    ),
                )?;
                app.speech_to_text = config;
            }
            if let Some(config) = patch.text_to_speech {
                valid(
                    config.connection_id.as_deref(),
                    &config.api_key,
                    matches_media(
                        &source,
                        "tts",
                        &config.provider,
                        &config.api_style,
                        config.base_url.as_deref(),
                    ),
                )?;
                app.text_to_speech = config;
            }
            if let Some(config) = patch.system_one {
                valid(
                    config.connection_id.as_deref(),
                    &config.api_key,
                    matches_system_one(&source, &config),
                )?;
                app.system_one = config;
            }
            if patch.native_search {
                let provider = crate::provider_registry::provider_type_from_key(&source.provider)
                    .ok_or_else(invalid)?;
                let mode = crate::llm::native_search::SearchExecutionMode::Auto;
                let plan = crate::llm::native_search::NativeSearchPlan::resolve_with_engine(
                    mode,
                    provider,
                    source.base_url.as_deref(),
                    &source.model,
                    app.web_search.provider_native_engine,
                );
                if patch.deep_seek_search || !plan.uses_provider_native() {
                    return Err(invalid());
                }
                app.web_search.execution_mode = mode;
            }
            if patch.deep_seek_search {
                if !deepseek::supports_connection(&source.provider, source.base_url.as_deref()) {
                    return Err(invalid());
                }
                deepseek::bind_connection(&mut app.web_search, id);
            }
            write_app_config(&transaction, &app)?;
        }
        if let Some(config) = patch.embedding {
            valid(
                config.connection_id.as_deref(),
                &config.api_key,
                matches_embedding(&source, &config),
            )?;
            write_embedder_config(&transaction, &config)?;
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selected_bindings_are_atomic_and_keep_unrelated_settings() {
        let db = Database::open_memory().unwrap();
        let account = db.save_agent_config(&serde_json::from_value(json!({
            "name":"Second account", "provider":"open_ai", "model":"gpt-5", "apiKey":"fixture", "isDefault":false
        })).unwrap()).unwrap();
        let mut app = db.load_app_config().unwrap();
        app.trace_enabled = true;
        db.save_app_config(&app).unwrap();
        let mut image = app.image_generation.clone();
        image.connection_id = Some(account.id.clone());
        let mut embedding = EmbedderConfig {
            provider: "api".into(),
            connection_id: Some(account.id.clone()),
            ..Default::default()
        };
        let accepted = embedding.clone();
        embedding.api_base_url = "https://example.invalid/v1".into();
        assert!(db
            .link_provider_capabilities(
                &account.id,
                CapabilityLinkPatch {
                    image_generation: Some(image.clone()),
                    embedding: Some(embedding),
                    ..Default::default()
                }
            )
            .is_err());
        assert!(db
            .load_app_config()
            .unwrap()
            .image_generation
            .connection_id
            .is_none());
        db.link_provider_capabilities(
            &account.id,
            CapabilityLinkPatch {
                image_generation: Some(image),
                embedding: Some(accepted),
                ..Default::default()
            },
        )
        .unwrap();
        let saved = db.load_app_config().unwrap();
        assert!(saved.trace_enabled);
        assert_eq!(
            saved.image_generation.connection_id.as_deref(),
            Some(account.id.as_str())
        );
        assert!(saved.image_generation.api_key.is_empty());
        let raw = db.get_embedder_config().unwrap();
        assert_eq!(raw.connection_id.as_deref(), Some(account.id.as_str()));
        assert!(raw.api_key.is_empty());
        assert_eq!(db.runtime_embedder_config().unwrap().api_key, "fixture");
    }
}
