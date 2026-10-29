//! Configuration preflight for adapters whose ACP authentication is a no-op.
//! This checks presence only; model discovery does not establish entitlement.
use super::{ExternalAgentLaunch, ExternalAgentPreset};
use crate::error::CoreError;
use std::{collections::BTreeMap, io::Read, path::Path};

fn value(env: &BTreeMap<String, String>, name: &str) -> Option<String> {
    env.iter()
        .find(|(key, _)| {
            if cfg!(windows) {
                key.eq_ignore_ascii_case(name)
            } else {
                key.as_str() == name
            }
        })
        .map(|(_, value)| value.clone())
        .or_else(|| std::env::var(name).ok())
}

fn glm_credentials_present(key: Option<&str>, config_root: Option<&Path>) -> bool {
    if key.is_some_and(|key| !key.trim().is_empty()) {
        return true;
    }
    let Some(root) = config_root else {
        return false;
    };
    let Ok(file) = std::fs::File::open(root.join("glm-acp-agent/credentials.json")) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file.take(65_537).read_to_end(&mut bytes).is_err() || bytes.len() > 65_536 {
        return false;
    }
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("z_ai_api_key")
                .and_then(serde_json::Value::as_str)
                .map(|key| !key.trim().is_empty())
        })
        .unwrap_or(false)
}

pub fn validate_launch_credentials(
    preset: &ExternalAgentPreset,
    launch: &ExternalAgentLaunch,
) -> Result<(), CoreError> {
    // A user-selected wrapper can provide its own credentials. Do not guess
    // its configuration. Standard GLM launches use the reviewed resolver below.
    if preset.provider != "acp_glm_acp_agent" || launch.executable.is_some() {
        return Ok(());
    }
    let env = preset.launch_env(launch);
    let key = value(&env, "Z_AI_API_KEY");
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let config_root = value(&env, "XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            value(&env, home_var)
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
                .or_else(dirs::home_dir)
                .map(|home| home.join(".config"))
        })
        .map(|root| Path::new(&launch.working_directory).join(root));
    if glm_credentials_present(key.as_deref(), config_root.as_deref()) {
        return Ok(());
    }
    Err(CoreError::InvalidInput("GLM Agent needs a Coding Plan API key. Enter Z_AI_API_KEY in this agent's settings or run glm-acp-agent --setup. ZCode account sign-in does not authenticate this ACP adapter. No prompt was sent.".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn glm_preflight_accepts_environment_or_agent_owned_credentials_without_exposing_them() {
        let root = tempfile::tempdir().unwrap();
        assert!(!glm_credentials_present(None, Some(root.path())));
        assert!(!glm_credentials_present(Some(" "), Some(root.path())));
        assert!(glm_credentials_present(Some("test-key"), Some(root.path())));
        std::fs::create_dir(root.path().join("glm-acp-agent")).unwrap();
        let path = root.path().join("glm-acp-agent/credentials.json");
        for payload in [
            "{}",
            "invalid",
            r#"{"z_ai_api_key":""}"#,
            r#"{"z_ai_api_key":123}"#,
        ] {
            std::fs::write(&path, payload).unwrap();
            assert!(!glm_credentials_present(None, Some(root.path())));
        }
        std::fs::write(&path, r#"{"z_ai_api_key":"stored-test-key"}"#).unwrap();
        assert!(glm_credentials_present(None, Some(root.path())));
    }
}
