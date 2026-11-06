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
    /// Internal delegated transcripts have their own desktop workspace and are
    /// never independent phone chats, including direct-ID reads and controls.
    pub fn require_remote_conversation(&self, conversation_id: &str) -> Result<(), CoreError> {
        let visible = self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1 AND parent_conversation_id IS NULL)",
            [conversation_id], |row| row.get::<_, bool>(0),
        )?;
        if !visible {
            return Err(CoreError::NotFound("Remote conversation not found".into()));
        }
        Ok(())
    }

    pub fn remote_conversations(&self, before: Option<&str>) -> Result<Value, CoreError> {
        let conn = self.conn();
        let mut query=conn.prepare("SELECT id,title,model,updated_at FROM conversations WHERE archived_at IS NULL AND parent_conversation_id IS NULL AND (?1 IS NULL OR (updated_at,id)<(SELECT updated_at,id FROM conversations WHERE id=?1 AND parent_conversation_id IS NULL)) ORDER BY updated_at DESC,id DESC LIMIT 51")?;
        let mut rows=query.query_map([before],|row|Ok(json!({"id":row.get::<_,String>(0)?,"title":row.get::<_,String>(1)?,"model":row.get::<_,String>(2)?,"updatedAt":row.get::<_,String>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
        let more = rows.len() > 50;
        rows.truncate(50);
        let cursor = more.then(|| rows.last().unwrap()["id"].clone());
        Ok(json!({"items":rows,"nextCursor":cursor}))
    }
    pub fn remote_archived_conversations(&self, before: Option<&str>) -> Result<Value, CoreError> {
        let cursor = before.map(serde_json::from_str).transpose()?;
        let page = self.conversation_storage_page("archived", cursor.as_ref())?;
        Ok(
            json!({"items":page.entries,"nextCursor":page.next.map(|cursor| serde_json::to_string(&cursor)).transpose()?}),
        )
    }
    pub fn remote_messages(
        &self,
        conversation_id: &str,
        before: Option<i64>,
    ) -> Result<Value, CoreError> {
        self.require_remote_conversation(conversation_id)?;
        let conversation = self.get_conversation(conversation_id)?;
        let conn = self.conn();
        // Only project call IDs and names. Raw arguments, results, and private
        // provider envelopes should never enter the historical phone page.
        let mut query = conn.prepare(
            "WITH page AS (
                SELECT id, role, substr(content,1,8000) AS content, length(content) AS total,
                    sort_order, created_at, substr(COALESCE(thinking,''),1,4000) AS thinking,
                    length(COALESCE(thinking,'')) AS thinking_total,
                    (role='user' AND COALESCE(display_artifact_kind,'') IN ('steering','questionResponse')) AS continues_turn,
                    CASE WHEN role='assistant' AND json_valid(tool_calls_json) THEN tool_calls_json ELSE '[]' END AS calls,
                    CASE WHEN role='assistant' AND json_valid(artifacts_json) THEN json_extract(artifacts_json,'$.phase')='commentary' ELSE 0 END AS commentary
                FROM messages
                WHERE conversation_id=?1 AND role IN ('user','assistant')
                  AND (content <> '' OR COALESCE(thinking,'') <> '' OR image_attachments_json IS NOT NULL
                       OR (role='assistant' AND COALESCE(tool_calls_json,'[]') NOT IN ('','[]','null')))
                  AND COALESCE(display_artifact_kind,'') NOT IN ('checkpointContinuation','replayableRuntimeContext')
                  AND (?2 IS NULL OR sort_order<?2)
                ORDER BY sort_order DESC LIMIT 31
             )
             SELECT id,role,content,total,sort_order,created_at,thinking,thinking_total,
                    (COALESCE(commentary,0) OR json_array_length(calls)>0) AS intermediate,
                    (SELECT json_group_array(json_object('id',call_id,'name',name)) FROM (
                        SELECT substr(json_extract(value,'$.id'),1,256) AS call_id,
                               substr(json_extract(value,'$.name'),1,256) AS name
                        FROM json_each(page.calls) WHERE type='object' AND json_type(value,'$.name')='text'
                        LIMIT 32
                    )) AS tool_summaries, continues_turn
             FROM page ORDER BY sort_order DESC",
        )?;
        let mut rows = query
            .query_map(params![conversation_id, before], |row| {
                let calls: String = row.get(9)?;
                let tools: Value = serde_json::from_str(&calls).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        9,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(
                    json!({"id":row.get::<_,String>(0)?, "role":row.get::<_,String>(1)?,
                "content":row.get::<_,String>(2)?, "totalChars":row.get::<_,u64>(3)?,
                "sortOrder":row.get::<_,i64>(4)?, "createdAt":row.get::<_,String>(5)?,
                "thinking":row.get::<_,String>(6)?, "thinkingTotalChars":row.get::<_,u64>(7)?,
                "intermediate":row.get::<_,bool>(8)?, "toolCalls":tools, "continuesTurn":row.get::<_,bool>(10)?}),
                )
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let more = rows.len() > 30;
        rows.truncate(30);
        let cursor = more.then(|| rows.last().unwrap()["sortOrder"].clone());
        rows.reverse();
        // A page can begin inside a long turn. Include the owning prompt ID
        // on that boundary row so a phone can attach live replay without
        // loading an earlier page or guessing from tool names or text.
        if let Some(first) = rows
            .first_mut()
            .filter(|row| more && (row["role"] != "user" || row["continuesTurn"] == true))
        {
            let anchor: Option<String> = conn.query_row(
                "SELECT id FROM messages WHERE conversation_id=?1 AND role='user' AND sort_order<?2
                 AND COALESCE(display_artifact_kind,'') NOT IN ('steering','questionResponse','checkpointContinuation','replayableRuntimeContext')
                 ORDER BY sort_order DESC LIMIT 1",
                params![conversation_id, first["sortOrder"].as_i64()], |row| row.get(0),
            ).optional()?;
            if let Some(id) = anchor {
                first["precedingUserId"] = json!(id);
            }
        }
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
        self.require_remote_conversation(conversation_id)?;
        let conn = self.conn();
        Ok(conn.query_row("SELECT substr(content,?3+1,8000),length(content) FROM messages WHERE conversation_id=?1 AND id=?2 AND role IN ('user','assistant') AND COALESCE(display_artifact_kind,'') NOT IN ('checkpointContinuation','replayableRuntimeContext')",params![conversation_id,message_id,offset],|row|Ok(json!({"content":row.get::<_,String>(0)?,"totalChars":row.get::<_,u64>(1)?,"offset":offset})))?)
    }
    pub fn remote_latest_run(&self, conversation_id: &str) -> Result<Option<String>, CoreError> {
        let conn = self.conn();
        Ok(conn.query_row("SELECT r.id FROM agent_task_runs r JOIN conversations c ON c.id=r.conversation_id WHERE r.conversation_id=?1 AND c.parent_conversation_id IS NULL ORDER BY r.created_at DESC,r.rowid DESC LIMIT 1",[conversation_id],|row|row.get(0)).optional()?)
    }

    pub fn remote_message_thinking(
        &self,
        conversation_id: &str,
        message_id: &str,
        offset: u32,
    ) -> Result<Value, CoreError> {
        self.require_remote_conversation(conversation_id)?;
        Ok(self.conn().query_row("SELECT substr(COALESCE(thinking,''),?3+1,4000),length(COALESCE(thinking,'')) FROM messages WHERE conversation_id=?1 AND id=?2 AND role='assistant' AND COALESCE(display_artifact_kind,'') NOT IN ('checkpointContinuation','replayableRuntimeContext')",params![conversation_id,message_id,offset],|row|Ok(json!({"content":row.get::<_,String>(0)?,"totalChars":row.get::<_,u64>(1)?,"offset":offset})))?)
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
        self.require_remote_conversation(conversation_id)?;
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
                "userMessageId":run.user_message_id,"finalMessageId":final_message_id,"errorMessage":run.error_message},
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
            let mut artifacts = json!({"providerTurnEnvelope":{"secret":"never return"}, "huge":"x".repeat(100_000)});
            let mut tool_calls = Vec::new();
            if index == 3 || index == 31 {
                artifacts["kind"] = json!("steering");
            } else if index == 4 {
                artifacts["kind"] = json!("questionResponse");
            } else if index == 32 {
                artifacts["phase"] = json!("commentary");
            } else if index == 33 {
                tool_calls.push(crate::llm::ToolCallRequest {
                    id: "inspect".into(),
                    name: "read_file".into(),
                    arguments: json!({"path":"report.md"}).to_string(),
                    thought_signature: None,
                });
            }
            db.add_message(&ConversationMessage {
                id: format!("m{index}"),
                conversation_id: chat.id.clone(),
                role: if index == 5 || index >= 32 {
                    Role::Assistant
                } else {
                    Role::User
                },
                content: if index == 34 {
                    "你好🙂".repeat(3000)
                } else if index == 33 {
                    String::new()
                } else {
                    "message".into()
                },
                tool_call_id: None,
                tool_calls,
                artifacts: Some(artifacts),
                token_count: 1,
                created_at: String::new(),
                sort_order: index,
                thinking: None,
                image_attachments: None,
            })
            .unwrap();
        }
        db.add_message(&ConversationMessage {
            id: "resume-control".into(),
            conversation_id: chat.id.clone(),
            role: Role::User,
            content: "Private checkpoint and runtime instructions".into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: Some(json!({"kind":"checkpointContinuation"})),
            token_count: 0,
            created_at: String::new(),
            sort_order: 35,
            thinking: None,
            image_attachments: None,
        })
        .unwrap();
        assert!(db
            .remote_message_text(&chat.id, "resume-control", 0)
            .is_err());
        let page = db.remote_messages(&chat.id, None).unwrap();
        assert!(!page.to_string().contains("Private checkpoint"));
        assert_eq!(page["messages"].as_array().unwrap().len(), 30);
        assert_eq!(page["messages"][0]["sortOrder"], 5);
        assert_eq!(page["messages"][0]["precedingUserId"], "m2");
        assert_eq!(page["messages"][26]["continuesTurn"], true);
        assert_eq!(page["messages"][27]["intermediate"], true);
        assert_eq!(page["messages"][28]["intermediate"], true);
        assert_eq!(
            page["messages"][28]["toolCalls"],
            json!([{"id":"inspect","name":"read_file"}])
        );
        assert!(!page.to_string().contains("report.md"));
        assert_eq!(page["messages"][29]["intermediate"], false);
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
        let child = db
            .delegated_conversation(&chat.id, "worker", "custom", "fixture")
            .unwrap();
        let message = ConversationMessage {
            id: "private-worker-message".into(),
            conversation_id: child.id.clone(),
            role: Role::User,
            content: "Internal worker transcript".into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: None,
            token_count: 4,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        };
        db.add_message(&message).unwrap();
        let turn = db
            .create_conversation_turn(&child.id, &message.id, None)
            .unwrap();
        let run = db
            .create_agent_task_run(&child.id, &turn.id, &message.id, "worker", None, None)
            .unwrap();
        let listing = db.remote_conversations(None).unwrap();
        assert_eq!(listing["items"].as_array().unwrap().len(), 1);
        assert_eq!(listing["items"][0]["id"], chat.id);
        assert!(db.require_remote_conversation(&child.id).is_err());
        assert!(db.remote_messages(&child.id, None).is_err());
        assert!(db.remote_message_text(&child.id, &message.id, 0).is_err());
        assert!(db.remote_latest_run(&child.id).unwrap().is_none());
        assert!(db
            .remote_run_page(&child.id, Some(&run.id), 0, None)
            .is_err());
        assert!(
            db.get_conversation(&child.id).is_ok(),
            "the desktop child workspace remains available"
        );
    }

    #[test]
    fn remote_history_preserves_readable_thinking_without_private_replay() {
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
            id: "thinking-history".into(),
            conversation_id: chat.id.clone(),
            role: Role::Assistant,
            content: "Result".into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: Some(
                json!({"providerTurnEnvelope":{"secret":"opaque replay must stay private"}}),
            ),
            token_count: 1,
            created_at: String::new(),
            sort_order: 0,
            thinking: Some("思考🙂".repeat(3000)),
            image_attachments: None,
        })
        .unwrap();
        let page = db.remote_messages(&chat.id, None).unwrap();
        assert_eq!(page["messages"][0]["thinkingTotalChars"], 9000);
        assert_eq!(
            page["messages"][0]["thinking"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            4000
        );
        assert!(!page.to_string().contains("opaque replay"));
        let tail = db
            .remote_message_thinking(&chat.id, "thinking-history", 8000)
            .unwrap();
        assert_eq!(tail["content"].as_str().unwrap().chars().count(), 1000);
        assert!(db
            .remote_message_thinking("other", "thinking-history", 0)
            .is_err());
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
