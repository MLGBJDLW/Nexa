//! Native file-ASR contracts; managed segmentation is owned by speech_to_text.
use super::*;
use futures::StreamExt;
use serde_json::{json, Value};
use std::io::Read;

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

fn meta_request(config: &SpeechToTextConfig, audio: &[u8]) -> Result<Value, CoreError> {
    let reader = hound::WavReader::new(std::io::Cursor::new(audio)).map_err(|error| {
        CoreError::InvalidInput(format!("Meta transcription requires a WAV: {error}"))
    })?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
        || !matches!(spec.sample_rate, 16_000 | 24_000)
        || u64::from(reader.duration()) > u64::from(spec.sample_rate) * 600
        || audio.len() > 32 * 1024 * 1024
    {
        return Err(CoreError::InvalidInput(
            "Meta ASR accepts mono PCM16 WAV at 16/24 kHz, up to ten minutes and 32 MB per segment"
                .into(),
        ));
    }
    let mut body = json!({"model":config.model.trim(),"audioEncoding":"WAV","mode":"PUSH_TO_TALK"});
    let names: Vec<_> = transcription_language_hints(config.language.as_deref())
        .iter()
        .filter_map(|code| {
            match code
                .split('-')
                .next()
                .unwrap_or(code)
                .to_ascii_lowercase()
                .as_str()
            {
                "ar" => Some("Arabic"),
                "bn" => Some("Bengali"),
                "nl" => Some("Dutch"),
                "en" => Some("English"),
                "fr" => Some("French"),
                "de" => Some("German"),
                "he" => Some("Hebrew"),
                "hi" => Some("Hindi"),
                "id" => Some("Indonesian"),
                "it" => Some("Italian"),
                "ja" => Some("Japanese"),
                "kn" => Some("Kannada"),
                "ko" => Some("Korean"),
                "ms" => Some("Malay"),
                "zh" => Some("Mandarin Chinese"),
                "mr" => Some("Marathi"),
                "pl" => Some("Polish"),
                "pt" => Some("Portuguese"),
                "es" => Some("Spanish"),
                "tl" | "fil" => Some("Tagalog"),
                "ta" => Some("Tamil"),
                "te" => Some("Telugu"),
                "th" => Some("Thai"),
                "tr" => Some("Turkish"),
                "vi" => Some("Vietnamese"),
                _ => None,
            }
        })
        .collect();
    if !names.is_empty() {
        body["languageBias"] = json!(names);
    }
    Ok(body)
}

fn google_request(config: &SpeechToTextConfig, audio: &[u8]) -> Value {
    let languages = transcription_language_hints(config.language.as_deref());
    crate::google_audio::transcription_request(
        config.model.trim(),
        &BASE64.encode(audio),
        &languages,
    )
}

fn endpoint(config: &SpeechToTextConfig) -> String {
    let base = config
        .base_url
        .as_deref()
        .unwrap_or_default()
        .trim()
        .trim_end_matches('/');
    if config.api_style == "gemini_interactions_asr" {
        crate::google_audio::endpoint(base)
    } else if base.ends_with("/asr/transcribe") {
        base.into()
    } else {
        format!("{base}/asr/transcribe")
    }
}

fn decode(
    config: &SpeechToTextConfig,
    status: reqwest::StatusCode,
    bytes: &[u8],
) -> Result<String, CoreError> {
    if !status.is_success() {
        return Err(CoreError::Llm(format!(
            "Speech provider returned {status}: {}",
            String::from_utf8_lossy(bytes)
                .chars()
                .take(500)
                .collect::<String>()
        )));
    }
    let value: Value = serde_json::from_slice(bytes)?;
    if config.api_style == "gemini_interactions_asr" {
        crate::google_audio::transcript(&value)
    } else {
        value["transcript"]
            .as_str()
            .map(|text| text.trim().to_owned())
            .ok_or_else(|| CoreError::Parse("Meta ASR response has no transcript".into()))
    }
}

