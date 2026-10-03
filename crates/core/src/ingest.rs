//! Ingestion module — discovers and imports files from sources.
//!
//! Walks a source's directory tree, applies include/exclude glob filters,
//! parses matching files, and upserts documents + chunks into the database.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::db::Database;
pub use crate::embedding_job::{EmbedResult, ScanProgress};
use crate::error::CoreError;
#[cfg(feature = "video")]
use crate::parse::hash_file_content;
#[cfg(test)]
use crate::parse::parse_file;
use crate::parse::{
    detect_mime_type, file_appears_binary, parse_file_with_media_config, ParsedChunk,
    ParsedDocument,
};
use crate::privacy::{self, PrivacyConfig};

pub const NATIVE_PARSER_PROFILE: &str = "native-v3";

#[derive(Debug, Clone)]
pub struct IndexedDocument {
    pub id: String,
    pub content_hash: String,
    pub parser_profile: String,
    pub parsed_hash: String,
    pub redaction_profile: String,
}

fn parsed_hash(parsed: &ParsedDocument) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(parsed.title.as_bytes());
    hasher.update(
        parsed
            .metadata
            .get("redaction_profile")
            .map(String::as_str)
            .unwrap_or_default()
            .as_bytes(),
    );
    for chunk in &parsed.chunks {
        hasher.update(chunk.content.as_bytes());
        hasher.update(&[0]);
        hasher.update(
            serde_json::to_string(&chunk.locator)
                .unwrap_or_default()
                .as_bytes(),
        );
        hasher.update(chunk.extraction_method.as_bytes());
        hasher.update(
            chunk
                .heading_context
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
    }
    for artifact in &parsed.visual_artifacts {
        hasher.update(artifact.to_chunk_content().as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn parsed_metadata_json(parsed: &ParsedDocument) -> Result<String, CoreError> {
    let mut metadata = parsed.metadata.clone();
    metadata.insert("parsed_hash".into(), parsed_hash(parsed));
    metadata
        .entry("parser_profile".into())
        .or_insert_with(|| NATIVE_PARSER_PROFILE.into());
    Ok(serde_json::to_string(&metadata)?)
}

fn parser_profile(parsed: &ParsedDocument) -> &str {
    parsed
        .metadata
        .get("parser_profile")
        .map(String::as_str)
        .unwrap_or(NATIVE_PARSER_PROFILE)
}

fn apply_privacy(
    parsed: &mut ParsedDocument,
    config: &PrivacyConfig,
    stored_fingerprint: &str,
) -> Result<(), CoreError> {
    privacy::validate_config(config)?;
    parsed.metadata.insert(
        "privacy_config_fingerprint".into(),
        stored_fingerprint.into(),
    );
    parsed.metadata.insert(
        "redaction_profile".into(),
        privacy::redaction_fingerprint(config)?,
    );
    parsed
        .metadata
        .insert("redaction_enabled".into(), config.enabled.to_string());
    if config.enabled {
        parsed.title = privacy::redact_content(&parsed.title, &config.redact_patterns);
        for chunk in &mut parsed.chunks {
            let content = privacy::redact_content(&chunk.content, &config.redact_patterns);
            if content != chunk.content {
                // Original byte counts cannot trim an altered redacted prefix.
                chunk.overlap_start = 0;
                chunk.content = content;
            }
            if let Some(heading) = &mut chunk.heading_context {
                *heading = privacy::redact_content(heading, &config.redact_patterns);
            }
        }
        crate::visual_document::redact_visual_artifacts(&mut parsed.visual_artifacts, |value| {
            privacy::redact_content(value, &config.redact_patterns)
        });
    }
    Ok(())
}

fn validate_privacy_at_commit(
    conn: &rusqlite::Connection,
    parsed: &ParsedDocument,
) -> Result<(), CoreError> {
    if let Some(expected) = parsed.metadata.get("privacy_config_fingerprint") {
        if *expected != privacy::config_fingerprint(&privacy::load_config_on(conn)?)? {
            return Err(CoreError::Conflict(
                "Privacy settings changed while scanning; retry with the current rules".into(),
            ));
        }
    }
    Ok(())
}

fn revoke_changed_redaction_history(
    conn: &rusqlite::Connection,
    doc_id: &str,
    parsed: &ParsedDocument,
) -> Result<(), CoreError> {
    if parsed
        .metadata
        .get("redaction_enabled")
        .is_some_and(|value| value == "true")
    {
        if let Some(profile) = parsed.metadata.get("redaction_profile") {
            // Also cover explicit scan_source_with_privacy overrides, which need
            // not change the globally saved configuration.
            conn.execute("DELETE FROM evidence_snapshots WHERE document_id=?1 AND COALESCE(json_extract(document_metadata,'$.redaction_profile'),'')!=?2", params![doc_id, profile])?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// File size limits
// ---------------------------------------------------------------------------

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024; // 100 MB (text/docs)
const MAX_VIDEO_FILE_SIZE: u64 = 2 * 1024 * 1024 * 1024; // 2 GB
const MAX_AUDIO_FILE_SIZE: u64 = 500 * 1024 * 1024; // 500 MB
const CODE_SOURCE_EXTENSIONS: &[&str] = &[
    "astro", "bash", "bat", "c", "cc", "cjs", "clj", "cljs", "cmd", "cpp", "cs", "css", "cxx",
    "dart", "erl", "ex", "exs", "fish", "fs", "fsx", "go", "gql", "graphql", "h", "hpp", "hrl",
    "hxx", "java", "js", "jsx", "kt", "kts", "less", "lua", "mjs", "ml", "mli", "nim", "php",
    "proto", "ps1", "py", "r", "rb", "rs", "sass", "scala", "scss", "sh", "sol", "sql", "svelte",
    "swift", "ts", "tsx", "vue", "zig", "zsh",
];
const CODE_SOURCE_FILENAMES: &[&str] = &[
    "build.gradle",
    "cmakelists.txt",
    "dockerfile",
    "gemfile",
    "justfile",
    "makefile",
    "podfile",
    "rakefile",
    "settings.gradle",
];
const GENERATED_BINARY_EXTENSIONS: &[&str] = &[
    "class", "dll", "dylib", "exe", "o", "obj", "pdb", "pyc", "pyo", "so", "wasm",
];

/// Configurable file-size limits for ingestion.
#[derive(Debug, Clone, Copy)]
pub struct FileSizeLimits {
    pub max_text: u64,
    pub max_video: u64,
    pub max_audio: u64,
}

impl Default for FileSizeLimits {
    fn default() -> Self {
        Self {
            max_text: MAX_FILE_SIZE,
            max_video: MAX_VIDEO_FILE_SIZE,
            max_audio: MAX_AUDIO_FILE_SIZE,
        }
    }
}

/// Returns the appropriate file-size limit based on file extension.
fn max_file_size_for_path(path: &Path, limits: &FileSizeLimits) -> u64 {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp4" | "mkv" | "webm" | "avi" | "mov" | "flv" | "mpeg" | "mpg" | "wmv" | "m4v") => {
            limits.max_video
        }
        Some("mp3" | "wav" | "flac" | "aac" | "ogg" | "wma" | "m4a" | "opus") => limits.max_audio,
        _ => limits.max_text,
    }
}

fn is_code_source_file(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase());
    if let Some(file_name) = file_name.as_deref() {
        if CODE_SOURCE_FILENAMES.contains(&file_name) {
            return true;
        }
    }

    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| CODE_SOURCE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn is_unhandled_binary_file(path: &Path) -> bool {
    if detect_mime_type(path) != "application/octet-stream" {
        return false;
    }

    let generated_artifact = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| GENERATED_BINARY_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false);
    generated_artifact || file_appears_binary(path).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Summary of an ingestion run for a single source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestResult {
    pub source_id: String,
    pub files_scanned: usize,
    pub files_added: usize,
    pub files_updated: usize,
    pub files_skipped: usize,
    pub files_failed: usize,
    pub files_purged: usize,
    pub errors: Vec<String>,
}

/// Result of ingesting a single file (for incremental watcher events).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestFileResult {
    /// File was new and inserted.
    Added,
    /// File content changed and was updated.
    Updated,
    /// File content unchanged — skipped.
    Unchanged,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan a source directory and ingest all matching files.
///
/// Walks the source's `root_path` recursively, applies include/exclude globs,
/// and for each matching file: parses it, checks for changes via content hash,
/// and inserts or updates the document and its chunks in the database.
pub fn scan_source(db: &Database, source_id: &str) -> Result<IngestResult, CoreError> {
    scan_source_inner(db, source_id, None, None)
}

/// Scan a source directory with an optional [`PrivacyConfig`].
///
/// When `privacy` is `Some`, its `exclude_patterns` are merged with the
/// source's own exclude globs and content redaction is applied to every
/// chunk before storage. When `None`, the stored config is loaded from the
/// database (falling back to defaults).
pub fn scan_source_with_privacy(
    db: &Database,
    source_id: &str,
    privacy: Option<&PrivacyConfig>,
) -> Result<IngestResult, CoreError> {
    scan_source_inner(db, source_id, privacy, None)
}

/// Scan a source directory with progress reporting.
///
/// Same as [`scan_source`] but calls `on_progress` at regular intervals
/// during the file-walk loop (approximately every 10 files).
pub fn scan_source_with_progress(
    db: &Database,
    source_id: &str,
    on_progress: impl Fn(ScanProgress),
) -> Result<IngestResult, CoreError> {
    scan_source_inner(db, source_id, None, Some(&on_progress))
}

/// Internal scan implementation shared by all public scan functions.
fn scan_source_inner(
    db: &Database,
    source_id: &str,
    privacy: Option<&PrivacyConfig>,
    on_progress: Option<&dyn Fn(ScanProgress)>,
) -> Result<IngestResult, CoreError> {
    let source = db.get_source(source_id)?;
    let services = db.knowledge_services_config()?;

    // Load file size limits from app config.
    let app_cfg = db.load_app_config().unwrap_or_default();
    let file_limits = FileSizeLimits {
        max_text: app_cfg.max_text_file_size,
        max_video: app_cfg.max_video_file_size,
        max_audio: app_cfg.max_audio_file_size,
    };

    // Resolve privacy config: explicit > stored > default.
    let stored_config = db.load_privacy_config()?;
    let stored_fingerprint = privacy::config_fingerprint(&stored_config)?;
    let privacy_cfg = privacy.unwrap_or(&stored_config);
    privacy::validate_config(privacy_cfg)?;
    if privacy_cfg.enabled {
        let conn = db.conn();
        if stored_fingerprint != privacy::config_fingerprint(&privacy::load_config_on(&conn)?)? {
            return Err(CoreError::Conflict(
                "Privacy settings changed before scanning; retry with the current rules".into(),
            ));
        }
        conn.execute("DELETE FROM evidence_snapshots WHERE source_id=?1 AND COALESCE(json_extract(document_metadata,'$.redaction_profile'),'')!=?2", params![source_id, privacy::redaction_fingerprint(privacy_cfg)?])?;
    }

    // Load video config from DB so user settings are used during parsing.
    #[cfg(feature = "video")]
    let video_config = db.load_video_config().ok();
    let ocr_config = db.load_ocr_config().ok();
    #[cfg(feature = "video")]
    let speech_config = app_cfg.speech_to_text.clone();

    let root = Path::new(&source.root_path);
    if !root.exists() {
        return Err(CoreError::InvalidInput(format!(
            "Source root path does not exist: {}",
            source.root_path
        )));
    }

    // Merge source excludes with privacy excludes.
    let mut all_excludes = source.exclude_globs.clone();
    all_excludes.extend(privacy_cfg.exclude_patterns.iter().cloned());

    let include_set = build_glob_set(&source.include_globs)?;
    let exclude_set = build_glob_set(&all_excludes)?;
    let has_includes = !source.include_globs.is_empty();

    let mut result = IngestResult {
        source_id: source_id.to_string(),
        files_scanned: 0,
        files_added: 0,
        files_updated: 0,
        files_skipped: 0,
        files_failed: 0,
        files_purged: 0,
        errors: Vec::new(),
    };

    // Derive max chunk size from the configured embedding model.
    // Floor at 1500 chars — the embedder truncates its own input, but
    // chunks must be large enough for good FTS recall.
    let max_chunk_chars = db
        .get_embedder_config()
        .ok()
        .map(|cfg| cfg.local_embedding_model().max_chunk_chars().max(1500));

    // Collect all files recursively, sorted for deterministic order.
    let files = walk_directory(root)?;
    let total_files = files.len();

    // Emit initial progress.
    if let Some(cb) = &on_progress {
        cb(ScanProgress {
            source_id: source_id.to_string(),
            phase: "scanning".to_string(),
            current: 0,
            total: total_files,
            current_file: None,
        });
    }

    // Pre-fetch all existing document paths and hashes for this source
    // to avoid N individual database lookups during scanning.
    let existing_docs = db.get_document_paths_for_source(source_id)?;

    // Collect new and updated documents for batched database operations.
    let mut new_docs: Vec<ParsedDocument> = Vec::new();
    let mut update_docs: Vec<(String, ParsedDocument)> = Vec::new(); // (doc_id, parsed)

    // Progress throttle: emit every 10 files.
    let progress_interval: usize = 10;
    let mut files_processed: usize = 0;

    for file_path in &files {
        // Compute relative path for glob matching, normalised to forward slashes.
        let rel_str = file_path
            .strip_prefix(root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .replace('\\', "/");

        // Include filter: if globs are specified, file must match at least one.
        if has_includes && !include_set.is_match(&rel_str) {
            files_processed += 1;
            continue;
        }

        // Exclude filter: skip files matching any exclude pattern.
        if exclude_set.is_match(&rel_str) {
            files_processed += 1;
            continue;
        }

        if is_code_source_file(file_path) {
            debug!(
                "Skipping source code file for knowledge embedding: {}",
                file_path.display()
            );
            result.files_skipped += 1;
            files_processed += 1;
            continue;
        }

        let file_path_str = file_path.to_string_lossy();
        if is_unhandled_binary_file(file_path) {
            debug!(
                "Skipping unsupported binary file for knowledge embedding: {}",
                file_path.display()
            );
            let _ = db.clear_scan_error(source_id, &file_path_str);
            result.files_skipped += 1;
            files_processed += 1;
            continue;
        }

        result.files_scanned += 1;
        files_processed += 1;

        // Emit progress at regular intervals.
        if let Some(cb) = &on_progress {
            if files_processed.is_multiple_of(progress_interval) || files_processed == total_files {
                cb(ScanProgress {
                    source_id: source_id.to_string(),
                    phase: "scanning".to_string(),
                    current: files_processed,
                    total: total_files,
                    current_file: Some(rel_str.clone()),
                });
            }
        }

        // Skip files exceeding the size limit to avoid excessive memory usage.
        match std::fs::metadata(file_path) {
            Ok(meta) if meta.len() > max_file_size_for_path(file_path, &file_limits) => {
                warn!(
                    "Skipping large file ({}MB): {}",
                    meta.len() / 1024 / 1024,
                    file_path.display()
                );
                result.files_skipped += 1;
                continue;
            }
            _ => {} // proceed normally (missing metadata is handled by parse_file)
        }

        // Skip files that have repeatedly failed (backoff).
        if !db
            .should_retry_scan(source_id, &file_path_str)
            .unwrap_or(true)
        {
            debug!(
                "Skipping file with repeated failures: {}",
                file_path.display()
            );
            result.files_skipped += 1;
            continue;
        }

        let media_progress = |progress: f32| {
            if let Some(cb) = &on_progress {
                cb(ScanProgress {
                    source_id: source_id.to_string(),
                    phase: "analyzing_media".to_string(),
                    current: progress.clamp(0.0, 100.0).round() as usize,
                    total: 100,
                    current_file: Some(rel_str.clone()),
                });
            }
        };
        match classify_file(
            file_path,
            &existing_docs,
            privacy_cfg,
            &stored_fingerprint,
            ocr_config.as_ref(),
            #[cfg(feature = "video")]
            video_config.as_ref(),
            #[cfg(feature = "video")]
            Some(&speech_config),
            Some(&media_progress),
            max_chunk_chars,
            &services,
        ) {
            Ok(FileClassification::New(parsed)) => {
                // File succeeded — clear any previous error record.
                let _ = db.clear_scan_error(source_id, &file_path_str);
                new_docs.push(parsed);
                result.files_added += 1;
            }
            Ok(FileClassification::Changed(doc_id, parsed)) => {
                // File succeeded — clear any previous error record.
                let _ = db.clear_scan_error(source_id, &file_path_str);
                update_docs.push((doc_id, parsed));
                result.files_updated += 1;
            }
            Ok(FileClassification::Unchanged) => {
                result.files_skipped += 1;
            }
            Err(e) => {
                let msg = format!("{}: {}", file_path.display(), e);
                warn!("Failed to ingest file: {}", msg);
                // Persist the scan error for tracking and backoff.
                let _ = db.upsert_scan_error(source_id, &file_path_str, &msg);
                result.errors.push(msg);
                result.files_failed += 1;
            }
        }
    }

    // Batch-insert all new documents in a single transaction.
    if !new_docs.is_empty() {
        debug!("Batch-inserting {} new documents", new_docs.len());
        batch_insert_documents(db, source_id, &new_docs)?;
    }

    // Batch-update all changed documents in a single transaction.
    if !update_docs.is_empty() {
        debug!("Batch-updating {} changed documents", update_docs.len());
        batch_update_documents(db, &update_docs)?;
    }

    // Exclusion changes also apply to files already removed from the live index.
    let scope_paths = db.knowledge_scope_paths(source_id)?;
    for doc_path in &scope_paths {
        let existing_path = Path::new(doc_path);
        let relative = relative_source_path(root, existing_path);
        let excluded = relative.as_ref().is_none_or(|relative| {
            (has_includes && !include_set.is_match(relative)) || exclude_set.is_match(relative)
        });
        if excluded || is_code_source_file(existing_path) || is_unhandled_binary_file(existing_path)
        {
            info!(
                "Purging unsupported document from knowledge source: {}",
                doc_path
            );
            match db.forget_document_in_source(source_id, doc_path) {
                Ok(true) => result.files_purged += 1,
                Ok(false) => {
                    debug!("Unsupported document already removed: {}", doc_path);
                }
                Err(e) => {
                    let msg = format!("Failed to purge unsupported document {}: {}", doc_path, e);
                    warn!("{}", msg);
                    result.errors.push(msg);
                }
            }
        } else if !existing_path.exists() {
            info!(
                "Purging stale document (file removed from disk): {}",
                doc_path
            );
            match db.delete_document_in_source(source_id, doc_path) {
                Ok(true) => result.files_purged += 1,
                Ok(false) => {
                    debug!("Stale document already removed: {}", doc_path);
                }
                Err(e) => {
                    let msg = format!("Failed to purge stale document {}: {}", doc_path, e);
                    warn!("{}", msg);
                    result.errors.push(msg);
                }
            }
        }
    }

    // Emit progress for purge phase.
    if result.files_purged > 0 {
        if let Some(cb) = &on_progress {
            cb(ScanProgress {
                source_id: source_id.to_string(),
                phase: "purging".to_string(),
                current: result.files_purged,
                total: result.files_purged,
                current_file: None,
            });
        }
    }

    // Emit final progress.
    if let Some(cb) = &on_progress {
        cb(ScanProgress {
            source_id: source_id.to_string(),
            phase: "scanning".to_string(),
            current: total_files,
            total: total_files,
            current_file: None,
        });
    }

    info!(
        "Scan complete for source {}: scanned={}, added={}, updated={}, skipped={}, failed={}, purged={}",
        source_id,
        result.files_scanned,
        result.files_added,
        result.files_updated,
        result.files_skipped,
        result.files_failed,
        result.files_purged
    );

    db.record_source_scan(&result)?;
    Ok(result)
}

/// Generate embeddings for all un-embedded chunks belonging to a source.
///
/// Reads the persisted [`EmbedderConfig`] to decide which embedder to use:
/// - `"tfidf"` — loads/builds TF-IDF from the corpus (original behaviour).
/// - `"local"` or `"api"` — delegates to [`create_embedder`].
pub fn embed_source(db: &Database, source_id: &str) -> Result<EmbedResult, CoreError> {
    crate::embedding_job::embed_source(db, source_id)
}

/// Generate embeddings with progress reporting.
///
/// Same as [`embed_source`] but calls `on_progress` after each batch of
/// chunks is embedded.
pub fn embed_source_with_progress(
    db: &Database,
    source_id: &str,
    on_progress: impl Fn(ScanProgress),
) -> Result<EmbedResult, CoreError> {
    crate::embedding_job::embed_source_with_progress(db, source_id, on_progress)
}

/// Delete all embeddings and rebuild them using the configured provider.
pub fn rebuild_embeddings(db: &Database) -> Result<EmbedResult, CoreError> {
    crate::embedding_job::rebuild_embeddings(db)
}

/// Rebuild all embeddings with progress reporting.
pub fn rebuild_embeddings_with_progress(
    db: &Database,
    on_progress: impl Fn(ScanProgress),
) -> Result<EmbedResult, CoreError> {
    crate::embedding_job::rebuild_embeddings_with_progress(db, on_progress)
}

/// Insert multiple parsed documents in a single transaction for bulk operations.
///
/// Much faster than calling `insert_document` per file, as SQLite transactions
/// are expensive per-call. Returns the number of documents inserted.
pub fn batch_insert_documents(
    db: &Database,
    source_id: &str,
    parsed_docs: &[ParsedDocument],
) -> Result<usize, CoreError> {
    let mut conn = db.conn();
    let tx = conn.transaction()?;
    let mut count = 0usize;
    for parsed in parsed_docs {
        validate_privacy_at_commit(&tx, parsed)?;
        let doc_id = uuid::Uuid::new_v4().to_string();
        let metadata_json = parsed_metadata_json(parsed)?;
        tx.execute(
            "INSERT INTO documents (id, source_id, path, title, mime_type, file_size,
                                    modified_at, content_hash, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), ?7, ?8)",
            params![
                &doc_id,
                source_id,
                &parsed.file_path,
                &parsed.title,
                &parsed.mime_type,
                parsed.file_size,
                &parsed.content_hash,
                &metadata_json,
            ],
        )?;
        insert_chunks(&tx, &doc_id, &parsed.chunks)?;
        insert_visual_artifact_chunks(&tx, &doc_id, parsed.chunks.len(), &parsed.visual_artifacts)?;
        count += 1;
    }
    tx.commit()?;
    Ok(count)
}

/// Update multiple existing documents in a single transaction for bulk operations.
///
/// Each entry is `(doc_id, parsed_document)`. Old chunks are deleted first
/// and new chunks are inserted, all within a single transaction.
pub fn batch_update_documents(
    db: &Database,
    updates: &[(String, ParsedDocument)],
) -> Result<usize, CoreError> {
    let mut conn = db.conn();
    let tx = conn.transaction()?;
    let mut count = 0usize;
    for (doc_id, parsed) in updates {
        validate_privacy_at_commit(&tx, parsed)?;
        // Delete old chunks — FTS triggers fire automatically.
        tx.execute("DELETE FROM chunks WHERE document_id = ?1", params![doc_id])?;
        revoke_changed_redaction_history(&tx, doc_id, parsed)?;

        // Update the document record.
        let metadata_json = parsed_metadata_json(parsed)?;
        tx.execute(
            "UPDATE documents
             SET mime_type = ?1, file_size = ?2, modified_at = datetime('now'),
                 content_hash = ?3, indexed_at = datetime('now'),
                 title = ?4, metadata = ?5
             WHERE id = ?6",
            params![
                &parsed.mime_type,
                parsed.file_size,
                &parsed.content_hash,
                &parsed.title,
                &metadata_json,
                doc_id,
            ],
        )?;

        insert_chunks(&tx, doc_id, &parsed.chunks)?;
        insert_visual_artifact_chunks(&tx, doc_id, parsed.chunks.len(), &parsed.visual_artifacts)?;
        count += 1;
    }
    tx.commit()?;
    Ok(count)
}

// ---------------------------------------------------------------------------
// Database methods for document CRUD
// ---------------------------------------------------------------------------

impl Database {
    /// Look up a document by its file path.
    ///
    /// Returns `(id, content_hash)` if a matching row exists, `None` otherwise.
    pub fn get_document_by_path(
        &self,
        file_path: &str,
    ) -> Result<Option<(String, String)>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, content_hash FROM documents WHERE path = ?1")?;
        let result = stmt.query_row(params![file_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        });
        match result {
            Ok(pair) => Ok(Some(pair)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(CoreError::Database(e)),
        }
    }

    /// Pre-fetch all document paths and content hashes for a given source.
    ///
    /// Returns a `HashMap` from file path to `(document_id, content_hash)`,
    /// enabling O(1) lookups instead of N individual database queries.
    pub fn get_document_paths_for_source(
        &self,
        source_id: &str,
    ) -> Result<HashMap<String, IndexedDocument>, CoreError> {
        let source = self.get_source(source_id)?;
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT id, path, content_hash, COALESCE(json_extract(metadata,'$.parser_profile'),''), COALESCE(json_extract(metadata,'$.parsed_hash'),''), COALESCE(json_extract(metadata,'$.redaction_profile'),'') FROM documents WHERE source_id = ?1")?;
        let rows = stmt.query_map(params![source_id], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(0)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        let records = rows.collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        drop(conn);
        let mut map = HashMap::new();
        for (path, id, content_hash, parser_profile, parsed_hash, redaction_profile) in records {
            let key = relative_source_path(Path::new(&source.root_path), Path::new(&path))
                .map(|relative| {
                    Path::new(&source.root_path)
                        .join(relative)
                        .to_string_lossy()
                        .to_string()
                })
                .unwrap_or(path);
            map.entry(key).or_insert(IndexedDocument {
                id,
                content_hash,
                parser_profile,
                parsed_hash,
                redaction_profile,
            });
        }
        Ok(map)
    }

    fn knowledge_scope_paths(&self, source_id: &str) -> Result<Vec<String>, CoreError> {
        let conn = self.conn();
        let mut statement=conn.prepare("SELECT path FROM documents WHERE source_id=?1 UNION SELECT document_path FROM evidence_snapshots WHERE source_id=?1 UNION SELECT path FROM research_documents WHERE source_id=?1")?;
        let rows = statement
            .query_map([source_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_document_in_source(
        &self,
        source_id: &str,
        path: &str,
    ) -> Result<Option<IndexedDocument>, CoreError> {
        use rusqlite::OptionalExtension;
        let read = |row: &rusqlite::Row<'_>| {
            Ok(IndexedDocument {
                id: row.get(0)?,
                content_hash: row.get(1)?,
                parser_profile: row.get(2)?,
                parsed_hash: row.get(3)?,
                redaction_profile: row.get(4)?,
            })
        };
        let exact = self.conn().query_row("SELECT id,content_hash,COALESCE(json_extract(metadata,'$.parser_profile'),''),COALESCE(json_extract(metadata,'$.parsed_hash'),''),COALESCE(json_extract(metadata,'$.redaction_profile'),'') FROM documents WHERE source_id=?1 AND path=?2", params![source_id, path], read).optional()?;
        if exact.is_some() {
            return Ok(exact);
        }
        let aliases = serde_json::to_string(&document_path_aliases(path))?;
        Ok(self.conn().query_row("SELECT id,content_hash,COALESCE(json_extract(metadata,'$.parser_profile'),''),COALESCE(json_extract(metadata,'$.parsed_hash'),''),COALESCE(json_extract(metadata,'$.redaction_profile'),'') FROM documents WHERE source_id=?1 AND path IN (SELECT value FROM json_each(?2)) ORDER BY indexed_at DESC LIMIT 1", params![source_id, aliases], read).optional()?)
    }

    pub fn delete_document_in_source(
        &self,
        source_id: &str,
        path: &str,
    ) -> Result<bool, CoreError> {
        Ok(self.conn().execute(
            "DELETE FROM documents WHERE source_id=?1 AND path=?2",
            params![source_id, path],
        )? > 0)
    }

    pub fn forget_document_in_source(
        &self,
        source_id: &str,
        path: &str,
    ) -> Result<bool, CoreError> {
        let aliases = serde_json::to_string(&document_path_aliases(path))?;
        let mut conn = self.conn();
        let transaction = conn.transaction()?;
        // Resolve identity before deleting: revisions may retain a different
        // path spelling, and deleted files may only have archive/research rows.
        let document_ids = {
            let mut statement = transaction.prepare(
                "SELECT id FROM documents WHERE source_id=?1 AND path IN (SELECT value FROM json_each(?2))
                 UNION SELECT document_id FROM evidence_snapshots WHERE source_id=?1 AND document_path IN (SELECT value FROM json_each(?2))
                 UNION SELECT document_id FROM research_documents WHERE source_id=?1 AND path IN (SELECT value FROM json_each(?2))",
            )?;
            let rows =
                statement.query_map(params![source_id, aliases], |row| row.get::<_, String>(0))?;
            serde_json::to_string(&rows.collect::<Result<Vec<_>, _>>()?)?
        };
        let changed = transaction.execute(
            "DELETE FROM documents WHERE source_id=?1 AND id IN (SELECT value FROM json_each(?2))",
            params![source_id, document_ids],
        )? > 0;
        let archived = transaction.execute(
            "DELETE FROM evidence_snapshots WHERE source_id=?1 AND document_id IN (SELECT value FROM json_each(?2))",
            params![source_id, document_ids],
        )?;
        let research = transaction.execute(
            "DELETE FROM research_documents WHERE source_id=?1 AND document_id IN (SELECT value FROM json_each(?2))",
            params![source_id, document_ids],
        )?;
        transaction.commit()?;
        Ok(changed || archived > 0 || research > 0)
    }

    /// Insert a new document and all its chunks within a single transaction.
    ///
    /// Returns the generated document ID.
    pub fn insert_document(
        &self,
        source_id: &str,
        parsed: &ParsedDocument,
    ) -> Result<String, CoreError> {
        let doc_id = uuid::Uuid::new_v4().to_string();

        let mut conn = self.conn();
        let tx = conn.transaction()?;

        validate_privacy_at_commit(&tx, parsed)?;

        let metadata_json = parsed_metadata_json(parsed)?;
        tx.execute(
            "INSERT INTO documents (id, source_id, path, title, mime_type, file_size,
                                    modified_at, content_hash, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), ?7, ?8)",
            params![
                &doc_id,
                source_id,
                &parsed.file_path,
                &parsed.title,
                &parsed.mime_type,
                parsed.file_size,
                &parsed.content_hash,
                &metadata_json,
            ],
        )?;

        insert_chunks(&tx, &doc_id, &parsed.chunks)?;
        insert_visual_artifact_chunks(&tx, &doc_id, parsed.chunks.len(), &parsed.visual_artifacts)?;

        tx.commit()?;
        Ok(doc_id)
    }

    /// Update an existing document record and replace all its chunks.
    ///
    /// Old chunks are deleted first (FTS triggers handle cleanup),
    /// then the document row is updated and new chunks are inserted.
    pub fn update_document(&self, doc_id: &str, parsed: &ParsedDocument) -> Result<(), CoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;

        validate_privacy_at_commit(&tx, parsed)?;

        // Delete old chunks — FTS triggers fire automatically.
        tx.execute("DELETE FROM chunks WHERE document_id = ?1", params![doc_id])?;
        revoke_changed_redaction_history(&tx, doc_id, parsed)?;

        // Update the document record.
        let metadata_json = parsed_metadata_json(parsed)?;
        tx.execute(
            "UPDATE documents
             SET mime_type = ?1, file_size = ?2, modified_at = datetime('now'),
                 content_hash = ?3, indexed_at = datetime('now'),
                 title = ?4, metadata = ?5
             WHERE id = ?6",
            params![
                &parsed.mime_type,
                parsed.file_size,
                &parsed.content_hash,
                &parsed.title,
                &metadata_json,
                doc_id,
            ],
        )?;

        insert_chunks(&tx, doc_id, &parsed.chunks)?;
        insert_visual_artifact_chunks(&tx, doc_id, parsed.chunks.len(), &parsed.visual_artifacts)?;

        tx.commit()?;
        Ok(())
    }

    /// Delete all documents (and their chunks via CASCADE) for a source.
    ///
    /// Returns the number of documents deleted.
    pub fn delete_documents_for_source(&self, source_id: &str) -> Result<usize, CoreError> {
        let conn = self.conn();
        let count = conn.execute(
            "DELETE FROM documents WHERE source_id = ?1",
            params![source_id],
        )?;
        Ok(count)
    }

    /// Delete a document (and its chunks via CASCADE) by file path.
    ///
    /// Returns `true` if a document was found and deleted, `false` if no
    /// document matched the given path.
    pub fn delete_document_by_path(&self, file_path: &str) -> Result<bool, CoreError> {
        let conn = self.conn();
        let deleted = conn.execute("DELETE FROM documents WHERE path = ?1", params![file_path])?;
        Ok(deleted > 0)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Classification of a file during scanning.
enum FileClassification {
    /// New file — not yet in the database.
    New(ParsedDocument),
    /// Changed file — content hash differs from stored version.
    Changed(String, ParsedDocument), // (doc_id, parsed)
    /// Unchanged file — content hash matches, skip.
    Unchanged,
}

/// Classify a file as new, changed, or unchanged (without performing DB writes).
#[allow(clippy::too_many_arguments)]
fn classify_file(
    path: &Path,
    existing_docs: &HashMap<String, IndexedDocument>,
    privacy: &PrivacyConfig,
    stored_fingerprint: &str,
    ocr_config: Option<&crate::ocr::OcrConfig>,
    #[cfg(feature = "video")] video_config: Option<&crate::video::VideoConfig>,
    #[cfg(feature = "video")] speech_config: Option<&crate::app_settings::SpeechToTextConfig>,
    progress_callback: Option<&dyn Fn(f32)>,
    max_chunk_chars: Option<usize>,
    services: &crate::knowledge_services::KnowledgeServicesConfig,
) -> Result<FileClassification, CoreError> {
    #[cfg(feature = "video")]
    let file_path = path.to_string_lossy().to_string();
    #[cfg(feature = "video")]
    let known_content_hash = {
        let redaction_profile = privacy::redaction_fingerprint(privacy)?;
        let mime_type = detect_mime_type(path);
        if mime_type.starts_with("audio/") || mime_type.starts_with("video/") {
            let hash = hash_file_content(path)?;
            if existing_docs.get(&file_path).is_some_and(|existing| {
                existing.content_hash == hash
                    && existing.parser_profile == NATIVE_PARSER_PROFILE
                    && existing.redaction_profile == redaction_profile
            }) {
                debug!("Skipping unchanged media before analysis: {}", file_path);
                return Ok(FileClassification::Unchanged);
            }
            Some(hash)
        } else {
            None
        }
    };

    let mut parsed = if let Some(parsed) =
        crate::knowledge_services::parse_pdf(services, path, max_chunk_chars.unwrap_or(2000))?
    {
        parsed
    } else {
        parse_file_with_media_config(
            path,
            ocr_config,
            #[cfg(feature = "video")]
            video_config,
            #[cfg(feature = "video")]
            speech_config,
            None,
            progress_callback,
            max_chunk_chars,
            #[cfg(feature = "video")]
            known_content_hash.as_deref(),
            #[cfg(not(feature = "video"))]
            None,
        )?
    };

    apply_privacy(&mut parsed, privacy, stored_fingerprint)?;

    match existing_docs.get(&parsed.file_path) {
        Some(existing) => {
            if existing.content_hash == parsed.content_hash
                && existing.parser_profile == parser_profile(&parsed)
                && existing.parsed_hash == parsed_hash(&parsed)
            {
                debug!("Skipping unchanged file: {}", parsed.file_path);
                Ok(FileClassification::Unchanged)
            } else {
                debug!("File changed: {}", parsed.file_path);
                Ok(FileClassification::Changed(existing.id.clone(), parsed))
            }
        }
        None => {
            debug!("New file: {}", parsed.file_path);
            Ok(FileClassification::New(parsed))
        }
    }
}

/// Insert chunks for a document within an existing transaction.
fn insert_chunks(
    tx: &rusqlite::Transaction<'_>,
    doc_id: &str,
    chunks: &[ParsedChunk],
) -> Result<(), CoreError> {
    for chunk in chunks {
        let chunk_id = uuid::Uuid::new_v4().to_string();
        let chunk_hash = blake3::hash(chunk.content.as_bytes()).to_hex().to_string();
        let (line_start, line_end) = match &chunk.locator {
            crate::evidence::EvidenceLocator::Text {
                line_start,
                line_end,
                ..
            } => (*line_start as i64, *line_end as i64),
            _ => (0, 0),
        };
        let metadata = {
            let mut meta = serde_json::Map::new();
            meta.insert("locator".into(), serde_json::to_value(&chunk.locator)?);
            meta.insert(
                "extraction_method".into(),
                serde_json::json!(chunk.extraction_method),
            );
            if let Some(h) = &chunk.heading_context {
                meta.insert(
                    "heading_context".to_string(),
                    serde_json::Value::String(h.clone()),
                );
            }
            if chunk.overlap_start > 0 {
                meta.insert(
                    "overlap_start".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(chunk.overlap_start)),
                );
            }
            serde_json::Value::Object(meta).to_string()
        };

        tx.execute(
            "INSERT INTO chunks (id, document_id, chunk_index, kind, content,
                                 start_offset, end_offset, line_start, line_end,
                                 content_hash, metadata_json)
             VALUES (?1, ?2, ?3, 'text', ?4, ?5, ?6, ?10, ?7, ?8, ?9)",
            params![
                &chunk_id,
                doc_id,
                chunk.chunk_index,
                &chunk.content,
                chunk.start_offset,
                chunk.end_offset,
                line_end,
                &chunk_hash,
                &metadata,
                line_start,
            ],
        )?;
    }
    Ok(())
}

fn insert_visual_artifact_chunks(
    tx: &rusqlite::Transaction<'_>,
    doc_id: &str,
    base_chunk_count: usize,
    artifacts: &[crate::visual_document::ParsedVisualArtifact],
) -> Result<(), CoreError> {
    for artifact in artifacts {
        let chunk_id = uuid::Uuid::new_v4().to_string();
        let content = artifact.to_chunk_content();
        let chunk_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        let line_end = content.lines().count().max(1) as i64;
        let metadata = serde_json::Value::Object(
            crate::visual_document::build_visual_artifact_metadata(artifact),
        )
        .to_string();
        let chunk_index = base_chunk_count as i32 + artifact.artifact_index;

        tx.execute(
            "INSERT INTO chunks (id, document_id, chunk_index, kind, content,
                                 start_offset, end_offset, line_start, line_end,
                                 content_hash, metadata_json)
             VALUES (?1, ?2, ?3, 'visual_artifact', ?4, 0, 0, 1, ?5, ?6, ?7)",
            params![
                &chunk_id,
                doc_id,
                chunk_index,
                &content,
                line_end,
                &chunk_hash,
                &metadata,
            ],
        )?;
    }
    Ok(())
}

/// Build a `GlobSet` from a list of glob pattern strings.
fn build_glob_set(patterns: &[String]) -> Result<GlobSet, CoreError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|e| {
            CoreError::InvalidInput(format!("Invalid glob pattern '{pattern}': {e}"))
        })?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| CoreError::InvalidInput(format!("Failed to build glob set: {e}")))
}

