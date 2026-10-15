//! One launch plan for connection checks, chat, and account probes. Registry
//! distributions supply missing ACP adapters; a similarly named CLI is not ACP.
use super::{ExternalAgentLaunch, ExternalAgentPreset, PackageDistribution};
use crate::error::CoreError;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

mod install;

#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub env_remove: Vec<String>,
    pub prepared: bool,
}

/// npm's Windows .cmd wrappers are unnecessary when their declared Node entry
/// point is present. Native argv works in both stdio and interactive PTYs and
/// preserves literal spaces/Unicode without another command interpreter.
fn native_npm_plan(mut plan: LaunchPlan, preset: &ExternalAgentPreset) -> LaunchPlan {
    if !cfg!(windows)
        || !plan
            .executable
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"))
    {
        return plan;
    }
    let Some(directory) = plan.executable.parent() else {
        return plan;
    };
    let node = directory.join("node.exe");
    let Some(node) = is_executable(&node)
        .then_some(node)
        .or_else(|| find_executable("node", &plan.env))
    else {
        return plan;
    };
    let stem = plan
        .executable
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let entry = if stem.eq_ignore_ascii_case("npx") {
        Some(directory.join("node_modules/npm/bin/npx-cli.js"))
    } else {
        preset
            .distribution
            .as_ref()
            .and_then(|d| d.npx.as_ref())
            .and_then(|package| {
                let name = package
                    .package
                    .rsplit_once('@')
                    .map_or(package.package.as_str(), |(name, _)| name);
                let root = directory
                    .join("node_modules")
                    .join(name)
                    .canonicalize()
                    .ok()?;
                let manifest: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(root.join("package.json")).ok()?).ok()?;
                let bin = manifest["bin"]
                    .as_str()
                    .or_else(|| manifest["bin"][stem].as_str())?;
                let entry = root.join(bin).canonicalize().ok()?;
                entry.starts_with(&root).then_some(entry)
            })
    };
    if let Some(entry) = entry.filter(|path| path.is_file()) {
        plan.executable = node;
        plan.args.insert(0, entry.to_string_lossy().into_owned());
    }
    plan
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidInput(message.into())
}

/// Startup logs are local diagnostics, not a channel for credential disclosure.
pub fn redact_diagnostic(value: &str, secrets: &[String]) -> String {
    let mut value = value.to_string();
    for secret in secrets {
        value = value.replace(secret, "[REDACTED]");
    }
    for pattern in [
        r"\x1b\[[0-?]*[ -/]*[@-~]",
        r#"(?i)(?:authorization|api[_-]?key|access[_-]?token|refresh[_-]?token|password|secret)\s*["']?\s*[:=]\s*[^\r\n,]+"#,
        r"(?i)\b(?:sk-|gh[pousr]_|eyJ)[A-Za-z0-9_./+=-]+",
        r"https?://[^\s]+[?#][^\s]+",
    ] {
        value = regex::Regex::new(pattern)
            .unwrap()
            .replace_all(&value, "[REDACTED]")
            .into_owned();
    }
    let characters = crate::privacy::redact_content(&value, &[])
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<Vec<_>>();
    let bounded = if characters.len() > 2000 {
        characters[..500]
            .iter()
            .copied()
            .chain("\n…\n".chars())
            .chain(characters[characters.len() - 1497..].iter().copied())
            .collect::<String>()
    } else {
        characters.into_iter().collect::<String>()
    };
    bounded.trim().to_string()
}

fn env_value<'a>(env: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    env.iter()
        .find(|(name, _)| {
            if cfg!(windows) {
                name.eq_ignore_ascii_case(key)
            } else {
                name.as_str() == key
            }
        })
        .map(|(_, value)| value.as_str())
}

pub fn set_env(env: &mut BTreeMap<String, String>, key: &str, value: String) {
    if cfg!(windows) {
        env.retain(|name, _| !name.eq_ignore_ascii_case(key));
    }
    env.insert(key.into(), value);
}

