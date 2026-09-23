use super::*;

#[tokio::test]
async fn dashscope_tts_waits_for_ack_collects_binary_and_requires_finish() {
    for outcome in ["task-finished", "task-failed", "close"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let run: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(run["header"]["action"], "run-task");
            assert_eq!(run["payload"]["model"], "qwen-audio-3.0-tts-flash");
            assert_eq!(run["payload"]["parameters"]["voice"], "longanhuan_v3.6");
            assert_eq!(run["payload"]["parameters"]["format"], "mp3");
            assert_eq!(run["payload"]["parameters"]["rate"], 1.25);
            let id = run["header"]["task_id"].as_str().unwrap();
            socket
                .send(Message::Text(
                    json!({"header":{"event":"task-started","task_id":id}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            let mut text = String::new();
            loop {
                let event: Value =
                    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(event["header"]["task_id"], id);
                if event["header"]["action"] == "finish-task" {
                    break;
                }
                assert_eq!(event["header"]["action"], "continue-task");
                text.push_str(event["payload"]["input"]["text"].as_str().unwrap());
            }
            assert_eq!(text, "你好、こんにちは 🌍".repeat(100));
            socket
                .send(Message::Binary(b"ID3".to_vec().into()))
                .await
                .unwrap();
            if outcome != "close" {
                socket.send(Message::Text(json!({"header":{"event":outcome,"task_id":id,"error_message":"voice unavailable"}}).to_string().into())).await.unwrap();
            } else {
                socket.close(None).await.unwrap();
            }
        });
        let config = TextToSpeechConfig {
            api_style: "dashscope_speech".into(),
            api_key: "test".into(),
            base_url: Some(format!("http://{address}/api/v1/services/audio/tts")),
            output_format: "mp3".into(),
            ..Default::default()
        };
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            synthesize_dashscope_stream(
                &config,
                &"你好、こんにちは 🌍".repeat(100),
                "qwen-audio-3.0-tts-flash",
                "longanhuan_v3.6",
                1.25,
            ),
        )
        .await
        .unwrap();
        match outcome {
            "task-finished" => assert_eq!(result.unwrap().bytes, b"ID3"),
            "task-failed" => assert!(result
                .unwrap_err()
                .to_string()
                .contains("voice unavailable")),
            _ => assert!(result
                .unwrap_err()
                .to_string()
                .contains("before task-finished")),
        }
        server.await.unwrap();
    }
}

#[test]
fn dashscope_audio_generation_has_no_voice_and_enforces_model_limits() {
    let config = TextToSpeechConfig { api_style:"dashscope_audio_generation".into(), model:"qwen-audio-3.1-tts-next".into(), api_key:"test".into(), base_url:Some("https://workspace.cn-beijing.maas.aliyuncs.com/api/v1/services/audio/tts/SpeechSynthesizer".into()), voice:String::new(), output_format:"wav".into(), ..Default::default() };
    assert!(config.is_configured());
    let body =
        dashscope_audio_generation_body(&config, "A calm voice says: 你好。", &config.model, 1.0)
            .unwrap();
    assert_eq!(body["input"]["text_prompt"], "A calm voice says: 你好。");
    assert!(body["input"].get("voice").is_none());
    assert!(
        dashscope_audio_generation_body(&config, &"中".repeat(3001), &config.model, 1.0).is_err()
    );
    assert!(dashscope_audio_generation_body(
        &TextToSpeechConfig {
            output_format: "opus".into(),
            ..config.clone()
        },
        "test",
        &config.model,
        1.0
    )
    .is_err());
    assert!(!TextToSpeechConfig {
        base_url: None,
        ..config
    }
    .is_configured());
}
