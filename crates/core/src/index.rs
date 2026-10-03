//! Indexing module — FTS5 index management utilities.
//!
//! The FTS5 table `fts_chunks` is kept in sync with `chunks` via SQL
//! triggers (see migration v002).  This module provides rebuild,
//! optimize, integrity-check, and statistics helpers.

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::CoreError;

/// Aggregate statistics about the search index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub total_sources: i64,
    pub total_documents: i64,
    pub total_chunks: i64,
    pub fts_rows: i64,
    /// `true` when `total_chunks == fts_rows`.
    pub is_synced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceIndexHealth {
    pub source_id: String,
    pub documents: usize,
    pub chunks: usize,
    pub keyword_chunks: usize,
    pub embedded_chunks: usize,
    pub compiled_documents: usize,
    pub stale_documents: usize,
    pub partial_documents: usize,
    pub parse_warnings: usize,
    pub failed_files: usize,
    pub needs_reparse: usize,
    pub embedding_space: String,
    pub last_scan: Option<crate::ingest::IngestResult>,
}

impl Database {
    pub fn source_index_health(&self) -> Result<Vec<SourceIndexHealth>, CoreError> {
        let config = self.get_embedder_config()?;
        let space = match config.provider.as_str() {
            "tfidf" => "tfidf-v1".into(),
            "api" => crate::embed::ApiEmbedder::configured_space_id(&config),
            _ => config.local_embedding_model().model_name().into(),
        };
        let conn = self.conn();
        let mut statement = conn.prepare(
            "WITH doc_health AS (
              SELECT d.source_id,COUNT(*) documents,
                SUM(CASE WHEN ds.input_revision=d.index_revision AND COALESCE(json_extract(ds.coverage_json,'$.complete'),0)=1 THEN 1 ELSE 0 END) compiled,
                SUM(CASE WHEN ds.id IS NOT NULL AND ds.input_revision!=d.index_revision THEN 1 ELSE 0 END) stale,
                SUM(CASE WHEN ds.input_revision=d.index_revision AND COALESCE(json_extract(ds.coverage_json,'$.complete'),0)=0 THEN 1 ELSE 0 END) partial,
                SUM(CASE WHEN COALESCE(json_extract(d.metadata,'$.parse_warnings'),'')!='' THEN 1 ELSE 0 END) warnings,
                SUM(CASE WHEN COALESCE(json_extract(d.metadata,'$.parser_profile'),'')=''
                  OR (json_extract(d.metadata,'$.parser_profile') LIKE 'native-%' AND json_extract(d.metadata,'$.parser_profile')!=?2)
                  THEN 1 ELSE 0 END) reparse
              FROM documents d LEFT JOIN document_summaries ds ON ds.document_id=d.id GROUP BY d.source_id
            ), chunk_health AS (
              SELECT d.source_id,COUNT(*) chunks,
                SUM(EXISTS(SELECT 1 FROM fts_chunks_docsize f WHERE f.id=c.rowid)) keywords,
                SUM(EXISTS(SELECT 1 FROM embeddings e WHERE e.chunk_id=c.id AND e.model=?1 AND e.dimensions>0 AND length(e.vector)=e.dimensions*4)) embedded
              FROM chunks c JOIN documents d ON d.id=c.document_id GROUP BY d.source_id
            )
            SELECT s.id,COALESCE(d.documents,0),COALESCE(c.chunks,0),COALESCE(c.keywords,0),COALESCE(c.embedded,0),
                   COALESCE(d.compiled,0),COALESCE(d.stale,0),COALESCE(d.partial,0),COALESCE(d.warnings,0),
                   (SELECT COUNT(*) FROM scan_errors se WHERE se.source_id=s.id),COALESCE(d.reparse,0),state.result_json
            FROM sources s LEFT JOIN doc_health d ON d.source_id=s.id LEFT JOIN chunk_health c ON c.source_id=s.id
            LEFT JOIN source_index_state state ON state.source_id=s.id ORDER BY s.created_at,s.id"
        )?;
        let rows = statement
            .query_map(
                rusqlite::params![&space, crate::ingest::NATIVE_PARSER_PROFILE],
                |row| {
                    let scan: Option<String> = row.get(11)?;
                    Ok(SourceIndexHealth {
                        source_id: row.get(0)?,
                        documents: row.get(1)?,
                        chunks: row.get(2)?,
                        keyword_chunks: row.get(3)?,
                        embedded_chunks: row.get(4)?,
                        compiled_documents: row.get(5)?,
                        stale_documents: row.get(6)?,
                        partial_documents: row.get(7)?,
                        parse_warnings: row.get(8)?,
                        failed_files: row.get(9)?,
                        needs_reparse: row.get(10)?,
                        embedding_space: space.clone(),
                        last_scan: scan.and_then(|json| serde_json::from_str(&json).ok()),
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub(crate) fn record_source_scan(
        &self,
        result: &crate::ingest::IngestResult,
    ) -> Result<(), CoreError> {
        self.conn().execute("INSERT INTO source_index_state(source_id,result_json) VALUES(?1,?2) ON CONFLICT(source_id) DO UPDATE SET result_json=excluded.result_json,scanned_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')", rusqlite::params![result.source_id,serde_json::to_string(result)?])?;
        Ok(())
    }
    /// Drop and recreate FTS5 content from the `chunks` table.
    ///
    /// Equivalent to the FTS5 built-in `'rebuild'` command.
    pub fn rebuild_fts_index(&self) -> Result<(), CoreError> {
        let conn = self.conn();
        conn.execute("INSERT INTO fts_chunks(fts_chunks) VALUES('rebuild')", [])?;
        Ok(())
    }

    /// Collect aggregate counts for sources, documents, chunks, and FTS rows.
    pub fn get_index_stats(&self) -> Result<IndexStats, CoreError> {
        let conn = self.conn();

        let total_sources: i64 =
            conn.query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0))?;
        let total_documents: i64 =
            conn.query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))?;
        let total_chunks: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        // External-content FTS COUNT(*) reads the content view even if postings
        // are missing. The docsize table counts actual indexed rows.
        let fts_rows: i64 =
            conn.query_row("SELECT COUNT(*) FROM fts_chunks_docsize", [], |r| r.get(0))?;

        Ok(IndexStats {
            total_sources,
            total_documents,
            total_chunks,
            fts_rows,
            is_synced: total_chunks == fts_rows,
        })
    }

