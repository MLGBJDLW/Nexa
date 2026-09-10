use super::{Frame, LiveMode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::Request};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeLiveProtocol {
    OpenAiRealtime,
    GeminiLive,
    QwenRealtime,
}

// Credentials deliberately implement neither Debug nor Serialize.
pub struct NativeLiveConfig {
    pub protocol: NativeLiveProtocol,
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
}

impl NativeLiveProtocol {
    pub fn for_connection(provider: crate::llm::ProviderType, base_url: Option<&str>) -> Vec<Self> {
        use crate::llm::provider_boundary::*;
        let mut result = Vec::new();
        if is_openai_public_endpoint(provider, base_url) {
            result.push(Self::OpenAiRealtime);
        }
        if is_google_public_endpoint(provider, base_url) {
            result.push(Self::GeminiLive);
        }
        if provider == crate::llm::ProviderType::Qwen {
            result.push(Self::QwenRealtime);
        }
        result
    }
    pub fn mode(self) -> LiveMode {
        match self {
            Self::OpenAiRealtime => LiveMode::OpenAiRealtime,
            Self::GeminiLive => LiveMode::GeminiLive,
            Self::QwenRealtime => LiveMode::QwenRealtime,
        }
    }
    pub fn sample_rate(self) -> u32 {
        match self {
            Self::OpenAiRealtime => 24_000,
            Self::GeminiLive | Self::QwenRealtime => 16_000,
        }
    }
    pub fn default_model(self) -> &'static str {
        match self {
            Self::OpenAiRealtime => "gpt-realtime-2.1",
            Self::GeminiLive => "gemini-3.1-flash-live-preview",
            Self::QwenRealtime => "qwen3.5-omni-flash-realtime",
        }
    }
}

impl NativeLiveConfig {
    pub(super) fn request(&self) -> Result<Request<()>, String> {
        if self.api_key.trim().is_empty() || self.model.trim().is_empty() || self.model.len() > 160
        {
            return Err("Select a live model and a connection with an API key".into());
        }
        let mut url = match self.protocol {
            NativeLiveProtocol::QwenRealtime => {
                qwen_endpoint(self.base_url.as_deref().unwrap_or_default())?
            }
            NativeLiveProtocol::OpenAiRealtime => {
                if !crate::llm::provider_boundary::is_openai_public_endpoint(
                    crate::llm::ProviderType::OpenAi,
                    self.base_url.as_deref(),
                ) {
                    return Err("Native OpenAI Live requires a verified OpenAI endpoint; use incremental mode for other endpoints".into());
                }
                url::Url::parse("wss://api.openai.com/v1/realtime").unwrap()
            }
            NativeLiveProtocol::GeminiLive => {
                if !crate::llm::provider_boundary::is_google_public_endpoint(
                    crate::llm::ProviderType::Google,
                    self.base_url.as_deref(),
                ) {
                    return Err("Native Gemini Live requires a verified Google endpoint; use incremental mode for other endpoints".into());
                }
                url::Url::parse("wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent").unwrap()
            }
        };
        match self.protocol {
            NativeLiveProtocol::OpenAiRealtime | NativeLiveProtocol::QwenRealtime => {
                url.query_pairs_mut().append_pair("model", &self.model);
            }
            NativeLiveProtocol::GeminiLive => {
                url.query_pairs_mut()
                    .append_pair("key", self.api_key.trim());
            }
        }
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| "Invalid live endpoint")?;
        if self.protocol != NativeLiveProtocol::GeminiLive {
            request.headers_mut().insert(
                "Authorization",
                format!("Bearer {}", self.api_key.trim())
                    .parse()
                    .map_err(|_| "Invalid live credential format")?,
            );
        }
        Ok(request)
    }

    pub(super) fn setup(&self, instructions: &str) -> Value {
        match self.protocol {
            NativeLiveProtocol::QwenRealtime => json!({"type":"session.update", "session":{
                "modalities":["text"], "instructions":instructions, "turn_detection":null,
                "input_audio_format":"pcm16", "input_audio_transcription":{"model":"gummy-realtime-v1"}
            }}),
            NativeLiveProtocol::OpenAiRealtime => json!({
                "type":"session.update", "session": {
                    "type":"realtime", "model":self.model, "output_modalities":["text"],
                    "instructions":instructions, "tools":[],
                    "audio":{"input":{
                        "format":{"type":"audio/pcm","rate":24000},
                        "transcription":{"model":"gpt-4o-mini-transcribe"},
                        "turn_detection":{"type":"server_vad","create_response":false,"interrupt_response":false}
                    }}
                }
            }),
            NativeLiveProtocol::GeminiLive => json!({"setup":{
                "model":format!("models/{}", self.model.trim_start_matches("models/")),
                "generationConfig":{"responseModalities":["AUDIO"]},
                "systemInstruction":{"parts":[{"text":instructions}]},
                "inputAudioTranscription":{}, "outputAudioTranscription":{}
            }}),
        }
    }
    pub(super) fn safe_error(&self, message: &str) -> String {
        message
            .replace(self.api_key.trim(), "[redacted]")
            .chars()
            .take(500)
            .collect()
    }
}

