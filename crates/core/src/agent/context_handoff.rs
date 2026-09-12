//! No-model-call window handoff. Persistence succeeds before live history changes.

use super::*;
use crate::context_history::{ContextHistoryArchive, ContextManagementMode};
use rusqlite::OptionalExtension;
use std::collections::HashSet;

fn is_checkpoint(message: &Message) -> bool {
    message.role == Role::System
        && message
            .text_content()
            .starts_with("## Earlier conversation context")
}

/// Allow a long single user turn to release old completed tool exchanges. Never
/// split an assistant call batch from its results or discard the newest two
/// assistant exchanges. The active request is separately retained verbatim.
fn handoff_boundary(messages: &[Message], model: &str, target: u32) -> Option<usize> {
    let prefix = messages
        .iter()
        .position(|message| message.role != Role::System || is_checkpoint(message))?;
    let assistants = messages
        .iter()
        .enumerate()
        .skip(prefix)
        .filter_map(|(index, message)| (message.role == Role::Assistant).then_some(index))
        .collect::<Vec<_>>();
    let latest_user = messages
        .iter()
        .rposition(|message| message.role == Role::User)?;
    let last_boundary = assistants
        .iter()
        .rev()
        .nth(1)
        .copied()
        .unwrap_or(latest_user);
    let mut suffix = vec![0_u32; messages.len() + 1];
    for index in (0..messages.len()).rev() {
        suffix[index] = suffix[index + 1]
            .saturating_add(estimate_message_tokens_for_model(model, &messages[index]));
    }
    let mut pending = HashSet::new();
    let mut selected = None;
    for index in prefix..last_boundary {
        let message = &messages[index];
        if let Some(calls) = &message.tool_calls {
            for call in calls {
                pending.insert(call.id.as_str());
            }
        }
        if message.role == Role::Tool {
            if let Some(id) = message.name.as_deref() {
                pending.remove(id);
            }
        }
        let boundary = index + 1;
        if pending.is_empty() && messages[boundary].role != Role::Tool {
            selected = Some(boundary);
            if suffix[boundary] <= target {
                break;
            }
        }
    }
    selected
}

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
        let Some(boundary) = handoff_boundary(messages, model, target) else {
            return Ok(false);
        };
        let prefix = messages
            .iter()
            .position(|message| message.role != Role::System || is_checkpoint(message))
            .unwrap_or(messages.len());
        let notes = run
            .db
            .get_agent_scratchpad(conversation_id)?
            .map(|notes| notes.content)
            .unwrap_or_default();
        let archive = ContextHistoryArchive::prepare(
            conversation_id,
            run.turn_id,
            &messages[prefix..boundary],
            &notes,
        )?;
        // Locate the original active-turn request in the already-normalized
        // history; never inject raw database text around privacy redaction.
        let original: Option<String> = if let Some(turn_id) = run.turn_id {
            run.db.conn().query_row(
                "SELECT m.content FROM conversation_turns t JOIN messages m ON m.id=t.user_message_id WHERE t.id=?1 AND t.conversation_id=?2 AND m.role='user'",
                rusqlite::params![turn_id,conversation_id], |row| row.get(0),
            ).optional()?
        } else {
            None
        };
        let user_indices = messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| (message.role == Role::User).then_some(index))
            .collect::<Vec<_>>();
        let mut retained_users = user_indices
            .iter()
            .rev()
            .take(2)
            .copied()
            .collect::<Vec<_>>();
        if let Some(original) = original {
            if let Some(index) = user_indices
                .iter()
                .find(|index| messages[**index].text_content() == original)
            {
                retained_users.push(*index);
            }
        }
        retained_users.sort_unstable();
        retained_users.dedup();
        let mut next = messages[..prefix].to_vec();
        next.extend(
            messages[prefix..boundary]
                .iter()
                .filter(|message| message.role == Role::System && !is_checkpoint(message))
                .cloned(),
        );
        next.push(Message::text(Role::System, archive.checkpoint_text()));
        for index in retained_users.into_iter().filter(|index| *index < boundary) {
            next.push(messages[index].clone());
        }
        next.extend_from_slice(&messages[boundary..]);
        let tokens = |messages: &[Message]| {
            messages
                .iter()
                .map(|message| estimate_message_tokens_for_model(model, message))
                .fold(0_u32, u32::saturating_add)
        };
        if tokens(&next) >= tokens(messages) {
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
        let boundary = handoff_boundary(&history, "gpt-4o", 1000).unwrap();
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
        let boundary = handoff_boundary(&history, "gpt-4o", 1000).unwrap();
        assert_eq!(boundary, 2);
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
