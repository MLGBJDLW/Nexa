//! Build and runtime source identity for reproducible evaluation reports.
//!
//! This module is compiled by both the build script and the evaluator. It hashes
//! the same sorted Git-owned/untracked source paths in both places. Ignored build
//! output and private configuration never enter the fingerprint.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone)]
pub struct CapturedSource {
    pub source_sha: String,
    pub source_dirty: bool,
    pub source_fingerprint: String,
    pub source_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct SourceProvenance {
    pub source_sha: String,
    pub source_dirty: bool,
    pub source_fingerprint: String,
    pub compiled_source_sha: String,
    pub compiled_source_dirty: bool,
    pub compiled_source_fingerprint: String,
}

fn git(root: &Path, arguments: &[&str]) -> Result<Output, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| format!("Cannot inspect evaluator source with git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Cannot inspect evaluator source (git {}): {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

pub fn repository_root(start: &Path) -> Result<PathBuf, String> {
    let output = git(start, &["rev-parse", "--show-toplevel"])?;
    let value = String::from_utf8(output.stdout)
        .map_err(|_| "Git repository path is not valid UTF-8".to_string())?;
    Ok(PathBuf::from(value.trim()))
}

fn hash_value(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

pub fn capture_checkout(root: &Path) -> Result<CapturedSource, String> {
    let source_sha = String::from_utf8(git(root, &["rev-parse", "HEAD"])?.stdout)
        .map_err(|_| "Git source SHA is not valid UTF-8".to_string())?
        .trim()
        .to_owned();
    let source_dirty = !git(root, &["status", "--porcelain", "--untracked-files=all"])?
        .stdout
        .is_empty();
    let listed = git(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    let mut names = BTreeSet::new();
    for path in listed
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = std::str::from_utf8(path)
            .map_err(|_| "Evaluator source paths must be valid UTF-8".to_string())?;
        if !Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(format!(
                "Git returned a source path outside the checkout: {path}"
            ));
        }
        names.insert(path.to_owned());
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"nexa-agent-eval-source-v1\0");
    let mut source_paths = Vec::with_capacity(names.len());
    for name in names {
        let path = root.join(&name);
        hash_value(&mut hasher, name.as_bytes());
        source_paths.push(path.clone());
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // A tracked deletion is a valid dirty development snapshot.
                hasher.update(b"missing\0");
                continue;
            }
            Err(error) => return Err(format!("Cannot inspect source {name}: {error}")),
        };
        if metadata.file_type().is_symlink() {
            hasher.update(b"symlink\0");
            let target = std::fs::read_link(&path)
                .map_err(|error| format!("Cannot inspect source symlink {name}: {error}"))?;
            // Hash the link, never a possibly external target or secret file.
            hash_value(&mut hasher, target.as_os_str().as_encoded_bytes());
        } else if metadata.is_file() {
            hasher.update(b"file\0");
            hasher.update(&metadata.len().to_le_bytes());
            let mut file = std::fs::File::open(&path)
                .map_err(|error| format!("Cannot read evaluator source {name}: {error}"))?;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let count = file
                    .read(&mut buffer)
                    .map_err(|error| format!("Cannot hash evaluator source {name}: {error}"))?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
        } else {
            // Git submodules cannot be described by this checkout's file bytes.
            // Refuse a false provenance claim instead of ignoring their source.
            return Err(format!(
                "Evaluator source {name} is not a regular file or symlink; nested repositories require explicit provenance support"
            ));
        }
    }
    Ok(CapturedSource {
        source_sha,
        source_dirty,
        source_fingerprint: hasher.finalize().to_hex().to_string(),
        source_paths,
    })
}

fn verify_snapshot(
    current: CapturedSource,
    compiled_sha: &str,
    compiled_dirty: bool,
    compiled_fingerprint: &str,
) -> Result<SourceProvenance, String> {
    if compiled_sha.is_empty() || compiled_fingerprint.is_empty() {
        return Err(
            "Evaluator build provenance is unavailable; rebuild with cargo run -p nexa-agent-eval"
                .into(),
        );
    }
    if current.source_sha != compiled_sha || current.source_fingerprint != compiled_fingerprint {
        return Err(format!(
            "Evaluator source differs from its compiled executable (compiled SHA {compiled_sha}, current SHA {}). Rebuild with cargo run -p nexa-agent-eval before creating a report. If Cargo reuses a stale binary after adding an untracked file, run cargo clean -p nexa-agent-eval first.",
            current.source_sha
        ));
    }
    Ok(SourceProvenance {
        source_sha: current.source_sha,
        source_dirty: current.source_dirty || compiled_dirty,
        source_fingerprint: current.source_fingerprint,
        compiled_source_sha: compiled_sha.into(),
        compiled_source_dirty: compiled_dirty,
        compiled_source_fingerprint: compiled_fingerprint.into(),
    })
}

