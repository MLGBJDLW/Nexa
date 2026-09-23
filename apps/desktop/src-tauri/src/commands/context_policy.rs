use super::*;
use nexa_core::context_policy::{policy_snapshot, ModelContextPolicy, ModelContextPolicySnapshot};

#[tauri::command]
pub async fn get_model_context_policy_cmd(
    state: tauri::State<'_, AppState>,
    agent_config_id: String,
    model: String,
) -> Result<ModelContextPolicySnapshot, String> {
    state
        .db_executor
        .read(move |db| {
            let mut config = db.get_agent_config(&agent_config_id)?;
            config.model = model;
            policy_snapshot(db, &config)
        })
        .await
        .map(|result| result.value)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_model_context_policy_cmd(
    state: tauri::State<'_, AppState>,
    agent_config_id: String,
    model: String,
    policy: ModelContextPolicy,
) -> Result<ModelContextPolicySnapshot, String> {
    state
        .db_executor
        .write(move |db| {
            let mut config = db.get_agent_config(&agent_config_id)?;
            if matches!(config.provider.as_str(), "github_copilot" | "openai_codex") {
                return Err(nexa_core::error::CoreError::InvalidInput(
                    "This subscription service manages context and compaction".into(),
                ));
            }
            config.model = model;
            db.save_model_context_policy(
                &config.provider,
                config.base_url.as_deref(),
                &config.model,
                &policy,
            )?;
            policy_snapshot(db, &config)
        })
        .await
        .map(|result| result.value)
        .map_err(|error| error.to_string())
}
