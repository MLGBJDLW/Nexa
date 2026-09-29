use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn read_json_request(socket: &mut tokio::net::TcpStream) -> (String, Value) {
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
        return (
            headers.to_string(),
            serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
        );
    }
}

async fn reply(socket: &mut tokio::net::TcpStream, content_type: &str, body: &str) {
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
}

#[tokio::test]
async fn eleven_v4_routes_dialogue_without_changing_existing_speech_requests() {
    for model in ["eleven_v4", "eleven_flash_v2_5"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, body) = read_json_request(&mut socket).await;
            assert!(headers.to_lowercase().contains("xi-api-key: test"));
            assert!(headers.contains("output_format=mp3_44100_128"));
            assert_eq!(body["model_id"], model);
            if model == "eleven_v4" {
                assert!(headers.starts_with("POST /v1/text-to-dialogue?"));
                assert_eq!(
                    body["inputs"],
                    json!([{"text":"你好. Hello.","voice_id":"Private/Voice"}])
                );
                assert_eq!(body["settings"]["speed"], json!(1.1_f32));
                assert!(body.get("voice_settings").is_none());
            } else {
                assert!(headers.starts_with("POST /v1/text-to-speech/Private%2FVoice?"));
                assert_eq!(body["text"], "你好. Hello.");
                assert_eq!(body["voice_settings"]["speed"], json!(1.1_f32));
                assert!(body.get("inputs").is_none());
            }
            reply(&mut socket, "audio/mpeg", "ID3").await;
        });
        let config = TextToSpeechConfig {
            provider: "elevenlabs".into(),
            api_style: "elevenlabs_speech".into(),
            api_key: "test".into(),
            base_url: Some(format!("http://{address}/v1")),
            output_format: "mp3".into(),
            ..Default::default()
        };
        let generated = synthesize_elevenlabs(
            &reqwest::Client::new(),
            &config,
            "你好. Hello.",
            model,
            "Private/Voice",
            1.1,
        )
        .await
        .unwrap();
        assert_eq!(generated.bytes, b"ID3");
        assert_eq!(generated.media_type, "audio/mpeg");
        server.await.unwrap();
    }
}

#[tokio::test]
async fn minimax_honors_container_and_reports_provider_status_errors() {
    for format in ["wav", "flac", "error"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, body) = read_json_request(&mut socket).await;
            assert!(headers.starts_with("POST /v1/t2a_v2 "));
            assert_eq!(body["output_format"], "hex");
            assert_eq!(body["stream"], false);
            assert_eq!(
                body["audio_setting"]["format"],
                if format == "error" { "mp3" } else { format }
            );
            let response = if format == "error" {
                json!({"base_resp":{"status_code":1008,"status_msg":"voice is unavailable"}})
            } else {
                json!({"base_resp":{"status_code":0},"data":{"audio":"494433"}})
            };
            reply(&mut socket, "application/json", &response.to_string()).await;
        });
        let config = TextToSpeechConfig {
            provider: "minimax".into(),
            api_style: "minimax_speech".into(),
            api_key: "test".into(),
            base_url: Some(format!("http://{address}/v1")),
            output_format: if format == "error" { "mp3" } else { format }.into(),
            ..Default::default()
        };
        let result = synthesize_minimax(
            &reqwest::Client::new(),
            &config,
            "Hello.",
            "speech-2.8-turbo",
            "male-qn-qingse",
            1.0,
        )
        .await;
        if format == "error" {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("voice is unavailable"));
        } else {
            let generated = result.unwrap();
            assert_eq!(generated.bytes, b"ID3");
            assert_eq!(generated.media_type, media_type_for_format(format));
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn groq_preserves_documented_speed_field_and_wav_format() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (_, body) = read_json_request(&mut socket).await;
        assert_eq!(body["speed"], 1.0);
        assert_eq!(body["response_format"], "wav");
        reply(&mut socket, "audio/wav", "RIFF").await;
    });
    let config = TextToSpeechConfig {
        provider: "groq".into(),
        api_key: "test".into(),
        base_url: Some(format!("http://{address}/v1")),
        ..Default::default()
    };
    synthesize_openai(
        &reqwest::Client::new(),
        &config,
        "Hello.",
        "canopylabs/orpheus-v1-english",
        "hannah",
        1.0,
    )
    .await
    .unwrap();
    server.await.unwrap();
}
