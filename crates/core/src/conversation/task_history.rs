//! Bounded, payload-free projection for the Task Center history panel.

use rusqlite::params;
use serde::Serialize;

use crate::{db::Database, error::CoreError};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskHistoryItem {
    pub id: String,
    pub run_id: String,
    pub event_type: String,
    pub label: String,
    pub status: Option<String>,
    pub created_at: String,
    pub event_seq: Option<u64>,
    pub source: &'static str,
}

impl Database {
    /// Read at most 50 items per independent timeline. Filter visibility before
    /// limiting, and never deserialize tool outputs, snapshots, or trace payloads.
    /// The renderer merges these with the independently bounded scheduler feed.
    pub fn get_agent_task_history(
        &self,
        run_id: &str,
        include_developer: bool,
    ) -> Result<Vec<AgentTaskHistoryItem>, CoreError> {
        let conn = self.conn();
        let mut canonical = conn.prepare(
            "SELECT run_id || ':' || event_seq, run_id, kind, label,
                    COALESCE(status, CASE kind WHEN 'done' THEN 'completed' WHEN 'error' THEN 'failed' END),
                    created_at, event_seq
             FROM agent_run_events
             WHERE run_id = ?1
               AND kind NOT IN ('outputDelta', 'outputSnapshot', 'thinking', 'usageUpdated')
               AND visibility != 'internal' AND (?2 OR visibility != 'developer')
             ORDER BY julianday(created_at) DESC, event_seq DESC LIMIT 50",
        )?;
        let mut items = canonical
            .query_map(params![run_id, include_developer], |row| {
                history_from_row(row, "agentRun")
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut timeline = conn.prepare(
            "SELECT id, run_id, event_type, label, status, created_at,
                    CAST(json_extract(payload_json, '$.eventSeq') AS INTEGER)
             FROM agent_task_run_events
             WHERE run_id = ?1
               AND json_extract(payload_json, '$.taskTimeline.kind') IN ('subtask', 'verification')
               AND COALESCE(json_extract(payload_json, '$.taskTimeline.visibility'),
                   CASE json_extract(payload_json, '$.taskTimeline.kind')
                   WHEN 'verification' THEN 'developer' ELSE 'user' END) != 'internal'
               AND (?2 OR COALESCE(json_extract(payload_json, '$.taskTimeline.visibility'),
                   CASE json_extract(payload_json, '$.taskTimeline.kind')
                   WHEN 'verification' THEN 'developer' ELSE 'user' END) != 'developer')
             ORDER BY julianday(created_at) DESC,
                      CAST(json_extract(payload_json, '$.eventSeq') AS INTEGER) DESC, id DESC LIMIT 50",
        )?;
        items.extend(
            timeline
                .query_map(params![run_id, include_developer], |row| {
                    history_from_row(row, "taskEvent")
                })?
                .collect::<Result<Vec<_>, _>>()?,
        );
        Ok(items)
    }
}

fn history_from_row(
    row: &rusqlite::Row<'_>,
    source: &'static str,
) -> rusqlite::Result<AgentTaskHistoryItem> {
    Ok(AgentTaskHistoryItem {
        id: row.get(0)?,
        run_id: row.get(1)?,
        event_type: row.get(2)?,
        label: row.get(3)?,
        status: row.get(4)?,
        created_at: row.get(5)?,
        event_seq: row
            .get::<_, Option<i64>>(6)?
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_history_filters_before_limit_and_does_not_materialize_large_payloads() {
        let db = Database::open_memory().unwrap();
        // Canonical events do not require a task FK. A large invisible suffix
        // must not crowd the last visible user events out of the bounded result.
        {
            let mut conn = db.conn();
            let tx = conn.transaction().unwrap();
            for seq in 1..=2_000 {
                let (kind, visibility) = if seq <= 100 {
                    ("status", "user")
                } else if seq <= 200 {
                    ("status", "developer")
                } else if seq <= 300 {
                    ("status", "internal")
                } else {
                    ("outputDelta", "user")
                };
                tx.execute("INSERT INTO agent_run_events (run_id, turn_id, event_seq, version, kind, phase, visibility, label, payload_json)
                    VALUES ('history-run', 'turn', ?1, 2, ?2, 'responding', ?3, ?4, ?5)",
                    params![seq, kind, visibility, format!("History {seq}"), serde_json::json!({
                        "content": "x".repeat(4_096), "blockId": "history", "channel": "answer", "offset": 0, "delta": "x",
                    }).to_string()]).unwrap();
            }
            tx.commit().unwrap();
        }
        let history = db.get_agent_task_history("history-run", false).unwrap();
        assert_eq!(history.len(), 50);
        assert_eq!(history.first().unwrap().event_seq, Some(100));
        assert_eq!(history.last().unwrap().event_seq, Some(51));
        let history_bytes = serde_json::to_vec(&history).unwrap().len();
        let ledger_bytes = serde_json::to_vec(&db.list_agent_run_events("history-run").unwrap())
            .unwrap()
            .len();
        assert!(history_bytes < 16_000);
        assert!(ledger_bytes > 8_000_000);
        println!("Task history: ledger={ledger_bytes} bytes / 2000 rows, projection={history_bytes} bytes / 50 rows");
        let developer = db.get_agent_task_history("history-run", true).unwrap();
        assert_eq!(developer.first().unwrap().event_seq, Some(200));
        assert!(developer
            .iter()
            .all(|item| (151..=200).contains(&item.event_seq.unwrap())));
        assert!(db
            .get_agent_task_history("missing", true)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn task_history_limits_after_timeline_visibility_and_ignores_non_timeline_rows() {
        let db = Database::open_memory().unwrap();
        {
            let conn = db.conn();
            // This fixture exercises projection only; the task lifecycle itself
            // is covered by conversation persistence tests.
            conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
            for seq in 1..=300 {
                let timeline = if seq <= 60 {
                    serde_json::json!({"kind":"subtask"})
                } else if seq <= 120 {
                    serde_json::json!({"kind":"verification"})
                } else if seq <= 180 {
                    serde_json::json!({"kind":"subtask","visibility":"internal"})
                } else {
                    serde_json::Value::Null
                };
                conn.execute("INSERT INTO agent_task_run_events (id, run_id, event_type, label, payload_json)
                    VALUES (?1, 'timeline-run', 'status', ?2, ?3)", params![format!("event-{seq:03}"), format!("Timeline {seq}"),
                        serde_json::json!({"eventSeq":seq,"taskTimeline":timeline}).to_string()]).unwrap();
            }
            conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        }
        let user = db.get_agent_task_history("timeline-run", false).unwrap();
        assert_eq!(user.len(), 50);
        assert_eq!(user.first().unwrap().event_seq, Some(60));
        assert_eq!(user.last().unwrap().event_seq, Some(11));
        assert!(user.iter().all(|item| item.source == "taskEvent"));
        let developer = db.get_agent_task_history("timeline-run", true).unwrap();
        assert_eq!(developer.len(), 50);
        assert_eq!(developer.first().unwrap().event_seq, Some(120));
        assert_eq!(developer.last().unwrap().event_seq, Some(71));
    }
}
