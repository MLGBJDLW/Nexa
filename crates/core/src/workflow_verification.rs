//! Host-owned source identity for verification and review receipts.
use crate::{db::Database, error::CoreError, workspace::Workspace};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

mod cache;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Fresh inventory/metadata with unchanged file content reused.
    Incremental,
    /// Fresh bytes for check/review receipts, resume and completion boundaries.
    Authoritative,
}

#[derive(Clone, Default)]
pub struct Capture {
    files: Arc<Mutex<cache::FileHashes>>,
    cancellation: CancellationToken,
}

impl Capture {
    pub fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            ..Self::default()
        }
    }

    pub async fn capture(
        &self,
        db: &Database,
        conversation: Option<&str>,
        turn: Option<&str>,
        source_scope: &[String],
        workspace: Option<&Workspace>,
        mode: CaptureMode,
    ) -> Result<VerificationSourceState, CoreError> {
        let db = db.clone();
        let conversation = conversation.map(str::to_owned);
        let turn = turn.map(str::to_owned);
        let source_scope = source_scope.to_vec();
        let workspace = workspace.cloned();
        let files = self.files.clone();
        let cancel = self.cancellation.child_token();
        // Dropping the async capture stops its blocking scan at the next entry
        // or content chunk; it cannot leave an abandoned full-workspace reader.
        let _cancel_on_drop = cancel.clone().drop_guard();
        tokio::task::spawn_blocking(move || {
            let mut files = files.lock().unwrap_or_else(|error| error.into_inner());
            capture_sync(
                &db,
                conversation.as_deref(),
                turn.as_deref(),
                &source_scope,
                workspace.as_ref(),
                &mut Scan {
                    files: &mut files,
                    mode,
                    cancel: &cancel,
                },
            )
        })
        .await
        .map_err(|error| CoreError::Internal(format!("Verification capture failed: {error}")))?
    }
}

struct Scan<'a> {
    files: &'a mut cache::FileHashes,
    mode: CaptureMode,
    cancel: &'a CancellationToken,
}

