use super::*;
use crate::error::CoreError;
use crate::llm::{CompletionResponse, FinishReason, ProviderStreamEvent, StreamChunk};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};

fn png(value: u8) -> String {
    let image = image::RgbImage::from_pixel(8, 8, image::Rgb([value, value, value]));
    let mut buffer = Cursor::new(Vec::new());
    image
        .write_to(&mut buffer, image::ImageFormat::Png)
        .unwrap();
    B64.encode(buffer.into_inner())
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition should become true");
}

#[test]
fn native_live_protocols_keep_credentials_and_modalities_scoped() {
    let openai = NativeLiveConfig {
        protocol: NativeLiveProtocol::OpenAiRealtime,
        api_key: "private-test-key".into(),
        model: "gpt-realtime-2.1".into(),
        base_url: None,
    };
    assert_eq!(
        openai.setup("Observe")["session"]["audio"]["input"]["format"]["rate"],
        24000
    );
    assert!(!openai
        .setup("Observe")
        .to_string()
        .contains("private-test-key"));
    assert!(openai
        .request()
        .unwrap()
        .headers()
        .contains_key("Authorization"));
    let private = NativeLiveConfig {
        base_url: Some("https://private.example/v1".into()),
        ..openai
    };
    assert!(private.request().is_err());
    let google = NativeLiveConfig {
        protocol: NativeLiveProtocol::GeminiLive,
        api_key: "key".into(),
        model: "gemini-3.1-flash-live-preview".into(),
        base_url: None,
    };
    assert_eq!(
        google.setup("Observe")["setup"]["generationConfig"]["responseModalities"],
        json!(["AUDIO"])
    );
    assert!(google.setup("Observe")["setup"]
        .get("outputAudioTranscription")
        .is_some());
    assert_eq!(
        protocol::audio(google.protocol, &[0, 0])["realtimeInput"]["audio"]["mimeType"],
        "audio/pcm;rate=16000"
    );
    assert!(validate_pcm(&[0]).is_err());
    assert!(validate_pcm(&vec![0; MAX_AUDIO_BYTES + 2]).is_err());
    let qwen = NativeLiveConfig {
        protocol: NativeLiveProtocol::QwenRealtime,
        api_key: "qwen-test-key".into(),
        model: "qwen3.5-omni-flash-realtime".into(),
        base_url: Some("wss://workspace.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime".into()),
    };
    assert_eq!(
        qwen.setup("Observe")["session"]["modalities"],
        json!(["text"])
    );
    assert!(qwen
        .request()
        .unwrap()
        .headers()
        .contains_key("Authorization"));
    for url in [
        "wss://evil.cn-beijing.maas.aliyuncs.com.attacker.test/api-ws/v1/realtime",
        "ws://workspace.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime",
        "wss://workspace.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime?key=secret",
        "wss://user@workspace.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime",
    ] {
        assert!(protocol::qwen_endpoint(url).is_err(), "{url}");
    }
    let events = protocol::Decoder::default().parse(
        NativeLiveProtocol::QwenRealtime,
        &json!({"type":"response.text.delta","response_id":"q1","delta":"你好"}),
    );
    assert!(
        matches!(&events[0],protocol::NativeEvent::Entry {text,complete:false,..} if text=="你好")
    );
}

