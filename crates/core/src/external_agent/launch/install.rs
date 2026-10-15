//! Versioned registry archives are installed atomically with their companion
//! files. Only the bundled registry can supply these URLs, never a chat message.
use super::invalid;
use crate::{error::CoreError, external_agent::BinaryDistribution};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};
use tokio::{io::AsyncWriteExt, sync::Mutex};

mod integrity;

const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

fn install_lock(key: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<std::sync::Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, value| value.strong_count() > 0);
    if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key.to_owned(), Arc::downgrade(&lock));
    lock
}

fn relative_path(value: &str) -> Result<PathBuf, CoreError> {
    let value = value.replace('\\', "/");
    let path = Path::new(&value);
    if value.is_empty()
        || value.contains(':')
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(invalid("ACP archive contains an unsafe path"));
    }
    Ok(path
        .components()
        .filter_map(|c| {
            if let Component::Normal(v) = c {
                Some(v)
            } else {
                None
            }
        })
        .collect())
}

fn verify_target(root: &Path, relative: &Path) -> Result<PathBuf, CoreError> {
    let path = root.join(relative);
    let canonical = path.canonicalize().map_err(|e| {
        invalid(format!(
            "ACP archive did not contain its declared executable: {e}"
        ))
    })?;
    if !canonical.starts_with(root.canonicalize()?) || !canonical.is_file() {
        return Err(invalid(
            "ACP archive executable escapes its installation directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = canonical.metadata()?.permissions().mode() | 0o700;
        std::fs::set_permissions(&canonical, std::fs::Permissions::from_mode(permissions))?;
    }
    Ok(path)
}

fn extract_zip(archive: &Path, root: &Path) -> Result<(), CoreError> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)
        .map_err(|e| invalid(format!("Invalid ACP ZIP archive: {e}")))?;
    let mut size = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| invalid(format!("Cannot read ACP archive: {e}")))?;
        let relative = relative_path(entry.name())?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(invalid("ACP ZIP archive contains a symbolic link"));
        }
        let path = root.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(path)?;
            continue;
        }
        size = size
            .checked_add(entry.size())
            .filter(|size| *size <= MAX_UNPACKED_BYTES)
            .ok_or_else(|| invalid("ACP archive exceeds the extraction size limit"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = File::options().write(true).create_new(true).open(path)?;
        let expected_size = entry.size();
        let copied = std::io::copy(
            &mut entry.by_ref().take(expected_size.saturating_add(1)),
            &mut file,
        )?;
        if copied != expected_size {
            return Err(invalid("ACP archive file length mismatch"));
        }
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(())
}

fn extract_tar(reader: impl Read, root: &Path) -> Result<(), CoreError> {
    let mut archive = tar::Archive::new(reader);
    let mut size = 0u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        relative_path(&path.to_string_lossy())?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir() || kind.is_symlink() || kind.is_hard_link()) {
            return Err(invalid("ACP archive contains a special filesystem entry"));
        }
        if let Some(target) = entry.link_name()? {
            // Safe links inside a distribution are needed by macOS/Linux tools.
            // Reject absolute/escaping targets before tar's own traversal checks.
            if target.is_absolute() {
                return Err(invalid("ACP archive contains an absolute link"));
            }
            let mut depth = if kind.is_symlink() {
                path.parent().map_or(0, |p| {
                    p.components()
                        .filter(|c| matches!(c, Component::Normal(_)))
                        .count()
                })
            } else {
                0
            };
            for component in target.components() {
                match component {
                    Component::Normal(_) => depth += 1,
                    Component::CurDir => (),
                    Component::ParentDir if depth > 0 => depth -= 1,
                    _ => {
                        return Err(invalid(
                            "ACP archive link escapes its installation directory",
                        ))
                    }
                }
            }
        }
        size = size
            .checked_add(entry.size())
            .filter(|size| *size <= MAX_UNPACKED_BYTES)
            .ok_or_else(|| invalid("ACP archive exceeds the extraction size limit"))?;
        if !entry.unpack_in(root)? {
            return Err(invalid(
                "ACP archive entry escapes its installation directory",
            ));
        }
    }
    Ok(())
}

fn extract(archive: &Path, url: &str, root: &Path, executable: &Path) -> Result<(), CoreError> {
    std::fs::create_dir_all(root)?;
    if url.ends_with(".zip") {
        extract_zip(archive, root)
    } else if url.ends_with(".tar.gz") {
        extract_tar(flate2::read::GzDecoder::new(File::open(archive)?), root)
    } else if url.ends_with(".tar.bz2") {
        extract_tar(bzip2::read::BzDecoder::new(File::open(archive)?), root)
    } else {
        let target = root.join(executable);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(archive, target)?;
        Ok(())
    }
}

