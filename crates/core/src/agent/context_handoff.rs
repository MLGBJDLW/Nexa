//! No-model-call window handoff. Persistence succeeds before live history changes.

use super::context_window::ContextReductionPlan;
use super::*;
use crate::context_history::{ContextHistoryArchive, ContextManagementMode};

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
        target: u32,
        run: context_compaction::CompactionRunContext<'_>,
    ) -> Result<bool, CoreError> {
        let Some(conversation_id) = run.conversation_id else {
            return Ok(false);
        };
        let metrics = self.context_metrics_snapshot(model, messages, &[]);
        let costs = metrics
            .messages
            .iter()
            .map(|message| message.cache_tokens)
            .collect::<Vec<_>>();
        let Some(plan) =
            ContextReductionPlan::prepare_with_costs(messages, target, run.active_request, &costs)
        else {
            return Ok(false);
        };
        let notes = run
            .db
            .get_agent_scratchpad(conversation_id)?
            .map(|notes| notes.content)
            .unwrap_or_default();
        let archive = ContextHistoryArchive::prepare(
            conversation_id,
            run.turn_id,
            plan.evicted(messages),
            &notes,
        )?;
        let next = plan.replacement(
            messages,
            Message::text(Role::System, archive.checkpoint_text()),
        );
        let before_tokens = metrics
            .messages
            .iter()
            .map(|message| message.base_tokens)
            .sum::<u32>();
        let after_tokens = self
            .context_metrics_snapshot(model, &next, &[])
            .messages
            .iter()
            .map(|message| message.base_tokens)
            .sum::<u32>();
        if after_tokens >= before_tokens {
            return Ok(false);
        }
        if self.cancel_token.is_cancelled() {
            return Err(CoreError::Cancelled(
                "Context handoff cancelled before commit".into(),
            ));
        }
        run.db.archive_context_history(&archive)?;
        if self.cancel_token.is_cancelled() {
            return Err(CoreError::Cancelled(
                "Context handoff cancelled; working history retained".into(),
            ));
        }
        *messages = next;
        Ok(true)
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
    fn one_long_user_turn_can_switch_without_splitting_tool_batches() {
        let history = history();
        let plan = ContextReductionPlan::prepare(&history, "gpt-4o", 1000, None).unwrap();
        let boundary = 1 + plan.evicted(&history).len();
        assert!(boundary > 2);
        assert_eq!(history[boundary].role, Role::Assistant);
        assert_eq!(history[boundary - 1].role, Role::Tool);
        assert_eq!(
            history[boundary..]
                .iter()
                .filter(|message| message.role == Role::Assistant)
                .count(),
            2
        );
    }

    #[test]
    fn unresolved_tool_results_block_a_handoff_boundary() {
        let mut history = history();
        history.remove(3);
        // No boundary after the incomplete call is safe, even though later
        // unrelated tool pairs are complete.
        assert!(ContextReductionPlan::prepare(&history, "gpt-4o", 1000, None).is_none());
    }

    #[tokio::test]
    async fn history_mode_preserves_request_and_notes_without_a_summary_call() {
        let (executor, db, id) = setup();
        db.upsert_agent_scratchpad(
            &id,
            "Report written. Verify the totals next; do not regenerate it.",
        )
        .unwrap();
        let mut messages = history();
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
        assert!(messages
            .iter()
            .any(|message| message.text_content().contains("Verify the totals next")));
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
            1000,
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
}