#[tokio::test]
async fn live_records_preserve_summary_and_enforce_device_ownership() {
    let db = crate::db::Database::open_memory().unwrap();
    let record = LiveRecord {
        snapshot: LiveSnapshot {
            id: "record".into(),
            mode: LiveMode::Incremental,
            model: "qwen".into(),
            phase: LivePhase::Stopped,
            sample_rate: 16000,
            started_at: chrono::Utc::now().to_rfc3339(),
            sequence: 1,
            entries: vec![],
            metrics: LiveMetrics::default(),
            error: None,
        },
        summary: Some("Summary".into()),
    };
    db.save_live_record("phone-a", &record).unwrap();
    db.save_live_record(
        "phone-a",
        &LiveRecord {
            summary: None,
            ..record.clone()
        },
    )
    .unwrap();
    assert_eq!(
        db.load_live_record("phone-a", "record")
            .unwrap()
            .summary
            .as_deref(),
        Some("Summary")
    );
    assert!(db.load_live_record("phone-b", "record").is_err());
    assert!(db.list_live_records("phone-b").unwrap().is_empty());
    assert_eq!(db.list_live_records("desktop").unwrap().len(), 1);
    db.save_live_record(
        "desktop",
        &LiveRecord {
            summary: Some("Updated".into()),
            ..record
        },
    )
    .unwrap();
    assert_eq!(
        db.load_live_record("phone-a", "record")
            .unwrap()
            .summary
            .as_deref(),
        Some("Updated")
    );
}

#[tokio::test]
async fn stop_interrupts_a_model_that_never_finishes() {
    let manager = LiveSessionManager::default();
    let db = crate::db::Database::open_memory().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let opened = manager
        .start(
            "owner",
            LiveRoute::Incremental {
                provider: Arc::new(SlowObserver {
                    requests: requests.clone(),
                    release: Arc::new(Notify::new()),
                }),
                request: CompletionRequest {
                    model: "slow".into(),
                    ..Default::default()
                },
            },
            LiveOptions::default(),
        )
        .unwrap();
    manager
        .transcript("owner", &opened.id, "speech", "test", false, true)
        .unwrap();
    until(|| !requests.lock().unwrap().is_empty()).await;
    assert!(manager
        .record_for_summary(&db, "owner", &opened.id)
        .is_err());
    let stopped = tokio::time::timeout(
        Duration::from_millis(250),
        manager.stop("owner", &opened.id),
    )
    .await
    .expect("stop must interrupt the pending model request")
    .unwrap();
    assert_eq!(stopped.phase, LivePhase::Stopped);
    assert!(db.load_live_record("owner", &opened.id).is_err());
    let mut record = manager
        .record_for_summary(&db, "owner", &opened.id)
        .unwrap();
    assert_eq!(record.snapshot.phase, LivePhase::Stopped);
    assert!(manager
        .record_for_summary(&db, "another-phone", &opened.id)
        .is_err());
    record.summary = Some("Already summarized".into());
    db.save_live_record("owner", &record).unwrap();
    assert_eq!(
        manager
            .record_for_summary(&db, "owner", &opened.id)
            .unwrap()
            .summary,
        record.summary
    );
    assert!(manager
        .transcript("owner", &opened.id, "late", "late speech", false, true)
        .is_err());
}

