#[cfg(test)]
#[path = "realtime_transcription_gemini_tests.rs"]
mod gemini_tests;
mod session;
use std::collections::HashMap;
use std::io::SeekFrom;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures::{SinkExt, StreamExt};
use nexa_core::speech_to_text::transcription_language_hints as language_hints;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
use url::Url;
use uuid::Uuid;

use super::realtime_transcript::RealtimeTranscript;
use crate::app_events::emit_app_event;

const REALTIME_TRANSCRIPTION_EVENT: &str = "speech-to-text:realtime";
const REALTIME_COMMAND_BUFFER: usize = 64;
const MAX_AUDIO_CHUNK_BYTES: usize = 256 * 1024;
const SESSION_ID_HEADER: &str = "x-nexa-session-id";
const FINAL_TRANSCRIPT_TIMEOUT: Duration = Duration::from_secs(30);
const REPLAY_FINAL_TRANSCRIPT_TIMEOUT: Duration = Duration::from_secs(180);
const SESSION_READY_TIMEOUT: Duration = Duration::from_secs(15);

type RealtimeSessions = Arc<Mutex<HashMap<String, mpsc::Sender<RealtimeCommand>>>>;
pub(super) type LiveTranscriptCallback =
    Arc<dyn Fn(&str, Option<&str>, Option<&str>, Option<&str>) + Send + Sync>;

#[derive(Clone, Default)]
pub struct RealtimeTranscriptionState {
    sessions: RealtimeSessions,
    cancellations: Arc<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>>,
}

