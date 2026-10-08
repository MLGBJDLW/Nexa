//! ElevenLabs' dialogue WebSocket transport. Produces the same saved-audio
//! artifact as HTTP synthesis; a per-turn marker is not connection completion.
use super::*;

fn endpoint(config: &TextToSpeechConfig, model: &str) -> Result<Url, CoreError> {
    let mut url = Url::parse(&base_url(config))
        .map_err(|_| CoreError::InvalidInput("Invalid ElevenLabs endpoint.".into()))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CoreError::InvalidInput(
            "ElevenLabs endpoint cannot contain credentials, query, or fragment.".into(),
        ));
    }
    let scheme = match url.scheme() {
        "https" | "wss" => "wss",
        "http" | "ws" if matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) => {
            "ws"
        }
        _ => {
            return Err(CoreError::InvalidInput(
                "ElevenLabs dialogue requires HTTPS (or a loopback development server).".into(),
            ))
        }
    };
    url.set_scheme(scheme)
        .map_err(|_| CoreError::InvalidInput("Invalid speech endpoint scheme.".into()))?;
    url.path_segments_mut()
        .map_err(|_| CoreError::InvalidInput("Invalid speech endpoint path.".into()))?
        .pop_if_empty()
        .push("text-to-dialogue")
        .push("stream-input");
    url.query_pairs_mut()
        .append_pair("model_id", model)
        .append_pair("output_format", "mp3_44100_128");
    Ok(url)
}

pub(super) async fn synthesize_dialogue(
    config: &TextToSpeechConfig,
    text: &str,
    model: &str,
    voice: &str,
    speed: f32,
) -> Result<GeneratedSpeech, CoreError> {
    if (speed - 1.0).abs() > f32::EPSILON {
        return Err(CoreError::InvalidInput(
            "This ElevenLabs dialogue model does not support a speed control; set speed to 1."
                .into(),
        ));
    }
    if config.output_format != "mp3" {
        return Err(CoreError::InvalidInput(
            "ElevenLabs dialogue currently produces MP3 in Nexa.".into(),
        ));
    }
    let mut request = endpoint(config, model)?
        .as_str()
        .into_client_request()
        .map_err(|_| CoreError::InvalidInput("Invalid ElevenLabs WebSocket request.".into()))?;
    request.headers_mut().insert(
        "xi-api-key",
        config
            .api_key
            .trim()
            .parse()
            .map_err(|_| CoreError::InvalidInput("Invalid ElevenLabs API key header.".into()))?,
    );
    let transport_error =
        |_| CoreError::TransientLlm("ElevenLabs dialogue transport failed.".into());
    tokio::time::timeout(Duration::from_secs(300), async {
        crate::privacy::runtime::ensure_invocation_current()?;
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(request))
            .await.map_err(|_| CoreError::TransientLlm("ElevenLabs dialogue connection timed out.".into()))?
            .map_err(transport_error)?;
        for body in [json!({"voices":[voice]}), json!({"inputs":[{"text":text,"voice_id":voice,"new_turn":false}]}), json!({"close_socket":true})] {
            crate::privacy::runtime::ensure_invocation_current()?;
            socket.send(Message::Text(body.to_string().into())).await.map_err(transport_error)?;
        }
        let mut audio = Vec::new();
        while let Some(message) = socket.next().await {
            crate::privacy::runtime::ensure_invocation_current()?;
            match message.map_err(transport_error)? {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).map_err(|_| CoreError::Parse("Invalid ElevenLabs dialogue event.".into()))?;
                    if event.get("error").is_some_and(|value| !value.is_null()) || event.get("code").is_some() {
                        return Err(CoreError::Llm("ElevenLabs rejected the dialogue request; check model access and voice compatibility.".into()));
                    }
                    if let Some(encoded) = event.get("audio").and_then(Value::as_str).filter(|value| !value.is_empty()) {
                        if encoded.len() > MAX_GENERATED_AUDIO_BYTES.saturating_mul(4) / 3 + 4 {
                            return Err(CoreError::Llm("ElevenLabs audio exceeds the response safety limit.".into()));
                        }
                        let bytes = BASE64_STANDARD.decode(encoded).map_err(|_| CoreError::Parse("Invalid ElevenLabs audio encoding.".into()))?;
                        if audio.len().saturating_add(bytes.len()) > MAX_GENERATED_AUDIO_BYTES {
                            return Err(CoreError::Llm("ElevenLabs audio exceeds the response safety limit.".into()));
                        }
                        audio.extend_from_slice(&bytes);
                    }
                    if event.get("is_final").and_then(Value::as_bool) == Some(true) {
                        if audio.is_empty() { return Err(CoreError::Llm("ElevenLabs completed without audio.".into())); }
                        let _ = socket.close(None).await;
                        return Ok(GeneratedSpeech { bytes: audio, media_type: "audio/mpeg".into() });
                    }
                },
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.map_err(transport_error)?,
                Message::Close(_) => break,
                _ => {},
            }
        }
        Err(CoreError::TransientLlm("ElevenLabs dialogue closed before is_final.".into()))
    }).await.map_err(|_| CoreError::TransientLlm("ElevenLabs dialogue timed out.".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn elevenlabs_dialogue_flushes_and_requires_connection_final() {
        for (model, finish) in [
            ("eleven_v4_turbo", true),
            ("eleven_v3_conversational", true),
            ("eleven_v4_turbo", false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_hdr_async(
                    stream,
                    move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                          response| {
                        assert_eq!(request.headers()["xi-api-key"], "fixture");
                        assert_eq!(request.uri().path(), "/v1/text-to-dialogue/stream-input");
                        assert!(request.uri().query().unwrap().contains(model));
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                let mut frames = Vec::new();
                for _ in 0..3 {
                    frames.push(
                        serde_json::from_str::<Value>(
                            socket.next().await.unwrap().unwrap().to_text().unwrap(),
                        )
                        .unwrap(),
                    );
                }
                assert_eq!(frames[0], json!({"voices":["voice"]}));
                assert_eq!(
                    frames[1]["inputs"][0],
                    json!({"text":"你好, speech","voice_id":"voice","new_turn":false})
                );
                assert_eq!(frames[2], json!({"close_socket":true}));
                socket.send(Message::Text(json!({"audio":BASE64_STANDARD.encode(b"ID3"),"is_final_audio_for_turn":true}).to_string().into())).await.unwrap();
                if finish {
                    socket
                        .send(Message::Text(json!({"is_final":true}).to_string().into()))
                        .await
                        .unwrap();
                } else {
                    socket.close(None).await.unwrap();
                }
            });
            let config = TextToSpeechConfig {
                api_key: "fixture".into(),
                output_format: "mp3".into(),
                base_url: Some(format!("http://{address}/v1")),
                ..Default::default()
            };
            let result = synthesize_dialogue(&config, "你好, speech", model, "voice", 1.0).await;
            if finish {
                assert_eq!(result.unwrap().bytes, b"ID3");
            } else {
                assert!(result.unwrap_err().to_string().contains("before is_final"));
            }
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn elevenlabs_dialogue_rejects_unsupported_speed_before_network() {
        assert!(synthesize_dialogue(
            &TextToSpeechConfig::default(),
            "hello",
            "eleven_v4_turbo",
            "voice",
            1.2
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("speed"));
    }
}
