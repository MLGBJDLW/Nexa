//! Desktop OAuth: public configuration in SQLite, secrets in the OS vault, no model login tool.
mod network;
#[cfg(test)]
mod tests;
mod vault;
pub use network::{bearer_challenge, BearerChallenge};

use super::McpServer;
use crate::{db::Database, error::CoreError};
use aes_gcm::aead::{rand_core::RngCore, OsRng};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    Url,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OAuthConfig {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub issuer: Option<String>,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub redirect_port: Option<u16>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthStatus {
    pub connector_id: String,
    pub config: Option<OAuthConfig>,
    pub status: String,
    pub authorization_epoch: u64,
    pub expires_at: Option<i64>,
    pub scopes: Vec<String>,
    pub detail: Option<String>,
    pub login_id: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginOperation {
    pub login_id: String,
    pub authorization_url: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectReceipt {
    pub local_disconnected: bool,
    pub remote_revocation: String,
}

#[derive(Serialize, Deserialize)]
struct Credential {
    discovery: network::Discovery,
    client_id: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: i64,
    scopes: Vec<String>,
    generation: u64,
}
struct AuthRow {
    config: OAuthConfig,
    credential_id: Option<String>,
    login_id: Option<String>,
}
pub struct McpAuthService {
    db: Database,
    vault: vault::Vault,
    operations: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    logins: Mutex<HashMap<String, CancellationToken>>,
    vault_transaction: tokio::sync::Mutex<()>,
}
#[derive(Clone)]
pub(crate) struct RequestAuth {
    service: Arc<McpAuthService>,
    connector_id: String,
    epoch: u64,
}

pub(super) fn auth_error(code: &str, message: &str) -> CoreError {
    CoreError::McpAuth {
        code: code.into(),
        message: message.into(),
    }
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn random_secret() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

impl McpAuthService {
    pub fn shared(db: &Database) -> Arc<Self> {
        static SERVICES: OnceLock<Mutex<HashMap<usize, Weak<McpAuthService>>>> = OnceLock::new();
        let mut services = SERVICES
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        services.retain(|_, value| value.strong_count() > 0);
        let key = db.runtime_identity();
        if let Some(service) = services.get(&key).and_then(Weak::upgrade) {
            return service;
        }
        let service = Arc::new(Self {
            db: db.clone(),
            vault: vault::Vault::shared(),
            operations: Mutex::new(HashMap::new()),
            logins: Mutex::new(HashMap::new()),
            vault_transaction: tokio::sync::Mutex::new(()),
        });
        services.insert(key, Arc::downgrade(&service));
        service
    }
    fn row(&self, id: &str) -> Result<Option<AuthRow>, CoreError> {
        let row = self
            .db
            .conn()
            .query_row(
                "SELECT config_json,credential_id,login_id FROM mcp_oauth WHERE connector_id=?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        row.map(|(config, credential_id, login_id)| {
            Ok(AuthRow {
                config: serde_json::from_str(&config)?,
                credential_id,
                login_id,
            })
        })
        .transpose()
    }

    /// Called once during application startup; browser callbacks cannot survive a process exit.
    pub fn recover_logins(db: &Database) -> Result<(), CoreError> {
        let mut conn = db.conn();
        let tx = conn.transaction()?;
        tx.execute("UPDATE mcp_servers SET oauth_epoch=oauth_epoch+1 WHERE id IN (SELECT connector_id FROM mcp_oauth WHERE status='authorizing')",[])?;
        tx.execute("UPDATE mcp_oauth SET status='login_failed',login_id=NULL,detail='Sign-in was interrupted. Start a new sign-in from settings.' WHERE status='authorizing'",[])?;
        tx.commit()?;
        Ok(())
    }
    pub fn status(&self, id: &str) -> Result<OAuthStatus, CoreError> {
        let server = self.db.get_mcp_server(id)?;
        let row = self.db.conn().query_row("SELECT config_json,status,expires_at,scopes,detail,login_id FROM mcp_oauth WHERE connector_id=?1", [id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get(2)?,row.get::<_,String>(3)?,row.get(4)?,row.get(5)?))).optional()?;
        Ok(match row {
            Some((config, status, expires_at, scopes, detail, login_id)) => OAuthStatus {
                connector_id: id.into(),
                config: Some(serde_json::from_str(&config)?),
                status,
                authorization_epoch: server.oauth_epoch,
                expires_at,
                scopes: scopes.split_whitespace().map(String::from).collect(),
                detail,
                login_id,
            },
            None => OAuthStatus {
                connector_id: id.into(),
                config: None,
                status: "not_configured".into(),
                authorization_epoch: server.oauth_epoch,
                expires_at: None,
                scopes: vec![],
                detail: None,
                login_id: None,
            },
        })
    }
    pub async fn configure(
        &self,
        id: &str,
        config: Option<OAuthConfig>,
    ) -> Result<OAuthStatus, CoreError> {
        let server = self.db.get_mcp_server(id)?;
        if let Some(config) = &config {
            validate_config(&server, config)?;
        }
        self.cancel_listener(id);
        {
            let mut conn = self.db.conn();
            let tx = conn.transaction()?;
            tx.execute(
                "UPDATE mcp_servers SET oauth_epoch=oauth_epoch+1 WHERE id=?1",
                [id],
            )?;
            if let Some(config) = config {
                tx.execute("INSERT INTO mcp_oauth(connector_id,config_json) VALUES(?1,?2) ON CONFLICT(connector_id) DO UPDATE SET config_json=excluded.config_json,credential_id=NULL,expires_at=NULL,scopes='',status='signed_out',detail=NULL,login_id=NULL",params![id,serde_json::to_string(&config)?])?;
            } else {
                tx.execute("DELETE FROM mcp_oauth WHERE connector_id=?1", [id])?;
            }
            tx.commit()?;
        }
        self.cleanup().await?;
        self.status(id)
    }
    fn operation(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.operations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.into())
            .or_default()
            .clone()
    }
    fn cancel_listener(&self, id: &str) {
        if let Some(cancel) = self
            .logins
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
        {
            cancel.cancel();
        }
    }
    fn current(&self, id: &str, epoch: u64) -> Result<McpServer, CoreError> {
        let server = self.db.get_mcp_server(id)?;
        if !server.enabled || server.oauth_epoch != epoch {
            return Err(auth_error(
                "authorization_changed",
                "MCP authorization changed; refresh the connector before using it.",
            ));
        }
        Ok(server)
    }
    pub(crate) fn request_auth(
        self: &Arc<Self>,
        server: &McpServer,
    ) -> Result<Option<RequestAuth>, CoreError> {
        Ok(self.row(&server.id)?.map(|_| RequestAuth {
            service: self.clone(),
            connector_id: server.id.clone(),
            epoch: server.oauth_epoch,
        }))
    }
    pub async fn begin(self: &Arc<Self>, id: &str) -> Result<LoginOperation, CoreError> {
        let operation = self.operation(id);
        let _guard = operation.lock().await;
        let server = self.db.get_mcp_server(id)?;
        if !server.enabled {
            return Err(auth_error(
                "disabled",
                "Enable this connector before signing in.",
            ));
        }
        let row = self.row(id)?.ok_or_else(|| {
            auth_error("not_configured", "Save OAuth settings before signing in.")
        })?;
        validate_config(&server, &row.config)?;
        self.cancel_listener(id);
        let login_id = uuid::Uuid::new_v4().to_string();
        let epoch = {
            let mut conn = self.db.conn();
            let tx = conn.transaction()?;
            let changed = tx.execute(
                "UPDATE mcp_servers SET oauth_epoch=oauth_epoch+1 WHERE id=?1 AND oauth_epoch=?2",
                params![id, server.oauth_epoch],
            )?;
            if changed != 1 {
                return Err(auth_error(
                    "authorization_changed",
                    "Connector changed before sign-in could start.",
                ));
            }
            tx.execute("UPDATE mcp_oauth SET credential_id=NULL,expires_at=NULL,status='authorizing',detail=NULL,login_id=?2 WHERE connector_id=?1",params![id,login_id])?;
            let epoch = tx.query_row(
                "SELECT oauth_epoch FROM mcp_servers WHERE id=?1",
                [id],
                |row| row.get::<_, u64>(0),
            )?;
            tx.commit()?;
            epoch
        };
        let prepared = tokio::time::timeout(
            Duration::from_secs(60),
            self.prepare_login(&server, &row.config),
        )
        .await
        .unwrap_or_else(|_| {
            Err(auth_error(
                "timeout",
                "OAuth discovery timed out; retry from settings.",
            ))
        });
        let (discovery, listener, redirect_uri, client_id, verifier, state, authorization_url) =
            match prepared {
                Ok(value) => value,
                Err(error) => {
                    self.set_error(id, epoch, "login_failed", &error)?;
                    return Err(error);
                }
            };
        self.current(id, epoch)?;
        let cancel = CancellationToken::new();
        self.logins
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.into(), cancel.clone());
        let service = self.clone();
        let connector = id.to_string();
        let expected_login = login_id.clone();
        let scopes = row.config.scopes;
        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(auth_error("cancelled", "Sign-in was cancelled.")),
                result = tokio::time::timeout(Duration::from_secs(300), service.complete_login(&connector,epoch,&expected_login,listener,redirect_uri,client_id,verifier,state,discovery,scopes)) => result.unwrap_or_else(|_| Err(auth_error("timeout", "Sign-in timed out. Start a new sign-in from settings."))),
            };
            if let Err(error) = result {
                let _ = service.set_error(&connector, epoch, "login_failed", &error);
            }
            // A newer login owns its own cancellation token; never remove it here.
            let _ = service.cleanup().await;
        });
        Ok(LoginOperation {
            login_id,
            authorization_url,
        })
    }

    async fn prepare_login(
        &self,
        server: &McpServer,
        config: &OAuthConfig,
    ) -> Result<
        (
            network::Discovery,
            tokio::net::TcpListener,
            String,
            String,
            String,
            String,
            String,
        ),
        CoreError,
    > {
        let endpoint = server
            .url
            .as_deref()
            .ok_or_else(|| auth_error("invalid_config", "Remote URL required."))?;
        let discovery = network::discover(endpoint, config).await?;
        let origin =
            Url::parse(endpoint).map_err(|_| auth_error("invalid_config", "Invalid endpoint."))?;
        let listener = tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            config.redirect_port.unwrap_or(0),
        ))
        .await
        .map_err(|_| {
            auth_error(
                "callback_unavailable",
                "The loopback callback port is unavailable.",
            )
        })?;
        let redirect_uri = format!(
            "http://127.0.0.1:{}/oauth/callback",
            listener.local_addr()?.port()
        );
        let client_id = if let Some(client_id) = config.client_id.as_ref().filter(|v| !v.is_empty())
        {
            if client_id.starts_with("https://") && !discovery.cimd_supported {
                return Err(auth_error("client_registration_required","This server does not advertise client metadata documents; provide its pre-registered client ID."));
            }
            client_id.clone()
        } else {
            let registration = discovery.registration_endpoint.as_deref().ok_or_else(|| auth_error("client_registration_required","Provide a pre-registered client ID; this server has no dynamic registration endpoint."))?;
            let registration = network::json_request(registration,&origin,None,Some(&serde_json::json!({"client_name":"Nexa","application_type":"native","redirect_uris":[redirect_uri],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none"}))).await?;
            if registration
                .get("token_endpoint_auth_method")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|method| method != "none")
            {
                return Err(auth_error("client_registration_required","Nexa requires a public native client with token endpoint authentication 'none'."));
            }
            registration
                .get("client_id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty() && id.len() < 4096)
                .ok_or_else(|| {
                    auth_error(
                        "invalid_response",
                        "Registration returned no usable client ID.",
                    )
                })?
                .to_owned()
        };
        let verifier = random_secret();
        let state = random_secret();
        let mut url = Url::parse(&discovery.authorization_endpoint)
            .map_err(|_| auth_error("invalid_metadata", "Invalid authorization URL."))?;
        if url.query_pairs().any(|(key, _)| {
            matches!(
                key.as_ref(),
                "response_type"
                    | "client_id"
                    | "redirect_uri"
                    | "state"
                    | "code_challenge"
                    | "code_challenge_method"
                    | "resource"
                    | "scope"
            )
        }) {
            return Err(auth_error(
                "invalid_metadata",
                "Authorization endpoint contains reserved request parameters.",
            ));
        }
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", &client_id),
            ("redirect_uri", &redirect_uri),
            ("state", &state),
            ("code_challenge", &pkce_challenge(&verifier)),
            ("code_challenge_method", "S256"),
            ("resource", &discovery.resource),
        ]);
        if !config.scopes.is_empty() {
            url.query_pairs_mut()
                .append_pair("scope", &config.scopes.join(" "));
        }
        Ok((
            discovery,
            listener,
            redirect_uri,
            client_id,
            verifier,
            state,
            url.to_string(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn complete_login(
        &self,
        id: &str,
        epoch: u64,
        login_id: &str,
        listener: tokio::net::TcpListener,
        redirect_uri: String,
        client_id: String,
        verifier: String,
        state: String,
        discovery: network::Discovery,
        scopes: Vec<String>,
    ) -> Result<(), CoreError> {
        let code = loop {
            self.current(id, epoch)?;
            let (mut socket, peer) = tokio::select! { result=listener.accept()=>result?, _=tokio::time::sleep(Duration::from_secs(1))=>continue };
            if !peer.ip().is_loopback() {
                continue;
            }
            let request = tokio::time::timeout(Duration::from_secs(3), async {
                let mut bytes = Vec::new();
                let mut buffer = [0; 1024];
                while bytes.len() < 8192 {
                    let count = socket.read(&mut buffer).await?;
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                        break;
                    }
                }
                Ok::<_, std::io::Error>(String::from_utf8_lossy(&bytes).into_owned())
            })
            .await;
            let Ok(Ok(request)) = request else { continue };
            let result = callback_code(&request, &state, &discovery);
            let accepted = matches!(result, Ok(Some(_)));
            let body = if accepted {
                "Nexa received the sign-in response. Return to Nexa to check the connection."
            } else {
                "This sign-in response could not be accepted. Return to Nexa."
            };
            let response = format!("HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Security-Policy: default-src 'none'\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",if accepted {"200 OK"} else {"400 Bad Request"},body.len(),body);
            let _ = socket.write_all(response.as_bytes()).await;
            match result {
                Ok(Some(code)) => break code,
                Ok(None) => continue,
                Err(error) => return Err(error),
            }
        };
        drop(listener);
        let server = self.current(id, epoch)?;
        if self.row(id)?.and_then(|row| row.login_id).as_deref() != Some(login_id) {
            return Err(auth_error(
                "authorization_changed",
                "Sign-in was superseded.",
            ));
        }
        let origin = Url::parse(server.url.as_deref().unwrap_or_default())
            .map_err(|_| auth_error("invalid_config", "Invalid endpoint."))?;
        let form = vec![
            ("grant_type".into(), "authorization_code".into()),
            ("code".into(), code),
            ("client_id".into(), client_id.clone()),
            ("redirect_uri".into(), redirect_uri),
            ("code_verifier".into(), verifier),
            ("resource".into(), discovery.resource.clone()),
        ];
        let value =
            network::json_request(&discovery.token_endpoint, &origin, Some(&form), None).await?;
        let credential = parse_token(value, discovery, client_id, None, &scopes, 0)?;
        self.publish(id, epoch, None, Some(login_id), credential, false)
            .await
    }

    fn set_error(
        &self,
        id: &str,
        epoch: u64,
        status: &str,
        error: &CoreError,
    ) -> Result<(), CoreError> {
        self.db.conn().execute("UPDATE mcp_oauth SET status=?3,detail=?4,login_id=NULL WHERE connector_id=?1 AND EXISTS(SELECT 1 FROM mcp_servers WHERE id=?1 AND oauth_epoch=?2)",params![id,epoch,status,error.to_string()])?;
        Ok(())
    }
    async fn publish(
        &self,
        id: &str,
        epoch: u64,
        old_id: Option<&str>,
        login_id: Option<&str>,
        credential: Credential,
        advance_epoch: bool,
    ) -> Result<(), CoreError> {
        let _vault_guard = self.vault_transaction.lock().await;
        self.current(id, epoch)?;
        let key = uuid::Uuid::new_v4().to_string();
        // Register cleanup before the OS operation: cancellation/crash cannot orphan a late write.
        self.db
            .conn()
            .execute("INSERT INTO mcp_oauth_cleanup VALUES(?1)", [&key])?;
        self.vault
            .write(&key, serde_json::to_string(&credential)?)
            .await?;
        let published = {
            let mut conn = self.db.conn();
            let tx = conn.transaction()?;
            let expires_at = (credential.expires_at != i64::MAX).then_some(credential.expires_at);
            let affected = tx.execute("UPDATE mcp_oauth SET credential_id=?3,expires_at=?4,scopes=?5,status='connected',detail=NULL,login_id=NULL WHERE connector_id=?1 AND credential_id IS ?6 AND login_id IS ?7 AND EXISTS(SELECT 1 FROM mcp_servers WHERE id=?1 AND oauth_epoch=?2 AND enabled=1)",params![id,epoch,key,expires_at,credential.scopes.join(" "),old_id,login_id])?;
            if affected == 1 {
                if advance_epoch {
                    tx.execute(
                        "UPDATE mcp_servers SET oauth_epoch=oauth_epoch+1 WHERE id=?1",
                        [id],
                    )?;
                }
                tx.execute(
                    "DELETE FROM mcp_oauth_cleanup WHERE credential_id=?1",
                    [&key],
                )?;
            }
            tx.commit()?;
            affected == 1
        };
        if !published {
            self.vault.delete(&key).await?;
            return Err(auth_error(
                "authorization_changed",
                "Authorization changed before credentials could be activated.",
            ));
        }
        Ok(())
    }
    async fn credential(&self, id: &str) -> Result<(String, Credential), CoreError> {
        let key = self
            .row(id)?
            .and_then(|row| row.credential_id)
            .ok_or_else(|| {
                auth_error(
                    "login_required",
                    "Sign in to this MCP connector from settings.",
                )
            })?;
        let value = self.vault.read(&key).await?.ok_or_else(|| {
            auth_error(
                "login_required",
                "The system credential is missing; sign in again.",
            )
        })?;
        let credential = serde_json::from_str(&value).map_err(|_| {
            auth_error(
                "invalid_credential",
                "Stored OAuth credential is invalid; disconnect and sign in again.",
            )
        })?;
        Ok((key, credential))
    }
    async fn token(
        &self,
        id: &str,
        epoch: u64,
        rejected: Option<&str>,
    ) -> Result<String, CoreError> {
        let operation = self.operation(id);
        let _guard = operation.lock().await;
        let server = self.current(id, epoch)?;
        let (old_id, mut credential) = self.credential(id).await?;
        self.current(id, epoch)?;
        let status = self.status(id)?.status;
        if status == "reauthorization_required" {
            return Err(auth_error(
                "login_required",
                "This authorization must be renewed from settings.",
            ));
        }
        if rejected.is_none_or(|token| token != credential.access_token)
            && status != "refresh_failed"
            && credential.expires_at > now() + 30
        {
            return Ok(credential.access_token);
        }
        let refresh = match credential.refresh_token.as_ref() {
            Some(refresh) => refresh,
            None => {
                let error = auth_error(
                    "login_required",
                    "This authorization has no refresh token; sign in again.",
                );
                self.set_error(id, epoch, "reauthorization_required", &error)?;
                return Err(error);
            }
        };
        let origin = Url::parse(server.url.as_deref().unwrap_or_default())
            .map_err(|_| auth_error("invalid_config", "Invalid endpoint."))?;
        let form = vec![
            ("grant_type".into(), "refresh_token".into()),
            ("refresh_token".into(), refresh.clone()),
            ("client_id".into(), credential.client_id.clone()),
            ("resource".into(), credential.discovery.resource.clone()),
        ];
        let result = network::json_request(
            &credential.discovery.token_endpoint,
            &origin,
            Some(&form),
            None,
        )
        .await;
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                let status = if matches!(&error,CoreError::McpAuth{code,..} if code=="invalid_grant")
                {
                    "reauthorization_required"
                } else {
                    "refresh_failed"
                };
                self.set_error(id, epoch, status, &error)?;
                return Err(error);
            }
        };
        let previous_scopes = credential.scopes.clone();
        credential = parse_token(
            value,
            credential.discovery,
            credential.client_id,
            credential.refresh_token,
            &previous_scopes,
            credential.generation + 1,
        )?;
        if credential
            .scopes
            .iter()
            .any(|scope| !previous_scopes.contains(scope))
        {
            let error = auth_error(
                "scope_changed",
                "Authorization returned additional scopes; reconnect explicitly from settings.",
            );
            self.set_error(id, epoch, "reauthorization_required", &error)?;
            return Err(error);
        }
        let scopes_changed = credential.scopes != previous_scopes;
        let token = credential.access_token.clone();
        self.publish(id, epoch, Some(&old_id), None, credential, scopes_changed)
            .await?;
        // The new reference is durable. Cleanup failure cannot make it use its predecessor.
        let _ = self.cleanup().await;
        if scopes_changed {
            return Err(auth_error(
                "authorization_changed",
                "Authorization scopes changed; refresh the connector.",
            ));
        }
        self.current(id, epoch)?;
        Ok(token)
    }
    pub async fn disconnect(
        &self,
        id: &str,
        revoke_remote: bool,
    ) -> Result<DisconnectReceipt, CoreError> {
        self.cancel_listener(id);
        let server = self.db.get_mcp_server(id)?;
        let old = {
            let mut conn = self.db.conn();
            let tx = conn.transaction()?;
            let old = tx
                .query_row(
                    "SELECT credential_id FROM mcp_oauth WHERE connector_id=?1",
                    [id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
            tx.execute(
                "UPDATE mcp_servers SET oauth_epoch=oauth_epoch+1 WHERE id=?1",
                [id],
            )?;
            tx.execute("UPDATE mcp_oauth SET credential_id=NULL,expires_at=NULL,status='disconnected',detail=NULL,login_id=NULL WHERE connector_id=?1",[id])?;
            tx.commit()?;
            old
        };
        let mut remote = "not_requested".to_string();
        if revoke_remote {
            remote = "not_supported".into();
            if let Some(key) = old {
                match self.vault.read(&key).await {
                    Ok(Some(value)) => {
                        if let Ok(credential) = serde_json::from_str::<Credential>(&value) {
                            if let (Some(endpoint), Ok(origin)) = (
                                &credential.discovery.revocation_endpoint,
                                Url::parse(server.url.as_deref().unwrap_or_default()),
                            ) {
                                let token = credential
                                    .refresh_token
                                    .as_ref()
                                    .unwrap_or(&credential.access_token);
                                let form = vec![
                                    ("token".into(), token.clone()),
                                    ("client_id".into(), credential.client_id.clone()),
                                ];
                                remote = if network::revoke(endpoint, &origin, &form).await.is_ok()
                                {
                                    "confirmed"
                                } else {
                                    "failed"
                                }
                                .into();
                            }
                        }
                    }
                    Err(_) => remote = "failed".into(),
                    _ => (),
                }
            }
        }
        // Public status already records local disconnection even if keychain cleanup is unavailable.
        if let Err(error) = self.cleanup().await {
            self.db.conn().execute(
                "UPDATE mcp_oauth SET detail=?2 WHERE connector_id=?1",
                params![id, error.to_string()],
            )?;
        }
        Ok(DisconnectReceipt {
            local_disconnected: true,
            remote_revocation: remote,
        })
    }
    pub async fn cleanup(&self) -> Result<(), CoreError> {
        let _vault_guard = self.vault_transaction.lock().await;
        let keys = {
            let conn = self.db.conn();
            let mut statement =
                conn.prepare("SELECT credential_id FROM mcp_oauth_cleanup LIMIT 128")?;
            let keys = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            keys
        };
        for key in keys {
            self.vault.delete(&key).await?;
            self.db.conn().execute(
                "DELETE FROM mcp_oauth_cleanup WHERE credential_id=?1",
                [key],
            )?;
        }
        Ok(())
    }
}

impl RequestAuth {
    pub fn note_rejection(&self, status: u16, challenge: Option<&BearerChallenge>) {
        let scope = challenge
            .and_then(|challenge| challenge.scope.as_deref())
            .unwrap_or_default()
            .chars()
            .take(2048)
            .collect::<String>();
        let detail = if status == 403 && !scope.is_empty() {
            format!("The connector requests scopes: {scope}. Review the scopes in OAuth settings and sign in explicitly.")
        } else {
            format!("HTTP {status} rejected authorization. Retry or sign in from settings; the operation was not replayed.")
        };
        let _=self.service.db.conn().execute("UPDATE mcp_oauth SET detail=?3,status=CASE WHEN ?4=401 THEN 'refresh_failed' ELSE status END WHERE connector_id=?1 AND EXISTS(SELECT 1 FROM mcp_servers WHERE id=?1 AND oauth_epoch=?2)",params![self.connector_id,self.epoch,detail,status]);
    }
    pub fn validate_target(&self, url: &Url) -> Result<(), CoreError> {
        let server = self.service.current(&self.connector_id, self.epoch)?;
        let origin = Url::parse(server.url.as_deref().unwrap_or_default())
            .map_err(|_| auth_error("invalid_config", "Invalid MCP endpoint."))?;
        network::validate_url(url.as_str(), &origin)?;
        if url.origin() != origin.origin() {
            return Err(auth_error(
                "resource_mismatch",
                "OAuth credentials cannot follow an MCP endpoint to another origin.",
            ));
        }
        Ok(())
    }
    pub async fn apply(&self, headers: &mut HeaderMap, force: bool) -> Result<(), CoreError> {
        let rejected = if force {
            headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
        } else {
            None
        };
        let token = self
            .service
            .token(&self.connector_id, self.epoch, rejected)
            .await?;
        let mut value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
            auth_error(
                "invalid_token",
                "Authorization server returned an invalid bearer token.",
            )
        })?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
        Ok(())
    }
}

fn validate_config(server: &McpServer, config: &OAuthConfig) -> Result<(), CoreError> {
    if server.transport == "stdio" || server.builtin_id.is_some() {
        return Err(auth_error(
            "invalid_config",
            "OAuth is available for user-configured remote MCP connectors.",
        ));
    }
    let origin = Url::parse(server.url.as_deref().unwrap_or_default())
        .map_err(|_| auth_error("invalid_config", "Remote URL required."))?;
    network::validate_url(origin.as_str(), &origin)?;
    if let Some(headers) = &server.headers_json {
        let headers: BTreeMap<String, String> = serde_json::from_str(headers)?;
        if headers.keys().any(|key| {
            key.eq_ignore_ascii_case("authorization") || key.eq_ignore_ascii_case("cookie")
        }) {
            return Err(auth_error(
                "invalid_config",
                "Remove Authorization and Cookie headers before configuring OAuth.",
            ));
        }
    }
    if config.scopes.len() > 64
        || config.scopes.iter().any(|scope| {
            scope.is_empty()
                || scope.len() > 256
                || scope
                    .chars()
                    .any(|ch| !ch.is_ascii() || ch.is_whitespace() || ch.is_control())
        })
        || config.client_id.as_ref().is_some_and(|value| {
            value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control)
        })
        || config.redirect_port == Some(0)
    {
        return Err(auth_error(
            "invalid_config",
            "Invalid OAuth client ID, scopes, or fixed callback port.",
        ));
    }
    for value in [&config.issuer, &config.resource].into_iter().flatten() {
        network::validate_url(value, &origin)?;
    }
    Ok(())
}
fn callback_code(
    request: &str,
    state: &str,
    discovery: &network::Discovery,
) -> Result<Option<String>, CoreError> {
    let line = request.lines().next().unwrap_or_default();
    let parts = line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "GET" || !parts[1].starts_with("/oauth/callback?") {
        return Ok(None);
    }
    let url = Url::parse(&format!("http://127.0.0.1{}", parts[1]))
        .map_err(|_| auth_error("invalid_callback", "Invalid OAuth callback."))?;
    let mut values = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if values
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Ok(None);
        }
    }
    let Some(received) = values.get("state") else {
        return Ok(None);
    };
    if state.as_bytes().ct_eq(received.as_bytes()).unwrap_u8() != 1 {
        return Ok(None);
    }
    if values
        .get("iss")
        .is_some_and(|issuer| issuer != &discovery.issuer)
        || discovery.iss_required && !values.contains_key("iss")
    {
        return Err(auth_error(
            "issuer_mismatch",
            "OAuth callback issuer did not match this login.",
        ));
    }
    if values.contains_key("error") {
        return Err(auth_error(
            "authorization_denied",
            "Authorization was declined or failed in the browser.",
        ));
    }
    values
        .remove("code")
        .filter(|code| !code.is_empty() && code.len() <= 4096)
        .map(Some)
        .ok_or_else(|| auth_error("invalid_callback", "OAuth callback has no usable code."))
}
fn parse_token(
    value: serde_json::Value,
    discovery: network::Discovery,
    client_id: String,
    old_refresh: Option<String>,
    requested_scopes: &[String],
    generation: u64,
) -> Result<Credential, CoreError> {
    if !value
        .get("token_type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("bearer"))
    {
        return Err(auth_error(
            "invalid_token",
            "Only bearer access tokens are supported.",
        ));
    }
    let token = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| {
                !value.is_empty() && value.len() <= 24_576 && !value.chars().any(char::is_control)
            })
            .map(String::from)
    };
    let access_token = token("access_token").ok_or_else(|| {
        auth_error(
            "invalid_token",
            "Authorization server returned no usable access token.",
        )
    })?;
    let refresh_token = token("refresh_token").or(old_refresh);
    let expires_at = value
        .get("expires_in")
        .and_then(serde_json::Value::as_u64)
        .map(|seconds| now() + seconds.min(365 * 24 * 3600) as i64)
        .unwrap_or(i64::MAX);
    let mut scopes = value
        .get("scope")
        .and_then(serde_json::Value::as_str)
        .map(|scope| {
            scope
                .split_whitespace()
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| requested_scopes.to_vec());
    scopes.sort();
    scopes.dedup();
    Ok(Credential {
        discovery,
        client_id,
        access_token,
        refresh_token,
        expires_at,
        scopes,
        generation,
    })
}
