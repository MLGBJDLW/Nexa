use super::{auth_error, OAuthConfig};
use crate::error::CoreError;
use futures::StreamExt;
use reqwest::{header::HeaderMap, Url};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

const MAX_JSON: usize = 128 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Discovery {
    pub issuer: String,
    pub resource: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub revocation_endpoint: Option<String>,
    pub registration_endpoint: Option<String>,
    pub scopes_supported: Vec<String>,
    pub iss_required: bool,
    pub cimd_supported: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BearerChallenge {
    pub resource_metadata: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

/// Split only outside quoted strings, then associate auth-params with their scheme.
pub fn bearer_challenge(headers: &HeaderMap) -> Option<BearerChallenge> {
    for header in headers.get_all(reqwest::header::WWW_AUTHENTICATE) {
        let Ok(raw) = header.to_str() else { continue };
        if raw.len() > 16_384 {
            continue;
        }
        let mut parts = Vec::new();
        let (mut quoted, mut escaped, mut start) = (false, false, 0);
        for (index, ch) in raw.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if quoted && ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                quoted = !quoted;
            }
            if ch == ',' && !quoted {
                parts.push(raw[start..index].trim());
                start = index + 1;
            }
        }
        if quoted || escaped {
            continue;
        }
        parts.push(raw[start..].trim());
        let mut current: Option<BTreeMap<String, String>> = None;
        for part in parts.into_iter().chain(std::iter::once("End")) {
            // A scheme has whitespace before its first parameter name; "scope =" is a param.
            let (scheme, params) = part.split_once(char::is_whitespace).unwrap_or((part, ""));
            let is_scheme = !part.contains('=')
                || (!scheme.contains('=')
                    && !params.trim_start().starts_with('=')
                    && params.contains('='));
            let param = if is_scheme {
                if let Some(values) = current.take() {
                    return Some(challenge_from(values));
                }
                if scheme.eq_ignore_ascii_case("bearer") {
                    current = Some(BTreeMap::new());
                }
                params.trim()
            } else {
                part
            };
            if let (Some(values), Some((name, raw_value))) =
                (current.as_mut(), param.split_once('='))
            {
                let raw_value = raw_value.trim();
                let value = if raw_value.starts_with('"')
                    && raw_value.ends_with('"')
                    && raw_value.len() >= 2
                {
                    let mut result = String::new();
                    let mut escape = false;
                    for ch in raw_value[1..raw_value.len() - 1].chars() {
                        if escape {
                            result.push(ch);
                            escape = false;
                        } else if ch == '\\' {
                            escape = true;
                        } else {
                            result.push(ch);
                        }
                    }
                    result
                } else {
                    raw_value.into()
                };
                if values
                    .insert(name.trim().to_ascii_lowercase(), value)
                    .is_some()
                {
                    return None;
                }
            }
        }
    }
    None
}
fn challenge_from(mut values: BTreeMap<String, String>) -> BearerChallenge {
    BearerChallenge {
        resource_metadata: values.remove("resource_metadata"),
        scope: values.remove("scope"),
        error: values.remove("error"),
    }
}

pub(super) fn validate_url(raw: &str, local_origin: &Url) -> Result<Url, CoreError> {
    let url = Url::parse(raw).map_err(|_| {
        auth_error(
            "invalid_metadata",
            "OAuth metadata contains an invalid URL.",
        )
    })?;
    let allowed_loopback = local_origin.scheme() == "http"
        && local_origin.host_str().is_some_and(is_loopback_host)
        && url.origin() == local_origin.origin();
    if (url.scheme() != "https" && !allowed_loopback)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(auth_error("invalid_metadata", "OAuth endpoints require HTTPS without credentials or fragments (explicit loopback connectors are allowed)."));
    }
    if !allowed_loopback
        && url.host().is_none_or(|host| match host {
            url::Host::Ipv4(ip) => !public_ip(ip.into()),
            url::Host::Ipv6(ip) => !public_ip(ip.into()),
            url::Host::Domain(host) => {
                host.eq_ignore_ascii_case("localhost")
                    || host.ends_with(".local")
                    || host.ends_with(".internal")
            }
        })
    {
        return Err(auth_error("invalid_metadata","OAuth metadata cannot target private or local addresses outside the configured loopback origin."));
    }
    Ok(url)
}
fn is_loopback_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
        || host.eq_ignore_ascii_case("localhost")
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation()
                || a == 0
                || a >= 224
                || a == 100 && (64..=127).contains(&b)
                || a == 192 && b == 0
                || a == 198 && (18..=19).contains(&b))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || s[0] == 0x2001 && s[1] == 0xdb8
                || s[0] & 0xffc0 == 0xfec0)
                && ip.to_ipv4_mapped().is_none_or(|ip| public_ip(ip.into()))
        }
    }
}

