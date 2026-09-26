//! Read-only Git projections. Commands never invoke a shell or external diff driver.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures::{stream, StreamExt};
use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const MAX_OUTPUT: u64 = 2 * 1024 * 1024;
const MAX_FILES: usize = 300;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileStatus {
    pub path: String,
    pub index: char,
    pub worktree: char,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorkspaceStatus {
    pub root: String,
    pub branch: String,
    pub oid: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<GitFileStatus>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationGitStatus {
    pub source_id: String,
    #[serde(flatten)]
    pub status: GitWorkspaceStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorkspaceIssue {
    pub source_id: String,
    pub root: String,
    pub message: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorkspaceSnapshot {
    pub repos: Vec<ConversationGitStatus>,
    pub issues: Vec<GitWorkspaceIssue>,
    pub checked_sources: usize,
}

/// An empty conversation scope has the same meaning here as in file tools:
/// all registered sources. Explicit conversation/project links still win.
pub fn conversation_sources(
    db: &crate::db::Database,
    conversation_id: &str,
) -> Result<Vec<(String, PathBuf)>, crate::error::CoreError> {
    let ids = db.get_effective_conversation_source_scope(conversation_id)?;
    let sources = if ids.is_empty() {
        db.list_sources()?
    } else {
        ids.iter()
            .map(|id| db.get_source(id))
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(sources
        .into_iter()
        .map(|source| (source.id, PathBuf::from(source.root_path)))
        .collect())
}

/// A broken source cannot hide healthy repositories. Bound process fan-out and
/// canonicalize scopes once to avoid polling the same directory twice.
pub async fn snapshot(roots: Vec<(String, PathBuf)>) -> GitWorkspaceSnapshot {
    let mut snapshot = GitWorkspaceSnapshot::default();
    let mut scopes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (id, root) in roots {
        match tokio::fs::canonicalize(&root).await {
            Ok(root) if seen.insert(root.clone()) => scopes.push((id, root)),
            Ok(_) => {}
            Err(error) => snapshot.issues.push(GitWorkspaceIssue {
                source_id: id,
                root: root.display().to_string(),
                message: error.to_string(),
            }),
        }
    }
    snapshot.checked_sources = scopes.len() + snapshot.issues.len();
    let results = stream::iter(scopes.into_iter().map(|(source_id, root)| async move {
        let result = match tokio::fs::metadata(&root).await {
            Ok(metadata) if metadata.is_dir() => status(&root).await,
            Ok(_) => Ok(None),
            Err(error) => Err(error.to_string()),
        };
        (source_id, root, result)
    }))
    .buffered(4)
    .collect::<Vec<_>>()
    .await;
    for (source_id, root, result) in results {
        match result {
            Ok(Some(status)) => snapshot
                .repos
                .push(ConversationGitStatus { source_id, status }),
            Ok(None) => {}
            Err(message) => snapshot.issues.push(GitWorkspaceIssue {
                source_id,
                root: root.display().to_string(),
                message,
            }),
        }
    }
    snapshot
}

async fn output(cwd: &Path, args: &[&str]) -> Result<Option<String>, String> {
    let mut command = Command::new("git");
    command
        .current_dir(cwd)
        .args([
            "--no-optional-locks",
            "-c",
            "core.quotepath=false",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("Git executable was not found in the desktop host's PATH".into());
        }
        Err(error) => return Err(error.to_string()),
    };
    let stdout = child.stdout.take().ok_or("Git output pipe unavailable")?;
    let stderr = child.stderr.take().ok_or("Git error pipe unavailable")?;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        let mut error_bytes = Vec::new();
        let mut stdout = stdout.take(MAX_OUTPUT + 1);
        let mut stderr = stderr.take(32 * 1024);
        tokio::try_join!(
            stdout.read_to_end(&mut bytes),
            stderr.read_to_end(&mut error_bytes)
        )
        .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_OUTPUT {
            return Err("Git output exceeds the 2 MiB preview limit".into());
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if !status.success() {
            let message = String::from_utf8_lossy(&error_bytes);
            let repository_marker = cwd
                .ancestors()
                .any(|dir| dir.join(".git").symlink_metadata().is_ok());
            if args.first() == Some(&"rev-parse")
                && message.contains("not a git repository")
                && !repository_marker
            {
                return Ok(None);
            }
            return Err(format!(
                "Git command failed ({status}): {}",
                message.chars().take(800).collect::<String>().trim()
            ));
        }
        // Never turn an undecodable filename into a different actionable path.
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "Git returned non-UTF-8 paths or content".into())
    })
    .await
    .map_err(|_| "Git operation timed out".to_string())?;
    result
}

fn parse_status(root: String, raw: &str) -> GitWorkspaceStatus {
    let mut status = GitWorkspaceStatus {
        root,
        branch: String::new(),
        oid: String::new(),
        upstream: None,
        ahead: 0,
        behind: 0,
        files: Vec::new(),
        truncated: false,
    };
    let mut records = raw.split('\0');
    while let Some(record) = records.next() {
        if let Some(value) = record.strip_prefix("# branch.head ") {
            status.branch = value.into();
        } else if let Some(value) = record.strip_prefix("# branch.oid ") {
            status.oid = value.into();
        } else if let Some(value) = record.strip_prefix("# branch.upstream ") {
            status.upstream = Some(value.into());
        } else if let Some(value) = record.strip_prefix("# branch.ab ") {
            let mut counts = value.split_whitespace();
            status.ahead = counts
                .next()
                .and_then(|v| v.trim_start_matches('+').parse().ok())
                .unwrap_or(0);
            status.behind = counts
                .next()
                .and_then(|v| v.trim_start_matches('-').parse().ok())
                .unwrap_or(0);
        } else {
            let parsed = if let Some(path) = record.strip_prefix("? ") {
                Some((path, '?', '?'))
            } else {
                let fields = match record.as_bytes().first() {
                    Some(b'1') => 9,
                    Some(b'2') => {
                        records.next();
                        10
                    } // Original rename path is a separate NUL record.
                    Some(b'u') => 11,
                    _ => continue,
                };
                let parts: Vec<_> = record.splitn(fields, ' ').collect();
                parts.get(1).and_then(|xy| {
                    let mut chars = xy.chars();
                    Some((*parts.get(fields - 1)?, chars.next()?, chars.next()?))
                })
            };
            if let Some((path, index, worktree)) = parsed {
                if status.files.len() < MAX_FILES {
                    status.files.push(GitFileStatus {
                        path: path.into(),
                        index,
                        worktree,
                    });
                } else {
                    status.truncated = true;
                }
            }
        }
    }
    status
}

pub async fn status(scope: &Path) -> Result<Option<GitWorkspaceStatus>, String> {
    let Some(root) = output(scope, &["rev-parse", "--show-toplevel"]).await? else {
        return Ok(None);
    };
    let Some(raw) = output(
        scope,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=normal",
            "--",
            ".",
        ],
    )
    .await?
    else {
        return Ok(None);
    };
    Ok(Some(parse_status(
        root.trim_end_matches(['\r', '\n']).into(),
        &raw,
    )))
}

pub async fn diff(scope: &Path, path: &str, staged: bool) -> Result<String, String> {
    let status = status(scope).await?.ok_or("Git repository unavailable")?;
    let file = status
        .files
        .iter()
        .find(|file| file.path == path)
        .ok_or("File is no longer changed in this source scope")?;
    if file.index == '?' {
        return Err("Untracked files have no Git diff".into());
    }
    let literal = format!(":(literal){path}");
    let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--no-color"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", &literal]);
    output(&PathBuf::from(status.root), &args)
        .await?
        .ok_or("Git diff unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as SyncCommand;

    #[test]
    fn conversation_git_sources_match_file_tool_scope() {
        use crate::{
            conversation::CreateConversationInput, project::CreateProjectInput,
            sources::CreateSourceInput,
        };
        let db = crate::db::Database::open_memory().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let add = |dir: &Path| {
            db.add_source(CreateSourceInput {
                root_path: dir.display().to_string(),
                include_globs: vec![],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap()
        };
        let a = add(a.path());
        let b = add(b.path());
        let create = |project_id| {
            db.create_conversation(&CreateConversationInput {
                provider: "openai".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id,
                persona_id: None,
            })
            .unwrap()
        };
        let conversation = create(None);
        assert_eq!(
            conversation_sources(&db, &conversation.id).unwrap().len(),
            2
        );
        db.set_conversation_sources(&conversation.id, std::slice::from_ref(&a.id))
            .unwrap();
        assert_eq!(
            conversation_sources(&db, &conversation.id).unwrap()[0].0,
            a.id
        );
        let project = db
            .create_project(&CreateProjectInput {
                name: "scoped".into(),
                description: None,
                icon: None,
                color: None,
                system_prompt: None,
                source_scope: Some(vec![b.id.clone()]),
            })
            .unwrap();
        let scoped = create(Some(project.id));
        let roots = conversation_sources(&db, &scoped.id).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].0, b.id);
        assert!(conversation_sources(&db, "missing-conversation").is_err());
    }

    #[tokio::test]
    async fn snapshot_keeps_healthy_repositories_and_deduplicates_scopes() {
        let repo = tempfile::tempdir().unwrap();
        let plain = tempfile::tempdir().unwrap();
        assert!(SyncCommand::new("git")
            .arg("init")
            .current_dir(repo.path())
            .output()
            .unwrap()
            .status
            .success());
        let result = snapshot(vec![
            ("repo".into(), repo.path().into()),
            ("duplicate".into(), repo.path().join(".")),
            ("plain".into(), plain.path().into()),
            ("gone".into(), plain.path().join("missing")),
        ])
        .await;
        assert_eq!(result.checked_sources, 3);
        assert_eq!(result.repos.len(), 1);
        assert_eq!(result.repos[0].source_id, "repo");
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].source_id, "gone");
        assert!(snapshot(vec![]).await.repos.is_empty());
    }

    #[test]
    fn parses_unicode_renames_and_branch_tracking() {
        let raw = "# branch.head feature/demo\0# branch.oid abc\0# branch.upstream origin/main\0# branch.ab +2 -1\0? 中文 name.txt\x002 R. N... 100644 100644 100644 a b R100 new name.txt\0old name.txt\0";
        let parsed = parse_status("root".into(), raw);
        assert_eq!((parsed.ahead, parsed.behind), (2, 1));
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].path, "中文 name.txt");
        assert_eq!(parsed.files[1].path, "new name.txt");
    }

    #[tokio::test]
    async fn real_repository_honors_scope_and_staged_diff() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            SyncCommand::new("git")
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap()
        };
        assert!(status(dir.path()).await.unwrap().is_none());
        assert!(git(&["init"]).status.success());
        std::fs::create_dir(dir.path().join("scope")).unwrap();
        std::fs::write(dir.path().join("scope/中文.txt"), "hello\n").unwrap();
        std::fs::write(dir.path().join("outside.txt"), "secret\n").unwrap();
        assert!(git(&["add", "."]).status.success());
        let scope = dir.path().join("scope");
        let snapshot = status(&scope).await.unwrap().unwrap();
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].path, "scope/中文.txt");
        assert!(diff(&scope, "scope/中文.txt", true)
            .await
            .unwrap()
            .contains("+hello"));
        assert!(diff(&scope, "outside.txt", true).await.is_err());
        assert!(diff(&scope, ":(glob)**", true).await.is_err());
        std::fs::write(dir.path().join(".git/index"), "corrupt index").unwrap();
        assert!(status(&scope)
            .await
            .unwrap_err()
            .contains("Git command failed"));
    }
}
