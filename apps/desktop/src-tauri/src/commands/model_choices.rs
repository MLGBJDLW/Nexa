//! Host credentials and subscription discovery; shared core owns cloud choices.
use super::{db_config_to_provider_config, list_subscription_models_cmd, AppState};
pub use nexa_core::model_choices::ModelChoices;
use nexa_core::model_choices::{cloud_model_choices, ModelChoice};
use nexa_core::provider_catalog::ReasoningCapability;
use tauri::{AppHandle, Manager};

pub async fn model_choices(app: &AppHandle, connection_id: String) -> Result<ModelChoices, String> {
    let id = connection_id.clone();
    let config = app
        .state::<AppState>()
        .db_executor
        .read(move |db| db.get_agent_config(&id))
        .await
        .map_err(|e| e.to_string())?
        .value;
    if crate::agent_runtime::AgentRuntimeKind::from_provider(&config.provider).is_none() {
        return cloud_model_choices(
            connection_id,
            &config.provider,
            &config.model,
            db_config_to_provider_config(&config, Some(12)),
        )
        .await
        .map_err(|e| e.to_string());
    }
    let discovered = list_subscription_models_cmd(
        app.state(),
        config.provider.clone(),
        Some(config.id.clone()),
    )
    .await
    .ok();
    let succeeded = discovered.is_some();
    Ok(ModelChoices {
        connection_id,
        models: discovered
            .unwrap_or_default()
            .into_iter()
            .map(|model| ModelChoice {
                id: model.id,
                name: model.name,
                vision: None,
                available: true,
                replacement_model_id: None,
                reasoning: (!model.reasoning_efforts.is_empty()).then_some(ReasoningCapability {
                    mode: Some("optional".into()),
                    disabled_mode: None,
                    effort_levels: model.reasoning_efforts,
                    default_effort: None,
                    default_enabled: None,
                    effort_budget_exclusive: true,
                    thinking_budget: None,
                }),
            })
            .collect(),
        discovery_succeeded: succeeded,
        error: (!succeeded).then_some("discovery_unavailable"),
    }
    .retain_selected(&config.provider, config.base_url.as_deref(), &config.model))
}

#[tauri::command]
pub async fn model_choices_cmd(
    app: AppHandle,
    connection_id: String,
) -> Result<ModelChoices, String> {
    model_choices(&app, connection_id).await
}