fn append_paths(paths: &mut Vec<PathBuf>, value: &std::ffi::OsStr) {
    for path in std::env::split_paths(value) {
        if path.is_absolute() && !paths.contains(&path) {
            paths.push(path);
        }
    }
}

/// Desktop apps inherit a login-time PATH. Include current registered paths and
/// common per-user CLI bins, without consulting the workspace or running a shell.
/// An explicitly configured PATH is authoritative and is never broadened.
pub fn process_environment(overrides: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env = overrides.clone();
    if env_value(overrides, "PATH").is_some() {
        return env;
    }
    let mut paths = Vec::new();
    if let Some(value) = std::env::var_os("PATH") {
        append_paths(&mut paths, &value);
    }
    #[cfg(windows)]
    let environment_reference = regex::Regex::new(r"%([^%]+)%").unwrap();
    #[cfg(windows)]
    for (root, key) in [
        (&windows_registry::CURRENT_USER, "Environment"),
        (
            &windows_registry::LOCAL_MACHINE,
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        ),
    ] {
        if let Ok(key) = root.open(key) {
            if let Ok(value) = key.get_string("Path") {
                // Expand only environment references, never execute shell text.
                let expanded =
                    environment_reference.replace_all(&value, |captures: &regex::Captures<'_>| {
                        std::env::var(&captures[1]).unwrap_or_else(|_| captures[0].into())
                    });
                append_paths(&mut paths, std::ffi::OsStr::new(expanded.as_ref()));
            }
        }
    }
    let mut extra = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for suffix in [
            ".local/bin",
            ".cargo/bin",
            ".bun/bin",
            ".npm-global/bin",
            ".hermes/venv/bin",
            ".hermes/venv/Scripts",
            ".hermes/hermes-agent/venv/bin",
            ".hermes/hermes-agent/venv/Scripts",
        ] {
            extra.push(home.join(suffix));
        }
    }
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            extra.push(PathBuf::from(appdata).join("npm"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            for suffix in ["Microsoft/WinGet/Links", "agy/bin", "Programs/nodejs", "uv"] {
                extra.push(local.join(suffix));
            }
            // Codex desktop keeps versioned native CLI binaries here.
            if let Ok(entries) = std::fs::read_dir(local.join("OpenAI/Codex/bin")) {
                let mut versions: Vec<_> = entries
                    .flatten()
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .map(|entry| {
                        (
                            entry.metadata().and_then(|m| m.modified()).ok(),
                            entry.path(),
                        )
                    })
                    .collect();
                versions.sort_by_key(|entry| std::cmp::Reverse(entry.0));
                extra.extend(versions.into_iter().map(|(_, path)| path));
            }
        }
        if let Some(programs) = std::env::var_os("ProgramFiles") {
            extra.push(PathBuf::from(programs).join("nodejs"));
        }
    }
    #[cfg(unix)]
    extra.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    for path in extra {
        if path.is_absolute() && path.is_dir() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    if let Ok(path) = std::env::join_paths(paths) {
        set_env(&mut env, "PATH", path.to_string_lossy().into_owned());
    }
    env
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn find_executable(command: &str, env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.is_absolute() {
        return is_executable(path).then(|| path.to_path_buf());
    }
    if command.is_empty() || command.contains(['/', '\\']) {
        return None;
    }
    let names = if cfg!(windows) && path.extension().is_none() {
        vec![
            format!("{command}.exe"),
            format!("{command}.cmd"),
            format!("{command}.bat"),
        ]
    } else {
        vec![command.to_string()]
    };
    let value = env_value(env, "PATH")
        .map(OsString::from)
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    std::env::split_paths(&value)
        .filter(|path| path.is_absolute())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| is_executable(path))
}

fn npm_package_name(package: &str) -> &str {
    package
        .rsplit_once('@')
        .filter(|(name, _)| !name.is_empty())
        .map_or(package, |(name, _)| name)
}

