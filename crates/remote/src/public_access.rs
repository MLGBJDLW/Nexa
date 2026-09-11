//! Managed public routes selected by end-to-end reachability, independent of locale.
use crate::{
    tunnel::{self, PublicTunnelProvider},
    Endpoint, EndpointKind, RemoteServer,
};
use base64::Engine;
use futures::{stream::FuturesUnordered, StreamExt};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRouteStatus {
    pub provider: PublicTunnelProvider,
    pub phase: &'static str,
    pub urls: Vec<String>,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

pub struct ManagedPublicAccess {
    stop: CancellationToken,
    statuses: Arc<Mutex<Vec<PublicRouteStatus>>>,
}
impl Drop for ManagedPublicAccess {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl ManagedPublicAccess {
    pub fn statuses(&self) -> Vec<PublicRouteStatus> {
        self.statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn preferred_url(&self) -> Option<String> {
        self.statuses()
            .into_iter()
            .filter(|route| route.phase == "ready")
            .min_by_key(|route| route.latency_ms.unwrap_or(u64::MAX))
            .and_then(|route| route.urls.first().cloned())
    }
    pub fn is_running(&self) -> bool {
        self.preferred_url().is_some()
    }
}

/// A helper printing a URL is not readiness. Verify the public TLS/HTTP path,
/// the installation identity, and the WebSocket upgrade used by actual clients.
pub async fn probe_endpoint(url: &str, server_id: &str) -> Result<u64, String> {
    let started = tokio::time::Instant::now();
    let client = reqwest::Client::builder()
        .http1_only()
        .user_agent("Nexa-Remote-Probe")
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| "probe_setup_failed")?;
    let response = client
        .get(format!("{url}/api/health"))
        .send()
        .await
        .map_err(|_| "public_https_unreachable")?;
    if !response.status().is_success() {
        return Err("public_http_error".into());
    }
    #[cfg(test)]
    eprintln!(
        "[DEBUG-remote-connect] health type={:?}, bytes={:?}",
        response.headers().get("Content-Type"),
        response.content_length()
    );
    if response
        .content_length()
        .is_some_and(|length| length > 4096)
    {
        return Err("public_identity_mismatch".into());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(bytes) = stream.next().await {
        let bytes = bytes.map_err(|_| "public_https_unreachable")?;
        if body.len() + bytes.len() > 4096 {
            return Err("public_identity_mismatch".into());
        }
        body.extend_from_slice(&bytes);
    }
    #[cfg(test)]
    eprintln!(
        "[DEBUG-remote-connect] health preview={}",
        String::from_utf8_lossy(&body)
            .chars()
            .take(180)
            .collect::<String>()
    );
    let value: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| "public_identity_mismatch")?;
    if value["serverId"] != server_id {
        return Err("public_identity_mismatch".into());
    }
    // A route can forward GET/Upgrade while stalling request bodies. Echo a
    // nonce through the real POST body path without using a pairing credential.
    let nonce = uuid::Uuid::new_v4().to_string();
    let echo = client
        .post(format!("{url}/api/health"))
        .header("Origin", url)
        .json(&serde_json::json!({"nonce":nonce}))
        .send()
        .await
        .map_err(|_| "public_rpc_failed")?;
    if !echo.status().is_success() || echo.content_length().is_some_and(|length| length > 512) {
        return Err("public_rpc_failed".into());
    }
    let mut echo_bytes = Vec::new();
    let mut chunks = echo.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| "public_rpc_failed")?;
        if echo_bytes.len() + chunk.len() > 512 {
            return Err("public_rpc_failed".into());
        }
        echo_bytes.extend_from_slice(&chunk);
    }
    let echo: serde_json::Value =
        serde_json::from_slice(&echo_bytes).map_err(|_| "public_rpc_failed")?;
    if echo["serverId"] != server_id || echo["nonce"] != nonce {
        return Err("public_rpc_failed".into());
    }
    let admission = client
        .post(format!("{url}/api/rpc"))
        .header("Origin", url)
        .json(&serde_json::json!({"method":"connections.list"}))
        .send()
        .await
        .map_err(|_| "public_rpc_failed")?;
    if admission.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Err("public_rpc_failed".into());
    }
    // Reuse the same proxy-aware TLS stack for the upgrade. A separate raw TCP
    // WebSocket probe gives false negatives on networks requiring an HTTP proxy.
    let key = base64::engine::general_purpose::STANDARD.encode(uuid::Uuid::new_v4().as_bytes());
    let expected = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
    let response = client
        .get(format!("{url}/api/events"))
        .header("Origin", url)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", key)
        .send()
        .await
        .map_err(|_| "public_websocket_failed")?;
    if response.status() != reqwest::StatusCode::SWITCHING_PROTOCOLS
        || response
            .headers()
            .get("Sec-WebSocket-Accept")
            .and_then(|value| value.to_str().ok())
            != Some(expected.as_str())
    {
        return Err("public_websocket_failed".into());
    }
    // Drop the upgraded connection without sending a pairing token or user data.
    Ok(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
}