#[tokio::test]
async fn native_live_streams_before_completion_and_replaces_pending_frames() {
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let frame_payloads = Arc::new(Mutex::new(Vec::<String>::new()));
    let server_payloads = frame_payloads.clone();
    let audio_seen = Arc::new(AtomicU64::new(0));
    let server_audio = audio_seen.clone();
    let release = Arc::new(Notify::new());
    let server_release = release.clone();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let setup: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(setup["type"], "session.update");
        socket
            .send(Message::Text(
                json!({"type":"session.updated"}).to_string().into(),
            ))
            .await
            .unwrap();
        let mut responses = 0;
        loop {
            tokio::select! {
                message = socket.next() => {
                    let Some(Ok(Message::Text(message))) = message else { break; };
                    let value: Value = serde_json::from_str(&message).unwrap();
                    match value["type"].as_str().unwrap_or_default() {
                        "conversation.item.create" => {
                            if let Some(image) = value.pointer("/item/content/0/image_url").and_then(Value::as_str) { server_payloads.lock().unwrap().push(image.into()); }
                        }
                        "input_audio_buffer.append" => {
                            server_audio.fetch_add(1, Ordering::SeqCst);
                            socket.send(Message::Text(json!({"type":"conversation.item.input_audio_transcription.delta","item_id":"speech","delta":"hello"}).to_string().into())).await.unwrap();
                        }
                        "response.create" => {
                            responses += 1;
                            socket.send(Message::Text(json!({"type":"response.output_text.delta","response_id":format!("r{responses}"),"delta":"Visible update"}).to_string().into())).await.unwrap();
                        }
                        _ => {}
                    }
                }
                _ = server_release.notified() => {
                    socket.send(Message::Text(json!({"type":"response.output_text.done","response_id":"r1","text":"Visible update"}).to_string().into())).await.unwrap();
                    socket.send(Message::Text(json!({"type":"response.done","response":{"status":"completed"}}).to_string().into())).await.unwrap();
                }
            }
        }
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
        .await
        .unwrap();
    let manager = LiveSessionManager::default();
    let started = manager
        .start(
            "owner",
            LiveRoute::NativeSocket {
                config: NativeLiveConfig {
                    protocol: NativeLiveProtocol::OpenAiRealtime,
                    model: "fixture".into(),
                    api_key: "fixture".into(),
                    base_url: None,
                },
                socket: Box::new(socket),
            },
            LiveOptions {
                interval: Duration::from_secs(1),
                ..Default::default()
            },
        )
        .unwrap();
    until(|| manager.snapshot("owner", &started.id).unwrap().phase == LivePhase::Listening).await;
    assert!(manager.snapshot("other-device", &started.id).is_err());
    manager
        .frame("owner", &started.id, "image/png", &png(1))
        .unwrap();
    until(|| frame_payloads.lock().unwrap().len() == 1).await;
    manager.audio("owner", &started.id, vec![0; 4800]).unwrap();
    until(|| {
        manager
            .snapshot("owner", &started.id)
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.text == "hello" && !entry.complete)
    })
    .await;
    for value in 2..=4 {
        manager
            .frame("owner", &started.id, "image/png", &png(value))
            .unwrap();
    }
    assert_eq!(
        frame_payloads.lock().unwrap().len(),
        1,
        "No old-frame queue is drained during a response"
    );
    release.notify_one();
    until(|| frame_payloads.lock().unwrap().len() == 2).await;
    assert!(frame_payloads.lock().unwrap()[1].ends_with(&png(4)));
    let snapshot = manager.stop("owner", &started.id).await.unwrap();
    assert_eq!(snapshot.phase, LivePhase::Stopped);
    assert_eq!(snapshot.metrics.frames_replaced, 2);
    assert_eq!(snapshot.metrics.audio_ms, 100);
    assert!(manager
        .session("owner", &started.id)
        .unwrap()
        .frames
        .borrow()
        .is_none());
    assert!(manager.audio("owner", &started.id, vec![0; 2]).is_err());
    server.abort();
}

struct SlowObserver {
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
    release: Arc<Notify>,
}