enum RealtimeCommand {
    Append(Vec<u8>),
    Finish(oneshot::Sender<Result<String, String>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RealtimeDialect {
    OpenAi,
    Gemini,
    DashScope,
    DashScopeTask,
}

impl RealtimeDialect {
    fn from_config(config: &nexa_core::app_settings::SpeechToTextConfig) -> Result<Self, String> {
        if !config.is_configured() {
            return Err("Realtime transcription is not fully configured".to_string());
        }
        match config.api_style.as_str() {
            "openai_realtime_transcription" if config.model.trim() == "gpt-live-transcribe" => {
                Ok(Self::OpenAi)
            }
            "gemini_realtime_asr" if config.model.trim() == "gemini-3.5-transcribe-live" => {
                Ok(Self::Gemini)
            }
            "dashscope_realtime_asr"
                if nexa_core::dashscope_speech::is_realtime_asr_model(&config.model) =>
            {
                Ok(Self::DashScope)
            }
            "dashscope_streaming_asr"
                if nexa_core::dashscope_speech::is_streaming_asr_model(&config.model) =>
            {
                Ok(Self::DashScopeTask)
            }
            "openai_realtime_transcription"
            | "gemini_realtime_asr"
            | "dashscope_realtime_asr"
            | "dashscope_streaming_asr" => Err(format!(
                "Unsupported realtime transcription model for {}: {}",
                config.api_style,
                config.model.trim()
            )),
            value => Err(format!(
                "Unsupported realtime transcription API style: {value}"
            )),
        }
    }

    fn sample_rate(self) -> u32 {
        match self {
            Self::OpenAi => 24_000,
            Self::Gemini | Self::DashScope | Self::DashScopeTask => 16_000,
        }
    }

    fn waits_for_session_finished(self) -> bool {
        matches!(self, Self::DashScope | Self::DashScopeTask)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptUpdate {
    AppendDelta,
    ReplaceSnapshot,
}

impl TranscriptUpdate {
    fn wire_name(self) -> &'static str {
        match self {
            Self::AppendDelta => "appendDelta",
            Self::ReplaceSnapshot => "replaceSnapshot",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ParsedRealtimeServerEvent {
    Interim {
        utterance_id: Option<String>,
        text: String,
        update: TranscriptUpdate,
    },
    Final {
        utterance_id: Option<String>,
        text: String,
    },
    SessionFinished,
    Error(String),
    Other,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RealtimeTranscriptionFrontendEvent<'a> {
    session_id: &'a str,
    sequence: u64,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    update: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    utterance_id: Option<&'a str>,
}

struct RealtimeEventSink {
    frontend: Arc<dyn Fn(&RealtimeTranscriptionFrontendEvent<'_>) + Send + Sync>,
    // The actor owns its listener before it can consume any provider event.
    live: Option<LiveTranscriptCallback>,
}

fn build_realtime_endpoint(base_url: &str, model: &str) -> Result<String, String> {
    let mut url = Url::parse(base_url.trim())
        .map_err(|error| format!("Invalid Realtime base URL: {error}"))?;
    let websocket_scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        "wss" => "wss",
        "ws" => "ws",
        scheme => return Err(format!("Unsupported Realtime URL scheme: {scheme}")),
    };
    url.set_scheme(websocket_scheme)
        .map_err(|_| "Unable to set Realtime WebSocket scheme".to_string())?;

    let path = url.path().trim_end_matches('/').to_string();
    if !path.ends_with("/realtime") {
        let realtime_path = if path.is_empty() {
            "/realtime".to_string()
        } else {
            format!("{path}/realtime")
        };
        url.set_path(&realtime_path);
    } else if url.path().ends_with('/') {
        url.set_path(&path);
    }
    url.set_query(None);
    url.query_pairs_mut().append_pair("model", model.trim());
    Ok(url.to_string())
}

fn build_session_update(
    dialect: RealtimeDialect,
    model: &str,
    language: Option<&str>,
    task_id: &str,
) -> Value {
    match dialect {
        RealtimeDialect::Gemini => serde_json::json!({
            "setup": {
                "model": format!("models/{}", model.trim_start_matches("models/")),
                "generationConfig": {"responseModalities":["TEXT"]},
                "inputAudioTranscription": {"languageCodes": language_hints(language)},
                "realtimeInputConfig": {"automaticActivityDetection":{"disabled":true}}
            }
        }),
        RealtimeDialect::OpenAi => {
            let mut transcription = serde_json::json!({
                "model": model.trim(),
                "delay": "low"
            });
            let languages = language_hints(language);
            if !languages.is_empty() {
                transcription["languages"] = serde_json::json!(languages);
            }
            serde_json::json!({
                "type": "session.update",
                "session": {
                    "type": "transcription",
                    "audio": {
                        "input": {
                            "format": {
                                "type": "audio/pcm",
                                "rate": dialect.sample_rate()
                            },
                            "transcription": transcription,
                            "turn_detection": null
                        }
                    }
                }
            })
        }
        RealtimeDialect::DashScopeTask => {
            let mut parameters =
                serde_json::json!({"format":"pcm", "sample_rate":16000, "heartbeat":true});
            let languages: Vec<_> = language_hints(language).into_iter().take(4).collect();
            if model.trim() == "qwen-audio-3.1-asr-flash-message" {
                parameters["intermediate_result_enabled"] = serde_json::json!(true);
            } else if !languages.is_empty() {
                parameters["language_hints"] = serde_json::json!(languages);
            }
            nexa_core::dashscope_speech::task_message(
                "run-task",
                task_id,
                serde_json::json!({
                    "task_group":"audio", "task":"asr", "function":"recognition", "model":model.trim(), "parameters":parameters, "input":{}
                }),
            )
        }
        RealtimeDialect::DashScope => {
            let mut transcription = serde_json::Map::new();
            if let Some(language) = language_hints(language).into_iter().next() {
                transcription.insert("language".to_string(), Value::String(language));
            }
            serde_json::json!({
                "event_id": format!("event_{}", Uuid::new_v4().simple()),
                "type": "session.update",
                "session": {
                    "input_audio_format": "pcm",
                    "sample_rate": dialect.sample_rate(),
                    "input_audio_transcription": transcription,
                    "turn_detection": { "type": "server_vad", "threshold": 0.0, "silence_duration_ms": 400 }
                }
            })
        }
    }
}

fn parse_server_event(dialect: RealtimeDialect, event: &Value) -> ParsedRealtimeServerEvent {
    if dialect == RealtimeDialect::Gemini {
        if let Some(message) = event.pointer("/error/message").and_then(Value::as_str) {
            return ParsedRealtimeServerEvent::Error(message.into());
        }
        if event.get("goAway").is_some() {
            return ParsedRealtimeServerEvent::Error(
                "Provider requested a fresh transcription connection".into(),
            );
        }
        if let Some(transcription) = event
            .pointer("/serverContent/inputTranscription")
            .and_then(Value::as_object)
        {
            return ParsedRealtimeServerEvent::Final {
                utterance_id: None,
                // Protobuf JSON may omit the default empty string. The final
                // object itself still authoritatively acknowledges silence.
                text: transcription
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            };
        }
        if let Some(text) = event
            .pointer("/serverContent/interimInputTranscription/text")
            .and_then(Value::as_str)
        {
            return ParsedRealtimeServerEvent::Interim {
                utterance_id: None,
                text: text.into(),
                update: TranscriptUpdate::ReplaceSnapshot,
            };
        }
        return ParsedRealtimeServerEvent::Other;
    }
    if dialect == RealtimeDialect::DashScopeTask {
        return match event.pointer("/header/event").and_then(Value::as_str) {
            Some("task-finished") => ParsedRealtimeServerEvent::SessionFinished,
            Some("task-failed" | "error") => {
                ParsedRealtimeServerEvent::Error(nexa_core::dashscope_speech::task_error(event))
            }
            Some("result-generated") => {
                let Some(sentence) = event.pointer("/payload/output/sentence") else {
                    return ParsedRealtimeServerEvent::Other;
                };
                if sentence["heartbeat"].as_bool() == Some(true) {
                    return ParsedRealtimeServerEvent::Other;
                }
                // Qwen Audio 3.0 uses begin_time as the stable sentence key;
                // newer models also provide sentence_id. Collapsing missing
                // IDs into one finalized item suppresses every later interim.
                let utterance_id = sentence["sentence_id"]
                    .as_u64()
                    .map(|id| id.to_string())
                    .or_else(|| sentence["sentence_id"].as_str().map(str::to_owned))
                    .or_else(|| {
                        sentence["begin_time"]
                            .as_u64()
                            .map(|time| format!("at-{time}"))
                    });
                let text = sentence["text"].as_str().unwrap_or_default().to_string();
                if sentence["sentence_end"].as_bool() == Some(true) {
                    ParsedRealtimeServerEvent::Final { utterance_id, text }
                } else {
                    ParsedRealtimeServerEvent::Interim {
                        utterance_id,
                        text,
                        update: TranscriptUpdate::ReplaceSnapshot,
                    }
                }
            }
            _ => ParsedRealtimeServerEvent::Other,
        };
    }
    let utterance_id = event
        .get("item_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "conversation.item.input_audio_transcription.delta"
            if dialect == RealtimeDialect::OpenAi =>
        {
            ParsedRealtimeServerEvent::Interim {
                utterance_id,
                text: event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                update: TranscriptUpdate::AppendDelta,
            }
        }
        "conversation.item.input_audio_transcription.text"
            if dialect == RealtimeDialect::DashScope =>
        {
            ParsedRealtimeServerEvent::Interim {
                utterance_id,
                text: format!(
                    "{}{}",
                    event
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    event
                        .get("stash")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                ),
                update: TranscriptUpdate::ReplaceSnapshot,
            }
        }
        "conversation.item.input_audio_transcription.completed" => {
            ParsedRealtimeServerEvent::Final {
                utterance_id,
                text: event
                    .get("transcript")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            }
        }
        "conversation.item.input_audio_transcription.failed" => ParsedRealtimeServerEvent::Error(
            event
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Realtime transcription item failed")
                .to_string(),
        ),
        "session.finished" => ParsedRealtimeServerEvent::SessionFinished,
        "error" => ParsedRealtimeServerEvent::Error(
            event
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| event.get("message").and_then(Value::as_str))
                .unwrap_or("Realtime transcription session failed")
                .to_string(),
        ),
        _ => ParsedRealtimeServerEvent::Other,
    }
}

fn emit_realtime_event(
    events: &RealtimeEventSink,
    session_id: &str,
    sequence: &mut u64,
    kind: &str,
    text: Option<&str>,
    update: Option<TranscriptUpdate>,
    utterance_id: Option<&str>,
) {
    *sequence = sequence.saturating_add(1);
    if kind != "interim" && kind != "final" {
        emit_live_transcript(
            events,
            kind,
            text,
            update.map(TranscriptUpdate::wire_name),
            utterance_id,
        );
    }
    (events.frontend)(&RealtimeTranscriptionFrontendEvent {
        session_id,
        sequence: *sequence,
        kind,
        text,
        update: update.map(TranscriptUpdate::wire_name),
        utterance_id,
    });
}

fn emit_live_transcript(
    events: &RealtimeEventSink,
    kind: &str,
    text: Option<&str>,
    update: Option<&str>,
    utterance_id: Option<&str>,
) {
    if let Some(callback) = &events.live {
        callback(kind, text, update, utterance_id);
    }
}

fn finish_messages(dialect: RealtimeDialect, task_id: &str) -> Vec<Value> {
    if dialect == RealtimeDialect::Gemini {
        return vec![serde_json::json!({"realtimeInput":{"activityEnd":{}}})];
    }
    if dialect == RealtimeDialect::DashScopeTask {
        return vec![nexa_core::dashscope_speech::task_message(
            "finish-task",
            task_id,
            serde_json::json!({"input":{}}),
        )];
    }
    vec![serde_json::json!({
        "event_id": format!("event_{}", Uuid::new_v4().simple()),
        "type": if dialect == RealtimeDialect::DashScope { "session.finish" } else { "input_audio_buffer.commit" }
    })]
}

fn audio_message(dialect: RealtimeDialect, audio_data: &[u8]) -> Message {
    if dialect == RealtimeDialect::Gemini {
        Message::Text(serde_json::json!({"realtimeInput":{"audio":{"data":BASE64.encode(audio_data),"mimeType":"audio/pcm;rate=16000"}}}).to_string().into())
    } else if dialect == RealtimeDialect::DashScopeTask {
        Message::Binary(audio_data.to_vec().into())
    } else {
        Message::Text(append_message(audio_data).to_string().into())
    }
}

fn append_message(audio_data: &[u8]) -> Value {
    serde_json::json!({
        "event_id": format!("event_{}", Uuid::new_v4().simple()),
        "type": "input_audio_buffer.append",
        "audio": BASE64.encode(audio_data),
    })
}

fn resolve_pending_final(
    pending_final: &mut Option<oneshot::Sender<Result<String, String>>>,
    result: Result<String, String>,
) {
    if let Some(sender) = pending_final.take() {
        let _ = sender.send(result);
    }
}

fn replay_wav_data_bytes(header: &[u8; 44], expected_sample_rate: u32) -> Result<u32, String> {
    if &header[0..4] != b"RIFF"
        || &header[8..12] != b"WAVE"
        || &header[12..16] != b"fmt "
        || &header[36..40] != b"data"
    {
        return Err("Managed Realtime replay requires a canonical PCM WAV".to_string());
    }
    let audio_format = u16::from_le_bytes([header[20], header[21]]);
    let channels = u16::from_le_bytes([header[22], header[23]]);
    let sample_rate = u32::from_le_bytes(header[24..28].try_into().expect("four bytes"));
    let bits_per_sample = u16::from_le_bytes([header[34], header[35]]);
    if audio_format != 1
        || channels != 1
        || sample_rate != expected_sample_rate
        || bits_per_sample != 16
    {
        return Err(format!(
            "Managed Realtime replay requires mono {expected_sample_rate} Hz little-endian PCM16"
        ));
    }
    Ok(u32::from_le_bytes(
        header[40..44].try_into().expect("four bytes"),
    ))
}

async fn wait_for_session_ready<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    dialect: RealtimeDialect,
    task_id: &str,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(SESSION_READY_TIMEOUT, async {
        while let Some(incoming) = socket.next().await {
            match incoming.map_err(|error| format!("Realtime configuration connection failed: {error}"))? {
                Message::Text(text) => {
                    let Ok(event) = serde_json::from_str::<Value>(&text) else { continue; };
                    if dialect == RealtimeDialect::Gemini && event.get("setupComplete").is_some() { return Ok(()); }
                    if dialect == RealtimeDialect::DashScopeTask {
                        if event.pointer("/header/task_id").and_then(Value::as_str) != Some(task_id) { continue; }
                        match event.pointer("/header/event").and_then(Value::as_str) {
                            Some("task-started") => return Ok(()),
                            Some("task-failed" | "error") => return Err(nexa_core::dashscope_speech::task_error(&event)),
                            _ => continue,
                        }
                    }
                    match event.get("type").and_then(Value::as_str) {
                        Some("session.updated" | "transcription_session.updated") => return Ok(()),
                        Some("error") => return Err(event.pointer("/error/message").and_then(Value::as_str).unwrap_or("Realtime session configuration was rejected").to_string()),
                        _ => {}
                    }
                }
                Message::Ping(payload) => socket.send(Message::Pong(payload)).await
                    .map_err(|error| format!("Realtime configuration heartbeat failed: {error}"))?,
                Message::Close(frame) => return Err(format!("Realtime connection closed before session configuration was accepted: {frame:?}")),
                _ => {}
            }
        }
        Err("Realtime connection ended before session configuration was accepted".to_string())
    }).await.map_err(|_| "Timed out waiting for realtime session configuration".to_string())?
}

/// Finalized recordings use the same segmented provider session and reconnect
/// logic as live dictation, reading the original file without copying or deleting it.
pub(super) async fn transcribe_realtime_spool(
    wav_path: &Path,
    config: &nexa_core::app_settings::SpeechToTextConfig,
) -> Result<String, String> {
    session::replay(wav_path, config).await
}

#[tauri::command]
pub async fn start_realtime_transcription_cmd(
    app_handle: AppHandle,
    app_state: State<'_, super::AppState>,
    realtime_state: State<'_, RealtimeTranscriptionState>,
    live_capture: Option<bool>,
) -> Result<String, String> {
    start_realtime_transcription(app_handle, app_state, realtime_state, live_capture, None).await
}

pub(super) async fn start_realtime_transcription(
    app_handle: AppHandle,
    app_state: State<'_, super::AppState>,
    realtime_state: State<'_, RealtimeTranscriptionState>,
    live_capture: Option<bool>,
    live_callback: Option<LiveTranscriptCallback>,
) -> Result<String, String> {
    let config = app_state
        .db_executor
        .write(|db| Ok(db.load_app_config()?.speech_to_text))
        .await
        .map_err(|error| error.to_string())?
        .value;
    let events = RealtimeEventSink {
        frontend: Arc::new(move |event| {
            emit_app_event(&app_handle, REALTIME_TRANSCRIPTION_EVENT, event);
        }),
        live: live_callback,
    };
    start_realtime_session(config, &realtime_state, live_capture, events).await
}

async fn start_realtime_session(
    config: nexa_core::app_settings::SpeechToTextConfig,
    realtime_state: &RealtimeTranscriptionState,
    live_capture: Option<bool>,
    events: RealtimeEventSink,
) -> Result<String, String> {
    let dialect = RealtimeDialect::from_config(&config)?;

    let task_id = Uuid::new_v4().simple().to_string();
    let socket = session::connect(&config, dialect, &task_id).await?;

    let session_id = Uuid::new_v4().to_string();
    let (command_tx, command_rx) = mpsc::channel(REALTIME_COMMAND_BUFFER);
    realtime_state
        .sessions
        .lock()
        .await
        .insert(session_id.clone(), command_tx);

    let sessions = realtime_state.sessions.clone();
    let cancellations = realtime_state.cancellations.clone();
    let cancellation = Arc::new(tokio::sync::Notify::new());
    cancellations
        .lock()
        .await
        .insert(session_id.clone(), cancellation.clone());
    let actor_session_id = session_id.clone();
    tokio::spawn(async move {
        let _ = live_capture;
        tokio::select! {
            _ = cancellation.notified() => {},
            result = session::run(config, dialect, task_id, socket, command_rx, &events, &actor_session_id) => {
                if let Err(error) = result { log::debug!("Realtime session ended: {error}"); }
            }
        }
        cancellations.lock().await.remove(&actor_session_id);
        sessions.lock().await.remove(&actor_session_id);
    });

    Ok(session_id)
}

/// Remote dictation reuses the desktop transcript assembler and provider lifecycle,
/// but emits only to its authenticated device's event stream.
pub(super) async fn start_remote_transcription(
    config: nexa_core::app_settings::SpeechToTextConfig,
    state: &RealtimeTranscriptionState,
    callback: Arc<dyn Fn(Value) + Send + Sync>,
) -> Result<(String, u32), String> {
    let sample_rate = RealtimeDialect::from_config(&config)?.sample_rate();
    let events = RealtimeEventSink {
        frontend: Arc::new(move |event| {
            if let Ok(value) = serde_json::to_value(event) {
                callback(value);
            }
        }),
        live: None,
    };
    let id = start_realtime_session(config, state, None, events).await?;
    Ok((id, sample_rate))
}

async fn session_sender(
    state: &RealtimeTranscriptionState,
    session_id: &str,
) -> Result<mpsc::Sender<RealtimeCommand>, String> {
    state
        .sessions
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| "Realtime transcription session is not active".to_string())
}

fn raw_realtime_audio(body: &tauri::ipc::InvokeBody) -> Result<Vec<u8>, String> {
    let tauri::ipc::InvokeBody::Raw(audio_data) = body else {
        return Err("Realtime transcription requires a raw binary request body".to_string());
    };
    if audio_data.len() > MAX_AUDIO_CHUNK_BYTES {
        return Err(format!(
            "Realtime audio chunk exceeds {MAX_AUDIO_CHUNK_BYTES} bytes"
        ));
    }
    if !audio_data.len().is_multiple_of(2) {
        return Err("Realtime PCM16 audio must contain complete 16-bit samples".to_string());
    }
    Ok(audio_data.clone())
}

#[tauri::command]
pub async fn append_realtime_transcription_audio_cmd(
    request: tauri::ipc::Request<'_>,
    state: State<'_, RealtimeTranscriptionState>,
) -> Result<(), String> {
    let session_id = request
        .headers()
        .get(SESSION_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Realtime transcription session header is missing".to_string())?
        .to_string();
    let audio_data = raw_realtime_audio(request.body())?;
    if audio_data.is_empty() {
        return Ok(());
    }
    session_sender(&state, &session_id)
        .await?
        .send(RealtimeCommand::Append(audio_data))
        .await
        .map_err(|_| "Realtime transcription session has closed".to_string())
}

#[tauri::command]
pub async fn finish_realtime_transcription_cmd(
    session_id: String,
    state: State<'_, RealtimeTranscriptionState>,
) -> Result<String, String> {
    let sender = session_sender(&state, &session_id).await?;
    let (response_tx, response_rx) = oneshot::channel();
    sender
        .send(RealtimeCommand::Finish(response_tx))
        .await
        .map_err(|_| "Realtime transcription session has closed".to_string())?;
    // Backlog duration is not a timeout. The session owner separately bounds
    // a provider operation or an outage without ending a progressing recording.
    response_rx
        .await
        .map_err(|_| "Realtime transcription session ended unexpectedly".to_string())?
}

#[tauri::command]
pub async fn cancel_realtime_transcription_cmd(
    session_id: String,
    state: State<'_, RealtimeTranscriptionState>,
) -> Result<(), String> {
    if let Some(cancel) = state.cancellations.lock().await.remove(&session_id) {
        cancel.notify_one();
    }
    state.sessions.lock().await.remove(&session_id);
    Ok(())
}

pub(super) async fn append_live_transcription_audio(
    state: &RealtimeTranscriptionState,
    session_id: &str,
    pcm: Vec<u8>,
) -> Result<(), String> {
    nexa_core::live_analysis::validate_pcm(&pcm)?;
    session_sender(state, session_id)
        .await?
        .send(RealtimeCommand::Append(pcm))
        .await
        .map_err(|_| "Live microphone transcription has closed".into())
}

#[cfg(test)]
mod tests {
    use super::{
        build_realtime_endpoint, build_session_update, finish_messages, parse_server_event,
        raw_realtime_audio, replay_wav_data_bytes, transcribe_realtime_spool,
        ParsedRealtimeServerEvent, RealtimeDialect, TranscriptUpdate, MAX_AUDIO_CHUNK_BYTES,
    };
    use futures::{SinkExt, StreamExt};
    use serde_json::Value;
    use tokio_tungstenite::tungstenite::Message;

    #[tokio::test]
    async fn finishing_dictation_returns_the_specific_provider_error() {
        use std::sync::Arc;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let event: Value = serde_json::from_str(&text).unwrap();
                match event["type"].as_str().unwrap() {
                    "session.update" => socket
                        .send(Message::Text(
                            serde_json::json!({"type":"session.updated"})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap(),
                    "input_audio_buffer.commit" => {
                        socket.send(Message::Text(serde_json::json!({"type":"error","error":{"message":"Provider quota exhausted"}}).to_string().into())).await.unwrap();
                        break;
                    }
                    _ => {}
                }
            }
        });
        let state = super::RealtimeTranscriptionState::default();
        let id = super::start_realtime_session(
            nexa_core::app_settings::SpeechToTextConfig {
                provider: "openai".into(),
                api_style: "openai_realtime_transcription".into(),
                api_key: "test".into(),
                base_url: Some(format!("http://{address}/v1")),
                model: "gpt-live-transcribe".into(),
                ..Default::default()
            },
            &state,
            None,
            super::RealtimeEventSink {
                frontend: Arc::new(|_| {}),
                live: None,
            },
        )
        .await
        .unwrap();
        super::session_sender(&state, &id)
            .await
            .unwrap()
            .send(super::RealtimeCommand::Append(vec![0; 4_800]))
            .await
            .unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        super::session_sender(&state, &id)
            .await
            .unwrap()
            .send(super::RealtimeCommand::Finish(sender))
            .await
            .unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), receiver)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, Err("Provider quota exhausted".into()));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn recording_survives_first_final_and_replays_interrupted_utterance() {
        use base64::Engine as _;
        use std::sync::{Arc, Mutex};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let first_audio: Vec<u8> = (0..2_400)
            .flat_map(|_| 1_000_i16.to_le_bytes())
            .chain(std::iter::repeat_n(0, 19_200))
            .collect();
        let second_audio: Vec<u8> = (0..2_400).flat_map(|_| 2_000_i16.to_le_bytes()).collect();
        let expected_first = first_audio.clone();
        let expected_second = second_audio.clone();
        let server = tokio::spawn(async move {
            for connection in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let mut received = Vec::new();
                let mut first_done = false;
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    match event["type"].as_str().unwrap() {
                        "session.update" => {
                            assert!(event["session"]["audio"]["input"]["turn_detection"].is_null());
                            socket
                                .send(Message::Text(
                                    serde_json::json!({"type":"session.updated"})
                                        .to_string()
                                        .into(),
                                ))
                                .await
                                .unwrap();
                        }
                        "input_audio_buffer.append" => {
                            let bytes = super::BASE64
                                .decode(event["audio"].as_str().unwrap())
                                .unwrap();
                            if connection == 0 && first_done {
                                assert_eq!(bytes, expected_second);
                                socket.close(None).await.unwrap();
                                break;
                            }
                            received.extend(bytes);
                        }
                        "input_audio_buffer.commit" => {
                            let (id, text) = if connection == 0 {
                                assert_eq!(received.len(), expected_first.len());
                                assert!(received == expected_first);
                                first_done = true;
                                ("first", "First utterance.")
                            } else {
                                assert_eq!(
                                    received.len(),
                                    expected_second.len(),
                                    "only unconfirmed audio must replay"
                                );
                                assert!(received == expected_second);
                                ("second", "Second utterance.")
                            };
                            for value in [
                                serde_json::json!({"type":"input_audio_buffer.committed", "item_id":id}),
                                serde_json::json!({"type":"conversation.item.input_audio_transcription.completed", "item_id":id, "transcript":text}),
                            ] {
                                socket
                                    .send(Message::Text(value.to_string().into()))
                                    .await
                                    .unwrap();
                            }
                            if connection == 1 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
        let state = super::RealtimeTranscriptionState::default();
        let received = Arc::new(Mutex::new(Vec::new()));
        let captured = received.clone();
        let callback: super::LiveTranscriptCallback = Arc::new(move |kind, text, _, utterance| {
            captured.lock().unwrap().push((
                kind.to_owned(),
                text.map(str::to_owned),
                utterance.map(str::to_owned),
            ));
        });
        let callback_lifetime = Arc::downgrade(&callback);
        let id = super::start_realtime_session(
            nexa_core::app_settings::SpeechToTextConfig {
                provider: "openai".into(),
                api_style: "openai_realtime_transcription".into(),
                api_key: "test".into(),
                base_url: Some(format!("http://{address}/v1")),
                model: "gpt-live-transcribe".into(),
                ..Default::default()
            },
            &state,
            Some(true),
            super::RealtimeEventSink {
                frontend: Arc::new(|_| {}),
                live: Some(callback),
            },
        )
        .await
        .unwrap();
        let sender = super::session_sender(&state, &id).await.unwrap();
        sender
            .send(super::RealtimeCommand::Append(first_audio))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !received
                .lock()
                .unwrap()
                .iter()
                .any(|event| event.0 == "final")
            {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            state.sessions.lock().await.contains_key(&id),
            "utterance completion must not end recording"
        );
        sender
            .send(super::RealtimeCommand::Append(second_audio))
            .await
            .unwrap();
        let (response, result) = tokio::sync::oneshot::channel();
        sender
            .send(super::RealtimeCommand::Finish(response))
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), result)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            "First utterance. Second utterance."
        );
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !state.sessions.lock().await.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        server.await.unwrap();
        assert!(state.cancellations.lock().await.is_empty());
        assert!(callback_lifetime.upgrade().is_none());
        let received = received.lock().unwrap();
        let finals: Vec<_> = received.iter().filter(|event| event.0 == "final").collect();
        assert_eq!(finals.len(), 2);
        assert_eq!(finals[0].1.as_deref(), Some("First utterance."));
        assert_eq!(finals[1].1.as_deref(), Some("Second utterance."));
        assert!(received.iter().any(|event| event.0 == "reconnecting"));
        assert!(!received.iter().any(|event| event.0 == "error"));
    }

    #[test]
    fn realtime_audio_requires_bounded_aligned_raw_pcm() {
        assert_eq!(
            raw_realtime_audio(&tauri::ipc::InvokeBody::Raw(vec![1, 2])).unwrap(),
            vec![1, 2]
        );
        assert!(raw_realtime_audio(&tauri::ipc::InvokeBody::Raw(vec![1])).is_err());
        assert!(raw_realtime_audio(&tauri::ipc::InvokeBody::Raw(vec![
            0;
            MAX_AUDIO_CHUNK_BYTES + 2
        ]))
        .is_err());
        assert!(
            raw_realtime_audio(&tauri::ipc::InvokeBody::Json(serde_json::json!({
                "audioData": [1, 2]
            })))
            .is_err()
        );
    }

    #[test]
    fn replay_accepts_only_canonical_mono_24khz_pcm16() {
        let mut header = [0_u8; 44];
        header[0..4].copy_from_slice(b"RIFF");
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[20..22].copy_from_slice(&1_u16.to_le_bytes());
        header[22..24].copy_from_slice(&1_u16.to_le_bytes());
        header[24..28].copy_from_slice(&24_000_u32.to_le_bytes());
        header[34..36].copy_from_slice(&16_u16.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&128_u32.to_le_bytes());

        assert_eq!(replay_wav_data_bytes(&header, 24_000).unwrap(), 128);
        header[24..28].copy_from_slice(&16_000_u32.to_le_bytes());
        assert!(replay_wav_data_bytes(&header, 24_000).is_err());
        assert_eq!(replay_wav_data_bytes(&header, 16_000).unwrap(), 128);
    }

    #[test]
    fn realtime_endpoint_upgrades_https_and_selects_the_model() {
        assert_eq!(
            build_realtime_endpoint("https://api.openai.com/v1", "gpt-live-transcribe")
                .expect("valid OpenAI endpoint"),
            "wss://api.openai.com/v1/realtime?model=gpt-live-transcribe"
        );
        assert_eq!(
            build_realtime_endpoint("https://proxy.example/v1/realtime/", "gpt-live-transcribe")
                .expect("valid proxy endpoint"),
            "wss://proxy.example/v1/realtime?model=gpt-live-transcribe"
        );
    }

    #[test]
    fn realtime_dialects_reject_models_without_a_verified_wire_contract() {
        let mut config = nexa_core::app_settings::SpeechToTextConfig {
            provider: "alibaba_model_studio".to_string(),
            api_style: "dashscope_realtime_asr".to_string(),
            api_key: "test-key".to_string(),
            base_url: Some("https://dashscope.aliyuncs.com/api-ws/v1".to_string()),
            model: "qwen3-asr-flash".to_string(),
            ..nexa_core::app_settings::SpeechToTextConfig::default()
        };
        assert!(RealtimeDialect::from_config(&config).is_err());
        config.model = "qwen3-asr-flash-realtime".to_string();
        assert_eq!(
            RealtimeDialect::from_config(&config),
            Ok(RealtimeDialect::DashScope)
        );
        config.model = "qwen3-asr-flash-realtime-2026-02-10".into();
        assert_eq!(
            RealtimeDialect::from_config(&config),
            Ok(RealtimeDialect::DashScope)
        );
        config.model = "qwen3-asr-flash-realtime-2099-01-01".into();
        assert!(RealtimeDialect::from_config(&config).is_err());
    }

    #[test]
    fn session_update_uses_live_transcription_pcm_and_language_hints() {
        let payload = build_session_update(
            RealtimeDialect::OpenAi,
            "gpt-live-transcribe",
            Some("zh-cn, en"),
            "test-task",
        );
        assert_eq!(payload["type"], "session.update");
        assert_eq!(payload["session"]["type"], "transcription");
        assert_eq!(
            payload["session"]["audio"]["input"]["format"]["rate"],
            24_000
        );
        assert_eq!(
            payload["session"]["audio"]["input"]["transcription"]["model"],
            "gpt-live-transcribe"
        );
        assert_eq!(
            payload["session"]["audio"]["input"]["transcription"]["delay"],
            "low"
        );
        assert_eq!(
            payload["session"]["audio"]["input"]["transcription"]["languages"],
            serde_json::json!(["zh-cn", "en"])
        );
        assert!(payload["session"]["audio"]["input"]["turn_detection"].is_null());

        let dashscope = build_session_update(
            RealtimeDialect::DashScope,
            "qwen3-asr-flash-realtime",
            Some("zh"),
            "test-task",
        );
        assert_eq!(dashscope["session"]["input_audio_format"], "pcm");
        assert_eq!(dashscope["session"]["sample_rate"], 16_000);
        assert_eq!(
            dashscope["session"]["input_audio_transcription"]["language"],
            "zh"
        );
        assert_eq!(dashscope["session"]["turn_detection"]["type"], "server_vad");
        assert_eq!(
            dashscope["session"]["turn_detection"]["silence_duration_ms"],
            400
        );
        assert_eq!(
            finish_messages(RealtimeDialect::OpenAi, "test-task").len(),
            1
        );
        assert_eq!(
            finish_messages(RealtimeDialect::DashScope, "test-task").len(),
            1
        );
        assert_eq!(
            finish_messages(RealtimeDialect::DashScope, "test-task")[0]["type"],
            "session.finish"
        );
    }

    #[test]
    fn server_transcript_events_are_normalized_for_the_frontend() {
        assert_eq!(
            parse_server_event(
                RealtimeDialect::OpenAi,
                &serde_json::json!({
                    "type": "conversation.item.input_audio_transcription.delta",
                    "item_id": "item_1",
                    "delta": "hello"
                })
            ),
            ParsedRealtimeServerEvent::Interim {
                utterance_id: Some("item_1".to_string()),
                text: "hello".to_string(),
                update: TranscriptUpdate::AppendDelta,
            }
        );
        assert_eq!(
            parse_server_event(
                RealtimeDialect::OpenAi,
                &serde_json::json!({
                    "type": "conversation.item.input_audio_transcription.completed",
                    "item_id": "item_1",
                    "transcript": "hello world"
                })
            ),
            ParsedRealtimeServerEvent::Final {
                utterance_id: Some("item_1".to_string()),
                text: "hello world".to_string(),
            }
        );
        assert_eq!(
            parse_server_event(
                RealtimeDialect::DashScope,
                &serde_json::json!({
                    "type": "conversation.item.input_audio_transcription.text",
                    "item_id": "item_qwen",
                    "text": "今天",
                    "stash": "天气"
                })
            ),
            ParsedRealtimeServerEvent::Interim {
                utterance_id: Some("item_qwen".to_string()),
                text: "今天天气".to_string(),
                update: TranscriptUpdate::ReplaceSnapshot,
            }
        );
        assert_eq!(
            parse_server_event(
                RealtimeDialect::DashScope,
                &serde_json::json!({
                    "type": "session.finished"
                })
            ),
            ParsedRealtimeServerEvent::SessionFinished
        );
        assert_eq!(
            parse_server_event(
                RealtimeDialect::OpenAi,
                &serde_json::json!({
                    "type": "error",
                    "error": { "message": "bad session" }
                })
            ),
            ParsedRealtimeServerEvent::Error("bad session".to_string())
        );
    }

    pub(super) fn canonical_pcm_wav(sample_rate: u32) -> Vec<u8> {
        let data_bytes = 640_u32;
        let mut wav = vec![0_u8; 44 + data_bytes as usize];
        wav[0..4].copy_from_slice(b"RIFF");
        wav[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
        wav[8..12].copy_from_slice(b"WAVE");
        wav[12..16].copy_from_slice(b"fmt ");
        wav[16..20].copy_from_slice(&16_u32.to_le_bytes());
        wav[20..22].copy_from_slice(&1_u16.to_le_bytes());
        wav[22..24].copy_from_slice(&1_u16.to_le_bytes());
        wav[24..28].copy_from_slice(&sample_rate.to_le_bytes());
        wav[28..32].copy_from_slice(&(sample_rate * 2).to_le_bytes());
        wav[32..34].copy_from_slice(&2_u16.to_le_bytes());
        wav[34..36].copy_from_slice(&16_u16.to_le_bytes());
        wav[36..40].copy_from_slice(b"data");
        wav[40..44].copy_from_slice(&data_bytes.to_le_bytes());
        wav
    }

    #[tokio::test(start_paused = true)]
    async fn long_silent_replay_reaches_speech_without_a_recording_duration_timeout() {
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        use std::time::Duration;
        // Real socket readiness must not race Tokio's automatic clock jumps.
        // A blocking guard holds the paused clock while the server advances one
        // packet duration after receiving each packet. Its real-time deadline
        // releases the clock if I/O stalls so the existing timeout still fails.
        let (release_clock, hold_clock) = std::sync::mpsc::channel::<()>();
        let clock_guard = tokio::task::spawn_blocking(move || {
            let _ = hold_clock.recv_timeout(Duration::from_secs(30));
        });
        let started = tokio::time::Instant::now();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let silence_bytes = 181 * 16_000 * 2;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut received = 0;
            let mut heard_speech = false;
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let event: Value = serde_json::from_str(&text).unwrap();
                let response = match event["type"].as_str().unwrap() {
                    "session.update" => Some(serde_json::json!({"type":"session.updated"})),
                    "input_audio_buffer.append" => {
                        let bytes = BASE64.decode(event["audio"].as_str().unwrap()).unwrap();
                        assert_eq!(bytes.len(), 3_200, "retain the 100 ms PCM packet cadence");
                        received += bytes.len();
                        if bytes.iter().any(|byte| *byte != 0) {
                            assert!(received > silence_bytes);
                            assert!(started.elapsed() >= Duration::from_secs(181));
                            heard_speech = true;
                            Some(
                                serde_json::json!({"type":"conversation.item.input_audio_transcription.completed", "item_id":"late-speech", "transcript":"Speech after silence."}),
                            )
                        } else {
                            None
                        }
                    }
                    "session.finish" => {
                        assert!(heard_speech);
                        Some(serde_json::json!({"type":"session.finished"}))
                    }
                    _ => None,
                };
                if let Some(response) = response {
                    socket
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .unwrap();
                }
                if event["type"] == "session.finish" {
                    break;
                }
                if event["type"] == "input_audio_buffer.append" {
                    tokio::time::advance(Duration::from_millis(100)).await;
                }
            }
            assert_eq!(received, silence_bytes + 3_200);
        });
        let mut wav = canonical_pcm_wav(16_000);
        wav.resize(44 + silence_bytes, 0);
        wav.extend((0..1_600).flat_map(|_| 1_000_i16.to_le_bytes()));
        let data_bytes = (wav.len() - 44) as u32;
        wav[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
        wav[40..44].copy_from_slice(&data_bytes.to_le_bytes());
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &wav).unwrap();
        let config = nexa_core::app_settings::SpeechToTextConfig {
            provider: "alibaba-qwen-realtime".into(),
            api_style: "dashscope_realtime_asr".into(),
            api_key: "fixture".into(),
            base_url: Some(format!("http://{address}/v1")),
            model: "qwen3-asr-flash-realtime".into(),
            ..Default::default()
        };
        assert_eq!(
            tokio::time::timeout(
                Duration::from_secs(220),
                transcribe_realtime_spool(file.path(), &config)
            )
            .await
            .unwrap()
            .unwrap(),
            "Speech after silence."
        );
        assert_eq!(std::fs::read(file.path()).unwrap(), wav);
        server.await.unwrap();
        assert!(started.elapsed() >= Duration::from_secs(181));
        drop(release_clock);
        clock_guard.await.unwrap();
    }

