//! Host-owned source identity for verification and review receipts.
use crate::{db::Database, error::CoreError, workspace::Workspace};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
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
                    #[cfg(test)]
                    after_inventory: None,
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
    #[cfg(test)]
    after_inventory: Option<&'a mut dyn FnMut()>,
}

fn check_cancelled(cancel: &CancellationToken) -> Result<(), CoreError> {
    if cancel.is_cancelled() {
        Err(CoreError::Cancelled("Verification source capture".into()))
    } else {
        Ok(())
    }
}

fn hash_file_mode(hash: &mut blake3::Hasher, metadata: &std::fs::Metadata) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        hash.update(&metadata.permissions().mode().to_le_bytes());
    }
    #[cfg(not(unix))]
    hash.update(&[u8::from(metadata.permissions().readonly())]);
}

fn hash_path(hash: &mut blake3::Hasher, path: &Path) {
    let bytes = path.as_os_str().as_encoded_bytes();
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn git_relative_path(bytes: &[u8]) -> Result<PathBuf, CoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes).map(PathBuf::from).map_err(|_| {
            CoreError::InvalidInput(
                "Git returned a path that cannot be represented on this platform".into(),
            )
        })
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
    hash.update(b"verification-source-v4");
    let mut inventories = BTreeMap::new();
    let mut pending = false;
    for root in roots {
        pending |= !collect_root(&root, &mut hash, &mut paths, &mut inventories, scan.cancel)?;
    }
    #[cfg(test)]
    if let Some(observer) = scan.after_inventory.take() {
        observer();
    }
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
        hash_path(&mut hash, &path);
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hash.update(b"deleted");
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.file_type().is_symlink() => {
                hash_path(&mut hash, &std::fs::read_link(&path)?);
            }
            Ok(metadata) if metadata.is_file() => {
                hash_file_mode(&mut hash, &metadata);
                let (digest, stable) = scan.files.read(&path, &metadata, scan.mode, scan.cancel)?;
                hash.update(digest.as_bytes());
                pending |= !stable;
                file_paths.insert(path);
            }
            Ok(_) => {
                let canonical = path.canonicalize()?;
                if visited_directories.insert(canonical) {
                    pending |=
                        !collect_root(&path, &mut hash, &mut paths, &mut inventories, scan.cancel)?;
                }
            }
        }
    }
    check_cancelled(scan.cancel)?;
    // A clean Git file can become dirty after its initial diff, and a new
    // ordinary-folder entry can appear after its walk. Neither was hashed.
    // Revalidate every inventory (including gitlinks) before issuing receipts.
    // Metadata also catches later changes to an already-dirty path. A moving
    // workspace remains pending rather than receiving a mixed-state receipt.
    for (root, before) in inventories {
        pending |= collect_inventory(&root, scan.cancel)? != before;
    }
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
    inventories: &mut BTreeMap<PathBuf, Inventory>,
    cancel: &CancellationToken,
) -> Result<bool, CoreError> {
    let inventory = collect_inventory(root, cancel)?;
    hash.update(inventory.digest.as_bytes());
    paths.extend(inventory.paths.iter().cloned());
    match inventories.entry(root.to_owned()) {
        std::collections::btree_map::Entry::Occupied(before) => Ok(before.get() == &inventory),
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(inventory);
            Ok(true)
        }
    }
}

#[derive(PartialEq, Eq)]
struct Inventory {
    digest: blake3::Hash,
    paths: BTreeSet<PathBuf>,
    observations: BTreeMap<PathBuf, PathObservation>,
}

#[derive(PartialEq, Eq)]
enum PathObservation {
    Missing,
    Present {
        fingerprint: cache::Fingerprint,
        kind: (bool, bool, bool),
        link: Option<PathBuf>,
    },
}

