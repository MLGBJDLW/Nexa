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

    fn score(&self) -> f32 {
        match self {
            Self::Upvote => 1.0,
            Self::Downvote => -1.0,
            Self::Pin => 2.0,
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

    /// Compute aggregate feedback scores per chunk for a given query.
    ///
    /// Returns `chunk_id → score` where upvote = +1, downvote = −1, pin = +2.
    pub fn get_feedback_scores(
        &self,
        query_text: &str,
    ) -> Result<HashMap<String, f32>, CoreError> {
        let feedbacks = self.get_feedback_for_query(query_text)?;
        let mut scores: HashMap<String, f32> = HashMap::new();
        for fb in feedbacks {
            *scores.entry(fb.chunk_id).or_insert(0.0) += fb.action.score();
        }
        Ok(scores)
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

/// Apply feedback-based score boosts to search results, then re-sort descending.
///
/// For each `(chunk_id, score)` pair, if the chunk has a feedback score the
/// result score is adjusted by `feedback_score * boost_weight`.
pub fn apply_feedback_boost(
    results: &mut Vec<(String, f32)>,
    feedback_scores: &HashMap<String, f32>,
    boost_weight: f32,
) {
    for (chunk_id, score) in results.iter_mut() {
        if let Some(&fb_score) = feedback_scores.get(chunk_id.as_str()) {
            *score += fb_score * boost_weight;
        }
    }
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
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

    #[test]
    fn test_feedback_scores() {
        let (db, chunk_id) = setup_db_with_chunk();

        db.add_feedback(&chunk_id, "q", FeedbackAction::Upvote).unwrap();
        db.add_feedback(&chunk_id, "q", FeedbackAction::Pin).unwrap();
        db.add_feedback(&chunk_id, "q", FeedbackAction::Downvote).unwrap();

        let scores = db.get_feedback_scores("q").unwrap();
        // +1.0 + 2.0 + (-1.0) = 2.0
        assert_eq!(scores.get(&chunk_id), Some(&2.0));
    }

    #[test]
    fn test_multiple_chunks_aggregate() {
        let (db, chunk_id_a) = setup_db_with_chunk();

        // Create a second chunk in the same document.
        let chunk_id_b = uuid::Uuid::new_v4().to_string();
        {
            let conn = db.conn();
            let doc_id: String = conn
                .query_row("SELECT document_id FROM chunks LIMIT 1", [], |r| r.get(0))
                .unwrap();
            conn.execute(
                "INSERT INTO chunks (id, document_id, chunk_index, kind, content, start_offset, end_offset, line_start, line_end, content_hash)
                 VALUES (?1, ?2, 1, 'text', 'other content', 12, 25, 2, 2, 'chunkhash2')",
                params![&chunk_id_b, &doc_id],
            )
            .unwrap();
        }

        db.add_feedback(&chunk_id_a, "q", FeedbackAction::Upvote).unwrap();
        db.add_feedback(&chunk_id_a, "q", FeedbackAction::Upvote).unwrap();
        db.add_feedback(&chunk_id_b, "q", FeedbackAction::Downvote).unwrap();

        let scores = db.get_feedback_scores("q").unwrap();
        assert_eq!(scores.get(&chunk_id_a), Some(&2.0));
        assert_eq!(scores.get(&chunk_id_b), Some(&-1.0));
    }

    #[test]
    fn test_apply_feedback_boost() {
        let mut results = vec![
            ("chunk-a".to_string(), 0.5),
            ("chunk-b".to_string(), 0.8),
            ("chunk-c".to_string(), 0.3),
        ];
        let mut feedback_scores = HashMap::new();
        feedback_scores.insert("chunk-a".to_string(), 2.0);  // upvote+pin
        feedback_scores.insert("chunk-c".to_string(), -1.0); // downvote

        apply_feedback_boost(&mut results, &feedback_scores, 0.1);

        // chunk-a: 0.5 + 2.0*0.1 = 0.7
        // chunk-b: 0.8 (no feedback)
        // chunk-c: 0.3 + (-1.0)*0.1 = 0.2
        // Sorted DESC: chunk-b(0.8), chunk-a(0.7), chunk-c(0.2)
        assert_eq!(results[0].0, "chunk-b");
        assert!((results[0].1 - 0.8).abs() < 1e-6);
        assert_eq!(results[1].0, "chunk-a");
        assert!((results[1].1 - 0.7).abs() < 1e-6);
        assert_eq!(results[2].0, "chunk-c");
        assert!((results[2].1 - 0.2).abs() < 1e-6);
    }
}
