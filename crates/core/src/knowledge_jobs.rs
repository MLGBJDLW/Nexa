//! Durable indexing activity. Only the runtime owning a job may finish it;
//! opening a page or a database read lane never changes its lifecycle.
use crate::{db::Database, error::CoreError};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeJob {
    pub id: String,
    pub kind: String,
    pub source_id: Option<String>,
    pub status: String,
    pub progress: Value,
    pub error: Option<String>,
    pub revision: u64,
    pub started_at: String,
    pub finished_at: Option<String>,
}

fn read_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeJob> {
    Ok(KnowledgeJob {
        id: row.get(0)?,
        kind: row.get(1)?,
        source_id: row.get(2)?,
        status: row.get(3)?,
        progress: serde_json::from_str(&row.get::<_, String>(4)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        error: row.get(5)?,
        revision: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
    })
}

impl Database {
    pub fn start_knowledge_job(
        &self,
        kind: &str,
        source_id: Option<&str>,
    ) -> Result<KnowledgeJob, CoreError> {
        let scoped = matches!(kind, "scan" | "embed");
        if !matches!(
            kind,
            "scan" | "embed" | "scan-all" | "rebuild-embeddings" | "compile" | "research"
        ) || scoped != source_id.is_some()
        {
            return Err(CoreError::InvalidInput(
                "Invalid knowledge job kind or source scope".into(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.conn().execute(
            "INSERT INTO knowledge_jobs(id,kind,source_id,status) VALUES(?1,?2,?3,'running')",
            params![id, kind, source_id],
        )?;
        self.knowledge_job(&id)?
            .ok_or_else(|| CoreError::NotFound(id))
    }

    pub fn knowledge_job(&self, id: &str) -> Result<Option<KnowledgeJob>, CoreError> {
        Ok(self.conn().query_row("SELECT id,kind,source_id,status,progress_json,error,revision,started_at,finished_at FROM knowledge_jobs WHERE id=?1", [id], read_job).optional()?)
    }

    pub fn list_knowledge_jobs(&self) -> Result<Vec<KnowledgeJob>, CoreError> {
        let conn = self.conn();
        let mut statement = conn.prepare("SELECT id,kind,source_id,status,progress_json,error,revision,started_at,finished_at FROM knowledge_jobs ORDER BY (status='running') DESC, started_at DESC LIMIT 100")?;
        let jobs = statement
            .query_map([], read_job)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(jobs)
    }

    pub fn update_knowledge_job(
        &self,
        id: &str,
        progress: &Value,
    ) -> Result<Option<KnowledgeJob>, CoreError> {
        self.conn().execute("UPDATE knowledge_jobs SET progress_json=?2,revision=revision+1 WHERE id=?1 AND status='running'", params![id, serde_json::to_string(progress)?])?;
        self.knowledge_job(id)
    }

    pub fn finish_knowledge_job(
        &self,
        id: &str,
        error: Option<&str>,
    ) -> Result<Option<KnowledgeJob>, CoreError> {
        self.conn().execute("UPDATE knowledge_jobs SET status=?2,error=?3,revision=revision+1,finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND status='running'", params![id, if error.is_some() { "failed" } else { "completed" }, error])?;
        self.knowledge_job(id)
    }

    /// Call once during host startup, before accepting new work, never on page mount.
    pub fn recover_knowledge_jobs(&self) -> Result<usize, CoreError> {
        Ok(self.conn().execute("UPDATE knowledge_jobs SET status='interrupted',error='runtime_interrupted',revision=revision+1,finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE status='running'", [])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_preserve_other_jobs_and_terminal_states() {
        let db = Database::open_memory().unwrap();
        let first = db.start_knowledge_job("scan-all", None).unwrap();
        let second = db.start_knowledge_job("rebuild-embeddings", None).unwrap();
        let progress = db
            .update_knowledge_job(&first.id, &serde_json::json!({"current": 3, "total": 9}))
            .unwrap()
            .unwrap();
        assert!(progress.revision > first.revision);
        db.finish_knowledge_job(&first.id, None).unwrap();
        db.update_knowledge_job(&first.id, &serde_json::json!({"current": 1}))
            .unwrap();
        assert_eq!(
            db.knowledge_job(&first.id).unwrap().unwrap().progress["current"],
            3
        );
        assert_eq!(
            db.knowledge_job(&second.id).unwrap().unwrap().status,
            "running"
        );
        assert_eq!(db.recover_knowledge_jobs().unwrap(), 1);
        assert_eq!(
            db.knowledge_job(&second.id).unwrap().unwrap().status,
            "interrupted"
        );
        assert_eq!(
            db.knowledge_job(&first.id).unwrap().unwrap().status,
            "completed"
        );
        assert_eq!(db.recover_knowledge_jobs().unwrap(), 0);
        assert!(db.start_knowledge_job("scan", None).is_err());
        assert!(db
            .start_knowledge_job("embed", Some("missing-source"))
            .is_err());
    }
    #[test]
    fn failed_terminal_persistence_remains_retryable() {
        let db = Database::open_memory().unwrap();
        let job = db.start_knowledge_job("research", None).unwrap();
        db.conn().execute_batch("CREATE TRIGGER reject_finish BEFORE UPDATE OF status ON knowledge_jobs BEGIN SELECT RAISE(ABORT,'injected storage failure'); END;").unwrap();
        assert!(db.finish_knowledge_job(&job.id, None).is_err());
        assert_eq!(
            db.knowledge_job(&job.id).unwrap().unwrap().status,
            "running"
        );
        db.conn()
            .execute_batch("DROP TRIGGER reject_finish")
            .unwrap();
        assert_eq!(
            db.finish_knowledge_job(&job.id, None)
                .unwrap()
                .unwrap()
                .status,
            "completed"
        );
    }
}
