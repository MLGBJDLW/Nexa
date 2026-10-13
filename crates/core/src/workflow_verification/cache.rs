use super::{check_cancelled, CaptureMode};
use crate::error::CoreError;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, Metadata},
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Fingerprint {
    length: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    readonly: bool,
    #[cfg(unix)]
    identity_and_change: (u64, u64, i64, i64, u32),
}

impl Fingerprint {
    pub(super) fn of(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            readonly: metadata.permissions().readonly(),
            #[cfg(unix)]
            identity_and_change: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
                metadata.mode(),
            ),
        }
    }
}

struct Entry {
    fingerprint: Fingerprint,
    digest: blake3::Hash,
}

/// This cache belongs to one active capture owner and retains only its current
/// inventory. Metadata reuse is an optimization for routine observations, never
/// the authority for issuing a check/review receipt or accepting completion.
#[derive(Default)]
pub(super) struct FileHashes {
    entries: BTreeMap<PathBuf, Entry>,
    #[cfg(test)]
    pub bytes_read: u64,
}

impl FileHashes {
    pub fn retain(&mut self, paths: &BTreeSet<PathBuf>) {
        self.entries.retain(|path, _| paths.contains(path));
    }

    pub fn read(
        &mut self,
        path: &Path,
        metadata: &Metadata,
        mode: CaptureMode,
        cancel: &CancellationToken,
    ) -> Result<(blake3::Hash, bool), CoreError> {
        check_cancelled(cancel)?;
        let before = Fingerprint::of(metadata);
        if mode == CaptureMode::Incremental {
            if let Some(entry) = self
                .entries
                .get(path)
                .filter(|entry| entry.fingerprint == before)
            {
                return Ok((entry.digest, true));
            }
        }
        let mut file = File::open(path)?;
        let opened = Fingerprint::of(&file.metadata()?);
        let mut hash = blake3::Hasher::new();
        let mut buffer = [0u8; 65536];
        loop {
            check_cancelled(cancel)?;
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
            #[cfg(test)]
            {
                self.bytes_read += count as u64;
            }
        }
        check_cancelled(cancel)?;
        let digest = hash.finalize();
        // Check both the open file and its path: an atomic replacement can
        // leave the old handle unchanged while moving the path to new bytes.
        let stable = before == opened
            && opened == Fingerprint::of(&file.metadata()?)
            && std::fs::symlink_metadata(path).is_ok_and(|after| {
                after.is_file()
                    && !after.file_type().is_symlink()
                    && opened == Fingerprint::of(&after)
            });
        if stable {
            self.entries.insert(
                path.to_owned(),
                Entry {
                    fingerprint: opened,
                    digest,
                },
            );
        } else {
            self.entries.remove(path);
        }
        Ok((digest, stable))
    }
}
