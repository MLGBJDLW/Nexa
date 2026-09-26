//! Optional cloud mirrors. Local chunks, scope, revisions and vectors remain
//! authoritative; adapters never receive document text or provider credentials.
mod adapter;
mod config;
mod sync;
#[cfg(test)]
mod tests;
use crate::{db::Database, error::CoreError, models::SearchFilters};
pub(crate) type RankedHits = Vec<(String, f32)>;

pub(crate) fn ranked_candidates(
    db: &Database,
    space: &str,
    vector: &[f32],
    filters: &SearchFilters,
    limit: usize,
) -> Result<(RankedHits, &'static str), CoreError> {
    let local =
        || crate::search::vector_search_top_k_scoped(db, vector, space, limit, None, filters);
    let mode = match db.vector_store_config() {
        Ok(config) => config.mode,
        Err(error) => {
            tracing::warn!(
                "Cloud vector configuration unavailable; using local retrieval: {error}"
            );
            return Ok((local()?, "hybrid+local-fallback"));
        }
    };
    match mode {
        VectorSearchMode::Local => Ok((local()?, "hybrid")),
        VectorSearchMode::Cloud => match cloud_candidates(db, space, vector, filters, limit) {
            Ok(hits) if !hits.is_empty() => Ok((hits, "hybrid+cloud")),
            _ => Ok((local()?, "hybrid+local-fallback")),
        },
        VectorSearchMode::Hybrid => {
            let local = local()?;
            match cloud_candidates(db, space, vector, filters, limit) {
                Ok(cloud) if !cloud.is_empty() => Ok((
                    fuse_dense_candidates(&local, &cloud, limit),
                    "hybrid+fusion",
                )),
                _ => Ok((local, "hybrid+local-fallback")),
            }
        }
    }
}
pub use config::{VectorSearchMode, VectorStoreConfig, VectorStoreProvider};
pub use sync::{
    cancel_sync, notify_sync, resume_sync, start_background_sync, sync_vectors, VectorSyncReport,
    VectorSyncStatus,
};

pub fn test_connection(db: &Database, config: &VectorStoreConfig) -> Result<(), CoreError> {
    let (space, dimensions) = config::embedding_space(&db.get_embedder_config()?)?;
    let remote = adapter::RemoteStore::new(
        config,
        "connection-test",
        &space,
        dimensions,
        std::time::Duration::from_secs(10),
    )?;
    remote.probe()?;
    Ok(())
}

/// Cloud responses are candidate IDs only. Every hit is revalidated against the
/// local source filter and exact current embedding revision before hydration.
pub(crate) fn cloud_candidates(
    db: &Database,
    space: &str,
    vector: &[f32],
    filters: &SearchFilters,
    limit: usize,
) -> Result<Vec<(String, f32)>, CoreError> {
    if limit > 200 {
        return Err(CoreError::InvalidInput(
            "Large result pages use local vector retrieval".into(),
        ));
    }
    if !filters.file_types.is_empty() || filters.date_from.is_some() || filters.date_to.is_some() {
        return Err(CoreError::InvalidInput(
            "File/date filters use authoritative local vector retrieval".into(),
        ));
    }
    let config = db.vector_store_config()?;
    if config.mode == VectorSearchMode::Local {
        return Err(CoreError::Cancelled("Cloud retrieval is disabled".into()));
    }
    let sources = if filters.source_ids.is_empty() {
        db.list_sources()?.into_iter().map(|s| s.id).collect()
    } else {
        filters
            .source_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    let remote = adapter::RemoteStore::new(
        &config,
        &db.vector_store_owner()?,
        space,
        vector.len(),
        std::time::Duration::from_millis(1500),
    )?;
    let guard_db = db.clone();
    let expected = config.clone();
    let remote = remote.with_request_guard(std::sync::Arc::new(move || {
        let latest = guard_db.vector_store_config()?;
        if latest.mode == VectorSearchMode::Local
            || latest.store_id() != expected.store_id()
            || latest.api_key != expected.api_key
        {
            return Err(CoreError::Cancelled(
                "Cloud retrieval settings changed".into(),
            ));
        }
        Ok(())
    }));
    let hits = remote.query(vector, &sources, limit)?;
    let ids = hits
        .iter()
        .map(|hit| hit.chunk_id.clone())
        .collect::<Vec<_>>();
    let allowed = crate::search::scoped_chunk_ids(db, &ids, filters)?;
    let conn = db.conn();
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for hit in hits {
        if !allowed.contains(&hit.chunk_id) || !seen.insert(hit.chunk_id.clone()) {
            continue;
        }
        use rusqlite::OptionalExtension;
        let current:Option<(String,String)>=conn.query_row("SELECT e.id || ':' || e.revision,d.source_id FROM embeddings e JOIN chunks c ON c.id=e.chunk_id JOIN documents d ON d.id=c.document_id WHERE e.model=?1 AND e.chunk_id=?2",rusqlite::params![space,hit.chunk_id],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        if current
            .as_ref()
            .is_some_and(|(version, source)| version == &hit.version && source == &hit.source_id)
        {
            let score = 1.0 / (60.0 + result.len() as f32);
            result.push((hit.chunk_id, score));
        }
    }
    Ok(result)
}

/// Local/cloud copies are one semantic signal, not two independent votes.
pub(crate) fn fuse_dense_candidates(
    local: &[(String, f32)],
    cloud: &[(String, f32)],
    limit: usize,
) -> Vec<(String, f32)> {
    let mut scores = std::collections::HashMap::<String, f32>::new();
    for list in [local, cloud] {
        for (rank, (id, _)) in list.iter().enumerate() {
            let score = 1.0 / (60.0 + rank as f32);
            scores
                .entry(id.clone())
                .and_modify(|v| *v = v.max(score))
                .or_insert(score);
        }
    }
    let mut values = scores.into_iter().collect::<Vec<_>>();
    values.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    values.truncate(limit);
    values
}
