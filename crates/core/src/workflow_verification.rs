//! Host-owned source identity for verification and review receipts.
use crate::{db::Database, error::CoreError, workspace::Workspace};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSourceState {
    pub digest: String,
    pub pending: bool,
}

pub fn capture(
    db: &Database,
    conversation: Option<&str>,
    turn: Option<&str>,
    source_scope: &[String],
    workspace: Option<&Workspace>,
) -> Result<VerificationSourceState, CoreError> {
    let mut roots = BTreeSet::new();
    for id in source_scope {
        roots.insert(PathBuf::from(db.get_source(id)?.root_path));
    }
    if let Some(workspace) = workspace {
        roots.extend(workspace.roots.iter().map(PathBuf::from));
    }
    let mut paths = BTreeSet::new();
    let mut hash = blake3::Hasher::new();
    for root in roots {
        hash.update(root.to_string_lossy().as_bytes());
        if !root.is_dir() {
            paths.insert(root);
            continue;
        }
        if let Some(head) = git(&root, &["rev-parse", "HEAD"]) {
            hash.update(&head);
            hash.update(
                &git(&root, &["diff", "--cached", "--binary", "--no-ext-diff"]).ok_or_else(
                    || CoreError::Internal("Cannot inspect staged verification source".into()),
                )?,
            );
            let changed = git(
                &root,
                &[
                    "ls-files",
                    "-z",
                    "--modified",
                    "--others",
                    "--exclude-standard",
                ],
            )
            .ok_or_else(|| {
                CoreError::Internal("Cannot inspect verification source changes".into())
            })?;
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
            for entry in ignore::WalkBuilder::new(&root)
                .hidden(false)
                .require_git(false)
                .follow_links(false)
                .build()
            {
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
    for path in paths {
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
                let mut file = std::fs::File::open(&path)?;
                let mut buffer = [0u8; 65536];
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    hash.update(&buffer[..count]);
                }
            }
            Ok(_) => {
                hash.update(b"directory");
            }
        }
    }
    Ok(VerificationSourceState {
        digest: hash.finalize().to_hex().to_string(),
        pending,
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
    fn source_identity_observes_external_edits_and_ignores_unchanged_reads() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("source.rs"), "state A").unwrap();
        let db = Database::open_memory().unwrap();
        let workspace = Workspace {
            roots: vec![temp.path().to_string_lossy().into_owned()],
        };
        let capture = || super::capture(&db, None, None, &[], Some(&workspace)).unwrap();
        let first = capture();
        assert_eq!(capture(), first);
        std::fs::write(temp.path().join("source.rs"), "state B").unwrap();
        assert_ne!(capture(), first);
        std::fs::write(temp.path().join("source.rs"), "state A").unwrap();
        assert_eq!(capture(), first);
        std::fs::remove_file(temp.path().join("source.rs")).unwrap();
        assert_ne!(capture(), first);
    }
}
