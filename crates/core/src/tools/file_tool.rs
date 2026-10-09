//! FileTool — reads files from managed source directories.

#[cfg(test)]
use crate::db::Database;

use std::path::PathBuf;
use std::sync::OnceLock;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::CoreError;
use crate::privacy;

use super::document_utils::{document_attachment, read_file_evidence};
use super::path_utils::resolve_existing_file_for_file_access;
use super::{Tool, ToolCategory, ToolDef, ToolResult};

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/read_file.json");

/// Tool that reads a file from the knowledge base, validating that it
/// belongs to a registered source root and optionally applying privacy
/// redaction.
pub struct FileTool;

#[derive(Deserialize)]
struct FileArgs {
    path: String,
    #[serde(default = "default_start_line")]
    start_line: usize,
    #[serde(default = "default_max_lines")]
    max_lines: usize,
}

fn default_start_line() -> usize {
    1
}

fn default_max_lines() -> usize {
    100
}

#[async_trait]
impl Tool for FileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }

    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::Core, ToolCategory::FileSystem]
    }

    async fn execute(
        &self,
        context: crate::tools::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        let value: serde_json::Value = serde_json::from_str(context.arguments)
            .map_err(|e| CoreError::InvalidInput(format!("Invalid read_file arguments: {e}")))?;
        if let Some(error) =
            super::file_tool_contract::argument_error(self.name(), context.call_id, &value)
        {
            return Ok(error);
        }
        let native_candidate = !value
            .as_object()
            .is_some_and(|args| args.contains_key("start_line") || args.contains_key("max_lines"));
        let file_policy = super::file_access_policy_for_context(&context)?;
        let crate::tools::ToolExecutionContext {
            call_id,
            arguments,
            db,
            ..
        } = context;
        let args: FileArgs = serde_json::from_str(arguments)
            .map_err(|e| CoreError::InvalidInput(format!("Invalid read_file arguments: {e}")))?;

        let db = db.clone();
        let call_id = call_id.to_string();
        tokio::task::spawn_blocking(move || {
            let requested = PathBuf::from(&args.path);


            if file_policy.sources.is_empty()
                && !(file_policy.allow_unregistered_absolute_paths && requested.is_absolute())
            {
                return Ok(ToolResult {
                    call_id: call_id.clone(),
                    content: format!(
                        "Access denied: '{}' is not within any directory available in the current source scope.",
                        args.path
                    ),
                    is_error: true,
                    artifacts: None,
                });
            }
            let canonical = resolve_existing_file_for_file_access(
                &requested,
                &file_policy.sources,
                file_policy.allow_unregistered_absolute_paths,
            )
            .map_err(CoreError::InvalidInput)?;

            // Read text files directly; for supported binary docs, parse and extract text.
            let privacy_config = db.load_privacy_config()?;
            let (raw, document) = read_file_evidence(&canonical, native_candidate && !privacy_config.enabled)?;

            // Skip to start_line (1-based) and truncate to max_lines.
            let start = args.start_line.max(1);
            let max = args.max_lines.max(1);
            let total_lines = raw.lines().count();
            let lines: Vec<&str> = raw.lines().skip(start - 1).take(max).collect();
            let showing_end = (start - 1 + lines.len()).min(total_lines);
            let truncated = showing_end < total_lines || start > 1;
            let content = lines.join("\n");
            let canonical_str = canonical.to_string_lossy().to_string();
            let document_info: Option<(String, Option<String>)> = db
                .conn()
                .query_row(
                    "SELECT id, title FROM documents WHERE path = ?1",
                    rusqlite::params![&canonical_str],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();
            let file_label = canonical
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("file");
            let suggested_citation = match &document_info {
                Some((document_id, _)) => format!("[doc:{document_id}|{file_label}]"),
                None => format!("[file:{canonical_str}:{start}-{showing_end}|{file_label}]"),
            };

            // Apply privacy redaction.
            let redacted = if privacy_config.enabled {
                privacy::redact_content(&content, &privacy_config.redact_patterns)
            } else {
                content
            };

            let mut text = format!("File: {}\n", canonical.display());
            if let Some((document_id, title)) = &document_info {
                let title_display = title.as_deref().unwrap_or("(untitled)");
                text.push_str(&format!("Document ID: {document_id}\n"));
                text.push_str(&format!("Title: {title_display}\n"));
            }
            text.push_str(&format!("Suggested citation: {suggested_citation}\n"));
            if truncated {
                text.push_str(&format!(
                    "(showing lines {start}–{showing_end} of {total_lines})\n"
                ));
            }
            text.push_str("---\n");
            text.push_str(&redacted);

            let mut result = ToolResult {
                call_id,
                content: text.clone(),
                is_error: false,
                artifacts: Some(serde_json::json!({
                    "path": canonical_str,
                    "documentId": document_info.as_ref().map(|(id, _)| id.clone()),
                    "documentTitle": document_info.as_ref().and_then(|(_, title)| title.clone()),
                    "lineStart": start,
                    "lineEnd": showing_end,
                    "totalLines": total_lines,
                    "suggestedCitation": suggested_citation,
                })),
            };
            if let Some(document) = document {
                let attachment = document_attachment(document, text);
                result.artifacts.as_mut().unwrap()["toolOutput"] = serde_json::json!({
                    "llmContent": result.content, "displayContent": result.content,
                    "attachments": [attachment],
                });
            }
            Ok(result)
        })
        .await
        .map_err(|e| CoreError::Internal(format!("task join failed: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_settings::{AppConfig, ShellAccessMode};
    use crate::sources::CreateSourceInput;
    use std::io::Write;
    use std::path::Path;

    fn setup_db_with_source(root: &Path) -> Database {
        let db = Database::open_memory().expect("open in-memory db");
        db.add_source(CreateSourceInput {
            root_path: root.to_string_lossy().to_string(),
            include_globs: vec![],
            exclude_globs: vec![],
            watch_enabled: false,
        })
        .expect("register source root");
        db
    }

    fn enable_open_file_access(db: &Database) {
        let mut config = AppConfig::default();
        config.shell_access_mode = ShellAccessMode::Open;
        db.save_app_config(&config).expect("save app config");
    }

    #[tokio::test]
    async fn read_file_falls_back_to_document_parser_for_binary_images() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let image_path = dir.path().join("diagram.png");
        std::fs::write(&image_path, [0_u8, 159, 1, 2, 3]).expect("write binary image bytes");

        let db = setup_db_with_source(dir.path());
        let tool = FileTool;
        let args = serde_json::json!({
            "path": image_path.to_string_lossy().to_string()
        })
        .to_string();

        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "call-1",
                &args,
                &db,
                &[],
            ))
            .await
            .expect("read_file should fallback for image");

        assert!(!result.is_error);
        assert!(
            result.content.contains("[Image: diagram.png]"),
            "unexpected content: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn read_file_keeps_binary_error_for_unsupported_types() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let bin_path = dir.path().join("payload.bin");
        std::fs::write(&bin_path, [0_u8, 1, 2, 3]).expect("write binary payload");

        let db = setup_db_with_source(dir.path());
        let tool = FileTool;
        let args = serde_json::json!({
            "path": bin_path.to_string_lossy().to_string()
        })
        .to_string();

        let err = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "call-2",
                &args,
                &db,
                &[],
            ))
            .await
            .expect_err("unsupported binary should still error");

        match err {
            CoreError::Parse(msg) => {
                assert!(msg.contains("File appears to be binary"), "msg was: {msg}");
            }
            other => panic!("expected parse error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_file_returns_suggested_document_citation_when_indexed() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let file_path = dir.path().join("notes.md");
        std::fs::write(&file_path, "# Notes\nhello world\n").expect("write text file");
        let canonical_path = std::fs::canonicalize(&file_path).unwrap();

        let db = setup_db_with_source(dir.path());
        db.conn()
            .execute(
                "INSERT INTO documents (id, source_id, path, title, mime_type, file_size, modified_at, content_hash)
                 VALUES (?1, (SELECT id FROM sources LIMIT 1), ?2, 'Notes', 'text/markdown', 20, '2025-01-01 00:00:00', 'hash-notes')",
                rusqlite::params!["doc-1", canonical_path.to_string_lossy().to_string()],
            )
            .unwrap();

        let tool = FileTool;
        let args = serde_json::json!({
            "path": file_path.to_string_lossy().to_string()
        })
        .to_string();

        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "call-3",
                &args,
                &db,
                &[],
            ))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Document ID: doc-1"));
        assert!(result
            .content
            .contains("Suggested citation: [doc:doc-1|notes.md]"));
        assert_eq!(result.artifacts.unwrap()["documentId"], "doc-1");
    }

    #[tokio::test]
    async fn read_file_respects_source_scope() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let file_path = dir.path().join("notes.md");
        std::fs::write(&file_path, "hello world\n").expect("write text file");

        let db = setup_db_with_source(dir.path());
        let tool = FileTool;
        let args = serde_json::json!({
            "path": file_path.to_string_lossy().to_string()
        })
        .to_string();

        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "call-4",
                &args,
                &db,
                &["out-of-scope".to_string()],
            ))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("current source scope"));
    }

    #[tokio::test]
    async fn read_file_open_mode_allows_absolute_path_outside_source() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let other_dir = tempfile::tempdir().expect("create other tempdir");
        let file_path = other_dir.path().join("outside.md");
        std::fs::write(&file_path, "hello from outside\n").expect("write text file");

        let db = setup_db_with_source(dir.path());
        enable_open_file_access(&db);
        let tool = FileTool;
        let args = serde_json::json!({
            "path": file_path.to_string_lossy().to_string()
        })
        .to_string();

        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "call-open",
                &args,
                &db,
                &[],
            ))
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("hello from outside"));
    }

    #[tokio::test]
    async fn read_file_resolves_source_relative_docx_paths() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let docs_dir = dir.path().join("docs");
        std::fs::create_dir_all(&docs_dir).expect("create docs dir");
        let docx_path = docs_dir.join("status.docx");
        let file = std::fs::File::create(&docx_path).expect("create docx");
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("[Content_Types].xml", options)
            .expect("content types");
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        )
        .expect("write content types");
        zip.add_directory("_rels/", options).expect("rels dir");
        zip.start_file("_rels/.rels", options).expect("rels file");
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        )
        .expect("write rels");
        zip.add_directory("word/", options).expect("word dir");
        zip.start_file("word/document.xml", options)
            .expect("document file");
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>DOCX fallback works</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .expect("write document");
        zip.finish().expect("finish docx");

        let db = setup_db_with_source(dir.path());
        let tool = FileTool;
        let args = serde_json::json!({
            "path": "docs/status.docx"
        })
        .to_string();

        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "call-docx",
                &args,
                &db,
                &[],
            ))
            .await
            .expect("relative docx read should succeed");

        assert!(!result.is_error);
        assert!(
            result.content.contains("DOCX fallback works"),
            "unexpected content: {}",
            result.content
        );
    }
}

