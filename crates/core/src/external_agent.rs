//! External executors are not API endpoints. Only launch preferences live here;
//! credentials, models and tool execution remain owned by the installed agent.
use crate::{db::Database, error::CoreError};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::OnceLock};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentPreset {
    pub provider: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
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
}

impl ExternalAgentLaunch {
    pub fn validate(&self) -> Result<(), CoreError> {
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
    pub fn external_agent_launch(&self, provider: &str) -> Result<ExternalAgentLaunch, CoreError> {
        if preset(provider).is_none() {
            return Err(CoreError::InvalidInput("Unknown external agent.".into()));
        }
        let connection = self.conn();
        ensure_launch_storage(&connection)?;
        let json: Option<String> = connection
            .query_row(
                "SELECT value FROM app_config WHERE key = ?1",
                [format!("external_agent:{provider}")],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(CoreError::from))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub fn save_external_agent_launch(
        &self,
        provider: &str,
        launch: &ExternalAgentLaunch,
    ) -> Result<(), CoreError> {
        if preset(provider).is_none() {
            return Err(CoreError::InvalidInput("Unknown external agent.".into()));
        }
        launch.validate()?;
        let connection = self.conn();
        ensure_launch_storage(&connection)?;
        connection.execute(
            "INSERT INTO app_config (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=datetime('now')",
            params![format!("external_agent:{provider}"), serde_json::to_string(launch)?],
        )?;
        Ok(())
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
    #[test]
    fn launch_preferences_round_trip_without_becoming_an_api_endpoint() {
        let db = Database::open_memory().unwrap();
        let cwd = tempfile::tempdir().unwrap();
        let launch = ExternalAgentLaunch {
            executable: None,
            working_directory: cwd.path().to_string_lossy().into_owned(),
        };
        for item in presets() {
            assert!(is_agent_runtime(&item.provider));
            db.save_external_agent_launch(&item.provider, &launch)
                .unwrap();
            assert_eq!(db.external_agent_launch(&item.provider).unwrap(), launch);
            let mut input: crate::conversation::SaveAgentConfigInput = serde_json::from_value(serde_json::json!({
                "name":item.name,"provider":item.provider,"apiKey":"","model":"vendor/模型:opaque","isDefault":false
            })).unwrap();
            assert_eq!(
                db.save_agent_config(&input).unwrap().model,
                "vendor/模型:opaque"
            );
            input.api_key = "api-credential-must-not-cross-runtime".into();
            assert!(db.save_agent_config(&input).is_err());
            input.api_key.clear();
            input.base_url = Some("https://example.com/api".into());
            assert!(db.save_agent_config(&input).is_err());
        }
        assert!(!is_agent_runtime("google"));
        assert!(db.save_external_agent_launch("google", &launch).is_err());
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
        };
        let proposed = ExternalAgentLaunch {
            executable: None,
            working_directory: second.path().to_string_lossy().into_owned(),
        };
        let mut input: crate::conversation::SaveAgentConfigInput = serde_json::from_value(serde_json::json!({
            "name":"Original","provider":"hermes","apiKey":"","model":"native-model","isDefault":false
        })).unwrap();
        let saved = db.save_external_agent_profile(&input, &original).unwrap();
        input.id = Some(saved.id.clone());
        input.name = "Changed".into();
        db.conn().execute_batch("CREATE TRIGGER fail_external_launch BEFORE UPDATE ON app_config WHEN NEW.key = 'external_agent:hermes' BEGIN SELECT RAISE(ABORT, 'fixture launch write failure'); END;").unwrap();
        assert!(db.save_external_agent_profile(&input, &proposed).is_err());
        assert_eq!(db.get_agent_config(&saved.id).unwrap().name, "Original");
        assert_eq!(db.external_agent_launch("hermes").unwrap(), original);
    }
}
