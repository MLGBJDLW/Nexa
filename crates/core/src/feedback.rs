/// User feedback module — 👍/👎/pin on evidence cards.

use std::collections::HashMap;
use std::fmt;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::CoreError;

// ── Types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feedback {
    pub id: String,
    pub chunk_id: String,
    pub query_text: String,
    pub action: FeedbackAction,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackAction {
    Upvote,
    Downvote,
    Pin,
}

impl fmt::Display for FeedbackAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Upvote => write!(f, "upvote"),
            Self::Downvote => write!(f, "downvote"),
            Self::Pin => write!(f, "pin"),
        }
    }
}

impl FeedbackAction {
    fn from_str(s: &str) -> Result<Self, CoreError> {
        match s {
            "upvote" => Ok(Self::Upvote),
            "downvote" => Ok(Self::Downvote),
            "pin" => Ok(Self::Pin),
            other => Err(CoreError::InvalidInput(format!(
                "Invalid feedback action: {other}"
            ))),
        }
    }

    /// Per-event search boost adjustment.
    ///
    /// Upvote → +0.15, Downvote → −0.15, Pin → +0.25.
    pub(crate) fn boost(&self) -> f64 {
        match self {
            Self::Upvote => 0.15,
            Self::Downvote => -0.15,
            Self::Pin => 0.25,
        }
    }
}

// ── Database methods ─────────────────────────────────────────────────

impl Database {
    /// Record a feedback action on a chunk for a given query.
    pub fn add_feedback(
        &self,
        chunk_id: &str,
        query_text: &str,
        action: FeedbackAction,
    ) -> Result<Feedback, CoreError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let action_str = action.to_string();

        let conn = self.conn();
        conn.execute(
            "INSERT INTO feedback (id, chunk_id, query_text, action, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&id, chunk_id, query_text, &action_str, &now],
        )?;

        Ok(Feedback {
            id,
            chunk_id: chunk_id.to_string(),
            query_text: query_text.to_string(),
            action,
            created_at: now,
        })
    }

    /// Get all feedback entries for a specific query text.
    pub fn get_feedback_for_query(
        &self,
        query_text: &str,
    ) -> Result<Vec<Feedback>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, chunk_id, query_text, action, created_at
             FROM feedback
             WHERE query_text = ?1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![query_text], row_to_feedback)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Get all feedback entries for a specific chunk.
    // TODO: integrate — per-chunk feedback query, not yet exposed in UI
    pub fn get_feedback_for_chunk(
        &self,
        chunk_id: &str,
    ) -> Result<Vec<Feedback>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, chunk_id, query_text, action, created_at
             FROM feedback
             WHERE chunk_id = ?1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![chunk_id], row_to_feedback)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Delete a feedback entry by id.
    pub fn delete_feedback(&self, feedback_id: &str) -> Result<(), CoreError> {
        let conn = self.conn();
        let affected = conn.execute(
            "DELETE FROM feedback WHERE id = ?1",
            params![feedback_id],
        )?;
        if affected == 0 {
            return Err(CoreError::NotFound(format!(
                "Feedback not found: {feedback_id}"
            )));
        }
        Ok(())
    }

    /// Compute per-chunk feedback adjustments for search re-ranking.
    ///
    /// Each upvote adds +0.15, each downvote −0.15, each pin +0.25.
    /// The total per-chunk adjustment is clamped to \[−0.5, +0.5\].
    /// Only chunks present in `chunk_ids` are included.
    pub fn get_feedback_adjustments(
        &self,
        query_text: &str,
        chunk_ids: &[String],
    ) -> Result<HashMap<String, f64>, CoreError> {
        let feedbacks = self.get_feedback_for_query(query_text)?;
        let id_set: std::collections::HashSet<&str> =
            chunk_ids.iter().map(|s| s.as_str()).collect();
        let mut adjustments: HashMap<String, f64> = HashMap::new();
        for fb in feedbacks {
            if id_set.contains(fb.chunk_id.as_str()) {
                *adjustments.entry(fb.chunk_id).or_insert(0.0) += fb.action.boost();
            }
        }
        for val in adjustments.values_mut() {
            *val = val.clamp(-0.5, 0.5);
        }
        Ok(adjustments)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn row_to_feedback(row: &rusqlite::Row<'_>) -> rusqlite::Result<Feedback> {
    let action_str: String = row.get(3)?;
    let action = FeedbackAction::from_str(&action_str)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e)))?;
    Ok(Feedback {
        id: row.get(0)?,
        chunk_id: row.get(1)?,
        query_text: row.get(2)?,
        action,
        created_at: row.get(4)?,
    })
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    /// Set up an in-memory DB with a minimal chunk for FK satisfaction.
    fn setup_db_with_chunk() -> (Database, String) {
        let db = Database::open_memory().expect("open_memory");

        let source_id = uuid::Uuid::new_v4().to_string();
        let doc_id = uuid::Uuid::new_v4().to_string();
        let chunk_id = uuid::Uuid::new_v4().to_string();

        let conn = db.conn();
        conn.execute(
            "INSERT INTO sources (id, kind, root_path, include_globs, exclude_globs, watch_enabled)
             VALUES (?1, 'local_folder', '/tmp/test', '[]', '[]', 0)",
            params![&source_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents (id, source_id, path, mime_type, file_size, modified_at, content_hash)
             VALUES (?1, ?2, '/tmp/test/a.md', 'text/plain', 100, datetime('now'), 'hash')",
            params![&doc_id, &source_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks (id, document_id, chunk_index, kind, content, start_offset, end_offset, line_start, line_end, content_hash)
             VALUES (?1, ?2, 0, 'text', 'test content', 0, 12, 1, 1, 'chunkhash')",
            params![&chunk_id, &doc_id],
        )
        .unwrap();
        drop(conn);

        (db, chunk_id)
    }

    #[test]
    fn test_add_and_get_feedback() {
        let (db, chunk_id) = setup_db_with_chunk();

        let fb = db
            .add_feedback(&chunk_id, "test query", FeedbackAction::Upvote)
            .expect("add_feedback");
        assert_eq!(fb.chunk_id, chunk_id);
        assert_eq!(fb.action, FeedbackAction::Upvote);

        let by_query = db.get_feedback_for_query("test query").unwrap();
        assert_eq!(by_query.len(), 1);
        assert_eq!(by_query[0].id, fb.id);

        let by_chunk = db.get_feedback_for_chunk(&chunk_id).unwrap();
        assert_eq!(by_chunk.len(), 1);
        assert_eq!(by_chunk[0].id, fb.id);
    }

    #[test]
    fn test_delete_feedback() {
        let (db, chunk_id) = setup_db_with_chunk();

        let fb = db
            .add_feedback(&chunk_id, "q", FeedbackAction::Pin)
            .unwrap();
        db.delete_feedback(&fb.id).unwrap();

        let remaining = db.get_feedback_for_chunk(&chunk_id).unwrap();
        assert!(remaining.is_empty());

        let err = db.delete_feedback("nonexistent");
        assert!(err.is_err());
    }

}