pub(super) async fn transcribe(
    audio: Vec<u8>,
    config: &SpeechToTextConfig,
) -> Result<String, CoreError> {
    let request = async_client()
        .post(endpoint(config))
        .header("Accept", "application/json");
    let response = if config.api_style == "gemini_interactions_asr" {
        request
            .header("x-goog-api-key", config.api_key.trim())
            .json(&google_request(config, &audio))
            .send()
            .await
    } else {
        let metadata = reqwest::multipart::Part::text(meta_request(config, &audio)?.to_string())
            .mime_str("application/json")
            .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        let file = reqwest::multipart::Part::bytes(audio)
            .file_name("voice.wav")
            .mime_str("audio/wav")
            .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        request
            .bearer_auth(config.api_key.trim())
            .multipart(
                reqwest::multipart::Form::new()
                    .part("request", metadata)
                    .part("audio", file),
            )
            .send()
            .await
    }
    .map_err(|error| CoreError::Llm(format!("Native speech request failed: {error}")))?;
    let status = response.status();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| CoreError::Llm(error.to_string()))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(CoreError::Parse(
                "Speech response exceeds its transport envelope".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    decode(config, status, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn native_file_asr_contracts_work_for_dictation_and_video_ingestion() {
        for google in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let audio = [
                super::super::pcm16_wav_header(16_000, 1, 320)
                    .unwrap()
                    .to_vec(),
                vec![0; 320],
            ]
            .concat();
            let expected_audio = audio.clone();
            let server = tokio::spawn(async move {
                for _ in 0..2 {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut bytes = Vec::new();
                    let mut buffer = [0; 4096];
                    loop {
                        let count = socket.read(&mut buffer).await.unwrap();
                        assert!(count > 0);
                        bytes.extend_from_slice(&buffer[..count]);
                        let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n")
                        else {
                            continue;
                        };
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length:")
                                    .map(|value| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if bytes.len() < end + 4 + length {
                            continue;
                        }
                        let body = &bytes[end + 4..end + 4 + length];
                        if google {
                            assert!(headers.starts_with("post /v1beta/interactions "));
                            assert!(headers.contains("x-goog-api-key: fixture"));
                            assert!(!headers.contains("authorization:"));
                            let value: Value = serde_json::from_slice(body).unwrap();
                            assert_eq!(
                                BASE64
                                    .decode(value["input"][0]["data"].as_str().unwrap())
                                    .unwrap(),
                                expected_audio
                            );
                            assert_eq!(
                                value["generation_config"]["transcription_config"]
                                    ["language_codes"][0],
                                "zh"
                            );
                            assert_eq!(value["store"], false);
                        } else {
                            assert!(headers.starts_with("post /v1/asr/transcribe "));
                            assert!(headers.contains("authorization: bearer fixture"));
                            let body = String::from_utf8_lossy(body);
                            assert!(body.contains("name=\"request\""));
                            assert!(body.contains("application/json"));
                            assert!(body.contains("\"audioEncoding\":\"WAV\""));
                            assert!(body
                                .contains("\"languageBias\":[\"Mandarin Chinese\",\"English\"]"));
                            assert!(body.contains("name=\"audio\""));
                            assert!(!body.contains("name=\"file\""));
                        }
                        let response = if google { json!({"status":"completed","steps":[{"type":"model_output","content":[{"type":"text","text":"Complete transcript."}]}]}) }
                            else { json!({"transcript":"Complete transcript."}) }.to_string();
                        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).as_bytes()).await.unwrap();
                        break;
                    }
                }
            });
            let config = SpeechToTextConfig {
                provider: if google { "google" } else { "meta" }.into(),
                api_style: if google {
                    "gemini_interactions_asr"
                } else {
                    "meta_asr"
                }
                .into(),
                model: if google {
                    "gemini-3.5-transcribe"
                } else {
                    "muse-voice-transcribe-1.0"
                }
                .into(),
                api_key: "fixture".into(),
                language: Some("zh,en".into()),
                base_url: Some(format!(
                    "http://{address}/{}",
                    if google { "v1beta" } else { "v1" }
                )),
                ..Default::default()
            };
            assert!(config.is_configured());
            assert_eq!(
                transcribe(audio.clone(), &config).await.unwrap(),
                "Complete transcript."
            );
            assert_eq!(
                tokio::task::spawn_blocking(move || transcribe_blocking(&audio, &config))
                    .await
                    .unwrap()
                    .unwrap(),
                "Complete transcript."
            );
            server.await.unwrap();
        }
    }
}

pub(super) fn transcribe_blocking(
    audio: &[u8],
    config: &SpeechToTextConfig,
) -> Result<String, CoreError> {
    let request = blocking_client()
        .post(endpoint(config))
        .header("Accept", "application/json");
    let response = if config.api_style == "gemini_interactions_asr" {
        request
            .header("x-goog-api-key", config.api_key.trim())
            .json(&google_request(config, audio))
            .send()
    } else {
        let metadata =
            reqwest::blocking::multipart::Part::text(meta_request(config, audio)?.to_string())
                .mime_str("application/json")
                .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        let file = reqwest::blocking::multipart::Part::bytes(audio.to_vec())
            .file_name("voice.wav")
            .mime_str("audio/wav")
            .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        request
            .bearer_auth(config.api_key.trim())
            .multipart(
                reqwest::blocking::multipart::Form::new()
                    .part("request", metadata)
                    .part("audio", file),
            )
            .send()
    }
    .map_err(|error| CoreError::Llm(format!("Native speech request failed: {error}")))?;
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(CoreError::Parse(
            "Speech response exceeds its transport envelope".into(),
        ));
    }
    decode(config, status, &bytes)
}
