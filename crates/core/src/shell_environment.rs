//! Shared shell discovery and launch plans for agent commands and terminals.
//! Saved identifiers are preferences, never executable paths or shell fragments.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub mod wsl_process;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShellProfile {
    pub id: String,
    pub label: String,
    pub program: String,
    pub kind: String,
    pub distribution: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellDiscovery {
    pub profiles: Vec<ShellProfile>,
    pub default_profile_id: String,
    pub warnings: Vec<String>,
}

fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}

fn find_program(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return executable(path).then(|| path.to_path_buf());
    }
    let filename = if cfg!(windows) && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|p| p.join(&filename))
        .find(|p| executable(p))
}

fn local_profile(id: &str, label: &str, program: &str) -> Option<ShellProfile> {
    let program = find_program(program)?;
    Some(ShellProfile {
        id: id.into(),
        label: label.into(),
        program: program.to_string_lossy().into_owned(),
        kind: id.into(),
        distribution: None,
    })
}

fn local_profiles() -> Vec<ShellProfile> {
    let mut profiles = Vec::new();
    #[cfg(windows)]
    {
        for (id, label, program) in [
            ("pwsh", "PowerShell 7", "pwsh"),
            ("powershell", "Windows PowerShell", "powershell"),
            ("cmd", "Command Prompt", "cmd"),
            ("sh", "sh", "sh"),
        ] {
            if let Some(profile) = local_profile(id, label, program) {
                profiles.push(profile);
            }
        }
        // System32/bash.exe is the legacy WSL launcher, not Git Bash.
        let bash = find_program("bash")
            .filter(|p| {
                !p.to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("system32")
            })
            .or_else(|| {
                ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"]
                    .into_iter()
                    .filter_map(std::env::var_os)
                    .flat_map(|root| {
                        let root = PathBuf::from(root);
                        [
                            root.join("Git/bin/bash.exe"),
                            root.join("Programs/Git/bin/bash.exe"),
                        ]
                    })
                    .find(|p| executable(p))
            });
        if let Some(program) = bash {
            let sh = program.with_file_name("sh.exe");
            if !profiles.iter().any(|p| p.id == "sh") && executable(&sh) {
                profiles.push(ShellProfile {
                    id: "sh".into(),
                    label: "sh".into(),
                    program: sh.to_string_lossy().into_owned(),
                    kind: "sh".into(),
                    distribution: None,
                });
            }
            profiles.push(ShellProfile {
                id: "bash".into(),
                label: "Git Bash".into(),
                program: program.to_string_lossy().into_owned(),
                kind: "bash".into(),
                distribution: None,
            });
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            if let Some(mut profile) = local_profile("login", "Login shell", &shell) {
                profile.kind = Path::new(&shell)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                // Only shells with a known command and interactive contract.
                if matches!(
                    profile.kind.as_str(),
                    "bash" | "zsh" | "sh" | "dash" | "fish" | "pwsh"
                ) {
                    profiles.push(profile);
                }
            }
        }
        for (id, label) in [
            ("bash", "Bash"),
            ("zsh", "Zsh"),
            ("sh", "sh"),
            ("pwsh", "PowerShell"),
        ] {
            if let Some(profile) = local_profile(id, label, id) {
                profiles.push(profile);
            }
        }
    }
    profiles
}

/// No subprocess is started on the command path. Missing explicit choices fail
/// visibly; they must never silently execute in a different OS or interpreter.
pub fn resolve_profile(preference: &str) -> Result<ShellProfile, String> {
    let preference = preference.trim();
    if let Some(distribution) = preference.strip_prefix("wsl:") {
        if !cfg!(windows) {
            return Err("WSL is only available on Windows".into());
        }
        if distribution.is_empty() || distribution.chars().any(char::is_control) {
            return Err("Invalid WSL distribution".into());
        }
        let program = find_program("wsl")
            .ok_or("WSL is unavailable. Refresh shell environments in Settings.")?;
        return Ok(ShellProfile {
            id: preference.into(),
            label: format!("WSL · {distribution}"),
            program: program.to_string_lossy().into_owned(),
            kind: "wsl".into(),
            distribution: Some(distribution.into()),
        });
    }
    let profiles = local_profiles();
    let profile = if preference.is_empty() || matches!(preference, "default" | "auto") {
        profiles.into_iter().next()
    } else {
        profiles.into_iter().find(|p| p.id == preference)
    };
    profile.ok_or_else(|| {
        format!("Shell '{preference}' is unavailable. Refresh shell environments in Settings.")
    })
}

pub async fn discover_shells() -> ShellDiscovery {
    let mut result = ShellDiscovery {
        profiles: local_profiles(),
        default_profile_id: String::new(),
        warnings: Vec::new(),
    };
    result.default_profile_id = result
        .profiles
        .first()
        .map(|p| p.id.clone())
        .unwrap_or_default();
    #[cfg(windows)]
    if let Some(program) = find_program("wsl") {
        let mut command = tokio::process::Command::new(&program);
        command.args(["--list", "--quiet"]).kill_on_drop(true);
        crate::background_process::configure_tokio_background(&mut command);
        match tokio::time::timeout(std::time::Duration::from_secs(4), command.output()).await {
            Ok(Ok(output)) if output.status.success() => {
                use futures::{stream, StreamExt};
                let program = program.to_string_lossy().into_owned();
                let checks = stream::iter(parse_wsl_distributions(&output.stdout).into_iter().enumerate()).map(|(index, distribution)| {
                    let program = program.clone();
                    async move {
                        let mut probe = tokio::process::Command::new(&program);
                        probe.args(["--distribution", &distribution, "--exec", "bash", "-c", "command -v setsid >/dev/null"])
                            .kill_on_drop(true).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                        crate::background_process::configure_tokio_background(&mut probe);
                        let ready = matches!(tokio::time::timeout(std::time::Duration::from_secs(8), probe.status()).await, Ok(Ok(status)) if status.success());
                        (index, distribution, ready)
                    }
                }).buffer_unordered(4).collect::<Vec<_>>().await;
                let mut checks = checks;
                checks.sort_by_key(|(index, _, _)| *index);
                for (_, distribution, ready) in checks {
                    if !ready {
                        result.warnings.push(format!("WSL distribution '{distribution}' did not provide a ready Bash environment."));
                        continue;
                    }
                    result.profiles.push(ShellProfile { id: format!("wsl:{distribution}"), label: format!("WSL · {distribution}"), program: program.clone(), kind: "wsl".into(), distribution: Some(distribution) });
                }
            }
            _ => result.warnings.push("WSL discovery did not complete. Local shells are still available; retry to refresh WSL.".into()),
        }
    }
    result
}

