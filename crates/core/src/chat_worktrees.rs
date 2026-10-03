//! User-owned chat worktrees. Git snapshots and durable ownership precede removal.
use crate::{db::Database, error::CoreError, workspace::Workspace};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, Weak},
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock},
};

#[cfg(test)]
mod tests;

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidInput(message.into())
}

type WorkspaceLocks = Mutex<HashMap<String, Weak<RwLock<()>>>>;
static LOCKS: LazyLock<WorkspaceLocks> = LazyLock::new(Mutex::default);
fn activity_lock(db: &Database, conversation: &str) -> Arc<RwLock<()>> {
    let database = db
        .db_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| db.runtime_identity().to_string());
    let key = format!("{database}\0{conversation}");
    let mut locks = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, value| value.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(RwLock::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}
/// Hold from before workspace resolution through the last owned write/process.
pub fn activity(db: &Database, conversation: &str) -> Result<OwnedRwLockReadGuard<()>, CoreError> {
    activity_lock(db, conversation)
        .try_read_owned()
        .map_err(|_| invalid("This chat's worktree is changing; wait for the operation to finish"))
}
pub(crate) fn exclusive(
    db: &Database,
    conversation: &str,
) -> Result<OwnedRwLockWriteGuard<()>, CoreError> {
    activity_lock(db, conversation)
        .try_write_owned()
        .map_err(|_| invalid("Stop the chat and close its terminals before changing its worktree"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatWorktree {
    pub id: String,
    pub conversation_id: String,
    pub project_id: String,
    pub source_root: String,
    pub common_dir: String,
    pub path: String,
    pub workspace_subdir: String,
    pub branch: String,
    pub start_sha: String,
    pub status: String,
    pub snapshot_sha: Option<String>,
    #[serde(default)]
    pub index_snapshot_sha: Option<String>,
    pub detail: Option<String>,
}

impl Database {
    pub fn chat_worktree(&self, conversation: &str) -> Result<Option<ChatWorktree>, CoreError> {
        let value: Option<String> = self
            .conn()
            .query_row(
                "SELECT record_json FROM chat_worktrees WHERE conversation_id=?1",
                [conversation],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(CoreError::from))
            .transpose()
    }
    fn save_chat_worktree(&self, record: &ChatWorktree) -> Result<(), CoreError> {
        self.conn().execute("INSERT INTO chat_worktrees(conversation_id,record_json) VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET record_json=excluded.record_json", rusqlite::params![record.conversation_id, serde_json::to_string(record)?])?;
        Ok(())
    }
    pub fn chat_worktree_workspace(
        &self,
        conversation: &str,
    ) -> Result<Option<Workspace>, CoreError> {
        let Some(record) = self.chat_worktree(conversation)? else {
            return Ok(None);
        };
        if record.status != "ready" {
            return Err(invalid("The managed worktree is archived or needs recovery. Restore/recover it before running this chat."));
        }
        validate_owned_path(self, &record, true)?;
        let root = Path::new(&record.path).join(&record.workspace_subdir);
        let workspace = Workspace::validate(&[root.to_string_lossy().into_owned()])?;
        let canonical_root = std::fs::canonicalize(&record.path)?;
        if !std::fs::canonicalize(&workspace.roots[0])?.starts_with(canonical_root) {
            return Err(invalid("Managed workspace escaped its owned checkout"));
        }
        Ok(Some(workspace))
    }
}

fn display(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
    }
    value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
}
fn base(db: &Database) -> Result<PathBuf, CoreError> {
    let parent = db
        .db_path()
        .and_then(Path::parent)
        .ok_or_else(|| invalid("Managed worktrees require a saved application database"))?;
    Ok(parent.join("chat-worktrees"))
}
fn validate_owned_path(
    db: &Database,
    record: &ChatWorktree,
    exists: bool,
) -> Result<(), CoreError> {
    let id = uuid::Uuid::parse_str(&record.id)
        .map_err(|_| invalid("Invalid worktree ownership identifier"))?;
    if id.to_string() != record.id {
        return Err(invalid("Invalid worktree ownership identifier"));
    }
    let expected = base(db)?.join(&record.id);
    if Path::new(&record.path) != expected
        || Path::new(&record.workspace_subdir).is_absolute()
        || Path::new(&record.workspace_subdir).components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err(invalid("Worktree ownership path mismatch"));
    }
    let parent = base(db)?;
    if parent.exists() && std::fs::symlink_metadata(&parent)?.file_type().is_symlink() {
        return Err(invalid("The managed worktree directory must not be a link"));
    }
    if exists {
        if std::fs::symlink_metadata(&expected)?
            .file_type()
            .is_symlink()
            || std::fs::canonicalize(&expected)? != std::fs::canonicalize(parent)?.join(&record.id)
        {
            return Err(invalid("The managed checkout must not be a link"));
        }
    } else if expected.exists() {
        return Err(invalid(
            "The worktree destination already exists; recover it before retrying",
        ));
    }
    Ok(())
}

/// Bounded Git process, no shell, credential prompts, hooks, or inherited Git routing.
pub(crate) async fn git(
    cwd: &Path,
    args: &[&str],
    index: Option<&Path>,
) -> Result<Vec<u8>, CoreError> {
    let mut command = Command::new("git");
    command
        .current_dir(cwd)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.quotepath=false",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "Nexa worktree snapshot")
        .env("GIT_AUTHOR_EMAIL", "nexa@localhost")
        .env("GIT_COMMITTER_NAME", "Nexa worktree snapshot")
        .env("GIT_COMMITTER_EMAIL", "nexa@localhost")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| invalid("Git stdout unavailable"))?
        .take(32 * 1024 * 1024 + 1);
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| invalid("Git stderr unavailable"))?
        .take(65537);
    let result = tokio::time::timeout(Duration::from_secs(120), async {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let (out, err, status) = tokio::join!(
            stdout.read_to_end(&mut output),
            stderr.read_to_end(&mut errors),
            child.wait()
        );
        out?;
        err?;
        if output.len() > 32 * 1024 * 1024 || errors.len() > 65536 {
            return Err(invalid("Git output exceeds the worktree operation limit"));
        }
        if !status?.success() {
            return Err(invalid(format!(
                "Git operation failed: {}",
                String::from_utf8_lossy(&errors)
                    .chars()
                    .take(2000)
                    .collect::<String>()
            )));
        }
        Ok(output)
    })
    .await
    .map_err(|_| invalid("Git operation timed out; recover the worktree before retrying"))?;
    result
}
pub(crate) async fn git_text(cwd: &Path, args: &[&str]) -> Result<String, CoreError> {
    String::from_utf8(git(cwd, args, None).await?)
        .map(|s| s.trim().to_string())
        .map_err(|_| invalid("Git returned a non-UTF-8 path or reference"))
}
async fn identity(db: &Database, record: &ChatWorktree) -> Result<(), CoreError> {
    validate_owned_path(db, record, true)?;
    let root = Path::new(&record.path);
    if std::fs::symlink_metadata(root.join(".git"))?
        .file_type()
        .is_symlink()
    {
        return Err(invalid("The managed Git pointer must not be a link"));
    }
    let actual = git_text(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await?;
    if std::fs::canonicalize(actual)? != std::fs::canonicalize(&record.common_dir)? {
        return Err(invalid("Worktree repository ownership changed"));
    }
    let top = git_text(root, &["rev-parse", "--show-toplevel"]).await?;
    if std::fs::canonicalize(top)? != std::fs::canonicalize(root)? {
        return Err(invalid("Managed path is no longer a Git worktree root"));
    }
    let registrations = git(root, &["worktree", "list", "--porcelain", "-z"], None).await?;
    let expected = std::fs::canonicalize(root)?;
    if !registrations
        .split(|b| *b == 0)
        .filter_map(|part| part.strip_prefix(b"worktree "))
        .filter_map(|part| std::str::from_utf8(part).ok())
        .any(|path| std::fs::canonicalize(path).is_ok_and(|path| path == expected))
    {
        return Err(invalid("Git does not register this managed checkout"));
    }
    Ok(())
}

pub async fn inspect(db: &Database, conversation: &str) -> Result<Option<ChatWorktree>, CoreError> {
    let _guard = activity(db, conversation)?;
    let Some(mut record) = db.chat_worktree(conversation)? else {
        return Ok(None);
    };
    if record.status == "ready" {
        if let Err(error) = identity(db, &record).await {
            record.status = "needs_recovery".into();
            record.detail = Some(error.to_string());
        } else {
            record.branch = git_text(
                Path::new(&record.path),
                &["rev-parse", "--abbrev-ref", "HEAD"],
            )
            .await?;
        }
    }
    Ok(Some(record))
}

pub async fn create(
    db: &Database,
    conversation: &str,
    start_ref: &str,
) -> Result<ChatWorktree, CoreError> {
    let _guard = exclusive(db, conversation)?;
    if db.chat_worktree(conversation)?.is_some() {
        return Err(invalid(
            "This chat already owns a worktree; restore or recover it",
        ));
    }
    let conv = db.get_conversation(conversation)?;
    let project_id = conv
        .project_id
        .ok_or_else(|| invalid("Choose a Git project before creating a worktree"))?;
    let project = db.get_project(&project_id)?;
    let workspace = Workspace::validate(&project.workspace_roots.unwrap_or_default())?;
    if workspace.roots.len() != 1 {
        return Err(invalid(
            "Managed worktrees require a project with exactly one workspace folder",
        ));
    }
    let cwd = Path::new(&workspace.roots[0]);
    let source_root = git_text(cwd, &["rev-parse", "--show-toplevel"]).await?;
    let common_dir = git_text(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await?;
    let start_ref = if start_ref.trim().is_empty() {
        "HEAD"
    } else {
        start_ref.trim()
    };
    if start_ref.starts_with('-')
        || start_ref.len() > 256
        || start_ref.chars().any(char::is_control)
    {
        return Err(invalid("Invalid starting reference"));
    }
    let start_sha = git_text(
        cwd,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{start_ref}^{{commit}}"),
        ],
    )
    .await?;
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(base(db)?)?;
    let subdir = std::fs::canonicalize(cwd)?
        .strip_prefix(std::fs::canonicalize(&source_root)?)
        .map_err(|_| invalid("Workspace is outside its Git root"))?
        .to_string_lossy()
        .into_owned();
    let mut record = ChatWorktree {
        conversation_id: conversation.into(),
        project_id,
        path: display(&base(db)?.join(&id)),
        branch: format!("nexa/chat-{}", &id[..8]),
        id,
        source_root,
        common_dir,
        workspace_subdir: subdir,
        start_sha,
        status: "creating".into(),
        snapshot_sha: None,
        index_snapshot_sha: None,
        detail: None,
    };
    validate_owned_path(db, &record, false)?;
    db.save_chat_worktree(&record)?;
    let result = git_text(
        Path::new(&record.source_root),
        &[
            "worktree",
            "add",
            "-b",
            &record.branch,
            &record.path,
            &record.start_sha,
        ],
    )
    .await;
    finish(db, &mut record, result.map(|_| ()), "ready")?;
    Ok(record)
}
fn finish(
    db: &Database,
    record: &mut ChatWorktree,
    result: Result<(), CoreError>,
    success: &str,
) -> Result<(), CoreError> {
    match result {
        Ok(()) => {
            record.status = success.into();
            record.detail = None;
            db.save_chat_worktree(record)
        }
        Err(error) => {
            record.status = "needs_recovery".into();
            record.detail = Some(error.to_string());
            db.save_chat_worktree(record)?;
            Err(error)
        }
    }
}

async fn snapshot_tree(root: &Path, index: &Path) -> Result<String, CoreError> {
    git(root, &["read-tree", "HEAD"], Some(index)).await?;
    git(root, &["add", "--all", "--", "."], Some(index)).await?;
    String::from_utf8(git(root, &["write-tree"], Some(index)).await?)
        .map(|s| s.trim().into())
        .map_err(|_| invalid("Invalid Git tree"))
}
async fn removable(root: &Path) -> Result<(), CoreError> {
    if !git(
        root,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ],
        None,
    )
    .await?
    .is_empty()
    {
        return Err(invalid("Preserve or remove ignored files before archiving this worktree. They are not included in Git snapshots."));
    }
    let entries = git(root, &["ls-files", "--stage", "-z"], None).await?;
    if entries
        .split(|b| *b == 0)
        .any(|line| line.starts_with(b"160000 "))
    {
        return Err(invalid(
            "Archive is unavailable for worktrees containing submodules",
        ));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut count = 0;
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            count += 1;
            if count > 200_000 {
                return Err(invalid(
                    "Worktree scan exceeds 200,000 entries; preserve it manually",
                ));
            }
            if entry.file_name() == ".git" {
                if dir != root {
                    return Err(invalid(
                        "Archive is unavailable for embedded Git repositories",
                    ));
                }
            } else if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}
pub async fn archive(db: &Database, conversation: &str) -> Result<ChatWorktree, CoreError> {
    let _guard = exclusive(db, conversation)?;
    let _files = tokio::time::timeout(
        Duration::from_secs(5),
        crate::file_mutation::lock_native_tree_mutation(),
    )
    .await
    .map_err(|_| invalid("Native file changes are still settling; retry after they finish"))?;
    let mut record = db
        .chat_worktree(conversation)?
        .ok_or_else(|| invalid("No managed worktree"))?;
    if record.status != "ready" {
        return Err(invalid("Recover the worktree before archiving"));
    }
    identity(db, &record).await?;
    let root = PathBuf::from(&record.path);
    removable(&root).await?;
    let index = tempfile::Builder::new()
        .prefix("nexa-snapshot-")
        .tempdir_in(base(db)?)?;
    let index_path = index.path().join("index");
    let head = git_text(&root, &["rev-parse", "HEAD"]).await?;
    let original_index = git_text(&root, &["write-tree"]).await?;
    let index_snapshot = git_text(
        &root,
        &[
            "commit-tree",
            &original_index,
            "-p",
            &head,
            "-m",
            "Nexa preserved staging area",
        ],
    )
    .await?;
    let tree = snapshot_tree(&root, &index_path).await?;
    let snapshot = git_text(
        &root,
        &[
            "commit-tree",
            &tree,
            "-p",
            &head,
            "-p",
            &index_snapshot,
            "-m",
            "Nexa recoverable chat worktree snapshot",
        ],
    )
    .await?;
    git_text(
        &root,
        &[
            "update-ref",
            &format!("refs/nexa/chat-worktrees/{}/snapshot", record.id),
            &snapshot,
        ],
    )
    .await?;
    record.snapshot_sha = Some(snapshot);
    record.index_snapshot_sha = Some(index_snapshot);
    record.status = "archiving".into();
    db.save_chat_worktree(&record)?;
    let result = async {
        identity(db, &record).await?;
        removable(&root).await?;
        if tree != snapshot_tree(&root, &index_path).await?
            || head != git_text(&root, &["rev-parse", "HEAD"]).await?
            || original_index != git_text(&root, &["write-tree"]).await?
        {
            return Err(invalid(
                "Files or staging changed while creating the snapshot; the checkout was kept",
            ));
        }
        git_text(
            Path::new(&record.common_dir),
            &["worktree", "remove", "--force", &record.path],
        )
        .await?;
        Ok(())
    }
    .await;
    finish(db, &mut record, result, "archived")?;
    Ok(record)
}
pub async fn restore(db: &Database, conversation: &str) -> Result<ChatWorktree, CoreError> {
    let _guard = exclusive(db, conversation)?;
    let mut record = db
        .chat_worktree(conversation)?
        .ok_or_else(|| invalid("No managed worktree"))?;
    if record.status != "archived" {
        return Err(invalid("Only archived worktrees can be restored"));
    }
    validate_owned_path(db, &record, false)?;
    let snapshot = record
        .snapshot_sha
        .as_deref()
        .ok_or_else(|| invalid("The snapshot is unavailable"))?
        .to_string();
    let reference = format!("refs/nexa/chat-worktrees/{}/snapshot", record.id);
    if git_text(
        Path::new(&record.common_dir),
        &["rev-parse", "--verify", &reference],
    )
    .await?
        != snapshot
    {
        return Err(invalid("The saved snapshot reference changed"));
    }
    record.status = "restoring".into();
    db.save_chat_worktree(&record)?;
    let result = git_text(
        Path::new(&record.common_dir),
        &["worktree", "add", "--detach", &record.path, &snapshot],
    )
    .await;
    record.branch = "(detached snapshot)".into();
    finish(db, &mut record, result.map(|_| ()), "ready")?;
    Ok(record)
}
/// Recovery only reconciles existing ownership. It never deletes a directory.
pub async fn recover(db: &Database, conversation: &str) -> Result<Option<ChatWorktree>, CoreError> {
    let _guard = exclusive(db, conversation)?;
    let Some(mut record) = db.chat_worktree(conversation)? else {
        return Ok(None);
    };
    if Path::new(&record.path).exists() {
        identity(db, &record).await?;
        record.status = "ready".into();
        record.detail = None;
        db.save_chat_worktree(&record)?;
        Ok(Some(record))
    } else if let Some(snapshot) = record.snapshot_sha.as_deref() {
        validate_owned_path(db, &record, false)?;
        if git_text(
            Path::new(&record.common_dir),
            &[
                "rev-parse",
                "--verify",
                &format!("refs/nexa/chat-worktrees/{}/snapshot", record.id),
            ],
        )
        .await?
            != snapshot
        {
            return Err(invalid("Snapshot verification failed; ownership retained"));
        }
        record.status = "archived".into();
        record.detail = None;
        db.save_chat_worktree(&record)?;
        Ok(Some(record))
    } else if matches!(record.status.as_str(), "creating" | "needs_recovery") {
        validate_owned_path(db, &record, false)?;
        let listed = git(
            Path::new(&record.common_dir),
            &["worktree", "list", "--porcelain", "-z"],
            None,
        )
        .await?;
        if listed
            .split(|b| *b == 0)
            .filter_map(|line| line.strip_prefix(b"worktree "))
            .filter_map(|line| std::str::from_utf8(line).ok())
            .any(|path| {
                let actual = path.replace('\\', "/");
                let expected = record.path.replace('\\', "/");
                if cfg!(windows) {
                    actual.eq_ignore_ascii_case(&expected)
                } else {
                    actual == expected
                }
            })
        {
            return Err(invalid("Git still registers this missing checkout; repair Git's worktree metadata manually"));
        }
        db.conn().execute(
            "DELETE FROM chat_worktrees WHERE conversation_id=?1",
            [conversation],
        )?;
        Ok(None)
    } else {
        Err(invalid(
            "The checkout is missing and no recovery snapshot is recorded; ownership retained",
        ))
    }
}
