use futures::{SinkExt, StreamExt};
use nexa_remote::*;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
#[derive(Default)]
struct Host {
    calls: AtomicUsize,
    disconnected: Mutex<Vec<String>>,
}

#[tokio::test]
async fn html_preview_is_opaque_bounded_and_revoked_with_its_owner() {
    let directory = tempfile::tempdir().unwrap();
    let server = RemoteServer::new(
        AuthStore::open(directory.path()).unwrap(),
        Arc::new(Host::default()),
    );
    let listener = server
        .listen(([127, 0, 0, 1], 0).into(), None)
        .await
        .unwrap();
    let origin = format!("http://{}", listener.address);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let paired: Value = client.post(format!("{origin}/api/pair")).header("Origin", &origin)
        .json(&json!({"code":server.pairing().code,"name":"Phone","clientNonce":uuid::Uuid::new_v4().to_string()}))
        .send().await.unwrap().json().await.unwrap();
    let owner = paired["device"]["id"].as_str().unwrap();
    let path = server
        .create_html_preview(
            owner,
            "<button>Interactive</button><script>document.body.dataset.ready='yes'</script>".into(),
        )
        .unwrap();
    let response = client.get(format!("{origin}{path}")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.headers().get("x-frame-options").is_none());
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.contains("sandbox allow-scripts;"));
    assert!(!csp.contains("allow-same-origin"));
    assert!(csp.contains("connect-src 'none'"));
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(server
        .create_html_preview(owner, "x".repeat(512 * 1024 + 1))
        .is_err());
    server.revoke(owner).await.unwrap();
    assert_eq!(
        client
            .get(format!("{origin}{path}"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    server.stop().await;
}

#[tokio::test]
async fn incomplete_post_bodies_do_not_starve_health_or_pairing() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let directory = tempfile::tempdir().unwrap();
    let server = RemoteServer::new(
        AuthStore::open(directory.path()).unwrap(),
        Arc::new(Host::default()),
    );
    let listener = server
        .listen(([127, 0, 0, 1], 0).into(), None)
        .await
        .unwrap();
    let origin = format!("http://{}", listener.address);
    let mut stalled = Vec::new();
    for _ in 0..32 {
        let mut socket = tokio::net::TcpStream::connect(listener.address)
            .await
            .unwrap();
        socket.write_all(format!("POST /api/pair HTTP/1.1\r\nHost: {}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{{", listener.address).as_bytes()).await.unwrap();
        stalled.push(socket);
    }
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("{origin}/api/health"))
            .send()
            .await
            .unwrap()
            .status(),
        200,
        "partial bodies must not consume all request permits"
    );
    let paired = client.post(format!("{origin}/api/pair")).header("Origin", &origin)
        .json(&json!({"code":server.pairing().code,"name":"Phone","clientNonce":uuid::Uuid::new_v4().to_string()})).send().await.unwrap();
    assert_eq!(paired.status(), 200);
    let mut response = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(12),
        stalled[0].read_to_end(&mut response),
    )
    .await
    .expect("incomplete body must expire")
    .unwrap();
    assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 408"));
    server.stop().await;
}

