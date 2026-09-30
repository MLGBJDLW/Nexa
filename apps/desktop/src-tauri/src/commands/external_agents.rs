//! Explicit local launch preferences are separate from HTTP model credentials.
use super::AppState;
use nexa_core::external_agent::ExternalAgentLaunch;

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
