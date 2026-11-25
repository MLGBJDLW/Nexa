//! SearchFilesTool - source-scoped text search for local files.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use async_trait::async_trait;
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
use crate::db::Database;
use crate::error::CoreError;

use super::path_utils::{resolve_path_for_file_access, PathKind};
use super::{Tool, ToolCategory, ToolDef, ToolResult};

static DEF: OnceLock<ToolDef> = OnceLock::new();
const DEF_JSON: &str = include_str!("../../prompts/tools/search_files.json");

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const DEFAULT_MAX_RESULTS: usize = 50;
const MAX_RESULTS: usize = 200;
const MAX_CONTEXT_LINES: usize = 3;
const MAX_DISPLAY_CHARS: usize = 300;

#[derive(Deserialize)]
struct SearchFilesArgs {
    query: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    case_sensitive: bool,
    #[serde(default)]
    include_globs: Vec<String>,
    #[serde(default)]
    exclude_globs: Vec<String>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    context_lines: Option<usize>,
    #[serde(default)]
    include_hidden: bool,
}

#[derive(Debug)]
struct SearchRoot {
    root: PathBuf,
    display_base: PathBuf,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextLine {
    line_number: usize,
    text: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FileMatch {
    path: String,
    line_number: usize,
    line: String,
    before: Vec<ContextLine>,
    after: Vec<ContextLine>,
}

enum Matcher {
    Literal { query: String, case_sensitive: bool },
    Regex(regex::Regex),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchMode {
    Literal,
    Regex,
}

impl MatchMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Literal => "literal",
            Self::Regex => "regex",
        }
    }
}

impl Matcher {
    fn new(query: &str, regex: bool, case_sensitive: bool) -> Result<Self, String> {
        if query.trim().is_empty() {
            return Err("search_files query must not be empty.".into());
        }

        if regex {
            RegexBuilder::new(query)
                .case_insensitive(!case_sensitive)
                .build()
                .map(Self::Regex)
                .map_err(|e| {
                    format!(
                        "Invalid regex for search_files: {e}. If you wanted exact text, omit \
                         regex or set \"regex\": false; only escape regex metacharacters such \
                         as | when regex mode is intentional."
                    )
                })
        } else {
            Ok(Self::Literal {
                query: if case_sensitive {
                    query.to_owned()
                } else {
                    query.to_lowercase()
                },
                case_sensitive,
            })
        }
    }

    fn mode(&self) -> MatchMode {
        match self {
            Self::Literal { .. } => MatchMode::Literal,
            Self::Regex(_) => MatchMode::Regex,
        }
    }

    fn is_match(&self, line: &str) -> bool {
        match self {
            Self::Literal {
                query,
                case_sensitive,
            } => {
                if *case_sensitive {
                    line.contains(query)
                } else {
                    line.to_lowercase().contains(query)
                }
            }
            Self::Regex(regex) => regex.is_match(line),
        }
    }
}

pub struct SearchFilesTool;

#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        &ToolDef::from_json(&DEF, DEF_JSON).description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        ToolDef::from_json(&DEF, DEF_JSON).parameters.clone()
    }

    fn categories(&self) -> &'static [ToolCategory] {
        &[ToolCategory::FileSystem]
    }

    async fn execute(
        &self,
        context: crate::tools::ToolExecutionContext<'_>,
    ) -> Result<ToolResult, CoreError> {
        let file_policy = super::file_access_policy_for_context(&context)?;
        let args: SearchFilesArgs = serde_json::from_str(context.arguments)
            .map_err(|e| CoreError::InvalidInput(format!("Invalid search_files arguments: {e}")))?;
        let cancel = context
            .cancel_token
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        // Dropping the future (e.g. on a tool timeout) also stops its blocking walk.
        let _guard = cancel.clone().drop_guard();
        let call_id = context.call_id.to_string();
        tokio::task::spawn_blocking(move || search(call_id, args, file_policy, cancel))
            .await
            .map_err(|e| CoreError::Internal(format!("task join failed: {e}")))?
    }
}

