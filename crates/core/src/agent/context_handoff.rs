//! No-model-call window handoff. Persistence succeeds before live history changes.

use super::*;
use crate::context_history::{ContextHistoryArchive, ContextManagementMode, HistoryHandoffPlan};

impl AgentExecutor {
    pub(super) fn history_handoff_enabled(&self, conversation_id: Option<&str>) -> bool {
        self.config.context_management_mode == ContextManagementMode::History
            && conversation_id.is_some()
            && self
                .tools
                .tool_names()
                .iter()
                .any(|name| name == "context_history")
    }

    pub(super) fn handoff_context(
        &self,
        messages: &mut Vec<Message>,
        model: &str,
        run: context_compaction::CompactionRunContext<'_>,
    ) -> Result<bool, CoreError> {
        self.handoff_context_with_intent(messages, model, run, false, &self.cancel_token)
    }

    fn handoff_context_with_intent(
        &self,
        messages: &mut Vec<Message>,
        model: &str,
        run: context_compaction::CompactionRunContext<'_>,
        manual: bool,
        cancellation: &CancellationToken,
    ) -> Result<bool, CoreError> {
        let Some(conversation_id) = run.conversation_id else {
            return Ok(false);
        };
        let metrics = self.context_metrics_snapshot(model, messages, &[]);
        let Some(plan) = HistoryHandoffPlan::prepare(messages, run.active_request) else {
            return Ok(false);
        };
        let notes = run
            .db
            .get_agent_scratchpad(conversation_id)?
            .map(|notes| notes.content)
            .unwrap_or_default();
        let archive =
            ContextHistoryArchive::prepare(conversation_id, run.turn_id, messages, &notes)?;
        let next = plan.replacement(
            messages,
            Message::text(Role::System, archive.checkpoint_text()),
        );
        let before_tokens = metrics
            .messages
            .iter()
            .map(|message| message.cache_tokens)
            .fold(0, u32::saturating_add);
        let after_tokens = self
            .context_metrics_snapshot(model, &next, &[])
            .messages
            .iter()
            .map(|message| message.cache_tokens)
            .fold(0, u32::saturating_add);
        if !manual && after_tokens >= before_tokens {
            return Ok(false);
        }
        if self.cancel_token.is_cancelled() || cancellation.is_cancelled() {
            return Err(CoreError::Cancelled(
                "Context handoff cancelled before commit".into(),
            ));
        }
        if !crate::context_maintenance::commit_live_history_handoff(
            run.db,
            archive,
            model,
            &[&self.cancel_token, cancellation],
        )? {
            return Ok(false);
        }
        // Commit is the linearization point. A later cancellation must not
        // leave the live and persisted windows disagreeing about that commit.
        *messages = next;
        Ok(true)
    }

    pub(super) fn queue_context_fork(
        &self,
        request: crate::context_maintenance::ContextForkRequest,
    ) {
        let mut pending = self
            .pending_context_fork
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if pending
            .as_ref()
            .is_some_and(|pending| pending.cancellation.is_cancelled())
        {
            if let Some(cancelled) = pending.take() {
                cancelled.complete(Err(CoreError::Cancelled(
                    "Fork cancelled while queued".into(),
                )));
            }
        }
        if pending.is_some() {
            request.complete(Err(CoreError::InvalidInput(
                "A Fork is already waiting for this turn".into(),
            )));
        } else {
            *pending = Some(request);
        }
    }

    pub(super) fn apply_pending_context_fork(
        &self,
        messages: &mut Vec<Message>,
        model: &str,
        run: context_compaction::CompactionRunContext<'_>,
        terminal: bool,
    ) -> bool {
        let Some(request) = self
            .pending_context_fork
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
            return false;
        };
        let before = messages.len();
        let tokens_before = self.context_input_tokens(model, messages, &[]);
        let result = (|| {
            request.begin()?;
            if !self.history_handoff_enabled(run.conversation_id) {
                return Err(CoreError::InvalidInput("This running turn does not support history Fork; start a new turn after enabling history mode".into()));
            }
            let conversation_id = run.conversation_id.unwrap_or_default().to_string();
            let db = run.db;
            let changed = self.handoff_context_with_intent(
                messages,
                model,
                run,
                true,
                &request.cancellation,
            )?;
            let projection =
                crate::context_maintenance::load_context_projection(db, &conversation_id)?;
            Ok(crate::context_maintenance::ContextCompactionResult {
                conversation_id,
                checkpoint_id: projection.checkpoint_id,
                messages_before: before,
                messages_after: messages.len(),
                tokens_before,
                tokens_after: self.context_input_tokens(model, messages, &[]),
                evicted_messages: before.saturating_sub(messages.len()),
                summary_kind: if !changed {
                    "noop"
                } else if terminal {
                    "history_fork_terminal"
                } else {
                    "history_fork_live"
                }
                .into(),
                fallback_reason: None,
            })
        })();
        let changed = result
            .as_ref()
            .is_ok_and(|result| result.summary_kind != "noop");
        request.complete(result);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ToolCallRequest;