#[tokio::test]
async fn qwen_native_live_submits_audio_before_images_and_commits_each_observation() {
    use tokio_tungstenite::tungstenite::Message;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let server_seen = seen.clone();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let setup: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(setup["session"]["modalities"], json!(["text"]));
        assert!(setup["session"]["turn_detection"].is_null());
        socket
            .send(Message::Text(
                json!({"type":"session.updated"}).to_string().into(),
            ))
            .await
            .unwrap();
        while let Some(Ok(Message::Text(message))) = socket.next().await {
            let event: Value = serde_json::from_str(&message).unwrap();
            let kind = event["type"].as_str().unwrap_or_default();
            server_seen.lock().unwrap().push(kind.into());
            if kind == "response.create" {
                for event in [
                    json!({"type":"response.text.delta","response_id":"q1","delta":"发现会议结论"}),
                    json!({"type":"response.text.done","response_id":"q1","text":"发现会议结论"}),
                    json!({"type":"response.done","response":{"status":"completed"}}),
                ] {
                    socket
                        .send(Message::Text(event.to_string().into()))
                        .await
                        .unwrap();
                }
            }
        }
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
        .await
        .unwrap();
    let manager = LiveSessionManager::default();
    let opened = manager
        .start(
            "phone",
            LiveRoute::NativeSocket {
                config: NativeLiveConfig {
                    protocol: NativeLiveProtocol::QwenRealtime,
                    model: "qwen".into(),
                    api_key: "fixture".into(),
                    base_url: None,
                },
                socket: Box::new(socket),
            },
            LiveOptions {
                interval: Duration::from_secs(1),
                ..Default::default()
            },
        )
        .unwrap();
    until(|| manager.input_state("phone", &opened.id).unwrap().1 == LivePhase::Listening).await;
    let mut jpeg = Cursor::new(Vec::new());
    image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .unwrap();
    manager
        .frame(
            "phone",
            &opened.id,
            "image/jpeg",
            &B64.encode(jpeg.into_inner()),
        )
        .unwrap();
    assert!(seen.lock().unwrap().is_empty());
    manager.audio("phone", &opened.id, vec![0; 3200]).unwrap();
    until(|| {
        manager
            .snapshot("phone", &opened.id)
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.text == "发现会议结论" && entry.complete)
    })
    .await;
    assert_eq!(
        &*seen.lock().unwrap(),
        &[
            "input_audio_buffer.append",
            "input_image_buffer.append",
            "input_audio_buffer.commit",
            "response.create"
        ]
    );
    manager.stop("phone", &opened.id).await.unwrap();
    server.abort();
}
#[async_trait]
impl LlmProvider for SlowObserver {
    fn name(&self) -> &str {
        "observer-fixture"
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }
    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        unreachable!()
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<futures::stream::BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let count = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request.clone());
            requests.len()
        };
        let release = self.release.clone();
        Ok(futures::stream::once(async move {
            if count == 1 {
                release.notified().await;
            }
            ProviderStreamEvent::Chunk {
                chunk: Box::new(StreamChunk {
                    delta: "Observed change".into(),
                    tool_call_delta: None,
                    finish_reason: Some(FinishReason::Stop),
                    usage: None,
                    thinking_delta: None,
                }),
            }
        })
        .boxed())
    }
}

#[tokio::test]
async fn slow_observer_consumes_latest_frame_and_speech_without_a_backlog() {
    let manager = LiveSessionManager::default();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(Notify::new());
    let session = manager
        .start(
            "owner",
            LiveRoute::Incremental {
                provider: Arc::new(SlowObserver {
                    requests: requests.clone(),
                    release: release.clone(),
                }),
                request: CompletionRequest {
                    model: "observer".into(),
                    tools: Some(vec![]),
                    ..Default::default()
                },
            },
            LiveOptions {
                interval: Duration::from_secs(1),
                ..Default::default()
            },
        )
        .unwrap();
    manager
        .frame("owner", &session.id, "image/png", &png(1))
        .unwrap();
    until(|| requests.lock().unwrap().len() == 1).await;
    for number in 0..100 {
        manager
            .transcript(
                "owner",
                &session.id,
                "speech",
                &format!("speech {number}"),
                false,
                true,
            )
            .unwrap();
    }
    manager
        .frame("owner", &session.id, "image/png", &png(2))
        .unwrap();
    manager
        .frame("owner", &session.id, "image/png", &png(3))
        .unwrap();
    release.notify_one();
    until(|| requests.lock().unwrap().len() == 2).await;
    let request = requests.lock().unwrap()[1].clone();
    assert!(request.tools.is_none());
    assert!(request.messages[1].text_content().contains("speech 99"));
    assert!(request.messages[1].parts.iter().any(
        |part| matches!(part, crate::llm::ContentPart::Image { data, .. } if data == &png(3))
    ));
    assert_eq!(
        manager.stop("owner", &session.id).await.unwrap().phase,
        LivePhase::Stopped
    );
}
