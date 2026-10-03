//! ReindexTool — triggers re-indexing of a document by path or an entire source.

use std::path::Path;
use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;

#[cfg(test)]
use crate::db::Database;
use crate::error::CoreError;
use crate::ingest;

use super::path_utils::resolve_existing_file_in_sources;
use super::{ensure_source_in_scope, scoped_sources, Tool, ToolCategory, ToolDef, ToolResult};

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/reindex_document.json");

#[derive(Deserialize)]
struct ReindexArgs {
    path: Option<String>,
    source_id: Option<String>,
}

pub struct ReindexTool;

/// Find the source whose `root_path` contains the given file path.
fn find_source_for_path(
    sources: &[crate::models::Source],
    file_path: &Path,
) -> Option<crate::models::Source> {
    let canonical = std::fs::canonicalize(file_path).ok()?;
    sources
        .iter()
        .filter_map(|source| {
            let root = std::fs::canonicalize(&source.root_path).ok()?;
            canonical
                .starts_with(&root)
                .then_some((root.components().count(), source))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, source)| source.clone())
}

#[async_trait]
impl Tool for ReindexTool {
    fn name(&self) -> &str {
        "reindex_document"
    }

    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }

    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::SourceManagement]
    }

    async fn execute(
        &self,
        context: crate::tools::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        let crate::tools::ToolExecutionContext {
            call_id,
            arguments,
            db,
            source_scope,
            ..
        } = context;
        let args: ReindexArgs = serde_json::from_str(arguments).map_err(|e| {
            CoreError::InvalidInput(format!("Invalid reindex_document arguments: {e}"))
        })?;

        if args.path.is_none() && args.source_id.is_none() {
            return Ok(ToolResult {
                call_id: call_id.to_string(),
                content: "At least one of 'path' or 'source_id' must be provided.".to_string(),
                is_error: true,
                artifacts: None,
            });
        }

        let db = db.clone();
        let call_id = call_id.to_string();
        let source_scope = source_scope.to_vec();

        tokio::task::spawn_blocking(move || {
            if let Some(ref source_id) = args.source_id {
                // Validate source exists.
                let source = db.get_source(source_id).map_err(|_| {
                    CoreError::NotFound(format!("Source not found: {source_id}"))
                })?;
                ensure_source_in_scope(&source.id, &source_scope)
                    .map_err(CoreError::InvalidInput)?;

                let result = ingest::scan_source(&db, source_id)?;
                return Ok(ToolResult {
                    call_id,
                    content: format!(
                        "Re-scanned source '{}': {} files scanned, {} added, {} updated, {} skipped.",
                        source_id,
                        result.files_scanned,
                        result.files_added,
                        result.files_updated,
                        result.files_skipped,
                    ),
                    is_error: false,
                    artifacts: None,
                });
            }

            // path mode
            let file_path_str = args.path.as_ref().unwrap();
            let sources = scoped_sources(&db, &source_scope)?;
            let file_path = resolve_existing_file_in_sources(Path::new(file_path_str), &sources)
                .map_err(CoreError::InvalidInput)?;

            let source = match find_source_for_path(&sources, &file_path) {
                Some(s) => s,
                None => {
                    return Ok(ToolResult {
                        call_id,
                        content: format!(
                            "File '{}' is not within any registered source directory.",
                            file_path_str
                        ),
                        is_error: true,
                        artifacts: None,
                    });
                }
            };

            // Preserve document identity and archived evidence even when its
            // file hash is unchanged. Mutations stay inside the selected source.
            // Shared ingestion resolves both current and legacy path spellings.
            let outcome = ingest::reindex_single_file(&db, &source.id, &file_path)?;
            let status = match outcome {
                ingest::IngestFileResult::Added => "added (re-indexed)",
                ingest::IngestFileResult::Updated => "updated",
                ingest::IngestFileResult::Unchanged => "re-indexed (unchanged content)",
            };

            Ok(ToolResult {
                call_id,
                content: format!("Document '{}' {status}.", file_path_str),
                is_error: false,
                artifacts: None,
            })
        })
        .await
        .map_err(|e| CoreError::Internal(format!("Task join error: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn setup_db_with_source(dir: &Path) -> (Database, String) {
        let db = Database::open_memory().expect("open in-memory db");
        let source = db
            .add_source(crate::sources::CreateSourceInput {
                root_path: dir.to_string_lossy().to_string(),
                include_globs: vec![],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .expect("add source");
        (db, source.id)
    }

    fn create_test_file(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).expect("create file");
        f.write_all(content.as_bytes()).expect("write file");
        path
    }

    #[tokio::test]
    async fn test_reindex_single_file() {
        let tmp = TempDir::new().expect("tempdir");
        let file = create_test_file(tmp.path(), "note.md", "# Hello\nWorld\n");
        let (db, source_id) = setup_db_with_source(tmp.path());

        // Initial ingest
        ingest::ingest_single_file(&db, &source_id, &file).expect("initial ingest");

        // Update the file content
        std::fs::write(&file, "# Hello\nUpdated world\n").expect("overwrite");

        // Reindex via tool
        let tool = ReindexTool;
        let args = serde_json::json!({ "path": file.to_string_lossy() }).to_string();
        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "c1",
                &args,
                &db,
                &[],
            ))
            .await
            .expect("execute");

        assert!(
            !result.is_error,
            "Expected success, got: {}",
            result.content
        );
        assert!(
            result.content.contains("added (re-indexed)") || result.content.contains("updated"),
            "Unexpected message: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_reindex_source() {
        let tmp = TempDir::new().expect("tempdir");
        create_test_file(tmp.path(), "a.md", "# A\nContent A\n");
        create_test_file(tmp.path(), "b.md", "# B\nContent B\n");
        let (db, source_id) = setup_db_with_source(tmp.path());

        let tool = ReindexTool;
        let args = serde_json::json!({ "source_id": source_id }).to_string();
        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "c2",
                &args,
                &db,
                &[],
            ))
            .await
            .expect("execute");

        assert!(
            !result.is_error,
            "Expected success, got: {}",
            result.content
        );
        assert!(result.content.contains("Re-scanned source"));
        assert!(result.content.contains("2 files scanned"));
    }

    #[tokio::test]
    async fn test_reindex_requires_param() {
        let db = Database::open_memory().expect("open in-memory db");
        let tool = ReindexTool;
        let args = "{}";
        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "c3",
                args,
                &db,
                &[],
            ))
            .await
            .expect("execute");
        assert!(result.is_error);
        assert!(result.content.contains("At least one"));
    }
    #[tokio::test]
    async fn reindex_path_keeps_the_authorized_nested_source_and_document_identity() {
        let root = TempDir::new().unwrap();
        let nested = root.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let file = create_test_file(&nested, "policy.md", "Policy allowance is 500 yuan.");
        let (db, parent) = setup_db_with_source(root.path());
        let child = db
            .add_source(crate::sources::CreateSourceInput {
                root_path: nested.to_string_lossy().into(),
                include_globs: vec![],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        for source in [&parent, &child.id] {
            ingest::ingest_single_file(&db, source, &file).unwrap();
        }
        let revision = |source: &str| {
            db.conn()
                .query_row(
                    "SELECT id,index_revision FROM documents WHERE source_id=?1",
                    [source],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap()
        };
        let parent_before = revision(&parent);
        let child_before = revision(&child.id);
        let args = serde_json::json!({"path":file.to_string_lossy()}).to_string();
        let result = ReindexTool
            .execute(crate::tools::ToolExecutionContext::new(
                "scoped-reindex",
                &args,
                &db,
                std::slice::from_ref(&child.id),
            ))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(revision(&parent), parent_before);
        let child_after = revision(&child.id);
        assert_eq!(child_after.0, child_before.0);
        assert_ne!(child_after.1, child_before.1);
        assert_eq!(
            db.conn()
                .query_row("SELECT COUNT(*) FROM documents", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        let mut privacy = db.load_privacy_config().unwrap();
        privacy.exclude_patterns.push("**/policy.md".into());
        db.save_privacy_config(&privacy).unwrap();
        assert!(ReindexTool
            .execute(crate::tools::ToolExecutionContext::new(
                "excluded-reindex",
                &args,
                &db,
                std::slice::from_ref(&child.id)
            ))
            .await
            .is_err());
        assert_eq!(revision(&parent), parent_before);
        assert!(db
            .get_document_paths_for_source(&child.id)
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn excluded_legacy_paths_revoke_current_historical_and_research_evidence() {
        let folder = TempDir::new().unwrap();
        let inner = folder.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        let alias = inner.join("..");
        let file = create_test_file(folder.path(), "policy.md", "Policy allowance is 500 yuan.");
        let (db, source) = setup_db_with_source(&alias);
        ingest::ingest_single_file(&db, &source, &file).unwrap();
        db.conn()
            .execute(
                "UPDATE documents SET path=?2 WHERE source_id=?1",
                rusqlite::params![
                    source,
                    std::fs::canonicalize(&file).unwrap().to_string_lossy()
                ],
            )
            .unwrap();
        let query = crate::models::SearchQuery {
            text: "allowance".into(),
            filters: Default::default(),
            limit: 2,
            offset: 0,
        };
        let reference = crate::search::search(&db, &query)
            .unwrap()
            .evidence_cards
            .remove(0)
            .evidence_ref
            .unwrap();
        let set = crate::research_workspace::create(
            &db,
            crate::research_workspace::CreateResearchSet {
                title: "Policy".into(),
                questions: vec!["allowance".into()],
                documents: vec![reference.clone()],
            },
        )
        .unwrap();
        // Keep an older revision and a saved comparison under the legacy spelling.
        ingest::reindex_single_file(&db, &source, &file).unwrap();
        db.conn()
            .execute(
                "UPDATE documents SET path=?2 WHERE source_id=?1",
                rusqlite::params![
                    source,
                    std::fs::canonicalize(&file).unwrap().to_string_lossy()
                ],
            )
            .unwrap();
        assert_eq!(
            crate::search::resolve_evidence_ref(&db, &reference)
                .unwrap()
                .evidence_ref
                .unwrap()
                .status,
            "historical"
        );
        let mut privacy = db.load_privacy_config().unwrap();
        privacy.exclude_patterns.push("**/policy.md".into());
        db.save_privacy_config(&privacy).unwrap();
        let args = serde_json::json!({"path":file.to_string_lossy()}).to_string();
        assert!(ReindexTool
            .execute(crate::tools::ToolExecutionContext::new(
                "excluded-legacy",
                &args,
                &db,
                std::slice::from_ref(&source),
            ))
            .await
            .is_err());
        assert!(crate::search::search(&db, &query)
            .unwrap()
            .evidence_cards
            .is_empty());
        assert!(crate::search::resolve_evidence_ref(&db, &reference).is_err());
        assert!(crate::research_workspace::get(&db, &set.summary.id)
            .unwrap()
            .documents
            .is_empty());
        for table in [
            "documents",
            "evidence_snapshots",
            "research_documents",
            "research_cells",
        ] {
            let count: i64 = db
                .conn()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "excluded evidence remains in {table}");
        }
    }
    #[tokio::test]
    async fn tool_first_ingestion_survives_scans_with_an_aliased_root_and_legacy_path() {
        let folder = TempDir::new().unwrap();
        let inner = folder.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        let alias = inner.join("..");
        let file = create_test_file(folder.path(), "policy.md", "Policy allowance is 500 yuan.");
        let (db, source) = setup_db_with_source(&alias);
        let args = serde_json::json!({"path":file.to_string_lossy()}).to_string();
        ReindexTool
            .execute(crate::tools::ToolExecutionContext::new(
                "tool-first",
                &args,
                &db,
                std::slice::from_ref(&source),
            ))
            .await
            .unwrap();
        let stored = || {
            db.conn()
                .query_row(
                    "SELECT id,path FROM documents WHERE source_id=?1",
                    [&source],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .unwrap()
        };
        let first = stored();
        ingest::scan_source(&db, &source).unwrap();
        assert_eq!(stored().0, first.0);
        assert_eq!(db.get_index_stats().unwrap().total_documents, 1);
        // Simulate a path stored by the old Windows tool (or a canonical Unix alias).
        db.conn()
            .execute(
                "UPDATE documents SET path=?2 WHERE id=?1",
                rusqlite::params![
                    first.0,
                    std::fs::canonicalize(&file).unwrap().to_string_lossy()
                ],
            )
            .unwrap();
        let reference = crate::search::search(
            &db,
            &crate::models::SearchQuery {
                text: "allowance".into(),
                filters: Default::default(),
                limit: 2,
                offset: 0,
            },
        )
        .unwrap()
        .evidence_cards
        .remove(0)
        .evidence_ref
        .unwrap();
        let scan = ingest::scan_source(&db, &source).unwrap();
        assert_eq!(scan.files_added, 0);
        assert_eq!(scan.files_purged, 0);
        assert_eq!(stored().0, first.0);
        assert_eq!(db.get_index_stats().unwrap().total_documents, 1);
        assert!(crate::search::resolve_evidence_ref(&db, &reference).is_ok());
        std::fs::remove_file(&file).unwrap();
        ingest::scan_source(&db, &source).unwrap();
        assert_eq!(
            crate::search::resolve_evidence_ref(&db, &reference)
                .unwrap()
                .evidence_ref
                .unwrap()
                .status,
            "missing"
        );
    }
}
