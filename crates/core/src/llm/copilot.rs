//! GitHub Copilot's account-scoped API, using Nexa's existing protocol codecs.
//! Authentication belongs to this saved provider; CLI/ACP sessions remain separate.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use super::{
    provider_turn::RouteSnapshot, reasoning_profile::ReasoningApiStyle, CompletionRequest,
    CompletionResponse, LlmProvider, ProviderConfig, ProviderStreamEvent, ProviderType,
    ReplayHistoryProjection, Role,
};
use crate::{error::CoreError, provider_catalog::ProviderModelCatalogSnapshot};

pub const DEFAULT_BASE_URL: &str = "https://api.githubcopilot.com";

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Deserialize)]
pub(super) struct Model {
    pub id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    supported_endpoints: Vec<String>,
    #[serde(default)]
    capabilities: Capabilities,
    #[serde(default)]
    model_picker_enabled: bool,
    #[serde(default)]
    is_chat_default: bool,
    #[serde(default)]
    policy: Option<ModelPolicy>,
    #[serde(skip)]
    account_scope: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ModelPolicy {
    state: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Capabilities {
    #[serde(default)]
    supports: Supports,
    #[serde(default)]
    limits: Limits,
    #[serde(default, rename = "type")]
    kind: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Supports {
    #[serde(default)]
    reasoning_effort: Vec<String>,
    #[serde(default)]
    adaptive_thinking: bool,
    #[serde(default)]
    thinking: bool,
    #[serde(default)]
    vision: bool,
    #[serde(default)]
    tool_calls: bool,
    #[serde(default)]
    parallel_tool_calls: bool,
    #[serde(default)]
    structured_outputs: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Limits {
    max_context_window_tokens: Option<u64>,
    max_prompt_tokens: Option<u64>,
    max_output_tokens: Option<u32>,
    min_thinking_budget: Option<u32>,
    max_thinking_budget: Option<u32>,
}

impl Model {
    fn available(&self) -> bool {
        self.model_picker_enabled
            && self.capabilities.kind == "chat"
            && self
                .policy
                .as_ref()
                .is_none_or(|policy| policy.state == "enabled")
            && self.supported_endpoints.iter().any(|s| {
                matches!(
                    s.as_str(),
                    "/v1/messages" | "/responses" | "/chat/completions"
                )
            })
    }
    pub(super) fn api_style(&self) -> ReasoningApiStyle {
        if self.supported_endpoints.iter().any(|s| s == "/v1/messages") {
            ReasoningApiStyle::AnthropicMessages
        } else if self.supported_endpoints.iter().any(|s| s == "/responses") {
            ReasoningApiStyle::OpenAiResponses
        } else {
            ReasoningApiStyle::OpenAiChatCompletions
        }
    }

    pub(super) fn route(
        &self,
        config: &ProviderConfig,
        request: &CompletionRequest,
    ) -> RouteSnapshot {
        let profile = super::reasoning_profile::resolve_reasoning_profile(
            config.provider_type,
            config.base_url.as_deref(),
            self.api_style(),
            &self.id,
        );
        let mut route = RouteSnapshot::from_profile_for_request(&profile, request);
        if self.api_style() == ReasoningApiStyle::OpenAiChatCompletions
            && self.id.starts_with("gemini-")
        {
            route.replay_policy = super::reasoning_profile::ReasoningReplayPolicy::OpaqueSignature;
        }
        route.provider_endpoint_id.push(':');
        route.provider_endpoint_id.push_str(&self.account_scope);
        route
    }

    fn resolve_request(&self, request: &CompletionRequest) -> Result<CompletionRequest, CoreError> {
        let mut request = request.clone();
        request.provider_type = Some(ProviderType::GithubCopilotApi);
        let supports = &self.capabilities.supports;
        if let Some(effort) = request.reasoning_effort.as_ref() {
            let effort = serde_json::to_value(effort)?
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if !supports.reasoning_effort.contains(&effort) {
                return Err(CoreError::InvalidInput(format!(
                    "GitHub Copilot model '{}' does not advertise reasoning effort '{effort}'",
                    self.id
                )));
            }
        }
        if let Some(max) = self.capabilities.limits.max_output_tokens {
            if request.max_tokens.is_some_and(|tokens| tokens > max) {
                return Err(CoreError::InvalidInput(format!(
                    "GitHub Copilot model '{}' permits at most {max} output tokens",
                    self.id
                )));
            }
            request.max_tokens.get_or_insert(max);
        }
        if let Some(budget) = request.thinking_budget {
            let limits = &self.capabilities.limits;
            if !supports.thinking
                || supports.adaptive_thinking
                || limits.min_thinking_budget.is_some_and(|min| budget < min)
                || limits.max_thinking_budget.is_some_and(|max| budget > max)
            {
                return Err(CoreError::InvalidInput(format!(
                    "Unsupported thinking budget for GitHub Copilot model '{}'",
                    self.id
                )));
            }
        }
        Ok(request)
    }

    /// Apply only Copilot's advertised controls; generic codec heuristics do not
    /// get to translate an explicit effort or rename this account's model ID.
    pub(super) fn wire_body<T: serde::Serialize>(
        &self,
        body: &T,
        request: &CompletionRequest,
        config: &ProviderConfig,
    ) -> Result<bytes::Bytes, CoreError> {
        let mut body = serde_json::to_value(body)?;
        let object = body
            .as_object_mut()
            .ok_or_else(|| CoreError::Llm("Invalid Copilot body".into()))?;
        let supports = &self.capabilities.supports;
        let disabled = request.reasoning_enabled == Some(false);
        let effort = request
            .reasoning_effort
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        object.insert("model".into(), json!(self.id));
        match self.api_style() {
            ReasoningApiStyle::AnthropicMessages => {
                object.remove("thinking");
                object.remove("output_config");
                if !disabled && supports.adaptive_thinking {
                    object.insert(
                        "thinking".into(),
                        json!({"type":"adaptive", "display":"summarized"}),
                    );
                } else if !disabled && supports.thinking {
                    if let Some(budget) = request.thinking_budget {
                        object.insert(
                            "thinking".into(),
                            json!({"type":"enabled", "budget_tokens":budget}),
                        );
                    }
                }
                if !disabled {
                    if let Some(effort) = effort {
                        object.insert("output_config".into(), json!({"effort":effort}));
                    }
                }
            }
            ReasoningApiStyle::OpenAiResponses => {
                object.remove("reasoning");
                if !disabled && !supports.reasoning_effort.is_empty() {
                    let mut reasoning = json!({"summary":"auto"});
                    if let Some(effort) = effort {
                        reasoning["effort"] = effort;
                    }
                    object.insert("reasoning".into(), reasoning);
                }
                object.insert("store".into(), json!(false));
            }
            _ => {
                object.remove("reasoning_effort");
                object.remove("reasoning");
                if !disabled {
                    if let Some(effort) = effort {
                        object.insert("reasoning_effort".into(), effort);
                    }
                }
                if let Some(messages) = object.get_mut("messages").and_then(Value::as_array_mut) {
                    let route = self.route(config, request);
                    for (wire, source) in messages.iter_mut().zip(&request.messages) {
                        let Some(envelope) = source
                            .provider_turn()
                            .filter(|e| e.is_compatible_with(&route))
                        else {
                            continue;
                        };
                        if let super::provider_turn::ProviderReplayPayload::CopilotReasoning {
                            opaque,
                            text,
                        } = &envelope.replay_payload
                        {
                            if let Some(opaque) = opaque {
                                wire["reasoning_opaque"] = json!(opaque);
                            }
                            if let Some(text) = text {
                                wire["reasoning_text"] = json!(text);
                            }
                        }
                    }
                }
            }
        }
        if !disabled
            && (supports.thinking
                || supports.adaptive_thinking
                || !supports.reasoning_effort.is_empty())
        {
            object.remove("temperature");
            object.remove("top_p");
        }
        super::serialized_json_body(&body, "GitHub Copilot request")
    }
}

pub(super) fn headers(
    builder: reqwest::RequestBuilder,
    request: Option<&CompletionRequest>,
) -> reqwest::RequestBuilder {
    // A tool continuation is agent initiated even if it contains older user messages.
    let user_initiated = request.is_none_or(|r| {
        r.messages
            .iter()
            .rev()
            .find(|m| m.role != Role::System)
            .is_none_or(|m| m.role == Role::User)
    });
    builder
        .header(
            "Editor-Version",
            concat!("Nexa/", env!("CARGO_PKG_VERSION")),
        )
        .header("User-Agent", concat!("Nexa/", env!("CARGO_PKG_VERSION")))
        .header("Copilot-Integration-Id", "vscode-chat")
        .header("X-Github-Api-Version", "2025-10-01")
        .header("X-Initiator", if user_initiated { "user" } else { "agent" })
        .header("X-Interaction-Type", "conversation-agent")
        .header("OpenAI-Intent", "conversation-agent")
}

struct Catalog {
    models: Vec<Arc<Model>>,
    adapters: HashMap<String, Box<dyn LlmProvider>>,
}

pub struct CopilotProvider {
    config: ProviderConfig,
    catalog: OnceCell<Catalog>,
}

impl CopilotProvider {
    pub fn new(mut config: ProviderConfig) -> Result<Self, CoreError> {
        config.base_url = super::normalize_base_url(config.base_url.take());
        if !super::provider_boundary::is_copilot_endpoint(config.base_url.as_deref()) {
            return Err(CoreError::InvalidInput(
                "GitHub Copilot requires an official HTTPS API endpoint".into(),
            ));
        }
        if config
            .api_key
            .as_deref()
            .is_none_or(|s| s.trim().is_empty())
        {
            return Err(CoreError::InvalidInput(
                "GitHub Copilot account token is not configured".into(),
            ));
        }
        // Child codecs have their own public API defaults. Never let an absent
        // Copilot URL fall through to those defaults after optional discovery.
        config
            .base_url
            .get_or_insert_with(|| DEFAULT_BASE_URL.into());
        Ok(Self {
            config,
            catalog: OnceCell::new(),
        })
    }

    async fn catalog(&self) -> Result<&Catalog, CoreError> {
        self.catalog.get_or_try_init(|| self.discover()).await
    }

    async fn discover(&self) -> Result<Catalog, CoreError> {
        crate::privacy::runtime::ensure_invocation_current()?;
        let transport = super::transport::shared_http_transport(&self.config)?.for_request()?;
        let token = self.config.api_key.as_deref().unwrap_or_default();
        let mut config = self.config.clone();
        if config
            .base_url
            .as_deref()
            .is_none_or(|s| s == DEFAULT_BASE_URL)
        {
            let discovery = headers(
                transport.client().post("https://api.github.com/graphql"),
                None,
            )
            .bearer_auth(token)
            .json(&json!({"query":"query { viewer { copilotEndpoints { api } } }"}));
            let response =
                super::with_request_timeout(discovery, Some(std::time::Duration::from_secs(15)))
                    .send()
                    .await;
            if let Ok(response) = response {
                if response.status().is_success() {
                    if let Ok(value) = response.json::<Value>().await {
                        if let Some(endpoint) = value
                            .pointer("/data/viewer/copilotEndpoints/api")
                            .and_then(Value::as_str)
                        {
                            if !super::provider_boundary::is_copilot_endpoint(Some(endpoint)) {
                                return Err(CoreError::Llm(
                                    "GitHub returned an untrusted Copilot endpoint".into(),
                                ));
                            }
                            config.base_url = Some(endpoint.trim_end_matches('/').into());
                        }
                    }
                }
            }
        }
        let endpoint = config.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL);
        let request =
            headers(transport.client().get(format!("{endpoint}/models")), None).bearer_auth(token);
        let response = super::with_request_timeout(
            request,
            super::configured_request_timeout(config.timeout_secs.unwrap_or(60)),
        )
        .send()
        .await
        .map_err(|e| CoreError::TransientLlm(super::transport::describe_http_error(&e)))?;
        if !response.status().is_success() {
            return Err(CoreError::Llm(format!(
                "GitHub Copilot model discovery failed (HTTP {})",
                response.status()
            )));
        }
        #[derive(Deserialize)]
        struct Models {
            data: Vec<Model>,
        }
        let data: Models = response
            .json()
            .await
            .map_err(|e| CoreError::Llm(super::transport::describe_http_error(&e)))?;
        let scope = blake3::hash(token.as_bytes()).to_hex().to_string();
        let mut adapters: HashMap<String, Box<dyn LlmProvider>> = HashMap::new();
        let mut models = Vec::new();
        for mut model in data.data {
            if !model.available() {
                continue;
            }
            model.account_scope = scope.clone();
            let model = Arc::new(model);
            let adapter: Box<dyn LlmProvider> =
                if model.api_style() == ReasoningApiStyle::AnthropicMessages {
                    Box::new(
                        super::anthropic::AnthropicProvider::new(config.clone())?
                            .with_copilot_model(model.clone()),
                    )
                } else {
                    Box::new(
                        super::openai::OpenAiProvider::new(config.clone())?
                            .with_copilot_model(model.clone()),
                    )
                };
            adapters.insert(model.id.clone(), adapter);
            models.push(model);
        }
        if let Some(index) = models.iter().position(|m| m.is_chat_default) {
            let default = models.remove(index);
            models.insert(0, default);
        }
        Ok(Catalog { models, adapters })
    }

    async fn select(
        &self,
        request: &CompletionRequest,
    ) -> Result<(&dyn LlmProvider, CompletionRequest), CoreError> {
        let catalog = self.catalog().await?;
        let model = catalog
            .models
            .iter()
            .find(|m| m.id == request.model)
            .ok_or_else(|| {
                CoreError::InvalidInput(format!(
                    "Model '{}' is unavailable to this GitHub Copilot account",
                    request.model
                ))
            })?;
        let provider = catalog
            .adapters
            .get(&model.id)
            .ok_or_else(|| CoreError::Llm("Missing Copilot adapter".into()))?;
        let mut request = model.resolve_request(request)?;
        request.messages = super::reasoning_replay::prepare_provider_replay_history(
            &request.messages,
            &provider.route_snapshot(&request),
        )
        .messages;
        Ok((provider.as_ref(), request))
    }
}

#[async_trait]
impl LlmProvider for CopilotProvider {
    fn name(&self) -> &str {
        "GitHub Copilot API"
    }
    fn manages_request_timeout(&self) -> bool {
        true
    }
    fn stream_max_retries(&self) -> Option<u32> {
        self.config.streaming.stream_max_retries
    }
    fn replay_history_projection(&self, _: &CompletionRequest) -> ReplayHistoryProjection {
        ReplayHistoryProjection::ProviderSelectedRoute
    }
    fn route_snapshot(&self, request: &CompletionRequest) -> RouteSnapshot {
        self.catalog
            .get()
            .and_then(|c| c.adapters.get(&request.model))
            .map(|p| p.route_snapshot(request))
            .unwrap_or_else(|| {
                RouteSnapshot::unknown(
                    "github_copilot_api",
                    &request.model,
                    super::reasoning_profile::ReasoningReplayPolicy::Unknown,
                )
            })
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(self
            .catalog()
            .await?
            .models
            .iter()
            .map(|m| m.id.clone())
            .collect())
    }
    async fn model_catalog_snapshot(
        &self,
    ) -> Result<Option<ProviderModelCatalogSnapshot>, CoreError> {
        let catalog = self.catalog().await?;
        let mut snapshot = crate::provider_catalog::build_effective_model_catalog(
            "github_copilot_api",
            self.config.base_url.as_deref(),
            Some(self.list_models().await?),
            None,
            chrono::Utc::now().to_rfc3339(),
        );
        for entry in &mut snapshot.models {
            let Some(model) = catalog.models.iter().find(|m| m.id == entry.id) else {
                continue;
            };
            let s = &model.capabilities.supports;
            entry.name = if model.name.is_empty() {
                model.id.clone()
            } else {
                model.name.clone()
            };
            entry.recommended = model.is_chat_default;
            entry.supports_tools = Some(s.tool_calls);
            entry.supports_structured_output = Some(s.structured_outputs);
            entry.reasoning_efforts = s.reasoning_effort.clone();
            if s.vision {
                entry.modalities.push("image".into());
            }
            let reasoning = (!s.reasoning_effort.is_empty() || s.thinking || s.adaptive_thinking).then(|| json!({
                "mode": if model.api_style() == ReasoningApiStyle::OpenAiChatCompletions && model.id.starts_with("gemini-") { "always" } else { "optional" }, "effortLevels":s.reasoning_effort, "defaultEnabled":true,
                "effortBudgetExclusive":true,
                "thinkingBudget": if s.thinking && !s.adaptive_thinking { json!({"enabled":true, "minTokens":model.capabilities.limits.min_thinking_budget, "maxTokens":model.capabilities.limits.max_thinking_budget}) } else { Value::Null }
            }));
            entry.capabilities = Some(serde_json::from_value(
                json!({"vision":s.vision,"reasoning":reasoning}),
            )?);
            if let Some(descriptor) = snapshot.descriptors.iter_mut().find(|d| d.id == model.id) {
                descriptor.display_name = entry.name.clone();
                descriptor.recommended = model.is_chat_default;
                descriptor.input_modalities = vec![crate::model_catalog::ModelModality::Text];
                if s.vision {
                    descriptor
                        .input_modalities
                        .push(crate::model_catalog::ModelModality::Image);
                }
                descriptor.capabilities.vision = s.vision;
                descriptor.capabilities.tool_calling = s.tool_calls;
                descriptor.capabilities.parallel_tool_calling = s.parallel_tool_calls;
                descriptor.capabilities.structured_output = s.structured_outputs;
                descriptor.capabilities.reasoning =
                    reasoning.map(serde_json::from_value).transpose()?;
                descriptor.limits.context_tokens = model
                    .capabilities
                    .limits
                    .max_prompt_tokens
                    .or(model.capabilities.limits.max_context_window_tokens);
                descriptor.limits.max_output_tokens =
                    model.capabilities.limits.max_output_tokens.map(u64::from);
            }
        }
        Ok(Some(snapshot))
    }
    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        Ok(self.complete_with_route(request).await?.0)
    }
    async fn complete_with_route(
        &self,
        request: &CompletionRequest,
    ) -> Result<(CompletionResponse, RouteSnapshot), CoreError> {
        let (provider, request) = self.select(request).await?;
        provider.complete_with_route(&request).await
    }
    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let (provider, request) = self.select(request).await?;
        provider.stream_events(&request).await
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        self.catalog().await?;
        Ok(())
    }
}
