//! Shared home and managed model locations. Project rules retain project scope.
//!
//! Old populated model directories remain readable until explicit consolidation.
//! Consolidation copies and verifies files first, then commits every model path
//! in one transaction. It never deletes originals or overwrites conflicts.

use crate::{db::Database, embed::LocalEmbeddingModel, error::CoreError};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub fn home_dir() -> Result<PathBuf, CoreError> {
    let path = std::env::var_os(crate::user_extensions::NEXA_HOME_ENV)
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".nexa")))
        .ok_or_else(|| CoreError::InvalidInput("Cannot determine Nexa home".into()))?;
    if !path.is_absolute() {
        return Err(CoreError::InvalidInput(
            "NEXA_HOME must be an absolute path".into(),
        ));
    }
    Ok(path)
}

pub fn model_root() -> Result<PathBuf, CoreError> {
    Ok(home_dir()?.join("models"))
}

fn legacy_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for directory in [dirs::data_dir(), dirs::data_local_dir()]
        .into_iter()
        .flatten()
    {
        let root = directory.join(crate::APP_DIR).join("models");
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

fn populated(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some())
}

fn resolve_existing(canonical: PathBuf, legacy: impl IntoIterator<Item = PathBuf>) -> PathBuf {
    if populated(&canonical) {
        return canonical;
    }
    legacy
        .into_iter()
        .find(|path| populated(path))
        .unwrap_or(canonical)
}

