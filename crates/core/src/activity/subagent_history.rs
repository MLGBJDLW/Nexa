//! The UI reads the privacy-projected durable journal, never a worker's raw
//! in-memory buffers. Ownership, cursor and policy revision share one DB lock.
use super::{ActivityEvent, ActivityRecord};
use crate::{db::Database, error::CoreError};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentHistoryPage {
    pub record: ActivityRecord,
    pub events: Vec<ActivityEvent>,
    pub cursor: u64,
    pub has_more: bool,
    pub history_truncated: bool,
    pub privacy_revision: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{
        ActivityEventKind, ActivityRuntime, ActivitySpec, ActivityState, ActivitySurface,
    };
    use crate::conversation::CreateConversationInput;

    fn conversation(db: &Database) -> String {
        db.create_conversation(&CreateConversationInput {
            provider: "open_ai".into(),
            model: "test".into(),
            system_prompt: None,
            collection_context: None,
            project_id: None,
            persona_id: None,
        })
        .unwrap()
        .id
    }

    #[test]
    fn full_subagent_history_survives_ring_eviction_and_reload_but_other_chats_cannot_read_it() {
        let db = Database::open_memory().unwrap();
        let owner = conversation(&db);
        let other = conversation(&db);
        let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
        runtime
            .start(
                ActivitySpec::new(ActivitySurface::Process, "spawn_subagent")
                    .with_activity_id("worker")
                    .with_conversation_id(&owner),
            )
            .unwrap();
        for index in 0..2200 {
            runtime
                .append(
                    "worker",
                    ActivityEventKind::Progress,
                    serde_json::json!({ "index": index }),
                )
                .unwrap();
        }
        runtime
            .transition(
                "worker",
                ActivityState::Completed,
                serde_json::json!({ "done": true }),
            )
            .unwrap();
        drop(runtime);
        let recovered = ActivityRuntime::with_database(db.clone()).unwrap();
        assert_eq!(
            recovered.get("worker").unwrap().state,
            ActivityState::Completed
        );
        let mut cursor = 0;
        let mut count = 0;
        loop {
            let page = db.read_subagent_history(&owner, "worker", cursor).unwrap();
            assert!(!page.history_truncated);
            assert!(page.events.len() <= 256);
            for event in &page.events {
                assert_eq!(event.seq, cursor + 1);
                cursor = event.seq;
                count += 1;
            }
            if !page.has_more {
                break;
            }
        }
        assert_eq!(count, 2202);
        assert!(db.read_subagent_history(&other, "worker", 0).is_err());
        db.conn()
            .execute(
                "DELETE FROM activity_events WHERE activity_id='worker' AND seq<20",
                [],
            )
            .unwrap();
        assert!(
            db.read_subagent_history(&owner, "worker", 0)
                .unwrap()
                .history_truncated
        );
    }

    #[test]
    fn subagent_history_uses_current_privacy_projection_and_keeps_protocol_identity() {
        let db = Database::open_memory().unwrap();
        let owner = conversation(&db);
        let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
        runtime
            .start(
                ActivitySpec::new(ActivitySurface::Process, "spawn_subagent")
                    .with_activity_id("privateCODE")
                    .with_conversation_id(&owner),
            )
            .unwrap();
        let payload = serde_json::json!({ "subagentEvent": "stream", "agentId": "privateCODE", "detail": {
            "event": { "type": "streamBlockSnapshot", "blockId": "privateCODE", "channel": "answer", "text": "value privateCODE" }
        }});
        runtime
            .append("privateCODE", ActivityEventKind::Progress, payload.clone())
            .unwrap();
        let before = db
            .read_subagent_history(&owner, "privateCODE", 0)
            .unwrap()
            .privacy_revision;
        db.save_privacy_config(&crate::privacy::PrivacyConfig {
            enabled: true,
            redact_patterns: vec![crate::privacy::RedactRule {
                name: "test".into(),
                pattern: "privateCODE".into(),
                replacement: "[PRIVATE]".into(),
            }],
            ..Default::default()
        })
        .unwrap();
        // A late writer with the old raw buffer is also projected by storage.
        runtime
            .append("privateCODE", ActivityEventKind::Progress, payload)
            .unwrap();
        let page = db.read_subagent_history(&owner, "privateCODE", 0).unwrap();
        assert_ne!(page.privacy_revision, before);
        for event in page.events.iter().skip(1) {
            assert_eq!(event.activity_id, "privateCODE");
            assert_eq!(event.payload["detail"]["event"]["blockId"], "privateCODE");
            assert_eq!(event.payload["detail"]["event"]["text"], "value [PRIVATE]");
        }
    }

    #[test]
    fn deleting_parent_removes_full_history_and_late_writes_cannot_restore_it() {
        let db = Database::open_memory().unwrap();
        let owner = conversation(&db);
        let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
        runtime
            .start(
                ActivitySpec::new(ActivitySurface::Process, "spawn_subagent")
                    .with_activity_id("deleted-worker")
                    .with_conversation_id(&owner),
            )
            .unwrap();
        runtime
            .append(
                "deleted-worker",
                ActivityEventKind::Progress,
                serde_json::json!({ "content": "history" }),
            )
            .unwrap();
        db.delete_conversation(&owner).unwrap();
        assert!(db
            .read_subagent_history(&owner, "deleted-worker", 0)
            .is_err());
        assert!(runtime
            .append(
                "deleted-worker",
                ActivityEventKind::Progress,
                serde_json::json!({ "content": "late" })
            )
            .is_err());
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM activity_events WHERE activity_id='deleted-worker'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}

impl Database {
    pub fn read_subagent_history(
        &self,
        conversation_id: &str,
        agent_id: &str,
        after_seq: u64,
    ) -> Result<SubagentHistoryPage, CoreError> {
        let conn = self.conn();
        let raw: Option<String> = conn.query_row(
            "SELECT record_json FROM activity_records WHERE activity_id=?1 AND conversation_id=?2
             AND EXISTS(SELECT 1 FROM conversations WHERE id=?2)",
            params![agent_id, conversation_id], |row| row.get(0),
        ).optional()?;
        let record: ActivityRecord = serde_json::from_str(
            &raw.ok_or_else(|| CoreError::NotFound("Subagent history".into()))?,
        )?;
        if record.owner_tool != "spawn_subagent"
            || record.activity_id != agent_id
            || record.conversation_id.as_deref() != Some(conversation_id)
        {
            return Err(CoreError::NotFound("Subagent history".into()));
        }
        let privacy_revision = conn.query_row(
            "SELECT revision FROM privacy_chat_state WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let earliest: Option<u64> = conn.query_row(
            "SELECT MIN(seq) FROM activity_events WHERE activity_id=?1",
            [agent_id],
            |row| row.get(0),
        )?;
        let mut statement = conn.prepare("SELECT event_json FROM activity_events WHERE activity_id=?1 AND seq>?2 AND seq<=?3 ORDER BY seq LIMIT 257")?;
        let rows = statement.query_map(
            params![
                agent_id,
                after_seq.min(i64::MAX as u64) as i64,
                record.last_event_seq as i64
            ],
            |row| row.get::<_, String>(0),
        )?;
        let mut events = Vec::new();
        let mut bytes = 0usize;
        let mut cursor = after_seq;
        let mut has_more = false;
        let mut history_truncated = earliest.is_some_and(|seq| seq > after_seq.saturating_add(1));
        for row in rows {
            let raw = row?;
            // Always allow one event so even a large tool result makes progress.
            if !events.is_empty() && (events.len() >= 256 || bytes + raw.len() > 512 * 1024) {
                has_more = true;
                break;
            }
            let event: ActivityEvent = serde_json::from_str(&raw)?;
            if event.activity_id != agent_id || event.seq <= cursor {
                return Err(CoreError::Internal(
                    "Invalid subagent journal identity or sequence".into(),
                ));
            }
            history_truncated |= event.seq != cursor.saturating_add(1)
                || (matches!(
                    event.payload["subagentEvent"].as_str(),
                    Some("thinkingDelta" | "outputDelta")
                ) && event.payload["detail"]["delta"].is_null());
            cursor = event.seq;
            bytes += raw.len();
            events.push(event);
        }
        history_truncated |= !has_more && cursor < record.last_event_seq;
        Ok(SubagentHistoryPage {
            record,
            events,
            cursor,
            has_more,
            history_truncated,
            privacy_revision,
        })
    }
}