/// Ingest a single file for incremental watcher events.
///
/// Parses the file, checks its content hash against the database, and
/// inserts or updates the document accordingly. Returns the outcome.
pub fn ingest_single_file(
    db: &Database,
    source_id: &str,
    path: &Path,
) -> Result<IngestFileResult, CoreError> {
    ingest_file(db, source_id, path, false)
}

pub fn reindex_single_file(
    db: &Database,
    source_id: &str,
    path: &Path,
) -> Result<IngestFileResult, CoreError> {
    ingest_file(db, source_id, path, true)
}

fn ingest_file(
    db: &Database,
    source_id: &str,
    path: &Path,
    force: bool,
) -> Result<IngestFileResult, CoreError> {
    if !path.is_file() {
        return Err(CoreError::InvalidInput(format!(
            "Path is not a file: {}",
            path.display()
        )));
    }

    let source = db.get_source(source_id)?;
    let canonical_root = std::fs::canonicalize(&source.root_path)?;
    let canonical_path = std::fs::canonicalize(path)?;
    let relative = canonical_path
        .strip_prefix(&canonical_root)
        .map_err(|_| CoreError::InvalidInput("File is outside the selected source".into()))?
        .to_string_lossy()
        .replace('\\', "/");
    // Match directory scanning at every single-file entry point, including
    // watcher and tool paths containing a Windows verbatim prefix or root alias.
    let stable_path = Path::new(&source.root_path).join(&relative);
    let path = stable_path.as_path();
    let privacy_cfg = db.load_privacy_config()?;
    privacy::validate_config(&privacy_cfg)?;
    let stored_fingerprint = privacy::config_fingerprint(&privacy_cfg)?;
    let includes = build_glob_set(&source.include_globs)?;
    let mut excludes = source.exclude_globs.clone();
    excludes.extend(privacy_cfg.exclude_patterns.iter().cloned());
    if (!source.include_globs.is_empty() && !includes.is_match(&relative))
        || build_glob_set(&excludes)?.is_match(&relative)
    {
        db.forget_document_in_source(source_id, &path.to_string_lossy())?;
        return Err(CoreError::InvalidInput(
            "File is excluded by source or privacy rules".into(),
        ));
    }

    if is_code_source_file(path) {
        debug!(
            "Skipping source code file for knowledge embedding: {}",
            path.display()
        );
        let path_str = path.to_string_lossy();
        let _ = db.forget_document_in_source(source_id, path_str.as_ref())?;
        return Ok(IngestFileResult::Unchanged);
    }

    if is_unhandled_binary_file(path) {
        debug!(
            "Skipping unsupported binary file for knowledge embedding: {}",
            path.display()
        );
        let path_str = path.to_string_lossy();
        let _ = db.clear_scan_error(source_id, &path_str);
        let _ = db.forget_document_in_source(source_id, path_str.as_ref())?;
        return Ok(IngestFileResult::Unchanged);
    }

    // Load file size limits from app config.
    let app_cfg = db.load_app_config().unwrap_or_default();
    let file_limits = FileSizeLimits {
        max_text: app_cfg.max_text_file_size,
        max_video: app_cfg.max_video_file_size,
        max_audio: app_cfg.max_audio_file_size,
    };

    // Skip files exceeding the size limit.
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > max_file_size_for_path(path, &file_limits) {
            warn!(
                "Skipping large file ({}MB): {}",
                meta.len() / 1024 / 1024,
                path.display()
            );
            return Ok(IngestFileResult::Unchanged);
        }
    }

    // Skip files that have repeatedly failed (backoff).
    let path_str = path.to_string_lossy();
    if !force && !db.should_retry_scan(source_id, &path_str).unwrap_or(true) {
        debug!("Skipping file with repeated failures: {}", path.display());
        return Ok(IngestFileResult::Unchanged);
    }

    // Load video config from DB so user settings are used during parsing.
    #[cfg(feature = "video")]
    let video_config = db.load_video_config().ok();
    let ocr_config = db.load_ocr_config().ok();
    #[cfg(feature = "video")]
    let speech_config = app_cfg.speech_to_text.clone();

    // Derive max chunk size from the configured embedding model.
    // Floor at 1500 chars — the embedder truncates its own input, but
    // chunks must be large enough for good FTS recall.
    let max_chunk_chars = db
        .get_embedder_config()
        .ok()
        .map(|cfg| cfg.local_embedding_model().max_chunk_chars().max(1500));

    let existing_document = db.get_document_in_source(source_id, &path_str)?;
    #[cfg(feature = "video")]
    let known_content_hash = {
        let redaction_profile = privacy::redaction_fingerprint(&privacy_cfg)?;
        let mime_type = detect_mime_type(path);
        if mime_type.starts_with("audio/") || mime_type.starts_with("video/") {
            let hash = hash_file_content(path)?;
            if existing_document.as_ref().is_some_and(|existing| {
                !force
                    && existing.content_hash == hash
                    && existing.parser_profile == NATIVE_PARSER_PROFILE
                    && existing.redaction_profile == redaction_profile
            }) {
                debug!("Single-file ingest: unchanged media before analysis {path_str}");
                return Ok(IngestFileResult::Unchanged);
            }
            Some(hash)
        } else {
            None
        }
    };

    let services = db.knowledge_services_config()?;
    let parsed_result =
        crate::knowledge_services::parse_pdf(&services, path, max_chunk_chars.unwrap_or(2000))
            .and_then(|enhanced| {
                if let Some(parsed) = enhanced {
                    return Ok(parsed);
                }
                parse_file_with_media_config(
                    path,
                    ocr_config.as_ref(),
                    #[cfg(feature = "video")]
                    video_config.as_ref(),
                    #[cfg(feature = "video")]
                    Some(&speech_config),
                    None,
                    None,
                    max_chunk_chars,
                    #[cfg(feature = "video")]
                    known_content_hash.as_deref(),
                    #[cfg(not(feature = "video"))]
                    None,
                )
            });

    let mut parsed = match parsed_result {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("{}: {}", path.display(), e);
            let _ = db.upsert_scan_error(source_id, &path_str, &msg);
            return Err(e);
        }
    };

    apply_privacy(&mut parsed, &privacy_cfg, &stored_fingerprint)?;

    // Clear any previous scan error on success.
    let _ = db.clear_scan_error(source_id, &path_str);

    // Check if the document already exists.
    match existing_document {
        Some(existing) => {
            if !force
                && existing.content_hash == parsed.content_hash
                && existing.parser_profile == parser_profile(&parsed)
                && existing.parsed_hash == parsed_hash(&parsed)
            {
                debug!("Single-file ingest: unchanged {}", parsed.file_path);
                Ok(IngestFileResult::Unchanged)
            } else {
                debug!("Single-file ingest: updating {}", parsed.file_path);
                db.update_document(&existing.id, &parsed)?;
                Ok(IngestFileResult::Updated)
            }
        }
        None => {
            debug!("Single-file ingest: adding {}", parsed.file_path);
            db.insert_document(source_id, &parsed)?;
            Ok(IngestFileResult::Added)
        }
    }
}

