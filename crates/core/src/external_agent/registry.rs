//! The official ACP directory supplies versions, never credentials or models.
//! Checking a directory does not replace an installed adapter. Activation is
//! explicit and the exact selected descriptor survives offline restarts.
use super::{bundled_presets, AgentDistribution, ExternalAgentLaunch, ExternalAgentPreset};
use crate::error::CoreError;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const SOURCE: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const MAX_BYTES: usize = 4 * 1024 * 1024;
const REFRESH_SECONDS: u64 = 3600;

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::InvalidInput(message.into())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    version: u32,
    checked_at: Option<u64>,
    available: Vec<ExternalAgentPreset>,
    selected: BTreeMap<String, ExternalAgentPreset>,
}
#[derive(Default)]
struct State {
    directory: Option<PathBuf>,
    cache: Cache,
}
fn state() -> &'static RwLock<State> {
    static STATE: OnceLock<RwLock<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}
fn operation() -> &'static tokio::sync::Mutex<()> {
    static OPERATION: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    OPERATION.get_or_init(Default::default)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    pub preset: ExternalAgentPreset,
    pub available_version: Option<String>,
    pub update_available: bool,
    pub fingerprint: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySnapshot {
    pub source: &'static str,
    pub checked_at: Option<u64>,
    pub entries: Vec<RegistryEntry>,
}