fn collect_inventory(root: &Path, cancel: &CancellationToken) -> Result<Inventory, CoreError> {
    check_cancelled(cancel)?;
    let mut hash = blake3::Hasher::new();
    let mut paths = BTreeSet::new();
    let mut observed_paths = BTreeSet::new();
    hash_path(&mut hash, root);
    if !root.is_dir() {
        paths.insert(root.to_owned());
    } else if let Some(head) = git(root, &["rev-parse", "HEAD"]) {
        hash.update(&head);
        // Git may ignore permission changes (Windows readonly, non-executable
        // Unix modes, or core.filemode=false). Inspect metadata for every tracked
        // path without rereading clean tracked content. Recurse through gitlinks
        // so the same guarantee applies to an otherwise clean submodule.
        let tracked =
            git(root, &["ls-files", "-v", "-z", "--cached", "--", "."]).ok_or_else(|| {
                CoreError::Internal("Cannot inspect tracked verification modes".into())
            })?;
        for entry in tracked
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
        {
            check_cancelled(cancel)?;
            let (tag, entry) = match entry {
                [tag, b' ', path @ ..] if !path.is_empty() => (*tag, path),
                _ => {
                    return Err(CoreError::Internal(
                        "Invalid Git verification inventory".into(),
                    ))
                }
            };
            let path = root.join(git_relative_path(entry)?);
            observed_paths.insert(path.clone());
            // Git's index hints may hide arbitrary worktree edits from diff.
            // Treat those files as content-bearing even when Git reports clean.
            if tag.is_ascii_lowercase() || tag == b'S' {
                paths.insert(path.clone());
            }
            hash.update(b"tracked-mode\0");
            hash.update(&(entry.len() as u64).to_le_bytes());
            hash.update(entry);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    hash_file_mode(&mut hash, &metadata);
                    if metadata.is_dir() && !metadata.file_type().is_symlink() {
                        paths.insert(path);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    hash.update(b"deleted");
                }
                Err(error) => return Err(error.into()),
            }
        }
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
            paths.insert(root.join(git_relative_path(entry)?));
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
    observed_paths.extend(paths.iter().cloned());
    let mut observations = BTreeMap::new();
    for path in observed_paths {
        check_cancelled(cancel)?;
        let observed = match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PathObservation::Missing,
            Err(error) => return Err(error.into()),
            Ok(metadata) => PathObservation::Present {
                fingerprint: cache::Fingerprint::of(&metadata),
                kind: (
                    metadata.is_file(),
                    metadata.is_dir(),
                    metadata.file_type().is_symlink(),
                ),
                link: metadata
                    .file_type()
                    .is_symlink()
                    .then(|| std::fs::read_link(&path))
                    .transpose()?,
            },
        };
        observations.insert(path, observed);
    }
    Ok(Inventory {
        digest: hash.finalize(),
        paths,
        observations,
    })
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

    #[test]
    fn a_clean_git_file_edited_after_inventory_cannot_attest_old_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.rs");
        std::fs::write(&source, "AAAA").unwrap();
        for args in [
            vec!["init"],
            vec!["config", "user.name", "Nexa test"],
            vec!["config", "user.email", "test@example.invalid"],
            vec!["add", "source.rs"],
            vec!["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
        ] {
            assert!(git(root.path(), &args).is_some());
        }
        let db = Database::open_memory().unwrap();
        let workspace = Workspace {
            roots: vec![root.path().to_string_lossy().into()],
        };
        let mut files = cache::FileHashes::default();
        let cancel = CancellationToken::new();
        let mut edit_source = || std::fs::write(&source, "BBBB").unwrap();
        let mut scan = Scan {
            files: &mut files,
            mode: CaptureMode::Authoritative,
            cancel: &cancel,
            after_inventory: None,
        };
        let initial = capture_sync(&db, None, None, &[], Some(&workspace), &mut scan).unwrap();
        scan.after_inventory = Some(&mut edit_source);
        let racing = capture_sync(&db, None, None, &[], Some(&workspace), &mut scan).unwrap();
        assert!(
            racing.pending,
            "a clean path modified after Git diff must not preserve old receipts"
        );
        let settled = capture_sync(&db, None, None, &[], Some(&workspace), &mut scan).unwrap();
        assert!(!settled.pending);
        assert_ne!(initial.digest, settled.digest);
    }

    #[test]
    fn new_paths_after_inventory_remain_pending_in_git_and_plain_folders() {
        for git_repository in [false, true] {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("source.rs"), "AAAA").unwrap();
            if git_repository {
                for args in [
                    vec!["init"],
                    vec!["config", "user.name", "Nexa test"],
                    vec!["config", "user.email", "test@example.invalid"],
                    vec!["add", "source.rs"],
                    vec!["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
                ] {
                    assert!(git(root.path(), &args).is_some());
                }
            }
            let db = Database::open_memory().unwrap();
            let workspace = Workspace {
                roots: vec![root.path().to_string_lossy().into()],
            };
            let mut files = cache::FileHashes::default();
            let cancel = CancellationToken::new();
            let mut add_source = || std::fs::write(root.path().join("new.rs"), "BBBB").unwrap();
            let mut scan = Scan {
                files: &mut files,
                mode: CaptureMode::Authoritative,
                cancel: &cancel,
                after_inventory: None,
            };
            let initial = capture_sync(&db, None, None, &[], Some(&workspace), &mut scan).unwrap();
            scan.after_inventory = Some(&mut add_source);
            assert!(
                capture_sync(&db, None, None, &[], Some(&workspace), &mut scan)
                    .unwrap()
                    .pending
            );
            let settled = capture_sync(&db, None, None, &[], Some(&workspace), &mut scan).unwrap();
            assert!(!settled.pending);
            assert_ne!(initial.digest, settled.digest);
        }
    }

    #[tokio::test]
    async fn index_hints_cannot_hide_worktree_edits_from_verification() {
        for flags in [
            vec!["--assume-unchanged"],
            vec!["--skip-worktree"],
            vec!["--assume-unchanged", "--skip-worktree"],
        ] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.rs");
            std::fs::write(&source, "AAAA").unwrap();
            for args in [
                vec!["init"],
                vec!["config", "user.name", "Nexa test"],
                vec!["config", "user.email", "test@example.invalid"],
                vec!["add", "source.rs"],
                vec!["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
            ] {
                assert!(git(root.path(), &args).is_some());
            }
            for flag in &flags {
                assert!(git(root.path(), &["update-index", flag, "--", "source.rs"]).is_some());
            }
            let db = Database::open_memory().unwrap();
            let workspace = Workspace {
                roots: vec![root.path().to_string_lossy().into()],
            };
            let scanner = Capture::default();
            let snapshot = |mode| scanner.capture(&db, None, None, &[], Some(&workspace), mode);
            let initial = snapshot(CaptureMode::Authoritative).await.unwrap();
            let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
            std::fs::write(&source, "BBBB").unwrap();
            std::fs::File::options()
                .write(true)
                .open(&source)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
            assert!(git(root.path(), &["diff", "--name-only", "HEAD"])
                .unwrap()
                .is_empty());
            let changed = snapshot(CaptureMode::Authoritative).await.unwrap();
            assert_ne!(
                initial, changed,
                "index flags {flags:?} must not preserve old receipts"
            );
            std::fs::write(&source, "CCCC").unwrap();
            assert_ne!(snapshot(CaptureMode::Incremental).await.unwrap(), changed);
            std::fs::write(&source, "AAAA").unwrap();
            assert_eq!(snapshot(CaptureMode::Authoritative).await.unwrap(), initial);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn native_path_bytes_survive_git_capture_and_remain_distinct() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        for git_repository in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let first = root.path().join(OsStr::from_bytes(b"source-\xff"));
            let second = root.path().join(OsStr::from_bytes(b"source-\xfe"));
            assert_eq!(first.to_string_lossy(), second.to_string_lossy());
            std::fs::write(&first, "initial").unwrap();
            if git_repository {
                for args in [
                    vec!["init"],
                    vec!["config", "user.name", "Nexa test"],
                    vec!["config", "user.email", "test@example.invalid"],
                    vec!["add", "--all"],
                    vec!["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
                ] {
                    assert!(git(root.path(), &args).is_some());
                }
            }
            let db = Database::open_memory().unwrap();
            let workspace = Workspace {
                roots: vec![root.path().to_string_lossy().into()],
            };
            let snapshot = || capture(&db, None, None, &[], Some(&workspace));
            let initial = snapshot().await.unwrap();
            std::fs::write(&first, "changed").unwrap();
            let edited = snapshot().await.unwrap();
            assert_ne!(edited, initial);
            std::fs::rename(&first, &second).unwrap();
            let renamed = snapshot().await.unwrap();
            assert_ne!(
                renamed, edited,
                "lossy-equivalent filenames must retain distinct identity"
            );
            std::fs::write(&second, "changed again").unwrap();
            assert_ne!(snapshot().await.unwrap(), renamed);
            std::fs::rename(&second, &first).unwrap();
            std::fs::write(&first, "initial").unwrap();
            assert_eq!(snapshot().await.unwrap(), initial);

            // Symlink targets also carry native path identity, even when dangling.
            let link = root.path().join("link");
            std::os::unix::fs::symlink(OsStr::from_bytes(b"target-\xff"), &link).unwrap();
            let first_link = snapshot().await.unwrap();
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink(OsStr::from_bytes(b"target-\xfe"), &link).unwrap();
            assert_ne!(snapshot().await.unwrap(), first_link);
        }
    }

    #[tokio::test]
    async fn permission_only_changes_invalidate_clean_dirty_and_plain_workspace_receipts() {
        for state in ["plain", "clean", "dirty"] {
            let root = tempfile::tempdir().unwrap();
            let file = root.path().join("script.sh");
            std::fs::write(&file, "echo initial").unwrap();
            if state != "plain" {
                for args in [
                    vec!["init"],
                    vec!["config", "user.name", "Nexa test"],
                    vec!["config", "user.email", "test@example.invalid"],
                    vec!["add", "script.sh"],
                    vec!["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
                ] {
                    assert!(git(root.path(), &args).is_some());
                }
            }
            if state == "dirty" {
                std::fs::write(&file, "echo changed").unwrap();
            }
            let db = Database::open_memory().unwrap();
            let workspace = Workspace {
                roots: vec![root.path().to_string_lossy().into()],
            };
            let scanner = Capture::default();
            let snapshot = || {
                scanner.capture(
                    &db,
                    None,
                    None,
                    &[],
                    Some(&workspace),
                    CaptureMode::Incremental,
                )
            };
            #[cfg(unix)]
            let changes = [true, false];
            #[cfg(not(unix))]
            let changes = [false];
            for executable in changes {
                let initial = snapshot().await.unwrap();
                let bytes_before = scanner.files.lock().unwrap().bytes_read;
                let permissions = std::fs::metadata(&file).unwrap().permissions();
                let mut changed = permissions.clone();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    changed.set_mode(if executable {
                        permissions.mode() ^ 0o111
                    } else {
                        permissions.mode() & !0o222
                    });
                }
                #[cfg(not(unix))]
                changed.set_readonly(!permissions.readonly());
                std::fs::set_permissions(&file, changed).unwrap();
                let changed_state = snapshot().await;
                std::fs::set_permissions(&file, permissions).unwrap();
                assert_ne!(
                    changed_state.unwrap(),
                    initial,
                    "state={state}, executable={executable}"
                );
                assert_eq!(snapshot().await.unwrap(), initial);
                if state == "clean" && !executable {
                    assert_eq!(
                        scanner.files.lock().unwrap().bytes_read,
                        bytes_before,
                        "clean Git content must remain unread for a mode-only change"
                    );
                }
            }
        }
    }

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