async fn probe_routes(
    server: &RemoteServer,
    urls: &[String],
) -> (Vec<String>, Option<u64>, Option<String>) {
    let identity = server.manifest().server_id;
    let mut probes = FuturesUnordered::new();
    for url in urls {
        if server
            .add_endpoint(Endpoint {
                url: url.clone(),
                kind: EndpointKind::Tunnel,
            })
            .is_ok()
        {
            let identity = identity.clone();
            probes.push(async move { (url.clone(), probe_endpoint(url, &identity).await) });
        }
    }
    let mut reachable = Vec::new();
    let mut last_error = None;
    while let Some((url, result)) = probes.next().await {
        match result {
            Ok(latency) => reachable.push((url, latency)),
            Err(error) => {
                server.retire_endpoint(&url);
                last_error = Some(error);
            }
        }
    }
    reachable.sort_by_key(|(_, latency)| *latency);
    let latency = reachable.first().map(|(_, latency)| *latency);
    (
        reachable.into_iter().map(|(url, _)| url).collect(),
        latency,
        last_error,
    )
}

pub async fn start(
    server: Arc<RemoteServer>,
    directory: PathBuf,
    loopback: String,
    provider: PublicTunnelProvider,
) -> ManagedPublicAccess {
    let providers = if provider == PublicTunnelProvider::Auto {
        vec![
            PublicTunnelProvider::LocalhostRun,
            PublicTunnelProvider::Pinggy,
            PublicTunnelProvider::Cloudflare,
        ]
    } else {
        vec![provider]
    };
    let stop = server.shutdown.child_token();
    let statuses = Arc::new(Mutex::new(
        providers
            .iter()
            .map(|provider| PublicRouteStatus {
                provider: *provider,
                phase: "connecting",
                urls: vec![],
                latency_ms: None,
                error: None,
            })
            .collect::<Vec<_>>(),
    ));
    let changed = Arc::new(Notify::new());
    let access = ManagedPublicAccess {
        stop: stop.clone(),
        statuses: statuses.clone(),
    };
    for selected in providers {
        let server = server.clone();
        let directory = directory.clone();
        let loopback = loopback.clone();
        let stop = stop.clone();
        let statuses = statuses.clone();
        let changed = changed.clone();
        tokio::spawn(async move {
            let update = |phase, urls, latency_ms, error| {
                if let Some(status) = statuses
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter_mut()
                    .find(|status| status.provider == selected)
                {
                    *status = PublicRouteStatus {
                        provider: selected,
                        phase,
                        urls,
                        latency_ms,
                        error,
                    };
                }
                changed.notify_waiters();
            };
            if provider == PublicTunnelProvider::Auto
                && selected == PublicTunnelProvider::Cloudflare
            {
                tokio::select! { _ = stop.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(4)) => {} }
            }
            let mut failures = 0_u32;
            loop {
                if stop.is_cancelled() {
                    break;
                }
                if provider == PublicTunnelProvider::Auto
                    && selected == PublicTunnelProvider::Cloudflare
                    && statuses
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .iter()
                        .filter(|route| route.phase == "ready")
                        .count()
                        >= 2
                {
                    update("standby", vec![], None, None);
                    tokio::select! { _ = stop.cancelled() => break, _ = tokio::time::sleep(Duration::from_secs(15)) => {} }
                    continue;
                }
                update("connecting", vec![], None, None);
                let attempt = async {
                    if selected == PublicTunnelProvider::Cloudflare {
                        tunnel::start(&directory.join("cloudflare"), &loopback).await
                    } else {
                        tunnel::start_ssh(&directory.join(selected.name()), &loopback, selected)
                            .await
                    }
                };
                let result =
                    tokio::select! { _ = stop.cancelled() => break, result = attempt => result };
                match result {
                    Ok(tunnel) => {
                        let mut registered = Vec::<String>::new();
                        let started = tokio::time::Instant::now();
                        let mut missed = 0;
                        loop {
                            if !tunnel.is_running()
                                || (selected == PublicTunnelProvider::Pinggy
                                    && started.elapsed() > Duration::from_secs(50 * 60))
                            {
                                break;
                            }
                            let urls = tunnel.urls();
                            let (reachable, latency, error) = tokio::select! {
                                _ = stop.cancelled() => break,
                                result = probe_routes(&server, &urls) => result,
                            };
                            registered.extend(
                                urls.into_iter()
                                    .filter(|url| !registered.contains(url))
                                    .collect::<Vec<_>>(),
                            );
                            if reachable.is_empty() {
                                update("checking", vec![], None, error);
                                missed += 1;
                                if missed >= 3 {
                                    break;
                                }
                            } else {
                                failures = 0;
                                missed = 0;
                                update("ready", reachable, latency, None);
                            }
                            let delay = Duration::from_secs(if missed > 0 { 3 } else { 20 });
                            tokio::select! { _ = stop.cancelled() => break, _ = tokio::time::sleep(delay) => {} }
                        }
                        for url in registered {
                            server.retire_endpoint(&url);
                        }
                        let _ = tunnel.stop().await;
                        update(
                            "retrying",
                            vec![],
                            None,
                            Some("public_route_interrupted".into()),
                        );
                    }
                    Err(error) => update("retrying", vec![], None, Some(error)),
                }
                failures = failures.saturating_add(1);
                let delay = Duration::from_secs((5_u64 * 2_u64.pow(failures.min(4))).min(60));
                tokio::select! { _ = stop.cancelled() => break, _ = tokio::time::sleep(delay) => {} }
            }
            update("stopped", vec![], None, None);
        });
    }
    let wait = async {
        loop {
            let change = changed.notified();
            let snapshot = access.statuses();
            if snapshot.iter().any(|route| route.phase == "ready")
                || snapshot
                    .iter()
                    .all(|route| !matches!(route.phase, "connecting" | "checking"))
            {
                break;
            }
            change.await;
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(25), wait).await;
    access
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuthStore, RemoteCommand, RemoteHost, WebAsset};
    use serde_json::{json, Value};
    struct ProbeHost;
    #[async_trait::async_trait]
    impl RemoteHost for ProbeHost {
        async fn execute(&self, _owner: &str, _command: RemoteCommand) -> Result<Value, String> {
            Ok(json!({"ok":true}))
        }
        async fn disconnected(&self, _owner: &str) {}
        fn asset(&self, _path: &str) -> Option<WebAsset> {
            Some(WebAsset {
                bytes: b"Nexa remote probe".to_vec(),
                mime_type: "text/plain".into(),
            })
        }
    }
    #[tokio::test]
    async fn probe_verifies_installation_identity_and_websocket_upgrade() {
        let directory = tempfile::tempdir().unwrap();
        let server = RemoteServer::new(
            AuthStore::open(directory.path()).unwrap(),
            Arc::new(ProbeHost),
        );
        let listener = server
            .listen(([127, 0, 0, 1], 0).into(), None)
            .await
            .unwrap();
        let url = format!("http://{}", listener.address);
        assert!(probe_endpoint(&url, &server.server_id()).await.is_ok());
        assert_eq!(
            probe_endpoint(&url, "another-installation")
                .await
                .unwrap_err(),
            "public_identity_mismatch"
        );
        server.stop().await;
    }
    #[tokio::test]
    #[ignore = "explicit public tunnel smoke test; exposes only an isolated synthetic host"]
    async fn public_tunnel_smoke() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let provider = match std::env::var("NEXA_REMOTE_SMOKE_PROVIDER").as_deref() {
            Ok("auto") => PublicTunnelProvider::Auto,
            Ok("pinggy") => PublicTunnelProvider::Pinggy,
            Ok("cloudflare") => PublicTunnelProvider::Cloudflare,
            _ => PublicTunnelProvider::LocalhostRun,
        };
        let directory = tempfile::tempdir().unwrap();
        let server = RemoteServer::new(
            AuthStore::open(directory.path()).unwrap(),
            Arc::new(ProbeHost),
        );
        let listener = server
            .listen(([127, 0, 0, 1], 0).into(), None)
            .await
            .unwrap();
        let access = start(
            server.clone(),
            directory.path().join("helper"),
            format!("http://{}", listener.address),
            provider,
        )
        .await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while access.preferred_url().is_none() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let url = access.preferred_url().unwrap_or_else(|| {
            panic!(
                "No verified public route: {}",
                serde_json::to_string(&access.statuses()).unwrap()
            )
        });
        let client = reqwest::Client::builder()
            .http1_only()
            .user_agent("Nexa-Remote-Probe")
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let code = server.pairing();
        let response = client.post(format!("{url}/api/pair")).header("Origin", &url).json(&json!({"code":code.code,"name":"Isolated network probe","clientNonce":uuid::Uuid::new_v4().to_string()})).send().await.unwrap();
        assert!(
            response.status().is_success(),
            "pairing status {}",
            response.status()
        );
        let paired: Value = response.json().await.unwrap();
        let token = paired["token"].as_str().unwrap();
        let response = client
            .post(format!("{url}/api/rpc"))
            .header("Origin", &url)
            .bearer_auth(token)
            .json(&json!({"method":"connections.list"}))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert_eq!(
            response.json::<Value>().await.unwrap()["result"]["ok"],
            true
        );
        println!("Verified {}: public HTTPS, installation identity, WebSocket upgrade, pairing and authenticated RPC", provider.name());
        server.stop().await;
        drop(access);
    }
}
