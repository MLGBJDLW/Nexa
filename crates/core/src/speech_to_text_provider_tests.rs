use super::*;
use tokio::io::AsyncWriteExt;

#[test]
fn multipart_dialects_preserve_custom_and_legacy_models() {
    let mut config = SpeechToTextConfig {
        provider: "open_ai".into(),
        model: "gpt-transcribe".into(),
        api_style: "openai_transcription".into(),
        base_url: Some("https://api.openai.com/v1".into()),
        language: Some("auto,zh;en / zh".into()),
        ..Default::default()
    };
    assert_eq!(
        transcription_form_fields(&config),
        vec![
            ("model", "gpt-transcribe".into()),
            ("languages[]", "zh".into()),
            ("languages[]", "en".into()),
        ]
    );
    config.provider = "custom".into();
    assert!(transcription_form_fields(&config)
        .iter()
        .any(|(key, _)| *key == "language"));
    config.provider = "open_ai".into();
    config.base_url = Some("https://private.example/v1".into());
    assert!(transcription_form_fields(&config)
        .iter()
        .any(|(key, _)| *key == "language"));
    config.base_url = Some("https://api.openai.com/v1/audio/transcriptions".into());
    assert!(transcription_form_fields(&config)
        .iter()
        .any(|(key, _)| *key == "languages[]"));
    config.model = "gpt-4o-mini-transcribe".into();
    assert!(transcription_form_fields(&config).contains(&("response_format", "json".into())));
    config.provider = "siliconflow".into();
    config.base_url = Some("https://api.siliconflow.cn/v1".into());
    config.model = "FunAudioLLM/SenseVoiceSmall".into();
    assert_eq!(
        transcription_form_fields(&config),
        vec![("model", config.model.clone())]
    );
}

#[tokio::test]
async fn gpt_transcribe_uses_verified_multipart_dialect_for_both_call_paths() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() < end + 4 + length {
                    continue;
                }
                assert!(headers.starts_with("POST /v1/audio/transcriptions "));
                let body = String::from_utf8_lossy(&bytes[end + 4..end + 4 + length]);
                assert!(body.contains("name=\"model\"\r\n\r\ngpt-transcribe"));
                assert!(body.contains("name=\"languages[]\"\r\n\r\nzh"));
                assert!(body.contains("name=\"languages[]\"\r\n\r\nen"));
                assert!(!body.contains("name=\"language\""));
                assert!(!body.contains("name=\"response_format\""));
                let output = r#"{"text":"你好, hello.","languages":[{"code":"zh"},{"code":"en"}]}"#;
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", output.len(), output).as_bytes()).await.unwrap();
                break;
            }
        }
    });
    let config = SpeechToTextConfig {
        provider: "open_ai".into(),
        api_style: "openai_transcription".into(),
        model: "gpt-transcribe".into(),
        api_key: "test".into(),
        base_url: Some("https://api.openai.com/v1".into()),
        language: Some("zh,en".into()),
        ..Default::default()
    };
    let endpoint = format!("http://{address}/v1/audio/transcriptions");
    assert_eq!(
        transcribe_openai_compatible_wav_at_endpoint(b"RIFF".to_vec(), &config, &endpoint)
            .await
            .unwrap(),
        "你好, hello."
    );
    let result = tokio::task::spawn_blocking(move || {
        transcribe_openai_compatible_wav_blocking_at_endpoint(b"RIFF", &config, &endpoint)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result, "你好, hello.");
    server.await.unwrap();
}