    struct NoModelCalls;
    #[async_trait::async_trait]
    impl LlmProvider for NoModelCalls {
        fn name(&self) -> &str {
            "handoff-test"
        }
        async fn list_models(&self) -> Result<Vec<String>, CoreError> {
            Ok(vec![])
        }
        async fn complete(
            &self,
            _: &crate::llm::CompletionRequest,
        ) -> Result<crate::llm::CompletionResponse, CoreError> {
            panic!("A history handoff must not request an LLM summary")
        }
        async fn stream_events(
            &self,
            _: &crate::llm::CompletionRequest,
        ) -> Result<futures::stream::BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError>
        {
            panic!("A history handoff must not sample the model")
        }
        async fn health_check(&self) -> Result<(), CoreError> {
            Ok(())
        }
    }

    fn setup() -> (AgentExecutor, Database, String) {
        let db = Database::open_memory().unwrap();
        let id = db
            .create_conversation(&crate::conversation::CreateConversationInput {
                provider: "custom".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap()
            .id;
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(
            crate::tools::context_history_tool::ContextHistoryTool,
        ));
        let executor = AgentExecutor::new(
            Box::new(NoModelCalls),
            tools,
            AgentConfig {
                context_management_mode: ContextManagementMode::History,
                context_window: Some(64_000),
                ..AgentConfig::default()
            },
        );
        (executor, db, id)
    }

    fn history() -> Vec<Message> {
        let mut messages = vec![
            Message::text(Role::System, "Stable policy"),
            Message::text(Role::User, "Complete the report and verify the result"),
        ];
        for index in 0..8 {
            let id = format!("call-{index}");
            let mut assistant = Message::text(Role::Assistant, "Checking evidence");
            assistant.tool_calls = Some(vec![ToolCallRequest {
                id: id.clone(),
                name: "read_file".into(),
                arguments: "{}".into(),
                thought_signature: None,
            }]);
            let mut tool = Message::text(Role::Tool, "Exact historical evidence. ".repeat(1000));
            tool.name = Some(id);
            messages.extend([assistant, tool]);
        }
        messages
    }

    #[test]
    fn handoff_retains_task_corrections_and_rules_but_no_prior_trajectory() {
        let mut messages = history();
        let request = messages[1].clone();
        messages[2].reasoning_content = Some("Old reasoning must be flushed".into());
        messages.push(Message::text(
            Role::System,
            "## Earlier conversation context (compacted)\nold summary",
        ));
        messages.push(Message::text(
            Role::User,
            "Keep the original workbook layout",
        ));
        let plan = HistoryHandoffPlan::prepare(&messages, Some(&request)).unwrap();
        let next = plan.replacement(
            &messages,
            Message::text(Role::System, "fresh archive locator"),
        );
        assert_eq!(next.len(), 4);
        assert_eq!(next[0].text_content(), "Stable policy");
        assert_eq!(next[2].text_content(), request.text_content());
        assert_eq!(next[3].text_content(), "Keep the original workbook layout");
        assert!(next.iter().all(
            |message| !matches!(message.role, Role::Assistant | Role::Tool)
                && message.reasoning_content.is_none()
        ));
    }

    #[test]
    fn unresolved_tool_results_block_a_handoff_boundary() {
        let mut history = history();
        history.remove(3);
        // No boundary after the incomplete call is safe, even though later
        // unrelated tool pairs are complete.
        assert!(HistoryHandoffPlan::prepare(&history, None).is_none());
    }

    #[test]
    fn fork_does_not_retain_expired_step_controllers_as_stable_policy() {
        let mut messages = history();
        let request = messages[1].clone();
        for _ in 0..20 {
            messages.push(
                prompt_ir::controller_state_message(
                    "Previous step evidence and recovery guidance.\n".repeat(100),
                )
                .unwrap(),
            );
        }
        let plan = HistoryHandoffPlan::prepare(&messages, Some(&request)).unwrap();
        let next = plan.replacement(&messages, Message::text(Role::System, "archive locator"));
        assert_eq!(
            next.len(),
            3,
            "a fresh window must not inherit obsolete step controllers"
        );
        assert_eq!(next[0].text_content(), "Stable policy");
        assert_eq!(next[2].text_content(), request.text_content());
    }

    #[test]
    fn automatic_handoff_counts_reasoning_that_will_be_removed() {
        let (executor, db, id) = setup();
        let request = Message::text(Role::User, "Continue verifying the artifact");
        let mut assistant = Message::text(Role::Assistant, "Checking");
        assistant.reasoning_content = Some("Old reasoning to flush.\n".repeat(2000));
        let mut messages = vec![
            Message::text(Role::System, "Stable policy"),
            request.clone(),
            assistant,
        ];
        let changed = executor
            .handoff_context(
                &mut messages,
                "gpt-4o",
                context_compaction::CompactionRunContext {
                    db: &db,
                    conversation_id: Some(&id),
                    turn_id: None,
                    active_request: Some(&request),
                },
            )
            .unwrap();
        assert!(
            changed,
            "reasoning-dominated context must not be judged non-reducible from text-only costs"
        );
        assert!(messages
            .iter()
            .all(|message| message.reasoning_content.is_none()));
        assert!(messages
            .iter()
            .any(|message| message.parts == request.parts));
    }

    #[test]
    fn interrupted_old_turn_does_not_block_a_fresh_active_window() {
        let mut messages = history();
        messages.remove(3); // An old cancelled turn has an unpaired tool call.
        let request = Message::text(Role::User, "Continue with the verified files");
        messages.push(request.clone());
        messages.push(Message::text(Role::Assistant, "Current completed work"));
        let plan = HistoryHandoffPlan::prepare(&messages, Some(&request))
            .expect("a historical interrupted tool cannot pin all later windows forever");
        let next = plan.replacement(&messages, Message::text(Role::System, "archive locator"));
        assert_eq!(next.len(), 3);
        assert_eq!(next[2].text_content(), request.text_content());
    }

    #[tokio::test]
    async fn history_mode_retains_the_task_and_archives_notes_without_inheriting_old_reasoning() {
        let (executor, db, id) = setup();
        db.upsert_agent_scratchpad(
            &id,
            "Report written. Verify the totals next; do not regenerate it.",
        )
        .unwrap();
        let mut messages = history();
        for (index, message) in messages.iter().enumerate() {
            db.add_message(&crate::conversation::ConversationMessage {
                id: format!("persisted-{index}"),
                conversation_id: id.clone(),
                role: message.role.clone(),
                content: message.text_content(),
                tool_call_id: message.name.clone(),
                tool_calls: message.tool_calls.clone().unwrap_or_default(),
                artifacts: None,
                token_count: 1,
                created_at: String::new(),
                sort_order: index as i64,
                thinking: message.reasoning_content.clone(),
                image_attachments: None,
            })
            .unwrap();
        }
        let original_request = messages[1].text_content();
        let (tx, _rx) = mpsc::channel(4);
        let usage = executor
            .aggressive_compact(
                &mut messages,
                "gpt-4o",
                &tx,
                context_compaction::CompactionRunContext {
                    db: &db,
                    conversation_id: Some(&id),
                    turn_id: None,
                    active_request: None,
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(usage.total_tokens, 0);
        assert_eq!(messages[0].text_content(), "Stable policy");
        assert!(messages.iter().any(
            |message| message.role == Role::User && message.text_content() == original_request
        ));
        assert!(!messages
            .iter()
            .any(|message| message.text_content().contains("Verify the totals next")));
        assert!(messages
            .iter()
            .all(|message| !matches!(message.role, Role::Assistant | Role::Tool)));
        let windows = db.list_context_history(&id, None, 10).unwrap();
        let window = windows["windows"][0]["windowId"].as_str().unwrap();
        assert!(db
            .read_context_history_notes(&id, window, 0, 16000)
            .unwrap()["notes"]
            .as_str()
            .unwrap()
            .contains("Verify the totals next"));
        assert!(db
            .read_context_history_notes("other-conversation", window, 0, 16000)
            .is_err());
        let reopened = crate::context_maintenance::load_context_projection(&db, &id).unwrap();
        assert!(reopened.projected);
        assert!(reopened
            .messages
            .iter()
            .all(|message| !matches!(message.role, Role::Assistant | Role::Tool)));
        assert!(reopened
            .messages
            .iter()
            .any(|message| message.content == original_request));
        let next_order = db.next_message_sort_order(&id).unwrap();
        db.add_message(&crate::conversation::ConversationMessage {
            id: "after-handoff".into(),
            conversation_id: id.clone(),
            role: Role::Assistant,
            content: "Verified result from fresh evidence".into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            artifacts: None,
            token_count: 1,
            created_at: String::new(),
            sort_order: next_order,
            thinking: None,
            image_attachments: None,
        })
        .unwrap();
        let resumed = crate::context_maintenance::load_context_projection(&db, &id).unwrap();
        assert!(resumed.projected);
        assert_eq!(
            resumed
                .messages
                .iter()
                .filter(|message| message.role == Role::Assistant)
                .count(),
            1
        );
        assert_eq!(resumed.messages.last().unwrap().id, "after-handoff");
        assert_eq!(
            db.list_context_history(&id, None, 10).unwrap()["windows"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(!db
            .search_context_history(&id, "Exact historical evidence", 10)
            .unwrap()["matches"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn failed_storage_cannot_cut_the_live_context() {
        let (executor, db, id) = setup();
        db.conn().execute_batch("CREATE TRIGGER corrupt_history AFTER INSERT ON context_history_items BEGIN UPDATE context_history_items SET content='corrupt' WHERE window_id=NEW.window_id AND ordinal=NEW.ordinal; END;").unwrap();
        let mut messages = history();
        let original = serde_json::to_value(&messages).unwrap();
        let result = executor.handoff_context(
            &mut messages,
            "gpt-4o",
            context_compaction::CompactionRunContext {
                db: &db,
                conversation_id: Some(&id),
                turn_id: None,
                active_request: None,
            },
        );
        assert!(result.is_err());
        assert_eq!(serde_json::to_value(messages).unwrap(), original);
        assert_eq!(
            db.list_context_history(&id, None, 10).unwrap()["windows"],
            serde_json::json!([])
        );
    }

    #[test]
    fn manual_fork_observes_parent_cancellation_after_database_admission() {
        for persisted in [false, true] {
            let (_, db, id) = setup();
            let messages = history();
            if persisted {
                for (index, message) in messages.iter().enumerate() {
                    db.add_message(&crate::conversation::ConversationMessage {
                        id: format!("parent-cancel-{index}"),
                        conversation_id: id.clone(),
                        role: message.role.clone(),
                        content: message.text_content(),
                        tool_call_id: message.name.clone(),
                        tool_calls: message.tool_calls.clone().unwrap_or_default(),
                        artifacts: None,
                        token_count: 1,
                        created_at: String::new(),
                        sort_order: index as i64,
                        thinking: None,
                        image_attachments: None,
                    })
                    .unwrap();
                }
            }
            let archive = ContextHistoryArchive::prepare(&id, None, &messages, "").unwrap();
            let parent = CancellationToken::new();
            let manual = CancellationToken::new();
            let blocked = db.conn();
            let (started_tx, started_rx) = std::sync::mpsc::channel();
            let result = std::thread::scope(|scope| {
                let worker = scope.spawn(|| {
                    started_tx.send(()).unwrap();
                    crate::context_maintenance::commit_live_history_handoff(
                        &db,
                        archive,
                        "test",
                        &[&parent, &manual],
                    )
                });
                started_rx.recv().unwrap();
                parent.cancel();
                drop(blocked);
                worker.join().unwrap()
            });
            assert!(matches!(result, Err(CoreError::Cancelled(_))), "{result:?}");
            assert_eq!(
                db.list_context_history(&id, None, 10).unwrap()["windows"],
                serde_json::json!([])
            );
        }
    }
}
