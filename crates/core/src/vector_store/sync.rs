use super::{
    adapter::{RemoteStore, VectorRecord},
    config::{embedding_space, VectorSearchMode},
};
use crate::{db::Database, error::CoreError};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

static SYNC_LOCK: Mutex<()> = Mutex::new(());
static SYNCING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();
pub fn notify_sync() {
    WAKE.notify_one();
}
pub fn cancel_sync() {
    CANCEL.store(true, Ordering::SeqCst);
}
pub fn resume_sync() {
    CANCEL.store(false, Ordering::SeqCst);
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VectorSyncStatus {
    pub local_vectors: usize,
    pub uploaded_vectors: usize,
    pub pending_uploads: usize,
    pub pending_deletes: usize,
    pub syncing: bool,
    pub paused: bool,
    pub last_error: Option<String>,
}
#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VectorSyncReport {
    pub uploaded: usize,
    pub deleted: usize,
    pub busy: bool,
}

const PENDING_JOIN:&str="FROM embeddings e JOIN chunks c ON c.id=e.chunk_id JOIN documents d ON d.id=c.document_id LEFT JOIN vector_store_receipts r ON r.store_id=?1 AND r.space_id=e.model AND r.chunk_id=e.chunk_id WHERE e.model=?2 AND (r.chunk_id IS NULL OR r.embedding_id!=e.id OR r.revision!=e.revision OR r.source_id!=d.source_id)";
const DELETED_JOIN:&str="FROM vector_store_receipts r LEFT JOIN embeddings e ON e.model=r.space_id AND e.chunk_id=r.chunk_id WHERE r.store_id=?1 AND e.id IS NULL";

