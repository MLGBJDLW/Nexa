//! Explicit local launch preferences are separate from HTTP model credentials.
use super::AppState;
use nexa_core::external_agent::ExternalAgentLaunch;

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
        .map(|catalog| summaries(catalog.models))
        .map_err(|error| error.to_string())
}

pub(super) fn summaries(
    models: Vec<crate::agent_runtime::acp::Model>,
) -> Vec<super::subscription_accounts::CopilotModelSummary> {
    models
        .into_iter()
        .map(|model| super::subscription_accounts::CopilotModelSummary {
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
) -> Result<crate::agent_runtime::acp::Catalog, String> {
    crate::agent_runtime::acp::probe(&provider, &launch, &state.db, model.as_deref())
        .await
        .map_err(|error| error.to_string())
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