#[cfg(test)]
mod native_document_tests {
    use super::*;
    use crate::llm::document::{tests::office_bytes, DOCX};
    use crate::sources::CreateSourceInput;

    #[tokio::test]
    async fn native_document_read_is_authorized_ephemeral_and_disabled_for_ranges_and_privacy() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.docx");
        std::fs::write(&file, office_bytes(DOCX)).unwrap();
        let db = Database::open_memory().unwrap();
        let mut privacy = db.load_privacy_config().unwrap();
        privacy.enabled = false;
        db.save_privacy_config(&privacy).unwrap();
        db.add_source(CreateSourceInput {
            root_path: dir.path().to_string_lossy().into(),
            include_globs: vec![],
            exclude_globs: vec![],
            watch_enabled: false,
        })
        .unwrap();
        let args = serde_json::json!({"path":file}).to_string();
        let result = FileTool
            .execute(crate::tools::ToolExecutionContext::new(
                "doc",
                &args,
                &db,
                &[],
            ))
            .await
            .unwrap();
        assert!(!result.is_error);
        let attachments = result.artifacts.as_ref().unwrap()["toolOutput"]["attachments"]
            .as_array()
            .unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0]["mimeType"], DOCX);
        assert!(attachments[0]["data"]["fallbackText"]
            .as_str()
            .unwrap()
            .contains("File:"));
        assert!(!result
            .content
            .contains(attachments[0]["data"]["base64"].as_str().unwrap()));
        for range in [
            serde_json::json!({"path":file,"max_lines":1}),
            serde_json::json!({"path":file,"start_line":1}),
        ] {
            let range = range.to_string();
            let result = FileTool
                .execute(crate::tools::ToolExecutionContext::new(
                    "range",
                    &range,
                    &db,
                    &[],
                ))
                .await
                .unwrap();
            assert!(result.artifacts.unwrap().get("toolOutput").is_none());
        }
        let mut privacy = db.load_privacy_config().unwrap();
        privacy.enabled = true;
        db.save_privacy_config(&privacy).unwrap();
        let result = FileTool
            .execute(crate::tools::ToolExecutionContext::new(
                "private",
                &args,
                &db,
                &[],
            ))
            .await
            .unwrap();
        assert!(result.artifacts.unwrap().get("toolOutput").is_none());
        let denied = Database::open_memory().unwrap();
        let result = FileTool
            .execute(crate::tools::ToolExecutionContext::new(
                "denied",
                &args,
                &denied,
                &[],
            ))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.artifacts.is_none());
    }

    #[tokio::test]
    async fn native_document_batch_preserves_each_file_and_range_fallback() {
        use crate::tools::read_files_tool::ReadFilesTool;
        let dir = tempfile::tempdir().unwrap();
        let doc = dir.path().join("report.docx");
        let text = dir.path().join("notes.txt");
        std::fs::write(&doc, office_bytes(DOCX)).unwrap();
        std::fs::write(&text, "plain notes").unwrap();
        let db = Database::open_memory().unwrap();
        let mut privacy = db.load_privacy_config().unwrap();
        privacy.enabled = false;
        db.save_privacy_config(&privacy).unwrap();
        db.add_source(CreateSourceInput {
            root_path: dir.path().to_string_lossy().into(),
            include_globs: vec![],
            exclude_globs: vec![],
            watch_enabled: false,
        })
        .unwrap();
        for ranged in [false, true] {
            let mut args = serde_json::json!({"paths":[doc,text]});
            if ranged {
                args["max_lines_per_file"] = 1.into();
            }
            let args = args.to_string();
            let result = ReadFilesTool
                .execute(crate::tools::ToolExecutionContext::new(
                    "batch",
                    &args,
                    &db,
                    &[],
                ))
                .await
                .unwrap();
            assert!(!result.is_error);
            assert!(result.content.contains("plain notes"));
            let artifacts = result.artifacts.unwrap();
            assert_eq!(artifacts["files"].as_array().unwrap().len(), 2);
            if ranged {
                assert!(artifacts.get("toolOutput").is_none());
            } else {
                assert_eq!(
                    artifacts["toolOutput"]["attachments"]
                        .as_array()
                        .unwrap()
                        .len(),
                    1
                );
            }
        }
    }
}
