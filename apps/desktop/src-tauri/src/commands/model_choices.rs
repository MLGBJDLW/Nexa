//! Safe model discovery shared by the phone picker and desktop Live.
use super::{db_config_to_provider_config, list_subscription_models_cmd, AppState};
use nexa_core::{
    llm::create_provider,
    provider_catalog::{build_effective_model_catalog, ReasoningCapability},
};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    id: String,
    name: String,
    reasoning: Option<ReasoningCapability>,
    vision: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoices {
    connection_id: String,
    models: Vec<ModelChoice>,
    discovery_succeeded: bool,
    error: Option<&'static str>,
}

pub async fn model_choices(app: &AppHandle, connection_id: String) -> Result<ModelChoices, String> {
    let id = connection_id.clone();
    let config = app
        .state::<AppState>()
        .db_executor
        .read(move |db| db.get_agent_config(&id))
        .await
        .map_err(|e| e.to_string())?
        .value;
    let subscription =
        crate::subscription_runtime::SubscriptionRuntimeKind::from_provider(&config.provider)
            .is_some();
    let (mut models, discovery_succeeded) = if subscription {
        match list_subscription_models_cmd(app.state(), config.provider.clone()).await {
            Ok(models) => (
                models
                    .into_iter()
                    .map(|model| ModelChoice {
                        id: model.id,
                        name: model.name,
                        vision: None,
                        reasoning: (!model.reasoning_efforts.is_empty()).then_some(
                            ReasoningCapability {
                                mode: Some("optional".into()),
                                effort_levels: model.reasoning_efforts,
                                default_effort: None,
                                effort_budget_exclusive: true,
                                thinking_budget: None,
                            },
                        ),
                    })
                    .collect::<Vec<_>>(),
                true,
            ),
            Err(_) => (vec![], false),
        }
    } else {
        let provider = create_provider(db_config_to_provider_config(&config, Some(12)))
            .map_err(|e| e.to_string())?;
        let live = tokio::time::timeout(Duration::from_secs(12), provider.list_models())
            .await
            .ok()
            .and_then(Result::ok);
        let discovered = live.is_some();
        let snapshot = build_effective_model_catalog(
            &config.provider,
            config.base_url.as_deref(),
            live,
            None,
            chrono::Utc::now().to_rfc3339(),
        );
        (
            snapshot
                .models
                .into_iter()
                .take(1500)
                .map(|model| ModelChoice {
                    id: model.id,
                    name: model.name,
                    reasoning: model
                        .capabilities
                        .as_ref()
                        .and_then(|cap| cap.reasoning.clone()),
                    vision: model.capabilities.as_ref().and_then(|cap| cap.vision),
                })
                .collect(),
            discovered,
        )
    };
    if !models.iter().any(|model| model.id == config.model) {
        models.insert(
            0,
            ModelChoice {
                id: config.model.clone(),
                name: config.model,
                reasoning: None,
                vision: None,
            },
        );
    }
    Ok(ModelChoices {
        connection_id,
        models,
        discovery_succeeded,
        error: (!discovery_succeeded).then_some("discovery_unavailable"),
    })
}

#[tauri::command]
pub async fn model_choices_cmd(
    app: AppHandle,
    connection_id: String,
) -> Result<ModelChoices, String> {
    model_choices(&app, connection_id).await
}