/// A workspace-scoped endpoint must be entered explicitly. Never send a saved
/// credential to a host inferred from a model's name or an arbitrary URL.
pub fn qwen_endpoint(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value)
        .map_err(|_| "Enter the Qwen workspace WebSocket endpoint from Model Studio")?;
    let host = url.host_str().unwrap_or_default();
    let trusted = [
        ".cn-beijing.maas.aliyuncs.com",
        ".ap-southeast-1.maas.aliyuncs.com",
    ]
    .iter()
    .any(|suffix| {
        host.strip_suffix(suffix).is_some_and(|workspace| {
            !workspace.is_empty()
                && workspace
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    });
    if !trusted
        || url.scheme() != "wss"
        || url.port().is_some_and(|p| p != 443)
        || url.path() != "/api-ws/v1/realtime"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Use the exact Beijing or Singapore workspace WSS endpoint without query parameters"
                .into(),
        );
    }
    Ok(url)
}

pub(super) fn audio(protocol: NativeLiveProtocol, data: &[u8]) -> Value {
    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(data);
    match protocol {
        NativeLiveProtocol::OpenAiRealtime | NativeLiveProtocol::QwenRealtime => {
            json!({"type":"input_audio_buffer.append","audio":data})
        }
        NativeLiveProtocol::GeminiLive => {
            json!({"realtimeInput":{"audio":{"data":data,"mimeType":"audio/pcm;rate=16000"}}})
        }
    }
}

pub(super) fn frame(protocol: NativeLiveProtocol, frame: &Frame, item_id: &str) -> Value {
    match protocol {
        NativeLiveProtocol::QwenRealtime => {
            json!({"type":"input_image_buffer.append","image":frame.data})
        }
        NativeLiveProtocol::OpenAiRealtime => {
            json!({"type":"conversation.item.create","item":{"id":item_id,"type":"message","role":"user","content":[{"type":"input_image","image_url":format!("data:{};base64,{}",frame.mime_type,frame.data)}]}})
        }
        NativeLiveProtocol::GeminiLive => {
            json!({"realtimeInput":{"video":{"data":frame.data,"mimeType":frame.mime_type}}})
        }
    }
}

pub(super) fn text(protocol: NativeLiveProtocol, text: &str) -> Value {
    match protocol {
        NativeLiveProtocol::OpenAiRealtime | NativeLiveProtocol::QwenRealtime => {
            json!({"type":"conversation.item.create","item":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}})
        }
        NativeLiveProtocol::GeminiLive => json!({"realtimeInput":{"text":text}}),
    }
}

pub(super) fn respond(protocol: NativeLiveProtocol) -> Value {
    match protocol {
        NativeLiveProtocol::QwenRealtime => {
            json!({"type":"response.create","response":{"modalities":["text"]}})
        }
        NativeLiveProtocol::OpenAiRealtime => {
            json!({"type":"response.create","response":{"output_modalities":["text"]}})
        }
        NativeLiveProtocol::GeminiLive => {
            json!({"clientContent":{"turns":[{"role":"user","parts":[{"text":"Briefly summarize meaningful new observations and speech since your last update. Avoid repeating unchanged details."}]}],"turnComplete":true}})
        }
    }
}

pub(super) enum NativeEvent {
    Ready,
    Started,
    Entry {
        id: String,
        role: &'static str,
        text: String,
        append: bool,
        complete: bool,
    },
    TurnComplete,
    AudioCommitted,
    Error(String),
}