fn fingerprint(preset: &ExternalAgentPreset) -> String {
    // Presentation changes do not invalidate an adapter or an existing session.
    blake3::hash(
        serde_json::json!([
            preset.registry_id,
            preset.registry_version,
            preset.distribution
        ])
        .to_string()
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}
pub fn launch_identity(preset: &ExternalAgentPreset) -> String {
    format!("{}-{}", fingerprint(preset), preset.managed_distribution)
}

impl Cache {
    fn retain_first_discovered_versions(&mut self) {
        // Bundled entries already have a stable initial selection. A newly
        // discovered agent needs the same anchor before later directory checks;
        // selecting a descriptor does not imply installation or authentication.
        for preset in &self.available {
            if !bundled_presets()
                .iter()
                .any(|entry| entry.provider == preset.provider)
            {
                self.selected
                    .entry(preset.provider.clone())
                    .or_insert_with(|| preset.clone());
            }
        }
    }

    fn resolve(&self, provider: &str) -> Option<ExternalAgentPreset> {
        self.selected
            .get(provider)
            .or_else(|| {
                bundled_presets()
                    .iter()
                    .find(|preset| preset.provider == provider)
            })
            .or_else(|| {
                self.available
                    .iter()
                    .find(|preset| preset.provider == provider)
            })
            .cloned()
    }
    fn snapshot(&self) -> RegistrySnapshot {
        let mut providers: Vec<_> = bundled_presets()
            .iter()
            .map(|preset| preset.provider.clone())
            .collect();
        for preset in self.available.iter().chain(self.selected.values()) {
            if !providers.contains(&preset.provider) {
                providers.push(preset.provider.clone());
            }
        }
        let entries = providers
            .into_iter()
            .filter_map(|provider| {
                let preset = self.resolve(&provider)?;
                let latest = self
                    .available
                    .iter()
                    .find(|entry| entry.provider == provider);
                Some(RegistryEntry {
                    available_version: latest.and_then(|entry| entry.registry_version.clone()),
                    update_available: latest
                        .is_some_and(|entry| fingerprint(entry) != fingerprint(&preset)),
                    fingerprint: latest.map(fingerprint),
                    preset,
                })
            })
            .collect();
        RegistrySnapshot {
            source: SOURCE,
            checked_at: self.checked_at,
            entries,
        }
    }
}
pub fn snapshot() -> RegistrySnapshot {
    state()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .cache
        .snapshot()
}
pub fn resolve(provider: &str) -> Option<ExternalAgentPreset> {
    state()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .cache
        .resolve(provider)
}

fn safe_https(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
fn validate_distribution(preset: &ExternalAgentPreset) -> Result<(), CoreError> {
    let distribution = preset
        .distribution
        .as_ref()
        .ok_or_else(|| invalid("ACP directory entry has no distribution"))?;
    if distribution.binary.is_empty() && distribution.npx.is_none() && distribution.uvx.is_none() {
        return Err(invalid("ACP directory entry has no launch method"));
    }
    for binary in distribution.binary.values() {
        if !safe_https(&binary.archive)
            || binary.cmd.is_empty()
            || binary.cmd.len() > 1024
            || binary.cmd.contains(['\0', ':'])
            || Path::new(&binary.cmd).is_absolute()
            || binary
                .cmd
                .replace('\\', "/")
                .split('/')
                .any(|part| part == "..")
            || binary.sha256.as_ref().is_some_and(|hash| {
                hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(invalid(
                "ACP directory contains an invalid binary distribution",
            ));
        }
        ExternalAgentLaunch {
            args: Some(binary.args.clone()),
            env: binary.env.clone(),
            ..Default::default()
        }
        .validate()?;
    }
    for (kind, package) in [("npx", &distribution.npx), ("uvx", &distribution.uvx)] {
        let Some(package) = package else {
            continue;
        };
        let pin = if kind == "npx" {
            package.package.rsplit_once('@')
        } else {
            package
                .package
                .split_once("==")
                .or_else(|| package.package.rsplit_once('@'))
        };
        let pinned = pin.is_some_and(|(name, version)| {
            !name.is_empty()
                && version.as_bytes().first().is_some_and(u8::is_ascii_digit)
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".+_-".contains(&byte))
        });
        if !pinned
            || package.package.len() > 512
            || package.package.chars().any(char::is_whitespace)
            || package.package.contains(['\0', '\\', ':'])
        {
            return Err(invalid("ACP directory packages must use an exact version"));
        }
        ExternalAgentLaunch {
            args: Some(package.args.clone()),
            env: package.env.clone(),
            ..Default::default()
        }
        .validate()?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct Directory {
    version: String,
    agents: Vec<DirectoryAgent>,
}
#[derive(Deserialize)]
struct DirectoryAgent {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    website: Option<String>,
    repository: Option<String>,
    distribution: AgentDistribution,
}

fn normalize(bytes: &[u8]) -> Result<Vec<ExternalAgentPreset>, CoreError> {
    let directory: Directory = serde_json::from_slice(bytes)?;
    if directory.version != "1.0.0" || directory.agents.is_empty() || directory.agents.len() > 500 {
        return Err(invalid("Unsupported ACP directory format"));
    }
    let mut seen = HashSet::new();
    directory
        .agents
        .into_iter()
        .map(|entry| {
            if !identifier(&entry.id)
                || !seen.insert(entry.id.clone())
                || entry.version.is_empty()
                || entry.version.len() > 128
                || entry.version.chars().any(char::is_control)
            {
                return Err(invalid(
                    "ACP directory contains an invalid or duplicate identity",
                ));
            }
            let previous = bundled_presets()
                .iter()
                .find(|preset| preset.registry_id.as_deref() == Some(&entry.id));
            let mut distribution = entry.distribution;
            // Feed-supplied runner arguments are never interpreted as package
            // manager flags. Preserve our compatibility pin for identical releases.
            for (kind, package) in [
                ("npx", &mut distribution.npx),
                ("uvx", &mut distribution.uvx),
            ] {
                if let Some(package) = package {
                    package.runner_args.clear();
                    let pinned = previous
                        .and_then(|preset| preset.distribution.as_ref())
                        .and_then(|d| {
                            if kind == "npx" {
                                d.npx.as_ref()
                            } else {
                                d.uvx.as_ref()
                            }
                        });
                    if let Some(pinned) = pinned.filter(|pinned| pinned.package == package.package)
                    {
                        package.runner_args.clone_from(&pinned.runner_args);
                    }
                }
            }
            for (platform, binary) in &mut distribution.binary {
                if binary.sha256.is_none() {
                    if let Some(pinned) = previous
                        .and_then(|preset| preset.distribution.as_ref())
                        .and_then(|d| d.binary.get(platform))
                        .filter(|pinned| {
                            pinned.archive == binary.archive && pinned.cmd == binary.cmd
                        })
                    {
                        binary.sha256.clone_from(&pinned.sha256);
                    }
                }
            }
            let (command, args, env) = if let Some(package) = distribution.npx.as_ref() {
                (
                    "npx".into(),
                    [
                        &["--yes".into(), package.package.clone()],
                        package.args.as_slice(),
                    ]
                    .concat(),
                    package.env.clone(),
                )
            } else if let Some(package) = distribution.uvx.as_ref() {
                (
                    "uvx".into(),
                    [&[package.package.clone()], package.args.as_slice()].concat(),
                    package.env.clone(),
                )
            } else if let Some(binary) = distribution.binary.values().next() {
                (
                    binary
                        .cmd
                        .replace('\\', "/")
                        .rsplit('/')
                        .next()
                        .unwrap_or_default()
                        .into(),
                    binary.args.clone(),
                    binary.env.clone(),
                )
            } else {
                return Err(invalid("ACP directory entry has no launch method"));
            };
            let docs_url = entry
                .website
                .or(entry.repository)
                .unwrap_or_else(|| "https://agentclientprotocol.com/overview/agents".into());
            if !safe_https(&docs_url) {
                return Err(invalid(
                    "ACP directory contains an invalid documentation URL",
                ));
            }
            let preset = ExternalAgentPreset {
                id: previous
                    .map(|p| p.id.clone())
                    .unwrap_or_else(|| format!("acp-{}", entry.id)),
                provider: previous
                    .map(|p| p.provider.clone())
                    .unwrap_or_else(|| format!("acp_{}", entry.id.replace('-', "_"))),
                name: entry
                    .name
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(120)
                    .collect(),
                description: entry
                    .description
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(800)
                    .collect(),
                icon: previous
                    .map(|p| p.icon.clone())
                    .unwrap_or_else(|| "/provider-icons/agents/custom.svg".into()),
                command,
                args,
                env,
                docs_url,
                registry_id: Some(entry.id),
                registry_version: Some(entry.version),
                distribution: Some(distribution),
                official_registry: true,
                managed_distribution: true,
            };
            validate_distribution(&preset)?;
            Ok(preset)
        })
        .collect()
}

fn write_cache(directory: &Path, cache: &Cache) -> Result<(), CoreError> {
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec(cache)?)?;
    file.as_file().sync_all()?;
    file.persist(directory.join("registry.json"))
        .map_err(|error| CoreError::Io(error.error))?;
    Ok(())
}

pub fn initialize(directory: &Path) -> Result<(), CoreError> {
    // Even an unreadable old cache must leave the bundled fallback and a
    // usable destination for an explicit refresh.
    {
        let mut state = state().write().unwrap_or_else(|e| e.into_inner());
        state.directory = Some(directory.to_owned());
        state.cache = Cache {
            version: 1,
            ..Default::default()
        };
    }
    let mut loaded = Cache {
        version: 1,
        ..Default::default()
    };
    let path = directory.join("registry.json");
    if path.exists() {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(invalid("ACP directory cache is too large"));
        }
        loaded = serde_json::from_slice(&bytes)?;
        if loaded.version != 1 {
            return Err(invalid("Unsupported ACP directory cache"));
        }
        for preset in loaded
            .available
            .iter_mut()
            .chain(loaded.selected.values_mut())
        {
            validate_distribution(preset)?;
            preset.official_registry = true;
            preset.managed_distribution = true;
        }
    }
    loaded.retain_first_discovered_versions();
    let mut state = state().write().unwrap_or_else(|e| e.into_inner());
    state.directory = Some(directory.to_owned());
    state.cache = loaded;
    Ok(())
}

pub async fn refresh(force: bool) -> Result<RegistrySnapshot, CoreError> {
    let _operation = operation().lock().await;
    let (directory, mut cache) = {
        let state = state().read().unwrap_or_else(|e| e.into_inner());
        if !force
            && state
                .cache
                .checked_at
                .is_some_and(|time| now().saturating_sub(time) < REFRESH_SECONDS)
        {
            return Ok(state.cache.snapshot());
        }
        (
            state
                .directory
                .clone()
                .ok_or_else(|| invalid("ACP directory cache is unavailable"))?,
            state.cache.clone(),
        )
    };
    let client = reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| invalid(format!("ACP directory client failed: {error}")))?;
    let response = client.get(SOURCE).send().await.and_then(reqwest::Response::error_for_status)
        .map_err(|error| invalid(format!("Could not refresh the ACP directory; the cached versions remain available. {error}")))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BYTES as u64)
    {
        return Err(invalid("ACP directory response is too large"));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|error| invalid(format!("ACP directory download interrupted: {error}")))?;
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err(invalid("ACP directory response is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    cache.available = normalize(&bytes)?;
    cache.retain_first_discovered_versions();
    cache.checked_at = Some(now());
    write_cache(&directory, &cache)?;
    let result = cache.snapshot();
    state().write().unwrap_or_else(|e| e.into_inner()).cache = cache;
    Ok(result)
}

pub fn update_candidate(provider: &str, expected: &str) -> Result<ExternalAgentPreset, CoreError> {
    state()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .cache
        .available
        .iter()
        .find(|preset| preset.provider == provider && fingerprint(preset) == expected)
        .cloned()
        .ok_or_else(|| invalid("The ACP directory changed; check for updates again"))
}

/// The host calls this only after the candidate completed its ACP handshake.
pub async fn activate(candidate: ExternalAgentPreset) -> Result<RegistrySnapshot, CoreError> {
    let _operation = operation().lock().await;
    let expected = fingerprint(&candidate);
    update_candidate(&candidate.provider, &expected)?;
    let (directory, mut cache) = {
        let state = state().read().unwrap_or_else(|e| e.into_inner());
        (
            state
                .directory
                .clone()
                .ok_or_else(|| invalid("ACP directory cache is unavailable"))?,
            state.cache.clone(),
        )
    };
    cache.selected.insert(candidate.provider.clone(), candidate);
    write_cache(&directory, &cache)?;
    let result = cache.snapshot();
    state().write().unwrap_or_else(|e| e.into_inner()).cache = cache;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory(id: &str, package: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"version":"1.0.0","agents":[{
            "id":id,"name":"Agent","version":"2.0.0","repository":"https://github.com/example/agent",
            "distribution":{"npx":{"package":package,"args":["--acp"]}}
        }]})).unwrap()
    }
    #[test]
    fn directory_keeps_stable_provider_identity_and_requires_exact_packages() {
        let candidate = normalize(&directory("gemini", "@google/gemini-cli@2.0.0"))
            .unwrap()
            .remove(0);
        assert_eq!(candidate.provider, "gemini_cli");
        assert!(candidate.official_registry && candidate.managed_distribution);
        assert!(normalize(&directory("new-agent", "package@latest")).is_err());
        assert!(normalize(&directory("../bad", "package@2.0.0")).is_err());
        assert_eq!(
            normalize(&directory("new-agent", "package@2.0.0")).unwrap()[0].provider,
            "acp_new_agent"
        );
    }
    #[test]
    fn refresh_is_discovery_and_activation_changes_only_the_selected_adapter() {
        let available = normalize(&directory("gemini", "@google/gemini-cli@2.0.0")).unwrap();
        let mut cache = Cache {
            version: 1,
            available,
            ..Default::default()
        };
        assert_ne!(
            cache
                .resolve("gemini_cli")
                .unwrap()
                .registry_version
                .as_deref(),
            Some("2.0.0")
        );
        assert!(
            cache
                .snapshot()
                .entries
                .iter()
                .find(|entry| entry.preset.provider == "gemini_cli")
                .unwrap()
                .update_available
        );
        cache
            .selected
            .insert("gemini_cli".into(), cache.available[0].clone());
        assert_eq!(
            cache
                .resolve("gemini_cli")
                .unwrap()
                .registry_version
                .as_deref(),
            Some("2.0.0")
        );
        assert!(cache.resolve("custom_acp").is_some());
    }
    #[tokio::test]
    #[ignore = "Reads the public official ACP directory; no agent downloads or authentication"]
    async fn live_official_directory_normalizes_every_published_agent() {
        let directory = tempfile::tempdir().unwrap();
        initialize(directory.path()).unwrap();
        let snapshot = refresh(true).await.unwrap();
        assert!(snapshot.entries.len() >= bundled_presets().len());
        assert!(snapshot.checked_at.is_some());
        println!(
            "Official ACP directory: {} entries, {} available updates",
            snapshot.entries.len(),
            snapshot
                .entries
                .iter()
                .filter(|entry| entry.update_available)
                .count()
        );
    }

    #[test]
    fn newly_discovered_agents_keep_their_first_version_until_activation() {
        let mut cache = Cache {
            version: 1,
            available: normalize(&directory("new-agent", "new-agent@2.0.0")).unwrap(),
            ..Default::default()
        };
        cache.retain_first_discovered_versions();
        let original = fingerprint(&cache.resolve("acp_new_agent").unwrap());
        cache.available[0].registry_version = Some("3.0.0".into());
        cache.available[0]
            .distribution
            .as_mut()
            .unwrap()
            .npx
            .as_mut()
            .unwrap()
            .package = "new-agent@3.0.0".into();
        cache.retain_first_discovered_versions();
        assert_eq!(
            fingerprint(&cache.resolve("acp_new_agent").unwrap()),
            original
        );
        let snapshot = cache.snapshot();
        let entry = snapshot
            .entries
            .iter()
            .find(|entry| entry.preset.provider == "acp_new_agent")
            .unwrap();
        assert!(entry.update_available);
        assert_eq!(entry.available_version.as_deref(), Some("3.0.0"));
        assert_eq!(entry.preset.registry_version.as_deref(), Some("2.0.0"));
    }
}
