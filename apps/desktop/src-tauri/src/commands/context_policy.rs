use super::*;
use nexa_core::context_policy::{policy_snapshot, ModelContextPolicy, ModelContextPolicySnapshot};

#[tauri::command]
pub async fn get_model_context_policy_cmd(
    state: tauri::State<'_, AppState>,
    agent_config_id: String,
    model: String,
) -> Result<ModelContextPolicySnapshot, String> {
    let (provider, mut snapshot) = state
        .db_executor
        .read(move |db| {
            let mut config = db.get_agent_config(&agent_config_id)?;
            config.model = model;
            Ok((config.provider.clone(), policy_snapshot(db, &config)?))
        })
        .await
        .map(|result| result.value)
        .map_err(|error| error.to_string())?;
    if provider == "github_copilot" {
        let models =
            super::subscription_accounts::list_subscription_models_cmd(state, provider, None)
                .await?;
        snapshot.context_tiers = models
            .into_iter()
            .find(|model| model.id == snapshot.model)
            .ok_or_else(|| {
                "This model is no longer available to the Copilot account. Refresh models."
                    .to_string()
            })?
            .context_tiers;
    }
    Ok(snapshot)
}

#[tauri::command]
pub async fn save_model_context_policy_cmd(
    state: tauri::State<'_, AppState>,
    agent_config_id: String,
    model: String,
    policy: ModelContextPolicy,
) -> Result<ModelContextPolicySnapshot, String> {
    let snapshot =
        get_model_context_policy_cmd(state.clone(), agent_config_id.clone(), model.clone()).await?;
    if snapshot.managed_by_provider
        && (snapshot.context_tiers.is_empty()
            || policy.context_window.is_some()
            || policy.auto_compact_percent.is_some()
            || !snapshot
                .context_tiers
                .iter()
                .any(|tier| tier.id == policy.context_tier.as_deref().unwrap_or("default")))
    {
        return Err("Select a context tier advertised by this Copilot account; the runtime owns compaction.".into());
    }
    let tiers = snapshot.context_tiers;
    let mut saved = state
        .db_executor
        .write(move |db| {
            let mut config = db.get_agent_config(&agent_config_id)?;
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
        .map_err(|error| error.to_string())?;
    saved.context_tiers = tiers;
    Ok(saved)
}
