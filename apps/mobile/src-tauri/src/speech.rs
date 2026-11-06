use base64::Engine;
use nexa_core::app_settings::{SpeechToTextConfig, TextToSpeechConfig};
use serde_json::{json, Value};
use std::sync::OnceLock;
use tauri::{ipc::Channel, AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::platform;

pub const SAVED_KEY: &str = "__NEXA_SAVED_KEY__";
pub fn apply_input_language(
    config: &mut SpeechToTextConfig,
    saved: Option<&str>,
    requested: Option<&str>,
) {
    let explicit = saved.filter(|value| !value.trim().is_empty());
    let existing = config
        .language
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    config.language = explicit
        .or(existing)
        .or_else(|| requested.filter(|value| !value.trim().is_empty()))
        .map(str::to_owned);
}
pub fn supports_input(api_style: &str) -> bool {
    matches!(
        api_style,
        "openai_transcription"
            | "dashscope_asr"
            | "dashscope_audio_asr"
            | "gemini_interactions_asr"
            | "meta_asr"
    )
}
pub fn validate_input(config: &SpeechToTextConfig) -> Result<(), String> {
    if !supports_input(&config.api_style) || !config.is_configured() {
        return Err("speech_configuration".into());
    }
    crate::validate_provider_url(config.base_url.as_deref().ok_or("speech_configuration")?)
}
pub fn supports_output(api_style: &str) -> bool {
    matches!(
        api_style,
        "openai_speech"
            | "elevenlabs_speech"
            | "mistral_speech"
            | "mistral_realtime_speech"
            | "minimax_speech"
            | "azure_speech"
            | "dashscope_speech"
            | "dashscope_audio_generation"
            | "gemini_interactions_speech"
    )
}
pub fn validate_output(config: &TextToSpeechConfig) -> Result<(), String> {
    if !supports_output(&config.api_style) || !config.is_configured() {
        return Err("speech_configuration".into());
    }
    crate::validate_provider_url(config.base_url.as_deref().ok_or("speech_configuration")?)
}

#[derive(Default)]
pub struct SpeechRuntime {
    input: Mutex<Option<(uuid::Uuid, CancellationToken)>>,
    output: Mutex<Option<(uuid::Uuid, CancellationToken)>>,
    progress: OnceLock<Channel<Value>>,
}
impl SpeechRuntime {
    pub async fn cancel(&self, output: bool) {
        let slot = if output { &self.output } else { &self.input };
        if let Some((_, token)) = slot.lock().await.as_ref() {
            token.cancel();
        }
    }
    pub async fn cancel_input(&self, request_id: Option<uuid::Uuid>) {
        if let Some((owner, token)) = self.input.lock().await.as_ref() {
            if request_id.is_none_or(|id| id == *owner) {
                token.cancel();
            }
        }
    }
    pub async fn busy(&self) -> bool {
        self.input.lock().await.is_some() || self.output.lock().await.is_some()
    }
    pub async fn listen(
        &self,
        app: &AppHandle,
        mut args: Value,
        config: Option<SpeechToTextConfig>,
        request_id: Option<uuid::Uuid>,
    ) -> Result<Value, String> {
        let id = request_id.unwrap_or_else(uuid::Uuid::new_v4);
        let token = CancellationToken::new();
        {
            let mut slot = self.input.lock().await;
            if slot.is_some() {
                return Err("speech_busy".into());
            }
            *slot = Some((id, token.clone()));
        }
        args["action"] = json!("listen");
        if request_id.is_some() {
            // Android keeps native channels for the process lifetime. Share one
            // across recordings; the UI accepts events only for its current ID.
            let channel = self.progress.get_or_init(|| {
                let app = app.clone();
                Channel::new(move |event| {
                    app.emit_to("main", "mobile:dictation", event.deserialize::<Value>()?)
                })
            });
            args["progress"] = json!(channel);
        }
        let result = tokio::select! {
            _ = token.cancelled() => Err("speech_cancelled".into()),
            result = async {
                let recorded = platform::call(app, "speech", args).await?;
                let Some(config) = config else { return Ok(recorded) };
                let encoded = recorded["audioBase64"].as_str().ok_or("speech_failed")?;
                if encoded.len() > 2_600_000 { return Err("speech_failed".into()); }
                let audio = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| "speech_failed")?;
                let text = tokio::time::timeout(std::time::Duration::from_secs(90), nexa_core::speech_to_text::transcribe_cloud_wav(audio, &config)).await
                    .map_err(|_| "speech_timeout")?.map_err(|_| "speech_cloud_failed")?;
                if text.trim().is_empty() { return Err("speech_no_match".into()); }
                Ok(json!({"text":text}))
            } => result,
        };
        if token.is_cancelled() {
            let _ = platform::call(
                app,
                "speech",
                json!({"action":"cancel", "requestId":request_id}),
            )
            .await;
        }
        let mut slot = self.input.lock().await;
        if slot.as_ref().is_some_and(|(owner, _)| *owner == id) {
            *slot = None;
        }
        result
    }
    pub async fn speak(
        &self,
        app: &AppHandle,
        args: Value,
        config: Option<TextToSpeechConfig>,
    ) -> Result<Value, String> {
        let id = uuid::Uuid::new_v4();
        let token = CancellationToken::new();
        {
            let mut slot = self.output.lock().await;
            if slot.is_some() {
                return Err("speech_busy".into());
            }
            *slot = Some((id, token.clone()));
        }
        let result = tokio::select! {
            _ = token.cancelled() => Err("speech_cancelled".into()),
            result = async {
                let Some(config) = config else { return platform::call(app, "speech", args).await };
                let text = args["text"].as_str().unwrap_or_default();
                let audio = tokio::time::timeout(std::time::Duration::from_secs(90), nexa_core::tools::text_to_speech_tool::synthesize_speech_audio(&config, text, None, None, None)).await
                    .map_err(|_| "speech_timeout")?.map_err(|_| "speech_cloud_failed")?;
                if audio.bytes.len() > 8 * 1024 * 1024 { return Err("speech_failed".into()); }
                platform::call(app, "speech", json!({"action":"play","audioBase64":base64::engine::general_purpose::STANDARD.encode(audio.bytes),"mimeType":audio.media_type})).await
            } => result,
        };
        if token.is_cancelled() {
            let _ = platform::call(app, "speech", json!({"action":"stop"})).await;
        }
        let mut slot = self.output.lock().await;
        if slot.as_ref().is_some_and(|(owner, _)| *owner == id) {
            *slot = None;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn saved_speech_language_precedes_the_chat_ui_locale() {
        let mut config = SpeechToTextConfig::default();
        apply_input_language(&mut config, Some("ja"), Some("zh-CN"));
        assert_eq!(config.language.as_deref(), Some("ja"));
        apply_input_language(&mut config, Some("en"), Some("zh-CN"));
        assert_eq!(config.language.as_deref(), Some("en"));
        config.language = None;
        apply_input_language(&mut config, Some(""), Some("zh-CN"));
        assert_eq!(config.language.as_deref(), Some("zh-CN"));
    }

    #[tokio::test]
    async fn phone_cloud_services_use_the_saved_account_and_shared_http_adapters() {
        let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", server.local_addr().unwrap());
        let task = tokio::spawn(async move {
            for (path, response, mime) in [
                (
                    "/v1/audio/transcriptions",
                    "{\"text\":\"你好，手机自配语音\"}".as_bytes(),
                    "application/json",
                ),
                (
                    "/v1/audio/speech",
                    b"RIFFprovider-audio".as_slice(),
                    "audio/wav",
                ),
            ] {
                let (mut socket, _) = server.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() < end + 4 + length {
                        continue;
                    }
                    assert!(headers.starts_with(&format!("POST {path} ")));
                    assert!(headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer fixture-mobile-key"));
                    let body = String::from_utf8_lossy(&request[end + 4..]);
                    assert!(body.contains(if path.ends_with("transcriptions") {
                        "fixture-input-model"
                    } else {
                        "fixture-output-model"
                    }));
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",response.len()).as_bytes()).await.unwrap();
                    socket.write_all(response).await.unwrap();
                    break;
                }
            }
        });
        let account: nexa_core::connection_sync::SharedModelConnection =
            serde_json::from_value(json!({
                "id":"saved", "name":"Saved server", "provider":"custom", "baseUrl":base,
                "model":"chat", "apiKey":"fixture-mobile-key"
            }))
            .unwrap();
        let mut config = nexa_core::app_settings::AppConfig {
            speech_to_text: SpeechToTextConfig {
                connection_id: Some(account.id.clone()),
                provider: "custom".into(),
                api_style: "openai_transcription".into(),
                base_url: Some(base.clone()),
                model: "fixture-input-model".into(),
                ..Default::default()
            },
            text_to_speech: TextToSpeechConfig {
                connection_id: Some(account.id.clone()),
                provider: "open_ai".into(),
                api_style: "openai_speech".into(),
                base_url: Some(base),
                model: "fixture-output-model".into(),
                voice: "fixture-voice".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        crate::speech_connections::resolve(&mut config, &[&account], false);
        validate_input(&config.speech_to_text).unwrap();
        let text = nexa_core::speech_to_text::transcribe_cloud_wav(
            b"RIFFfixture".to_vec(),
            &config.speech_to_text,
        )
        .await
        .unwrap();
        assert_eq!(text, "你好，手机自配语音");
        validate_output(&config.text_to_speech).unwrap();
        let audio = nexa_core::tools::text_to_speech_tool::synthesize_speech_audio(
            &config.text_to_speech,
            &text,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(audio.bytes, b"RIFFprovider-audio");
        task.await.unwrap();
    }

    #[test]
    fn phone_cloud_mode_rejects_desktop_executables_and_insecure_remote_keys() {
        assert!(validate_input(&SpeechToTextConfig::default()).is_err());
        let mut input = SpeechToTextConfig {
            provider: "custom".into(),
            api_style: "openai_transcription".into(),
            api_key: "fixture".into(),
            model: "whisper-1".into(),
            base_url: Some("http://public.example/v1".into()),
            ..Default::default()
        };
        assert!(validate_input(&input).is_err());
        input.base_url = Some("https://public.example/v1".into());
        assert!(validate_input(&input).is_ok());
        input.api_style = "openai_realtime_transcription".into();
        assert!(validate_input(&input).is_err());
        let output = TextToSpeechConfig {
            api_style: "sherpa_onnx".into(),
            ..Default::default()
        };
        assert!(validate_output(&output).is_err());
    }
}
