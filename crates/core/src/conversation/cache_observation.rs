//! Bounded recovery of cache telemetry without hydrating conversation contents.

use crate::{db::Database, error::CoreError};

impl Database {
    pub(crate) fn recent_prompt_cache_observations(
        &self,
        conversation_id: &str,
        exclude_turn_id: Option<&str>,
    ) -> Result<Vec<(String, serde_json::Value)>, CoreError> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "WITH recent AS MATERIALIZED (
                SELECT id, trace_json, created_at, rowid AS ordinal
                FROM conversation_turns
                WHERE conversation_id = ?1 AND (?2 IS NULL OR id <> ?2)
                    AND finished_at IS NOT NULL AND trace_json IS NOT NULL
                ORDER BY created_at DESC, rowid DESC LIMIT 8
             )
             SELECT recent.id, json_extract(item.value, '$.observation')
             FROM recent, json_each(
                CASE WHEN json_valid(recent.trace_json)
                    THEN json_extract(recent.trace_json, '$.items') ELSE '[]' END
             ) AS item
             WHERE CASE WHEN item.type = 'object'
                THEN json_extract(item.value, '$.kind') END = 'promptCache'
             ORDER BY recent.created_at DESC, recent.ordinal DESC, CAST(item.key AS INTEGER) DESC
             LIMIT 16",
        )?;
        let rows = statement
            .query_map(rusqlite::params![conversation_id, exclude_turn_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
        let mut observations = Vec::new();
        for row in rows {
            let (turn, json) = row?;
            if let Some(observation) = json.and_then(|json| serde_json::from_str(&json).ok()) {
                observations.push((turn, observation));
            }
        }
        Ok(observations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{ConversationMessage, CreateConversationInput};
    use crate::llm::Role;

    #[test]
    fn cache_seed_query_preserves_unsigned_hashes_and_excludes_incomplete_turns() {
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
        let mut ids = Vec::new();
        for index in 0..12 {
            let message = ConversationMessage {
                id: format!("user-{index}"),
                conversation_id: chat.id.clone(),
                role: Role::User,
                content: "fixture".into(),
                tool_call_id: None,
                tool_calls: vec![],
                artifacts: None,
                token_count: 1,
                created_at: String::new(),
                sort_order: index,
                thinking: None,
                image_attachments: None,
            };
            db.add_message(&message).unwrap();
            let turn = db
                .create_conversation_turn(&chat.id, &message.id, None)
                .unwrap();
            let trace = serde_json::json!({ "items": ["non-object legacy entry", { "kind": "promptCache", "observation": { "index": index, "hash": u64::MAX } }], "unrelated": "x".repeat(64_000) });
            db.conn()
                .execute(
                    "UPDATE conversation_turns SET trace_json=?1, finished_at=?2 WHERE id=?3",
                    rusqlite::params![
                        trace.to_string(),
                        if index < 11 { Some("2026-09-10") } else { None },
                        turn.id
                    ],
                )
                .unwrap();
            ids.push(turn.id);
        }
        let observations = db
            .recent_prompt_cache_observations(&chat.id, Some(&ids[10]))
            .unwrap();
        assert_eq!(observations.len(), 8);
        assert_eq!(observations[0].0, ids[9]);
        assert_eq!(observations[0].1["hash"].as_u64(), Some(u64::MAX));
        assert_eq!(observations[0].1["index"], 9);
        assert!(!observations[0].1.to_string().contains("unrelated"));
    }
}
