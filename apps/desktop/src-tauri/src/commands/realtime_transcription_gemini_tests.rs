use super::*;
#[tokio::test]
async fn gemini_replay_uses_manual_vad_and_drains_each_final_segment() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert!(request
                    .uri()
                    .query()
                    .unwrap()
                    .contains("key=fixture-secret"));
                assert!(request.headers().get("authorization").is_none());
                Ok(response)
            },
        )
        .await
        .unwrap();
        let mut active = false;
        let mut segments = 0;
        let mut bytes = 0;
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let event: Value = serde_json::from_str(&text).unwrap();
            if event.get("setup").is_some() {
                assert_eq!(event["setup"]["model"], "models/gemini-3.5-transcribe-live");
                assert_eq!(
                    event["setup"]["generationConfig"]["responseModalities"],
                    serde_json::json!(["TEXT"])
                );
                assert_eq!(
                    event["setup"]["realtimeInputConfig"]["automaticActivityDetection"]["disabled"],
                    true
                );
                socket
                    .send(Message::Text(
                        serde_json::json!({"setupComplete":{}}).to_string().into(),
                    ))
                    .await
                    .unwrap();
            } else if event.pointer("/realtimeInput/activityStart").is_some() {
                assert!(!active);
                active = true;
            } else if let Some(audio) = event.pointer("/realtimeInput/audio") {
                assert!(active);
                assert_eq!(audio["mimeType"], "audio/pcm;rate=16000");
                bytes += BASE64
                    .decode(audio["data"].as_str().unwrap())
                    .unwrap()
                    .len();
                for text in ["Par", "Partial correction"] {
                    socket.send(Message::Text(serde_json::json!({"serverContent":{"interimInputTranscription":{"text":text}}}).to_string().into())).await.unwrap();
                }
            } else if event.pointer("/realtimeInput/activityEnd").is_some() {
                assert!(active);
                active = false;
                segments += 1;
                assert_eq!(bytes, 16_000);
                bytes = 0;
                socket
                    .send(Message::Text(
                        serde_json::json!({"serverContent":{"turnComplete":true}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                // Input transcription is independent of other server events.
                // Turn completion must not discard a later authoritative final.
                tokio::time::sleep(Duration::from_millis(20)).await;
                socket.send(Message::Text(serde_json::json!({"serverContent":{"inputTranscription":{"text":format!("Sentence {segments}.")}}}).to_string().into())).await.unwrap();
                if segments == 2 {
                    break;
                }
            } else {
                panic!("Unexpected Gemini message: {event}");
            }
        }
        assert_eq!(segments, 2);
    });
    let mut audio = Vec::new();
    for sample in [1000_i16, 2000] {
        audio.extend((0..1600).flat_map(|_| sample.to_le_bytes()));
        audio.extend(std::iter::repeat_n(0, 12_800));
    }
    let mut header = vec![0u8; 44];
    header[..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(audio.len() as u32 + 36).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&1u16.to_le_bytes());
    header[24..28].copy_from_slice(&16_000u32.to_le_bytes());
    header[28..32].copy_from_slice(&32_000u32.to_le_bytes());
    header[32..34].copy_from_slice(&2u16.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&(audio.len() as u32).to_le_bytes());
    header.extend(audio);
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), &header).unwrap();
    let config = nexa_core::app_settings::SpeechToTextConfig {
        provider: "google".into(),
        api_style: "gemini_realtime_asr".into(),
        model: "gemini-3.5-transcribe-live".into(),
        base_url: Some(format!("http://{address}")),
        api_key: "fixture-secret".into(),
        ..Default::default()
    };
    assert_eq!(
        RealtimeDialect::from_config(&config).unwrap(),
        RealtimeDialect::Gemini
    );
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(5),
            transcribe_realtime_spool(file.path(), &config)
        )
        .await
        .unwrap()
        .unwrap(),
        "Sentence 1. Sentence 2."
    );
    assert_eq!(
        std::fs::read(file.path()).unwrap(),
        header,
        "replay must preserve the original recording"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn gemini_quiet_noise_finishes_with_an_authoritative_empty_transcript() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let event: Value = serde_json::from_str(&text).unwrap();
            if event.get("setup").is_some() {
                socket
                    .send(Message::Text(
                        serde_json::json!({"setupComplete":{}}).to_string().into(),
                    ))
                    .await
                    .unwrap();
            } else if event.pointer("/realtimeInput/activityEnd").is_some() {
                socket
                    .send(Message::Text(
                        serde_json::json!({"serverContent":{"turnComplete":true}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
                socket
                    .send(Message::Text(
                        serde_json::json!({"serverContent":{"inputTranscription":{}}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                break;
            }
        }
    });
    let mut wav = super::tests::canonical_pcm_wav(16_000);
    for sample in wav[44..].chunks_exact_mut(2) {
        sample.copy_from_slice(&1_i16.to_le_bytes());
    }
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), &wav).unwrap();
    let config = nexa_core::app_settings::SpeechToTextConfig {
        provider: "google".into(),
        api_style: "gemini_realtime_asr".into(),
        model: "gemini-3.5-transcribe-live".into(),
        base_url: Some(format!("http://{address}")),
        api_key: "fixture".into(),
        ..Default::default()
    };
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(3),
            transcribe_realtime_spool(file.path(), &config)
        )
        .await
        .unwrap()
        .unwrap(),
        ""
    );
    assert_eq!(std::fs::read(file.path()).unwrap(), wav);
    server.await.unwrap();
}
