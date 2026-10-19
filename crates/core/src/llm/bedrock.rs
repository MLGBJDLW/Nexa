//! Amazon Bedrock Converse/ConverseStream using Bedrock bearer API keys.
//! AWS access-key credentials are deliberately not interpreted as bearer keys.
use async_trait::async_trait;
use futures::stream::BoxStream;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;

use super::{
    provider_turn::{BedrockAssistantReplay, ProviderReplayPayload, RouteSnapshot},
    reasoning_profile::{ReasoningApiStyle, ReasoningReplayPolicy},
    transport::{shared_http_transport, HttpTransport},
    *,
};
use crate::error::CoreError;

mod event_stream;

pub struct BedrockProvider {
    config: ProviderConfig,
    transport: Arc<HttpTransport>,
    timeout: Option<Duration>,
}

impl BedrockProvider {
    pub fn new(config: ProviderConfig) -> Result<Self, CoreError> {
        let transport = shared_http_transport(&config)?;
        let timeout = configured_request_timeout(config.timeout_secs.unwrap_or(120));
        Ok(Self {
            config,
            transport,
            timeout,
        })
    }
    fn base_url(&self) -> &str {
        self.config
            .base_url
            .as_deref()
            .unwrap_or("https://bedrock-runtime.us-east-1.amazonaws.com")
    }
    fn credential(&self) -> Result<&str, CoreError> {
        self.config
            .api_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| {
                CoreError::InvalidInput(
                    "Configure an Amazon Bedrock API key (bearer token), not an AWS access key ID."
                        .into(),
                )
            })
    }
    fn url(&self, model: &str, streaming: bool) -> Result<reqwest::Url, CoreError> {
        let mut url = reqwest::Url::parse(self.base_url())
            .map_err(|_| CoreError::InvalidInput("Invalid Bedrock Runtime base URL".into()))?;
        url.path_segments_mut()
            .map_err(|_| CoreError::InvalidInput("Invalid Bedrock Runtime base URL".into()))?
            .pop_if_empty()
            .push("model")
            .push(model)
            .push(if streaming {
                "converse-stream"
            } else {
                "converse"
            });
        Ok(url)
    }
    async fn check(&self, response: reqwest::Response) -> Result<reqwest::Response, CoreError> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        let message =
            crate::sensitive_data::sanitize_diagnostic(&body, self.config.api_key.as_deref());
        if status.as_u16() == 429 {
            return Err(CoreError::RateLimited {
                retry_after_secs: 30,
            });
        }
        if status.is_server_error() {
            return Err(CoreError::ProviderUnavailable {
                status: status.as_u16(),
                message,
            });
        }
        Err(CoreError::Llm(format!("Bedrock HTTP {status}: {message}")))
    }
}

