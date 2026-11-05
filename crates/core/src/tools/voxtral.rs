use super::*;

pub(super) async fn synthesize(
    config: &TextToSpeechConfig,
    text: &str,
    model: &str,
    voice: &str,
) -> Result<GeneratedSpeech, CoreError> {
    if !matches!(model, "voxtral-mini-tts-3-0") {
        return Err(CoreError::InvalidInput(
            "Unsupported Voxtral realtime model.".into(),
        ));
    }
    let mut request = endpoint(&base_url(config))?
        .as_str()
        .into_client_request()
        .map_err(|_| CoreError::InvalidInput("Invalid Voxtral WebSocket URL.".into()))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", config.api_key.trim())
            .parse()
            .map_err(|_| CoreError::InvalidInput("Invalid Voxtral API key.".into()))?,
    );
    let format = normalize_format(&config.output_format);
    if !matches!(format, "wav" | "mp3" | "flac" | "opus") {
        return Err(CoreError::InvalidInput(
            "Unsupported Voxtral realtime audio format.".into(),
        ));
    }
    let wire_error = |_| CoreError::TransientLlm("Voxtral speech connection failed.".into());
    tokio::time::timeout(Duration::from_secs(180), async {
        let (mut socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .map_err(|_| CoreError::TransientLlm("Voxtral connection timed out.".into()))?
        .map_err(wire_error)?;
        let mut stage = 0;
        let mut audio = Vec::new();
        while let Some(message) = socket.next().await {
            let event = match message.map_err(wire_error)? {
                Message::Text(payload) => serde_json::from_str::<Value>(&payload)
                    .map_err(|_| CoreError::Parse("Invalid Voxtral realtime event.".into()))?,
                Message::Ping(payload) => {
                    socket
                        .send(Message::Pong(payload))
                        .await
                        .map_err(wire_error)?;
                    continue;
                }
                Message::Close(_) => break,
                _ => continue,
            };
            let outgoing = match (stage, event["type"].as_str()) {
                (0, Some("session.created")) => {
                    stage = 1;
                    vec![json!({"type":"session.update","session":{
                        "voice":voice,"response_format":format,"streaming_mode":"responsive"
                    }})]
                }
                (1, Some("session.updated")) => {
                    stage = 2;
                    vec![
                        json!({"type":"input_text.append","text":text}),
                        json!({"type":"input_text.end"}),
                    ]
                }
                (2, Some("speech.audio.delta")) => {
                    let encoded = event["audio_data"].as_str().ok_or_else(|| {
                        CoreError::Parse("Voxtral audio delta omitted audio_data.".into())
                    })?;
                    let bytes = decode_base64_audio(encoded, "Voxtral Realtime")?;
                    if audio.len().saturating_add(bytes.len()) > MAX_GENERATED_AUDIO_BYTES {
                        return Err(CoreError::Parse(
                            "Voxtral realtime audio exceeded the output limit.".into(),
                        ));
                    }
                    audio.extend(bytes);
                    continue;
                }
                (2, Some("speech.audio.done")) if !audio.is_empty() => {
                    return Ok(GeneratedSpeech {
                        bytes: audio,
                        media_type: media_type_for_format(format).into(),
                    });
                }
                (_, Some("error")) => {
                    let detail = event
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Voxtral realtime synthesis failed.");
                    return Err(CoreError::Llm(crate::sensitive_data::sanitize_diagnostic(
                        detail,
                        Some(&config.api_key),
                    )));
                }
                _ => continue,
            };
            for message in outgoing {
                socket
                    .send(Message::Text(message.to_string().into()))
                    .await
                    .map_err(wire_error)?;
            }
        }
        Err(CoreError::TransientLlm(
            "Voxtral closed without a complete audio result.".into(),
        ))
    })
    .await
    .map_err(|_| CoreError::TransientLlm("Voxtral realtime synthesis timed out.".into()))?
}

fn endpoint(base_url: &str) -> Result<Url, CoreError> {
    let invalid = || CoreError::InvalidInput("Invalid Voxtral realtime base URL.".into());
    let mut url = Url::parse(base_url).map_err(|_| invalid())?;
    let path = url.path().trim_end_matches('/');
    let scheme = match url.scheme() {
        "https" | "wss"
            if url.host_str() == Some("api.mistral.ai")
                && url.port().is_none()
                && matches!(path, "/v1" | "/v1/audio/speech/realtime") =>
        {
            "wss"
        }
        #[cfg(test)]
        "http" | "ws" if url.host_str() == Some("127.0.0.1") => "ws",
        _ => return Err(invalid()),
    };
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    url.set_scheme(scheme).map_err(|_| invalid())?;
    let path = url.path().trim_end_matches('/');
    if !path.ends_with("/audio/speech/realtime") {
        url.set_path(&format!("{path}/audio/speech/realtime"));
    }
    // Mistral's realtime wire guide uses this canonical model query.
    url.query_pairs_mut()
        .append_pair("model", "voxtral-mini-tts-3-0");
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn waits_for_ack_and_combines_audio_chunks() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for (send, expect) in [
                (json!({"type":"session.created"}), "session.update"),
                (json!({"type":"session.updated"}), "input_text.append"),
            ] {
                socket
                    .send(Message::Text(send.to_string().into()))
                    .await
                    .unwrap();
                let msg = socket.next().await.unwrap().unwrap().into_text().unwrap();
                assert_eq!(serde_json::from_str::<Value>(&msg).unwrap()["type"], expect);
            }
            let end = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&end).unwrap()["type"],
                "input_text.end"
            );
            for send in [
                json!({"type":"speech.audio.delta","audio_data":"UklGRg=="}),
                json!({"type":"speech.audio.delta","audio_data":"V0FWRQ=="}),
                json!({"type":"speech.audio.done"}),
            ] {
                socket
                    .send(Message::Text(send.to_string().into()))
                    .await
                    .unwrap();
            }
        });
        let config = TextToSpeechConfig {
            base_url: Some(base_url),
            api_key: "test-key".into(),
            output_format: "wav".into(),
            ..Default::default()
        };
        let audio = synthesize(&config, "Hello", "voxtral-mini-tts-3-0", "voice_447")
            .await
            .unwrap();
        assert_eq!(audio.bytes, b"RIFFWAVE");
        server.await.unwrap();
    }
}
