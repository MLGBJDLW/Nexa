//! Phone projections avoid loading image blobs, tool payloads, and full traces.
use crate::{db::Database, error::CoreError};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

impl crate::db_executor::DatabaseExecutor {
    pub async fn remote_pending_interactions(
        &self,
        conversation_id: Option<String>,
    ) -> Result<Vec<crate::interaction::InteractionRequest>, CoreError> {
        // Listing also expires due requests and records their status transitions.
        // File-backed readers are query-only, unlike the shared in-memory test DB.
        self.write(move |db| db.list_interaction_requests(conversation_id.as_deref(), false))
            .await
            .map(|result| result.value)
    }
}
impl Database {
    pub fn remote_conversations(&self, before: Option<&str>) -> Result<Value, CoreError> {
        let conn = self.conn();
        let mut query=conn.prepare("SELECT id,title,model,updated_at FROM conversations WHERE archived_at IS NULL AND (?1 IS NULL OR (updated_at,id)<(SELECT updated_at,id FROM conversations WHERE id=?1)) ORDER BY updated_at DESC,id DESC LIMIT 51")?;
        let mut rows=query.query_map([before],|row|Ok(json!({"id":row.get::<_,String>(0)?,"title":row.get::<_,String>(1)?,"model":row.get::<_,String>(2)?,"updatedAt":row.get::<_,String>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
        let more = rows.len() > 50;
        rows.truncate(50);
        let cursor = more.then(|| rows.last().unwrap()["id"].clone());
        Ok(json!({"items":rows,"nextCursor":cursor}))
    }
    pub fn remote_messages(
        &self,
        conversation_id: &str,
        before: Option<i64>,
    ) -> Result<Value, CoreError> {
        let conversation = self.get_conversation(conversation_id)?;
        let conn = self.conn();
        let mut query=conn.prepare("SELECT id,role,substr(content,1,8000),length(content),sort_order,created_at FROM messages WHERE conversation_id=?1 AND role IN ('user','assistant') AND (?2 IS NULL OR sort_order<?2) ORDER BY sort_order DESC LIMIT 31")?;
        let mut rows=query.query_map(params![conversation_id,before],|row|Ok(json!({"id":row.get::<_,String>(0)?,"role":row.get::<_,String>(1)?,"content":row.get::<_,String>(2)?,"totalChars":row.get::<_,u64>(3)?,"sortOrder":row.get::<_,i64>(4)?,"createdAt":row.get::<_,String>(5)?})))?.collect::<Result<Vec<_>,_>>()?;
        let more = rows.len() > 30;
        rows.truncate(30);
        let cursor = more.then(|| rows.last().unwrap()["sortOrder"].clone());
        rows.reverse();
        Ok(
            json!({"conversation":{"id":conversation.id,"title":conversation.title,"model":conversation.model},"messages":rows,"beforeOrder":cursor}),
        )
    }
    pub fn remote_message_text(
        &self,
        conversation_id: &str,
        message_id: &str,
        offset: u32,
    ) -> Result<Value, CoreError> {
        let conn = self.conn();
        Ok(conn.query_row("SELECT substr(content,?3+1,8000),length(content) FROM messages WHERE conversation_id=?1 AND id=?2 AND role IN ('user','assistant')",params![conversation_id,message_id,offset],|row|Ok(json!({"content":row.get::<_,String>(0)?,"totalChars":row.get::<_,u64>(1)?,"offset":offset})))?)
    }
    pub fn remote_latest_run(&self, conversation_id: &str) -> Result<Option<String>, CoreError> {
        let conn = self.conn();
        Ok(conn.query_row("SELECT id FROM agent_task_runs WHERE conversation_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 1",[conversation_id],|row|row.get(0)).optional()?)
    }

    /// A bounded run projection with a frozen ledger head. Sequence holes are
    /// intentional when live previews were not persisted; the phone must use
    /// this authority rather than wait forever for every integer in the stream.
    pub fn remote_run_page(
        &self,
        conversation_id: &str,
        known_run_id: Option<&str>,
        after_sequence: u64,
        durable_high_water: Option<u64>,
    ) -> Result<Value, CoreError> {
        let id = self
            .remote_latest_run(conversation_id)?
            .or_else(|| known_run_id.map(str::to_owned));
        let Some(id) = id else {
            return Ok(
                json!({"run":null,"events":[],"hasMore":false,"durableHighWater":0,"nextSequence":null}),
            );
        };
        let run = self.get_agent_task_run(&id)?;
        if run.conversation_id != conversation_id {
            return Err(CoreError::NotFound(
                "Run does not belong to this conversation".into(),
            ));
        }
        let same_run = known_run_id == Some(id.as_str());
        let page = self.list_agent_run_event_page(
            &id,
            if same_run { after_sequence } else { 0 },
            if same_run { durable_high_water } else { None },
            128,
        )?;
        // Avoid deserializing the potentially large private turn trace.
        let final_message_id: Option<String> = self.conn().query_row(
            "SELECT assistant_message_id FROM conversation_turns WHERE id=?1 AND conversation_id=?2",
            params![run.turn_id, conversation_id],
            |row| row.get(0),
        ).optional()?.flatten();
        Ok(json!({
            "run":{"id":run.id,"status":run.status,"phase":run.phase,"turnId":run.turn_id,
                "finalMessageId":final_message_id,"errorMessage":run.error_message},
            "events":page.events,"hasMore":page.has_more,"nextSequence":page.next_after_event_seq,
            "durableHighWater":page.durable_high_water,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        conversation::{ConversationMessage, CreateConversationInput},
        llm::Role,
    };
    #[tokio::test]
    async fn remote_interactions_work_with_a_file_backed_database() {
        let directory = tempfile::tempdir().unwrap();
        let db = Database::new(directory.path().join("remote.db")).unwrap();
        let executor = crate::db_executor::DatabaseExecutor::new(db, 8).unwrap();
        let requests = executor.remote_pending_interactions(None).await;
        assert!(
            requests.is_ok(),
            "remote Chat must load questions: {requests:?}"
        );
        assert!(requests.unwrap().is_empty());
    }
    #[test]
    fn remote_pages_preserve_unicode_and_skip_large_runtime_payloads() {
        let db = Database::open_memory().unwrap();
        let chat = db
            .create_conversation(&CreateConversationInput {
                provider: "custom".into(),
                model: "fixture".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        for index in 0..35 {
            db.add_message(&ConversationMessage{id:format!("m{index}"),conversation_id:chat.id.clone(),role:Role::User,content:if index==34{"你好🙂".repeat(3000)}else{"message".into()},tool_call_id:None,tool_calls:vec![],artifacts:Some(json!({"providerTurnEnvelope":{"secret":"never return"},"huge":"x".repeat(100_000)})),token_count:1,created_at:String::new(),sort_order:index,thinking:None,image_attachments:None}).unwrap();
        }
        let page = db.remote_messages(&chat.id, None).unwrap();
        assert_eq!(page["messages"].as_array().unwrap().len(), 30);
        assert_eq!(page["messages"][0]["sortOrder"], 5);
        assert_eq!(page["beforeOrder"], 5);
        assert!(!page.to_string().contains("never return"));
        let first = page["messages"][29]["content"].as_str().unwrap();
        assert_eq!(first.chars().count(), 8000);
        let tail = db.remote_message_text(&chat.id, "m34", 8000).unwrap();
        assert_eq!(
            format!("{first}{}", tail["content"].as_str().unwrap()),
            "你好🙂".repeat(3000)
        );
        let older = db.remote_messages(&chat.id, Some(5)).unwrap();
        assert_eq!(older["messages"].as_array().unwrap().len(), 5);
        assert!(older["beforeOrder"].is_null());
        assert!(db.remote_message_text("other", "m34", 0).is_err());
        assert!(db.remote_latest_run(&chat.id).unwrap().is_none());
        let mut latest = String::new();
        for message_id in ["m0", "m1"] {
            let turn = db
                .create_conversation_turn(&chat.id, message_id, None)
                .unwrap();
            latest = db
                .create_agent_task_run(&chat.id, &turn.id, message_id, "test", None, None)
                .unwrap()
                .id;
        }
        assert_eq!(
            db.remote_latest_run(&chat.id).unwrap().as_deref(),
            Some(latest.as_str())
        );
        assert!(db.remote_latest_run("other").unwrap().is_none());
    }

    #[test]
    fn remote_run_recovery_preserves_sparse_sequences_and_frozen_pages() {
        use crate::agent::StreamBlockChannel;
        use crate::agent_run::AgentRunEvent;
        let db = Database::open_memory().unwrap();
        let chat = db
            .create_conversation(&CreateConversationInput {
                provider: "custom".into(),
                model: "fixture".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        db.add_message(&ConversationMessage {
            id: "user".into(),
            conversation_id: chat.id.clone(),
            role: Role::User,
            content: "test".into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: None,
            token_count: 1,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        })
        .unwrap();
        let turn = db.create_conversation_turn(&chat.id, "user", None).unwrap();
        let run = db
            .create_agent_task_run(&chat.id, &turn.id, "user", "test", None, None)
            .unwrap();
        let events = (0..130)
            .map(|index| {
                AgentRunEvent::output_delta(
                    &run.id,
                    Some(&turn.id),
                    index * 2 + 1,
                    "answer",
                    StreamBlockChannel::Answer,
                    index as usize,
                    "x",
                )
            })
            .collect::<Vec<_>>();
        db.save_agent_run_events(&events).unwrap();
        let first = db.remote_run_page(&chat.id, None, 999, Some(999)).unwrap();
        assert_eq!(first["events"].as_array().unwrap().len(), 128);
        assert_eq!(first["durableHighWater"], 259);
        assert_eq!(first["nextSequence"], 255);
        assert_eq!(first["hasMore"], true);
        db.save_agent_run_events(&[AgentRunEvent::output_delta(
            &run.id,
            Some(&turn.id),
            261,
            "answer",
            StreamBlockChannel::Answer,
            130,
            "y",
        )])
        .unwrap();
        let second = db
            .remote_run_page(&chat.id, Some(&run.id), 255, Some(259))
            .unwrap();
        assert_eq!(second["events"].as_array().unwrap().len(), 2);
        assert_eq!(second["nextSequence"], 259);
        assert_eq!(second["hasMore"], false);
        assert!(second["run"]["finalMessageId"].is_null());
        assert!(db.remote_run_page("other", Some(&run.id), 0, None).is_err());
    }
}
