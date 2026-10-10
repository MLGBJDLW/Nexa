//! Bounded, cancellable embedding jobs.
//!
//! This module owns source scoping, bounded batches, cancellation checkpoints,
//! progress, and immediate per-batch persistence for embedding work.

use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::info;

mod estimate;
pub use estimate::EmbeddingEstimate;
use estimate::EmbeddingEstimator;

use crate::db::Database;
use crate::embed::{create_embedder_with_limits, Embedder, EmbeddingRuntimeLimits, TfIdfEmbedder};
use crate::error::CoreError;

/// Summary of an embedding run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedResult {
    pub source_id: String,
    pub chunks_embedded: usize,
    pub chunks_skipped: usize,
    pub model: String,
}

/// Progress information emitted during scanning or embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub source_id: String,
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub current_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<EmbeddingEstimate>,
}

/// Stable resource policy for one embedding job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingJobLimits {
    pub batch_size: usize,
    pub max_intra_threads: usize,
}

impl Default for EmbeddingJobLimits {
    fn default() -> Self {
        Self {
            batch_size: 8,
            max_intra_threads: 2,
        }
    }
}

/// Control plane supplied by the caller of an embedding job.
pub trait EmbeddingJobControl {
    fn checkpoint(&self) -> Result<(), CoreError> {
        Ok(())
    }

    fn on_progress(&self, _progress: ScanProgress) {}
}

#[derive(Debug, Default)]
pub struct NoopEmbeddingJobControl;

impl EmbeddingJobControl for NoopEmbeddingJobControl {}

struct CallbackControl<'a> {
    callback: &'a dyn Fn(ScanProgress),
}

impl EmbeddingJobControl for CallbackControl<'_> {
    fn on_progress(&self, progress: ScanProgress) {
        (self.callback)(progress);
    }
}

pub fn embed_source(db: &Database, source_id: &str) -> Result<EmbedResult, CoreError> {
    run_source(
        db,
        source_id,
        EmbeddingJobLimits::default(),
        &NoopEmbeddingJobControl,
    )
}

pub fn embed_source_with_progress(
    db: &Database,
    source_id: &str,
    on_progress: impl Fn(ScanProgress),
) -> Result<EmbedResult, CoreError> {
    run_source(
        db,
        source_id,
        EmbeddingJobLimits::default(),
        &CallbackControl {
            callback: &on_progress,
        },
    )
}

pub fn run_source(
    db: &Database,
    source_id: &str,
    limits: EmbeddingJobLimits,
    control: &dyn EmbeddingJobControl,
) -> Result<EmbedResult, CoreError> {
    control.checkpoint()?;
    let config = db.get_embedder_config()?;
    let batch_size = limits.batch_size.max(1);

    let (model, embedder): (String, Box<dyn Embedder>) = if config.provider == "tfidf" {
        let model = "tfidf-v1".to_string();
        let embedder = load_or_build_tfidf(db, &model, control)?;
        (model, Box::new(embedder))
    } else {
        let embedder = create_embedder_with_limits(
            &config,
            EmbeddingRuntimeLimits {
                max_intra_threads: limits.max_intra_threads,
            },
        )?;
        (embedder.vector_space_id().to_string(), embedder)
    };

    let total_source_chunks = db.count_chunks_for_source(source_id)?;
    let embedded = run_batches(db, Some(source_id), &model, &*embedder, batch_size, control)?;

    let skipped = total_source_chunks.saturating_sub(embedded);
    info!(
        "Embedding complete for source {}: embedded={}, skipped={}, provider={}",
        source_id, embedded, skipped, config.provider
    );

    Ok(EmbedResult {
        source_id: source_id.to_string(),
        chunks_embedded: embedded,
        chunks_skipped: skipped,
        model,
    })
}

pub fn rebuild_embeddings(db: &Database) -> Result<EmbedResult, CoreError> {
    rebuild_all(db, EmbeddingJobLimits::default(), &NoopEmbeddingJobControl)
}

