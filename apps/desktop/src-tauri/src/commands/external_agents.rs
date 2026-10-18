//! Explicit local launch preferences are separate from HTTP model credentials.
use super::AppState;
use nexa_core::agent::CancellationToken;
use nexa_core::external_agent::ExternalAgentLaunch;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Instant,
};

struct SetupEntry {
    token: CancellationToken,
    active: bool,
    created: Instant,
}
fn setup_operations() -> &'static Mutex<HashMap<String, SetupEntry>> {
    static OPERATIONS: OnceLock<Mutex<HashMap<String, SetupEntry>>> = OnceLock::new();
    OPERATIONS.get_or_init(Default::default)
}
struct SetupOperation {
    id: String,
    token: CancellationToken,
}
impl SetupOperation {
    fn new(id: Option<String>) -> Result<Self, String> {
        let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        uuid::Uuid::parse_str(&id)
            .map_err(|_| "Invalid external-agent setup request id".to_string())?;
        let mut operations = setup_operations()
            .lock()
            .map_err(|_| "External-agent setup state is unavailable")?;
        operations.retain(|_, entry| entry.active || entry.created.elapsed().as_secs() < 600);
        if let Some(entry) = operations.get(&id) {
            if entry.token.is_cancelled() {
                return Err("External-agent setup cancelled".into());
            }
            return Err("This external-agent setup request is already active".into());
        }
        let token = CancellationToken::new();
        operations.insert(
            id.clone(),
            SetupEntry {
                token: token.clone(),
                active: true,
                created: Instant::now(),
            },
        );
        Ok(Self { id, token })
    }
    async fn run<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, nexa_core::error::CoreError>>,
    ) -> Result<T, String> {
        tokio::select! {
            biased;
            _ = self.token.cancelled() => Err("External-agent setup cancelled".into()),
            result = future => result.map_err(|error| error.to_string()),
        }
    }
}
impl Drop for SetupOperation {
    fn drop(&mut self) {
        if let Ok(mut operations) = setup_operations().lock() {
            operations.remove(&self.id);
        }
    }
}

#[tauri::command]
pub fn cancel_external_agent_setup_cmd(request_id: String) -> Result<(), String> {
    uuid::Uuid::parse_str(&request_id)
        .map_err(|_| "Invalid external-agent setup request id".to_string())?;
    let mut operations = setup_operations()
        .lock()
        .map_err(|_| "External-agent setup state is unavailable")?;
    operations.retain(|_, entry| entry.active || entry.created.elapsed().as_secs() < 600);
    operations
        .entry(request_id)
        .or_insert_with(|| SetupEntry {
            token: CancellationToken::new(),
            active: false,
            created: Instant::now(),
        })
        .token
        .cancel();
    Ok(())
}

fn usage_launch(
    db: &nexa_core::db::Database,
    launch: ExternalAgentLaunch,
    conversation_id: Option<&str>,
    project_id: Option<&str>,
) -> Result<Option<ExternalAgentLaunch>, nexa_core::error::CoreError> {
    let workspace = if let Some(id) = conversation_id {
        db.conversation_workspace(id)?
    } else if let Some(id) = project_id {
        db.get_project(id)?
            .workspace_roots
            .map(|roots| nexa_core::workspace::Workspace { roots })
    } else {
        None
    };
    // A projectless new chat has no final managed directory yet. Never inspect
    // discovery/.hermes in place of the future chat's cwd-dependent account.
    if conversation_id.is_none()
        && workspace.is_none()
        && launch.working_directory.trim().is_empty()
    {
        return Ok(None);
    }
    launch
        .resolve(db, workspace.as_ref(), conversation_id)
        .map(Some)
}

#[tauri::command]
pub fn get_external_agent_registry_cmd() -> nexa_core::external_agent::registry::RegistrySnapshot {
    nexa_core::external_agent::registry::snapshot()
}

