use super::*;
use nexa_core::{
    agent::ToolVisualObservation,
    approval::{ApprovalDecision, ToolApprovalMode},
    conversation::{ConversationMessage, CreateConversationInput},
    llm::Role,
    tools::{Tool, ToolExecutionContext, ToolRegistry, ToolResult},
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct NonceTool {
    name: &'static str,
    calls: Arc<AtomicUsize>,
    nonce: String,
}

/// Offer the real core schemas while making accidental live-probe calls inert.
struct CatalogOnlyTool(nexa_core::llm::ToolDefinition);
#[async_trait::async_trait]
impl Tool for CatalogOnlyTool {
    fn name(&self) -> &str {
        &self.0.name
    }
    fn description(&self) -> &str {
        &self.0.description
    }
    fn parameters_schema(&self) -> serde_json::Value {
        self.0.parameters.clone()
    }
    async fn execute(&self, _context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        Err(CoreError::InvalidInput(
            "Only read_test_nonce may execute in this integration probe".into(),
        ))
    }
}

#[test]
fn saved_subscription_configs_keep_the_native_route_and_reasoning_level() {
    let db = Database::open_memory().unwrap();
    for provider in ["github_copilot", "openai_codex"].into_iter().chain(
        nexa_core::external_agent::presets()
            .iter()
            .map(|preset| preset.provider.as_str()),
    ) {
        let input = serde_json::from_value(serde_json::json!({"name":provider,"provider":provider,"apiKey":"","model":"gpt-native-test","isDefault":true,"reasoningEffort":"ultra"})).unwrap();
        let saved = db.save_agent_config(&input).unwrap();
        let loaded = db.get_agent_config(&saved.id).unwrap();
        assert_eq!(loaded.provider, provider);
        assert_eq!(loaded.model, "gpt-native-test");
        assert_eq!(loaded.reasoning_effort.as_deref(), Some("ultra"));
        assert!(loaded.api_key.is_empty());
        assert!(loaded.base_url.is_none());
        assert!(AgentRuntimeKind::from_provider(&loaded.provider).is_some());
    }
}

#[test]
fn subscription_prompt_preserves_one_kernel_and_the_active_routing_guidance() {
    let (mut request, _rx, _, _) = fixture(AgentRuntimeKind::Copilot, "native-model");
    request.config.system_prompt =
        nexa_core::agent::build_system_prompt(Some("Project instruction sentinel"), &[]);
    request.user_parts = vec![ContentPart::Text {
        text: "Fix the Rust function in src/main.rs and test it".into(),
    }];
    let prepared = request.prepare(false).unwrap();
    assert_eq!(
        prepared
            .system_prompt
            .matches("## Evidence and Context Discipline")
            .count(),
        1
    );
    assert_eq!(
        prepared
            .system_prompt
            .matches("Project instruction sentinel")
            .count(),
        1
    );
    assert!(prepared.system_prompt.contains("## Active Routing Plan"));
    assert!(prepared.system_prompt.contains("read it directly"));
}

#[test]
fn subscription_input_and_history_obey_the_saved_privacy_policy() {
    let (mut request, _rx, _, _) = fixture(AgentRuntimeKind::Codex, "test");
    let mut privacy = request.db.load_privacy_config().unwrap();
    privacy.enabled = true;
    privacy.redact_patterns = vec![nexa_core::privacy::RedactRule {
        name: "private marker".into(),
        pattern: "private-marker-123".into(),
        replacement: "[PRIVATE]".into(),
    }];
    request.db.save_privacy_config(&privacy).unwrap();
    request.user_parts = vec![ContentPart::Text {
        text: "Inspect private-marker-123".into(),
    }];
    request.history = vec![Message::text(Role::User, "Earlier private-marker-123")];
    let prepared = request.prepare(false).unwrap();
    assert_eq!(prepared.prompt, "Inspect [PRIVATE]");
    assert!(!prepared.system_prompt.contains("private-marker-123"));
    assert_eq!(
        redact_user_text("Steer private-marker-123", &prepared.privacy),
        "Steer [PRIVATE]"
    );
}
#[async_trait::async_trait]
impl Tool for NonceTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "Read the integration test nonce. This has no external effects."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{},"additionalProperties":false})
    }
    async fn execute(&self, context: ToolExecutionContext<'_>) -> Result<ToolResult, CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult {
            call_id: context.call_id.to_string(),
            content: self.nonce.clone(),
            is_error: false,
            artifacts: None,
        })
    }
}

