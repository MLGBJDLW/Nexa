//! A caller's model choice changes one run, without editing shared credentials.
use super::AgentConfig;
use crate::error::CoreError;
use serde::{Deserialize, Serialize};

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
        let model = self.model.trim();
        if model.is_empty() || model.len() > 512 || model.chars().any(char::is_control) {
            return Err(CoreError::InvalidInput("Choose a valid model ID".into()));
        }
        if self.reasoning_effort.as_deref().is_some_and(|effort| {
            !matches!(
                effort,
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
            )
        }) || self
            .thinking_budget
            .is_some_and(|budget| !(0..=i64::from(u32::MAX)).contains(&budget))
        {
            return Err(CoreError::InvalidInput(
                "Invalid model reasoning options".into(),
            ));
        }
        let mut selected = saved.clone();
        selected.model = model.into();
        selected.model_id = Some(model.into());
        selected.model_selection_resolution = None;
        selected.reasoning_enabled = self.reasoning_enabled;
        selected.reasoning_effort = self.reasoning_effort.clone();
        selected.thinking_budget = self.thinking_budget;
        Ok(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    }
}