fn search(
    call_id: String,
    args: SearchFilesArgs,
    file_policy: super::FileAccessPolicy,
    cancel: CancellationToken,
) -> Result<ToolResult, CoreError> {
    check_cancelled(&cancel)?;
    let matcher = Matcher::new(&args.query, args.regex, args.case_sensitive)
        .map_err(CoreError::InvalidInput)?;
    let match_mode = matcher.mode().as_str();
    let max_results = args
        .max_results
        .unwrap_or(DEFAULT_MAX_RESULTS)
        .clamp(1, MAX_RESULTS);
    let context_lines = args.context_lines.unwrap_or(0).min(MAX_CONTEXT_LINES);
    let include_set = build_globset(&args.include_globs)?;
    let exclude_set = build_globset(&args.exclude_globs)?;

    let roots = resolve_search_roots(&args, &file_policy)?;
    if roots.is_empty() {
        return Ok(ToolResult {
            call_id,
            content: "No source directories are available to search.".to_string(),
            is_error: true,
            artifacts: None,
        });
    }

    let mut page = MatchPage {
        skip: args.offset,
        limit: max_results,
        matches: Vec::new(),
        has_more: false,
        cancel: &cancel,
    };
    let mut searched_files = 0usize;
    let mut skipped_files = 0usize;

    'roots: for root in roots {
        check_cancelled(&cancel)?;
        if root.root.is_file() {
            if file_included(&root.root, &root.display_base, &include_set, &exclude_set) {
                searched_files += 1;
                search_file(
                    &root.root,
                    &root.display_base,
                    &matcher,
                    context_lines,
                    &mut page,
                )?;
            }
            if page.has_more {
                break;
            }
            continue;
        }

        let mut builder = ignore::WalkBuilder::new(&root.root);
        builder
            .sort_by_file_name(|a, b| a.cmp(b))
            .follow_links(false)
            .hidden(!args.include_hidden)
            .git_ignore(true)
            .git_exclude(true)
            .git_global(true)
            .require_git(false)
            .parents(true);

        for entry in builder.build() {
            check_cancelled(&cancel)?;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    skipped_files += 1;
                    continue;
                }
            };
            if !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            {
                continue;
            }
            let path = entry.path();
            if !file_included(path, &root.display_base, &include_set, &exclude_set) {
                continue;
            }
            searched_files += 1;
            search_file(path, &root.display_base, &matcher, context_lines, &mut page)?;
            if page.has_more {
                break 'roots;
            }
        }
    }

    let matches = page.matches;
    let next_offset = page
        .has_more
        .then(|| args.offset.saturating_add(matches.len()));
    let mut text = format!(
            "Found {} matching line(s) for {:?} using {match_mode} matching across {} searched file(s).",
            matches.len(),
            args.query,
            searched_files
        );
    if let Some(offset) = next_offset {
        text.push_str(&format!(
            " More matches are available. Repeat this search with offset={offset}."
        ));
    }
    if skipped_files > 0 {
        text.push_str(&format!(" Skipped {skipped_files} unreadable path(s)."));
    }
    text.push('\n');

    for item in &matches {
        text.push_str(&format!(
            "\n{}:{}: {}",
            item.path, item.line_number, item.line
        ));
    }

    Ok(ToolResult {
        call_id,
        content: text,
        is_error: false,
        artifacts: Some(json!({
            "kind": "fileSearchResults",
            "query": args.query,
            "regex": args.regex,
            "matchMode": match_mode,
            "caseSensitive": args.case_sensitive,
            "truncated": page.has_more,
            "nextOffset": next_offset,
            "searchedFiles": searched_files,
            "matches": matches,
        })),
    })
}

fn check_cancelled(cancel: &CancellationToken) -> Result<(), CoreError> {
    if cancel.is_cancelled() {
        return Err(CoreError::Cancelled("File search cancelled".into()));
    }
    Ok(())
}

fn resolve_search_roots(
    args: &SearchFilesArgs,
    file_policy: &super::FileAccessPolicy,
) -> Result<Vec<SearchRoot>, CoreError> {
    if let Some(path) = args.path.as_deref() {
        let requested = Path::new(path);
        if file_policy.sources.is_empty()
            && !(file_policy.allow_unregistered_absolute_paths && requested.is_absolute())
        {
            return Err(CoreError::InvalidInput(format!(
                "Access denied: '{path}' is not within any directory available in the current source scope."
            )));
        }
        let resolved = resolve_path_for_file_access(
            requested,
            &file_policy.sources,
            PathKind::Any,
            false,
            file_policy.allow_unregistered_absolute_paths,
        )
        .map_err(CoreError::InvalidInput)?;
        let display_base = if resolved.is_file() {
            resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| resolved.clone())
        } else {
            resolved.clone()
        };
        return Ok(vec![SearchRoot {
            root: resolved,
            display_base,
        }]);
    }

    let mut roots = Vec::new();
    for source in &file_policy.sources {
        if let Ok(root) = std::fs::canonicalize(&source.root_path) {
            roots.push(SearchRoot {
                root: root.clone(),
                display_base: root,
            });
        }
    }
    roots.sort_by(|a, b| a.root.cmp(&b.root));
    Ok(roots)
}

