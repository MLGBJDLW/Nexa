use super::*;

fn policy() -> crate::privacy::PrivacyConfig {
    crate::privacy::PrivacyConfig {
        enabled: true,
        redact_patterns: vec![crate::privacy::RedactRule {
            name: "source secret".into(),
            pattern: "privateCODE".into(),
            replacement: "[PRIVATE]".into(),
        }],
        ..Default::default()
    }
}

struct CapturedSourceTool {
    started: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl Tool for CapturedSourceTool {
    fn name(&self) -> &str {
        "mock_tool"
    }
    fn description(&self) -> &str {
        "Read a source and pause before returning its captured result"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{"value":{"type":"string"}}})
    }
    async fn execute(
        &self,
        context: crate::tools::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        let captured = "source privateCODE".to_string();
        self.started.notify_one();
        self.release.notified().await;
        Ok(ToolResult {
            call_id: context.call_id.into(),
            content: captured.clone(),
            is_error: false,
            artifacts: Some(serde_json::json!({"kind":"toolOutput","data":{"content":captured}})),
        })
    }
}

struct PrivacyRequestSpy {
    inner: MockProvider,
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
}

#[async_trait]
impl LlmProvider for PrivacyRequestSpy {
    fn name(&self) -> &str {
        self.inner.name()
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        self.inner.list_models().await
    }
    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        self.requests.lock().unwrap().push(request.clone());
        self.inner.complete(request).await
    }
    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError> {
        self.requests.lock().unwrap().push(request.clone());
        self.inner.stream_events(request).await
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

#[tokio::test]
async fn privacy_revocation_cancels_captured_tool_output_before_events_and_the_next_model_request()
{
    let db = Database::open_memory().unwrap();
    db.save_privacy_config(&crate::privacy::PrivacyConfig {
        enabled: false,
        ..Default::default()
    })
    .unwrap();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(CapturedSourceTool {
        started: started.clone(),
        release: release.clone(),
    }));
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentExecutor::new(
        Box::new(PrivacyRequestSpy {
            inner: MockProvider {
                stream_calls: calls.clone(),
            },
            requests: requests.clone(),
        }),
        tools,
        AgentConfig {
            model: Some("mock-model".into()),
            ..Default::default()
        },
    );
    let (tx, mut rx) = mpsc::channel(256);
    let run = agent.run(
        vec![],
        vec![ContentPart::Text {
            text: "Read the requested source".into(),
        }],
        &db,
        None,
        None,
        tx,
        0,
    );
    tokio::pin!(run);
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            _ = started.notified() => {},
            result = &mut run => panic!("run ended before the source was captured: {result:?}"),
        }
    })
    .await
    .unwrap();
    db.save_privacy_config(&policy()).unwrap();
    release.notify_one();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), &mut run)
            .await
            .unwrap(),
        Err(CoreError::Cancelled(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(requests.lock().unwrap().len(), 1);
    while let Ok(event) = rx.try_recv() {
        assert!(!format!("{event:?}").contains("privateCODE"));
    }
}

#[tokio::test]
async fn privacy_sanitizes_tool_artifacts_before_completion_events_and_live_model_context() {
    let db = Database::open_memory().unwrap();
    db.save_privacy_config(&policy()).unwrap();
    let release = Arc::new(Notify::new());
    release.notify_one();
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(CapturedSourceTool {
        started: Arc::new(Notify::new()),
        release,
    }));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentExecutor::new(
        Box::new(PrivacyRequestSpy {
            inner: MockProvider {
                stream_calls: Arc::new(AtomicUsize::new(0)),
            },
            requests: requests.clone(),
        }),
        tools,
        AgentConfig {
            model: Some("mock-model".into()),
            ..Default::default()
        },
    );
    let (tx, mut rx) = mpsc::channel(256);
    let history = vec![Message::text(Role::Assistant, "old source privateCODE")];
    agent
        .run(
            history,
            vec![ContentPart::Text {
                text: "Read the requested source".into(),
            }],
            &db,
            None,
            None,
            tx,
            0,
        )
        .await
        .unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| !serde_json::to_string(&request.messages)
            .unwrap()
            .contains("privateCODE")));
    let mut completed = false;
    while let Ok(event) = rx.try_recv() {
        assert!(!format!("{event:?}").contains("privateCODE"));
        if let AgentEvent::ToolRunCompleted { run } = event {
            completed = true;
            assert_eq!(run.content.as_deref(), Some("source [PRIVATE]"));
            assert_eq!(
                run.artifacts.unwrap()["data"]["content"],
                "source [PRIVATE]"
            );
        }
    }
    assert!(completed);
}

struct SummaryPrivacyBarrier {
    started: Arc<Notify>,
    release: Arc<Notify>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl LlmProvider for SummaryPrivacyBarrier {
    fn name(&self) -> &str {
        "summary-privacy-barrier"
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
    async fn stream_events(
        &self,
        _: &CompletionRequest,
    ) -> Result<BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError> {
        panic!("summarization must use complete")
    }
    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        self.release.notified().await;
        Err(CoreError::TransientLlm(
            "would otherwise retry with the old source".into(),
        ))
    }
}

#[tokio::test]
async fn privacy_revocation_stops_pre_summarization_without_a_retry_or_extractive_fallback() {
    let db = Database::open_memory().unwrap();
    db.save_privacy_config(&crate::privacy::PrivacyConfig {
        enabled: false,
        ..Default::default()
    })
    .unwrap();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let summary_calls = Arc::new(AtomicUsize::new(0));
    let model_calls = Arc::new(AtomicUsize::new(0));
    let agent = AgentExecutor::new(
        Box::new(MockProvider {
            stream_calls: model_calls.clone(),
        }),
        ToolRegistry::new(),
        AgentConfig {
            model: Some("mock-model".into()),
            context_window: Some(4096),
            max_tokens: Some(256),
            ..Default::default()
        },
    )
    .with_summarization_provider(Box::new(SummaryPrivacyBarrier {
        started: started.clone(),
        release: release.clone(),
        calls: summary_calls.clone(),
    }));
    let history = (0..12)
        .flat_map(|_| {
            [
                Message::text(Role::User, "Continue the task"),
                Message::text(Role::Assistant, "source privateCODE ".repeat(500)),
            ]
        })
        .collect();
    let (tx, mut events) = mpsc::channel(256);
    let run = agent.run(
        history,
        vec![ContentPart::Text {
            text: "Continue".into(),
        }],
        &db,
        None,
        None,
        tx,
        0,
    );
    tokio::pin!(run);
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            _ = started.notified() => {},
            result = &mut run => panic!("expected pre-summary, got {result:?}"),
        }
    })
    .await
    .unwrap();
    db.save_privacy_config(&policy()).unwrap();
    release.notify_one();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), &mut run)
            .await
            .unwrap(),
        Err(CoreError::Cancelled(_))
    ));
    assert_eq!(summary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(model_calls.load(Ordering::SeqCst), 0);
    while let Ok(event) = events.try_recv() {
        assert!(!format!("{event:?}").contains("privateCODE"));
    }
}