pub(super) fn fixture(
    kind: AgentRuntimeKind,
    model: &str,
) -> (
    AgentRuntimeTurnRequest,
    mpsc::Receiver<AgentEvent>,
    Arc<AtomicUsize>,
    String,
) {
    let db = Arc::new(Database::open_memory().unwrap());
    let conversation = db
        .create_conversation(&CreateConversationInput {
            provider: "subscription".into(),
            model: model.into(),
            system_prompt: None,
            collection_context: None,
            project_id: None,
            persona_id: None,
        })
        .unwrap();
    let (name, prompt) = if matches!(kind, AgentRuntimeKind::Codex) {
        ("mcp__audit__read_test_nonce", "Call the Nexa tool mcp__audit__read_test_nonce exactly once (it may have a protocol alias), then reply with the returned nonce and nothing else. Do not use any other tool.")
    } else {
        ("read_test_nonce", "Call read_test_nonce exactly once, then reply with the returned nonce and nothing else. Do not use any other tool.")
    };
    let user = ConversationMessage {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation.id.clone(),
        role: Role::User,
        content: prompt.into(),
        tool_call_id: None,
        tool_calls: vec![],
        artifacts: None,
        token_count: 30,
        created_at: String::new(),
        sort_order: 0,
        thinking: None,
        image_attachments: None,
    };
    db.add_message(&user).unwrap();
    let turn = db
        .create_conversation_turn(&conversation.id, &user.id, None)
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let nonce = uuid::Uuid::new_v4().to_string();
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(NonceTool {
        name,
        calls: calls.clone(),
        nonce: nonce.clone(),
    }));
    let (events, rx) = mpsc::channel(512);
    let (_steer, steering) = mpsc::unbounded_channel();
    let request = AgentRuntimeTurnRequest {
        kind,
        external: None,
        config: AgentConfig {
            model: Some(model.into()),
            max_iterations: 3,
            tool_approval_mode: ToolApprovalMode::AllowAll,
            ..AgentConfig::default()
        },
        dependencies: DesktopAgentSessionDependencies {
            tools,
            selected_skills: vec![],
            auto_loaded_skills: vec![],
            metrics: Default::default(),
        },
        db,
        conversation_id: conversation.id,
        turn_id: turn.id,
        next_sort_order: 1,
        history: vec![],
        user_parts: vec![ContentPart::Text {
            text: prompt.into(),
        }],
        events,
        cancellation: CancellationToken::new(),
        steering,
        approval: Arc::new(|_| Box::pin(async { ApprovalDecision::AllowOnce })),
        visual_interpreter: Arc::new(|_| {
            Box::pin(async {
                ToolVisualObservation::unavailable("test", "no-images", "No test images")
            })
        }),
    };
    (request, rx, calls, nonce)
}

struct DurableProbeDelivery(nexa_core::db::Database);
impl nexa_core::run_event_outbox::AgentRunEventDelivery for DurableProbeDelivery {
    fn deliver_run_event(&self, _: &str, event: &nexa_core::agent_run::AgentRunEvent) {
        if event.closes_run() {
            let turn = self.0.get_conversation_turn(&event.turn_id).unwrap();
            assert_eq!(
                turn.status, "success",
                "turn must close before terminal delivery"
            );
            assert_eq!(
                turn.assistant_message_id.as_deref(),
                event.payload["assistantMessageId"].as_str()
            );
            assert!(turn.finished_at.is_some());
        }
    }
    fn deliver_task_run_snapshot(&self, _: &str, _: nexa_core::conversation::AgentTaskRun) {}
}