fn build_globset(patterns: &[String]) -> Result<Option<GlobSet>, CoreError> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        add_glob_pattern(&mut builder, pattern)?;
    }
    builder
        .build()
        .map(Some)
        .map_err(|e| CoreError::InvalidInput(format!("Invalid glob pattern: {e}")))
}

fn add_glob_pattern(builder: &mut GlobSetBuilder, pattern: &str) -> Result<(), CoreError> {
    builder.add(
        Glob::new(pattern)
            .map_err(|e| CoreError::InvalidInput(format!("Invalid glob '{pattern}': {e}")))?,
    );
    if !pattern.contains('/') && !pattern.contains('\\') {
        let nested = format!("**/{pattern}");
        builder.add(
            Glob::new(&nested)
                .map_err(|e| CoreError::InvalidInput(format!("Invalid glob '{nested}': {e}")))?,
        );
    }
    Ok(())
}

fn file_included(
    path: &Path,
    base: &Path,
    include_set: &Option<GlobSet>,
    exclude_set: &Option<GlobSet>,
) -> bool {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let rel = normalize_path_for_glob(rel);
    if exclude_set
        .as_ref()
        .is_some_and(|set| set.is_match(rel.as_str()))
    {
        return false;
    }
    include_set
        .as_ref()
        .map(|set| set.is_match(rel.as_str()))
        .unwrap_or(true)
}

fn normalize_path_for_glob(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

struct MatchPage<'a> {
    cancel: &'a CancellationToken,
    skip: usize,
    limit: usize,
    matches: Vec<FileMatch>,
    has_more: bool,
}

fn search_file(
    path: &Path,
    base: &Path,
    matcher: &Matcher,
    context_lines: usize,
    page: &mut MatchPage<'_>,
) -> Result<(), CoreError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(()),
    };
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Ok(());
    }

    let Ok(file) = std::fs::File::open(path) else {
        return Ok(());
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_FILE_BYTES
    {
        return Ok(());
    }
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        return Ok(());
    }
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(_) => return Ok(()),
    };

    let lines: Vec<&str> = content.lines().collect();
    let display_path = normalize_path_for_glob(path.strip_prefix(base).unwrap_or(path));

    for (idx, line) in lines.iter().enumerate() {
        if idx % 1024 == 0 {
            check_cancelled(page.cancel)?;
        }
        if !matcher.is_match(line) {
            continue;
        }
        if page.skip > 0 {
            page.skip -= 1;
            continue;
        }
        if page.matches.len() == page.limit {
            page.has_more = true;
            break;
        }

        let before_start = idx.saturating_sub(context_lines);
        let before = lines[before_start..idx]
            .iter()
            .enumerate()
            .map(|(offset, text)| ContextLine {
                line_number: before_start + offset + 1,
                text: truncate_line(text),
            })
            .collect();
        let after_end = (idx + 1 + context_lines).min(lines.len());
        let after = lines[idx + 1..after_end]
            .iter()
            .enumerate()
            .map(|(offset, text)| ContextLine {
                line_number: idx + offset + 2,
                text: truncate_line(text),
            })
            .collect();

        page.matches.push(FileMatch {
            path: display_path.clone(),
            line_number: idx + 1,
            line: truncate_line(line),
            before,
            after,
        });
    }

    Ok(())
}

