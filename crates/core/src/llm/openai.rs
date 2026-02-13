//! OpenAI-compatible LLM provider.
//!
//! Also used for DeepSeek, LM Studio, Azure OpenAI, and custom endpoints
//! that expose the same `/v1/chat/completions` interface.

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::error::CoreError;
use super::{
    streaming::parse_sse_stream, CompletionRequest, CompletionResponse, FinishReason,
    LlmProvider, Message, ProviderConfig, Role, StreamChunk, ToolCallRequest, ToolDefinition,
    Usage,
};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_TIMEOUT_SECS: u64 = 120;

// ---------------------------------------------------------------------------
// OpenAI API wire types — request
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OaiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<OaiStreamOptions>,
}

#[derive(Serialize)]
struct OaiStreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct OaiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCallOut>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct OaiToolCallOut {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OaiFunctionOut,
}

#[derive(Serialize)]
struct OaiFunctionOut {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OaiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OaiToolFunction,
}

#[derive(Serialize)]
struct OaiToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

// ---------------------------------------------------------------------------
// OpenAI API wire types — response
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OaiResponse {
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Deserialize)]
struct OaiChoice {
    message: OaiResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OaiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCallIn>>,
}

#[derive(Deserialize)]
struct OaiToolCallIn {
    id: String,
    function: OaiFunctionIn,
}

#[derive(Deserialize)]
struct OaiFunctionIn {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct OaiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(Deserialize)]
struct OaiErrorResponse {
    error: OaiErrorBody,
}

#[derive(Deserialize)]
struct OaiErrorBody {
    message: String,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn role_str(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn parse_finish_reason(s: &str) -> FinishReason {
    match s {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_calls" => FinishReason::ToolCalls,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

fn convert_message(msg: &Message) -> OaiMessage {
    let mut oai = OaiMessage {
        role: role_str(&msg.role).to_string(),
        content: Some(msg.content.clone()),
        tool_calls: None,
        tool_call_id: None,
    };

    // Assistant messages may carry tool-call requests.
    if let Some(ref calls) = msg.tool_calls {
        oai.tool_calls = Some(
            calls
                .iter()
                .map(|tc| OaiToolCallOut {
                    id: tc.id.clone(),
                    call_type: "function".to_string(),
                    function: OaiFunctionOut {
                        name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                    },
                })
                .collect(),
        );
        // OpenAI accepts null content when tool_calls is present.
        if msg.content.is_empty() {
            oai.content = None;
        }
    }

    // Tool-result messages carry the originating tool_call_id.
    if msg.role == Role::Tool {
        oai.tool_call_id = msg.name.clone();
    }

    oai
}

fn convert_tools(tools: &[ToolDefinition]) -> Vec<OaiTool> {
    tools
        .iter()
        .map(|t| OaiTool {
            tool_type: "function".to_string(),
            function: OaiToolFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            },
        })
        .collect()
}

fn build_request_body(request: &CompletionRequest, stream: bool) -> OaiRequest {
    OaiRequest {
        model: request.model.clone(),
        messages: request.messages.iter().map(convert_message).collect(),
        temperature: request.temperature,
        max_tokens: request.max_tokens,
        tools: request.tools.as_ref().map(|t| convert_tools(t)),
        stop: request.stop.clone(),
        stream: if stream { Some(true) } else { None },
        stream_options: if stream {
            Some(OaiStreamOptions {
                include_usage: true,
            })
        } else {
            None
        },
    }
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

/// OpenAI-compatible LLM provider.
pub struct OpenAiProvider {
    client: reqwest::Client,
    config: ProviderConfig,
}

impl OpenAiProvider {
    /// Create a new provider with an async reqwest client.
    pub fn new(config: ProviderConfig) -> Result<Self, CoreError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .map_err(|e| CoreError::Llm(format!("Failed to create HTTP client: {e}")))?;

        Ok(Self { client, config })
    }

    fn base_url(&self) -> &str {
        self.config
            .base_url
            .as_deref()
            .unwrap_or(DEFAULT_BASE_URL)
    }

    fn api_key(&self) -> Result<&str, CoreError> {
        self.config
            .api_key
            .as_deref()
            .ok_or_else(|| CoreError::Llm("API key not configured".to_string()))
    }

    /// Check HTTP status and convert error responses into `CoreError`.
    async fn check_response(
        &self,
        response: reqwest::Response,
    ) -> Result<reqwest::Response, CoreError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        // 429 → rate-limited with optional Retry-After header.
        if status.as_u16() == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60);
            return Err(CoreError::RateLimited {
                retry_after_secs: retry_after,
            });
        }

        // Try to extract the structured error message.
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<OaiErrorResponse>(&body)
            .map(|e| e.error.message)
            .unwrap_or_else(|_| format!("HTTP {status}: {body}"));

        Err(CoreError::Llm(message))
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        let url = format!("{}/models", self.base_url());
        let api_key = self.api_key()?;

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
            .await
            .map_err(|e| CoreError::Llm(format!("Request failed: {e}")))?;

        let response = self.check_response(response).await?;

        let models: ModelsResponse = response
            .json()
            .await
            .map_err(|e| CoreError::Llm(format!("Failed to parse models response: {e}")))?;

        Ok(models.data.into_iter().map(|m| m.id).collect())
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, CoreError> {
        let url = format!("{}/chat/completions", self.base_url());
        let api_key = self.api_key()?;
        let body = build_request_body(request, false);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::Llm(format!("Request failed: {e}")))?;

        let response = self.check_response(response).await?;

        let oai: OaiResponse = response
            .json()
            .await
            .map_err(|e| CoreError::Llm(format!("Failed to parse completion response: {e}")))?;

        let choice = oai
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::Llm("No choices in response".to_string()))?;

        let tool_calls = choice.message.tool_calls.map(|tcs| {
            tcs.into_iter()
                .map(|tc| ToolCallRequest {
                    id: tc.id,
                    name: tc.function.name,
                    arguments: tc.function.arguments,
                })
                .collect()
        });

        let finish_reason = choice
            .finish_reason
            .as_deref()
            .map(parse_finish_reason)
            .unwrap_or(FinishReason::Other);

        let usage = oai
            .usage
            .map(|u| Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            })
            .unwrap_or_default();

        Ok(CompletionResponse {
            content: choice.message.content.unwrap_or_default(),
            tool_calls,
            finish_reason,
            usage,
        })
    }

    async fn stream(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, Result<StreamChunk, CoreError>>, CoreError> {
        let url = format!("{}/chat/completions", self.base_url());
        let api_key = self.api_key()?;
        let body = build_request_body(request, true);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::Llm(format!("Request failed: {e}")))?;

        let response = self.check_response(response).await?;

        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            if let Err(e) = parse_sse_stream(response, tx.clone()).await {
                let _ = tx.send(Err(e)).await;
            }
        });

        let stream =
            futures::stream::unfold(rx, |mut rx| async { rx.recv().await.map(|item| (item, rx)) });

        Ok(Box::pin(stream))
    }

    async fn health_check(&self) -> Result<(), CoreError> {
        self.list_models().await?;
        Ok(())
    }
}