fn body(request: &CompletionRequest, route: &RouteSnapshot) -> Result<Value, CoreError> {
    let system: Vec<_> = request
        .messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(Message::text_content)
        .filter(|text| !text.is_empty())
        .map(|text| json!({"text":text}))
        .collect();
    let tool_config = request.tools.as_ref().filter(|tools| !tools.is_empty()).map(|tools| json!({"tools": tools.iter().map(|tool| json!({"toolSpec":{"name":tool.name,"description":tool.description,"inputSchema":{"json":if request.model.contains("moonshotai.kimi-") { super::moonshot_schema::project_tool_parameters(&tool.parameters) } else { tool.parameters.clone() }}}})).collect::<Vec<_>>()}));
    let mut messages: Vec<Value> = Vec::new();
    for message in &request.messages {
        if message.role == Role::System {
            continue;
        }
        let mut content = Vec::new();
        let replay = message
            .provider_turn()
            .filter(|envelope| envelope.route.same_route_identity(route))
            .and_then(|envelope| match &envelope.replay_payload {
                ProviderReplayPayload::BedrockAssistantBlocks(payload)
                    if envelope.replay_payload.is_present() =>
                {
                    Some(payload)
                }
                _ => None,
            });
        if let Some(replay) = replay {
            content = replay.content.clone();
            let disabled_bound_model = (request.model.contains("claude-sonnet-5-5")
                || request.model.contains("claude-haiku-5-5"))
                && (request.reasoning_enabled == Some(false)
                    || request.reasoning_effort == Some(ReasoningEffort::None));
            if disabled_bound_model
                && replay.request_prefix != prefix(&system, tool_config.as_ref(), &messages)
            {
                content.retain(|block| block.get("reasoningContent").is_none());
            }
        } else {
            for part in &message.parts {
                match part {
                    ContentPart::Text { text } if !text.is_empty() => content.push(json!({"text":text})),
                    ContentPart::Image { media_type, data } if message.role == Role::User || (message.role == Role::Tool && request.model.contains("anthropic.claude-")) => {
                        let format = media_type.strip_prefix("image/").filter(|format| matches!(*format,"png"|"jpeg"|"gif"|"webp"))
                            .ok_or_else(|| CoreError::InvalidInput("Bedrock supports PNG, JPEG, GIF and WebP image inputs".into()))?;
                        content.push(json!({"image":{"format":format,"source":{"bytes":data}}}));
                    }
                    ContentPart::Image { .. } => content.push(json!({"text":"[Image tool result omitted on this Bedrock route; use the tool's textual result.]"})),
                    ContentPart::Document { document } => content.push(json!({"text":document.fallback("Bedrock local extraction")})),
                    _ => {}
                }
            }
            if let Some(calls) = &message.tool_calls {
                for call in calls {
                    let input: Value = serde_json::from_str(&call.arguments).map_err(|_| {
                        CoreError::InvalidInput("Invalid tool arguments in Bedrock history".into())
                    })?;
                    content.push(
                        json!({"toolUse":{"toolUseId":call.id,"name":call.name,"input":input}}),
                    );
                }
            }
        }
        if content.is_empty() {
            content.push(json!({"text":"(empty)"}));
        }
        let role = if message.role == Role::Assistant {
            "assistant"
        } else {
            "user"
        };
        if message.role == Role::Tool {
            let id = message
                .name
                .as_deref()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    CoreError::InvalidInput("Bedrock tool result is missing its call ID".into())
                })?;
            content = vec![json!({"toolResult":{"toolUseId":id,"content":content}})];
        }
        if let Some(last) = messages.last_mut().filter(|last| last["role"] == role) {
            last["content"].as_array_mut().unwrap().extend(content);
        } else {
            messages.push(json!({"role":role,"content":content}));
        }
    }
    let mut body = json!({"messages":messages});
    if !system.is_empty() {
        body["system"] = system.into();
    }
    let mut inference = serde_json::Map::new();
    if let Some(tokens) = request.max_tokens {
        inference.insert("maxTokens".into(), tokens.into());
    }
    if let Some(temperature) = request.temperature {
        inference.insert("temperature".into(), json!(temperature));
    }
    if let Some(stop) = &request.stop {
        inference.insert("stopSequences".into(), json!(stop));
    }
    if !inference.is_empty() {
        body["inferenceConfig"] = inference.into();
    }
    if let Some(tool_config) = tool_config {
        body["toolConfig"] = tool_config;
    }
    if let Some((_, model)) = request.model.split_once("anthropic.") {
        let mut native = request.clone();
        native.model = model.into();
        let fields = super::anthropic::cloud_reasoning_fields(&native)?;
        if fields.as_object().is_some_and(|fields| !fields.is_empty()) {
            body["additionalModelRequestFields"] = fields;
            if let Some(config) = body["inferenceConfig"].as_object_mut() {
                config.remove("temperature");
            }
        }
    } else if request.model.contains("openai.gpt-") {
        if let Some(effort) = request
            .reasoning_effort
            .as_ref()
            .filter(|effort| **effort != ReasoningEffort::None)
        {
            body["additionalModelRequestFields"]["reasoning"]["effort"] =
                serde_json::to_value(effort)?;
        }
        if let Some(config) = body["inferenceConfig"].as_object_mut() {
            config.remove("temperature");
        }
    }
    Ok(body)
}

