//! OpenRouter's account-scoped model metadata, using the existing catalog hook.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::error::CoreError;
use crate::model_catalog::{ModelModality, ReasoningCapability, ThinkingBudgetCapability};
use crate::provider_catalog::{build_effective_model_catalog, ProviderModelCatalogSnapshot};

use super::{is_openrouter_config, OpenAiProvider};

const CATALOG_TTL: Duration = Duration::from_secs(300);
const CATALOG_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CACHED_ACCOUNTS: usize = 32;

#[derive(Clone, Deserialize)]
#[serde(from = "RawModelEntry")]
pub(super) struct ModelEntry {
    pub id: String,
    name: Option<String>,
    supported_parameters: Option<Vec<String>>,
    architecture: Option<Architecture>,
    context_length: Option<u64>,
    top_provider: Option<TopProvider>,
    reasoning: Option<Reasoning>,
    endpoints: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawModelEntry {
    id: String,
    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}

impl From<RawModelEntry> for ModelEntry {
    fn from(mut raw: RawModelEntry) -> Self {
        fn take<T: serde::de::DeserializeOwned>(
            fields: &mut serde_json::Map<String, serde_json::Value>,
            key: &str,
        ) -> Option<T> {
            serde_json::from_value(fields.remove(key)?).ok()
        }
        // OpenAI-compatible vendors add differently shaped optional metadata.
        // Keep valid IDs when an optional field uses an unfamiliar schema.
        Self {
            id: raw.id,
            name: take(&mut raw.fields, "name"),
            supported_parameters: take(&mut raw.fields, "supported_parameters"),
            architecture: take(&mut raw.fields, "architecture"),
            context_length: take(&mut raw.fields, "context_length"),
            top_provider: take(&mut raw.fields, "top_provider"),
            reasoning: take(&mut raw.fields, "reasoning"),
            endpoints: take(&mut raw.fields, "endpoints"),
        }
    }
}

#[derive(Clone, Deserialize)]
struct Architecture {
    input_modalities: Option<Vec<String>>,
    output_modalities: Option<Vec<String>>,
}

fn known_modalities(values: &[String]) -> Vec<ModelModality> {
    values
        .iter()
        .filter_map(|value| serde_json::from_value(serde_json::Value::String(value.clone())).ok())
        .collect()
}

#[derive(Clone, Deserialize)]
struct TopProvider {
    max_completion_tokens: Option<u64>,
}

#[derive(Clone, Deserialize)]
struct Reasoning {
    mandatory: Option<bool>,
    default_enabled: Option<bool>,
    supported_efforts: Option<Vec<String>>,
    default_effort: Option<String>,
    supports_max_tokens: Option<bool>,
}

#[derive(Deserialize)]
pub(super) struct ModelsResponse {
    pub data: Vec<ModelEntry>,
    #[serde(default)]
    links: Links,
    total_count: Option<usize>,
}

#[derive(Default, Deserialize)]
struct Links {
    next: Option<String>,
}

#[derive(Default)]
struct CachedDiscovery {
    checked_at: Option<Instant>,
    models: Option<Arc<Vec<ModelEntry>>>,
    refreshed_at: Option<String>,
    refresh_succeeded: bool,
}

type AccountCache = VecDeque<(String, Arc<tokio::sync::Mutex<CachedDiscovery>>)>;

fn account_cache(provider: &OpenAiProvider) -> Arc<tokio::sync::Mutex<CachedDiscovery>> {
    // Hash the complete identity without retaining credentials in cache keys.
    let mut hasher = blake3::Hasher::new();
    hasher.update(provider.base_url().as_bytes());
    hasher.update(&[0]);
    hasher.update(
        provider
            .config
            .api_key
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    let key = hasher.finalize().to_hex().to_string();
    static CACHE: OnceLock<Mutex<AccountCache>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(VecDeque::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(index) = cache.iter().position(|(candidate, _)| candidate == &key) {
        let entry = cache.remove(index).expect("index came from cache");
        let value = Arc::clone(&entry.1);
        cache.push_back(entry);
        return value;
    }
    let value = Arc::new(tokio::sync::Mutex::new(CachedDiscovery::default()));
    cache.push_back((key, Arc::clone(&value)));
    while cache.len() > MAX_CACHED_ACCOUNTS {
        cache.pop_front();
    }
    value
}

impl ModelEntry {
    fn supports_chat(&self) -> bool {
        !self.id.trim().is_empty()
            && !self.id.ends_with(":batch")
            && self.endpoints.as_ref().is_none_or(|endpoints| {
                endpoints
                    .iter()
                    .any(|endpoint| endpoint == "/api/v1/chat/completions")
            })
            && self
                .architecture
                .as_ref()
                .and_then(|architecture| architecture.output_modalities.as_ref())
                .is_none_or(|modalities| modalities.iter().any(|modality| modality == "text"))
    }
}

impl OpenAiProvider {
    pub(super) async fn discover_model_entries(&self) -> Result<Vec<ModelEntry>, CoreError> {
        tokio::time::timeout(CATALOG_TIMEOUT, async {
            let transport = self.transport.for_request()?;
            let initial =
                reqwest::Url::parse(&format!("{}/models", self.base_url().trim_end_matches('/')))
                    .map_err(|_| CoreError::Llm("Invalid model catalog URL".into()))?;
            let mut url = initial.clone();
            let mut seen = HashSet::new();
            let mut rows = Vec::new();
            let mut total = None;
            loop {
                if url.origin() != initial.origin()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || !seen.insert(url.to_string())
                {
                    return Err(CoreError::Llm("Invalid model catalog pagination".into()));
                }
                let response = self
                    .request_headers(
                        transport
                            .client()
                            .get(url.clone())
                            .bearer_auth(self.api_key()?),
                        None,
                    )
                    .send()
                    .await
                    .map_err(|error| {
                        CoreError::Llm(format!(
                            "Model discovery failed: {}",
                            super::super::transport::describe_http_error(&error)
                        ))
                    })?;
                let response = self.check_response(response).await?;
                // A transport redirect must not turn a foreign catalog into
                // authoritative metadata for the configured endpoint.
                if response.url().origin() != initial.origin() {
                    return Err(CoreError::Llm(
                        "Model catalog redirected to another origin".into(),
                    ));
                }
                let page: ModelsResponse = response.json().await.map_err(|error| {
                    CoreError::Llm(format!(
                        "Invalid model catalog: {}",
                        super::super::transport::describe_http_error(&error)
                    ))
                })?;
                rows.extend(page.data);
                total = page.total_count.or(total);
                let Some(next) = page.links.next.filter(|next| !next.is_empty()) else {
                    break;
                };
                url = url
                    .join(&next)
                    .map_err(|_| CoreError::Llm("Invalid model catalog pagination".into()))?;
            }
            if total.is_some_and(|total| rows.len() < total) {
                return Err(CoreError::Llm("Incomplete model catalog".into()));
            }
            if is_openrouter_config(&self.config) {
                rows.retain(ModelEntry::supports_chat);
            }
            Ok(rows)
        })
        .await
        .map_err(|_| CoreError::Llm("Model discovery timed out".into()))?
    }

    pub(super) async fn openrouter_catalog_snapshot(&self) -> Option<ProviderModelCatalogSnapshot> {
        if !is_openrouter_config(&self.config) {
            return None;
        }
        let cache = account_cache(self);
        // Fit the caller's twelve-second cloud-choice deadline, including lock
        // waiting, parsing and fallback. A rich fallback avoids a second fetch.
        let (models, refreshed_at, succeeded) = tokio::time::timeout(CATALOG_TIMEOUT + Duration::from_millis(500), async {
            let mut cached = cache.lock().await;
            if cached.checked_at.is_none_or(|checked| checked.elapsed() >= CATALOG_TTL) {
                match self.discover_model_entries().await {
                    Ok(models) => {
                        cached.models = Some(Arc::new(models));
                        cached.refreshed_at = Some(chrono::Utc::now().to_rfc3339());
                        cached.refresh_succeeded = true;
                    }
                    Err(error) => {
                        cached.refresh_succeeded = false;
                        tracing::warn!("OpenRouter model discovery unavailable; preserving last known catalog: {error}");
                    }
                }
                // Failure cooldown prevents every turn retrying an unavailable API.
                cached.checked_at = Some(Instant::now());
            }
            (cached.models.clone(), cached.refreshed_at.clone(), cached.refresh_succeeded)
        }).await.unwrap_or_else(|_| cache.try_lock().ok().map(|cached| (cached.models.clone(), cached.refreshed_at.clone(), false)).unwrap_or_default());
        let mut snapshot = snapshot_from_models(
            self,
            models.as_deref().map(Vec::as_slice),
            refreshed_at.as_deref(),
        );
        snapshot.live_discovery_succeeded = succeeded;
        Some(snapshot)
    }
}

fn snapshot_from_models(
    provider: &OpenAiProvider,
    models: Option<&[ModelEntry]>,
    refreshed_at: Option<&str>,
) -> ProviderModelCatalogSnapshot {
    let mut snapshot = build_effective_model_catalog(
        crate::provider_registry::canonical_provider_key(provider.config.provider_type),
        provider.config.base_url.as_deref(),
        models.map(|models| models.iter().map(|model| model.id.clone()).collect()),
        None,
        refreshed_at
            .map(str::to_owned)
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
    );
    for model in models.into_iter().flatten() {
        let Some(descriptor) = snapshot
            .descriptors
            .iter_mut()
            .find(|descriptor| descriptor.id.eq_ignore_ascii_case(&model.id))
        else {
            continue;
        };
        if let Some(name) = model.name.as_deref().filter(|name| !name.trim().is_empty()) {
            descriptor.display_name = name.to_string();
        }
        if let Some(architecture) = &model.architecture {
            if let Some(inputs) = &architecture.input_modalities {
                descriptor.input_modalities = known_modalities(inputs);
                descriptor.capabilities.vision =
                    descriptor.input_modalities.contains(&ModelModality::Image);
                descriptor.capabilities.audio_input =
                    descriptor.input_modalities.contains(&ModelModality::Audio);
                descriptor.capabilities.video_input =
                    descriptor.input_modalities.contains(&ModelModality::Video);
            }
            if let Some(outputs) = &architecture.output_modalities {
                descriptor.output_modalities = known_modalities(outputs);
            }
        }
        let reasoning_parameters = model.supported_parameters.as_ref().map(|parameters| {
            parameters.iter().any(|parameter| {
                parameter.starts_with("reasoning")
                    || matches!(
                        parameter.as_str(),
                        "include_reasoning" | "thinking" | "thinking_budget"
                    )
            })
        });
        let reasoning_evidence = model.reasoning.as_ref().is_some_and(|reasoning| {
            reasoning.mandatory == Some(true)
                || reasoning.supports_max_tokens == Some(true)
                || reasoning
                    .supported_efforts
                    .as_ref()
                    .is_some_and(|efforts| !efforts.is_empty())
                || (reasoning_parameters.is_none() && reasoning.default_enabled.is_some())
        });
        if let Some(parameters) = &model.supported_parameters {
            descriptor.capabilities.tool_calling =
                parameters.iter().any(|parameter| parameter == "tools");
            descriptor.capabilities.structured_output = parameters.iter().any(|parameter| {
                matches!(parameter.as_str(), "structured_outputs" | "response_format")
            });
            if reasoning_parameters == Some(false) && !reasoning_evidence {
                descriptor.capabilities.reasoning = None;
            }
        }
        if let Some(context) = model.context_length.filter(|context| *context > 0) {
            descriptor.limits.context_tokens = Some(context);
        }
        if let Some(output) = model
            .top_provider
            .as_ref()
            .and_then(|provider| provider.max_completion_tokens)
            .filter(|output| *output > 0)
        {
            descriptor.limits.max_output_tokens = Some(output);
        }
        if let Some(reasoning) = model
            .reasoning
            .as_ref()
            .filter(|_| reasoning_parameters == Some(true) || reasoning_evidence)
        {
            let capability = descriptor
                .capabilities
                .reasoning
                .get_or_insert_with(ReasoningCapability::default);
            if let Some(mandatory) = reasoning.mandatory {
                capability.mode = Some(if mandatory { "always" } else { "optional" }.into());
            }
            if let Some(enabled) = reasoning
                .default_enabled
                .or_else(|| (reasoning.mandatory == Some(true)).then_some(true))
            {
                capability.default_enabled = Some(enabled);
            }
            if let Some(efforts) = &reasoning.supported_efforts {
                capability.effort_levels = efforts.clone();
            }
            if let Some(default) = &reasoning.default_effort {
                capability.default_effort = Some(default.clone());
            }
            if let Some(enabled) = reasoning.supports_max_tokens {
                capability.thinking_budget = Some(ThinkingBudgetCapability {
                    enabled,
                    ..Default::default()
                });
            }
        }
        if let Some(entry) = snapshot
            .models
            .iter_mut()
            .find(|entry| entry.id.eq_ignore_ascii_case(&model.id))
        {
            entry.name = descriptor.display_name.clone();
            entry.supports_tools = Some(descriptor.capabilities.tool_calling);
            entry.supports_structured_output = Some(descriptor.capabilities.structured_output);
            entry.modalities = descriptor
                .input_modalities
                .iter()
                .filter_map(|modality| {
                    serde_json::to_value(modality)
                        .ok()?
                        .as_str()
                        .map(str::to_owned)
                })
                .collect();
            entry.reasoning_efforts = descriptor
                .capabilities
                .reasoning
                .as_ref()
                .map(|reasoning| reasoning.effort_levels.clone())
                .unwrap_or_default();
            entry.capabilities = serde_json::from_value(serde_json::json!({ "vision":descriptor.capabilities.vision, "reasoning":descriptor.capabilities.reasoning })).ok();
        }
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ProviderConfig, ProviderType};

    fn provider() -> OpenAiProvider {
        OpenAiProvider::new(ProviderConfig {
            provider_type: ProviderType::OpenRouter,
            api_key: Some("catalog-test-key".into()),
            base_url: Some("https://openrouter.ai/api/v1".into()),
            org_id: None,
            timeout_secs: None,
            streaming: Default::default(),
        })
        .unwrap()
    }

    #[test]
    fn openrouter_catalog_preserves_future_model_capabilities_and_limits() {
        let models: ModelsResponse = serde_json::from_value(serde_json::json!({"data":[{
            "id":"google/future-gemini", "name":"Future Gemini", "supported_parameters":["tools","structured_outputs","reasoning"],
            "architecture":{"input_modalities":["text","image","file"],"output_modalities":["text"]},
            "context_length":1048576,"top_provider":{"max_completion_tokens":65536},
            "reasoning":{"mandatory":true,"supported_efforts":["low","high"],"default_effort":"high"}
        }]})).unwrap();
        let snapshot = snapshot_from_models(&provider(), Some(&models.data), None);
        let descriptor = snapshot
            .descriptors
            .iter()
            .find(|model| model.id == "google/future-gemini")
            .unwrap();
        assert!(descriptor.capabilities.tool_calling);
        assert!(descriptor.capabilities.vision);
        assert!(descriptor.capabilities.structured_output);
        assert_eq!(descriptor.limits.context_tokens, Some(1048576));
        assert_eq!(descriptor.limits.max_output_tokens, Some(65536));
        let reasoning = descriptor.capabilities.reasoning.as_ref().unwrap();
        assert_eq!(reasoning.mode.as_deref(), Some("always"));
        assert_eq!(reasoning.default_enabled, Some(true));
        assert_eq!(reasoning.effort_levels, ["low", "high"]);
        assert!(
            reasoning.thinking_budget.is_none(),
            "unadvertised controls are not invented"
        );
        let entry = snapshot
            .models
            .iter()
            .find(|entry| entry.id == descriptor.id)
            .unwrap();
        assert_eq!(entry.supports_tools, Some(true));
        assert_eq!(entry.reasoning_efforts, ["low", "high"]);
    }

    #[test]
    fn openrouter_catalog_filters_batch_and_nonchat_models() {
        let models: ModelsResponse = serde_json::from_value(serde_json::json!({"data":[
            {"id":"chat","endpoints":["/api/v1/chat/completions"]},
            {"id":"batch:batch","endpoints":["/api/v1/batches"]},
            {"id":"responses-only","endpoints":["/api/v1/responses"]},
            {"id":"image","architecture":{"output_modalities":["image"]}}
        ]}))
        .unwrap();
        assert_eq!(
            models
                .data
                .iter()
                .filter(|model| model.supports_chat())
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["chat"]
        );
    }

    #[test]
    fn openrouter_catalog_missing_metadata_preserves_curated_facts() {
        let provider = provider();
        let fallback = snapshot_from_models(&provider, None, None);
        let models: ModelsResponse =
            serde_json::from_value(serde_json::json!({"data":[{"id":"google/gemini-4-carbon"}]}))
                .unwrap();
        let live = snapshot_from_models(&provider, Some(&models.data), None);
        let previous = fallback
            .descriptors
            .iter()
            .find(|model| model.id == models.data[0].id)
            .unwrap();
        let current = live
            .descriptors
            .iter()
            .find(|model| model.id == models.data[0].id)
            .unwrap();
        assert_eq!(current.capabilities, previous.capabilities);
        assert_eq!(current.limits, previous.limits);
        let withdrawn: ModelsResponse = serde_json::from_value(serde_json::json!({"data":[{"id":"google/gemini-4-carbon","supported_parameters":["tools"]}]})).unwrap();
        let withdrawn = snapshot_from_models(&provider, Some(&withdrawn.data), None);
        assert!(withdrawn
            .descriptors
            .iter()
            .find(|model| model.id == "google/gemini-4-carbon")
            .unwrap()
            .capabilities
            .reasoning
            .is_none());
    }

    #[tokio::test]
    async fn openrouter_catalog_coalesces_pages_and_retains_last_good_after_failure() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut provider = provider();
        provider.config.base_url = Some(format!("http://{address}/v1"));
        let server = tokio::spawn(async move {
            for attempt in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&chunk[..count]);
                }
                let (status, body) = match attempt {
                    0 => (
                        "200 OK",
                        serde_json::json!({"data":[{"id":"catalog-page-one","supported_parameters":["tools"]}],"total_count":2,"links":{"next":"/v1/models?offset=1"}}),
                    ),
                    1 => {
                        assert!(String::from_utf8_lossy(&request)
                            .starts_with("GET /v1/models?offset=1 "));
                        (
                            "200 OK",
                            serde_json::json!({"data":[{"id":"catalog-page-two","supported_parameters":["tools"]}],"total_count":2}),
                        )
                    }
                    _ => (
                        "503 Service Unavailable",
                        serde_json::json!({"error":{"message":"catalog temporarily unavailable"}}),
                    ),
                };
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let (first, concurrent) = tokio::join!(
            provider.openrouter_catalog_snapshot(),
            provider.openrouter_catalog_snapshot()
        );
        let first = first.unwrap();
        assert!(first.live_discovery_succeeded);
        assert_eq!(first.refreshed_at, concurrent.unwrap().refreshed_at);
        assert!(first
            .descriptors
            .iter()
            .any(|model| model.id == "catalog-page-two" && model.capabilities.tool_calling));
        let cache = account_cache(&provider);
        cache.lock().await.checked_at = Some(Instant::now() - CATALOG_TTL);
        let stale = provider.openrouter_catalog_snapshot().await.unwrap();
        assert!(!stale.live_discovery_succeeded);
        assert_eq!(stale.refreshed_at, first.refreshed_at);
        assert!(stale
            .descriptors
            .iter()
            .any(|model| model.id == "catalog-page-two"));
        let cooldown = provider.openrouter_catalog_snapshot().await.unwrap();
        assert!(!cooldown.live_discovery_succeeded);
        server.await.unwrap();
        provider.config.api_key = Some("different-catalog-account".into());
        assert!(
            !Arc::ptr_eq(&cache, &account_cache(&provider)),
            "credentials isolate catalog caches"
        );
    }
}