impl Database {
    pub fn vector_sync_status(&self) -> Result<VectorSyncStatus, CoreError> {
        let config = self.vector_store_config()?;
        let embedding = self.get_embedder_config()?;
        let model_space = embedding_space(&embedding);
        let model_error = model_space.as_ref().err().map(ToString::to_string);
        let space = model_space.map(|(space, _)| space).unwrap_or_default();
        let store = config.store_id();
        let conn = self.conn();
        let local_vectors: usize = conn.query_row(
            "SELECT COUNT(*) FROM embeddings WHERE model=?1",
            [&space],
            |r| r.get(0),
        )?;
        let pending_uploads = conn.query_row(
            &format!("SELECT COUNT(*) {PENDING_JOIN}"),
            rusqlite::params![store, space],
            |r| r.get(0),
        )?;
        let pending_deletes =
            conn.query_row(&format!("SELECT COUNT(*) {DELETED_JOIN}"), [&store], |r| {
                r.get(0)
            })?;
        use rusqlite::OptionalExtension;
        let last_error = conn
            .query_row(
                "SELECT last_error FROM vector_store_config WHERE singleton=1",
                [],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        Ok(VectorSyncStatus {
            local_vectors,
            uploaded_vectors: local_vectors.saturating_sub(pending_uploads),
            pending_uploads,
            pending_deletes,
            syncing: SYNCING.load(Ordering::SeqCst),
            paused: CANCEL.load(Ordering::SeqCst),
            last_error: last_error.or_else(|| {
                (config.mode != VectorSearchMode::Local)
                    .then_some(model_error)
                    .flatten()
            }),
        })
    }
    fn pending_vectors(
        &self,
        store: &str,
        space: &str,
        limit: usize,
    ) -> Result<Vec<VectorRecord>, CoreError> {
        let conn = self.conn();
        let mut statement=conn.prepare(&format!("SELECT e.chunk_id,d.source_id,e.id,e.revision,e.vector {PENDING_JOIN} ORDER BY e.rowid LIMIT ?3"))?;
        let rows = statement
            .query_map(rusqlite::params![store, space, limit as i64], |r| {
                let blob: Vec<u8> = r.get(4)?;
                Ok(VectorRecord {
                    chunk_id: r.get(0)?,
                    source_id: r.get(1)?,
                    embedding_id: r.get(2)?,
                    revision: r.get(3)?,
                    vector: crate::embed::blob_to_vector(&blob),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    fn pending_vector_deletes(&self, store: &str) -> Result<Vec<(String, String)>, CoreError> {
        let conn = self.conn();
        let mut statement = conn.prepare(&format!(
            "SELECT r.space_id,r.chunk_id {DELETED_JOIN} ORDER BY r.space_id LIMIT 64"
        ))?;
        let rows = statement
            .query_map([store], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    fn acknowledge_vectors(
        &self,
        store: &str,
        space: &str,
        records: &[VectorRecord],
    ) -> Result<(), CoreError> {
        let mut conn = self.conn();
        let transaction = conn.transaction()?;
        for r in records {
            transaction.execute("INSERT INTO vector_store_receipts(store_id,space_id,chunk_id,source_id,embedding_id,revision) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(store_id,space_id,chunk_id) DO UPDATE SET source_id=excluded.source_id,embedding_id=excluded.embedding_id,revision=excluded.revision",rusqlite::params![store,space,r.chunk_id,r.source_id,r.embedding_id,r.revision])?;
        }
        transaction.commit()?;
        Ok(())
    }
}

/// SQLite stays authoritative. HTTP never holds its connection guard, and
/// acknowledgements record the exact uploaded revision, not a later edit.
pub fn sync_vectors(db: &Database, max_batches: usize) -> Result<VectorSyncReport, CoreError> {
    if CANCEL.load(Ordering::SeqCst) {
        return Ok(VectorSyncReport::default());
    }
    let Ok(_guard) = SYNC_LOCK.try_lock() else {
        return Ok(VectorSyncReport {
            busy: true,
            ..Default::default()
        });
    };
    struct Running;
    impl Drop for Running {
        fn drop(&mut self) {
            SYNCING.store(false, Ordering::SeqCst);
        }
    }
    SYNCING.store(true, Ordering::SeqCst);
    let _running = Running;
    let config = db.vector_store_config()?;
    if config.mode == VectorSearchMode::Local {
        return Ok(VectorSyncReport::default());
    }
    let current_space = embedding_space(&db.get_embedder_config()?).ok();
    let (space, dimensions) = current_space.clone().unwrap_or_else(|| (String::new(), 2));
    let store = config.store_id();
    let result = (|| {
        let owner = db.vector_store_owner()?;
        let remote = current_space
            .as_ref()
            .map(|_| {
                RemoteStore::new(
                    &config,
                    &owner,
                    &space,
                    dimensions,
                    std::time::Duration::from_secs(20),
                )
            })
            .transpose()?;
        let mut report = VectorSyncReport::default();
        let mut initialized = false;
        for _ in 0..max_batches {
            let latest = db.vector_store_config()?;
            if CANCEL.load(Ordering::SeqCst)
                || latest.mode == VectorSearchMode::Local
                || latest.store_id() != store
                || latest.api_key != config.api_key
                || embedding_space(&db.get_embedder_config()?).ok() != current_space
            {
                break;
            }
            let deletes = db.pending_vector_deletes(&store)?;
            let batch_size = (1_500_000 / (dimensions.saturating_mul(16) + 512)).clamp(1, 32);
            let records = if current_space.is_some() {
                db.pending_vectors(&store, &space, batch_size)?
            } else {
                Vec::new()
            };
            if deletes.is_empty() && records.is_empty() {
                break;
            }
            if !deletes.is_empty() {
                let mut groups = std::collections::BTreeMap::<String, Vec<String>>::new();
                for (old_space, id) in deletes {
                    groups.entry(old_space).or_default().push(id);
                }
                for (old_space, ids) in groups {
                    let old = RemoteStore::new(
                        &config,
                        &db.vector_store_owner()?,
                        &old_space,
                        2,
                        std::time::Duration::from_secs(20),
                    )?;
                    old.delete(&ids)?;
                    let mut conn = db.conn();
                    let tx = conn.transaction()?;
                    for id in &ids {
                        tx.execute("DELETE FROM vector_store_receipts WHERE store_id=?1 AND space_id=?2 AND chunk_id=?3",rusqlite::params![store,old_space,id])?;
                    }
                    tx.commit()?;
                    report.deleted += ids.len();
                }
            }
            if !records.is_empty() && !initialized {
                remote
                    .as_ref()
                    .expect("upload space exists")
                    .ensure_collection()?;
                initialized = true;
            }
            if !records.is_empty() {
                remote
                    .as_ref()
                    .expect("upload space exists")
                    .upsert(&records)?;
                db.acknowledge_vectors(&store, &space, &records)?;
                report.uploaded += records.len();
            }
        }
        Ok(report)
    })();
    let error = result.as_ref().err().map(ToString::to_string);
    // Do not attach an old endpoint's error to newly selected settings.
    let latest = db.vector_store_config()?;
    if latest.store_id() == store && latest.api_key == config.api_key {
        db.conn().execute(
            "UPDATE vector_store_config SET last_error=?1 WHERE singleton=1",
            [error],
        )?;
    }
    result
}

/// One bounded worker, no overlapping ticks. Disabled/local mode never contacts
/// a cloud. Deletion receipts survive offline periods and process restarts.
pub async fn start_background_sync(db: Database) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! { _=interval.tick()=>{}, _=WAKE.notified()=>{} }
        let db = db.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let report = sync_vectors(&db, 8)?;
            let more = (report.uploaded > 0 || report.deleted > 0)
                && db
                    .vector_sync_status()
                    .is_ok_and(|status| status.pending_uploads > 0 || status.pending_deletes > 0);
            Ok::<_, CoreError>(more)
        })
        .await;
        if matches!(outcome, Ok(Ok(true))) {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            notify_sync();
        }
    }
}