#[tauri::command]
pub async fn check_external_agent_updates_cmd(
    force: bool,
) -> Result<nexa_core::external_agent::registry::RegistrySnapshot, String> {
    nexa_core::external_agent::registry::refresh(force)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn update_external_agent_cmd(
    state: tauri::State<'_, AppState>,
    provider: String,
    fingerprint: String,
    request_id: Option<String>,
) -> Result<nexa_core::external_agent::registry::RegistrySnapshot, String> {
    let operation = SetupOperation::new(request_id)?;
    operation
        .run(crate::agent_runtime::acp::update_adapter(
            &provider,
            &fingerprint,
            &state.db,
        ))
        .await
}

#[tauri::command]
pub async fn get_external_agent_usage_cmd(
    state: tauri::State<'_, AppState>,
    agent_config_id: String,
    model: String,
    conversation_id: Option<String>,
    project_id: Option<String>,
) -> Result<crate::agent_runtime::acp::AccountUsageSnapshot, String> {
    let (provider, launch) = state
        .db_executor
        .read(move |db| {
            let config = db.get_agent_config(&agent_config_id)?;
            let launch = db.external_agent_launch(&agent_config_id)?;
            let launch = if config.provider == "hermes" {
                usage_launch(
                    db,
                    launch,
                    conversation_id.as_deref(),
                    project_id.as_deref(),
                )?
            } else {
                Some(launch)
            };
            Ok((config.provider, launch))
        })
        .await
        .map(|result| result.value)
        .map_err(|error| error.to_string())?;
    let Some(launch) = launch else {
        return Ok(crate::agent_runtime::acp::AccountUsageSnapshot::unsupported());
    };
    crate::agent_runtime::acp::read_account_usage(&provider, &launch, &model)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_external_agent_launch_cmd(
    state: tauri::State<'_, AppState>,
    agent_config_id: String,
) -> Result<ExternalAgentLaunch, String> {
    state
        .db_executor
        .read(move |db| db.external_agent_launch(&agent_config_id))
        .await
        .map(|result| result.value)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_external_agent_profile_cmd(
    state: tauri::State<'_, AppState>,
    config: nexa_core::conversation::SaveAgentConfigInput,
    launch: ExternalAgentLaunch,
) -> Result<nexa_core::conversation::AgentConfig, String> {
    state
        .db_executor
        .write(move |db| db.save_external_agent_profile(&config, &launch))
        .await
        .map(|result| result.value)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn probe_external_agent_cmd(
    state: tauri::State<'_, AppState>,
    provider: String,
    launch: ExternalAgentLaunch,
    model: Option<String>,
) -> Result<Vec<super::subscription_accounts::CopilotModelSummary>, String> {
    crate::agent_runtime::acp::probe(&provider, &launch, &state.db, model.as_deref())
        .await
        .map_err(|error| error.to_string())
        .and_then(connected_models)
}

pub(super) fn connected_models(
    catalog: crate::agent_runtime::acp::Catalog,
) -> Result<Vec<super::subscription_accounts::CopilotModelSummary>, String> {
    if catalog.requires_authentication {
        return Err(
            "Sign in to this external agent from its Settings profile, then refresh models.".into(),
        );
    }
    Ok(summaries(catalog.models))
}

pub(super) fn summaries(
    models: Vec<crate::agent_runtime::acp::Model>,
) -> Vec<super::subscription_accounts::CopilotModelSummary> {
    models
        .into_iter()
        .map(|model| super::subscription_accounts::CopilotModelSummary {
            supports_reasoning_effort: None,
            context_tiers: Vec::new(),
            id: model.id,
            name: model.name,
            reasoning_efforts: model.reasoning_efforts,
            context_window: None,
        })
        .collect()
}

#[tauri::command]
pub async fn inspect_external_agent_cmd(
    state: tauri::State<'_, AppState>,
    provider: String,
    launch: ExternalAgentLaunch,
    model: Option<String>,
    request_id: Option<String>,
) -> Result<crate::agent_runtime::acp::Catalog, String> {
    let operation = SetupOperation::new(request_id)?;
    operation
        .run(crate::agent_runtime::acp::probe(
            &provider,
            &launch,
            &state.db,
            model.as_deref(),
        ))
        .await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalAgentAuthentication {
    catalog: Option<crate::agent_runtime::acp::Catalog>,
    terminal: Option<super::TerminalSessionInfo>,
}

#[tauri::command]
pub async fn authenticate_external_agent_cmd(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    terminals: tauri::State<'_, super::TerminalState>,
    provider: String,
    launch: ExternalAgentLaunch,
    model: Option<String>,
    method_id: String,
    request_id: Option<String>,
) -> Result<ExternalAgentAuthentication, String> {
    let operation = SetupOperation::new(request_id)?;
    match operation
        .run(crate::agent_runtime::acp::authenticate(
            &provider,
            &launch,
            &state.db,
            model.as_deref(),
            &method_id,
        ))
        .await?
    {
        crate::agent_runtime::acp::Authentication::Connected(catalog) => {
            Ok(ExternalAgentAuthentication {
                catalog: Some(catalog),
                terminal: None,
            })
        }
        crate::agent_runtime::acp::Authentication::Terminal { plan, cwd, label } => {
            let state = terminals.inner().clone();
            let terminal = tokio::task::spawn_blocking(move || {
                super::terminal::start_external_login(app_handle, &state, plan, cwd, label)
            })
            .await
            .map_err(|error| error.to_string())??;
            if operation.token.is_cancelled() {
                let _ = terminals.close_session(&terminal.id);
                return Err("External-agent setup cancelled".into());
            }
            Ok(ExternalAgentAuthentication {
                catalog: None,
                terminal: Some(terminal),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn account_usage_resolves_the_chat_workspace_and_relative_home_together() {
        let directory = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();
        let db = nexa_core::db::Database::open_memory().unwrap();
        let project = db
            .create_project(
                &serde_json::from_value(
                    json!({"name":"Quota project", "workspaceRoots":[directory.path()]}),
                )
                .unwrap(),
            )
            .unwrap();
        let conversation = db.create_conversation(&serde_json::from_value(json!({"provider":"hermes", "model":"anthropic:claude", "projectId":project.id})).unwrap()).unwrap();
        let launch = ExternalAgentLaunch {
            working_directory: fallback.path().to_string_lossy().into(),
            env: [("HERMES_HOME".into(), ".hermes".into())].into(),
            ..Default::default()
        };
        let resolved = usage_launch(
            &db,
            launch.clone(),
            Some(&conversation.id),
            Some("unrelated-project"),
        )
        .unwrap()
        .unwrap();
        let workspace = db.conversation_workspace(&conversation.id).unwrap();
        let runtime = launch
            .resolve(&db, workspace.as_ref(), Some(&conversation.id))
            .unwrap();
        assert_eq!(resolved, runtime);
        assert_eq!(
            resolved.env.get("HERMES_HOME").map(String::as_str),
            Some(".hermes")
        );
        assert_eq!(
            usage_launch(&db, launch, None, Some(&project.id))
                .unwrap()
                .unwrap()
                .working_directory,
            runtime.working_directory
        );
    }

    #[test]
    fn account_usage_never_guesses_the_future_projectless_chat_directory() {
        let directory = tempfile::tempdir().unwrap();
        let db = nexa_core::db::Database::new(directory.path().join("nexa.db")).unwrap();
        let launch = ExternalAgentLaunch::default();
        assert!(usage_launch(&db, launch.clone(), None, None)
            .unwrap()
            .is_none());
        let conversation = db
            .create_conversation(
                &serde_json::from_value(json!({"provider":"hermes", "model":"anthropic:claude"}))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            usage_launch(&db, launch.clone(), Some(&conversation.id), None)
                .unwrap()
                .unwrap(),
            launch.resolve(&db, None, Some(&conversation.id)).unwrap()
        );
    }
}