pub fn rebuild_embeddings_with_progress(
    db: &Database,
    on_progress: impl Fn(ScanProgress),
) -> Result<EmbedResult, CoreError> {
    rebuild_all(
        db,
        EmbeddingJobLimits::default(),
        &CallbackControl {
            callback: &on_progress,
        },
    )
}

pub fn rebuild_all(
    db: &Database,
    limits: EmbeddingJobLimits,
    control: &dyn EmbeddingJobControl,
) -> Result<EmbedResult, CoreError> {
    control.checkpoint()?;
    let config = db.get_embedder_config()?;
    let batch_size = limits.batch_size.max(1);

    let (model, embedder): (String, Box<dyn Embedder>) = if config.provider == "tfidf" {
        let model = "tfidf-v1".to_string();
        let all_chunks = db.get_all_chunks()?;
        control.checkpoint()?;
        let corpus: Vec<&str> = all_chunks
            .iter()
            .map(|(_, content)| content.as_str())
            .collect();
        let embedder = TfIdfEmbedder::build_from_corpus(&corpus);
        db.save_embedder_state(&model, &embedder.vocabulary, &embedder.idf)?;
        drop(all_chunks);
        (model, Box::new(embedder))
    } else {
        let embedder = create_embedder_with_limits(
            &config,
            EmbeddingRuntimeLimits {
                max_intra_threads: limits.max_intra_threads,
            },
        )?;
        (embedder.vector_space_id().to_string(), embedder)
    };

    let deleted = db.delete_all_embeddings(&model)?;
    info!("Deleted {} existing embeddings", deleted);

    let embedded = run_batches(db, None, &model, &*embedder, batch_size, control)?;

    info!(
        "Rebuild complete: {} chunks embedded (provider={})",
        embedded, config.provider
    );
    Ok(EmbedResult {
        source_id: "all".to_string(),
        chunks_embedded: embedded,
        chunks_skipped: 0,
        model,
    })
}

fn load_or_build_tfidf(
    db: &Database,
    model: &str,
    control: &dyn EmbeddingJobControl,
) -> Result<TfIdfEmbedder, CoreError> {
    if let Some((vocab, idf)) = db.load_embedder_state(model)? {
        info!("Loaded existing embedder state for model '{}'", model);
        return Ok(TfIdfEmbedder::from_vocabulary(vocab, idf));
    }

    control.checkpoint()?;
    info!("No saved embedder state; building TF-IDF from full corpus");
    let all_chunks = db.get_all_chunks()?;
    let corpus: Vec<&str> = all_chunks
        .iter()
        .map(|(_, content)| content.as_str())
        .collect();
    let embedder = TfIdfEmbedder::build_from_corpus(&corpus);
    db.save_embedder_state(model, &embedder.vocabulary, &embedder.idf)?;
    Ok(embedder)
}

fn persist_embedded_batch(
    db: &Database,
    model: &str,
    embedder: &dyn Embedder,
    chunks: &[(String, String)],
    control: &dyn EmbeddingJobControl,
) -> Result<std::time::Duration, CoreError> {
    let texts: Vec<&str> = chunks.iter().map(|(_, content)| content.as_str()).collect();
    let started = Instant::now();
    let vectors = embedder.embed_documents(&texts)?;
    let inference_time = started.elapsed();
    if vectors.len() != chunks.len() {
        return Err(CoreError::Embedding(format!(
            "embedding batch cardinality mismatch: expected {}, got {}",
            chunks.len(),
            vectors.len()
        )));
    }
    control.checkpoint()?;
    let batch: Vec<(String, String, Vec<f32>)> = chunks
        .iter()
        .zip(vectors)
        .map(|((chunk_id, _), vector)| (chunk_id.clone(), model.to_string(), vector))
        .collect();
    db.batch_store_embeddings(&batch)?;
    Ok(inference_time)
}