#[tokio::test]
async fn fixed_https_origins_accept_browser_normalization_for_pairing_and_websockets() {
    let directory = tempfile::tempdir().unwrap();
    let server = RemoteServer::new(
        AuthStore::open(directory.path()).unwrap(),
        Arc::new(Host::default()),
    );
    let listener = server
        .listen(([127, 0, 0, 1], 0).into(), None)
        .await
        .unwrap();
    server
        .add_endpoint(Endpoint {
            url: "https://ExAmPlE.test:443".into(),
            kind: EndpointKind::Tunnel,
        })
        .unwrap();
    let origin = "https://example.test";
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let paired = client.post(format!("http://{}/api/pair", listener.address))
        .header("Host", "example.test").header("Origin", origin)
        .json(&json!({"code":server.pairing().code,"name":"Phone","clientNonce":uuid::Uuid::new_v4().to_string()}))
        .send().await.unwrap();
    assert_eq!(
        paired.status(),
        200,
        "A browser-normalized Origin must match the configured HTTPS origin"
    );
    let paired: Value = paired.json().await.unwrap();
    let mut request = format!("ws://{}/api/events", listener.address)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Host", "example.test".parse().unwrap());
    request
        .headers_mut()
        .insert("Origin", origin.parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket
        .send(Message::Text(
            json!({"token":paired["token"]}).to_string().into(),
        ))
        .await
        .unwrap();
    let event: Value =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(event["event"], "connection:ready");
    server
        .add_endpoint(Endpoint {
            url: origin.into(),
            kind: EndpointKind::Tunnel,
        })
        .unwrap();
    let manifest = server.manifest();
    let public: Vec<_> = manifest
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.kind == EndpointKind::Tunnel)
        .collect();
    assert_eq!(public.len(), 1);
    assert_eq!(public[0].url, origin);
    for wrong in ["https://example.test:444", "https://example.test.evil"] {
        assert_eq!(
            client
                .get(format!("http://{}/api/health", listener.address))
                .header("Host", "example.test")
                .header("Origin", wrong)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    socket.close(None).await.unwrap();
    server.stop().await;
}
#[async_trait::async_trait]
impl RemoteHost for Host {
    async fn execute(&self, owner: &str, _command: RemoteCommand) -> Result<Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"owner":owner}))
    }
    async fn disconnected(&self, owner: &str) {
        self.disconnected.lock().unwrap().push(owner.into());
    }
    fn asset(&self, path: &str) -> Option<WebAsset> {
        (path == "phone.html").then(|| WebAsset {
            bytes: b"phone application".to_vec(),
            mime_type: "text/html".into(),
        })
    }
}
#[tokio::test]
async fn authenticated_phone_routes_keep_origin_scope_and_revocation_on_real_sockets() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Host::default());
    let server = RemoteServer::new(AuthStore::open(dir.path()).unwrap(), host.clone());
    let listener = server
        .listen(([127, 0, 0, 1], 0).into(), None)
        .await
        .unwrap();
    let origin = format!("http://{}", listener.address);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert_eq!(
        client
            .get(&origin)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "phone application"
    );
    assert_eq!(
        client
            .get(format!("{origin}/api/manifest"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{origin}/api/health"))
            .header("Host", "attacker.test")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{origin}/api/health"))
            .header("Origin", "https://attacker.test")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let code = server.pairing().code;
    let nonce = uuid::Uuid::new_v4().to_string();
    let paired: Value = client
        .post(format!("{origin}/api/pair"))
        .header("Origin", &origin)
        .json(&json!({"code":code,"name":"Phone","clientNonce":nonce}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = paired["token"].as_str().unwrap();
    let owner = paired["device"]["id"].as_str().unwrap();
    let rpc = |body: Value| {
        client
            .post(format!("{origin}/api/rpc"))
            .header("Origin", &origin)
            .bearer_auth(token)
            .json(&body)
    };
    assert_eq!(
        rpc(json!({"method":"invoke","params":{"command":"run_shell"}}))
            .send()
            .await
            .unwrap()
            .status(),
        422
    );
    assert_eq!(rpc(json!({"method":"chat.start","params":{"conversationId":"chat","connectionId":"model","message":"hello","idempotencyKey":"bad"}})).send().await.unwrap().status(),400);
    assert_eq!(host.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        rpc(json!({"method":"connections.list"}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mut request = format!("ws://{}/api/events", listener.address)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", origin.parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket
        .send(Message::Text(json!({"token":token}).to_string().into()))
        .await
        .unwrap();
    let ready: Value =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(ready["event"], "connection:ready");
    server.publish(Some("other-owner"), "secret", json!({"never":"sent"}));
    server.publish(Some(owner), "owned", json!({"ok":true}));
    loop {
        let event: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_ne!(event["event"], "secret");
        if event["event"] == "owned" {
            break;
        }
    }
    socket
        .send(Message::Text(
            json!({"id":7,"method":"live.audio","params":{"sessionId":"session","data":"AA=="}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    loop {
        let event: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        if event["event"] == "connection:ack" {
            assert_eq!(event["payload"]["id"], 7);
            break;
        }
    }
    server.revoke(owner).await.unwrap();
    let closed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Some(Ok(Message::Close(frame))) = socket.next().await {
                return frame.unwrap().code;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(u16::from(closed), 1008);
    assert_eq!(
        rpc(json!({"method":"connections.list"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert!(host
        .disconnected
        .lock()
        .unwrap()
        .iter()
        .any(|id| id == owner));
    server.stop().await;
    drop(listener);
}
#[tokio::test]
async fn lan_tls_validates_with_the_public_certificate_and_reuses_the_same_ca() {
    let dir = tempfile::tempdir().unwrap();
    let addresses = vec!["127.0.0.1".parse().unwrap()];
    let identity = tls::identity(dir.path(), &addresses).await.unwrap();
    let first = std::fs::read(&identity.certificate_path).unwrap();
    let second = tls::identity(dir.path(), &addresses).await.unwrap();
    assert_eq!(first, std::fs::read(second.certificate_path).unwrap());
    let server = RemoteServer::new(
        AuthStore::open(dir.path()).unwrap(),
        Arc::new(Host::default()),
    );
    let listener = server
        .listen(([127, 0, 0, 1], 0).into(), Some(identity.config))
        .await
        .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .add_root_certificate(reqwest::Certificate::from_pem(&first).unwrap())
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("https://{}/api/health", listener.address))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert!(server.listen(([0, 0, 0, 0], 0).into(), None).await.is_err());
    server.stop().await;
}

#[tokio::test]
async fn lan_tls_recovers_each_incomplete_certificate_pair() {
    for missing in ["Nexa-LAN-CA.crt", "lan-ca-key.pem"] {
        let dir = tempfile::tempdir().unwrap();
        let addresses = vec!["127.0.0.1".parse().unwrap()];
        tls::identity(dir.path(), &addresses).await.unwrap();
        let stored: Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("lan-ca-identity.json")).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("lan-ca-key.pem"),
            stored["key_pem"].as_str().unwrap(),
        )
        .unwrap();
        std::fs::remove_file(dir.path().join("lan-ca-identity.json")).unwrap();
        std::fs::remove_file(dir.path().join(missing)).unwrap();
        let recovered = tls::identity(dir.path(), &addresses).await.unwrap();
        assert!(recovered.certificate_path.is_file());
        let stable = std::fs::read(recovered.certificate_path).unwrap();
        let repeated = tls::identity(dir.path(), &addresses).await.unwrap();
        assert_eq!(stable, std::fs::read(repeated.certificate_path).unwrap());
        std::fs::remove_file(dir.path().join("Nexa-LAN-CA.crt")).unwrap();
        let exported = tls::identity(dir.path(), &addresses).await.unwrap();
        assert_eq!(stable, std::fs::read(exported.certificate_path).unwrap());
    }
}

#[tokio::test]
#[ignore = "creates a temporary public tunnel exposing only this synthetic test host"]
async fn verified_public_tunnel_serves_only_the_synthetic_host_and_stops_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let server = RemoteServer::new(
        AuthStore::open(directory.path()).unwrap(),
        Arc::new(Host::default()),
    );
    let listener = server
        .listen(([127, 0, 0, 1], 0).into(), None)
        .await
        .unwrap();
    let helper = std::env::var_os("NEXA_TEST_TUNNEL_CACHE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| directory.path().join("helper"));
    let tunnel = tunnel::start(&helper, &format!("http://{}", listener.address))
        .await
        .unwrap();
    server
        .add_endpoint(Endpoint {
            url: tunnel.url.clone(),
            kind: EndpointKind::Tunnel,
        })
        .unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let mut verified = false;
    let mut last_error = String::new();
    for _ in 0..6 {
        match client
            .get(format!("{}/api/health", tunnel.url))
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                match response.json::<Value>().await {
                    Ok(payload) if payload["serverId"] == server.server_id() => {
                        verified = true;
                        break;
                    }
                    Ok(_) => last_error = format!("Unexpected health payload, HTTP {status}"),
                    Err(error) => last_error = format!("HTTP {status}: {error}"),
                }
            }
            Err(error) => last_error = format!("{error:?}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    if verified {
        let paired: Value = client.post(format!("{}/api/pair", tunnel.url))
            .header("Origin", &tunnel.url)
            .json(&json!({"code":server.pairing().code,"name":"Synthetic phone","clientNonce":uuid::Uuid::new_v4().to_string()}))
            .send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
        let token = paired["token"].as_str().unwrap();
        let owner = paired["device"]["id"].as_str().unwrap();
        let result: Value = client
            .post(format!("{}/api/rpc", tunnel.url))
            .header("Origin", &tunnel.url)
            .bearer_auth(token)
            .json(&json!({"method":"connections.list"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(result["result"]["owner"], owner);
        let mut request = format!("{}/api/events", tunnel.url.replacen("https:", "wss:", 1))
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Origin", tunnel.url.parse().unwrap());
        let (mut socket, _) =
            tokio::time::timeout(std::time::Duration::from_secs(15), connect_async(request))
                .await
                .unwrap()
                .unwrap();
        socket
            .send(Message::Text(json!({"token":token}).to_string().into()))
            .await
            .unwrap();
        let ready = tokio::time::timeout(std::time::Duration::from_secs(15), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(ready.to_text().unwrap()).unwrap()["event"],
            "connection:ready"
        );
        socket.send(Message::Text(json!({"id":1,"method":"live.audio","params":{"sessionId":"fixture","data":"AAA="}}).to_string().into())).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let message = socket.next().await.unwrap().unwrap();
                let event: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                if event["event"] == "connection:ack" {
                    assert_eq!(event["payload"]["id"], 1);
                    break;
                }
                assert_eq!(event["event"], "connection:heartbeat");
            }
        })
        .await
        .unwrap();
        socket.close(None).await.unwrap();
    }
    assert!(
        tunnel.stop().await,
        "the helper must stop before this test returns"
    );
    server.stop().await;
    drop(listener);
    assert!(
        verified,
        "the temporary HTTPS route did not reach the synthetic host: {last_error}"
    );
}
