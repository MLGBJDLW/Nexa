//! Credential boundaries shared by chat connections and capability providers.
use std::{collections::HashMap, sync::OnceLock};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scopes {
    endpoints: HashMap<String, String>,
    provider_aliases: HashMap<String, String>,
    workspace_routes: HashMap<String, HashMap<String, String>>,
}

fn catalog() -> &'static Scopes {
    static SCOPES: OnceLock<Scopes> = OnceLock::new();
    SCOPES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../shared/provider-credential-scopes.json"
        ))
        .expect("reviewed credential scopes")
    })
}

fn clean_url(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value.trim()).ok()?;
    (url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then_some(url)
}

fn workspace_scope(url: &url::Url) -> Option<String> {
    let chat = url.join("/compatible-mode/v1").ok()?;
    let region = crate::provider_catalog::alibaba_workspace_region(chat.as_str())?;
    let paths = catalog().workspace_routes.get(region)?;
    paths
        .values()
        .any(|path| path == url.path().trim_end_matches('/'))
        .then(|| format!("alibaba-workspace:{}", url.host_str().unwrap_or_default()))
}

pub(crate) fn scope(provider: &str, endpoint: Option<&str>) -> Option<String> {
    let scopes = catalog();
    let Some(endpoint) = endpoint.filter(|endpoint| !endpoint.trim().is_empty()) else {
        let provider = provider.trim().to_lowercase();
        return Some(
            scopes
                .provider_aliases
                .get(&provider)
                .cloned()
                .unwrap_or(provider),
        );
    };
    let url = clean_url(endpoint)?;
    let normalized = url.as_str().trim_end_matches('/');
    Some(
        scopes
            .endpoints
            .get(normalized)
            .cloned()
            .or_else(|| workspace_scope(&url))
            .unwrap_or_else(|| format!("endpoint:{normalized}")),
    )
}

/// Derive a destination from an application-owned capability preset. The caller
/// separately checks that its adapter and selected model are available.
pub fn feature_endpoint(
    provider: &str,
    source_endpoint: Option<&str>,
    feature: &str,
    preset_id: &str,
    preset_endpoint: &str,
) -> Option<String> {
    let source = source_endpoint.filter(|value| !value.trim().is_empty());
    if let Some(base) = source {
        let url = clean_url(base)?;
        if url.scheme() != "https" || url.port().is_some() {
            return None;
        }
        if let Some(region) = crate::provider_catalog::alibaba_workspace_region(url.as_str()) {
            let key = format!("{feature}:{preset_id}");
            let path = catalog().workspace_routes.get(region)?.get(&key)?;
            return Some(url.join(path).ok()?.to_string());
        }
    }
    let target = clean_url(preset_endpoint)?;
    let target_scope = catalog()
        .endpoints
        .get(target.as_str().trim_end_matches('/'))?;
    let source_scope = scope(provider, source)?;
    if &source_scope == target_scope {
        return Some(preset_endpoint.to_owned());
    }
    if source_scope == "alibaba-model-studio:singapore"
        && target_scope == "alibaba-model-studio:beijing"
        && matches!(feature, "stt" | "tts")
        && catalog()
            .workspace_routes
            .get("ap-southeast-1")
            .is_some_and(|routes| routes.contains_key(&format!("{feature}:{preset_id}")))
    {
        return Some(clean_url(source?)?.join(target.path()).ok()?.to_string());
    }
    None
}
