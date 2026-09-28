//! Explicit local launch preferences are separate from HTTP model credentials.
use super::AppState;
use nexa_core::external_agent::ExternalAgentLaunch;

#[tauri::command]
pub async fn get_external_agent_launch_cmd(
    state: tauri::State<'_, AppState>,
    provider: String,
) -> Result<ExternalAgentLaunch, String> {
    state
        .db_executor
        .read(move |db| db.external_agent_launch(&provider))
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
    provider: String,
    launch: ExternalAgentLaunch,
) -> Result<Vec<super::subscription_accounts::CopilotModelSummary>, String> {
    crate::agent_runtime::acp::probe(&provider, &launch)
        .await
        .map(|models| {
            models
                .into_iter()
                .map(|model| super::subscription_accounts::CopilotModelSummary {
                    id: model.id,
                    name: model.name,
                    reasoning_efforts: model.reasoning_efforts,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}
