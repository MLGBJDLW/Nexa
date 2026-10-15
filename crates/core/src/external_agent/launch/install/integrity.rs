//! Host-created receipts cover the complete immutable installation, including
//! helpers and safe archive links. Missing or changed contents require repair.
use super::{invalid, verify_target, BinaryDistribution, CoreError, MAX_UNPACKED_BYTES};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MARKER: &str = ".nexa-install.json";
const MAX_RECEIPT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    version: u32,
    url: String,
    cmd: String,
    sha256: String,
    entries: BTreeMap<String, Entry>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Entry {
    File {
        bytes: u64,
        sha256: String,
        mode: Option<u32>,
        readonly: bool,
    },
    Directory {
        mode: Option<u32>,
        readonly: bool,
    },
    Link {
        target: PathBuf,
    },
}

fn mode(metadata: &std::fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

fn snapshot(root: &Path) -> Result<BTreeMap<String, Entry>, CoreError> {
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid(
            "ACP installation root must be a directory, not a link",
        ));
    }
    let canonical = root.canonicalize()?;
    let mut directories = vec![root.to_owned()];
    let mut entries = BTreeMap::new();
    let mut total_bytes = 0u64;
    while let Some(directory) = directories.pop() {
        for child in std::fs::read_dir(directory)? {
            let path = child?.path();
            if path == root.join(MARKER) {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| invalid("ACP installation path escaped its root"))?;
            let name = relative
                .to_str()
                .ok_or_else(|| invalid("ACP installation has an unsupported filename"))?
                .to_owned();
            let metadata = std::fs::symlink_metadata(&path)?;
            let entry = if metadata.file_type().is_symlink() {
                if !path.canonicalize()?.starts_with(&canonical) {
                    return Err(invalid("ACP installation link escapes its root"));
                }
                Entry::Link {
                    target: std::fs::read_link(&path)?,
                }
            } else if metadata.is_dir() {
                directories.push(path);
                Entry::Directory {
                    mode: mode(&metadata),
                    readonly: metadata.permissions().readonly(),
                }
            } else if metadata.is_file() {
                total_bytes = total_bytes
                    .checked_add(metadata.len())
                    .filter(|total| *total <= MAX_UNPACKED_BYTES)
                    .ok_or_else(|| invalid("ACP installation exceeds its size limit"))?;
                let mut file = File::open(&path)?.take(metadata.len().saturating_add(1));
                let mut hash = Sha256::new();
                let mut buffer = [0u8; 65536];
                let mut bytes = 0u64;
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    bytes += count as u64;
                    hash.update(&buffer[..count]);
                }
                if bytes != metadata.len() {
                    return Err(invalid("ACP installation changed during verification"));
                }
                Entry::File {
                    bytes,
                    sha256: format!("{:x}", hash.finalize()),
                    mode: mode(&metadata),
                    readonly: metadata.permissions().readonly(),
                }
            } else {
                return Err(invalid(
                    "ACP installation contains a special filesystem entry",
                ));
            };
            entries.insert(name, entry);
        }
    }
    Ok(entries)
}

pub(super) fn write(
    root: &Path,
    binary: &BinaryDistribution,
    digest: &str,
) -> Result<(), CoreError> {
    let receipt = Receipt {
        version: 1,
        url: binary.archive.clone(),
        cmd: binary.cmd.clone(),
        sha256: digest.into(),
        entries: snapshot(root)?,
    };
    let bytes = serde_json::to_vec(&receipt)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(invalid("ACP installation receipt exceeds its size limit"));
    }
    // Never overwrite an archive-provided entry or follow its link.
    File::options()
        .write(true)
        .create_new(true)
        .open(root.join(MARKER))?
        .write_all(&bytes)?;
    Ok(())
}

pub(super) fn verify(
    root: &Path,
    binary: &BinaryDistribution,
    digest: &str,
    relative: &Path,
) -> Result<PathBuf, CoreError> {
    let receipt = read_receipt(root)?;
    if receipt.version != 1
        || receipt.url != binary.archive
        || receipt.cmd != binary.cmd
        || receipt.sha256 != digest
        || receipt.entries != snapshot(root)?
    {
        return Err(invalid(
            "ACP installation is incomplete or changed; it requires repair",
        ));
    }
    verify_target(root, relative)
}

pub(super) fn recorded_digest(
    root: &Path,
    binary: &BinaryDistribution,
) -> Result<String, CoreError> {
    let receipt = read_receipt(root)?;
    if receipt.version != 1
        || receipt.url != binary.archive
        || receipt.cmd != binary.cmd
        || receipt.sha256.len() != 64
        || !receipt.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid("ACP installation has no matching digest receipt"));
    }
    Ok(receipt.sha256)
}

fn read_receipt(root: &Path) -> Result<Receipt, CoreError> {
    let marker = root.join(MARKER);
    let metadata = std::fs::symlink_metadata(&marker)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_RECEIPT_BYTES
    {
        return Err(invalid("ACP installation receipt is invalid"));
    }
    let mut bytes = Vec::new();
    File::open(marker)?
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err(invalid("ACP installation receipt exceeds its size limit"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipts_cover_executables_companions_and_unexpected_files() {
        for mutation in ["executable", "companion", "missing", "extra", "receipt"] {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("agent"), "AAAA").unwrap();
            std::fs::write(root.path().join("helper"), "BBBB").unwrap();
            let binary = BinaryDistribution {
                archive: "https://example.invalid/agent.zip".into(),
                cmd: "agent".into(),
                ..Default::default()
            };
            verify_target(root.path(), Path::new("agent")).unwrap();
            write(root.path(), &binary, "pinned-archive").unwrap();
            assert!(verify(root.path(), &binary, "pinned-archive", Path::new("agent")).is_ok());
            match mutation {
                "executable" => std::fs::write(root.path().join("agent"), "CCCC").unwrap(),
                "companion" => std::fs::write(root.path().join("helper"), "DDDD").unwrap(),
                "missing" => std::fs::remove_file(root.path().join("helper")).unwrap(),
                "extra" => std::fs::write(root.path().join("injected.dll"), "extra").unwrap(),
                _ => std::fs::write(root.path().join(MARKER), "{}").unwrap(),
            }
            assert!(
                verify(root.path(), &binary, "pinned-archive", Path::new("agent")).is_err(),
                "{mutation}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn receipts_preserve_safe_links_and_reject_retargeting() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("agent"), "AAAA").unwrap();
        std::fs::write(outside.path().join("other"), "BBBB").unwrap();
        std::os::unix::fs::symlink("agent", root.path().join("alias")).unwrap();
        let binary = BinaryDistribution {
            archive: "https://example.invalid/agent.tar.gz".into(),
            cmd: "alias".into(),
            ..Default::default()
        };
        verify_target(root.path(), Path::new("alias")).unwrap();
        write(root.path(), &binary, "pinned-archive").unwrap();
        assert!(verify(root.path(), &binary, "pinned-archive", Path::new("alias")).is_ok());
        std::fs::remove_file(root.path().join("alias")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("other"), root.path().join("alias"))
            .unwrap();
        assert!(verify(root.path(), &binary, "pinned-archive", Path::new("alias")).is_err());
    }
}
