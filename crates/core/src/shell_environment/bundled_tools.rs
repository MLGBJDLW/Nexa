//! Pinned utilities shipped with Nexa. No package manager, profile edits or
//! persistent host/WSL installation is involved.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;
use sha2::{Digest, Sha256};

static ROOT: OnceLock<PathBuf> = OnceLock::new();

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    kind: String,
    manifest_version: u32,
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct ManifestFile {
    path: String,
    size: u64,
    sha256: String,
}

fn host_platform() -> String {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

/// Called during app setup before agent sessions are created. The resource
/// manifest and all executables are checked before exposing any PATH entry.
pub fn configure(resource_dir: &Path) -> Result<Option<PathBuf>, String> {
    let root = resource_dir.join("shell-tools");
    if !root.join("manifest.json").is_file() {
        return Ok(None); // Source checkouts can explicitly run shell-tools:bundle.
    }
    validate(&root)?;
    let host = root.join(host_platform());
    if !["rg", "jq"].iter().all(|name| {
        host.join(format!("{name}{}", if cfg!(windows) { ".exe" } else { "" }))
            .is_file()
    }) {
        return Err("Bundled shell tools do not contain the host platform".into());
    }
    ROOT.set(root)
        .map_err(|_| "Bundled shell tools were already configured")?;
    // Process-local PATH is inherited by native agents and their children.
    // This never changes the user's persistent environment or shell profiles.
    let mut paths = vec![host.clone()];
    paths.extend(
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>()),
    );
    let path = std::env::join_paths(paths).map_err(|error| error.to_string())?;
    std::env::set_var("PATH", path);
    Ok(Some(host))
}

fn validate(root: &Path) -> Result<(), String> {
    let manifest: Manifest = serde_json::from_slice(
        &std::fs::read(root.join("manifest.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if manifest.kind != "nexaShellTools"
        || manifest.manifest_version != 1
        || manifest.files.len() > 16
    {
        return Err("Invalid bundled shell tools manifest".into());
    }
    let root = std::fs::canonicalize(root).map_err(|error| error.to_string())?;
    let mut verified = std::collections::HashSet::new();
    for file in manifest.files {
        let path =
            std::fs::canonicalize(root.join(&file.path)).map_err(|error| error.to_string())?;
        if !path.starts_with(&root) || file.size == 0 || file.size > 32 * 1024 * 1024 {
            return Err(
                "Bundled shell tool escapes its resource directory or has an invalid size".into(),
            );
        }
        if std::fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .len()
            != file.size
        {
            return Err("Bundled shell tool has an unexpected size".into());
        }
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        if bytes.len() as u64 != file.size || format!("{:x}", Sha256::digest(&bytes)) != file.sha256
        {
            return Err(format!(
                "Bundled shell tool failed integrity verification: {}",
                file.path
            ));
        }
        if !verified.insert(file.path) {
            return Err("Duplicate bundled shell tool manifest entry".into());
        }
    }
    // A stale or partially staged platform must not become executable just
    // because files happen to exist beside the verified host utilities.
    for platform in ["windows-x64", "linux-x64", "linux-arm64", "darwin-arm64"] {
        if !root.join(platform).exists() {
            continue;
        }
        for tool in ["rg", "jq"] {
            let name = format!(
                "{platform}/{tool}{}",
                if platform.starts_with("windows-") {
                    ".exe"
                } else {
                    ""
                }
            );
            if !verified.contains(&name) {
                return Err(format!(
                    "Bundled shell tool is missing from the verified manifest: {name}"
                ));
            }
        }
    }
    Ok(())
}

pub fn posix_bootstrap(kind: &str) -> String {
    ROOT.get()
        .map(|root| bootstrap_for(root, kind))
        .unwrap_or_default()
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn bootstrap_for(root: &Path, kind: &str) -> String {
    if kind == "wsl" {
        let root = quote(&root.to_string_lossy());
        return format!(
            "if __nexa_tools_root=$(wslpath -u {root} 2>/dev/null); then\n  case $(uname -m) in x86_64) __nexa_tools_bin=\"$__nexa_tools_root/linux-x64\";; aarch64|arm64) __nexa_tools_bin=\"$__nexa_tools_root/linux-arm64\";; *) __nexa_tools_bin=;; esac\n  if [ -n \"$__nexa_tools_bin\" ] && [ -x \"$__nexa_tools_bin/rg\" ] && [ -x \"$__nexa_tools_bin/jq\" ]; then export PATH=\"$__nexa_tools_bin:$PATH\"; fi\nfi\nunset __nexa_tools_root __nexa_tools_bin\n"
        );
    }
    if !matches!(kind, "bash" | "zsh" | "sh" | "dash") {
        return String::new();
    }
    let bin = quote(&root.join(host_platform()).to_string_lossy());
    if cfg!(windows) {
        format!("if __nexa_tools_bin=$(cygpath -u {bin} 2>/dev/null); then export PATH=\"$__nexa_tools_bin:$PATH\"; fi\nunset __nexa_tools_bin\n")
    } else {
        format!("export PATH={bin}:\"$PATH\"\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modified_bundled_binary_is_rejected_before_path_publication() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rg"), b"binary").unwrap();
        let manifest = serde_json::json!({"kind":"nexaShellTools","manifestVersion":1,"files":[{"path":"rg","size":6,"sha256":format!("{:x}", Sha256::digest(b"binary"))}]});
        std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
        validate(dir.path()).unwrap();
        std::fs::write(dir.path().join("rg"), b"change").unwrap();
        assert!(validate(dir.path()).unwrap_err().contains("integrity"));
    }

    #[test]
    fn bash_bootstrap_quotes_paths_and_wsl_selects_linux_architecture() {
        let script = bootstrap_for(Path::new("C:/User's Tools"), "wsl");
        assert!(script.contains("'C:/User'\\''s Tools'"));
        assert!(script.contains("linux-x64") && script.contains("linux-arm64"));
        assert!(!script.contains("rg.exe"));
        assert!(bootstrap_for(Path::new("/tools"), "fish").is_empty());
    }
}