fn publish(root: &Path, directory: &Path, previous: &Path) -> Result<(), CoreError> {
    let parent = directory
        .parent()
        .ok_or_else(|| invalid("ACP cache has no parent"))?
        .canonicalize()?;
    for staged in [root, previous] {
        let owner = staged
            .parent()
            .ok_or_else(|| invalid("ACP staging path has no parent"))?
            .canonicalize()?;
        if !owner.starts_with(&parent) {
            return Err(invalid("ACP staging path escaped its cache"));
        }
    }
    if std::fs::symlink_metadata(previous).is_ok() {
        return Err(invalid("ACP repair backup already exists"));
    }
    let had_previous = match std::fs::symlink_metadata(directory) {
        Ok(_) => {
            std::fs::rename(directory, previous)?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if let Err(error) = std::fs::rename(root, directory) {
        if had_previous && std::fs::symlink_metadata(directory).is_err() {
            std::fs::rename(previous, directory)?;
        }
        return Err(error.into());
    }
    Ok(())
}

pub(super) async fn binary(
    binary: &BinaryDistribution,
    cache_root: &Path,
    official_registry: bool,
) -> Result<PathBuf, CoreError> {
    // Validate before consulting the cache as well as before downloading. A
    // previous unverified installation must not bypass the pinned registry.
    let expected_digest = match binary.sha256.as_deref() {
        Some(digest) if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) => Some(digest.to_ascii_lowercase()),
        None if official_registry => None,
        _ => return Err(invalid("This ACP archive has no valid pinned SHA-256 checksum or official registry provenance.")),
    };
    let url = url::Url::parse(&binary.archive)
        .map_err(|_| invalid("Invalid ACP registry archive URL"))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            "ACP registry archives require HTTPS without credentials",
        ));
    }
    let relative = relative_path(&binary.cmd)?;
    let key = blake3::hash(
        format!(
            "{}\0{}\0{}",
            binary.archive,
            binary.cmd,
            expected_digest.as_deref().unwrap_or("official-registry")
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    let directory = cache_root.join(&key);
    let lock = install_lock(&directory);
    let guard = lock.lock_owned().await;
    let check_directory = directory.clone();
    let check_binary = binary.clone();
    let check_digest = expected_digest.clone();
    let check_relative = relative.clone();
    let (guard, cached, expected_digest) = tokio::task::spawn_blocking(move || {
        // Some official publishers omit archive hashes. The first HTTPS
        // download records its observed digest; later launches and repairs
        // retain that identity. This is not a publisher signature.
        let digest = check_digest
            .or_else(|| integrity::recorded_digest(&check_directory, &check_binary).ok());
        let cached = digest.as_deref().and_then(|digest| {
            integrity::verify(&check_directory, &check_binary, digest, &check_relative).ok()
        });
        (guard, cached, digest)
    })
    .await
    .map_err(|error| invalid(format!("ACP cache verification failed: {error}")))?;
    if let Some(cached) = cached {
        return Ok(cached);
    }
    tokio::fs::create_dir_all(cache_root).await?;
    let scratch = tempfile::Builder::new()
        .prefix(".preparing-")
        .tempdir_in(cache_root)?;
    let archive = scratch.path().join("archive");
    let client = reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" || attempt.previous().len() >= 10 {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|error| invalid(format!("Cannot prepare the ACP download client: {error}")))?;
    let response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| invalid(format!("ACP adapter download failed: {error}")))?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_ARCHIVE_BYTES)
    {
        return Err(invalid("ACP download exceeds the archive size limit"));
    }
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(&archive).await?;
    let mut digest = Sha256::new();
    let mut received = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| invalid(format!("ACP adapter download interrupted: {error}")))?;
        received += chunk.len() as u64;
        if received > MAX_ARCHIVE_BYTES {
            return Err(invalid("ACP download exceeds the archive size limit"));
        }
        digest.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    let digest = format!("{:x}", digest.finalize());
    if expected_digest
        .as_ref()
        .is_some_and(|expected| expected != &digest)
    {
        return Err(invalid(
            "ACP archive checksum mismatch; the downloaded adapter was not installed",
        ));
    }
    let binary = binary.clone();
    tokio::task::spawn_blocking(move || {
        // Cancellation cannot interrupt blocking extraction. Keep its single
        // writer ownership until publication and cleanup have actually settled.
        let _guard = guard;
        let root = scratch.path().join("payload");
        extract(&archive, &binary.archive, &root, &relative)?;
        verify_target(&root, &relative)?;
        integrity::write(&root, &binary, &digest)?;
        // An interrupted preparation only leaves its unique temporary folder.
        // No incomplete tree is ever exposed under the immutable cache key.
        publish(&root, &directory, &scratch.path().join("previous"))?;
        integrity::verify(&directory, &binary, &digest, &relative)
    })
    .await
    .map_err(|error| invalid(format!("ACP adapter preparation failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn repair_publishes_complete_payload_and_restores_old_tree_on_failure() {
        let cache = tempfile::tempdir().unwrap();
        let preparation = tempfile::tempdir_in(cache.path()).unwrap();
        let installed = cache.path().join("installed");
        std::fs::create_dir(&installed).unwrap();
        std::fs::write(installed.join("agent"), "old").unwrap();
        let previous = preparation.path().join("previous");
        assert!(publish(&preparation.path().join("missing"), &installed, &previous).is_err());
        assert_eq!(std::fs::read(installed.join("agent")).unwrap(), b"old");
        let payload = preparation.path().join("payload");
        std::fs::create_dir(&payload).unwrap();
        std::fs::write(payload.join("agent"), "new").unwrap();
        std::fs::write(payload.join("helper"), "required companion").unwrap();
        publish(&payload, &installed, &previous).unwrap();
        assert_eq!(std::fs::read(installed.join("agent")).unwrap(), b"new");
        assert!(installed.join("helper").is_file());
    }

    #[tokio::test]
    async fn verified_cached_installation_does_not_require_a_download() {
        let cache = tempfile::tempdir().unwrap();
        let binary = BinaryDistribution {
            archive: "https://127.0.0.1:1/agent.zip".into(),
            cmd: "agent".into(),
            sha256: Some("0".repeat(64)),
            ..Default::default()
        };
        let digest = binary.sha256.as_deref().unwrap();
        let key =
            blake3::hash(format!("{}\0{}\0{}", binary.archive, binary.cmd, digest).as_bytes())
                .to_hex()
                .to_string();
        let root = cache.path().join(key);
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("agent"), "executable").unwrap();
        std::fs::write(root.join("helper"), "companion").unwrap();
        verify_target(&root, Path::new("agent")).unwrap();
        integrity::write(&root, &binary, digest).unwrap();
        assert_eq!(
            super::binary(&binary, cache.path(), false).await.unwrap(),
            root.join("agent")
        );
    }

    #[tokio::test]
    async fn damaged_cached_installations_are_not_reused() {
        let scratch = tempfile::tempdir().unwrap();
        let binary = BinaryDistribution {
            archive: "https://127.0.0.1:1/agent.zip".into(),
            cmd: "agent".into(),
            sha256: Some("0".repeat(64)),
            ..Default::default()
        };
        let key = blake3::hash(
            format!(
                "{}\0{}\0{}",
                binary.archive,
                binary.cmd,
                binary.sha256.as_deref().unwrap()
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        let cache = scratch.path().join(key);
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("agent"), "executable fixture").unwrap();
        std::fs::write(cache.join("helper"), "required companion").unwrap();
        std::fs::write(
            cache.join(".nexa-install.json"),
            serde_json::to_vec(
                &serde_json::json!({"url": binary.archive, "sha256": binary.sha256, "bytes": 42}),
            )
            .unwrap(),
        )
        .unwrap();
        std::fs::remove_file(cache.join("helper")).unwrap();
        assert!(
            super::binary(&binary, scratch.path(), false).await.is_err(),
            "a stale marker must not authorize an incomplete cached installation"
        );
    }

    #[tokio::test]
    async fn unpinned_archives_cannot_reuse_cache_or_start_downloads() {
        let scratch = tempfile::tempdir().unwrap();
        for checksum in [None, Some(""), Some("abcd"), Some(&"z".repeat(64))] {
            let binary = BinaryDistribution {
                archive: "https://127.0.0.1:1/agent.zip".into(),
                cmd: "agent".into(),
                sha256: checksum.map(str::to_owned),
                ..Default::default()
            };
            let legacy_key = blake3::hash(
                format!(
                    "{}\0{}\0{}",
                    binary.archive,
                    binary.cmd,
                    checksum.unwrap_or_default()
                )
                .as_bytes(),
            )
            .to_hex()
            .to_string();
            let cache = scratch.path().join(legacy_key);
            std::fs::create_dir_all(&cache).unwrap();
            std::fs::write(cache.join(".nexa-install.json"), "{}").unwrap();
            std::fs::write(cache.join("agent"), "unverified fixture").unwrap();
            let error = super::binary(&binary, scratch.path(), false)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("pinned SHA-256"), "{error}");
        }
    }

    #[test]
    fn zip_install_keeps_adjacent_helpers_and_rejects_path_escape() {
        let scratch = tempfile::tempdir().unwrap();
        for unsafe_entry in [false, true] {
            let archive = scratch.path().join(format!("{unsafe_entry}.zip"));
            let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
            for name in [
                "agy_acp_server.exe",
                "localharness_external.exe",
                if unsafe_entry {
                    "../escape"
                } else {
                    "data/info.json"
                },
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(b"fixture").unwrap();
            }
            zip.finish().unwrap();
            let root = scratch.path().join(format!("output-{unsafe_entry}"));
            let result = extract(
                &archive,
                "https://example.com/agent.zip",
                &root,
                Path::new("agy_acp_server.exe"),
            );
            assert_eq!(result.is_err(), unsafe_entry);
            if !unsafe_entry {
                assert!(root.join("localharness_external.exe").is_file());
            }
            assert!(!scratch.path().join("escape").exists());
        }
    }

    #[test]
    fn archive_command_must_be_a_relative_file() {
        for invalid in [
            "/agent",
            "../agent",
            "C:\\agent.exe",
            "a/../../agent",
            "\\\\host\\agent",
            "agent:stream",
        ] {
            assert!(relative_path(invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            relative_path("./bin/agent").unwrap(),
            Path::new("bin/agent")
        );
    }
}
