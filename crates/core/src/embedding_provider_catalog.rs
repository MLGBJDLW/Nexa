//! Shared embedding provider/model preset catalog.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum EmbeddingApiStyle {
    #[default]
    #[serde(rename = "openai_embeddings")]
    Openai,
    #[serde(rename = "voyage_embeddings")]
    Voyage,
    #[serde(rename = "cohere_embeddings")]
    Cohere,
    #[serde(rename = "gemini_embeddings")]
    Gemini,
    #[serde(rename = "jina_embeddings")]
    Jina,
}

fn default_batch_size() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingProviderPreset {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    #[serde(default)]
    pub workspace_region: Option<String>,
    pub description: String,
    #[serde(default)]
    pub api_style: EmbeddingApiStyle,
    #[serde(default = "default_batch_size")]
    pub max_batch_size: usize,
    pub models: Vec<EmbeddingModelPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelPreset {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub name: String,
    pub dimensions: usize,
    pub supports_dimension_override: bool,
    #[serde(default)]
    pub recommended: bool,
    #[serde(default)]
    pub allowed_dimensions: Vec<usize>,
    #[serde(default)]
    pub max_batch_size: Option<usize>,
    #[serde(default)]
    pub min_dimensions: Option<usize>,
    #[serde(default)]
    pub max_dimensions: Option<usize>,
    #[serde(default)]
    pub dimension_parameter: Option<String>,
    /// Conservative serving capacity when it differs from advertised metadata.
    #[serde(default)]
    pub effective_input_tokens: Option<usize>,
}

const EMBEDDING_PROVIDER_PRESETS_JSON: &str =
    include_str!("../../../shared/embedding-provider-presets.json");

pub fn load_embedding_provider_presets() -> Result<Vec<EmbeddingProviderPreset>, serde_json::Error>
{
    serde_json::from_str(EMBEDDING_PROVIDER_PRESETS_JSON)
}

pub fn find_embedding_model(base_url: &str, model: &str) -> Option<EmbeddingModelPreset> {
    find_embedding_provider(base_url)?
        .models
        .iter()
        .find(|candidate| {
            candidate.id == model.trim()
                || candidate.aliases.iter().any(|alias| alias == model.trim())
        })
        .cloned()
}

pub fn find_embedding_provider(base_url: &str) -> Option<&'static EmbeddingProviderPreset> {
    static PRESETS: OnceLock<Vec<EmbeddingProviderPreset>> = OnceLock::new();
    let normalized = normalize_base_url(base_url);
    let region = crate::provider_catalog::alibaba_workspace_region(&normalized);
    PRESETS
        .get_or_init(|| load_embedding_provider_presets().expect("embedded catalog must be valid"))
        .iter()
        .find(|preset| {
            normalize_base_url(&preset.base_url) == normalized
                || (region.is_some() && preset.workspace_region.as_deref() == region)
        })
}

pub(crate) fn normalize_base_url(base_url: &str) -> String {
    url::Url::parse(base_url.trim())
        .map(|url| url.to_string())
        .unwrap_or_else(|_| base_url.trim().into())
        .trim_end_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_catalog_is_valid_and_has_recommended_models() {
        let presets = load_embedding_provider_presets().expect("valid embedding provider catalog");
        assert!(presets.len() >= 5);
        for preset in presets.iter().filter(|preset| !preset.models.is_empty()) {
            assert!(preset.models.iter().any(|model| model.recommended));
        }
    }

    #[test]
    fn exact_provider_and_model_controls_dimension_override() {
        let openai = find_embedding_model("https://api.openai.com/v1/", "text-embedding-3-small")
            .expect("openai model");
        assert!(openai.supports_dimension_override);

        let mistral = find_embedding_model("https://api.mistral.ai/v1", "mistral-embed")
            .expect("mistral model");
        assert!(!mistral.supports_dimension_override);
        assert_eq!(mistral.dimensions, 1024);

        let qwen = find_embedding_model(
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "qwen3.7-text-embedding",
        )
        .expect("qwen model");
        assert!(qwen.supports_dimension_override);
        assert_eq!(qwen.dimensions, 1024);

        let singapore = "https://workspace.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1";
        assert_eq!(
            find_embedding_model(singapore, "text-embedding-v3")
                .unwrap()
                .allowed_dimensions,
            [512, 768, 1024]
        );
        assert!(find_embedding_model(singapore, "qwen3.7-text-embedding-flash").is_none());
        let hong_kong = "https://workspace.cn-hongkong.maas.aliyuncs.com/compatible-mode/v1";
        assert_eq!(find_embedding_provider(hong_kong).unwrap().models.len(), 1);
        assert!(find_embedding_model(hong_kong, "text-embedding-v4").is_some());
        assert!(find_embedding_provider(
            "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1"
        )
        .is_none());
    }
}
