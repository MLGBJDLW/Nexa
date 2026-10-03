use super::*;

#[test]
fn pkce_and_challenge_parser_follow_wire_contract() {
    assert_eq!(
        pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    assert_ne!(random_secret(), random_secret());
    let mut headers = HeaderMap::new();
    headers.insert(reqwest::header::WWW_AUTHENTICATE,HeaderValue::from_static("Basic realm=\"unrelated\", bEaReR resource_metadata=\"https://example.com/meta?a=1,b=2\", scope=\"read write\", error=\"insufficient_scope\""));
    let challenge = bearer_challenge(&headers).unwrap();
    assert_eq!(
        challenge.resource_metadata.as_deref(),
        Some("https://example.com/meta?a=1,b=2")
    );
    assert_eq!(challenge.scope.as_deref(), Some("read write"));
    assert_eq!(challenge.error.as_deref(), Some("insufficient_scope"));
}

fn discovery() -> network::Discovery {
    network::Discovery {
        issuer: "https://issuer.example".into(),
        resource: "https://mcp.example/api".into(),
        authorization_endpoint: "https://issuer.example/authorize".into(),
        token_endpoint: "https://issuer.example/token".into(),
        revocation_endpoint: None,
        registration_endpoint: None,
        scopes_supported: vec![],
        iss_required: true,
        cimd_supported: false,
    }
}
#[test]
fn callback_requires_exact_state_and_issuer_before_code_exchange() {
    let metadata = discovery();
    assert!(callback_code(
        "GET /oauth/callback?state=wrong&code=code HTTP/1.1",
        "expected",
        &metadata
    )
    .unwrap()
    .is_none());
    assert!(callback_code(
        "GET /oauth/callback?state=expected&code=code HTTP/1.1",
        "expected",
        &metadata
    )
    .is_err());
    assert!(callback_code(
        "GET /oauth/callback?state=expected&code=code&iss=https%3A%2F%2Fissuer.example%2F HTTP/1.1",
        "expected",
        &metadata
    )
    .is_err());
    assert_eq!(callback_code("GET /oauth/callback?state=expected&code=code&iss=https%3A%2F%2Fissuer.example HTTP/1.1","expected",&metadata).unwrap().as_deref(),Some("code"));
    assert!(callback_code(
        "GET /oauth/callback?state=expected&state=expected&code=code HTTP/1.1",
        "expected",
        &metadata
    )
    .unwrap()
    .is_none());
}

struct Peer {
    origin: String,
    task: tokio::task::JoinHandle<()>,
    token_calls: Arc<std::sync::atomic::AtomicUsize>,
    tool_calls: Arc<std::sync::atomic::AtomicUsize>,
    mode: Arc<std::sync::atomic::AtomicUsize>,
    reject_tool: Arc<std::sync::atomic::AtomicBool>,
    reject_list_once: Arc<std::sync::atomic::AtomicBool>,
    forms: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn peer() -> Peer {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let token_calls = Arc::new(AtomicUsize::new(0));
    let tool_calls = Arc::new(AtomicUsize::new(0));
    let mode = Arc::new(AtomicUsize::new(0));
    let reject_tool = Arc::new(AtomicBool::new(false));
    let reject_list_once = Arc::new(AtomicBool::new(false));
    let forms = Arc::new(Mutex::new(Vec::new()));
    let (base, tokens, tools, flags, deny_tool, deny_list, records) = (
        origin.clone(),
        token_calls.clone(),
        tool_calls.clone(),
        mode.clone(),
        reject_tool.clone(),
        reject_list_once.clone(),
        forms.clone(),
    );
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let (base, tokens, tools, flags, deny_tool, deny_list, records) = (
                base.clone(),
                tokens.clone(),
                tools.clone(),
                flags.clone(),
                deny_tool.clone(),
                deny_list.clone(),
                records.clone(),
            );
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let (header_end, body_length) = loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..position]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        break (position + 4, length);
                    }
                };
                while bytes.len() < header_end + body_length {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let header = String::from_utf8_lossy(&bytes[..header_end]);
                let request = header.lines().next().unwrap_or_default();
                let path = request.split_whitespace().nth(1).unwrap_or_default();
                let mut status = "200 OK";
                let mut extra = String::new();
                let value = match (request.starts_with("GET "), path) {
                    (true, "/mcp") => {
                        status = "401 Unauthorized";
                        extra = format!(
                            "WWW-Authenticate: Bearer resource_metadata=\"{base}/metadata\"\r\n"
                        );
                        serde_json::json!({})
                    }
                    (true, "/metadata") => {
                        serde_json::json!({"resource":format!("{base}/mcp"),"authorization_servers":[base],"scopes_supported":["read"]})
                    }
                    (true, "/.well-known/oauth-authorization-server") => {
                        serde_json::json!({"issuer":base,"authorization_endpoint":format!("{base}/authorize"),"token_endpoint":format!("{base}/token"),"registration_endpoint":format!("{base}/register"),"revocation_endpoint":format!("{base}/revoke"),"code_challenge_methods_supported":["S256"],"authorization_response_iss_parameter_supported":true})
                    }
                    (false, "/register") => {
                        let value: serde_json::Value =
                            serde_json::from_slice(&bytes[header_end..]).unwrap();
                        assert_eq!(value["application_type"], "native");
                        assert_eq!(value["token_endpoint_auth_method"], "none");
                        serde_json::json!({"client_id":"native-fixture","token_endpoint_auth_method":"none"})
                    }
                    (false, "/token") => {
                        let form = url::form_urlencoded::parse(&bytes[header_end..])
                            .map(|(k, v)| (k.into_owned(), v.into_owned()))
                            .collect::<BTreeMap<_, _>>();
                        assert_eq!(form.get("resource"), Some(&format!("{base}/mcp")));
                        records.lock().unwrap().push(form.clone());
                        let n = tokens.fetch_add(1, Ordering::SeqCst) + 1;
                        let flag = flags.load(Ordering::SeqCst);
                        if flag == 3 {
                            tokio::time::sleep(Duration::from_millis(150)).await;
                        }
                        if flag == 1 {
                            status = "503 Service Unavailable";
                            serde_json::json!({"error":"temporarily_unavailable","error_description":"never expose server-supplied-secret"})
                        } else if flag == 2 {
                            status = "400 Bad Request";
                            serde_json::json!({"error":"invalid_grant"})
                        } else {
                            serde_json::json!({"access_token":format!("access-{n}"),"refresh_token":format!("refresh-{n}"),"token_type":"Bearer","expires_in":3600,"scope":if flag==4 {""} else if flag==5 {"read write"} else {"read"}})
                        }
                    }
                    (false, "/revoke") => {
                        if flags.load(Ordering::SeqCst) == 1 {
                            status = "503 Service Unavailable";
                        }
                        serde_json::json!({})
                    }
                    (false, "/mcp") => {
                        assert!(header
                            .to_ascii_lowercase()
                            .contains("authorization: bearer access-"));
                        let value: serde_json::Value =
                            serde_json::from_slice(&bytes[header_end..]).unwrap();
                        let method = value["method"].as_str().unwrap_or_default();
                        let result = match method {
                            "initialize" => {
                                serde_json::json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"oauth-fixture","version":"1"}})
                            }
                            "notifications/initialized" => {
                                status = "202 Accepted";
                                serde_json::Value::Null
                            }
                            "tools/list" => {
                                if deny_list.swap(false, Ordering::SeqCst) {
                                    status = "401 Unauthorized";
                                }
                                serde_json::json!({"tools":[{"name":"write","inputSchema":{"type":"object"}}]})
                            }
                            "tools/call" => {
                                tools.fetch_add(1, Ordering::SeqCst);
                                if deny_tool.load(Ordering::SeqCst) {
                                    status = "401 Unauthorized";
                                }
                                serde_json::json!({"content":[{"type":"text","text":"written"}]})
                            }
                            _ => serde_json::json!({}),
                        };
                        serde_json::json!({"jsonrpc":"2.0","id":value["id"],"result":result})
                    }
                    _ => {
                        status = "404 Not Found";
                        serde_json::json!({})
                    }
                };
                let body = if status == "202 Accepted" {
                    String::new()
                } else {
                    value.to_string()
                };
                let response=format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",body.len());
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    Peer {
        origin,
        task,
        token_calls,
        tool_calls,
        mode,
        reject_tool,
        reject_list_once,
        forms,
    }
}

