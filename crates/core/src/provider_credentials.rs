//! Credential boundaries shared by chat connections and capability providers.
use std::{collections::HashMap, sync::OnceLock};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scopes {
    endpoints: HashMap<String, String>,
    provider_aliases: HashMap<String, String>,
}

pub(crate) fn scope(provider: &str, endpoint: Option<&str>) -> Option<String> {
    static SCOPES: OnceLock<Scopes> = OnceLock::new();
    let scopes = SCOPES.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../shared/provider-credential-scopes.json"
        ))
        .expect("reviewed credential scopes")
    });
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
    let url = url::Url::parse(endpoint.trim()).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let normalized = url.as_str().trim_end_matches('/');
    Some(
        scopes
            .endpoints
            .get(normalized)
            .cloned()
            .unwrap_or_else(|| format!("endpoint:{normalized}")),
    )
}