fn check_cancelled(cancel: &CancellationToken) -> Result<(), CoreError> {
    if cancel.is_cancelled() {
        Err(CoreError::Cancelled("Verification source capture".into()))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSourceState {
    pub digest: String,
    pub pending: bool,
}

pub async fn capture(
    db: &Database,
    conversation: Option<&str>,
    turn: Option<&str>,
    source_scope: &[String],
    workspace: Option<&Workspace>,
) -> Result<VerificationSourceState, CoreError> {
    Capture::default()
        .capture(
            db,
            conversation,
            turn,
            source_scope,
            workspace,
            CaptureMode::Authoritative,
        )
        .await
}

fn capture_sync(
    db: &Database,
    conversation: Option<&str>,
    turn: Option<&str>,
    source_scope: &[String],
    workspace: Option<&Workspace>,
    scan: &mut Scan<'_>,
) -> Result<VerificationSourceState, CoreError> {
    check_cancelled(scan.cancel)?;
    let mut roots = BTreeSet::new();
    for id in source_scope {
        roots.insert(PathBuf::from(db.get_source(id)?.root_path));
    }
    if let Some(workspace) = workspace {
        roots.extend(workspace.roots.iter().map(PathBuf::from));
    }
    let mut paths = BTreeSet::new();
    let mut hash = blake3::Hasher::new();
    hash.update(b"verification-source-v2");
    for root in roots {
        collect_root(&root, &mut hash, &mut paths, scan.cancel)?;
    }
    let mut pending = false;
    if let (Some(conversation), Some(turn)) = (conversation, turn) {
        let connection = db.conn();
        let mut statement = connection.prepare("SELECT absolute_path, MAX(pending), MAX(partial) FROM turn_file_change_events WHERE conversation_id=?1 AND turn_id=?2 GROUP BY absolute_path")?;
        let entries = statement.query_map(rusqlite::params![conversation, turn], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        for entry in entries {
            let (path, unsettled, partial) = entry?;
            pending |= unsettled;
            if let Some(path) = path {
                paths.insert(PathBuf::from(path));
            } else if partial {
                hash.update(b"unreconciled-file-effects");
            }
        }
    }
    let mut visited_directories = BTreeSet::new();
    let mut file_paths = BTreeSet::new();
    while let Some(path) = paths.pop_first() {
        check_cancelled(scan.cancel)?;
        hash.update(path.to_string_lossy().as_bytes());
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hash.update(b"deleted");
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.file_type().is_symlink() => {
                hash.update(std::fs::read_link(&path)?.to_string_lossy().as_bytes());
            }
            Ok(metadata) if metadata.is_file() => {
                let (digest, stable) = scan.files.read(&path, &metadata, scan.mode, scan.cancel)?;
                hash.update(digest.as_bytes());
                pending |= !stable;
                file_paths.insert(path);
            }
            Ok(_) => {
                let canonical = path.canonicalize()?;
                if visited_directories.insert(canonical) {
                    collect_root(&path, &mut hash, &mut paths, scan.cancel)?;
                }
            }
        }
    }
    check_cancelled(scan.cancel)?;
    scan.files.retain(&file_paths);
    Ok(VerificationSourceState {
        digest: hash.finalize().to_hex().to_string(),
        pending,
    })
}

fn collect_root(
    root: &Path,
    hash: &mut blake3::Hasher,
    paths: &mut BTreeSet<PathBuf>,
    cancel: &CancellationToken,
) -> Result<(), CoreError> {
    check_cancelled(cancel)?;
    hash.update(root.to_string_lossy().as_bytes());
    if !root.is_dir() {
        paths.insert(root.to_owned());
        return Ok(());
    }
    if let Some(head) = git(root, &["rev-parse", "HEAD"]) {
        hash.update(&head);
        hash.update(
            &git(
                root,
                &[
                    "diff",
                    "--cached",
                    "--binary",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--relative",
                    "--",
                    ".",
                ],
            )
            .ok_or_else(|| {
                CoreError::Internal("Cannot inspect staged verification source".into())
            })?,
        );
        // ls-files --modified omits dirty gitlink worktrees when their HEAD
        // did not change. diff explicitly includes those nested changes.
        let mut changed = git(
            root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=none",
                "--relative",
                "--name-only",
                "-z",
                "HEAD",
                "--",
                ".",
            ],
        )
        .ok_or_else(|| CoreError::Internal("Cannot inspect verification source changes".into()))?;
        changed.extend(
            git(root, &["ls-files", "-z", "--others", "--exclude-standard"]).ok_or_else(|| {
                CoreError::Internal("Cannot inspect untracked verification source".into())
            })?,
        );
        for entry in changed
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            let relative = std::str::from_utf8(entry).map_err(|_| {
                CoreError::InvalidInput("Verification source has a non-UTF8 path".into())
            })?;
            paths.insert(root.join(relative));
        }
    } else {
        // Reuse Git-ignore semantics in ordinary folders; do not follow links.
        for entry in ignore::WalkBuilder::new(root)
            .hidden(false)
            .require_git(false)
            .follow_links(false)
            .build()
        {
            check_cancelled(cancel)?;
            let entry = entry.map_err(|error| {
                CoreError::Internal(format!("Cannot inspect verification source: {error}"))
            })?;
            if entry
                .file_type()
                .is_some_and(|kind| kind.is_file() || kind.is_symlink())
            {
                paths.insert(entry.into_path());
            }
        }
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .output()
        .ok()
        .filter(|result| result.status.success())
        .map(|result| result.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn source_identity_tracks_gitlink_commits_and_successive_dirty_edits() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let child = root.join("dependency");
        std::fs::create_dir(&child).unwrap();
        let command = |root: &Path, args: &[&str]| {
            assert!(git(root, args).is_some(), "git {args:?}");
        };
        for repo in [root, child.as_path()] {
            command(repo, &["init"]);
            command(repo, &["config", "user.name", "Nexa test"]);
            command(repo, &["config", "user.email", "test@example.invalid"]);
        }
        std::fs::write(child.join("source.rs"), "A").unwrap();
        command(&child, &["add", "source.rs"]);
        command(
            &child,
            &["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
        );
        command(root, &["add", "dependency"]);
        command(
            root,
            &["-c", "commit.gpgsign=false", "commit", "-m", "gitlink"],
        );
        let db = Database::open_memory().unwrap();
        let workspace = Workspace {
            roots: vec![root.to_string_lossy().into_owned()],
        };
        let snapshot = || capture(&db, None, None, &[], Some(&workspace));
        let initial = snapshot().await.unwrap();
        std::fs::write(child.join("source.rs"), "B").unwrap();
        let dirty = snapshot().await.unwrap();
        assert_ne!(initial, dirty);
        std::fs::write(child.join("source.rs"), "C").unwrap();
        assert_ne!(dirty, snapshot().await.unwrap());
        command(&child, &["add", "source.rs"]);
        command(
            &child,
            &["-c", "commit.gpgsign=false", "commit", "-m", "change"],
        );
        assert_ne!(initial, snapshot().await.unwrap());
        assert_eq!(snapshot().await.unwrap(), snapshot().await.unwrap());
    }
    #[tokio::test]
    async fn source_identity_observes_external_edits_and_ignores_unchanged_reads() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("source.rs"), "state A").unwrap();
        let db = Database::open_memory().unwrap();
        let workspace = Workspace {
            roots: vec![temp.path().to_string_lossy().into_owned()],
        };
        let capture = || super::capture(&db, None, None, &[], Some(&workspace));
        let first = capture().await.unwrap();
        assert_eq!(capture().await.unwrap(), first);
        std::fs::write(temp.path().join("source.rs"), "state B").unwrap();
        assert_ne!(capture().await.unwrap(), first);
        std::fs::write(temp.path().join("source.rs"), "state A").unwrap();
        assert_eq!(capture().await.unwrap(), first);
        std::fs::remove_file(temp.path().join("source.rs")).unwrap();
        assert_ne!(capture().await.unwrap(), first);
    }

    #[tokio::test]
    async fn routine_snapshots_reuse_content_but_refresh_inventory_and_final_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let first_path = temp.path().join("first.txt");
        std::fs::write(&first_path, "AAA").unwrap();
        std::fs::write(temp.path().join("second.txt"), "BBBB").unwrap();
        let db = Database::open_memory().unwrap();
        let workspace = Workspace {
            roots: vec![temp.path().to_string_lossy().into()],
        };
        let scanner = Capture::default();
        let snapshot = |mode| scanner.capture(&db, None, None, &[], Some(&workspace), mode);
        let initial = snapshot(CaptureMode::Authoritative).await.unwrap();
        assert_eq!(scanner.files.lock().unwrap().bytes_read, 7);
        assert_eq!(snapshot(CaptureMode::Incremental).await.unwrap(), initial);
        assert_eq!(
            scanner.files.lock().unwrap().bytes_read,
            7,
            "unchanged observations must read no content"
        );
        std::fs::write(&first_path, "changed").unwrap();
        let edited = snapshot(CaptureMode::Incremental).await.unwrap();
        assert_ne!(initial, edited);
        assert_eq!(
            scanner.files.lock().unwrap().bytes_read,
            14,
            "only the edited file is rehashed"
        );
        std::fs::rename(
            temp.path().join("second.txt"),
            temp.path().join("renamed.txt"),
        )
        .unwrap();
        let renamed = snapshot(CaptureMode::Incremental).await.unwrap();
        assert_ne!(edited, renamed);
        std::fs::remove_file(temp.path().join("renamed.txt")).unwrap();
        let deleted = snapshot(CaptureMode::Incremental).await.unwrap();
        assert_ne!(renamed, deleted);
        let modified = std::fs::metadata(&first_path).unwrap().modified().unwrap();
        std::fs::write(&first_path, "CHANGED").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&first_path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let bytes_before = scanner.files.lock().unwrap().bytes_read;
        let strict = snapshot(CaptureMode::Authoritative).await.unwrap();
        assert_ne!(
            strict, deleted,
            "restored mtime and equal length cannot attest old bytes"
        );
        assert_eq!(scanner.files.lock().unwrap().bytes_read, bytes_before + 7);
        assert!(!strict.pending);
        std::fs::write(temp.path().join(".gitignore"), "ignored.txt\n").unwrap();
        let ignored = snapshot(CaptureMode::Incremental).await.unwrap();
        std::fs::write(
            temp.path().join("ignored.txt"),
            "data outside the source inventory",
        )
        .unwrap();
        assert_eq!(snapshot(CaptureMode::Incremental).await.unwrap(), ignored);
        std::fs::write(temp.path().join(".gitignore"), "").unwrap();
        assert_ne!(snapshot(CaptureMode::Incremental).await.unwrap(), ignored);
    }

    #[tokio::test]
    async fn cancelled_owner_cannot_read_or_publish_a_source_snapshot() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source.rs"), "content").unwrap();
        let workspace = Workspace {
            roots: vec![root.path().to_string_lossy().into()],
        };
        let db = Database::open_memory().unwrap();
        let cancellation = CancellationToken::new();
        let scanner = Capture::new(cancellation.clone());
        cancellation.cancel();
        let result = scanner
            .capture(
                &db,
                None,
                None,
                &[],
                Some(&workspace),
                CaptureMode::Authoritative,
            )
            .await;
        assert!(matches!(result, Err(CoreError::Cancelled(_))));
        assert_eq!(scanner.files.lock().unwrap().bytes_read, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn large_capture_does_not_block_the_async_executor() {
        use std::{
            io::Write,
            sync::atomic::{AtomicBool, Ordering},
            time::Duration,
        };
        let temp = tempfile::tempdir().unwrap();
        let mut file = std::fs::File::create(temp.path().join("dataset.bin")).unwrap();
        let chunk = vec![17u8; 1024 * 1024];
        for _ in 0..64 {
            file.write_all(&chunk).unwrap();
        }
        drop(file);
        let db = Database::open_memory().unwrap();
        let workspace = Workspace {
            roots: vec![temp.path().to_string_lossy().into()],
        };
        let heartbeat = Arc::new(AtomicBool::new(false));
        let observed = heartbeat.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1)).await;
            observed.store(true, Ordering::SeqCst);
        });
        capture(&db, None, None, &[], Some(&workspace))
            .await
            .unwrap();
        assert!(
            heartbeat.load(Ordering::SeqCst),
            "capture blocked unrelated runtime work"
        );
        timer.await.unwrap();
    }
}