fn prefix(system: &[Value], tool_config: Option<&Value>, messages: &[Value]) -> String {
    let encoded =
        serde_json::to_vec(&json!({"system":system,"toolConfig":tool_config,"messages":messages}))
            .expect("JSON prompt is serializable");
    blake3::hash(&encoded).to_hex().to_string()
}
fn request_prefix(body: &Value) -> String {
    prefix(
        body["system"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default(),
        body.get("toolConfig"),
        body["messages"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default(),
    )
}
pub(crate) fn signed_reasoning_complete(blocks: &[Value]) -> bool {
    blocks.iter().all(|block| {
        block.get("reasoningContent").is_none_or(|reasoning| {
            reasoning.get("redactedContent").is_some()
                || reasoning["reasoningText"]["signature"]
                    .as_str()
                    .is_some_and(|signature| !signature.is_empty())
        })
    })
}

fn finish_reason(value: &str) -> FinishReason {
    match value {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "tool_use" => FinishReason::ToolCalls,
        "max_tokens" => FinishReason::Length,
        "guardrail_intervened" | "content_filtered" => FinishReason::ContentFilter,
        "model_context_window_exceeded" => FinishReason::ContextLimit,
        "malformed_tool_use" | "malformed_model_output" => FinishReason::MalformedToolCall,
        other => FinishReason::Unknown(other.into()),
    }
}
fn usage(value: &Value) -> Usage {
    let tokens = |key: &str| value[key].as_u64().and_then(|n| u32::try_from(n).ok());
    let prompt = tokens("inputTokens").unwrap_or(0);
    let completion = tokens("outputTokens").unwrap_or(0);
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: tokens("totalTokens").unwrap_or(prompt.saturating_add(completion)),
        cache_read_tokens: tokens("cacheReadInputTokens"),
        cache_creation_tokens: tokens("cacheWriteInputTokens"),
        ..Usage::default()
    }
}

pub(crate) fn valid_replay(blocks: &[Value], calls: Option<&[ToolCallRequest]>) -> bool {
    if blocks.is_empty() {
        return false;
    }
    let mut native = Vec::new();
    let mut seen = HashSet::new();
    for block in blocks {
        if block.as_object().is_none_or(|object| object.len() != 1) {
            return false;
        }
        if let Some(tool) = block.get("toolUse") {
            let (Some(id), Some(name), Some(input)) = (
                tool["toolUseId"].as_str(),
                tool["name"].as_str(),
                tool["input"].as_object(),
            ) else {
                return false;
            };
            if id.is_empty() || name.is_empty() || !seen.insert(id) {
                return false;
            }
            native.push((id, name, input));
        } else if let Some(reasoning) = block.get("reasoningContent") {
            let signed = reasoning["reasoningText"]["text"].is_string()
                && reasoning["reasoningText"]
                    .get("signature")
                    .is_none_or(Value::is_string);
            let redacted = reasoning["redactedContent"]
                .as_str()
                .is_some_and(|s| !s.is_empty());
            if !signed && !redacted {
                return false;
            }
        } else if !block["text"].is_string() {
            return false;
        }
    }
    calls.is_none_or(|calls| {
        native.len() == calls.len()
            && native.iter().zip(calls).all(|((id, name, input), call)| {
                *id == call.id
                    && *name == call.name
                    && serde_json::from_str::<Value>(&call.arguments)
                        .ok()
                        .as_ref()
                        .and_then(Value::as_object)
                        == Some(*input)
            })
    })
}

fn response(
    value: Value,
    request_prefix: String,
    signed_reasoning: bool,
) -> Result<CompletionResponse, CoreError> {
    let blocks = value["output"]["message"]["content"]
        .as_array()
        .ok_or_else(|| {
            CoreError::StreamIncomplete("Bedrock response has no assistant content".into())
        })?
        .clone();
    let stop = value["stopReason"]
        .as_str()
        .ok_or_else(|| CoreError::StreamIncomplete("Bedrock response has no stopReason".into()))?;
    let empty_filter = blocks.is_empty() && finish_reason(stop) == FinishReason::ContentFilter;
    if !empty_filter
        && (!valid_replay(&blocks, None)
            || (signed_reasoning && !signed_reasoning_complete(&blocks)))
    {
        return Err(CoreError::StreamIncomplete(
            "Bedrock response contains incomplete native content".into(),
        ));
    }
    let mut content = String::new();
    let mut thinking = String::new();
    let mut calls = Vec::new();
    for block in &blocks {
        if let Some(text) = block["text"].as_str() {
            content.push_str(text);
        }
        if let Some(text) = block["reasoningContent"]["reasoningText"]["text"].as_str() {
            thinking.push_str(text);
        }
        if let Some(tool) = block.get("toolUse") {
            calls.push(ToolCallRequest {
                id: tool["toolUseId"].as_str().unwrap().into(),
                name: tool["name"].as_str().unwrap().into(),
                arguments: tool["input"].to_string(),
                thought_signature: None,
            });
        }
    }
    Ok(CompletionResponse {
        content,
        tool_calls: (!calls.is_empty()).then_some(calls),
        finish_reason: finish_reason(stop),
        usage: usage(&value["usage"]),
        thinking: (!thinking.is_empty()).then_some(thinking),
        provider_replay: (!blocks.is_empty()).then_some(
            ProviderReplayPayload::BedrockAssistantBlocks(BedrockAssistantReplay {
                content: blocks,
                request_prefix,
            }),
        ),
    })
}

#[derive(Default)]
struct Block {
    content: Value,
    arguments: String,
    stopped: bool,
}
#[derive(Default)]
struct StreamState {
    started: bool,
    blocks: BTreeMap<u32, Block>,
    stop: Option<FinishReason>,
    usage: Option<Usage>,
}
impl StreamState {
    fn event(&mut self, kind: &str, value: Value) -> Result<Option<StreamChunk>, CoreError> {
        let mut chunk = StreamChunk {
            delta: String::new(),
            tool_call_delta: None,
            finish_reason: None,
            usage: None,
            thinking_delta: None,
        };
        if kind.ends_with("Exception") || kind == "error" {
            return Err(CoreError::Llm(format!(
                "Bedrock stream {kind}: {}",
                value["message"]
                    .as_str()
                    .unwrap_or("provider stream failed")
            )));
        }
        match kind {
            "messageStart" if !self.started && value["role"] == "assistant" => self.started = true,
            "contentBlockStart" | "contentBlockDelta" | "contentBlockStop"
                if self.started && self.stop.is_none() =>
            {
                let index = value["contentBlockIndex"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| {
                        CoreError::StreamIncomplete("Bedrock block is missing an index".into())
                    })?;
                let block = self.blocks.entry(index).or_default();
                if block.stopped {
                    return Err(CoreError::StreamIncomplete(
                        "Bedrock reused a closed content block".into(),
                    ));
                }
                if kind == "contentBlockStart" {
                    let tool = &value["start"]["toolUse"];
                    if !block.content.is_null()
                        || !tool["toolUseId"].is_string()
                        || !tool["name"].is_string()
                    {
                        return Err(CoreError::StreamIncomplete(
                            "Invalid Bedrock tool start".into(),
                        ));
                    }
                    block.content = json!({"toolUse":tool});
                    chunk.tool_call_delta = Some(ToolCallDelta {
                        id: tool["toolUseId"].as_str().unwrap().into(),
                        name: Some(tool["name"].as_str().unwrap().into()),
                        arguments_delta: "".into(),
                        index: Some(index),
                        thought_signature: None,
                    });
                } else if kind == "contentBlockStop" {
                    if block.content.get("toolUse").is_some() {
                        block.content["toolUse"]["input"] = serde_json::from_str(&block.arguments)
                            .map_err(|_| {
                                CoreError::StreamIncomplete(
                                    "Incomplete Bedrock tool arguments".into(),
                                )
                            })?;
                    }
                    if !valid_replay(&[block.content.clone()], None) {
                        return Err(CoreError::StreamIncomplete(
                            "Incomplete Bedrock content block".into(),
                        ));
                    }
                    block.stopped = true;
                } else {
                    let delta = &value["delta"];
                    if let Some(text) = delta["text"].as_str() {
                        if block.content.is_null() {
                            block.content = json!({"text":""});
                        }
                        append(&mut block.content["text"], text)?;
                        chunk.delta = text.into();
                    } else if let Some(input) = delta["toolUse"]["input"].as_str() {
                        let tool = &block.content["toolUse"];
                        let id = tool["toolUseId"].as_str().ok_or_else(|| {
                            CoreError::StreamIncomplete(
                                "Bedrock tool delta preceded its start".into(),
                            )
                        })?;
                        block.arguments.push_str(input);
                        chunk.tool_call_delta = Some(ToolCallDelta {
                            id: id.into(),
                            name: None,
                            arguments_delta: input.into(),
                            index: Some(index),
                            thought_signature: None,
                        });
                    } else if let Some(reasoning) = delta.get("reasoningContent") {
                        if block.content.is_null() {
                            block.content = json!({"reasoningContent":{}});
                        }
                        if let Some(text) = reasoning["text"].as_str() {
                            append(
                                &mut block.content["reasoningContent"]["reasoningText"]["text"],
                                text,
                            )?;
                            chunk.thinking_delta = Some(text.into());
                        } else if let Some(signature) = reasoning["signature"].as_str() {
                            append(
                                &mut block.content["reasoningContent"]["reasoningText"]
                                    ["signature"],
                                signature,
                            )?;
                        } else if let Some(data) = reasoning["redactedContent"].as_str() {
                            use base64::Engine;
                            let codec = base64::engine::general_purpose::STANDARD;
                            let prior = block.content["reasoningContent"]["redactedContent"]
                                .as_str()
                                .unwrap_or("");
                            let mut bytes = codec.decode(prior).map_err(|_| {
                                CoreError::StreamIncomplete("Invalid redacted reasoning".into())
                            })?;
                            bytes.extend(codec.decode(data).map_err(|_| {
                                CoreError::StreamIncomplete("Invalid redacted reasoning".into())
                            })?);
                            block.content["reasoningContent"]["redactedContent"] =
                                codec.encode(bytes).into();
                        } else {
                            return Err(CoreError::StreamIncomplete(
                                "Unknown Bedrock reasoning delta".into(),
                            ));
                        }
                    } else {
                        return Err(CoreError::StreamIncomplete(
                            "Unknown Bedrock content delta".into(),
                        ));
                    }
                }
            }
            "messageStop"
                if self.started
                    && self.stop.is_none()
                    && self.blocks.values().all(|block| block.stopped) =>
            {
                self.stop = Some(finish_reason(value["stopReason"].as_str().ok_or_else(
                    || CoreError::StreamIncomplete("Bedrock omitted stopReason".into()),
                )?));
            }
            "metadata" if self.stop.is_some() && self.usage.is_none() => {
                self.usage = Some(usage(&value["usage"]));
            }
            _ => {
                return Err(CoreError::StreamIncomplete(format!(
                    "Unexpected Bedrock event: {kind}"
                )))
            }
        }
        Ok((!chunk.delta.is_empty()
            || chunk.tool_call_delta.is_some()
            || chunk.thinking_delta.is_some())
        .then_some(chunk))
    }
    fn terminal(
        self,
        request_prefix: String,
        signed_reasoning: bool,
    ) -> Result<(StreamChunk, ProviderReplayPayload), CoreError> {
        let stop = self.stop.ok_or_else(|| {
            CoreError::StreamIncomplete("Bedrock stream ended without messageStop".into())
        })?;
        let usage = self.usage.ok_or_else(|| {
            CoreError::StreamIncomplete("Bedrock stream ended without usage metadata".into())
        })?;
        let blocks: Vec<_> = self
            .blocks
            .into_values()
            .map(|block| block.content)
            .collect();
        let empty_filter = blocks.is_empty() && stop == FinishReason::ContentFilter;
        if !empty_filter
            && (!valid_replay(&blocks, None)
                || (signed_reasoning && !signed_reasoning_complete(&blocks)))
        {
            return Err(CoreError::StreamIncomplete(
                "Bedrock stream omitted native content or signature".into(),
            ));
        }
        Ok((
            StreamChunk {
                delta: String::new(),
                tool_call_delta: None,
                finish_reason: Some(stop),
                usage: Some(usage),
                thinking_delta: None,
            },
            if blocks.is_empty() {
                ProviderReplayPayload::None
            } else {
                ProviderReplayPayload::BedrockAssistantBlocks(BedrockAssistantReplay {
                    content: blocks,
                    request_prefix,
                })
            },
        ))
    }
}
fn append(value: &mut Value, fragment: &str) -> Result<(), CoreError> {
    if value.is_null() {
        *value = Value::String(String::new());
    }
    let Value::String(text) = value else {
        return Err(CoreError::StreamIncomplete(
            "Bedrock changed content block type".into(),
        ));
    };
    text.push_str(fragment);
    Ok(())
}

#[async_trait]
impl LlmProvider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }
    fn manages_request_timeout(&self) -> bool {
        true
    }
    fn stream_max_retries(&self) -> Option<u32> {
        self.config.streaming.stream_max_retries
    }
    fn reasoning_replay_policy(&self, _model: &str) -> ReasoningReplayPolicy {
        ReasoningReplayPolicy::NotRequired
    }
    fn route_snapshot(&self, request: &CompletionRequest) -> RouteSnapshot {
        RouteSnapshot {
            provider_endpoint_id: super::provider_boundary::endpoint_id(
                ProviderType::Bedrock,
                Some(self.base_url()),
            ),
            provider_family: "bedrock".into(),
            api_style: ReasoningApiStyle::BedrockConverse,
            model_id: request.model.clone(),
            reasoning_profile_id: "bedrock-converse-v1".into(),
            reasoning_profile_version: 1,
            replay_policy: ReasoningReplayPolicy::NotRequired,
        }
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        let transport = self.transport.for_request()?;
        let mut url = reqwest::Url::parse(self.base_url())
            .map_err(|_| CoreError::InvalidInput("Invalid Bedrock endpoint".into()))?;
        let host = url.host_str().unwrap_or_default().to_string();
        if let Some(region) = host.strip_prefix("bedrock-runtime.") {
            url.set_host(Some(&format!("bedrock.{region}")))
                .map_err(|_| CoreError::InvalidInput("Invalid Bedrock region".into()))?;
        }
        url.set_path("/foundation-models");
        let response = with_request_timeout(
            transport
                .client()
                .get(url)
                .bearer_auth(self.credential()?)
                .query(&[("byOutputModality", "TEXT")]),
            self.timeout,
        )
        .send()
        .await
        .map_err(|error| {
            super::transport::classify_http_error(&error, "Bedrock model discovery")
        })?;
        let value: Value = self.check(response).await?.json().await.map_err(|error| {
            super::transport::classify_http_error(&error, "Bedrock model discovery")
        })?;
        let items = value["modelSummaries"]
            .as_array()
            .ok_or_else(|| CoreError::Llm("Bedrock catalog has no modelSummaries array".into()))?;
        Ok(items
            .iter()
            .filter(|item| item["modelLifecycle"]["status"] != "EOL")
            .filter_map(|item| item["modelId"].as_str())
            .map(str::to_owned)
            .collect())
    }
    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        crate::privacy::runtime::ensure_invocation_current()?;
        let transport = self.transport.for_request()?;
        let body = body(request, &self.route_snapshot(request))?;
        let result = with_request_timeout(
            transport
                .client()
                .post(self.url(&request.model, false)?)
                .bearer_auth(self.credential()?)
                .json(&body),
            self.timeout,
        )
        .send()
        .await
        .map_err(|error| super::transport::classify_http_error(&error, "Bedrock Converse"))?;
        let value = self.check(result).await?.json().await.map_err(|error| {
            super::transport::classify_http_error(&error, "Bedrock Converse response")
        })?;
        response(
            value,
            request_prefix(&body),
            request.model.contains("anthropic.claude-"),
        )
    }
    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let transport = self.transport.for_request()?;
        let body = body(request, &self.route_snapshot(request))?;
        let response = send_stream_start_request(
            transport
                .client()
                .post(self.url(&request.model, true)?)
                .bearer_auth(self.credential()?)
                .json(&body),
            self.timeout,
            "Bedrock ConverseStream",
        )
        .await?;
        let response = self.check(response).await?;
        let idle = self.config.streaming.stream_idle_timeout();
        let request_prefix = request_prefix(&body);
        let signed_reasoning = request.model.contains("anthropic.claude-");
        let credential = self.credential()?.to_owned();
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let operation = async {
                let mut stream = response.bytes_stream();
                let mut decoder = event_stream::Decoder::default();
                let mut state = StreamState::default();
                while let Some(bytes) =
                    next_stream_item_with_idle_timeout(&mut stream, idle, "Bedrock stream").await?
                {
                    let bytes = bytes.map_err(|error| {
                        super::transport::classify_http_error(&error, "Bedrock stream")
                    })?;
                    for (kind, value) in decoder.push(&bytes)? {
                        if let Some(chunk) = state.event(&kind, value)? {
                            if tx
                                .send(ProviderStreamEvent::Chunk {
                                    chunk: Box::new(chunk),
                                })
                                .await
                                .is_err()
                            {
                                return Ok::<_, CoreError>(());
                            }
                        }
                    }
                }
                decoder.finish()?;
                let (chunk, replay) = state.terminal(request_prefix, signed_reasoning)?;
                let _ = tx
                    .send(ProviderStreamEvent::ReplayState {
                        replay: Box::new(replay),
                    })
                    .await;
                let _ = tx
                    .send(ProviderStreamEvent::Chunk {
                        chunk: Box::new(chunk),
                    })
                    .await;
                Ok(())
            };
            let result =
                tokio::select! { biased; _ = tx.closed() => return, result = operation => result };
            if let Err(error) = result {
                let error = match error {
                    CoreError::Llm(message) => CoreError::Llm(
                        crate::sensitive_data::sanitize_diagnostic(&message, Some(&credential)),
                    ),
                    error => error,
                };
                let _ = tx
                    .send(super::provider_stream_event_from_error(error))
                    .await;
            }
        });
        Ok(Box::pin(futures::stream::unfold(rx, |mut rx| async {
            rx.recv().await.map(|item| (item, rx))
        })))
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        self.list_models().await.map(|_| ())
    }
}

#[cfg(test)]
mod tests;