    /// Run the FTS5 `'optimize'` command to merge internal b-tree segments.
    pub fn optimize_fts_index(&self) -> Result<(), CoreError> {
        let conn = self.conn();
        conn.execute("INSERT INTO fts_chunks(fts_chunks) VALUES('optimize')", [])?;
        Ok(())
    }

    /// Run FTS5 integrity-check.
    ///
    /// Returns `true` when the index is consistent with the content
    /// table, `false` otherwise.
    // TODO: integrate — FTS5 integrity diagnostic, not yet exposed as Tauri command
    pub fn integrity_check(&self) -> Result<bool, CoreError> {
        let conn = self.conn();
        let result = conn.execute(
            "INSERT INTO fts_chunks(fts_chunks, rank) VALUES('integrity-check', 1)",
            [],
        );
        match result {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::SqliteFailure(_, _)) => Ok(false),
            Err(e) => Err(CoreError::Database(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers (mirror the ones in db.rs tests) ────────────────────────

    fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn setup() -> Database {
        Database::open_memory().expect("open in-memory db")
    }

    fn seed_data(db: &Database, chunk_count: usize) {
        let conn = db.conn();
        let src_id = new_id();
        conn.execute(
            "INSERT INTO sources (id, kind, root_path) VALUES (?1, 'local_folder', '/tmp/src')",
            rusqlite::params![&src_id],
        )
        .unwrap();

        let doc_id = new_id();
        conn.execute(
            "INSERT INTO documents (id, source_id, path, title, mime_type, file_size, modified_at, content_hash)
             VALUES (?1, ?2, '/tmp/doc.md', 'Doc', 'text/plain', 100, datetime('now'), 'hash')",
            rusqlite::params![&doc_id, &src_id],
        )
        .unwrap();

        for i in 0..chunk_count {
            let cid = new_id();
            conn.execute(
                "INSERT INTO chunks (id, document_id, chunk_index, kind, content, start_offset, end_offset, line_start, line_end, content_hash)
                 VALUES (?1, ?2, ?3, 'text', ?4, 0, 100, 1, 10, 'chash')",
                rusqlite::params![&cid, &doc_id, i as i64, format!("chunk content {i}")],
            )
            .unwrap();
        }
    }

    // ── tests ───────────────────────────────────────────────────────────

    #[test]
    fn stats_empty_db() {
        let db = setup();
        let stats = db.get_index_stats().unwrap();

        assert_eq!(stats.total_sources, 0);
        assert_eq!(stats.total_documents, 0);
        assert_eq!(stats.total_chunks, 0);
        assert_eq!(stats.fts_rows, 0);
        assert!(stats.is_synced);
    }

    #[test]
    fn stats_after_ingest() {
        let db = setup();
        seed_data(&db, 5);

        let stats = db.get_index_stats().unwrap();
        assert_eq!(stats.total_sources, 1);
        assert_eq!(stats.total_documents, 1);
        assert_eq!(stats.total_chunks, 5);
        assert_eq!(stats.fts_rows, 5);
        assert!(stats.is_synced);
    }

    #[test]
    fn rebuild_fts_index_succeeds() {
        let db = setup();
        seed_data(&db, 3);

        db.rebuild_fts_index().unwrap();

        let stats = db.get_index_stats().unwrap();
        assert_eq!(stats.total_chunks, 3);
        assert_eq!(stats.fts_rows, 3);
        assert!(stats.is_synced);
    }

    #[test]
    fn optimize_fts_index_succeeds() {
        let db = setup();
        seed_data(&db, 4);

        db.optimize_fts_index().unwrap();

        // Optimize is a no-error operation; verify counts unchanged.
        let stats = db.get_index_stats().unwrap();
        assert_eq!(stats.total_chunks, 4);
        assert!(stats.is_synced);
    }

    #[test]
    fn integrity_check_passes() {
        let db = setup();
        seed_data(&db, 2);

        assert!(db.integrity_check().unwrap());
    }
}
