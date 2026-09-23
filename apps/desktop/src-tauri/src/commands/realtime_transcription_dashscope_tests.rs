use super::*;
use serde_json::json;

async fn server(listener: tokio::net::TcpListener, fail: bool) {
    let (stream, _) = listener.accept().await.unwrap();
    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
    let run: Value =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(run["header"]["action"], "run-task");
    assert_eq!(run["payload"]["parameters"]["sample_rate"], 16000);
    assert_eq!(run["payload"]["parameters"]["format"], "pcm");
    let id = run["header"]["task_id"].as_str().unwrap();
    socket
        .send(Message::Text(
            json!({"header":{"event":"task-started","task_id":id}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut received = 0;
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Binary(audio) => {
                received += audio.len();
            }
            Message::Text(text) => {
                let finish: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(finish["header"]["action"], "finish-task");
                assert_eq!(finish["header"]["task_id"], id);
                break;
            }
            _ => panic!("Expected PCM or finish-task"),
        }
    }
    assert_eq!(received, 3200);
    for (sentence_id, text, final_result) in
        [(1, "你", false), (1, "你好。", true), (2, "Hello.", true)]
    {
        socket.send(Message::Text(json!({"header":{"event":"result-generated","task_id":id},"payload":{"output":{"sentence":{"sentence_id":sentence_id,"text":text,"sentence_end":final_result}}}}).to_string().into())).await.unwrap();
    }
    // Heartbeats must not become text, and a different task must not finish ours.
    socket.send(Message::Text(json!({"header":{"event":"result-generated","task_id":id},"payload":{"output":{"sentence":{"sentence_id":0,"heartbeat":true,"text":"heartbeat"}}}}).to_string().into())).await.unwrap();
    socket
        .send(Message::Text(
            json!({"header":{"event":"task-finished","task_id":"stale"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket.send(Message::Text(json!({"header":{"event":if fail {"task-failed"} else {"task-finished"},"task_id":id,"error_message":"quota exhausted"}}).to_string().into())).await.unwrap();
}

#[tokio::test]
async fn dashscope_task_protocol_covers_live_capture_replay_and_failure() {
    for mode in ["live", "replay", "failure"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(server(listener, mode == "failure"));
        let config = nexa_core::app_settings::SpeechToTextConfig {
            provider: "alibaba_model_studio".into(),
            api_style: "dashscope_streaming_asr".into(),
            model: "qwen-audio-3.1-asr-flash-streaming".into(),
            api_key: "test".into(),
            base_url: Some(format!("http://{address}/api-ws/v1")),
            ..Default::default()
        };
        let result = if mode == "replay" {
            let path = std::env::temp_dir().join(format!("nexa-qwen-audio-{}.wav", Uuid::new_v4()));
            let mut wav = vec![0u8; 44 + 3200];
            wav[..4].copy_from_slice(b"RIFF");
            wav[8..16].copy_from_slice(b"WAVEfmt ");
            wav[16..20].copy_from_slice(&16u32.to_le_bytes());
            wav[20..22].copy_from_slice(&1u16.to_le_bytes());
            wav[22..24].copy_from_slice(&1u16.to_le_bytes());
            wav[24..28].copy_from_slice(&16000u32.to_le_bytes());
            wav[34..36].copy_from_slice(&16u16.to_le_bytes());
            wav[36..40].copy_from_slice(b"data");
            wav[40..44].copy_from_slice(&3200u32.to_le_bytes());
            std::fs::write(&path, wav).unwrap();
            let result = transcribe_realtime_spool(&path, &config).await;
            std::fs::remove_file(path).unwrap();
            result
        } else {
            let state = RealtimeTranscriptionState::default();
            let id = start_realtime_session(
                config,
                &state,
                None,
                RealtimeEventSink {
                    frontend: Arc::new(|_| {}),
                    live: None,
                },
            )
            .await
            .unwrap();
            let sender = session_sender(&state, &id).await.unwrap();
            sender
                .send(RealtimeCommand::Append(vec![0; 3200]))
                .await
                .unwrap();
            let (tx, rx) = oneshot::channel();
            sender.send(RealtimeCommand::Finish(tx)).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), rx)
                .await
                .unwrap()
                .unwrap()
        };
        if mode == "failure" {
            assert_eq!(result.unwrap_err(), "quota exhausted");
        } else {
            let text = result.unwrap();
            assert!(text.contains("你好。") && text.contains("Hello."));
            assert!(!text.contains("heartbeat"));
            assert_eq!(text.matches("你好。").count(), 1);
        }
        task.await.unwrap();
    }
}
