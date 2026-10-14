//! Agent Session adapter over the existing [`AgentExecutor`].
//!
//! This Module is the first concrete Runtime Protocol Adapter: it translates
//! host-agnostic turn input into an executor run and exposes ordered
//! `AgentRunEvent`s through the Agent Session Interface.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::agent::{AgentConfig, AgentEvent, AgentExecutor};
use crate::agent_run::{AgentRunEvent, AgentRunPhase};
use crate::approval::ApprovalDecision;
use crate::conversation::AgentTaskRun;
use crate::db::Database;
use crate::db_executor::DatabaseExecutor;
use crate::error::CoreError;
use crate::llm::{ContentPart, LlmProvider, Message};
use crate::run_event_outbox::{AgentRunEventDelivery, AgentRunEventOutbox, AgentRunEventOutboxes};
use crate::runtime::{
    validate_runtime_turn_events, AgentSession, AgentSessionConfig, AgentTurnHandle,
    AgentTurnInput, AgentTurnResult, AgentTurnState,
};
use crate::tools::ToolRegistry;

pub struct ExecutorAgentSession {
    config: AgentSessionConfig,
    executor: AgentExecutor,
    db: Database,
    history: Vec<Message>,
    source_scope_override: Option<Vec<String>>,
    next_sort_order: i64,
    events_by_run: HashMap<String, Vec<AgentRunEvent>>,
    results_by_run: HashMap<String, AgentTurnResult>,
    event_outboxes: Option<AgentRunEventOutboxes>,
}

struct AgentSessionRunEventDelivery;

impl AgentRunEventDelivery for AgentSessionRunEventDelivery {
    fn deliver_run_event(&self, _conversation_id: &str, _event: &AgentRunEvent) {}

    fn deliver_task_run_snapshot(&self, _conversation_id: &str, _snapshot: AgentTaskRun) {}
}

impl ExecutorAgentSession {
    pub fn new(
        config: AgentSessionConfig,
        provider: Box<dyn LlmProvider>,
        tools: ToolRegistry,
        executor_config: AgentConfig,
        db: Database,
    ) -> Self {
        Self {
            config,
            executor: AgentExecutor::new(provider, tools, executor_config).with_activity_runtime(
                crate::activity::ActivityRuntime::with_database(db.clone())
                    .unwrap_or_else(|_| crate::activity::ActivityRuntime::new()),
            ),
            db,
            history: Vec::new(),
            source_scope_override: None,
            next_sort_order: 0,
            events_by_run: HashMap::new(),
            results_by_run: HashMap::new(),
            event_outboxes: None,
        }
    }

    pub fn with_history(mut self, history: Vec<Message>, next_sort_order: i64) -> Self {
        self.history = history;
        self.next_sort_order = next_sort_order;
        self
    }

    pub fn with_source_scope_override(mut self, source_scope: Vec<String>) -> Self {
        self.source_scope_override = Some(source_scope);
        self
    }

    pub fn with_approval_callback(mut self, cb: crate::approval::ApprovalCallback) -> Self {
        self.executor = self.executor.with_approval_callback(cb);
        self
    }

    pub fn with_confirmation_callback(mut self, cb: crate::agent::ConfirmationCallback) -> Self {
        self.executor = self.executor.with_confirmation_callback(cb);
        self
    }

    pub fn with_auto_loaded_skills_override(mut self, skills: Vec<crate::skills::Skill>) -> Self {
        self.executor = self.executor.with_auto_loaded_skills_override(skills);
        self
    }

    pub fn with_skills_override(mut self, skills: Vec<crate::skills::Skill>) -> Self {
        self.executor = self.executor.with_skills_override(skills);
        self
    }

    pub fn result_for_run(&self, run_id: &str) -> Option<&AgentTurnResult> {
        self.results_by_run.get(run_id)
    }

    fn outboxes(&mut self) -> Result<AgentRunEventOutboxes, CoreError> {
        if self.event_outboxes.is_none() {
            let database = DatabaseExecutor::new(self.db.clone(), 64)?;
            self.event_outboxes = Some(AgentRunEventOutboxes::new(
                database,
                Arc::new(AgentSessionRunEventDelivery),
            ));
        }
        Ok(self
            .event_outboxes
            .as_ref()
            .expect("Agent Session outboxes were initialized")
            .clone())
    }
}

#[async_trait]
impl AgentSession for ExecutorAgentSession {
    fn config(&self) -> &AgentSessionConfig {
        &self.config
    }

    async fn configure(&mut self, config: AgentSessionConfig) -> Result<(), CoreError> {
        self.config = config;
        self.config.apply_protocol_defaults();
        Ok(())
    }

