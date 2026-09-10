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
