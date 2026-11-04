//! Credential-free picker projections for every host; providers own discovery.
use crate::{
    error::CoreError,
    llm::{create_provider, ProviderConfig},
    provider_catalog::{
        build_effective_model_catalog, retired_model_for_endpoint, ReasoningCapability,
    },
};
use serde::Serialize;
use std::time::Duration;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    pub id: String,
    pub name: String,
    pub reasoning: Option<ReasoningCapability>,
    pub vision: Option<bool>,
    pub available: bool,
    pub replacement_model_id: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoices {
    pub connection_id: String,
    pub models: Vec<ModelChoice>,
    pub discovery_succeeded: bool,
    pub error: Option<&'static str>,
}
impl ModelChoices {
    pub fn retain_selected(mut self, provider: &str, base_url: Option<&str>, model: &str) -> Self {
        if !self.models.iter().any(|item| item.id == model) {
            let retired = retired_model_for_endpoint(provider, base_url, model);
            self.models.insert(
                0,
                ModelChoice {
                    id: model.into(),
                    name: model.into(),
                    reasoning: None,
                    vision: None,
                    available: retired.is_none(),
                    replacement_model_id: retired.and_then(|item| item.replacement_model_id),
                },
            );
        }
        self
    }
}
pub async fn cloud_model_choices(
    connection_id: String,
    provider_key: &str,
    selected_model: &str,
    config: ProviderConfig,
) -> Result<ModelChoices, CoreError> {
    let base_url = config.base_url.clone();
    let provider = create_provider(config)?;
    // A rich snapshot already includes discovery: avoid another network request
    // and two consecutive timeout windows on a slow mobile connection.
    let discovered = tokio::time::timeout(Duration::from_secs(12), async {
        if let Ok(Some(snapshot)) = provider.model_catalog_snapshot().await {
            return Ok::<_, CoreError>(snapshot);
        }
        let live = provider.list_models().await?;
        Ok(build_effective_model_catalog(
            provider_key,
            base_url.as_deref(),
            Some(live),
            None,
            chrono::Utc::now().to_rfc3339(),
        ))
    })
    .await
    .ok()
    .and_then(Result::ok);
    let succeeded = discovered.is_some();
    let retirement = crate::provider_catalog::ModelRetirementPolicy::for_endpoint(
        provider_key,
        base_url.as_deref(),
    );
    let snapshot = discovered.unwrap_or_else(|| {
        build_effective_model_catalog(
            provider_key,
            base_url.as_deref(),
            None,
            None,
            chrono::Utc::now().to_rfc3339(),
        )
    });
    Ok(ModelChoices {
        connection_id,
        models: snapshot
            .models
            .into_iter()
            .filter(|model| retirement.get(&model.id).is_none())
            .map(|model| ModelChoice {
                id: model.id,
                name: model.name,
                reasoning: model
                    .capabilities
                    .as_ref()
                    .and_then(|cap| cap.reasoning.clone()),
                vision: model.capabilities.as_ref().and_then(|cap| cap.vision),
                available: true,
                replacement_model_id: None,
            })
            .collect(),
        discovery_succeeded: succeeded,
        error: (!succeeded).then_some("discovery_unavailable"),
    }
    .retain_selected(provider_key, base_url.as_deref(), selected_model))
}
