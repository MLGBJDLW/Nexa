//! External executors are not API endpoints. Only launch preferences live here;
//! credentials, models and tool execution remain owned by the installed agent.
use crate::{db::Database, error::CoreError};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::OnceLock};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentPreset {
    pub provider: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    pub docs_url: String,
}

pub fn presets() -> &'static [ExternalAgentPreset] {
    static PRESETS: OnceLock<Vec<ExternalAgentPreset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../shared/external-agent-presets.json"))
            .expect("bundled external agent presets must be valid")
    })
}

pub fn preset(provider: &str) -> Option<&'static ExternalAgentPreset> {
    presets().iter().find(|entry| entry.provider == provider)
}

pub fn is_agent_runtime(provider: &str) -> bool {
    matches!(provider, "github_copilot" | "openai_codex") || preset(provider).is_some()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentLaunch {
    /// Optional absolute executable path; omitted uses the preset on PATH.
    #[serde(default)]
    pub executable: Option<String>,
    /// Deliberately explicit: never inherit the desktop process's cwd.
    pub working_directory: String,
    /// Native ACP select options, keyed by the advertised opaque config ID.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub config_options: std::collections::BTreeMap<String, String>,
    /// Model against which the saved model-dependent options were verified.
    /// Provider/mode preferences remain valid across chat model selections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_options_model: Option<String>,
    /// Explicitly selected user-managed MCP connectors forwarded to the agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_server_ids: Vec<String>,
}

impl ExternalAgentLaunch {
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.config_options.len() > 64
            || self
                .config_options_model
                .as_ref()
                .is_some_and(|model| model.len() > 1024)
            || self
                .config_options
                .iter()
                .any(|(key, value)| key.is_empty() || key.len() > 1024 || value.len() > 4096)
            || self.mcp_server_ids.len() > 32
            || self
                .mcp_server_ids
                .iter()
                .any(|id| id.is_empty() || id.len() > 256)
        {
            return Err(CoreError::InvalidInput(
                "External-agent options exceed the supported limits".into(),
            ));
        }
        let cwd = Path::new(&self.working_directory);
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(CoreError::InvalidInput(
                "Choose an existing absolute working directory for this external agent.".into(),
            ));
        }
        if let Some(executable) = self.executable.as_deref().filter(|value| !value.is_empty()) {
            let path = Path::new(executable);
            if !path.is_absolute() || !path.is_file() {
                return Err(CoreError::InvalidInput(
                    "The external agent executable must be an existing absolute file path.".into(),
                ));
            }
        }
        Ok(())
    }
}

impl Database {
    pub fn external_agent_launch(
        &self,
        agent_config_id: &str,
    ) -> Result<ExternalAgentLaunch, CoreError> {
        let connection = self.conn();
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='app_config')",
            [],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(ExternalAgentLaunch::default());
        }
        let json: Option<String> = connection
            .query_row(
                "SELECT value FROM app_config WHERE key = ?1",
                [format!("external_agent_profile:{agent_config_id}")],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(CoreError::from))
            .transpose()
            .map(Option::unwrap_or_default)
    }
}