fn truncate_line(line: &str) -> String {
    let mut out = line.chars().take(MAX_DISPLAY_CHARS).collect::<String>();
    if line.chars().count() > MAX_DISPLAY_CHARS {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::CreateSourceInput;

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

    #[tokio::test]
    async fn search_files_finds_literal_matches_with_glob_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "Alpha plan\nBeta plan\n").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "Alpha scratch\n").unwrap();

        let db = setup_db_with_source(dir.path());
        let tool = SearchFilesTool;
        let args = serde_json::json!({
            "query": "alpha",
            "include_globs": ["*.md"],
            "max_results": 10
        });

        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "search-1",
                &args.to_string(),
                &db,
                &[],
            ))
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("notes.md:1"));
        assert!(!result.content.contains("notes.txt"));
        assert_eq!(
            result.artifacts.as_ref().unwrap()["matches"][0]["path"],
            "notes.md"
        );
    }

    #[tokio::test]
    async fn search_files_supports_regex_and_context() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("journal.md"),
            "before\nTicket-123 resolved\nafter\n",
        )
        .unwrap();

        let db = setup_db_with_source(dir.path());
        let tool = SearchFilesTool;
        let args = serde_json::json!({
            "query": "ticket-\\d+",
            "regex": true,
            "context_lines": 1
        });

        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "search-2",
                &args.to_string(),
                &db,
                &[],
            ))
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.content);
        let artifact = result.artifacts.as_ref().unwrap();
        assert_eq!(artifact["matches"][0]["lineNumber"], 2);
        assert_eq!(artifact["matches"][0]["before"][0]["text"], "before");
        assert_eq!(artifact["matches"][0]["after"][0]["text"], "after");
    }

    #[tokio::test]
    async fn search_files_treats_pipe_as_literal_by_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("query.txt"),
            "alpha|beta exact\nalpha only\nbeta only\n",
        )
        .unwrap();

        let db = setup_db_with_source(dir.path());
        let tool = SearchFilesTool;
        let args = serde_json::json!({
            "query": "alpha|beta",
            "max_results": 10
        });

        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "grep-1",
                &args.to_string(),
                &db,
                &[],
            ))
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("using literal matching"));
        assert!(result.content.contains("query.txt:1"));
        assert!(!result.content.contains("query.txt:2"));
        assert!(!result.content.contains("query.txt:3"));
        let artifact = result.artifacts.as_ref().unwrap();
        assert_eq!(artifact["matchMode"], "literal");
        assert_eq!(artifact["matches"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn search_files_supports_regex_alternation_when_opted_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("query.txt"),
            "alpha item\nbeta item\ngamma item\n",
        )
        .unwrap();

        let db = setup_db_with_source(dir.path());
        let tool = SearchFilesTool;
        let args = serde_json::json!({
            "query": "alpha|beta",
            "regex": true,
            "max_results": 10
        });

        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "grep-2",
                &args.to_string(),
                &db,
                &[],
            ))
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("using regex matching"));
        assert!(result.content.contains("query.txt:1"));
        assert!(result.content.contains("query.txt:2"));
        assert!(!result.content.contains("query.txt:3"));
        let artifact = result.artifacts.as_ref().unwrap();
        assert_eq!(artifact["matchMode"], "regex");
        assert_eq!(artifact["matches"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn legacy_search_calls_page_matches_in_stable_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), "x").unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let db = setup_db_with_source(dir.path());
        let registry = super::super::default_tool_registry().filtered(&["grep_files".into()]);
        for (offset, name, path, next) in [
            (0, "grep_files", "a.txt", Some(1)),
            (1, "search_files", "b.txt", None),
        ] {
            let args = json!({"query": "x", "offset": offset, "max_results": 1}).to_string();
            let result = registry
                .execute(
                    name,
                    crate::tools::ToolExecutionContext::new("page", &args, &db, &[]),
                )
                .await
                .unwrap();
            let evidence = result.artifacts.unwrap();
            assert_eq!(evidence["matches"][0]["path"], path);
            assert_eq!(evidence["nextOffset"], json!(next));
            assert_eq!(evidence["truncated"], next.is_some());
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut context =
            crate::tools::ToolExecutionContext::new("stop", r#"{"query":"x"}"#, &db, &[]);
        context.cancel_token = Some(&cancel);
        assert!(matches!(
            SearchFilesTool.execute(context).await,
            Err(CoreError::Cancelled(_))
        ));
    }

    #[tokio::test]
    async fn search_files_invalid_regex_error_points_to_literal_mode() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("query.txt"), "alpha item\n").unwrap();

        let db = setup_db_with_source(dir.path());
        let tool = SearchFilesTool;
        let args = serde_json::json!({
            "query": "(",
            "regex": true
        });

        let err = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "grep-3",
                &args.to_string(),
                &db,
                &[],
            ))
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("search_files"), "{err}");
        assert!(err.contains("Invalid regex"), "{err}");
        assert!(err.contains("\"regex\": false"), "{err}");
    }
}