/// Reject a binary built from different source, including uncommitted edits.
/// Matching dirty development snapshots can run, but remain marked dirty.
pub fn verify_current_checkout() -> Result<SourceProvenance, String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    let root = repository_root(&cwd)?;
    verify_snapshot(
        capture_checkout(&root)?,
        option_env!("NEXA_EVAL_BUILD_SHA").unwrap_or_default(),
        option_env!("NEXA_EVAL_BUILD_DIRTY") == Some("true"),
        option_env!("NEXA_EVAL_BUILD_FINGERPRINT").unwrap_or_default(),
    )
}

/// Build-only helper. Watch concrete source files and Git metadata without
/// recursively scanning ignored output such as target or node_modules. New
/// untracked files are additionally checked by the runtime fingerprint.
#[allow(dead_code)]
pub fn cargo_watch_paths(root: &Path, source: &CapturedSource) -> Result<Vec<PathBuf>, String> {
    let mut paths = BTreeSet::new();
    for path in &source.source_paths {
        if path.exists() {
            paths.insert(path.clone());
        }
    }
    let mut git_paths = vec!["HEAD".to_string(), "index".into(), "packed-refs".into()];
    if let Ok(output) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
        git_paths.push(String::from_utf8_lossy(&output.stdout).trim().into());
    }
    for name in git_paths {
        let output = git(
            root,
            &["rev-parse", "--path-format=absolute", "--git-path", &name],
        )?;
        let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if path.exists() {
            paths.insert(path);
        }
    }
    Ok(paths.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "--quiet"]).unwrap();
        std::fs::write(directory.path().join("tracked.txt"), "initial\n").unwrap();
        std::fs::write(directory.path().join(".gitignore"), "ignored/\n").unwrap();
        git(directory.path(), &["add", "."]).unwrap();
        git(
            directory.path(),
            &[
                "-c",
                "user.name=Eval Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        )
        .unwrap();
        directory
    }

    #[test]
    fn fingerprint_tracks_modified_untracked_and_deleted_source_but_ignores_outputs() {
        let directory = fixture();
        let root = directory.path();
        let initial = capture_checkout(root).unwrap();
        assert!(!initial.source_dirty);
        std::fs::create_dir(root.join("ignored")).unwrap();
        std::fs::write(root.join("ignored/output.txt"), "generated report").unwrap();
        assert_eq!(
            capture_checkout(root).unwrap().source_fingerprint,
            initial.source_fingerprint
        );
        std::fs::write(root.join("tracked.txt"), "modified\n").unwrap();
        let modified = capture_checkout(root).unwrap();
        assert!(modified.source_dirty);
        assert_ne!(modified.source_fingerprint, initial.source_fingerprint);
        std::fs::write(root.join("new.rs"), "new source").unwrap();
        let added = capture_checkout(root).unwrap();
        assert_ne!(added.source_fingerprint, modified.source_fingerprint);
        std::fs::remove_file(root.join("tracked.txt")).unwrap();
        assert_ne!(
            capture_checkout(root).unwrap().source_fingerprint,
            added.source_fingerprint
        );
    }

    #[test]
    fn mismatched_binary_source_is_rejected_and_dirty_matching_build_stays_dirty() {
        let directory = fixture();
        let initial = capture_checkout(directory.path()).unwrap();
        let accepted = verify_snapshot(
            initial.clone(),
            &initial.source_sha,
            true,
            &initial.source_fingerprint,
        )
        .unwrap();
        assert!(accepted.source_dirty);
        assert!(verify_snapshot(
            initial.clone(),
            "different-commit",
            false,
            &initial.source_fingerprint
        )
        .is_err());
        assert!(verify_snapshot(
            initial.clone(),
            &initial.source_sha,
            false,
            "different-source"
        )
        .is_err());
        assert_eq!(accepted.compiled_source_sha, initial.source_sha);
        assert_eq!(
            accepted.compiled_source_fingerprint,
            initial.source_fingerprint
        );
    }
}