fn test_service(peer: &Peer) -> (Arc<McpAuthService>, McpServer) {
    let db = Database::open_memory().unwrap();
    let server = db
        .save_mcp_server(&super::super::SaveMcpServerInput {
            id: None,
            name: "OAuth fixture".into(),
            transport: "streamable_http".into(),
            url: Some(format!("{}/mcp", peer.origin)),
            command: None,
            args: None,
            env_json: None,
            headers_json: None,
            enabled: true,
        })
        .unwrap();
    (
        Arc::new(McpAuthService {
            db,
            vault: vault::Vault::memory(),
            operations: Mutex::new(HashMap::new()),
            logins: Mutex::new(HashMap::new()),
            vault_transaction: tokio::sync::Mutex::new(()),
        }),
        server,
    )
}
async fn finish_login(
    service: &Arc<McpAuthService>,
    server: &McpServer,
    peer: &Peer,
) -> LoginOperation {
    service
        .configure(
            &server.id,
            Some(OAuthConfig {
                scopes: vec!["read".into()],
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    let login = service.begin(&server.id).await.unwrap();
    let url = Url::parse(&login.authorization_url).unwrap();
    let params = url.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["resource"], format!("{}/mcp", peer.origin));
    let mut callback = Url::parse(&params["redirect_uri"]).unwrap();
    callback.query_pairs_mut().extend_pairs([
        ("state", params["state"].as_str()),
        ("code", "fixture-code"),
        ("iss", peer.origin.as_str()),
    ]);
    assert!(reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(callback)
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = service.status(&server.id).unwrap();
            if status.status == "connected" {
                break;
            }
            assert_eq!(status.status, "authorizing", "{status:?}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let form = peer.forms.lock().unwrap().last().unwrap().clone();
    assert_eq!(form["redirect_uri"], params["redirect_uri"]);
    assert_eq!(
        pkce_challenge(&form["code_verifier"]),
        params["code_challenge"]
    );
    login
}

#[tokio::test]
async fn oauth_login_refresh_and_http_retry_preserve_effect_boundaries() {
    use std::sync::atomic::Ordering;
    let peer = peer().await;
    let (service, server) = test_service(&peer);
    finish_login(&service, &server, &peer).await;
    let active = service.db.get_mcp_server(&server.id).unwrap();
    let before_digest = super::super::identity::connector_trust_digest(&active);
    let auth = service.request_auth(&active).unwrap().unwrap();
    let mut client = super::super::client::McpClient::connect_remote_authorized(
        &format!("{}/mcp", peer.origin),
        None,
        "oauth-test",
        false,
        Some(auth.clone()),
    )
    .await
    .unwrap();
    peer.reject_list_once.store(true, Ordering::SeqCst);
    assert_eq!(client.list_tools().await.unwrap().len(), 1);
    assert_eq!(
        peer.token_calls.load(Ordering::SeqCst),
        2,
        "one refresh after an actual read-side 401"
    );
    peer.reject_tool.store(true, Ordering::SeqCst);
    assert!(matches!(
        client.call_tool("write", serde_json::json!({})).await,
        Err(CoreError::McpHttpAuth { status: 401, .. })
    ));
    assert_eq!(
        peer.tool_calls.load(Ordering::SeqCst),
        1,
        "never transparently replay tools/call"
    );
    assert_eq!(peer.token_calls.load(Ordering::SeqCst), 2);
    let (key, mut credential) = service.credential(&server.id).await.unwrap();
    credential.expires_at = now() - 1;
    service
        .vault
        .write(&key, serde_json::to_string(&credential).unwrap())
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        service.token(&server.id, active.oauth_epoch, None),
        service.token(&server.id, active.oauth_epoch, None)
    );
    assert_eq!(a.unwrap(), "access-3");
    assert_eq!(b.unwrap(), "access-3");
    assert_eq!(peer.token_calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        super::super::identity::connector_trust_digest(
            &service.db.get_mcp_server(&server.id).unwrap()
        ),
        before_digest,
        "rotation does not move grants"
    );
    let status = serde_json::to_string(&service.status(&server.id).unwrap()).unwrap();
    assert!(!status.contains("access-"));
    assert!(!status.contains("refresh-"));
    assert!(service
        .db
        .get_mcp_server(&server.id)
        .unwrap()
        .headers_json
        .is_none());
    let disconnected = service.disconnect(&server.id, true).await.unwrap();
    assert!(disconnected.local_disconnected);
    assert_eq!(disconnected.remote_revocation, "confirmed");
    assert!(auth.apply(&mut HeaderMap::new(), false).await.is_err());
}

#[tokio::test]
async fn oauth_late_callback_and_refresh_cannot_resurrect_disconnected_authority() {
    use std::sync::atomic::Ordering;
    let peer = peer().await;
    let (service, server) = test_service(&peer);
    finish_login(&service, &server, &peer).await;
    let active = service.db.get_mcp_server(&server.id).unwrap();
    let (key, mut credential) = service.credential(&server.id).await.unwrap();
    credential.expires_at = now() - 1;
    service
        .vault
        .write(&key, serde_json::to_string(&credential).unwrap())
        .await
        .unwrap();
    peer.mode.store(3, Ordering::SeqCst);
    let refreshing = {
        let service = service.clone();
        let id = server.id.clone();
        tokio::spawn(async move { service.token(&id, active.oauth_epoch, None).await })
    };
    while peer.token_calls.load(Ordering::SeqCst) < 2 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    service.disconnect(&server.id, false).await.unwrap();
    assert!(refreshing.await.unwrap().is_err());
    assert_eq!(service.status(&server.id).unwrap().status, "disconnected");
    assert!(service
        .row(&server.id)
        .unwrap()
        .unwrap()
        .credential_id
        .is_none());
    let login = service.begin(&server.id).await.unwrap();
    let url = Url::parse(&login.authorization_url).unwrap();
    let params = url.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
    service.db.toggle_mcp_server(&server.id, false).unwrap();
    let mut callback = Url::parse(&params["redirect_uri"]).unwrap();
    callback.query_pairs_mut().extend_pairs([
        ("state", params["state"].as_str()),
        ("code", "late-code"),
        ("iss", peer.origin.as_str()),
    ]);
    let _ = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(callback)
        .send()
        .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        peer.token_calls.load(Ordering::SeqCst),
        2,
        "stale callback never exchanges its code"
    );
    assert_eq!(service.status(&server.id).unwrap().status, "signed_out");
}

#[tokio::test]
async fn oauth_refresh_failures_and_scope_changes_are_distinct_and_secret_safe() {
    use std::sync::atomic::Ordering;
    let peer = peer().await;
    let (service, server) = test_service(&peer);
    finish_login(&service, &server, &peer).await;
    let active = service.db.get_mcp_server(&server.id).unwrap();
    peer.mode.store(1, Ordering::SeqCst);
    let error = service
        .token(&server.id, active.oauth_epoch, Some("access-1"))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("server-supplied-secret"));
    assert_eq!(service.status(&server.id).unwrap().status, "refresh_failed");
    let receipt = service.disconnect(&server.id, true).await.unwrap();
    assert!(receipt.local_disconnected);
    assert_eq!(receipt.remote_revocation, "failed");
    peer.mode.store(0, Ordering::SeqCst);
    finish_login(&service, &server, &peer).await;
    let active = service.db.get_mcp_server(&server.id).unwrap();
    let (_, credential) = service.credential(&server.id).await.unwrap();
    peer.mode.store(2, Ordering::SeqCst);
    assert!(service
        .token(
            &server.id,
            active.oauth_epoch,
            Some(&credential.access_token)
        )
        .await
        .is_err());
    assert_eq!(
        service.status(&server.id).unwrap().status,
        "reauthorization_required"
    );
    peer.mode.store(0, Ordering::SeqCst);
    finish_login(&service, &server, &peer).await;
    let active = service.db.get_mcp_server(&server.id).unwrap();
    let (_, credential) = service.credential(&server.id).await.unwrap();
    peer.mode.store(4, Ordering::SeqCst);
    assert!(service
        .token(
            &server.id,
            active.oauth_epoch,
            Some(&credential.access_token)
        )
        .await
        .is_err());
    assert!(service.db.get_mcp_server(&server.id).unwrap().oauth_epoch > active.oauth_epoch);
    assert!(service.status(&server.id).unwrap().scopes.is_empty());
}

#[tokio::test]
#[ignore = "explicit OS credential-store smoke test; writes only random temporary fixture credentials"]
async fn os_vault_roundtrip_handles_long_tokens_and_deletes_temporary_credentials() {
    let vault = vault::Vault::shared();
    let id = format!("fixture-{}", uuid::Uuid::new_v4());
    let value = "temporary-oauth-fixture".repeat(500);
    vault.write(&id, value.clone()).await.unwrap();
    let read = vault.read(&id).await;
    let deleted = vault.delete(&id).await;
    assert_eq!(read.unwrap().as_deref(), Some(value.as_str()));
    deleted.unwrap();
    assert!(vault.read(&id).await.unwrap().is_none());
}

#[tokio::test]
async fn expired_credentials_without_refresh_show_reauthorization_required() {
    let peer = peer().await;
    let (service, server) = test_service(&peer);
    finish_login(&service, &server, &peer).await;
    let active = service.db.get_mcp_server(&server.id).unwrap();
    let (key, mut credential) = service.credential(&server.id).await.unwrap();
    credential.expires_at = now() - 1;
    credential.refresh_token = None;
    service
        .vault
        .write(&key, serde_json::to_string(&credential).unwrap())
        .await
        .unwrap();
    assert!(service
        .token(&server.id, active.oauth_epoch, None)
        .await
        .is_err());
    assert_eq!(
        service.status(&server.id).unwrap().status,
        "reauthorization_required"
    );
    assert!(service
        .status(&server.id)
        .unwrap()
        .detail
        .unwrap()
        .contains("no refresh token"));
}

#[tokio::test]
async fn initial_login_rejects_unrequested_scopes_before_publishing_credentials() {
    let peer = peer().await;
    let (service, server) = test_service(&peer);
    service
        .configure(
            &server.id,
            Some(OAuthConfig {
                scopes: vec!["read".into()],
                ..Default::default()
            }),
        )
        .await
        .unwrap();
    peer.mode.store(5, std::sync::atomic::Ordering::SeqCst);
    let login = service.begin(&server.id).await.unwrap();
    let params = Url::parse(&login.authorization_url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect::<BTreeMap<_, _>>();
    let mut callback = Url::parse(&params["redirect_uri"]).unwrap();
    callback.query_pairs_mut().extend_pairs([
        ("state", params["state"].as_str()),
        ("code", "fixture-code"),
        ("iss", peer.origin.as_str()),
    ]);
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(callback)
        .send()
        .await
        .unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = service.status(&server.id).unwrap();
            if status.status != "authorizing" {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(status.status, "login_failed");
    assert!(status.detail.unwrap().contains("unrequested scopes"));
    assert!(service
        .row(&server.id)
        .unwrap()
        .unwrap()
        .credential_id
        .is_none());
    assert!(service.credential(&server.id).await.is_err());
}