pub(super) async fn run_live(kind: AgentRuntimeKind, model: &str) {
    let (mut request, mut rx, calls, nonce) = fixture(kind, model);
    let assembler =
        nexa_core::package_host::PackageRuntimeAssembler::database_builtin(&request.db).unwrap();
    let catalog = assembler
        .assemble_tool_registry(assembler.builtin_tool_registry())
        .unwrap();
    for definition in catalog.tools.definitions() {
        request
            .dependencies
            .tools
            .register(Box::new(CatalogOnlyTool(definition)));
    }
    let db = request.db.clone();
    let conversation = request.conversation_id.clone();
    let turn_id = request.turn_id.clone();
    let turn = db.get_conversation_turn(&turn_id).unwrap();
    let task = db
        .create_agent_task_run(
            &conversation,
            &turn_id,
            &turn.user_message_id,
            "Native protocol probe",
            Some("subscription"),
            Some(model),
        )
        .unwrap();
    db.mark_agent_task_run_started(&task.id, "responding")
        .unwrap();
    let executor = nexa_core::db_executor::DatabaseExecutor::new(db.as_ref().clone(), 8).unwrap();
    let outboxes = nexa_core::run_event_outbox::AgentRunEventOutboxes::new(
        executor,
        Arc::new(DurableProbeDelivery(db.as_ref().clone())),
    );
    let outbox = outboxes.open(&conversation, &task.id).await.unwrap();
    let (forward_tx, forward_rx) = mpsc::channel(512);
    let forwarder = crate::agent_stream_bridge::AgentStreamForwarder::new(
        conversation.clone(),
        task.id.clone(),
        turn_id.clone(),
        outbox.as_ref().clone(),
        std::time::Instant::now(),
    );
    let forwarding = tokio::spawn(forwarder.run(forward_rx));
    let (steering_tx, steering_rx) = mpsc::unbounded_channel();
    request.steering = steering_rx;
    let (probe_tool, correction) = if matches!(kind, AgentRuntimeKind::Codex) {
        ("mcp__audit__read_test_nonce", "Keep the current mcp__audit__read_test_nonce call, and do not call any tool again. After its result arrives, reply with STEERING_CONFIRMED followed by that nonce.")
    } else {
        ("read_test_nonce", "Keep the current read_test_nonce call, and do not call any tool again. After its result arrives, reply with STEERING_CONFIRMED followed by that nonce.")
    };
    let drain = tokio::spawn(async move {
        let mut done = 0;
        let mut deltas = 0;
        let mut steered = false;
        let mut applied = 0;
        let mut tool_events = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                AgentEvent::Done { .. } => done += 1,
                AgentEvent::StreamBlockDelta { .. } => deltas += 1,
                AgentEvent::ToolRunStarted { ref run } | AgentEvent::ToolRunUpdated { ref run } => {
                    tool_events.push((run.tool_name.clone(), format!("{:?}", run.status)));
                    if run.tool_name == probe_tool && !steered {
                        steering_tx
                            .send(AgentSteeringMessage::text(correction))
                            .unwrap();
                        steered = true;
                    }
                }
                AgentEvent::Steering { .. } => applied += 1,
                _ => {}
            }
            forward_tx.send(event).await.unwrap();
        }
        (done, deltas, steered, applied, tool_events)
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(150), run(request))
        .await
        .expect("live runtime deadline")
        .expect("live runtime completion");
    assert!(
        result.text_content().contains(&nonce),
        "answer must use actual Nexa tool evidence: calls={}, answer={}",
        calls.load(Ordering::SeqCst),
        result.text_content()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let (done, deltas, steered, applied, tool_events) = drain.await.unwrap();
    forwarding.await.unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        outbox.wait_for_terminal_commit(),
    )
    .await
    .unwrap()
    .unwrap();
    let completed = db.get_conversation_turn(&turn_id).unwrap();
    assert_eq!(completed.status, "success");
    assert!(completed.finished_at.is_some());
    assert!(
        result.text_content().contains("STEERING_CONFIRMED"),
        "steered={steered}, applied={applied}, tools={tool_events:?}, synthetic answer={}",
        result.text_content()
    );
    assert_eq!(done, 1);
    assert!(deltas > 0);
    let history = db.get_messages(&conversation).unwrap();
    if matches!(kind, AgentRuntimeKind::Copilot) {
        let steering_index = history
            .iter()
            .position(|message| message.content == correction)
            .unwrap();
        let previous = &history[steering_index - 1];
        assert_eq!(
            previous.role,
            Role::Assistant,
            "the completed response precedes queued steering durably"
        );
        assert!(
            previous.content.contains(&nonce),
            "the first response must survive reload"
        );
    }
    assert_eq!(
        history
            .iter()
            .filter(|message| message.content == correction
                && message
                    .artifacts
                    .as_ref()
                    .is_some_and(|artifact| artifact["kind"] == "steering"))
            .count(),
        1
    );
    assert_eq!(
        history
            .iter()
            .filter(|message| message.role == Role::Tool)
            .count(),
        1
    );
    assert!(history.last().unwrap().content.contains(&nonce));
    assert_eq!(
        completed.assistant_message_id.as_deref(),
        Some(history.last().unwrap().id.as_str())
    );
}