    async fn start_turn(&mut self, input: AgentTurnInput) -> Result<AgentTurnHandle, CoreError> {
        let run_id = self
            .config
            .task_run_id
            .clone()
            .ok_or_else(|| {
                CoreError::InvalidInput(
                    "ExecutorAgentSession requires config.task_run_id backed by an existing Agent task run"
                        .to_string(),
                )
            })?;
        let task_run = self.db.get_agent_task_run(&run_id).map_err(|error| match error {
            CoreError::NotFound(_) => CoreError::InvalidInput(format!(
                "ExecutorAgentSession task_run_id {run_id} does not identify an existing Agent task run"
            )),
            other => other,
        })?;
        if let Some(configured_conversation_id) = self.config.conversation_id.as_deref() {
            if configured_conversation_id != task_run.conversation_id {
                return Err(CoreError::InvalidInput(format!(
                    "ExecutorAgentSession task_run_id {run_id} belongs to conversation {}, not {configured_conversation_id}",
                    task_run.conversation_id
                )));
            }
        }
        if !matches!(
            task_run.status.as_str(),
            "queued" | "running" | "waiting_approval"
        ) {
            return Err(CoreError::Conflict(format!(
                "ExecutorAgentSession cannot execute Agent Run {run_id} from status '{}'",
                task_run.status
            )));
        }

        let conversation_id = task_run.conversation_id;
        let turn_id = task_run.turn_id;
        // Checkpoint and interaction continuations append their user response
        // outside this cached session. Reload the durable append position before
        // each producer starts, retaining any higher caller-reserved position.
        self.next_sort_order = self
            .next_sort_order
            .max(self.db.next_message_sort_order(&conversation_id)?);
        let session_id = self.config.session_id.clone();
        let outbox = self.outboxes()?.open(&conversation_id, &run_id).await?;
        if outbox.is_closed_for_submission() {
            return Err(CoreError::Conflict(format!(
                "Agent Run {run_id} is already closed"
            )));
        }
        // The durable task state above is authoritative. A checkpoint resume
        // can reuse this session's cached outbox after atomically re-queuing
        // the run; reopen its publication boundary before starting a producer.
        outbox.resume_submissions().map_err(|error| {
            CoreError::Agent(format!("Could not reopen Agent Run {run_id}: {error}"))
        })?;
        let (tx, rx) = mpsc::channel::<AgentEvent>(64);
        let collector_run_id = run_id.clone();
        let collector_turn_id = turn_id.clone();
        let collector_outbox = Arc::clone(&outbox);
        let collector = tokio::spawn(async move {
            collect_run_events(rx, collector_outbox, collector_run_id, collector_turn_id).await
        });

        let mut user_parts = vec![ContentPart::Text {
            text: input.user_text,
        }];
        for attachment in input.attachments {
            if let Some(path) = attachment.path {
                user_parts.push(ContentPart::Text {
                    text: format!("[Attachment: {}]", path),
                });
            }
        }

        let result = self
            .executor
            .run_with_source_scope(
                self.history.clone(),
                user_parts,
                &self.db,
                Some(&conversation_id),
                Some(&turn_id),
                self.source_scope_override.clone(),
                tx,
                self.next_sort_order,
            )
            .await;

        let collected = collector
            .await
            .map_err(|error| CoreError::Agent(format!("Run Event collector failed: {error}")))?;
        let collected = match collected {
            Ok(collected) => collected,
            Err(submission_error) => {
                return match outbox.wait_for_terminal_commit().await {
                    Ok(_) => Err(submission_error),
                    Err(failure) => Err(CoreError::Agent(format!(
                        "Run Event submission failed and Agent Run {run_id} was failed closed: {failure}"
                    ))),
                };
            }
        };

        if !collected.terminal_submitted {
            let suspended = match &result {
                Err(CoreError::RecoveryRequired {
                    action_reconciliation,
                    ..
                }) => {
                    let reason = if *action_reconciliation {
                        "connection_interrupted_requires_action_reconciliation:provider_hosted_action"
                    } else {
                        "provider_connection_interrupted"
                    };
                    outbox
                        .pause_with_checkpoint(&turn_id, reason)
                        .await
                        .map_err(|error| {
                            CoreError::Agent(format!(
                                "Could not preserve interrupted task: {error}"
                            ))
                        })?;
                    Some(AgentTurnState::Paused)
                }
                Err(CoreError::AwaitingUserInput { .. }) => {
                    // The executor already saved the durable question checkpoint.
                    outbox.flush().await.map_err(|error| {
                        CoreError::Agent(format!("Could not publish pending interaction: {error}"))
                    })?;
                    Some(AgentTurnState::AwaitingUserInput)
                }
                _ => None,
            };
            if let Some(state) = suspended {
                self.events_by_run
                    .insert(run_id.clone(), self.db.list_agent_run_events(&run_id)?);
                self.results_by_run.remove(&run_id);
                // A paused handle is observable now; a terminal result and its
                // commit barrier belong to the later resumed execution.
                return Ok(AgentTurnHandle {
                    session_id,
                    run_id,
                    turn_id,
                    state,
                });
            }
        }

        let final_message = match result {
            Ok(message) => Some(message.text_content()),
            Err(err) => {
                if !collected.terminal_submitted {
                    outbox
                        .submit(AgentRunEvent::terminal_error(
                            &run_id,
                            Some(&turn_id),
                            0,
                            "Agent execution failed.",
                            "failed",
                            Some(&serde_json::json!({ "error": err.to_string() })),
                        ))
                        .map_err(|error| {
                            CoreError::Agent(format!(
                                "Could not submit the terminal Run Event: {error}"
                            ))
                        })?;
                }
                None
            }
        };

        let missing_terminal = !collected.terminal_submitted && final_message.is_some();
        if missing_terminal {
            outbox
                .submit(AgentRunEvent::terminal_error(
                    &run_id,
                    Some(&turn_id),
                    0,
                    "Agent execution ended without a terminal Run Event.",
                    "failed",
                    Some(&serde_json::json!({ "reason": "missing_terminal_event" })),
                ))
                .map_err(|error| {
                    CoreError::Agent(format!(
                        "Could not fail the incomplete turn closed: {error}"
                    ))
                })?;
        }

        outbox.wait_for_terminal_commit().await.map_err(|error| {
            CoreError::Agent(format!(
                "Agent Run {run_id} did not reach its durable terminal barrier: {error}"
            ))
        })?;
        let run_events = self.db.list_agent_run_events(&run_id)?;

        if missing_terminal {
            return Err(CoreError::Agent(
                "Runtime turn contract failed: Agent execution ended without a terminal Run Event"
                    .to_string(),
            ));
        }

        let report = validate_runtime_turn_events(&run_events)
            .map_err(|err| CoreError::Agent(format!("Runtime turn contract failed: {err}")))?;

        let status = report.terminal_status;
        let handle = AgentTurnHandle {
            session_id,
            run_id: run_id.clone(),
            turn_id: turn_id.clone(),
            state: AgentTurnState::Terminal(status.clone()),
        };
        let turn_result = AgentTurnResult {
            handle: handle.clone(),
            final_message,
            artifacts: serde_json::json!({
                "kind": "agentSessionTurn",
                "version": 1,
                "eventCount": report.event_count,
                "approvalDenied": report.approval_denied,
            }),
            usage: serde_json::json!({}),
            trace_id: None,
            status,
        };
        self.events_by_run.insert(run_id.clone(), run_events);
        self.results_by_run.insert(run_id, turn_result);

        Ok(handle)
    }