fn run_batches(
    db: &Database,
    source_id: Option<&str>,
    model: &str,
    embedder: &dyn Embedder,
    batch_size: usize,
    control: &dyn EmbeddingJobControl,
) -> Result<usize, CoreError> {
    let (mut total, chars) = db.missing_embedding_work(source_id, model)?;
    let mut estimator = EmbeddingEstimator::new(db, model, embedder.model_name(), chars)?;
    let source = source_id.unwrap_or("all");
    let mut current = 0;
    let mut cursor = String::new();
    control.on_progress(progress(source, current, total, estimator.snapshot(false)));
    loop {
        control.checkpoint()?;
        let mut chunks = db.missing_embedding_page(source_id, model, &cursor, batch_size)?;
        if chunks.is_empty() && !cursor.is_empty() {
            // A watcher may have replaced a document with IDs below the cursor.
            // Recheck only after traversing the index; never silently miss it.
            cursor.clear();
            let (pending, chars) = db.missing_embedding_work(source_id, model)?;
            total = current + pending;
            estimator.refresh_remaining(chars);
            chunks = db.missing_embedding_page(source_id, model, &cursor, batch_size)?;
        }
        if chunks.is_empty() {
            break;
        }
        let inference_time = persist_embedded_batch(db, model, embedder, &chunks, control)?;
        estimator.observe(
            db,
            chunks.iter().map(|(_, text)| text.chars().count()).sum(),
            chunks.len(),
            inference_time,
        );
        current += chunks.len();
        cursor = chunks.last().expect("nonempty batch").0.clone();
        control.on_progress(progress(
            source,
            current,
            total.max(current),
            estimator.snapshot(false),
        ));
    }
    control.on_progress(progress(
        source,
        current,
        total.max(current),
        estimator.snapshot(true),
    ));
    Ok(current)
}

fn progress(
    source_id: &str,
    current: usize,
    total: usize,
    embedding: EmbeddingEstimate,
) -> ScanProgress {
    ScanProgress {
        source_id: source_id.to_string(),
        phase: "embedding".to_string(),
        current,
        total,
        current_file: None,
        embedding: Some(embedding),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn chunk(db: &Database, id: &str, document: &str, index: usize) {
        db.conn().execute("INSERT INTO chunks(id,document_id,chunk_index,content,start_offset,end_offset,line_start,line_end,content_hash) VALUES (?1,?2,?3,'test content',0,12,1,1,?1)", rusqlite::params![id,document,index]).unwrap();
    }

    struct EditingControl<'a> {
        db: &'a Database,
        inserted: Cell<bool>,
    }
    impl EmbeddingJobControl for EditingControl<'_> {
        fn on_progress(&self, progress: ScanProgress) {
            if progress.current == 1 && !self.inserted.replace(true) {
                chunk(self.db, "a-new-behind-cursor", "document", 2);
            }
        }
    }

    #[test]
    fn keyset_job_catches_concurrent_edits_and_respects_source_and_model() {
        let db = Database::open_memory().unwrap();
        db.conn().execute_batch("INSERT INTO sources(id,root_path) VALUES ('source','/source'),('other','/other');
            INSERT INTO documents(id,source_id,path,modified_at,content_hash) VALUES ('document','source','one','now','hash'),('other-document','other','two','now','hash');").unwrap();
        chunk(&db, "m-first", "document", 0);
        chunk(&db, "z-last", "document", 1);
        chunk(&db, "n-other", "other-document", 0);
        db.store_embedding("m-first", "other-model", &[1.0])
            .unwrap();
        let control = EditingControl {
            db: &db,
            inserted: Cell::new(false),
        };
        let embedder = TfIdfEmbedder::build_from_corpus(&["test content"]);
        assert_eq!(
            run_batches(&db, Some("source"), "current-model", &embedder, 1, &control).unwrap(),
            3
        );
        assert_eq!(
            db.missing_embedding_work(Some("source"), "current-model")
                .unwrap()
                .0,
            0
        );
        assert!(db
            .get_embedding("a-new-behind-cursor", "current-model")
            .unwrap()
            .is_some());
        assert!(db
            .get_embedding("n-other", "current-model")
            .unwrap()
            .is_none());
        assert_eq!(
            db.get_embedding("m-first", "other-model").unwrap(),
            Some(vec![1.0])
        );
    }
}
