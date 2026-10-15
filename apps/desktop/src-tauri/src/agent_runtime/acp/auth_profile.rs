//! Google documents auth.type in GEMINI_HOME/antigravity-acp/settings.json.
//! Keep managed sign-in and inference in the same app-owned profile; never
//! rewrite another CLI's settings or copy its credential files.
use super::{error, Result};
use nexa_core::external_agent::{launch::LaunchPlan, ExternalAgentLaunch, ExternalAgentPreset};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::Path,
};

fn oauth_conflict(key: &str) -> bool {
    matches!(
        key.to_ascii_uppercase().as_str(),
        "GEMINI_API_KEY"
            | "GOOGLE_API_KEY"
            | "GOOGLE_APPLICATION_CREDENTIALS"
            | "GOOGLE_GENAI_USE_VERTEXAI"
            | "GOOGLE_GENAI_USE_GCA"
            | "GEMINI_DEFAULT_AUTH_TYPE"
            | "GOOGLE_CLOUD_PROJECT"
            | "GOOGLE_CLOUD_LOCATION"
            | "GOOGLE_CLOUD_QUOTA_PROJECT"
            | "GCLOUD_PROJECT"
            | "CLOUDSDK_CORE_PROJECT"
    )
}

pub(super) fn prepare(
    preset: &ExternalAgentPreset,
    launch: &ExternalAgentLaunch,
    cache: Option<&Path>,
    plan: &mut LaunchPlan,
) -> Result<()> {
    if preset.registry_id.as_deref() != Some("antigravity-acp")
        || launch.executable.is_some()
        || launch
            .env
            .keys()
            .any(|key| key.eq_ignore_ascii_case("GEMINI_HOME"))
    {
        return Ok(());
    }
    let Some(cache) = cache else {
        return Ok(());
    };
    let method = launch
        .authentication_method
        .as_deref()
        .unwrap_or("oauth-personal");
    // Unknown future methods still use their advertised ACP handshake. Do not
    // guess a settings schema for them.
    if !matches!(
        method,
        "oauth-personal" | "oauth-business" | "gemini-api-key" | "agent-platform"
    ) {
        return Ok(());
    }
    let home = cache.join("profiles").join("antigravity").join(method);
    let directory = home.join("antigravity-acp");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("settings.json");
    let mut settings = if path.is_file() {
        let mut bytes = Vec::new();
        std::fs::File::open(&path)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err(error("Managed Google settings are too large"));
        }
        serde_json::from_slice::<Value>(&bytes)?
    } else {
        json!({})
    };
    if !settings.is_object() {
        return Err(error("Managed Google settings must be a JSON object"));
    }
    if settings.get("auth").is_none() {
        settings["auth"] = json!({});
    }
    if !settings["auth"].is_object() {
        return Err(error("Managed Google auth settings must be a JSON object"));
    }
    if settings.pointer("/auth/type").and_then(Value::as_str) != Some(method) {
        settings["auth"]["type"] = json!(method);
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        file.write_all(&serde_json::to_vec_pretty(&settings)?)?;
        file.as_file().sync_all()?;
        file.persist(&path).map_err(|cause| {
            error(format!(
                "Could not prepare Google sign-in settings: {}",
                cause.error
            ))
        })?;
    }
    nexa_core::external_agent::launch::set_env(
        &mut plan.env,
        "GEMINI_HOME",
        home.to_string_lossy().into_owned(),
    );
    if matches!(method, "oauth-personal" | "oauth-business") {
        let mut removed: Vec<_> = std::env::vars_os()
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .chain(plan.env.keys().cloned())
            .filter(|key| oauth_conflict(key))
            .collect();
        removed.sort();
        removed.dedup();
        plan.env.retain(|key, _| !oauth_conflict(key));
        plan.env_remove.extend(removed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personal_login_and_inference_share_an_isolated_profile_without_api_credentials() {
        let root = tempfile::tempdir().unwrap();
        let preset = nexa_core::external_agent::preset("acp_antigravity_acp").unwrap();
        let launch = ExternalAgentLaunch {
            authentication_method: Some("oauth-personal".into()),
            ..Default::default()
        };
        let new_plan = || LaunchPlan {
            executable: "agent".into(),
            args: vec![],
            env: std::collections::BTreeMap::from([
                ("google_api_key".into(), "fixture-secret".into()),
                ("HTTPS_PROXY".into(), "http://proxy.invalid".into()),
            ]),
            env_remove: vec![],
            prepared: true,
        };
        let mut login = new_plan();
        prepare(&preset, &launch, Some(root.path()), &mut login).unwrap();
        let mut inference = new_plan();
        prepare(&preset, &launch, Some(root.path()), &mut inference).unwrap();
        assert_eq!(login.env["GEMINI_HOME"], inference.env["GEMINI_HOME"]);
        assert!(!login.env.contains_key("google_api_key"));
        assert!(login.env_remove.iter().any(|key| key == "google_api_key"));
        assert_eq!(login.env["HTTPS_PROXY"], "http://proxy.invalid");
        let settings = std::fs::read_to_string(
            Path::new(&login.env["GEMINI_HOME"]).join("antigravity-acp/settings.json"),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&settings).unwrap()["auth"]["type"],
            "oauth-personal"
        );
        assert!(!settings.contains("fixture-secret"));
        let mut custom = launch.clone();
        custom
            .env
            .insert("gemini_home".into(), "custom-home".into());
        let mut untouched = new_plan();
        prepare(&preset, &custom, Some(root.path()), &mut untouched).unwrap();
        assert!(!untouched.env.contains_key("GEMINI_HOME"));
        assert!(untouched.env_remove.is_empty());
    }
}