    async fn steer_turn(&mut self, _turn_id: &str, _text: String) -> Result<(), CoreError> {
        Err(CoreError::InvalidInput(
            "ExecutorAgentSession runs turns synchronously; steering requires a running turn"
                .to_string(),
        ))
    }

    async fn interrupt_turn(&mut self, _turn_id: &str, _reason: String) -> Result<(), CoreError> {
        Err(CoreError::InvalidInput(
            "ExecutorAgentSession runs turns synchronously; interrupt requires a running turn"
                .to_string(),
        ))
    }

    async fn resolve_approval(
        &mut self,
        _request_id: &str,
        _decision: ApprovalDecision,
    ) -> Result<(), CoreError> {
        Err(CoreError::InvalidInput(
            "approval resolution is handled by the configured approval adapter".to_string(),
        ))
    }

    async fn read_events(&self, run_id: &str) -> Result<Vec<AgentRunEvent>, CoreError> {
        if let Some(events) = self.events_by_run.get(run_id) {
            return Ok(events.clone());
        }

        let events = self.db.list_agent_run_events(run_id)?;
        if events.is_empty() {
            return Err(CoreError::NotFound(format!("run events for {run_id}")));
        }
        Ok(events)
    }

    async fn close(&mut self) -> Result<(), CoreError> {
        Ok(())
    }
}

async fn collect_run_events(
    mut rx: mpsc::Receiver<AgentEvent>,
    outbox: Arc<AgentRunEventOutbox>,
    run_id: String,
    turn_id: String,
) -> Result<CollectedRunEvents, CoreError> {
    let mut collected = CollectedRunEvents::default();
    while let Some(event) = rx.recv().await {
        let run_event = AgentRunEvent::from_agent_event(&event).with_context(
            Some(&run_id),
            Some(&turn_id),
            None,
        );
        let terminal = run_event.closes_run();
        outbox.submit(run_event).map_err(|error| {
            CoreError::Agent(format!("Could not submit an ordered Run Event: {error}"))
        })?;
        collected.terminal_submitted |= terminal;
    }
    Ok(collected)
}

#[derive(Debug, Default)]
struct CollectedRunEvents {
    terminal_submitted: bool,
}