async fn client(raw: &str, origin: &Url) -> Result<(reqwest::Client, Url), CoreError> {
    let url = validate_url(raw, origin)?;
    let host = url
        .host_str()
        .ok_or_else(|| auth_error("invalid_metadata", "OAuth URL has no host."))?;
    let addresses: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(443))),
    )
    .await
    .map_err(|_| auth_error("network_error", "OAuth DNS lookup timed out."))?
    .map_err(|_| auth_error("network_error", "OAuth DNS lookup failed."))?
    .collect();
    let local = origin.host_str().is_some_and(is_loopback_host) && origin.origin() == url.origin();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|addr| !public_ip(addr.ip()) && !(local && addr.ip().is_loopback()))
    {
        return Err(auth_error("invalid_metadata", "OAuth metadata resolved to a non-public address outside the configured loopback connector."));
    }
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(25))
        .connect_timeout(Duration::from_secs(10))
        .resolve_to_addrs(host, &addresses)
        .user_agent(crate::USER_AGENT)
        .build()
        .map_err(|_| auth_error("network_error", "OAuth HTTP client could not start."))?;
    Ok((http, url))
}

pub(super) async fn json_request(
    raw: &str,
    origin: &Url,
    form: Option<&[(String, String)]>,
    json: Option<&serde_json::Value>,
) -> Result<serde_json::Value, CoreError> {
    let (http, url) = client(raw, origin).await?;
    let request = if let Some(form) = form {
        http.post(url).form(form)
    } else if let Some(json) = json {
        http.post(url).json(json)
    } else {
        http.get(url)
    };
    let response = request
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| {
            auth_error(
                "network_error",
                "OAuth request failed or timed out; retry from settings.",
            )
        })?;
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|n| n > MAX_JSON as u64)
    {
        return Err(auth_error(
            "invalid_metadata",
            "OAuth response exceeds 128 KiB.",
        ));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| auth_error("network_error", "OAuth response was interrupted."))?;
        if bytes.len() + chunk.len() > MAX_JSON {
            return Err(auth_error(
                "invalid_metadata",
                "OAuth response exceeds 128 KiB.",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| auth_error("invalid_response", "OAuth endpoint returned invalid JSON."))?;
    if !status.is_success() {
        let code = match value.get("error").and_then(serde_json::Value::as_str) {
            Some("invalid_grant") => "invalid_grant",
            Some("invalid_client") => "invalid_client",
            Some("invalid_scope") => "invalid_scope",
            _ => "request_failed",
        };
        return Err(auth_error(
            code,
            &format!(
                "OAuth endpoint rejected the request (HTTP {}).",
                status.as_u16()
            ),
        ));
    }
    Ok(value)
}

pub(super) async fn revoke(
    raw: &str,
    origin: &Url,
    form: &[(String, String)],
) -> Result<(), CoreError> {
    let (http, url) = client(raw, origin).await?;
    let response = http
        .post(url)
        .form(form)
        .send()
        .await
        .map_err(|_| auth_error("network_error", "Remote revocation failed."))?;
    if response.status() == reqwest::StatusCode::OK {
        Ok(())
    } else {
        Err(auth_error(
            "request_failed",
            "Remote revocation was not confirmed.",
        ))
    }
}

pub(super) async fn discover(endpoint: &str, config: &OAuthConfig) -> Result<Discovery, CoreError> {
    let origin =
        Url::parse(endpoint).map_err(|_| auth_error("invalid_config", "Invalid MCP endpoint."))?;
    let (http, request_url) = client(endpoint, &origin).await?;
    let response = http
        .get(request_url.clone())
        .header(
            reqwest::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .send()
        .await
        .map_err(|_| {
            auth_error(
                "network_error",
                "Cannot read the MCP authorization challenge.",
            )
        })?;
    let challenge = bearer_challenge(response.headers());
    // The unauthenticated probe body may be an endless event stream; never consume it.
    drop(response);
    let mut root = origin.clone();
    root.set_path("/");
    root.set_query(None);
    let candidates = if let Some(metadata) = challenge.and_then(|c| c.resource_metadata) {
        vec![(metadata, endpoint.to_string())]
    } else {
        let mut path = root.clone();
        path.set_path(&format!(
            "/.well-known/oauth-protected-resource{}",
            origin.path().trim_end_matches('/')
        ));
        let mut well_known = root.clone();
        well_known.set_path("/.well-known/oauth-protected-resource");
        vec![
            (path.to_string(), endpoint.to_string()),
            (
                well_known.to_string(),
                root.as_str().trim_end_matches('/').to_string(),
            ),
        ]
    };
    let mut protected = None;
    for (url, expected) in candidates {
        if let Ok(value) = json_request(&url, &origin, None, None).await {
            let resource = value
                .get("resource")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let expected = config.resource.as_deref().unwrap_or(&expected);
            if resource != expected {
                return Err(auth_error("resource_mismatch", "Protected-resource metadata does not match this connector's resource identifier."));
            }
            let parsed_resource = validate_url(resource, &origin)?;
            let ancestor = parsed_resource.path().trim_end_matches('/');
            if parsed_resource.origin() != origin.origin()
                || !(origin.path() == ancestor
                    || origin.path().starts_with(&format!("{ancestor}/")))
            {
                return Err(auth_error(
                    "resource_mismatch",
                    "OAuth resource must identify this endpoint or its same-origin parent path.",
                ));
            }
            let resource = resource.to_owned();
            protected = Some((value, resource));
            break;
        }
    }
    let (protected, resource) = protected.ok_or_else(|| {
        auth_error(
            "discovery_failed",
            "No valid protected-resource metadata was found.",
        )
    })?;
    let issuers = protected
        .get("authorization_servers")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            auth_error(
                "discovery_failed",
                "No authorization server was advertised.",
            )
        })?;
    let issuer = if let Some(pinned) = &config.issuer {
        issuers
            .iter()
            .filter_map(serde_json::Value::as_str)
            .find(|value| *value == pinned)
    } else {
        issuers.first().and_then(serde_json::Value::as_str)
    }
    .ok_or_else(|| {
        auth_error(
            "issuer_mismatch",
            "Configured issuer is absent from protected-resource metadata.",
        )
    })?;
    let parsed = validate_url(issuer, &origin)?;
    if parsed.query().is_some() {
        return Err(auth_error(
            "issuer_mismatch",
            "OAuth issuer cannot contain a query.",
        ));
    }
    let mut as_origin = parsed.clone();
    as_origin.set_path("/");
    let path = parsed.path().trim_end_matches('/');
    let mut urls = Vec::new();
    for suffix in [
        format!("/.well-known/oauth-authorization-server{path}"),
        format!("/.well-known/openid-configuration{path}"),
        format!("{path}/.well-known/openid-configuration"),
    ] {
        let mut url = as_origin.clone();
        url.set_path(&suffix);
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    for url in urls {
        let Ok(metadata) = json_request(url.as_str(), &origin, None, None).await else {
            continue;
        };
        if metadata.get("issuer").and_then(serde_json::Value::as_str) != Some(issuer) {
            return Err(auth_error(
                "issuer_mismatch",
                "Authorization-server issuer does not exactly match discovery.",
            ));
        }
        if !metadata
            .get("code_challenge_methods_supported")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|methods| methods.iter().any(|method| method == "S256"))
        {
            return Err(auth_error(
                "pkce_required",
                "Authorization server must explicitly support PKCE S256.",
            ));
        }
        let required_url = |key: &str| -> Result<String, CoreError> {
            let value = metadata
                .get(key)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    auth_error(
                        "invalid_metadata",
                        "Authorization metadata is missing a required endpoint.",
                    )
                })?;
            validate_url(value, &origin)?;
            Ok(value.into())
        };
        let optional_url = |key: &str| -> Result<Option<String>, CoreError> {
            metadata
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(|value| validate_url(value, &origin).map(|_| value.to_string()))
                .transpose()
        };
        return Ok(Discovery {
            issuer: issuer.into(),
            resource,
            authorization_endpoint: required_url("authorization_endpoint")?,
            token_endpoint: required_url("token_endpoint")?,
            revocation_endpoint: optional_url("revocation_endpoint")?,
            registration_endpoint: optional_url("registration_endpoint")?,
            scopes_supported: protected
                .get("scopes_supported")
                .and_then(serde_json::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
            iss_required: metadata
                .get("authorization_response_iss_parameter_supported")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            cimd_supported: metadata
                .get("client_id_metadata_document_supported")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        });
    }
    Err(auth_error(
        "discovery_failed",
        "Authorization-server metadata could not be verified.",
    ))
}