/// Bounded path spellings shared by identity lookup and explicit revocation.
fn document_path_aliases(path: &str) -> Vec<String> {
    let mut aliases = vec![path.to_string()];
    if let Ok(canonical) = std::fs::canonicalize(path) {
        aliases.push(canonical.to_string_lossy().into());
    }
    #[cfg(windows)]
    for alias in aliases.clone() {
        let windows = alias.replace('/', "\\");
        let simple = if let Some(rest) = windows.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else {
            windows
                .strip_prefix(r"\\?\")
                .unwrap_or(&windows)
                .to_string()
        };
        let extended = if let Some(rest) = simple.strip_prefix(r"\\") {
            format!(r"\\?\UNC\{rest}")
        } else {
            format!(r"\\?\{simple}")
        };
        for spelling in [windows, simple, extended] {
            aliases.push(spelling.replace('\\', "/"));
            aliases.push(spelling);
        }
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn relative_source_path(root: &Path, path: &Path) -> Option<String> {
    fn normalized(path: &Path) -> String {
        let value = path.to_string_lossy().replace('\\', "/");
        if let Some(rest) = value.strip_prefix("//?/UNC/") {
            format!("//{rest}")
        } else {
            value.strip_prefix("//?/").unwrap_or(&value).to_string()
        }
    }
    if path.exists() {
        let canonical_root = std::fs::canonicalize(root).ok()?;
        let canonical_path = std::fs::canonicalize(path).ok()?;
        return canonical_path
            .strip_prefix(canonical_root)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/"));
    }
    let roots = [
        normalized(root),
        std::fs::canonicalize(root)
            .ok()
            .map(|value| normalized(&value))
            .unwrap_or_default(),
    ];
    let path = normalized(path);
    for root in roots {
        if root.is_empty() {
            continue;
        }
        let root = root.trim_end_matches('/');
        let Some(prefix) = path.get(..root.len()) else {
            continue;
        };
        if !(if cfg!(windows) {
            prefix.eq_ignore_ascii_case(root)
        } else {
            prefix == root
        }) {
            continue;
        }
        let Some(relative) = path
            .get(root.len()..)
            .and_then(|value| value.strip_prefix('/'))
        else {
            continue;
        };
        if relative.split('/').any(|segment| segment == "..") {
            continue;
        }
        return Some(relative.to_string());
    }
    None
}

/// Recursively walk a directory, collecting all file paths (sorted).
fn walk_directory(root: &Path) -> Result<Vec<PathBuf>, CoreError> {
    let mut files = Vec::new();
    walk_recursive(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), CoreError> {
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            continue;
        }
        if path.is_dir() {
            walk_recursive(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::CreateSourceInput;
    use std::fs;
    use tempfile::TempDir;

    fn test_db() -> Database {
        let db = Database::open_memory().expect("open in-memory db");
        db.save_embedder_config(&crate::embed::EmbedderConfig {
            provider: "tfidf".into(),
            ..crate::embed::EmbedderConfig::default()
        })
        .expect("set tfidf config for test");
        db
    }

    fn create_test_source(
        db: &Database,
        dir: &Path,
        include: Vec<String>,
        exclude: Vec<String>,
    ) -> String {
        db.add_source(CreateSourceInput {
            root_path: dir.to_string_lossy().to_string(),
            include_globs: include,
            exclude_globs: exclude,
            watch_enabled: false,
        })
        .expect("add source")
        .id
    }

    fn vault_path() -> PathBuf {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        manifest
            .join("..")
            .join("..")
            .join("testdata")
            .join("sample_vault")
    }

    // ── Scan sample vault ───────────────────────────────────────────────

    #[test]
    fn test_scan_sample_vault() {
        let vp = vault_path();
        if !vp.exists() {
            eprintln!("Skipping: test vault not found at {}", vp.display());
            return;
        }

        let db = test_db();
        let source_id = create_test_source(&db, &vp, vec![], vec![]);

        let result = scan_source(&db, &source_id).expect("scan_source");

        assert_eq!(result.source_id, source_id);
        // 4 files: 2 in docs/, 2 in notes/
        assert!(
            result.files_scanned >= 4,
            "expected >= 4 files scanned, got {}",
            result.files_scanned
        );
        assert_eq!(result.files_added, result.files_scanned);
        assert_eq!(result.files_updated, 0);
        assert_eq!(result.files_skipped, 0);
    }

    // ── Incremental scanning ────────────────────────────────────────────

    #[test]
    fn test_incremental_scan_skips_unchanged() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("hello.md");
        fs::write(
            &file,
            "# Hello\n\nThis is a test document with enough content to pass the \
             minimum chunk size threshold for parsing. It needs at least fifty \
             characters to not be discarded by the chunker.",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);

        // First scan — adds the file.
        let r1 = scan_source(&db, &sid).unwrap();
        assert_eq!(r1.files_added, 1);
        assert_eq!(r1.files_skipped, 0);

        // Second scan — same content, should skip.
        let r2 = scan_source(&db, &sid).unwrap();
        assert_eq!(r2.files_added, 0);
        assert_eq!(r2.files_skipped, 1);
        assert_eq!(r2.files_updated, 0);
    }

    #[test]
    fn test_incremental_scan_detects_changes() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("doc.md");
        fs::write(
            &file,
            "# Original\n\nOriginal content that is long enough to be a valid \
             chunk for the parser to process correctly and not be discarded.",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);

        let r1 = scan_source(&db, &sid).unwrap();
        assert_eq!(r1.files_added, 1);

        // Modify file content so the hash changes.
        fs::write(
            &file,
            "# Modified\n\nCompletely different content that should trigger an \
             update because the blake3 hash will differ from the original text.",
        )
        .unwrap();

        let r2 = scan_source(&db, &sid).unwrap();
        assert_eq!(r2.files_updated, 1);
        assert_eq!(r2.files_added, 0);
        assert_eq!(r2.files_skipped, 0);
    }

    // ── Glob filtering ──────────────────────────────────────────────────

    #[test]
    fn test_glob_include_filter() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("readme.md"),
            "# Readme\n\nMarkdown file with plenty of content to satisfy the \
             minimum chunk size requirement for the parser to accept it.",
        )
        .unwrap();
        fs::write(
            tmp.path().join("notes.txt"),
            "Plain text notes that are long enough to pass the minimum chunk \
             size in the parser so they actually produce at least one chunk.",
        )
        .unwrap();
        fs::write(
            tmp.path().join("data.log"),
            "2025-07-15 10:00:00 INFO Log entry data with plenty of content \
             here to meet minimum requirements for chunking algorithm.",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec!["**/*.md".to_string()], vec![]);

        let result = scan_source(&db, &sid).unwrap();
        assert_eq!(result.files_scanned, 1, "only .md files should be scanned");
        assert_eq!(result.files_added, 1);
    }

    #[test]
    fn test_glob_exclude_filter() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("readme.md"),
            "# Readme\n\nA markdown file with enough content to be parsed into \
             at least one chunk by the parser for indexing purposes.",
        )
        .unwrap();
        fs::write(
            tmp.path().join("notes.txt"),
            "Some plain text notes that contain enough words and characters to \
             pass as a valid parseable chunk in the plain text chunker.",
        )
        .unwrap();
        fs::create_dir_all(tmp.path().join("logs")).unwrap();
        fs::write(
            tmp.path().join("logs").join("app.log"),
            "2025-07-15 10:00:00 INFO Log file entry with timestamp and plenty \
             of text to ensure the chunk size minimum is met by the parser.",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec!["**/*.log".to_string()]);

        let result = scan_source(&db, &sid).unwrap();
        assert_eq!(result.files_scanned, 2, "log files should be excluded");
    }

    #[test]
    fn test_scan_source_skips_code_files_for_embedding() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("notes.md"),
            "# Notes\n\nMarkdown content with enough detail to produce a chunk for \
             the knowledge index while source code files are ignored.",
        )
        .unwrap();
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(
            tmp.path().join("src").join("app.ts"),
            "export const answer = 42;\nconsole.log(answer);\n",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);

        let result = scan_source(&db, &sid).unwrap();
        assert_eq!(
            result.files_scanned, 1,
            "only non-code files should be scanned"
        );
        assert_eq!(result.files_added, 1);
        assert_eq!(result.files_skipped, 1);

        let docs = db.get_document_paths_for_source(&sid).unwrap();
        assert_eq!(docs.len(), 1);
        assert!(docs.keys().any(|path| path.ends_with("notes.md")));
        assert!(docs.keys().all(|path| !path.ends_with("app.ts")));
    }

    #[test]
    fn test_scan_source_skips_compiled_and_unknown_binary_files_without_errors() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("notes.md"),
            "# Notes\n\nReadable knowledge content remains indexable while generated binary files are ignored safely.",
        )
        .unwrap();
        fs::write(
            tmp.path().join("module.pyc"),
            [0xA7, 0x0D, 0x0D, 0x0A, 0xFF, 0xFE],
        )
        .unwrap();
        fs::write(tmp.path().join("cache.bin"), b"binary\0payload").unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);

        let result = scan_source(&db, &sid).unwrap();
        assert_eq!(result.files_scanned, 1);
        assert_eq!(result.files_added, 1);
        assert_eq!(result.files_skipped, 2);
        assert_eq!(result.files_failed, 0);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_code_include_glob_is_skipped_for_embedding() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("app.ts"),
            "export function main() {\n  return 'not embedded';\n}\n",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec!["**/*.ts".to_string()], vec![]);

        let result = scan_source(&db, &sid).unwrap();
        assert_eq!(result.files_scanned, 0);
        assert_eq!(result.files_added, 0);
        assert_eq!(result.files_skipped, 1);
        assert!(db.get_document_paths_for_source(&sid).unwrap().is_empty());
    }

    // ── Error handling ──────────────────────────────────────────────────

    #[test]
    fn test_scan_missing_path() {
        let db = test_db();

        // Insert a source with a non-existent path directly (bypassing
        // add_source validation).
        let id = uuid::Uuid::new_v4().to_string();
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sources (id, kind, root_path) \
                 VALUES (?1, 'local_folder', '/nonexistent/path/abc123')",
                params![&id],
            )
            .unwrap();
        }

        let result = scan_source(&db, &id);
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidInput(msg) => {
                assert!(
                    msg.contains("does not exist"),
                    "expected 'does not exist' in: {msg}"
                );
            }
            other => panic!("expected InvalidInput, got: {other:?}"),
        }
    }

    // ── Document CRUD ───────────────────────────────────────────────────

    #[test]
    fn test_document_crud_via_db() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("test.md");
        fs::write(
            &file,
            "# Test\n\nSome content for the test document that is long enough \
             to meet the minimum chunk size requirement for parsing.",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);

        // Parse and insert.
        let parsed = parse_file(
            &file,
            None,
            #[cfg(feature = "video")]
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let doc_id = db.insert_document(&sid, &parsed).unwrap();
        assert!(!doc_id.is_empty());

        // Lookup by path — should find the document.
        let found = db.get_document_by_path(&parsed.file_path).unwrap();
        assert!(found.is_some());
        let (fid, fhash) = found.unwrap();
        assert_eq!(fid, doc_id);
        assert_eq!(fhash, parsed.content_hash);

        // Lookup missing path — should return None.
        let missing = db.get_document_by_path("/no/such/file.md").unwrap();
        assert!(missing.is_none());

        // Update with new content.
        fs::write(
            &file,
            "# Updated\n\nNew content for the updated test document that should \
             produce a different blake3 content hash value now.",
        )
        .unwrap();
        let parsed2 = parse_file(
            &file,
            None,
            #[cfg(feature = "video")]
            None,
            None,
            None,
            None,
        )
        .unwrap();
        db.update_document(&doc_id, &parsed2).unwrap();

        let (_, new_hash) = db
            .get_document_by_path(&parsed2.file_path)
            .unwrap()
            .unwrap();
        assert_ne!(new_hash, parsed.content_hash);
        assert_eq!(new_hash, parsed2.content_hash);

        // Delete all docs for source.
        let deleted = db.delete_documents_for_source(&sid).unwrap();
        assert_eq!(deleted, 1);

        let gone = db.get_document_by_path(&parsed2.file_path).unwrap();
        assert!(gone.is_none());
    }

    // ── FTS integration ─────────────────────────────────────────────────

    #[test]
    fn test_ingested_chunks_are_fts_searchable() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("searchable.md"),
            "# Searchable\n\nThis document contains the unique sentinel word \
             xylophonezebra that we will search for in the full-text index.",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);

        scan_source(&db, &sid).unwrap();

        let conn = db.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM fts_chunks WHERE fts_chunks MATCH 'xylophonezebra'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "FTS should find the ingested chunk");
    }

    // ── Embedding integration ───────────────────────────────────────────

    #[test]
    fn test_embed_source_after_scan() {
        let vp = vault_path();
        if !vp.exists() {
            eprintln!("Skipping: test vault not found at {}", vp.display());
            return;
        }

        let db = test_db();
        let sid = create_test_source(&db, &vp, vec![], vec![]);

        // Scan first to populate chunks.
        let scan = scan_source(&db, &sid).unwrap();
        assert!(scan.files_added > 0);

        // Now embed.
        let embed = embed_source(&db, &sid).unwrap();
        assert_eq!(embed.source_id, sid);
        assert!(embed.chunks_embedded > 0, "should embed at least one chunk");
        assert_eq!(embed.model, "tfidf-v1");
    }

    #[test]
    fn test_all_chunks_get_embeddings() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("file1.md"),
            "# File One\n\nFirst document with enough content to satisfy \
             the minimum chunk size requirement for the parser to accept it.",
        )
        .unwrap();
        fs::write(
            tmp.path().join("file2.md"),
            "# File Two\n\nSecond document also with plenty of content to \
             be properly chunked and indexed by the ingestion pipeline.",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);
        scan_source(&db, &sid).unwrap();

        let embed = embed_source(&db, &sid).unwrap();
        assert!(embed.chunks_embedded > 0);

        // Every chunk should now have an embedding.
        let missing = db
            .count_chunks_without_embeddings_for_source(&sid, "tfidf-v1")
            .unwrap();
        assert_eq!(missing, 0, "all source chunks should have embeddings");
    }

    #[test]
    fn test_rebuild_embeddings_clears_and_reembeds() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("doc.md"),
            "# Rebuild Test\n\nDocument used to verify that rebuild_embeddings \
             deletes existing vectors and creates fresh ones from scratch.",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);
        scan_source(&db, &sid).unwrap();

        // Initial embed.
        let e1 = embed_source(&db, &sid).unwrap();
        assert!(e1.chunks_embedded > 0);

        // Rebuild.
        let rebuild = rebuild_embeddings(&db).unwrap();
        assert!(rebuild.chunks_embedded > 0);
        assert_eq!(rebuild.chunks_skipped, 0);
        assert_eq!(rebuild.model, "tfidf-v1");

        // All chunks should still have embeddings after rebuild.
        let missing = db
            .count_chunks_without_embeddings_for_source(&sid, "tfidf-v1")
            .unwrap();
        assert_eq!(missing, 0);
    }

    #[test]
    fn test_incremental_embedding() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("first.md"),
            "# First\n\nInitial document with enough text to be parsed and \
             chunked properly by the ingestion system for embedding.",
        )
        .unwrap();

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);
        scan_source(&db, &sid).unwrap();

        let e1 = embed_source(&db, &sid).unwrap();
        let initial_embedded = e1.chunks_embedded;
        assert!(initial_embedded > 0);

        // Add a second file.
        fs::write(
            tmp.path().join("second.md"),
            "# Second\n\nA brand new document added after the first embedding \
             run to verify that only new chunks get embedded incrementally.",
        )
        .unwrap();

        // Re-scan picks up the new file.
        let scan2 = scan_source(&db, &sid).unwrap();
        assert_eq!(scan2.files_added, 1);

        // Embed again — should only embed the new chunks.
        let e2 = embed_source(&db, &sid).unwrap();
        assert!(e2.chunks_embedded > 0, "new chunks should be embedded");

        // No chunks should be missing.
        let missing = db
            .count_chunks_without_embeddings_for_source(&sid, "tfidf-v1")
            .unwrap();
        assert_eq!(missing, 0);
    }

    #[test]
    fn test_embedding_job_is_strictly_source_scoped() {
        let first = TempDir::new().unwrap();
        let second = TempDir::new().unwrap();
        fs::write(
            first.path().join("first.md"),
            "# First source\n\nEnough source-specific content to create a searchable chunk for the first source.",
        )
        .unwrap();
        fs::write(
            second.path().join("second.md"),
            "# Second source\n\nEnough different content to create a searchable chunk for the second source.",
        )
        .unwrap();

        let db = test_db();
        let first_id = create_test_source(&db, first.path(), vec![], vec![]);
        let second_id = create_test_source(&db, second.path(), vec![], vec![]);
        scan_source(&db, &first_id).unwrap();
        scan_source(&db, &second_id).unwrap();

        crate::embedding_job::run_source(
            &db,
            &first_id,
            crate::embedding_job::EmbeddingJobLimits {
                batch_size: 1,
                max_intra_threads: 1,
            },
            &crate::embedding_job::NoopEmbeddingJobControl,
        )
        .unwrap();

        assert_eq!(
            db.count_chunks_without_embeddings_for_source(&first_id, "tfidf-v1")
                .unwrap(),
            0
        );
        assert!(
            db.count_chunks_without_embeddings_for_source(&second_id, "tfidf-v1")
                .unwrap()
                > 0,
            "embedding one source must not mutate another source"
        );
    }

    #[test]
    fn test_batch_insert_documents() {
        let tmp = TempDir::new().unwrap();
        for i in 0..100 {
            fs::write(
                tmp.path().join(format!("doc_{:03}.md", i)),
                format!(
                    "# Document {i}\n\nThis is test document number {i} with enough \
                     content to pass the minimum chunk size requirement for parsing.",
                ),
            )
            .unwrap();
        }

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);

        // Parse all files.
        let mut parsed_docs: Vec<crate::parse::ParsedDocument> = Vec::new();
        for entry in fs::read_dir(tmp.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                parsed_docs.push(
                    parse_file(
                        &path,
                        None,
                        #[cfg(feature = "video")]
                        None,
                        None,
                        None,
                        None,
                    )
                    .unwrap(),
                );
            }
        }
        assert_eq!(parsed_docs.len(), 100);

        let count = batch_insert_documents(&db, &sid, &parsed_docs).unwrap();
        assert_eq!(count, 100);

        // Verify all documents exist.
        for parsed in &parsed_docs {
            let found = db.get_document_by_path(&parsed.file_path).unwrap();
            assert!(
                found.is_some(),
                "Document {} should exist",
                parsed.file_path
            );
        }
    }

    #[test]
    fn test_scan_source_prefetch_many_files() {
        let tmp = TempDir::new().unwrap();
        for i in 0..10 {
            fs::write(
                tmp.path().join(format!("file_{}.md", i)),
                format!(
                    "# File {i}\n\nContent of file number {i} with sufficient text to \
                     pass the minimum chunk size requirement for the parser.",
                ),
            )
            .unwrap();
        }

        let db = test_db();
        let sid = create_test_source(&db, tmp.path(), vec![], vec![]);

        // First scan adds all files.
        let r1 = scan_source(&db, &sid).unwrap();
        assert_eq!(r1.files_added, 10);

        // Second scan — pre-fetched paths/hashes make all lookups skip.
        let r2 = scan_source(&db, &sid).unwrap();
        assert_eq!(r2.files_skipped, 10);
        assert_eq!(r2.files_added, 0);
        assert_eq!(r2.files_updated, 0);
    }

    #[cfg(feature = "video")]
    #[test]
    fn unchanged_media_is_classified_before_any_ffmpeg_work() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("unchanged.mp4");
        fs::write(
            &path,
            b"invalid video bytes are sufficient for hash preflight",
        )
        .unwrap();
        let path_string = path.to_string_lossy().to_string();
        let hash = hash_file_content(&path).unwrap();
        let existing = HashMap::from([(
            path_string,
            IndexedDocument {
                id: "doc-1".into(),
                content_hash: hash,
                parser_profile: NATIVE_PARSER_PROFILE.into(),
                parsed_hash: String::new(),
                redaction_profile: privacy::redaction_fingerprint(&PrivacyConfig::default())
                    .unwrap(),
            },
        )]);
        let video = crate::video::VideoConfig {
            enabled: true,
            ffmpeg_path: Some("/definitely/missing/ffmpeg".into()),
            ..crate::video::VideoConfig::default()
        };

        let classification = classify_file(
            &path,
            &existing,
            &PrivacyConfig::default(),
            &privacy::config_fingerprint(&PrivacyConfig::default()).unwrap(),
            Some(&crate::ocr::OcrConfig::default()),
            Some(&video),
            Some(&crate::app_settings::SpeechToTextConfig::default()),
            None,
            None,
            &crate::knowledge_services::KnowledgeServicesConfig::default(),
        )
        .unwrap();

        assert!(matches!(classification, FileClassification::Unchanged));
    }
}