pub fn status_event_for_session(
    run_id: &str,
    turn_id: &str,
    event_seq: u64,
    label: &str,
    status: &str,
) -> AgentRunEvent {
    AgentRunEvent::status_update(
        run_id,
        Some(turn_id),
        event_seq,
        AgentRunPhase::Responding,
        label,
        Some(status),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use futures::{stream, StreamExt};

    use crate::conversation::{ConversationMessage, CreateConversationInput};
    use crate::llm::Role;
    use crate::llm::{
        CompletionRequest, CompletionResponse, FinishReason, StreamChunk, ToolCallDelta,
    };
    use crate::task_run::{AgentTaskRuntime, CreateTaskRunInput};

    struct StaticProvider;

    struct CountingProvider {
        stream_calls: Arc<AtomicUsize>,
    }

    struct ScriptedSessionProvider {
        events: Vec<crate::llm::ProviderStreamEvent>,
        calls: Arc<AtomicUsize>,
        settle_subtask: Option<(Database, String)>,
    }

    #[async_trait]
    impl LlmProvider for ScriptedSessionProvider {
        fn name(&self) -> &str {
            "static"
        }
        fn stream_max_retries(&self) -> Option<u32> {
            Some(0)
        }
        async fn list_models(&self) -> Result<Vec<String>, CoreError> {
            Ok(vec!["static-model".into()])
        }
        async fn complete(&self, _: &CompletionRequest) -> Result<CompletionResponse, CoreError> {
            Err(CoreError::TransientLlm("fixture connection lost".into()))
        }
        async fn stream_events(
            &self,
            _: &CompletionRequest,
        ) -> Result<futures::stream::BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some((db, id)) = &self.settle_subtask {
                let error = CoreError::RecoveryRequired {
                    message: "Hosted action outcome is uncertain".into(),
                    action_reconciliation: true,
                };
                db.finish_agent_subtask_run(
                    id,
                    "failed",
                    Some(&serde_json::json!({"recoveryRequired":error.recovery_requirement()})),
                    Some("connection lost"),
                )?;
            }
            Ok(stream::iter(self.events.clone()).boxed())
        }
        async fn health_check(&self) -> Result<(), CoreError> {
            Ok(())
        }
    }

    fn scripted_session(
        db: &Database,
        events: Vec<crate::llm::ProviderStreamEvent>,
        tools: ToolRegistry,
    ) -> (ExecutorAgentSession, String, String, Arc<AtomicUsize>) {
        let (conversation_id, turn_id, run_id) = create_started_run(db, "suspended");
        let calls = Arc::new(AtomicUsize::new(0));
        let session = ExecutorAgentSession::new(
            AgentSessionConfig {
                conversation_id: Some(conversation_id),
                task_run_id: Some(run_id.clone()),
                ..Default::default()
            },
            Box::new(ScriptedSessionProvider {
                events,
                calls: calls.clone(),
                settle_subtask: None,
            }),
            tools,
            AgentConfig {
                model: Some("static-model".into()),
                tool_approval_mode: crate::approval::ToolApprovalMode::AllowAll,
                ..Default::default()
            },
            db.clone(),
        )
        .with_history(vec![], 1);
        (session, turn_id, run_id, calls)
    }

    fn assert_resume_transcript_order(db: &Database, conversation: &str, response_id: &str) {
        let messages = db.get_messages(conversation).unwrap();
        assert!(
            messages
                .windows(2)
                .all(|pair| pair[0].sort_order < pair[1].sort_order),
            "conversation message order must be unique and increasing"
        );
        let response_index = messages
            .iter()
            .position(|message| message.id == response_id)
            .unwrap();
        assert!(
            messages[response_index + 1..]
                .iter()
                .any(|message| message.role == Role::Assistant),
            "resumed answer must follow the durable user response"
        );
    }

    #[tokio::test]
    async fn interrupted_executor_session_preserves_partial_work_without_a_terminal_error() {
        use crate::llm::{
            ProviderHostedToolEvent, ProviderHostedToolKind, ProviderHostedToolStatus,
            ProviderRecoveryCategory, ProviderStreamEvent,
        };
        for hosted_action in [false, true] {
            let db = Database::open_memory().unwrap();
            let mut events = vec![ProviderStreamEvent::Chunk {
                chunk: Box::new(StreamChunk {
                    delta: "Saved partial output".into(),
                    tool_call_delta: None,
                    finish_reason: None,
                    usage: None,
                    thinking_delta: None,
                }),
            }];
            if hosted_action {
                events.push(ProviderStreamEvent::HostedTool {
                    tool: Box::new(ProviderHostedToolEvent {
                        call_id: "hosted-1".into(),
                        tool_name: "web_search".into(),
                        kind: ProviderHostedToolKind::WebSearch,
                        provider_id: "static".into(),
                        status: ProviderHostedToolStatus::Running,
                        arguments: None,
                        content: None,
                        artifacts: None,
                    }),
                });
            }
            events.push(ProviderStreamEvent::RecoverableError {
                message: "fixture connection lost".into(),
                category: ProviderRecoveryCategory::Network,
            });
            let (mut session, turn_id, run_id, calls) =
                scripted_session(&db, events, ToolRegistry::new());
            let handle = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                session.start_turn(AgentTurnInput::text("Continue the task")),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(handle.state, AgentTurnState::Paused);
            assert!(session.result_for_run(&run_id).is_none());
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(db.get_agent_task_run(&run_id).unwrap().status, "paused");
            let turn = db.get_conversation_turn(&turn_id).unwrap();
            assert_eq!(turn.status, "paused");
            assert!(turn.trace.is_some());
            let checkpoints = db.list_task_resume_checkpoints(&run_id).unwrap();
            assert_eq!(checkpoints.len(), 1);
            assert!(checkpoints[0]
                .resume_prompt
                .contains("Saved partial output"));
            assert_eq!(
                checkpoints[0]
                    .resume_prompt
                    .contains("provider_hosted_action"),
                hosted_action
            );
            let events = session.read_events(&run_id).await.unwrap();
            assert!(events.iter().all(|event| !event.closes_run()));
            assert_eq!(events.last().unwrap().phase, AgentRunPhase::Paused);
            session.events_by_run.clear();
            assert_eq!(
                session.read_events(&run_id).await.unwrap().len(),
                events.len()
            );
            let response = ConversationMessage {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: db.get_agent_task_run(&run_id).unwrap().conversation_id,
                role: Role::User,
                content: checkpoints[0].resume_prompt.clone(),
                tool_call_id: None,
                tool_calls: vec![],
                artifacts: None,
                token_count: 1,
                created_at: String::new(),
                sort_order: 0,
                thinking: None,
                image_attachments: None,
            };
            db.resume_agent_turn_from_checkpoint(
                &response,
                None,
                None,
                "session-resume",
                &checkpoints[0].id,
            )
            .unwrap();
            session.executor = AgentExecutor::new(
                Box::new(StaticProvider),
                ToolRegistry::new(),
                AgentConfig {
                    model: Some("static-model".into()),
                    ..Default::default()
                },
            );
            let resumed = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                session.start_turn(AgentTurnInput::text(response.content)),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(resumed.run_id, run_id);
            assert_eq!(resumed.turn_id, turn_id);
            assert_resume_transcript_order(&db, &response.conversation_id, &response.id);
            assert_eq!(
                resumed.state,
                AgentTurnState::Terminal(crate::runtime::RuntimeTerminalStatus::Completed),
                "resume events: {:?}",
                session
                    .read_events(&run_id)
                    .await
                    .unwrap()
                    .iter()
                    .filter(|event| event.closes_run())
                    .collect::<Vec<_>>()
            );
            assert_eq!(db.get_agent_task_run(&run_id).unwrap().status, "completed");
            validate_runtime_turn_events(&session.read_events(&run_id).await.unwrap()).unwrap();
        }
    }

    #[tokio::test]
    async fn delegated_action_loss_pauses_parent_and_is_not_replayed_on_resume() {
        use crate::llm::ProviderStreamEvent;
        let db = Database::open_memory().unwrap();
        let (mut session, turn_id, run_id, calls) =
            scripted_session(&db, vec![], ToolRegistry::new());
        let worker = db
            .create_agent_subtask_run(&run_id, "worker-1", "worker", None, None)
            .unwrap();
        let safe_failure = db
            .create_agent_subtask_run(&run_id, "worker-2", "worker", None, None)
            .unwrap();
        db.finish_agent_subtask_run(&safe_failure.id, "failed", Some(&serde_json::json!({"recoveryRequired":{"actionReconciliation":false,"message":"safe transient error"}})), None).unwrap();
        assert!(db
            .agent_subtask_action_reconciliations(&run_id)
            .unwrap()
            .is_empty());
        session.executor = AgentExecutor::new(
            Box::new(ScriptedSessionProvider {
                calls: calls.clone(),
                settle_subtask: Some((db.clone(), worker.id.clone())),
                events: vec![ProviderStreamEvent::Chunk {
                    chunk: Box::new(StreamChunk {
                        delta: "Retained draft".into(),
                        tool_call_delta: None,
                        finish_reason: Some(FinishReason::Stop),
                        thinking_delta: None,
                        usage: Some(crate::llm::Usage {
                            prompt_tokens: 10,
                            completion_tokens: 2,
                            total_tokens: 12,
                            ..Default::default()
                        }),
                    }),
                }],
            }),
            ToolRegistry::new(),
            AgentConfig {
                model: Some("static-model".into()),
                ..Default::default()
            },
        );
        let handle = session
            .start_turn(AgentTurnInput::text("Continue the task"))
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(handle.state, AgentTurnState::Paused);
        assert!(session.result_for_run(&run_id).is_none());
        assert!(session
            .read_events(&run_id)
            .await
            .unwrap()
            .iter()
            .all(|event| !event.closes_run()));
        assert_eq!(
            db.get_usage_analytics(&crate::usage_analytics::UsageAnalyticsFilter::default())
                .unwrap()
                .totals
                .total_tokens,
            12
        );
        let checkpoint = db
            .list_task_resume_checkpoints(&run_id)
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(
            checkpoint.state["subtaskActionReconciliations"][&worker.id]["actionReconciliation"],
            true
        );
        assert!(checkpoint.resume_prompt.contains("Never redispatch"));
        let response = ConversationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: db.get_agent_task_run(&run_id).unwrap().conversation_id,
            role: Role::User,
            content: checkpoint.resume_prompt.clone(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: None,
            token_count: 1,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        };
        db.resume_agent_turn_from_checkpoint(
            &response,
            None,
            None,
            "delegated-resume",
            &checkpoint.id,
        )
        .unwrap();
        session.executor = AgentExecutor::new(
            Box::new(StaticProvider),
            ToolRegistry::new(),
            AgentConfig {
                model: Some("static-model".into()),
                ..Default::default()
            },
        );
        let resumed = session
            .start_turn(AgentTurnInput::text(response.content))
            .await
            .unwrap();
        assert_eq!(resumed.turn_id, turn_id);
        assert_resume_transcript_order(&db, &response.conversation_id, &response.id);
        assert_eq!(
            resumed.state,
            AgentTurnState::Terminal(crate::runtime::RuntimeTerminalStatus::Completed)
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "resumption must not replay the failed worker"
        );
        assert!(
            db.agent_subtask_action_reconciliations(&run_id)
                .unwrap()
                .contains_key(&worker.id),
            "checkpoint coverage does not fabricate a resolved worker outcome"
        );
    }

    #[tokio::test]
    async fn executor_session_keeps_a_durable_question_non_terminal() {
        let db = Database::open_memory().unwrap();
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(
            crate::tools::request_user_input_tool::RequestUserInputTool,
        ));
        let events = vec![crate::llm::ProviderStreamEvent::Chunk { chunk: Box::new(StreamChunk {
            delta: String::new(),
            tool_call_delta: Some(ToolCallDelta {
                id: "question-1".into(), name: Some("request_user_input".into()), index: Some(0), thought_signature: None,
                arguments_delta: serde_json::json!({"questions":[{"id":"scope","header":"Scope","question":"Which folder?","type":"short"}]}).to_string().into(),
            }),
            finish_reason: Some(FinishReason::ToolCalls), usage: None, thinking_delta: None,
        }) }];
        let (mut session, turn_id, run_id, calls) = scripted_session(&db, events, tools);
        let handle = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            session.start_turn(AgentTurnInput::text("Ask which folder to use")),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(handle.state, AgentTurnState::AwaitingUserInput);
        assert_eq!(
            db.get_conversation_turn(&turn_id).unwrap().status,
            "awaiting_user_input"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(session.result_for_run(&run_id).is_none());
        assert!(session
            .read_events(&run_id)
            .await
            .unwrap()
            .iter()
            .all(|event| !event.closes_run()));
        assert!(!db.list_task_resume_checkpoints(&run_id).unwrap().is_empty());
        let conversation = db.get_agent_task_run(&run_id).unwrap().conversation_id;
        let question = db
            .list_interaction_requests(Some(&conversation), false)
            .unwrap()
            .pop()
            .unwrap();
        db.mark_interaction_presented(&question.interaction_id)
            .unwrap();
        let response = ConversationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation.clone(),
            role: Role::User,
            content: "Which folder?\nRepo".into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: Some(serde_json::json!({"kind":"questionResponse", "version":2,
                "interactionId":question.interaction_id, "requestCallId":"question-1",
                "answers":[{"id":"scope","question":"Which folder?","answers":["Repo"]}]})),
            token_count: 4,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        };
        db.resume_agent_turn_with_interaction_response(
            &response,
            None,
            None,
            "question-resume",
            &crate::interaction::SubmitInteractionResponse {
                interaction_id: question.interaction_id,
                resume_token: question.resume_token,
                answers: std::collections::BTreeMap::from([("scope".into(), vec!["Repo".into()])]),
            },
        )
        .unwrap();
        session.executor = AgentExecutor::new(
            Box::new(StaticProvider),
            ToolRegistry::new(),
            AgentConfig {
                model: Some("static-model".into()),
                ..Default::default()
            },
        );
        let resumed = session
            .start_turn(AgentTurnInput::text(response.content))
            .await
            .unwrap();
        assert_eq!(
            resumed.state,
            AgentTurnState::Terminal(crate::runtime::RuntimeTerminalStatus::Completed)
        );
        assert_resume_transcript_order(&db, &conversation, &response.id);
    }

    fn settle_uncertain_worker(db: &Database, worker_id: &str) {
        db.finish_agent_subtask_run(worker_id, "failed", Some(&serde_json::json!({
            "recoveryRequired": {"actionReconciliation": true, "message": "Hosted action outcome is uncertain"}
        })), Some("connection lost")).unwrap();
    }

    #[tokio::test]
    async fn delegated_recovery_during_approval_prevents_the_file_write() {
        let db = Database::open_memory().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("must-not-be-created.txt");
        let workspace =
            crate::workspace::Workspace::validate(&[directory.path().to_string_lossy().into()])
                .unwrap();
        let mut tools = ToolRegistry::new().with_workspace(Some(workspace));
        tools.register(Box::new(crate::tools::create_file_tool::CreateFileTool));
        let events = vec![crate::llm::ProviderStreamEvent::Chunk {
            chunk: Box::new(StreamChunk {
                delta: String::new(),
                tool_call_delta: Some(ToolCallDelta {
                    id: "late-write".into(),
                    name: Some("create_file".into()),
                    index: Some(0),
                    thought_signature: None,
                    arguments_delta:
                        serde_json::json!({"path": target, "content": "must not execute"})
                            .to_string()
                            .into(),
                }),
                finish_reason: Some(FinishReason::ToolCalls),
                usage: None,
                thinking_delta: None,
            }),
        }];
        let (mut session, _, run_id, calls) = scripted_session(&db, vec![], ToolRegistry::new());
        let worker = db
            .create_agent_subtask_run(&run_id, "worker", "worker", None, None)
            .unwrap();
        let approvals = Arc::new(AtomicUsize::new(0));
        let callback_db = db.clone();
        let callback_count = approvals.clone();
        session.executor = AgentExecutor::new(
            Box::new(ScriptedSessionProvider {
                events,
                calls: calls.clone(),
                settle_subtask: None,
            }),
            tools,
            AgentConfig {
                model: Some("static-model".into()),
                dynamic_tool_visibility: false,
                tool_approval_mode: crate::approval::ToolApprovalMode::Ask,
                ..Default::default()
            },
        )
        .with_approval_callback(Arc::new(move |_| {
            callback_count.fetch_add(1, Ordering::SeqCst);
            settle_uncertain_worker(&callback_db, &worker.id);
            Box::pin(async { ApprovalDecision::AllowOnce })
        }));
        let handle = session
            .start_turn(AgentTurnInput::text("Create the requested file"))
            .await
            .unwrap();
        assert_eq!(approvals.load(Ordering::SeqCst), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            !target.exists(),
            "approval must not admit a new write over an uncertain worker"
        );
        assert_eq!(handle.state, AgentTurnState::Paused);
        assert!(session.result_for_run(&run_id).is_none());
        assert!(session
            .read_events(&run_id)
            .await
            .unwrap()
            .iter()
            .all(|event| !event.closes_run()));
    }

    #[tokio::test]
    async fn delegated_recovery_during_completion_hook_prevents_final_answer_commit() {
        let db = Database::open_memory().unwrap();
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join(".nexa/tools")).unwrap();
        #[cfg(windows)]
        let command = serde_json::json!({"program":"powershell.exe","args":["-NoProfile","-NonInteractive","-Command","Set-Content -LiteralPath hook-started.txt -Value entered; while (-not (Test-Path -LiteralPath hook-release.txt)) { Start-Sleep -Milliseconds 50 }; exit 0"],"timeoutSecs":20});
        #[cfg(not(windows))]
        let command = serde_json::json!({"program":"sh","args":["-c","touch hook-started.txt; while [ ! -f hook-release.txt ]; do sleep 0.05; done"],"timeoutSecs":20});
        std::fs::write(directory.path().join(".nexa/tools/check.json"), serde_json::to_vec(&serde_json::json!({"name":"check","description":"Completion barrier","command":command,"access":{"read":true,"write":true,"execute":true}})).unwrap()).unwrap();
        let workspace =
            crate::workspace::Workspace::validate(&[directory.path().to_string_lossy().into()])
                .unwrap();
        let mut tools = ToolRegistry::new().with_workspace(Some(workspace.clone()));
        tools.register(Box::new(crate::tools::project_tool::ProjectTool));
        let (mut session, _, run_id, _) = scripted_session(&db, vec![], ToolRegistry::new());
        let conversation_id = db.get_agent_task_run(&run_id).unwrap().conversation_id;
        db.execute_batch_for_test(
            "INSERT INTO projects(id,name) VALUES('recovery-hooks','Recovery hooks')",
        )
        .unwrap();
        db.conn()
            .execute(
                "UPDATE projects SET workspace_roots_json=?1 WHERE id='recovery-hooks'",
                [serde_json::to_string(&workspace.roots).unwrap()],
            )
            .unwrap();
        db.move_conversation_to_project(&conversation_id, "recovery-hooks")
            .unwrap();
        let catalog =
            crate::tools::project_tool::workspace_project_tool_catalog(&db, &workspace).unwrap();
        db.save_project_hook(
            "recovery-hooks",
            crate::project_hooks::ProjectHook {
                id: String::new(),
                name: "check".into(),
                manifest_hash: catalog.tools[0].manifest_hash.clone(),
                event: crate::project_hooks::HookEvent::BeforeComplete,
                arguments: serde_json::json!({}),
                enabled: true,
            },
        )
        .unwrap();
        session.executor = AgentExecutor::new(
            Box::new(StaticProvider),
            tools,
            AgentConfig {
                model: Some("static-model".into()),
                tool_approval_mode: crate::approval::ToolApprovalMode::AllowAll,
                ..Default::default()
            },
        );
        let worker = db
            .create_agent_subtask_run(&run_id, "worker", "worker", None, None)
            .unwrap();
        let during_hook = async {
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                while !directory.path().join("hook-started.txt").exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("completion hook reached its execution barrier");
            settle_uncertain_worker(&db, &worker.id);
            std::fs::write(directory.path().join("hook-release.txt"), "continue").unwrap();
        };
        let (result, ()) = tokio::join!(
            session.start_turn(AgentTurnInput::text("Answer the original question")),
            during_hook,
        );
        let hook_runs = db.project_hook_runs("recovery-hooks").unwrap();
        assert_eq!(hook_runs[0].status, "passed", "{}", hook_runs[0].detail);
        assert_eq!(result.unwrap().state, AgentTurnState::Paused);
        assert!(session.result_for_run(&run_id).is_none());
        assert!(session
            .read_events(&run_id)
            .await
            .unwrap()
            .iter()
            .all(|event| !event.closes_run()));
        assert!(!db
            .get_messages(&conversation_id)
            .unwrap()
            .iter()
            .any(|message| message.role == Role::Assistant && message.content == "session answer"));
    }

    fn create_started_run(db: &Database, suffix: &str) -> (String, String, String) {
        let conversation = db
            .create_conversation(&CreateConversationInput {
                provider: "static".to_string(),
                model: "static-model".to_string(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .expect("conversation");
        let message = ConversationMessage {
            id: format!("message-{suffix}"),
            conversation_id: conversation.id.clone(),
            role: Role::User,
            content: "hello".to_string(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            artifacts: None,
            token_count: 1,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        };
        db.add_message(&message).expect("message");
        let turn = db
            .create_conversation_turn(&conversation.id, &message.id, None)
            .expect("turn");
        let runtime = AgentTaskRuntime::new(db);
        let run = runtime
            .create_run(CreateTaskRunInput {
                conversation_id: &conversation.id,
                turn_id: &turn.id,
                user_message_id: &message.id,
                title: "Session turn",
                provider: Some("static"),
                model: Some("static-model"),
            })
            .expect("task run");
        runtime.start_run(&run.id, "routing").expect("start run");
        (conversation.id, turn.id, run.id)
    }

    #[async_trait]
    impl LlmProvider for StaticProvider {
        fn name(&self) -> &str {
            "static"
        }

        async fn list_models(&self) -> Result<Vec<String>, CoreError> {
            Ok(vec!["static-model".to_string()])
        }

        async fn complete(
            &self,
            _request: &CompletionRequest,
        ) -> Result<CompletionResponse, CoreError> {
            Err(CoreError::Llm("not used".to_string()))
        }

        async fn stream_events(
            &self,
            _request: &CompletionRequest,
        ) -> Result<futures::stream::BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError>
        {
            let chunks = vec![Ok(StreamChunk {
                delta: "session answer".to_string(),
                tool_call_delta: None::<ToolCallDelta>,
                finish_reason: Some(FinishReason::Stop),
                usage: None,
                thinking_delta: None,
            })];
            crate::llm::provider_events_from_chunk_stream(stream::iter(chunks).boxed())
        }

        async fn health_check(&self) -> Result<(), CoreError> {
            Ok(())
        }
    }

    #[async_trait]
    impl LlmProvider for CountingProvider {
        fn name(&self) -> &str {
            "counting"
        }

        async fn list_models(&self) -> Result<Vec<String>, CoreError> {
            Ok(vec!["static-model".to_string()])
        }

        async fn complete(
            &self,
            _request: &CompletionRequest,
        ) -> Result<CompletionResponse, CoreError> {
            Err(CoreError::Llm("not used".to_string()))
        }

        async fn stream_events(
            &self,
            _request: &CompletionRequest,
        ) -> Result<futures::stream::BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError>
        {
            self.stream_calls.fetch_add(1, Ordering::SeqCst);
            crate::llm::provider_events_from_chunk_stream(
                stream::iter(vec![Ok(StreamChunk {
                    delta: "must not execute".to_string(),
                    tool_call_delta: None::<ToolCallDelta>,
                    finish_reason: Some(FinishReason::Stop),
                    usage: None,
                    thinking_delta: None,
                })])
                .boxed(),
            )
        }

        async fn health_check(&self) -> Result<(), CoreError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn executor_agent_session_runs_turn_and_exposes_events() {
        let db = Database::open_memory().unwrap();
        let (conversation_id, _turn_id, run_id) = create_started_run(&db, "runs-turn");
        let mut runtime_config = AgentSessionConfig::default();
        runtime_config.conversation_id = Some(conversation_id);
        runtime_config.task_run_id = Some(run_id.clone());
        let executor_config = AgentConfig {
            model: Some("static-model".to_string()),
            ..AgentConfig::default()
        };
        let mut session = ExecutorAgentSession::new(
            runtime_config,
            Box::new(StaticProvider),
            ToolRegistry::new(),
            executor_config,
            db,
        );

        let handle = session
            .start_turn(AgentTurnInput::text("hello"))
            .await
            .expect("turn should run");
        let events = session.read_events(&handle.run_id).await.unwrap();
        let result = session.result_for_run(&handle.run_id).unwrap();
        let stored_events = session.db.list_agent_run_events(&handle.run_id).unwrap();

        validate_runtime_turn_events(&events).unwrap();
        validate_runtime_turn_events(&stored_events).unwrap();
        assert_eq!(handle.run_id, run_id);
        assert_eq!(
            result.status,
            crate::runtime::RuntimeTerminalStatus::Completed
        );
        assert_eq!(result.final_message.as_deref(), Some("session answer"));
        assert!(events.iter().any(|event| event.kind.as_str() == "done"));
        assert_eq!(stored_events.len(), events.len());
    }

    #[tokio::test]
    async fn executor_agent_session_reads_persisted_events_without_memory_cache() {
        let db = Database::open_memory().unwrap();
        let (conversation_id, _turn_id, run_id) = create_started_run(&db, "persisted");
        let mut runtime_config = AgentSessionConfig::default();
        runtime_config.conversation_id = Some(conversation_id);
        runtime_config.task_run_id = Some(run_id.clone());
        let executor_config = AgentConfig {
            model: Some("static-model".to_string()),
            ..AgentConfig::default()
        };
        let mut writer = ExecutorAgentSession::new(
            runtime_config.clone(),
            Box::new(StaticProvider),
            ToolRegistry::new(),
            executor_config.clone(),
            db.clone(),
        );

        let handle = writer
            .start_turn(AgentTurnInput::text("hello"))
            .await
            .expect("turn should run");
        let reader = ExecutorAgentSession::new(
            runtime_config,
            Box::new(StaticProvider),
            ToolRegistry::new(),
            executor_config,
            db,
        );

        let events = reader.read_events(&handle.run_id).await.unwrap();

        validate_runtime_turn_events(&events).unwrap();
        assert_eq!(handle.run_id, run_id);
        assert!(events.iter().any(|event| event.kind.as_str() == "done"));
    }

    #[tokio::test]
    async fn executor_agent_session_continues_the_outbox_durable_sequence() {
        let db = Database::open_memory().unwrap();
        let (conversation_id, turn_id, run_id) = create_started_run(&db, "outbox-sequence");
        db.save_agent_run_event(&status_event_for_session(
            &run_id,
            &turn_id,
            1,
            "Agent started",
            "running",
        ))
        .expect("seed durable head");

        let runtime_config = AgentSessionConfig {
            conversation_id: Some(conversation_id),
            task_run_id: Some(run_id.clone()),
            ..AgentSessionConfig::default()
        };
        let executor_config = AgentConfig {
            model: Some("static-model".to_string()),
            ..AgentConfig::default()
        };
        let mut session = ExecutorAgentSession::new(
            runtime_config,
            Box::new(StaticProvider),
            ToolRegistry::new(),
            executor_config,
            db.clone(),
        );

        let handle = session
            .start_turn(AgentTurnInput::text("hello"))
            .await
            .expect("turn should continue through the durable outbox");
        let events = session.read_events(&handle.run_id).await.unwrap();
        let task = db.get_agent_task_run(&run_id).expect("projected task run");

        assert_eq!(events.first().map(|event| event.event_seq), Some(1));
        assert_eq!(
            events.last().map(|event| event.event_seq),
            Some(events.len() as u64)
        );
        assert!(events
            .windows(2)
            .all(|pair| pair[1].event_seq == pair[0].event_seq + 1));
        assert_eq!(task.status, "completed");
    }

    #[tokio::test]
    async fn executor_agent_session_rejects_missing_task_backing_before_provider_work() {
        let db = Database::open_memory().unwrap();
        let stream_calls = Arc::new(AtomicUsize::new(0));
        let runtime_config = AgentSessionConfig {
            task_run_id: Some("missing-run".to_string()),
            ..AgentSessionConfig::default()
        };
        let mut session = ExecutorAgentSession::new(
            runtime_config,
            Box::new(CountingProvider {
                stream_calls: Arc::clone(&stream_calls),
            }),
            ToolRegistry::new(),
            AgentConfig {
                model: Some("static-model".to_string()),
                ..AgentConfig::default()
            },
            db.clone(),
        );

        let error = session
            .start_turn(AgentTurnInput::text("hello"))
            .await
            .expect_err("a durable task run is required");
        let stored_event_count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM agent_run_events", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert!(matches!(&error, CoreError::InvalidInput(_)));
        assert!(error.to_string().contains("existing Agent task run"));
        assert_eq!(stream_calls.load(Ordering::SeqCst), 0);
        assert_eq!(stored_event_count, 0);
    }

    #[tokio::test]
    async fn executor_agent_session_rejects_resumable_and_stopping_states_before_provider_work() {
        for status in ["paused", "awaiting_user_input", "cancelling"] {
            let db = Database::open_memory().unwrap();
            let (conversation_id, _turn_id, run_id) = create_started_run(&db, status);
            db.update_agent_task_run_progress(
                &run_id,
                Some(status),
                Some(status),
                None,
                None,
                None,
                None,
            )
            .expect("non-executable task state");
            let stream_calls = Arc::new(AtomicUsize::new(0));
            let mut session = ExecutorAgentSession::new(
                AgentSessionConfig {
                    conversation_id: Some(conversation_id),
                    task_run_id: Some(run_id.clone()),
                    ..AgentSessionConfig::default()
                },
                Box::new(CountingProvider {
                    stream_calls: Arc::clone(&stream_calls),
                }),
                ToolRegistry::new(),
                AgentConfig {
                    model: Some("static-model".to_string()),
                    ..AgentConfig::default()
                },
                db.clone(),
            );

            let error = session
                .start_turn(AgentTurnInput::text("hello"))
                .await
                .expect_err("a specialized resume or stop path is required");

            assert!(matches!(&error, CoreError::Conflict(_)));
            assert!(error.to_string().contains(status));
            assert_eq!(stream_calls.load(Ordering::SeqCst), 0);
            assert!(db
                .list_agent_run_events(&run_id)
                .expect("empty run ledger")
                .is_empty());
        }
    }

    #[tokio::test]
    async fn executor_agent_session_requires_a_task_run_id_before_provider_work() {
        let db = Database::open_memory().unwrap();
        let stream_calls = Arc::new(AtomicUsize::new(0));
        let mut session = ExecutorAgentSession::new(
            AgentSessionConfig::default(),
            Box::new(CountingProvider {
                stream_calls: Arc::clone(&stream_calls),
            }),
            ToolRegistry::new(),
            AgentConfig {
                model: Some("static-model".to_string()),
                ..AgentConfig::default()
            },
            db.clone(),
        );

        let error = session
            .start_turn(AgentTurnInput::text("hello"))
            .await
            .expect_err("task_run_id is required");
        let stored_event_count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM agent_run_events", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert!(matches!(&error, CoreError::InvalidInput(_)));
        assert!(error.to_string().contains("config.task_run_id"));
        assert_eq!(stream_calls.load(Ordering::SeqCst), 0);
        assert_eq!(stored_event_count, 0);
    }
}