fn adapter_args<'a>(
    preset: &ExternalAgentPreset,
    launch: &'a ExternalAgentLaunch,
    defaults: &'a [String],
) -> &'a [String] {
    let args = launch.args.as_deref().unwrap_or(defaults);
    // Older Settings examples exposed the complete runner argv. Accept that
    // prefix for the same package, including a pre-upgrade version, without
    // duplicating it or passing it to a native binary. Explicit executable
    // overrides keep their original argv contract.
    if launch.executable.is_none() && launch.args.is_some() {
        if let Some(distribution) = &preset.distribution {
            if let Some(package) = &distribution.npx {
                if args.first().map(String::as_str) == Some("--yes")
                    && args.get(1).is_some_and(|argument| {
                        npm_package_name(argument) == npm_package_name(&package.package)
                    })
                {
                    return &args[2..];
                }
            }
            if let Some(package) = &distribution.uvx {
                if args.first().is_some_and(|argument| {
                    argument.split(['=', '@']).next() == package.package.split(['=', '@']).next()
                }) {
                    return &args[1..];
                }
            }
        }
    }
    args
}

fn package_plan(
    preset: &ExternalAgentPreset,
    runner: &str,
    package: &PackageDistribution,
    launch: &ExternalAgentLaunch,
    env: &BTreeMap<String, String>,
) -> Option<LaunchPlan> {
    let mut env = env.clone();
    for (key, value) in &package.env {
        set_env(&mut env, key, value.clone());
    }
    for (key, value) in &launch.env {
        set_env(&mut env, key, value.clone());
    }
    let executable = find_executable(runner, &env)?;
    let mut args = package.runner_args.clone();
    args.extend(if runner == "npx" {
        vec!["--yes".into(), package.package.clone()]
    } else {
        vec![package.package.clone()]
    });
    args.extend_from_slice(adapter_args(preset, launch, &package.args));
    Some(LaunchPlan {
        executable,
        args,
        env,
        env_remove: Vec::new(),
        prepared: true,
    })
}

/// Resolve installed commands and package runners. Preparation is separate so a
/// read-only quota poll never silently downloads or starts a different adapter.
fn binary_plan(
    preset: &ExternalAgentPreset,
    launch: &ExternalAgentLaunch,
    executable: PathBuf,
    prepared: bool,
) -> LaunchPlan {
    let binary = preset.platform_launch().expect("platform binary selected");
    LaunchPlan {
        executable,
        args: adapter_args(preset, launch, &binary.args).to_vec(),
        env: process_environment(&preset.launch_env(launch)),
        env_remove: Vec::new(),
        prepared,
    }
}

pub fn installed(
    preset: &ExternalAgentPreset,
    launch: &ExternalAgentLaunch,
) -> Result<Option<LaunchPlan>, CoreError> {
    launch.validate_resolved()?;
    let env = process_environment(&preset.launch_env(launch));
    if let Some(executable) = launch
        .executable
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(LaunchPlan {
            executable: executable.into(),
            args: preset.launch_args(launch).to_vec(),
            env,
            env_remove: Vec::new(),
            prepared: false,
        }));
    }
    if preset.managed_distribution {
        // A selected release must not silently fall back to an older CLI on
        // PATH. An explicit executable above remains user-owned.
        if preset.platform_launch().is_some() {
            return Ok(None);
        }
        if let Some(distribution) = &preset.distribution {
            for (runner, package) in [("npx", &distribution.npx), ("uvx", &distribution.uvx)] {
                if let Some(plan) = package
                    .as_ref()
                    .and_then(|package| package_plan(preset, runner, package, launch, &env))
                {
                    return Ok(Some(native_npm_plan(plan, preset)));
                }
            }
        }
        return Ok(None);
    }
    if let Some(binary) = preset.platform_launch() {
        let command = binary.cmd.replace('\\', "/");
        if let Some(executable) =
            find_executable(command.rsplit('/').next().unwrap_or_default(), &env)
        {
            return Ok(Some(binary_plan(preset, launch, executable, false)));
        }
    }
    let command = preset.launch_command();
    let declared_runner = preset.distribution.as_ref().is_some_and(|distribution| {
        (command == "npx" && distribution.npx.is_some())
            || (command == "uvx" && distribution.uvx.is_some())
    });
    let executable = if declared_runner {
        None
    } else {
        find_executable(&command, &env)
    };
    if let Some(executable) = executable {
        return Ok(Some(native_npm_plan(
            LaunchPlan {
                executable,
                args: preset.launch_args(launch).to_vec(),
                env,
                env_remove: Vec::new(),
                prepared: matches!(preset.command.as_str(), "npx" | "uvx"),
            },
            preset,
        )));
    }
    if let Some(distribution) = &preset.distribution {
        for (runner, package) in [("npx", &distribution.npx), ("uvx", &distribution.uvx)] {
            if let Some(plan) = package
                .as_ref()
                .and_then(|package| package_plan(preset, runner, package, launch, &env))
            {
                return Ok(Some(native_npm_plan(plan, preset)));
            }
        }
    }
    Ok(None)
}

