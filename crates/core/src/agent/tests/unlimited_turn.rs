use super::*;

struct ProductiveLongTurn {
    calls: Arc<AtomicUsize>,
    answer_only: bool,
}

#[async_trait]
impl LlmProvider for ProductiveLongTurn {
    fn name(&self) -> &str {
        "productive-long-turn"
    }

    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec!["private-model".into()])
    }

    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        unreachable!("this fixture uses streaming")
    }

    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            index <= 300,
            "the turn should finish after its requested work"
        );
        let chunk = if self.answer_only {
            assert!(request.tools.as_ref().is_none_or(Vec::is_empty));
            StreamChunk {
                delta: if index == 0 {
                    "part-0 ".into()
                } else {
                    format!("<nexa-continuation-ack:{index}>\npart-{index} ")
                },
                tool_call_delta: None,
                finish_reason: Some(if index < 300 {
                    FinishReason::Length
                } else {
                    FinishReason::Stop
                }),
                usage: None,
                thinking_delta: None,
            }
        } else if index < 300 {
            assert!(request
                .tools
                .as_ref()
                .is_some_and(|tools| !tools.is_empty()));
            StreamChunk {
                delta: String::new(),
                tool_call_delta: Some(ToolCallDelta {
                    id: format!("productive-{index}"),
                    name: Some("recording_tool".into()),
                    arguments_delta: serde_json::json!({ "value": format!("item-{index}") })
                        .to_string()
                        .into(),
                    index: Some(0),
                    thought_signature: None,
                }),
                finish_reason: Some(FinishReason::ToolCalls),
                usage: None,
                thinking_delta: None,
            }
        } else {
            StreamChunk {
                delta: "Completed all 300 items.".into(),
                tool_call_delta: None,
                finish_reason: Some(FinishReason::Stop),
                usage: None,
                thinking_delta: None,
            }
        };
        Ok(Box::pin(stream::iter([ProviderStreamEvent::Chunk {
            chunk: Box::new(chunk),
        }])))
    }

    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

#[tokio::test]
async fn productive_turn_completes_more_than_256_provider_requests() {
    let calls = Arc::new(AtomicUsize::new(0));
    let executions = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(RecordingTool {
        executions: Arc::clone(&executions),
    }));
    let executor = AgentExecutor::new(
        Box::new(ProductiveLongTurn {
            calls: Arc::clone(&calls),
            answer_only: false,
        }),
        tools,
        AgentConfig {
            model: Some("private-model".into()),
            context_window_resolution: Some(crate::conversation::memory::ResolvedContextWindow {
                capacity_tokens: None,
                authority: crate::conversation::memory::ContextWindowAuthority::ProviderManaged,
            }),
            ..AgentConfig::default()
        },
    );
    let db = Database::open_memory().unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    let events = tokio::spawn(async move {
        let mut completed = 0;
        while let Some(event) = rx.recv().await {
            if matches!(event, AgentEvent::ToolRunCompleted { .. }) {
                completed += 1;
            }
        }
        completed
    });
    let answer = executor
        .run(
            vec![],
            vec![ContentPart::Text {
                text: "Process the 300 distinct items using recording_tool, then summarize.".into(),
            }],
            &db,
            None,
            None,
            tx,
            0,
        )
        .await
        .expect("productive work must not stop at an implicit provider-request ceiling");
    assert_eq!(answer.text_content(), "Completed all 300 items.");
    assert_eq!(calls.load(Ordering::SeqCst), 301);
    assert_eq!(executions.load(Ordering::SeqCst), 300);
    assert_eq!(events.await.unwrap(), 300);
}

#[tokio::test(start_paused = true)]
async fn answer_only_recovery_has_no_implicit_provider_request_ceiling() {
    let calls = Arc::new(AtomicUsize::new(0));
    let executor = AgentExecutor::new(
        Box::new(ProductiveLongTurn {
            calls: Arc::clone(&calls),
            answer_only: true,
        }),
        ToolRegistry::new(),
        AgentConfig {
            model: Some("private-model".into()),
            max_iterations: 0,
            ..AgentConfig::default()
        },
    );
    let db = Database::open_memory().unwrap();
    let (tx, mut rx) = mpsc::channel(64);
    let events = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let answer = executor
        .run(
            vec![],
            vec![ContentPart::Text {
                text: "Write the complete answer without tools.".into(),
            }],
            &db,
            None,
            None,
            tx,
            0,
        )
        .await
        .expect("a finite tool budget must not introduce a separate request limit");
    assert_eq!(calls.load(Ordering::SeqCst), 301);
    for index in 0..=300 {
        assert!(answer.text_content().contains(&format!("part-{index} ")));
    }
    assert!(!answer.text_content().contains("nexa-continuation-ack"));
    events.await.unwrap();
}

struct LongHostedWork {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl LlmProvider for LongHostedWork {
    fn name(&self) -> &str {
        "long-hosted-work"
    }

    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }

    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        unreachable!("hosted effects must not be replayed")
    }

    async fn stream_events(
        &self,
        _: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        assert_eq!(self.calls.fetch_add(1, Ordering::SeqCst), 0);
        Ok(Box::pin(stream::unfold(0, |index| async move {
            if index > 12 {
                return None;
            }
            tokio::time::sleep(Duration::from_secs(120)).await;
            let event = if index < 9 {
                ProviderStreamEvent::HostedTool {
                    tool: Box::new(crate::llm::ProviderHostedToolEvent {
                        call_id: "long-hosted-call".into(),
                        tool_name: "web_search".into(),
                        kind: crate::llm::ProviderHostedToolKind::WebSearch,
                        provider_id: "long-hosted-work".into(),
                        status: if index == 8 {
                            ProviderHostedToolStatus::Completed
                        } else {
                            ProviderHostedToolStatus::Running
                        },
                        arguments: None,
                        content: Some(format!("Completed research phase {index}")),
                        artifacts: None,
                    }),
                }
            } else if index < 12 {
                ProviderStreamEvent::Chunk {
                    chunk: Box::new(StreamChunk {
                        delta: String::new(),
                        tool_call_delta: None,
                        finish_reason: None,
                        usage: None,
                        thinking_delta: Some(format!("Reasoning after hosted completion {index}.")),
                    }),
                }
            } else {
                ProviderStreamEvent::Chunk {
                    chunk: Box::new(StreamChunk {
                        delta: "Long hosted work finished.".into(),
                        tool_call_delta: None,
                        finish_reason: Some(FinishReason::Stop),
                        usage: None,
                        thinking_delta: None,
                    }),
                }
            };
            Some((event, index + 1))
        })))
    }

    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn hosted_progress_can_continue_past_ten_minutes_without_replay() {
    let calls = Arc::new(AtomicUsize::new(0));
    let executor = AgentExecutor::new(
        Box::new(LongHostedWork {
            calls: Arc::clone(&calls),
        }),
        ToolRegistry::new(),
        AgentConfig {
            model: Some("private-model".into()),
            ..AgentConfig::default()
        },
    );
    let db = Database::open_memory().unwrap();
    let (tx, mut rx) = mpsc::channel(128);
    let answer = executor
        .run(
            vec![],
            vec![ContentPart::Text {
                text: "Finish the hosted research.".into(),
            }],
            &db,
            None,
            None,
            tx,
            0,
        )
        .await
        .expect("continuing hosted progress must not hit an implicit absolute deadline");
    assert_eq!(answer.text_content(), "Long hosted work finished.");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut completed = 0;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, AgentEvent::ToolRunCompleted { .. }) {
            completed += 1;
        }
    }
    assert_eq!(completed, 1);
}