/// Exercise the real read/edit implementations in an isolated disposable folder
/// with the full production tool catalog present (including Copilot's >30 boundary).
pub(super) async fn run_live_edit(kind: AgentRuntimeKind, model: &str) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("native-edit.txt");
    std::fs::write(&path, "old value\n").unwrap();
    let (mut request, mut rx, _, nonce) = fixture(kind, model);
    request.config.max_iterations = 10;
    let assembler =
        nexa_core::package_host::PackageRuntimeAssembler::database_builtin(&request.db).unwrap();
    let catalog = assembler
        .assemble_tool_registry(assembler.builtin_tool_registry())
        .unwrap();
    for definition in catalog
        .tools
        .definitions()
        .into_iter()
        .filter(|definition| !matches!(definition.name.as_str(), "read_file" | "edit_file"))
    {
        request
            .dependencies
            .tools
            .register(Box::new(CatalogOnlyTool(definition)));
    }
    request
        .dependencies
        .tools
        .register(Box::new(nexa_core::tools::file_tool::FileTool));
    request
        .dependencies
        .tools
        .register(Box::new(nexa_core::tools::edit_file_tool::EditFileTool));
    request.dependencies.tools =
        request
            .dependencies
            .tools
            .with_workspace(Some(nexa_core::workspace::Workspace {
                roots: vec![directory.path().to_string_lossy().into()],
            }));
    assert!(request.dependencies.tools.definitions().len() > 30);
    request.user_parts = vec![ContentPart::Text { text: format!(
        "In this disposable integration-test workspace, use read_file to read {}, then edit_file to replace old value with {}. Read it again to verify. Reply EDIT_VERIFIED and the exact new value. Use only read_file and edit_file; all other tools are unavailable in this test.",
        path.display(), nonce
    ) }];
    let db = request.db.clone();
    let conversation = request.conversation_id.clone();
    let drain = tokio::spawn(async move {
        let mut edits = 0;
        let mut done = 0;
        let mut native_capacity = None;
        while let Some(event) = rx.recv().await {
            match event {
                AgentEvent::ToolRunCompleted { run } if run.tool_name == "edit_file" => {
                    assert_eq!(
                        run.status,
                        nexa_core::agent::ToolRunStatus::Completed,
                        "{run:?}"
                    );
                    edits += 1;
                }
                AgentEvent::UsageUpdate {
                    context_breakdown: Some(breakdown),
                    ..
                } => {
                    native_capacity = breakdown.context_window.or(native_capacity);
                }
                AgentEvent::Done { .. } => done += 1,
                _ => {}
            }
        }
        (edits, done, native_capacity)
    });
    let answer = tokio::time::timeout(std::time::Duration::from_secs(180), run(request))
        .await
        .expect("native edit deadline")
        .expect("native edit completion");
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), nonce);
    assert!(answer.text_content().contains("EDIT_VERIFIED"));
    assert!(answer.text_content().contains(&nonce));
    let (edits, done, native_capacity) = drain.await.unwrap();
    assert_eq!(edits, 1);
    assert_eq!(done, 1);
    assert_eq!(db.list_file_checkpoints(None).unwrap().len(), 1);
    assert!(db
        .get_messages(&conversation)
        .unwrap()
        .last()
        .unwrap()
        .content
        .contains(&nonce));
    eprintln!(
        "Live {kind:?} model={model}: real edit once; native context capacity={native_capacity:?}"
    );
}
