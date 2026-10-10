//! External executors are not API endpoints. Only launch preferences live here;
//! credentials, models and tool execution remain owned by the installed agent.
use crate::{db::Database, error::CoreError};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, sync::OnceLock};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAgentPreset {
    pub provider: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    pub docs_url: String,
    #[serde(default)]
    pub registry_id: Option<String>,
    #[serde(default)]
    pub registry_version: Option<String>,
    #[serde(default)]
    pub distribution: Option<AgentDistribution>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AgentDistribution {
    #[serde(default)]
    pub binary: BTreeMap<String, BinaryDistribution>,
}

#[derive(Debug, Deserialize)]
pub struct BinaryDistribution {
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl ExternalAgentPreset {
    pub fn platform_launch(&self) -> Option<&BinaryDistribution> {
        let os = if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        };
        let platform = format!("{os}-{}", std::env::consts::ARCH);
        self.distribution.as_ref()?.binary.get(&platform)
    }

    pub fn launch_command(&self) -> String {
        // Package runners use their reviewed pinned argv. Binary commands are
        // installed by the user; nested archive paths are not assumed on PATH.
        if matches!(self.command.as_str(), "npx" | "uvx") {
            return self.command.clone();
        }
        self.platform_launch()
            .map(|platform| {
                platform
                    .cmd
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .into()
            })
            .unwrap_or_else(|| self.command.clone())
    }

    pub fn launch_args<'a>(&'a self, launch: &'a ExternalAgentLaunch) -> &'a [String] {
        launch.args.as_deref().unwrap_or_else(|| {
            if matches!(self.command.as_str(), "npx" | "uvx") {
                &self.args
            } else {
                self.platform_launch()
                    .map(|platform| platform.args.as_slice())
                    .unwrap_or(&self.args)
            }
        })
    }

    pub fn launch_env(&self, launch: &ExternalAgentLaunch) -> BTreeMap<String, String> {
        let mut env = self.env.clone();
        if let Some(platform) = self.platform_launch() {
            env.extend(platform.env.clone());
        }
        env.extend(launch.env.clone());
        env
    }
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
    /// Explicit replacement argv. None uses the preset; Some([]) means no args.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    /// User-managed environment, encrypted when stored, never a shell command.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Optional profile fallback. Blank follows the chat workspace or a managed
    /// folder; a resolved runtime launch never inherits the process cwd.
    #[serde(default)]
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
        if self.args.iter().flatten().any(|arg| arg.contains('\0'))
            || self.env.iter().any(|(key, value)| {
                key.is_empty() || key.contains(['=', '\0']) || value.contains('\0')
            })
        {
            return Err(CoreError::InvalidInput("External-agent arguments and environment cannot contain NUL; environment names cannot contain '='.".into()));
        }
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
        if !self.working_directory.trim().is_empty() && (!cwd.is_absolute() || !cwd.is_dir()) {
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

    pub fn validate_resolved(&self) -> Result<(), CoreError> {
        self.validate()?;
        if self.working_directory.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "Resolve the external agent's working directory before launch".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn storage_json(&self) -> Result<String, CoreError> {
        crate::crypto::encrypt_api_key(&serde_json::to_string(self)?)
    }

    /// One identity feeds process launch, protocol cwd, file permissions, and
    /// runtime cache binding. An explicitly empty project never falls back.
    pub fn resolve(
        &self,
        db: &Database,
        workspace: Option<&crate::workspace::Workspace>,
        conversation_id: Option<&str>,
    ) -> Result<Self, CoreError> {
        let mut preferences = self.clone();
        preferences.working_directory.clear();
        preferences.validate()?;
        let mut resolved = self.clone();
        let directory = if let Some(workspace) = workspace {
            workspace
                .cwd()
                .filter(|cwd| !cwd.trim().is_empty())
                .ok_or_else(|| {
                    CoreError::InvalidInput(
                        "Choose a project workspace folder before starting an external agent"
                            .into(),
                    )
                })?
                .to_string()
        } else if !self.working_directory.trim().is_empty() {
            self.working_directory.clone()
        } else {
            let app_data = db
                .db_path()
                .and_then(Path::parent)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| {
                    CoreError::InvalidInput(
                        "The managed external-agent workspace is unavailable".into(),
                    )
                })?;
            let key = conversation_id
                .map(|id| format!("chat-{}", blake3::hash(id.as_bytes()).to_hex()))
                .unwrap_or_else(|| "discovery".into());
            let managed = app_data
                .join("workspaces")
                .join("external-agents")
                .join(key);
            std::fs::create_dir_all(&managed).map_err(|error| {
                CoreError::InvalidInput(format!(
                    "Cannot create the external-agent workspace: {error}"
                ))
            })?;
            managed.to_string_lossy().into_owned()
        };
        resolved.working_directory = crate::workspace::Workspace::validate(&[directory])?
            .roots
            .remove(0);
        resolved.validate_resolved()?;
        Ok(resolved)
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
        let launch: ExternalAgentLaunch = json
            .map(|json| {
                let decoded = crate::crypto::decrypt_api_key(&json)?;
                serde_json::from_str(&decoded).map_err(CoreError::from)
            })
            .transpose()?
            .unwrap_or_default();
        Ok(launch)
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
    fn custom_launch_keeps_literal_arguments_and_encrypts_the_entire_profile() {
        let db = Database::open_memory().unwrap();
        let launch = ExternalAgentLaunch {
            executable: Some(std::env::current_exe().unwrap().to_string_lossy().into()),
            args: Some(vec![
                "--acp".into(),
                "literal & argument".into(),
                "private-argument".into(),
            ]),
            env: BTreeMap::from([("TEST_TOKEN".into(), "private-environment".into())]),
            ..Default::default()
        };
        let input = serde_json::from_value(serde_json::json!({"name":"Custom", "provider":"custom_acp", "apiKey":"", "model":"@nexa/agent-default", "isDefault":false})).unwrap();
        let saved = db.save_external_agent_profile(&input, &launch).unwrap();
        assert_eq!(db.external_agent_launch(&saved.id).unwrap(), launch);
        let stored: String = db
            .conn()
            .query_row(
                "SELECT value FROM app_config WHERE key=?1",
                [format!("external_agent_profile:{}", saved.id)],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stored.starts_with("enc:v1:"));
        assert!(!stored.contains("private-argument") && !stored.contains("private-environment"));
        let mut invalid = launch.clone();
        invalid.env.insert("invalid=name".into(), "x".into());
        assert!(invalid.validate().is_err());
        invalid = launch.clone();
        invalid.args = Some(vec!["nul\0byte".into()]);
        assert!(invalid.validate().is_err());
        db.conn()
            .execute(
                "UPDATE app_config SET value=?1 WHERE key=?2",
                rusqlite::params![
                    serde_json::to_string(&launch).unwrap(),
                    format!("external_agent_profile:{}", saved.id)
                ],
            )
            .unwrap();
        assert_eq!(
            db.external_agent_launch(&saved.id).unwrap(),
            launch,
            "legacy plaintext profiles remain readable"
        );
    }

    #[test]
    fn published_registry_entries_share_the_acp_route_and_preserve_platform_arguments() {
        let published = presets()
            .iter()
            .filter(|preset| preset.registry_id.is_some())
            .collect::<Vec<_>>();
        assert_eq!(published.len(), 41);
        let providers = presets()
            .iter()
            .map(|preset| &preset.provider)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(providers.len(), presets().len());
        for preset in published {
            assert!(is_agent_runtime(&preset.provider));
            assert!(preset
                .registry_version
                .as_ref()
                .is_some_and(|value| !value.is_empty()));
            assert!(!preset.launch_command().is_empty());
            assert_eq!(
                preset.launch_args(&ExternalAgentLaunch {
                    args: Some(vec![]),
                    ..Default::default()
                }),
                &[] as &[String]
            );
        }
        let qwen = preset("qwen_code").unwrap();
        assert!(qwen.args.contains(&"--experimental-skills".into()));
        assert!(preset("hermes").is_some() && preset("custom_acp").is_some());
    }

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
        assert!(ExternalAgentLaunch::default().validate().is_ok());
        assert!(ExternalAgentLaunch::default().validate_resolved().is_err());
    }

    #[test]
    fn automatic_directories_are_stable_scoped_and_project_authoritative() {
        let root = tempfile::tempdir().unwrap();
        let db = Database::new(root.path().join("nexa.db")).unwrap();
        let automatic: ExternalAgentLaunch = serde_json::from_str("{}").unwrap();
        automatic.validate().unwrap();
        let input: crate::conversation::SaveAgentConfigInput = serde_json::from_value(serde_json::json!({"name":"Automatic ACP", "provider":"hermes", "apiKey":"", "model":"native-model", "isDefault":false})).unwrap();
        let saved = db.save_external_agent_profile(&input, &automatic).unwrap();
        assert_eq!(
            db.external_agent_launch(&saved.id)
                .unwrap()
                .working_directory,
            ""
        );
        let first = automatic.resolve(&db, None, Some("../聊天:a")).unwrap();
        assert!(Path::new(&first.working_directory).is_dir());
        let normalized_root =
            crate::workspace::Workspace::validate(&[root.path().to_string_lossy().into()]).unwrap();
        assert!(Path::new(&first.working_directory).starts_with(normalized_root.cwd().unwrap()));
        assert_eq!(
            first,
            automatic.resolve(&db, None, Some("../聊天:a")).unwrap()
        );
        assert_ne!(
            first.working_directory,
            automatic
                .resolve(&db, None, Some("chat-b"))
                .unwrap()
                .working_directory
        );
        assert_ne!(
            first.working_directory,
            automatic
                .resolve(&db, None, None)
                .unwrap()
                .working_directory
        );
        let project = root.path().join("项目 空格");
        std::fs::create_dir(&project).unwrap();
        let workspace =
            crate::workspace::Workspace::validate(&[project.to_string_lossy().into()]).unwrap();
        let legacy = ExternalAgentLaunch {
            working_directory: root
                .path()
                .join("removed-legacy-folder")
                .to_string_lossy()
                .into(),
            ..Default::default()
        };
        assert_eq!(
            legacy
                .resolve(&db, Some(&workspace), Some("chat-a"))
                .unwrap()
                .working_directory,
            workspace.cwd().unwrap()
        );
        assert!(automatic
            .resolve(
                &db,
                Some(&crate::workspace::Workspace { roots: vec![] }),
                Some("chat-a")
            )
            .is_err());
        assert!(legacy.resolve(&db, None, Some("chat-a")).is_err());
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