pub(crate) fn ensure_launch_storage(connection: &rusqlite::Connection) -> Result<(), CoreError> {
    // app_config is intentionally initialized lazily by the settings store.
    // External-agent setup must also work before a first general-settings save.
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_config (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    )",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unconfigured_launch_is_readable_on_a_fresh_read_only_executor_lane() {
        let directory = tempfile::tempdir().unwrap();
        let db = Database::new(directory.path().join("external.db")).unwrap();
        let executor = crate::db_executor::DatabaseExecutor::new(db, 4).unwrap();
        let launch = executor
            .read(|db| db.external_agent_launch("hermes"))
            .await
            .unwrap()
            .value;
        assert_eq!(launch, ExternalAgentLaunch::default());
    }
    #[test]
    fn native_options_remain_bound_to_the_model_that_was_verified() {
        let db = Database::open_memory().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let launch = ExternalAgentLaunch {
            working_directory: cwd.path().to_string_lossy().into(),
            config_options: std::collections::BTreeMap::from([("fast".into(), "on".into())]),
            ..Default::default()
        };
        let mut input: crate::conversation::SaveAgentConfigInput = serde_json::from_value(serde_json::json!({"name":"Claude", "provider":"claude_code_acp", "apiKey":"", "model":"model-a", "isDefault":false})).unwrap();
        let saved = db.save_external_agent_profile(&input, &launch).unwrap();
        assert_eq!(
            db.external_agent_launch(&saved.id)
                .unwrap()
                .config_options_model
                .as_deref(),
            Some("model-a")
        );
        input.id = Some(saved.id.clone());
        input.model = "model-b".into();
        db.save_agent_config(&input).unwrap();
        assert_eq!(
            db.external_agent_launch(&saved.id)
                .unwrap()
                .config_options_model
                .as_deref(),
            Some("model-a"),
            "Changing the chat model must not rebind old native options"
        );
    }

    #[test]
    fn launch_preferences_round_trip_without_becoming_an_api_endpoint() {
        let db = Database::open_memory().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let launch = ExternalAgentLaunch {
            executable: None,
            working_directory: cwd.path().to_string_lossy().into_owned(),
            ..ExternalAgentLaunch::default()
        };
        for item in presets() {
            assert!(is_agent_runtime(&item.provider));
            let mut input: crate::conversation::SaveAgentConfigInput = serde_json::from_value(serde_json::json!({
                "name":item.name,"provider":item.provider,"apiKey":"","model":"vendor/模型:opaque","isDefault":false
            })).unwrap();
            let saved = db.save_external_agent_profile(&input, &launch).unwrap();
            assert_eq!(saved.model, "vendor/模型:opaque");
            assert_eq!(db.external_agent_launch(&saved.id).unwrap(), launch);
            input.api_key = "api-credential-must-not-cross-runtime".into();
            assert!(db.save_agent_config(&input).is_err());
            input.api_key.clear();
            input.base_url = Some("https://example.com/api".into());
            assert!(db.save_agent_config(&input).is_err());
        }
        assert!(!is_agent_runtime("google"));
        assert!(ExternalAgentLaunch::default().validate().is_err());
    }

    #[test]
    fn profile_and_launch_changes_roll_back_together() {
        let db = Database::open_memory().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let original = ExternalAgentLaunch {
            executable: None,
            working_directory: first.path().to_string_lossy().into_owned(),
            ..ExternalAgentLaunch::default()
        };
        let proposed = ExternalAgentLaunch {
            executable: None,
            working_directory: second.path().to_string_lossy().into_owned(),
            ..ExternalAgentLaunch::default()
        };
        let mut input: crate::conversation::SaveAgentConfigInput = serde_json::from_value(serde_json::json!({
            "name":"Original","provider":"hermes","apiKey":"","model":"native-model","isDefault":false
        })).unwrap();
        let saved = db.save_external_agent_profile(&input, &original).unwrap();
        input.id = Some(saved.id.clone());
        input.name = "Changed".into();
        db.conn().execute_batch("CREATE TRIGGER fail_external_launch BEFORE UPDATE ON app_config WHEN NEW.key LIKE 'external_agent_profile:%' BEGIN SELECT RAISE(ABORT, 'fixture launch write failure'); END;").unwrap();
        assert!(db.save_external_agent_profile(&input, &proposed).is_err());
        assert_eq!(db.get_agent_config(&saved.id).unwrap().name, "Original");
        assert_eq!(db.external_agent_launch(&saved.id).unwrap(), original);
    }

    #[test]
    fn two_profiles_of_the_same_agent_never_share_launch_settings() {
        let db = Database::open_memory().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let a = ExternalAgentLaunch {
            executable: None,
            working_directory: first.path().to_string_lossy().into_owned(),
            ..ExternalAgentLaunch::default()
        };
        let b = ExternalAgentLaunch {
            executable: None,
            working_directory: second.path().to_string_lossy().into_owned(),
            ..ExternalAgentLaunch::default()
        };
        let input: crate::conversation::SaveAgentConfigInput = serde_json::from_value(serde_json::json!({
            "name":"Hermes","provider":"hermes","apiKey":"","model":"native-model","isDefault":false
        })).unwrap();
        let profile_a = db.save_external_agent_profile(&input, &a).unwrap();
        let profile_b = db.save_external_agent_profile(&input, &b).unwrap();
        assert_ne!(profile_a.id, profile_b.id);
        assert_eq!(db.external_agent_launch(&profile_a.id).unwrap(), a);
        assert_eq!(db.external_agent_launch(&profile_b.id).unwrap(), b);
    }
}