#[derive(Default)]
pub(super) struct Decoder {
    turn: u64,
    generating: bool,
}
impl Decoder {
    pub(super) fn parse(
        &mut self,
        protocol: NativeLiveProtocol,
        value: &Value,
    ) -> Vec<NativeEvent> {
        let mut events = Vec::new();
        let field = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default();
        match protocol {
            NativeLiveProtocol::OpenAiRealtime | NativeLiveProtocol::QwenRealtime => {
                match field("type") {
                    "session.updated" => events.push(NativeEvent::Ready),
                    "response.created" => events.push(NativeEvent::Started),
                    "response.text.delta"
                    | "response.audio_transcript.delta"
                    | "response.output_text.delta"
                    | "response.output_audio_transcript.delta" => events.push(NativeEvent::Entry {
                        id: format!("output:{}", field("response_id")),
                        role: "assistant",
                        text: field("delta").into(),
                        append: true,
                        complete: false,
                    }),
                    "response.text.done"
                    | "response.audio_transcript.done"
                    | "response.output_text.done"
                    | "response.output_audio_transcript.done" => events.push(NativeEvent::Entry {
                        id: format!("output:{}", field("response_id")),
                        role: "assistant",
                        text: value
                            .get("text")
                            .or_else(|| value.get("transcript"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        append: false,
                        complete: true,
                    }),
                    "conversation.item.input_audio_transcription.delta" => {
                        events.push(NativeEvent::Entry {
                            id: format!("input:{}", field("item_id")),
                            role: "user",
                            text: field("delta").into(),
                            append: true,
                            complete: false,
                        })
                    }
                    "conversation.item.input_audio_transcription.completed" => {
                        events.push(NativeEvent::Entry {
                            id: format!("input:{}", field("item_id")),
                            role: "user",
                            text: field("transcript").into(),
                            append: false,
                            complete: true,
                        })
                    }
                    "input_audio_buffer.committed"
                        if protocol == NativeLiveProtocol::OpenAiRealtime =>
                    {
                        events.push(NativeEvent::AudioCommitted)
                    }
                    "response.done" => {
                        if value.pointer("/response/status").and_then(Value::as_str)
                            == Some("failed")
                        {
                            events.push(NativeEvent::Error(
                                "The live provider could not complete this response".into(),
                            ));
                        } else {
                            events.push(NativeEvent::TurnComplete);
                        }
                    }
                    "error" => events.push(NativeEvent::Error(
                        value
                            .pointer("/error/message")
                            .and_then(Value::as_str)
                            .unwrap_or("Live provider error")
                            .into(),
                    )),
                    _ => {}
                }
            }
            NativeLiveProtocol::GeminiLive => {
                if value.get("setupComplete").is_some() {
                    events.push(NativeEvent::Ready);
                }
                if value.get("goAway").is_some() {
                    events.push(NativeEvent::Error(
                        "The live provider requested reconnection; restart Live to continue".into(),
                    ));
                }
                if let Some(error) = value.get("error") {
                    events.push(NativeEvent::Error(
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Live provider error")
                            .into(),
                    ));
                }
                if let Some(content) = value.get("serverContent") {
                    if !self.generating
                        && (content.get("modelTurn").is_some()
                            || content.get("outputTranscription").is_some())
                    {
                        self.generating = true;
                        events.push(NativeEvent::Started);
                    }
                    for (key, role) in [
                        ("inputTranscription", "user"),
                        ("outputTranscription", "assistant"),
                    ] {
                        if let Some(text) = content
                            .get(key)
                            .and_then(|v| v.get("text"))
                            .and_then(Value::as_str)
                        {
                            events.push(NativeEvent::Entry {
                                id: format!("{role}:{}", self.turn),
                                role,
                                text: text.into(),
                                append: true,
                                complete: false,
                            });
                        }
                    }
                    if content.get("outputTranscription").is_none() {
                        if let Some(parts) = content
                            .pointer("/modelTurn/parts")
                            .and_then(Value::as_array)
                        {
                            for part in parts {
                                if part.get("thought").and_then(Value::as_bool) == Some(true) {
                                    continue;
                                }
                                if let Some(text) = part.get("text").and_then(Value::as_str) {
                                    events.push(NativeEvent::Entry {
                                        id: format!("assistant:{}", self.turn),
                                        role: "assistant",
                                        text: text.into(),
                                        append: true,
                                        complete: false,
                                    });
                                }
                            }
                        }
                    }
                    if content.get("turnComplete").and_then(Value::as_bool) == Some(true)
                        || content.get("interrupted").and_then(Value::as_bool) == Some(true)
                    {
                        for role in ["user", "assistant"] {
                            events.push(NativeEvent::Entry {
                                id: format!("{role}:{}", self.turn),
                                role,
                                text: String::new(),
                                append: true,
                                complete: true,
                            });
                        }
                        self.turn += 1;
                        self.generating = false;
                        events.push(NativeEvent::TurnComplete);
                    }
                }
            }
        }
        events
    }
}
