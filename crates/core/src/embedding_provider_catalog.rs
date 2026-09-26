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
        .find(|candidate| candidate.id == model.trim())
        .cloned()
}

pub fn find_embedding_provider(base_url: &str) -> Option<&'static EmbeddingProviderPreset> {
    static PRESETS: OnceLock<Vec<EmbeddingProviderPreset>> = OnceLock::new();
    PRESETS
        .get_or_init(|| load_embedding_provider_presets().expect("embedded catalog must be valid"))
        .iter()
        .find(|preset| {
            normalize_base_url(&preset.base_url) == normalize_base_url(base_url)
                || (preset.id == "alibaba-model-studio-cn" && is_model_studio_workspace(base_url))
        })
}

fn is_model_studio_workspace(base_url: &str) -> bool {
    url::Url::parse(base_url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| {
                ["cn-beijing", "ap-southeast-1", "cn-hongkong"]
                    .iter()
                    .any(|region| host.ends_with(&format!(".{region}.maas.aliyuncs.com")))
            })
            && url.path().trim_end_matches('/') == "/compatible-mode/v1"
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
    }
}
