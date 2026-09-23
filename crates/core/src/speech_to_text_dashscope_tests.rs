use super::*;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn qwen_audio_http_transcribes_async_and_ingestion_workers() {
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
                assert!(headers.starts_with("POST /generation "));
                let body: Value =
                    serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                assert_eq!(body["model"], "qwen-audio-3.1-asr-flash");
                assert_eq!(body["parameters"]["format"], "wav");
                assert_eq!(body["parameters"]["language_hints"], json!(["zh", "en"]));
                assert!(
                    body["input"]["messages"][0]["content"][0]["input_audio"]["data"]
                        .as_str()
                        .unwrap()
                        .starts_with("data:audio/wav;base64,")
                );
                assert!(body.get("asr_options").is_none());
                let output = json!({"output":{"text":"第一句。Hello.第二句。", "sentence":{"text":"第二句。"}}}).to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", output.len(), output).as_bytes()).await.unwrap();
                break;
            }
        }
    });
    let config = SpeechToTextConfig {
        provider: "alibaba_model_studio".into(),
        api_style: "dashscope_audio_asr".into(),
        model: "qwen-audio-3.1-asr-flash".into(),
        api_key: "test".into(),
        base_url: Some(format!("http://{address}/generation")),
        language: Some("zh,en".into()),
        ..Default::default()
    };
    assert_eq!(
        transcribe_cloud_wav(b"RIFF".to_vec(), &config)
            .await
            .unwrap(),
        "第一句。Hello.第二句。"
    );
    let blocking = tokio::task::spawn_blocking(move || {
        transcribe_cloud_wav_blocking(b"RIFF".to_vec(), &config)
    });
    assert_eq!(blocking.await.unwrap().unwrap(), "第一句。Hello.第二句。");
    server.await.unwrap();
}
