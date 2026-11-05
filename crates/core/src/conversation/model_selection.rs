//! Conversation and turn model choices never edit shared connection credentials.
use super::AgentConfig;
use crate::error::CoreError;
use serde::{Deserialize, Serialize};

pub(crate) fn read_selection(
    row: &rusqlite::Row<'_>,
    column: usize,
) -> rusqlite::Result<Option<TurnModelSelection>> {
    row.get::<_, Option<String>>(column)?
        .map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    column,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

impl From<&AgentConfig> for TurnModelSelection {
    fn from(config: &AgentConfig) -> Self {
        Self {
            model: config.model.clone(),
            reasoning_enabled: config.reasoning_enabled,
            reasoning_effort: config.reasoning_effort.clone(),
            thinking_budget: config.thinking_budget,
        }
    }
}

impl crate::db::Database {
    pub fn select_conversation_model(
        &self,
        id: &str,
        config_id: &str,
        selection: &TurnModelSelection,
    ) -> Result<super::Conversation, CoreError> {
        let config = self.get_agent_config(config_id)?;
        let selected = selection.apply(&config)?;
        self.persist_conversation_model(id, &selected)?;
        self.get_conversation(id)
    }

    /// The resolved route is captured before a run; no secret is stored here.
    pub fn persist_conversation_model(
        &self,
        id: &str,
        config: &AgentConfig,
    ) -> Result<(), CoreError> {
        let json = serde_json::to_string(&TurnModelSelection::from(config))?;
        let affected = self.conn().execute(
            "UPDATE conversations SET provider=?2, model=?3, agent_config_id=?4, model_selection_json=?5, updated_at=datetime('now') WHERE id=?1",
            rusqlite::params![id, config.provider, config.model, config.id, json])?;
        if affected == 0 {
            return Err(CoreError::NotFound(format!("Conversation {id}")));
        }
        Ok(())
    }

    pub fn resolve_conversation_agent_config(
        &self,
        conversation: &super::Conversation,
        requested_id: Option<&str>,
    ) -> Result<AgentConfig, CoreError> {
        let id = requested_id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .or(conversation.agent_config_id.as_deref());
        let configs = self.list_agent_configs()?;
        let config = if let Some(id) = id {
            configs.iter().find(|config| config.id == id).ok_or_else(|| CoreError::InvalidInput(
                "This conversation's model connection was removed. Select an available connection before continuing.".into()))?
        } else {
            // Legacy chats have no connection identity. Match their provider;
            // never silently switch providers because another window did.
            configs
                .iter()
                .find(|config| {
                    config.provider == conversation.provider
                        && config.model == conversation.model
                        && config.is_default
                })
                .or_else(|| {
                    configs.iter().find(|config| {
                        config.provider == conversation.provider
                            && config.model == conversation.model
                    })
                })
                .or_else(|| {
                    configs.iter().find(|config| {
                        config.provider == conversation.provider && config.is_default
                    })
                })
                .or_else(|| {
                    configs
                        .iter()
                        .find(|config| config.provider == conversation.provider)
                })
                .ok_or_else(|| {
                    CoreError::InvalidInput(
                        "Select a model connection for this conversation.".into(),
                    )
                })?
        };
        if conversation
            .agent_config_id
            .as_deref()
            .is_some_and(|saved| saved == config.id)
        {
            if config.provider != conversation.provider {
                return Err(CoreError::InvalidInput(
                    "The connection provider changed. Select a connection again before continuing."
                        .into(),
                ));
            }
            if let Some(selection) = &conversation.model_selection {
                return selection.apply(config);
            }
        }
        if requested_id.is_none() || config.provider == conversation.provider {
            let mut selection = TurnModelSelection::from(config);
            selection.model = conversation.model.clone();
            return selection.apply(config);
        }
        Ok(config.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TurnModelSelection {
    pub model: String,
    pub reasoning_enabled: Option<bool>,
    pub reasoning_effort: Option<String>,
    pub thinking_budget: Option<i64>,
}

impl TurnModelSelection {
    pub fn apply(&self, saved: &AgentConfig) -> Result<AgentConfig, CoreError> {
        self.validate_for_provider(&saved.provider)?;
        let mut selected = saved.clone();
        selected.model = self.model.trim().into();
        selected.model_id = Some(selected.model.clone());
        selected.model_selection_resolution = None;
        selected.reasoning_enabled = Some(true);
        selected.reasoning_effort = self
            .reasoning_effort
            .clone()
            .filter(|effort| effort != "none");
        selected.thinking_budget = self.thinking_budget.filter(|budget| *budget > 0);
        Ok(selected)
    }

    pub fn apply_to_shared(
        &self,
        saved: &mut crate::connection_sync::SharedModelConnection,
    ) -> Result<(), CoreError> {
        self.validate_for_provider(&saved.provider)?;
        saved.model = self.model.trim().into();
        saved.reasoning_enabled = Some(true);
        saved.reasoning_effort = self
            .reasoning_effort
            .clone()
            .filter(|effort| effort != "none");
        saved.thinking_budget = self.thinking_budget.filter(|budget| *budget > 0);
        Ok(())
    }

    fn validate_for_provider(&self, provider: &str) -> Result<(), CoreError> {
        let model = self.model.trim();
        if model.is_empty() || model.len() > 512 || model.chars().any(char::is_control) {
            return Err(CoreError::InvalidInput("Choose a valid model ID".into()));
        }
        if self.reasoning_effort.as_deref().is_some_and(|effort| {
            if crate::external_agent::preset(provider).is_some() {
                // ACP option IDs are opaque. The connected session validates
                // membership after selecting the actual model, before prompting.
                effort.is_empty() || effort.len() > 512 || effort.chars().any(char::is_control)
            } else {
                !matches!(
                    effort,
                    "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
                )
            }
        }) || self
            .thinking_budget
            .is_some_and(|budget| !(0..=i64::from(u32::MAX)).contains(&budget))
        {
            return Err(CoreError::InvalidInput(
                "Invalid model reasoning options".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversation_choice_survives_connection_edits_and_default_changes() {
        let db = crate::db::Database::open_memory().unwrap();
        let save = |id: &str, model: &str, default: bool| -> AgentConfig {
            let input = serde_json::from_value(serde_json::json!({
                "id":id,"name":id,"provider":"open_ai","apiKey":"test-key","model":model,"isDefault":default
            })).unwrap();
            db.save_agent_config(&input).unwrap()
        };
        let config = save("connection-a", "model-a", true);
        let conversation = db
            .create_conversation(&super::super::CreateConversationInput {
                provider: config.provider.clone(),
                model: config.model.clone(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        let selection = TurnModelSelection {
            model: "chat-model".into(),
            reasoning_enabled: Some(true),
            reasoning_effort: Some("high".into()),
            thinking_budget: None,
        };
        db.select_conversation_model(&conversation.id, &config.id, &selection)
            .unwrap();
        save("connection-a", "another-window-model", false);
        save("connection-b", "default-model", true);
        let reloaded = db.get_conversation(&conversation.id).unwrap();
        assert_eq!(reloaded.model_selection, Some(selection));
        for requested in [None, Some("connection-a")] {
            let resolved = db
                .resolve_conversation_agent_config(&reloaded, requested)
                .unwrap();
            assert_eq!(resolved.model, "chat-model");
            assert_eq!(resolved.reasoning_effort.as_deref(), Some("high"));
            assert_eq!(resolved.api_key, "test-key");
        }
        assert_eq!(
            db.get_agent_config("connection-a").unwrap().model,
            "another-window-model"
        );
        db.delete_agent_config("connection-a").unwrap();
        assert!(db
            .resolve_conversation_agent_config(&reloaded, None)
            .is_err());
    }
    #[test]
    fn selection_keeps_endpoint_credentials_limits_and_saved_config_owned_by_host() {
        let saved: AgentConfig = serde_json::from_value(serde_json::json!({
            "id":"private", "name":"Private API", "provider":"open_ai",
            "apiKey":"host-only", "baseUrl":"https://private.example/v1",
            "providerEndpointId":"text:private", "model":"original", "modelId":"original",
            "isDefault":true,"maxTokens":999,"maxIterations":7,"createdAt":"", "updatedAt":""
        }))
        .unwrap();
        let selection = TurnModelSelection {
            model: "another-model".into(),
            reasoning_enabled: Some(true),
            reasoning_effort: Some("high".into()),
            thinking_budget: None,
        };
        let selected = selection.apply(&saved).unwrap();
        assert_eq!(saved.model, "original");
        assert_eq!(selected.model_id.as_deref(), Some("another-model"));
        assert_eq!(selected.provider_endpoint_id, saved.provider_endpoint_id);
        assert_eq!(selected.base_url, saved.base_url);
        assert_eq!(selected.api_key, saved.api_key);
        assert_eq!(selected.max_tokens, saved.max_tokens);
        assert_eq!(selected.max_iterations, saved.max_iterations);
        assert!(serde_json::from_value::<TurnModelSelection>(
            serde_json::json!({"model":"model", "baseUrl":"https://other.example"})
        )
        .is_err());
        let mut native_selection = selection.clone();
        native_selection.reasoning_effort = Some("balanced/native".into());
        assert!(
            native_selection.apply(&saved).is_err(),
            "API effort enum stays strict"
        );
        let mut native = saved.clone();
        native.provider = "opencode".into();
        assert_eq!(
            native_selection
                .apply(&native)
                .unwrap()
                .reasoning_effort
                .as_deref(),
            Some("balanced/native")
        );
        native_selection.reasoning_effort = Some("bad\nvalue".into());
        assert!(native_selection.apply(&native).is_err());
    }
}