/// Read compatibility only. Fresh downloads converge on the canonical home.
pub fn resolve_model_dir(name: &str) -> Result<PathBuf, CoreError> {
    Ok(resolve_existing(
        model_root()?.join(name),
        legacy_roots().into_iter().map(|root| root.join(name)),
    ))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedModelPaths {
    pub root: String,
    pub embedding: String,
    pub ocr: String,
    pub whisper: String,
}

impl ManagedModelPaths {
    pub fn new(root: Option<&str>, model: &LocalEmbeddingModel) -> Result<Self, CoreError> {
        let root = root
            .filter(|root| !root.trim().is_empty())
            .map(|root| PathBuf::from(root.trim()))
            .map(Ok)
            .unwrap_or_else(model_root)?;
        if !root.is_absolute()
            || root
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(CoreError::InvalidInput(
                "Local model root must be an absolute path without parent traversal".into(),
            ));
        }
        Ok(Self {
            embedding: root.join(model.model_name()).to_string_lossy().into_owned(),
            ocr: root.join("paddleocr").to_string_lossy().into_owned(),
            whisper: root.join("whisper").to_string_lossy().into_owned(),
            root: root.to_string_lossy().into_owned(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStorageReport {
    pub paths: ManagedModelPaths,
    pub copied_files: u64,
    pub copied_bytes: u64,
    pub retained_sources: Vec<String>,
}

// Only the path-related state participates in CAS. Other concurrent settings,
// especially encrypted API credentials, are preserved by JSON field updates.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelPathSettings {
    root: Option<String>,
    embedding: Option<String>,
    embedding_model: Option<String>,
    ocr: Option<String>,
    whisper: Option<String>,
}

fn path_settings(conn: &rusqlite::Connection) -> Result<ModelPathSettings, CoreError> {
    let read = |sql: &str| -> Result<Option<String>, CoreError> {
        Ok(conn
            .query_row(sql, [], |row| row.get::<_, Option<String>>(0))
            .optional()?
            .flatten())
    };
    Ok(ModelPathSettings {
        root: read("SELECT json_extract(value,'$.localModelRoot') FROM app_config WHERE key='app_config'")?,
        embedding: read("SELECT value FROM embedder_config WHERE key='model_path'")?,
        embedding_model: read("SELECT value FROM embedder_config WHERE key='local_model'")?,
        ocr: read("SELECT json_extract(value,'$.modelPath') FROM ocr_config WHERE key='ocr_config'")?,
        whisper: read("SELECT COALESCE(json_extract(value,'$.modelPath'),json_extract(value,'$.model_path')) FROM video_config WHERE key='config'")?,
    })
}

fn configured_or_existing(value: Option<&str>, name: &str) -> Result<PathBuf, CoreError> {
    match value.filter(|value| !value.trim().is_empty()) {
        Some(value) => Ok(PathBuf::from(value)),
        None => resolve_model_dir(name),
    }
}

fn ensure_config_tables(conn: &rusqlite::Connection) -> Result<(), CoreError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS app_config (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT (datetime('now')));
        CREATE TABLE IF NOT EXISTS embedder_config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS ocr_config (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT (datetime('now')));
        CREATE TABLE IF NOT EXISTS video_config (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
    Ok(())
}

/// User-requested consolidation. Long copies run outside the database lock.
pub fn consolidate_models(
    db: &Database,
    root: Option<&str>,
) -> Result<ModelStorageReport, CoreError> {
    let expected = {
        let conn = db.conn();
        ensure_config_tables(&conn)?;
        path_settings(&conn)?
    };
    let model = expected
        .embedding_model
        .as_deref()
        .map(LocalEmbeddingModel::from_config_str)
        .unwrap_or_default();
    let paths = ManagedModelPaths::new(root, &model)?;
    let embedding_source =
        configured_or_existing(expected.embedding.as_deref(), model.model_name())?;
    let ocr_source = configured_or_existing(expected.ocr.as_deref(), "paddleocr")?;
    let whisper_source = configured_or_existing(expected.whisper.as_deref(), "whisper")?;
    let mut sources = vec![
        (embedding_source, PathBuf::from(&paths.embedding)),
        (ocr_source, PathBuf::from(&paths.ocr)),
        (whisper_source, PathBuf::from(&paths.whisper)),
    ];
    for other in [
        LocalEmbeddingModel::MultilingualMiniLM,
        LocalEmbeddingModel::MultilingualE5Base,
        LocalEmbeddingModel::Qwen3Embedding06B,
    ] {
        if other.model_name() == model.model_name() {
            continue;
        }
        let source = if expected
            .root
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            resolve_model_dir(other.model_name())?
        } else {
            Path::new(expected.root.as_deref().unwrap_or_default()).join(other.model_name())
        };
        sources.push((source, Path::new(&paths.root).join(other.model_name())));
    }
    let mut report = ModelStorageReport {
        paths,
        copied_files: 0,
        copied_bytes: 0,
        retained_sources: Vec::new(),
    };
    for (source, target) in sources {
        copy_model_tree(&source, &target, &mut report)?;
    }
    let mut conn = db.conn();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if path_settings(&tx)? != expected {
        return Err(CoreError::Conflict("Model paths changed during consolidation; originals and settings were preserved. Retry with the current settings.".into()));
    }
    tx.execute("INSERT INTO app_config(key,value) VALUES('app_config',json_set(?2,'$.localModelRoot',?1)) ON CONFLICT(key) DO UPDATE SET value=json_set(value,'$.localModelRoot',?1),updated_at=datetime('now')", params![report.paths.root, serde_json::to_string(&crate::app_settings::AppConfig::default())?])?;
    tx.execute("INSERT INTO embedder_config(key,value) VALUES('model_path',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&report.paths.embedding])?;
    tx.execute("INSERT INTO ocr_config(key,value) VALUES('ocr_config',json_set(?2,'$.modelPath',?1)) ON CONFLICT(key) DO UPDATE SET value=json_set(value,'$.modelPath',?1),updated_at=datetime('now')", params![report.paths.ocr, serde_json::to_string(&crate::ocr::OcrConfig::default())?])?;
    // Preserve all video settings. serde aliases must not coexist with the
    // canonical field, or the next load would report a duplicate field.
    #[cfg(feature = "video")]
    tx.execute("INSERT INTO video_config(key,value) VALUES('config',json_set(?2,'$.modelPath',?1)) ON CONFLICT(key) DO UPDATE SET value=json_set(json_remove(value,'$.model_path'),'$.modelPath',?1)", params![report.paths.whisper, serde_json::to_string(&crate::video::VideoConfig::default())?])?;
    #[cfg(not(feature = "video"))]
    tx.execute("UPDATE video_config SET value=json_set(json_remove(value,'$.model_path'),'$.modelPath',?1) WHERE key='config'", [&report.paths.whisper])?;
    crate::settings_schema_v2::sync_legacy_app_config_in_transaction(&tx)?;
    crate::capability_registry::sync_registry_in_transaction(&tx)?;
    tx.commit()?;
    Ok(report)
}

fn reject_link(path: &Path) -> Result<(), CoreError> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(CoreError::InvalidInput(format!(
                    "Model storage cannot traverse a symbolic link: {}",
                    ancestor.display()
                )))
            }
            #[cfg(windows)]
            Ok(meta)
                if {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_attributes() & 0x400 != 0
                } =>
            {
                return Err(CoreError::InvalidInput(format!(
                    "Model storage cannot traverse a reparse point: {}",
                    ancestor.display()
                )))
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<blake3::Hash, CoreError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(std::fs::File::open(path)?)?;
    Ok(hasher.finalize())
}

fn copy_model_tree(
    source: &Path,
    target: &Path,
    report: &mut ModelStorageReport,
) -> Result<(), CoreError> {
    reject_link(source)?;
    reject_link(target)?;
    if !source.exists() {
        return Ok(());
    }
    if !source.is_dir() {
        return Err(CoreError::InvalidInput(format!(
            "Model directory is not a folder: {}",
            source.display()
        )));
    }
    let canonical_source = source.canonicalize()?;
    if target.exists() && target.canonicalize()? == canonical_source {
        return Ok(());
    }
    // A nested destination would be copied recursively or become its own source.
    let target_identity = crate::file_mutation::canonical_file_identity(target)?;
    if target_identity.starts_with(&canonical_source)
        || canonical_source.starts_with(&target_identity)
    {
        return Err(CoreError::InvalidInput(
            "Source and destination model folders must not contain one another".into(),
        ));
    }
    let entries = walkdir::WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .take(10_001)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| CoreError::InvalidInput(error.to_string()))?;
    if entries.len() > 10_000 {
        return Err(CoreError::InvalidInput(
            "Model directory has too many files".into(),
        ));
    }
    let mut files = Vec::new();
    for entry in entries {
        reject_link(entry.path())?;
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            return Err(CoreError::InvalidInput(
                "Only regular model files can be consolidated".into(),
            ));
        }
        let name = entry.file_name().to_string_lossy();
        if name.ends_with(".partial") || name.ends_with(".part") || name.ends_with(".tmp") {
            return Err(CoreError::Conflict(
                "A model download is incomplete. Finish or cancel it before consolidating models."
                    .into(),
            ));
        }
        let destination = target.join(
            entry
                .path()
                .strip_prefix(source)
                .map_err(|error| CoreError::Internal(error.to_string()))?,
        );
        reject_link(&destination)?;
        let digest = file_hash(entry.path())?;
        if destination.exists() {
            if !destination.is_file() || file_hash(&destination)? != digest {
                return Err(CoreError::Conflict(format!(
                    "Different model file already exists at {}; no file was overwritten",
                    destination.display()
                )));
            }
        } else {
            files.push((entry.into_path(), destination, digest));
        }
    }
    if !files.is_empty() {
        report
            .retained_sources
            .push(source.to_string_lossy().into_owned());
    }
    for (source_file, destination, digest) in files {
        let parent = destination
            .parent()
            .ok_or_else(|| CoreError::Internal("Missing model parent directory".into()))?;
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        let bytes = std::io::copy(
            &mut std::fs::File::open(&source_file)?,
            temporary.as_file_mut(),
        )?;
        temporary.as_file().sync_all()?;
        if file_hash(temporary.path())? != digest || file_hash(&source_file)? != digest {
            return Err(CoreError::Conflict(
                "Model changed during consolidation; settings were preserved".into(),
            ));
        }
        temporary
            .persist_noclobber(&destination)
            .map_err(|error| CoreError::Io(error.error))?;
        report.copied_files += 1;
        report.copied_bytes += bytes;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Database, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let old = directory.path().join("old");
        db.save_app_config(&crate::app_settings::AppConfig {
            local_model_root: old.to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
        let embedding = old.join("selected-embedding");
        std::fs::create_dir_all(&embedding).unwrap();
        std::fs::write(embedding.join("model.onnx"), b"fixture model bytes").unwrap();
        db.save_embedder_config(&crate::embed::EmbedderConfig {
            model_path: embedding.to_string_lossy().into_owned(),
            api_model: "keep-cloud-model".into(),
            ..Default::default()
        })
        .unwrap();
        db.save_ocr_config(&crate::ocr::OcrConfig {
            model_path: old.join("ocr").to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
        ensure_config_tables(&db.conn()).unwrap();
        db.conn().execute("INSERT INTO video_config(key,value) VALUES('config',?1)", [serde_json::json!({"modelPath":old.join("whisper").to_string_lossy(),"enabled":false}).to_string()]).unwrap();
        (directory, db, old)
    }

    #[test]
    fn fresh_models_use_home_but_populated_legacy_remains_readable() {
        let tmp = tempfile::tempdir().unwrap();
        let new = tmp.path().join("new");
        let old = tmp.path().join("old");
        std::fs::create_dir_all(&old).unwrap();
        assert_eq!(resolve_existing(new.clone(), [old.clone()]), new);
        std::fs::write(old.join("model"), "legacy").unwrap();
        assert_eq!(resolve_existing(new.clone(), [old.clone()]), old);
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("model"), "canonical").unwrap();
        assert_eq!(resolve_existing(new.clone(), [old]), new);
    }

    #[test]
    fn consolidation_verifies_files_and_switches_all_paths_without_losing_settings() {
        let (tmp, db, old) = fixture();
        let target = tmp.path().join("central");
        let report = consolidate_models(&db, Some(target.to_str().unwrap())).unwrap();
        assert_eq!(report.copied_files, 1);
        assert_eq!(
            std::fs::read(Path::new(&report.paths.embedding).join("model.onnx")).unwrap(),
            b"fixture model bytes"
        );
        assert!(old.join("selected-embedding/model.onnx").is_file());
        assert_eq!(
            db.get_embedder_config().unwrap().api_model,
            "keep-cloud-model"
        );
        assert_eq!(
            db.get_embedder_config().unwrap().model_path,
            report.paths.embedding
        );
        assert_eq!(db.load_ocr_config().unwrap().model_path, report.paths.ocr);
        assert_eq!(
            db.load_app_config().unwrap().local_model_root,
            report.paths.root
        );
        assert_eq!(
            path_settings(&db.conn()).unwrap().whisper.as_deref(),
            Some(report.paths.whisper.as_str())
        );
        assert_eq!(
            consolidate_models(&db, Some(target.to_str().unwrap()))
                .unwrap()
                .copied_files,
            0
        );
    }

    #[test]
    fn conflicting_model_file_preserves_files_and_all_paths() {
        let (tmp, db, old) = fixture();
        let target = tmp.path().join("central");
        let paths = ManagedModelPaths::new(
            target.to_str(),
            &db.get_embedder_config().unwrap().local_embedding_model(),
        )
        .unwrap();
        std::fs::create_dir_all(&paths.embedding).unwrap();
        std::fs::write(
            Path::new(&paths.embedding).join("model.onnx"),
            b"existing target",
        )
        .unwrap();
        let before = path_settings(&db.conn()).unwrap();
        assert!(consolidate_models(&db, target.to_str()).is_err());
        assert_eq!(path_settings(&db.conn()).unwrap(), before);
        assert_eq!(
            std::fs::read(old.join("selected-embedding/model.onnx")).unwrap(),
            b"fixture model bytes"
        );
        assert_eq!(
            std::fs::read(Path::new(&paths.embedding).join("model.onnx")).unwrap(),
            b"existing target"
        );
    }

    #[test]
    fn failed_settings_commit_rolls_back_every_model_path() {
        let (tmp, db, old) = fixture();
        let before = path_settings(&db.conn()).unwrap();
        db.conn().execute_batch("CREATE TRIGGER reject_model_config BEFORE UPDATE ON ocr_config BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        assert!(consolidate_models(&db, tmp.path().join("central").to_str()).is_err());
        assert_eq!(path_settings(&db.conn()).unwrap(), before);
        assert!(old.join("selected-embedding/model.onnx").is_file());
    }

    #[test]
    fn partial_downloads_do_not_activate_a_new_location() {
        let (tmp, db, old) = fixture();
        std::fs::write(
            old.join("selected-embedding/model.onnx.partial"),
            b"in progress",
        )
        .unwrap();
        let before = path_settings(&db.conn()).unwrap();
        assert!(consolidate_models(&db, tmp.path().join("central").to_str()).is_err());
        assert_eq!(path_settings(&db.conn()).unwrap(), before);
    }
}