    #[tokio::test]
    async fn dashscope_realtime_spool_uses_snapshot_and_session_finish_contract() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock realtime server");
        let address = listener.local_addr().expect("mock address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept websocket");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("upgrade websocket");
            let mut event_types = Vec::new();
            while let Some(message) = socket.next().await {
                let message = message.expect("client websocket message");
                let Message::Text(text) = message else {
                    continue;
                };
                let event: Value = serde_json::from_str(&text).expect("client event json");
                let event_type = event["type"].as_str().unwrap_or_default().to_string();
                event_types.push(event_type.clone());
                if event_type == "session.update" {
                    assert_eq!(event["session"]["sample_rate"], 16_000);
                    assert_eq!(event["session"]["input_audio_format"], "pcm");
                    assert_eq!(event["session"]["turn_detection"]["type"], "server_vad");
                    assert!(
                        tokio::time::timeout(std::time::Duration::from_millis(25), socket.next())
                            .await
                            .is_err(),
                        "audio must wait for the provider's session.updated acknowledgement"
                    );
                    socket
                        .send(Message::Text(
                            serde_json::json!({"type":"session.updated"})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                }
                if event_type == "input_audio_buffer.append" {
                    let audio = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        event["audio"].as_str().unwrap(),
                    )
                    .unwrap();
                    assert!(
                        audio.len() <= 3_200,
                        "replay packets must stay within 100 ms of 16 kHz PCM"
                    );
                    // A complete VAD utterance arrives while audio is still
                    // uploading, before the user ends the microphone session.
                    socket
                        .send(Message::Text(
                            serde_json::json!({
                                "type": "conversation.item.input_audio_transcription.completed",
                                "item_id": "first-utterance", "transcript": "第一句。"
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .expect("send first utterance during upload");
                }
                if event_type == "session.finish" {
                    socket
                        .send(Message::Text(
                            serde_json::json!({
                                "type": "conversation.item.input_audio_transcription.text",
                                "item_id": "qwen-item",
                                "text": "实时",
                                "stash": "转写"
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .expect("send interim");
                    socket
                        .send(Message::Text(
                            serde_json::json!({
                                "type": "conversation.item.input_audio_transcription.completed",
                                "item_id": "qwen-item",
                                "transcript": "实时转写完成"
                            })
                            .to_string()
                            .into(),
                        ))
                        .await
                        .expect("send final");
                    socket
                        .send(Message::Text(
                            serde_json::json!({ "type": "session.finished" })
                                .to_string()
                                .into(),
                        ))
                        .await
                        .expect("send session finish");
                    break;
                }
            }
            assert_eq!(
                event_types.first().map(String::as_str),
                Some("session.update")
            );
            assert!(event_types
                .iter()
                .any(|value| value == "input_audio_buffer.append"));
            assert_eq!(
                event_types
                    .iter()
                    .filter(|value| *value == "input_audio_buffer.append")
                    .count(),
                3
            );
            assert!(!event_types
                .iter()
                .any(|value| value == "input_audio_buffer.commit"));
            assert_eq!(
                event_types.last().map(String::as_str),
                Some("session.finish")
            );
        });

        let wav_path =
            std::env::temp_dir().join(format!("nexa-qwen-realtime-{}.wav", uuid::Uuid::new_v4()));
        let mut wav = canonical_pcm_wav(16_000);
        wav.resize(44 + 9_600, 0);
        wav[4..8].copy_from_slice(&9_636_u32.to_le_bytes());
        wav[40..44].copy_from_slice(&9_600_u32.to_le_bytes());
        std::fs::write(&wav_path, wav).expect("write mock wav");
        let config = nexa_core::app_settings::SpeechToTextConfig {
            provider: "alibaba_model_studio".to_string(),
            api_style: "dashscope_realtime_asr".to_string(),
            api_key: "test-key".to_string(),
            base_url: Some(format!("http://{address}/api-ws/v1")),
            model: "qwen3-asr-flash-realtime".to_string(),
            language: Some("zh".to_string()),
            ..nexa_core::app_settings::SpeechToTextConfig::default()
        };

        let transcript = transcribe_realtime_spool(&wav_path, &config)
            .await
            .expect("mock realtime transcript");
        let _ = std::fs::remove_file(&wav_path);
        server.await.expect("mock server join");

        assert_eq!(transcript, "第一句。 实时转写完成");
    }

    #[tokio::test]
    async fn realtime_spool_retries_a_closed_connection_without_duplicate_transcripts() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for attempt in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                while let Some(Ok(Message::Text(text))) = socket.next().await {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    match event["type"].as_str().unwrap() {
                        "session.update" => socket
                            .send(Message::Text(
                                serde_json::json!({"type":"session.updated"})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap(),
                        "input_audio_buffer.append" if attempt == 0 => {
                            socket.send(Message::Text(serde_json::json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"old","transcript":"interrupted prefix"}).to_string().into())).await.unwrap();
                            socket.close(None).await.unwrap();
                            break;
                        }
                        "session.finish" => {
                            socket.send(Message::Text(serde_json::json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"new","transcript":"recovered audio"}).to_string().into())).await.unwrap();
                            socket
                                .send(Message::Text(
                                    serde_json::json!({"type":"session.finished"})
                                        .to_string()
                                        .into(),
                                ))
                                .await
                                .unwrap();
                            break;
                        }
                        _ => {}
                    }
                }
            }
        });
        let wav_path =
            std::env::temp_dir().join(format!("nexa-replay-retry-{}.wav", uuid::Uuid::new_v4()));
        std::fs::write(&wav_path, canonical_pcm_wav(16_000)).unwrap();
        let config = nexa_core::app_settings::SpeechToTextConfig {
            provider: "alibaba_model_studio".into(),
            api_style: "dashscope_realtime_asr".into(),
            api_key: "test".into(),
            base_url: Some(format!("http://{address}/api-ws/v1")),
            model: "qwen3-asr-flash-realtime".into(),
            ..Default::default()
        };
        let result = transcribe_realtime_spool(&wav_path, &config).await;
        assert!(
            wav_path.exists(),
            "the transport must not delete the retained recording"
        );
        std::fs::remove_file(wav_path).unwrap();
        assert_eq!(result.unwrap(), "recovered audio");
        server.await.unwrap();
    }
}

#[cfg(test)]
#[path = "realtime_transcription_dashscope_tests.rs"]
mod dashscope_tests;
