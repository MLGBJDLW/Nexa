use super::*;

const MARKER: &str = "VERIFIED_MIDDLE_MARKER";

fn long_output() -> String {
    format!(
        "{}{MARKER}{}",
        "证据🙂".repeat(5_000),
        "后续🙂".repeat(5_000)
    )
}

struct LongOutputTool(Arc<AtomicUsize>);

#[async_trait]
impl Tool for LongOutputTool {
    fn name(&self) -> &str {
        "long_output"
    }
    fn description(&self) -> &str {
        "Returns long evidence"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{},"additionalProperties":false})
    }
    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::Core]
    }
    async fn execute(
        &self,
        context: crate::tools::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult {
            call_id: context.call_id.into(),
            content: long_output(),
            is_error: false,
            artifacts: None,
        })
    }
}

struct ReadbackProvider(AtomicUsize);

#[async_trait]
impl LlmProvider for ReadbackProvider {
    fn name(&self) -> &str {
        "readback-fixture"
    }
    async fn list_models(&self) -> Result<Vec<String>, CoreError> {
        Ok(vec!["test".into()])
    }
    async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
        unreachable!("streaming fixture")
    }
    async fn health_check(&self) -> Result<(), CoreError> {
        Ok(())
    }
    async fn stream_events(
        &self,
        request: &CompletionRequest,
    ) -> Result<BoxStream<'_, ProviderStreamEvent>, CoreError> {
        let index = self.0.fetch_add(1, Ordering::SeqCst);
        let tool = match index {
            0 => Some(("long_output", serde_json::json!({}))),
            1 => {
                let result = request
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message.role == Role::Tool)
                    .unwrap()
                    .text_content();
                assert!(
                    !result.contains(MARKER),
                    "the initial context should exercise truncation"
                );
                let hint = result
                    .split("Full persisted text is available through context_history with ")
                    .nth(1)
                    .expect("readback must be advertised after persistence");
                let mut args: serde_json::Value =
                    serde_json::from_str(hint.split(". Follow nextOffset").next().unwrap())
                        .unwrap();
                args["offset"] = serde_json::json!(14_000);
                args["max_chars"] = serde_json::json!(16_000);
                Some(("context_history", args))
            }
            2 => {
                let result = request
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message.role == Role::Tool)
                    .unwrap()
                    .text_content();
                let page: serde_json::Value =
                    serde_json::from_str(result.split_once('\n').unwrap().1)
                        .expect("an exact page must remain valid JSON after context projection");
                let expected: String = long_output().chars().skip(14_000).take(16_000).collect();
                assert_eq!(page["text"], expected);
                assert!(page["text"].as_str().unwrap().contains(MARKER));
                None
            }
            _ => panic!("the task should finish after one output readback"),
        };
        let chunk = StreamChunk {
            delta: if tool.is_none() {
                MARKER.into()
            } else {
                String::new()
            },
            tool_call_delta: tool.as_ref().map(|(name, args)| ToolCallDelta {
                id: format!("output-{index}"),
                name: Some((*name).into()),
                arguments_delta: args.to_string().into(),
                index: Some(0),
                thought_signature: None,
            }),
            finish_reason: Some(if tool.is_some() {
                FinishReason::ToolCalls
            } else {
                FinishReason::Stop
            }),
            usage: None,
            thinking_delta: None,
        };
        crate::llm::provider_events_from_chunk_stream(Box::pin(stream::iter([Ok(chunk)])))
    }
}

#[tokio::test]
async fn truncated_tool_output_is_recovered_without_reexecution_or_a_history_handoff() {
    let db = Database::open_memory().unwrap();
    db.execute_batch_for_test("INSERT INTO conversations(id,provider,model) VALUES('chat','custom','test');
        INSERT INTO messages(id,conversation_id,role,content) VALUES('user','chat','user','Read the evidence');
        INSERT INTO conversation_turns(id,conversation_id,user_message_id) VALUES('turn','chat','user');").unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(LongOutputTool(executions.clone())));
    tools.register(Box::new(
        crate::tools::context_history_tool::ContextHistoryTool,
    ));
    let executor = AgentExecutor::new(
        Box::new(ReadbackProvider(AtomicUsize::new(0))),
        tools,
        AgentConfig {
            model: Some("test".into()),
            context_window: Some(128_000),
            ..Default::default()
        },
    );
    let (tx, mut rx) = mpsc::channel(256);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let answer = executor
        .run(
            vec![],
            vec![ContentPart::Text {
                text: "Read long_output and recover the exact middle marker.".into(),
            }],
            &db,
            Some("chat"),
            Some("turn"),
            tx,
            1,
        )
        .await
        .unwrap();
    assert_eq!(answer.text_content(), MARKER);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(
        db.list_context_history("chat", None, 10).unwrap()["windows"],
        serde_json::json!([])
    );
    drain.await.unwrap();
}