pub async fn prepare(
    preset: &ExternalAgentPreset,
    launch: &ExternalAgentLaunch,
    cache_root: Option<&Path>,
) -> Result<LaunchPlan, CoreError> {
    if let Some(plan) = installed(preset, launch)? {
        return Ok(plan);
    }
    if let Some(binary) = preset
        .platform_launch()
        .filter(|binary| !binary.archive.is_empty())
    {
        let cache_root = cache_root.ok_or_else(|| {
            invalid("The managed external-agent adapter directory is unavailable")
        })?;
        // A publisher may retain a rolling URL across versions. Installation
        // ownership must still distinguish the exact selected descriptor.
        let version_root = cache_root.join(super::registry::launch_identity(preset));
        let executable = install::binary(binary, &version_root, preset.official_registry).await?;
        return Ok(binary_plan(preset, launch, executable, true));
    }
    let hint = if preset
        .distribution
        .as_ref()
        .is_some_and(|d| d.npx.is_some())
    {
        "Install Node.js (including npx), or choose an installed ACP adapter executable."
    } else if preset
        .distribution
        .as_ref()
        .is_some_and(|d| d.uvx.is_some())
    {
        "Install uv (including uvx), or choose an installed ACP adapter executable."
    } else if preset
        .distribution
        .as_ref()
        .is_some_and(|d| !d.binary.is_empty())
    {
        "The registry does not publish this agent for your platform. Choose a compatible ACP executable."
    } else {
        "Install its ACP-capable CLI, or choose its absolute executable path in Settings."
    };
    Err(invalid(format!(
        "{}: '{}' was not found. {hint}",
        preset.name,
        preset.launch_command()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_startup_diagnostics_keep_the_final_cause_after_redaction() {
        let log = format!(
            "Installing adapter\n{}\nAPI_KEY=secret-value\nModuleNotFoundError: missing_runtime",
            "loading dependency\n".repeat(300)
        );
        let output = redact_diagnostic(&log, &[]);
        assert!(output.starts_with("Installing adapter"));
        assert!(output.ends_with("ModuleNotFoundError: missing_runtime"));
        assert!(!output.contains("secret-value"));
        assert!(output.chars().count() <= 2000);
    }

    #[test]
    fn startup_errors_retain_actionable_causes_without_credentials() {
        let value = redact_diagnostic("Cannot find module 'agent-adapter'\nAPI_KEY=very-secret-key\ncustom-profile-value\nhttps://login.example/oauth?code=secret\nBearer sk-exampletoken", &["custom-profile-value".into()]);
        assert!(value.contains("Cannot find module 'agent-adapter'"));
        for secret in [
            "very-secret-key",
            "custom-profile-value",
            "code=secret",
            "sk-exampletoken",
        ] {
            assert!(!value.contains(secret));
        }
    }

    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        });
        std::fs::write(&path, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    #[test]
    fn configured_path_is_used_by_discovery_and_the_child_without_cwd_search() {
        let dir = tempfile::tempdir().unwrap();
        let path = executable(dir.path(), "fixture-agent");
        let env = BTreeMap::from([("PATH".into(), dir.path().to_string_lossy().into())]);
        let launch = ExternalAgentLaunch {
            working_directory: dir.path().to_string_lossy().into(),
            env: env.clone(),
            ..Default::default()
        };
        let preset = ExternalAgentPreset {
            command: "fixture-agent".into(),
            ..Default::default()
        };
        let plan = installed(&preset, &launch).unwrap().unwrap();
        assert_eq!(plan.executable, path);
        assert_eq!(plan.env.get("PATH"), env.get("PATH"));
        assert!(find_executable(
            "fixture-agent",
            &BTreeMap::from([("PATH".into(), ".".into())])
        )
        .is_none());
    }

    #[test]
    fn missing_claude_and_codex_adapters_use_the_reviewed_pinned_package() {
        let dir = tempfile::tempdir().unwrap();
        let runner = executable(dir.path(), "npx");
        let launch = ExternalAgentLaunch {
            working_directory: dir.path().to_string_lossy().into(),
            env: BTreeMap::from([("PATH".into(), dir.path().to_string_lossy().into())]),
            ..Default::default()
        };
        for provider in ["claude_code_acp", "codex_acp", "gemini_cli", "qwen_code"] {
            let mut preset = super::super::preset(provider).unwrap();
            preset.distribution.as_mut().unwrap().binary.clear();
            let plan = installed(&preset, &launch).unwrap().unwrap();
            assert_eq!(plan.executable, runner);
            let package = preset.distribution.as_ref().unwrap().npx.as_ref().unwrap();
            assert_eq!(&plan.args[..2], &["--yes", package.package.as_str()]);
            assert_eq!(plan.args[2..], package.args);
        }
    }

    #[test]
    fn reviewed_runner_constraints_remain_separate_from_native_adapter_arguments() {
        let dir = tempfile::tempdir().unwrap();
        executable(dir.path(), "uvx");
        let mut launch = ExternalAgentLaunch {
            working_directory: dir.path().to_string_lossy().into(),
            env: BTreeMap::from([("PATH".into(), dir.path().to_string_lossy().into())]),
            args: Some(vec!["acp".into(), "--verbose".into()]),
            ..Default::default()
        };
        let preset = super::super::preset("acp_minion_code").unwrap();
        let plan = installed(&preset, &launch).unwrap().unwrap();
        assert_eq!(
            plan.args,
            vec![
                "--with",
                "agent-client-protocol==0.8.1",
                "minion-code@0.1.44",
                "acp",
                "--verbose"
            ]
        );
        launch.args = Some(vec![
            "minion-code@0.1.43".into(),
            "acp".into(),
            "--verbose".into(),
        ]);
        assert_eq!(
            installed(&preset, &launch).unwrap().unwrap().args,
            plan.args
        );
    }

    #[test]
    fn custom_agent_arguments_keep_the_declared_package_runner_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let runner = executable(dir.path(), "npx");
        let mut launch = ExternalAgentLaunch {
            working_directory: dir.path().to_string_lossy().into(),
            env: BTreeMap::from([("PATH".into(), dir.path().to_string_lossy().into())]),
            args: Some(vec!["acp".into(), "--custom-option".into()]),
            ..Default::default()
        };
        for provider in ["acp_kilo", "acp_sigit"] {
            let preset = super::super::preset(provider).unwrap();
            let mut package_preset = preset.clone();
            package_preset.distribution.as_mut().unwrap().binary.clear();
            let plan = installed(&package_preset, &launch).unwrap().unwrap();
            let package = preset.distribution.as_ref().unwrap().npx.as_ref().unwrap();
            assert_eq!(plan.executable, runner);
            assert_eq!(
                plan.args,
                vec!["--yes", &package.package, "acp", "--custom-option"]
            );
            let original = launch.args.take().unwrap();
            launch.args = Some(plan.args.clone());
            assert_eq!(
                installed(&package_preset, &launch).unwrap().unwrap().args,
                plan.args
            );
            launch.args.as_mut().unwrap()[1] =
                format!("{}@0.0.1", npm_package_name(&package.package));
            assert_eq!(
                installed(&package_preset, &launch).unwrap().unwrap().args,
                plan.args
            );
            assert_eq!(
                binary_plan(&preset, &launch, dir.path().join("native"), true).args,
                original
            );
            launch.args = Some(original);
        }
    }

    #[test]
    fn native_distribution_uses_its_own_arguments_instead_of_npx_argv() {
        let dir = tempfile::tempdir().unwrap();
        let mut launch = ExternalAgentLaunch {
            working_directory: dir.path().to_string_lossy().into_owned(),
            env: BTreeMap::from([("PATH".into(), dir.path().to_string_lossy().into_owned())]),
            ..Default::default()
        };
        for provider in ["acp_kilo", "acp_sigit"] {
            let mut preset = super::super::preset(provider).unwrap();
            preset.managed_distribution = false;
            let Some(binary) = preset.platform_launch() else {
                continue;
            };
            let command = binary.cmd.replace('\\', "/");
            let name = command.rsplit('/').next().unwrap();
            let path = dir.path().join(name);
            std::fs::write(&path, b"fixture").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let installed = installed(&preset, &launch).unwrap().unwrap();
            assert_eq!(installed.executable, path);
            assert_eq!(installed.args, binary.args);
            assert_eq!(
                binary_plan(&preset, &launch, path.clone(), true).args,
                binary.args
            );
            launch.args = Some(vec!["custom".into()]);
            assert_eq!(
                binary_plan(&preset, &launch, path, true).args,
                vec!["custom"]
            );
            launch.args = None;
        }
    }

    #[test]
    fn selected_official_binary_never_reuses_a_different_path_install() {
        let root = tempfile::tempdir().unwrap();
        let preset = super::super::preset("acp_antigravity_acp").unwrap();
        let Some(binary) = preset.platform_launch() else {
            return;
        };
        let path = root
            .path()
            .join(Path::new(&binary.cmd).file_name().unwrap());
        std::fs::write(&path, b"older user-installed adapter").unwrap();
        let mut launch = ExternalAgentLaunch {
            working_directory: root.path().to_string_lossy().into(),
            env: BTreeMap::from([("PATH".into(), root.path().to_string_lossy().into())]),
            ..Default::default()
        };
        assert!(preset.managed_distribution);
        assert!(installed(&preset, &launch).unwrap().is_none());
        launch.executable = Some(path.to_string_lossy().into());
        assert_eq!(
            installed(&preset, &launch).unwrap().unwrap().executable,
            path
        );
        let mut next = preset.clone();
        next.registry_version = Some("999.0.0".into());
        assert_ne!(
            super::super::registry::launch_identity(&preset),
            super::super::registry::launch_identity(&next)
        );
        // The identity also names the version cache. Exercise real directory
        // creation so Windows-invalid separators cannot break binary installs.
        for selected in [&preset, &next] {
            let identity = super::super::registry::launch_identity(selected);
            assert!(identity
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-'));
            let cache = root.path().join(identity);
            std::fs::create_dir(&cache).unwrap();
            std::fs::write(cache.join("install-receipt"), b"selected version").unwrap();
        }
    }

    #[test]
    fn explicit_executable_and_empty_args_are_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = executable(dir.path(), "my-agent");
        let launch = ExternalAgentLaunch {
            executable: Some(path.to_string_lossy().into()),
            working_directory: dir.path().to_string_lossy().into(),
            args: Some(vec![]),
            ..Default::default()
        };
        let plan = installed(&super::super::preset("codex_acp").unwrap(), &launch)
            .unwrap()
            .unwrap();
        assert_eq!(plan.executable, path);
        assert!(plan.args.is_empty());
        assert!(!plan.prepared);
    }
}