#[cfg(any(windows, test))]
fn parse_wsl_distributions(bytes: &[u8]) -> Vec<String> {
    let text = if bytes.starts_with(&[0xff, 0xfe])
        || bytes.iter().skip(1).step_by(2).take(16).any(|b| *b == 0)
    {
        let words: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16_lossy(&words)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    let mut names = Vec::new();
    for name in text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
    {
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

impl ShellProfile {
    pub fn invocation(
        &self,
        command: Option<&str>,
        cwd: &Path,
    ) -> Result<(String, Vec<String>), String> {
        let args: Vec<String> = match self.kind.as_str() {
            "wsl" => {
                // wsl --cd accepts an absolute Windows path and honours distro
                // automount configuration; do not assume the /mnt drive prefix.
                let cwd = cwd.to_string_lossy();
                let cwd = cwd.strip_prefix(r"\\?\").unwrap_or(&cwd);
                let mut args = vec![
                    "--distribution".into(),
                    self.distribution
                        .clone()
                        .ok_or("Missing WSL distribution")?,
                    "--cd".into(),
                    cwd.into(),
                    "--exec".into(),
                    "bash".into(),
                ];
                if let Some(command) = command {
                    args.extend(["-lc".into(), command.into()]);
                } else {
                    args.push("-il".into());
                }
                args
            }
            "powershell" | "pwsh" => {
                let mut args = vec!["-NoLogo".into()];
                if let Some(command) = command {
                    args.extend([
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        command.into(),
                    ]);
                }
                args
            }
            "cmd" => command
                .map(|c| vec!["/D".into(), "/S".into(), "/C".into(), c.into()])
                .unwrap_or_default(),
            "bash" | "zsh" => command
                .map(|c| vec!["-lc".into(), c.into()])
                .unwrap_or_else(|| vec!["-il".into()]),
            _ => command
                .map(|c| vec!["-c".into(), c.into()])
                .unwrap_or_default(),
        };
        Ok((self.program.clone(), args))
    }
}

pub fn agent_guidance(preference: &str, restricted: bool) -> String {
    format!("## Command environment\nHost OS: {}. Saved shell preference: {}. {} Use run_shell command for the selected shell (including WSL Bash); cwd is always an existing host path and is mapped by the runtime. Inside a WSL command use Linux paths and installed Linux tools. program plus args remains exact host argv; explicit shell overrides the preference. Do not assume Windows executables or dependencies exist inside WSL. Existing terminal sessions retain their original environment.", std::env::consts::OS, serde_json::to_string(preference).unwrap_or_default(), if restricted { "Restricted mode keeps native host argv and does not activate shell execution." } else { "The preference applies to command strings and new terminals." })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn windows_sh_remains_selectable_when_installed() {
        if find_program("sh").is_some() {
            let sh = resolve_profile("sh").unwrap();
            assert_eq!(
                sh.invocation(Some("printf ok"), Path::new("C:/"))
                    .unwrap()
                    .1,
                ["-c", "printf ok"]
            );
        }
    }
    #[test]
    fn wsl_discovery_decodes_unicode_and_deduplicates() {
        let bytes: Vec<u8> = "\u{feff}Ubuntu\r\n工作环境\r\nUbuntu\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(parse_wsl_distributions(&bytes), ["Ubuntu", "工作环境"]);
        assert_eq!(
            parse_wsl_distributions(b"Ubuntu\nDebian\n"),
            ["Ubuntu", "Debian"]
        );
    }
    #[test]
    fn wsl_plan_preserves_case_paths_and_literal_script() {
        let profile = ShellProfile {
            id: "wsl:Ubuntu".into(),
            label: "WSL".into(),
            kind: "wsl".into(),
            program: "wsl.exe".into(),
            distribution: Some("Ubuntu".into()),
        };
        let (_, args) = profile
            .invocation(Some("printf '%s' \"a;b\""), Path::new(r"\\?\D:\My Project"))
            .unwrap();
        assert_eq!(
            args,
            [
                "--distribution",
                "Ubuntu",
                "--cd",
                r"D:\My Project",
                "--exec",
                "bash",
                "-lc",
                "printf '%s' \"a;b\""
            ]
        );
        assert!(profile
            .invocation(None, Path::new("D:/work"))
            .unwrap()
            .1
            .ends_with(&["bash".into(), "-il".into()]));
    }
    #[test]
    fn unknown_preference_does_not_fall_back() {
        assert!(resolve_profile("not-a-shell").is_err());
        assert!(resolve_profile("wsl:").is_err());
    }
}
