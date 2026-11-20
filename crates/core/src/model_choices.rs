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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub service_tiers: Vec<crate::model_catalog::ServiceTier>,
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
                    service_tiers: Vec::new(),
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
        if let Ok(Some(snapshot)) = provider.discover_picker_catalog(None).await {
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
    let succeeded = discovered
        .as_ref()
        .is_some_and(|snapshot| snapshot.live_discovery_succeeded);
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
    let selected_tombstone = snapshot
        .tombstones
        .iter()
        .find(|m| m.matches_id_or_alias(selected_model));
    let mut choices = ModelChoices {
        connection_id,
        models: snapshot
            .models
            .into_iter()
            .filter(|model| retirement.get(&model.id).is_none())
            .map(|model| ModelChoice {
                service_tiers: crate::provider_catalog::model_service_tiers(
                    provider_key,
                    base_url.as_deref(),
                    &model.id,
                )
                .to_vec(),
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
    .retain_selected(provider_key, base_url.as_deref(), selected_model);
    if let Some(retired) = selected_tombstone {
        if let Some(selected) = choices.models.iter_mut().find(|m| m.id == selected_model) {
            selected.available = false;
            selected.replacement_model_id = retired.replacement_model_id.clone();
        }
    }
    Ok(choices)
}
