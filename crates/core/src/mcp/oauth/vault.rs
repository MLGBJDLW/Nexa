//! All OS vault operations execute on one dedicated thread. Secrets never use a file fallback.
use crate::error::CoreError;
use std::sync::{mpsc, Arc, OnceLock};
use tokio::sync::oneshot;

enum Operation {
    Read,
    Write(String),
    Delete,
}
struct Request {
    id: String,
    operation: Operation,
    reply: oneshot::Sender<Result<Option<String>, VaultError>>,
}
#[derive(Debug)]
enum VaultError {
    Unavailable,
    InvalidCredential,
}
impl From<keyring_core::Error> for VaultError {
    fn from(error: keyring_core::Error) -> Self {
        match error {
            keyring_core::Error::BadEncoding(_) | keyring_core::Error::BadDataFormat(_, _) => {
                Self::InvalidCredential
            }
            _ => Self::Unavailable,
        }
    }
}
#[derive(Clone)]
pub(super) struct Vault(mpsc::SyncSender<Request>);

impl Vault {
    #[cfg(test)]
    pub fn memory() -> Self {
        let (tx, rx) = mpsc::sync_channel::<Request>(64);
        std::thread::spawn(move || {
            let mut values = std::collections::HashMap::new();
            for request in rx {
                let result = match request.operation {
                    Operation::Read => values.get(&request.id).cloned(),
                    Operation::Write(value) => {
                        values.insert(request.id, value);
                        None
                    }
                    Operation::Delete => {
                        values.remove(&request.id);
                        None
                    }
                };
                let _ = request.reply.send(Ok(result));
            }
        });
        Self(tx)
    }
    pub fn shared() -> Self {
        static INSTANCE: OnceLock<Vault> = OnceLock::new();
        INSTANCE
            .get_or_init(|| {
                let (tx, rx) = mpsc::sync_channel::<Request>(64);
                std::thread::Builder::new()
                    .name("mcp-credential-vault".into())
                    .spawn(move || {
                        let mut store = None;
                        for request in rx {
                            let result = (|| {
                                if store.is_none() {
                                    store = Some(open_store().map_err(VaultError::from)?);
                                }
                                operate(
                                    store.as_deref().expect("initialized store"),
                                    &request.id,
                                    request.operation,
                                )
                            })();
                            if result.is_err() {
                                store = None;
                            }
                            let _ = request.reply.send(result);
                        }
                    })
                    .expect("start credential worker");
                Vault(tx)
            })
            .clone()
    }
    async fn request(&self, id: &str, operation: Operation) -> Result<Option<String>, CoreError> {
        let (reply, response) = oneshot::channel();
        self.0
            .try_send(Request {
                id: id.into(),
                operation,
                reply,
            })
            .map_err(|_| {
                CoreError::Mcp("System credential store is busy; retry shortly.".into())
            })?;
        tokio::time::timeout(std::time::Duration::from_secs(30), response)
            .await
            .map_err(|_| {
                CoreError::Mcp(
                    "System credential store is waiting for access; unlock it and retry.".into(),
                )
            })?
            .map_err(|_| CoreError::Mcp("Credential worker stopped.".into()))?
            .map_err(|error| match error {
                VaultError::Unavailable => CoreError::Mcp("System credential store operation failed; unlock it and retry.".into()),
                VaultError::InvalidCredential => super::auth_error("invalid_credential", "Stored OAuth credential is incomplete or corrupt; disconnect and sign in again."),
            })
    }
    pub async fn read(&self, id: &str) -> Result<Option<String>, CoreError> {
        self.request(id, Operation::Read).await
    }
    pub async fn write(&self, id: &str, value: String) -> Result<(), CoreError> {
        self.request(id, Operation::Write(value)).await.map(|_| ())
    }
    pub async fn delete(&self, id: &str) -> Result<(), CoreError> {
        self.request(id, Operation::Delete).await.map(|_| ())
    }
}

fn open_store() -> keyring_core::Result<Arc<keyring_core::CredentialStore>> {
    #[cfg(target_os = "windows")]
    {
        Ok(windows_native_keyring_store::Store::new()?)
    }
    #[cfg(target_os = "macos")]
    {
        Ok(apple_native_keyring_store::keychain::Store::new()?)
    }
    #[cfg(target_os = "linux")]
    {
        Ok(zbus_secret_service_keyring_store::Store::new()?)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err(keyring_core::Error::NotSupportedByStore(
            "OS vault unavailable".into(),
        ))
    }
}

fn entry(
    store: &keyring_core::CredentialStore,
    id: &str,
) -> keyring_core::Result<keyring_core::Entry> {
    #[cfg(target_os = "windows")]
    {
        store.build(
            "Nexa-McpOAuth-v1",
            id,
            Some(&std::collections::HashMap::from([("persistence", "Local")])),
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        store.build("Nexa-McpOAuth-v1", id, None)
    }
}

// Windows credentials have a small blob limit. Base64 chunks keep each password under 2 KiB
// in UTF-16 as well. The manifest is written first so interrupted writes remain deletable;
// SQLite only activates a new immutable record after every chunk has been stored.
fn operate(
    store: &keyring_core::CredentialStore,
    id: &str,
    operation: Operation,
) -> Result<Option<String>, VaultError> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let root = entry(store, id)?;
    let manifest = || match root.get_password() {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=128).contains(n))
            .ok_or(VaultError::InvalidCredential)
            .map(Some),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    };
    match operation {
        Operation::Write(value) => {
            let chunks = value.as_bytes().chunks(600).collect::<Vec<_>>();
            if chunks.is_empty() || chunks.len() > 128 {
                return Err(VaultError::InvalidCredential);
            }
            root.set_password(&chunks.len().to_string())?;
            for (index, chunk) in chunks.iter().enumerate() {
                entry(store, &format!("{id}/{index}"))?.set_password(&STANDARD.encode(chunk))?;
            }
            Ok(None)
        }
        Operation::Read => {
            let Some(count) = manifest()? else {
                return Ok(None);
            };
            let mut data = Vec::new();
            for index in 0..count {
                let chunk = match entry(store, &format!("{id}/{index}"))?.get_password() {
                    Ok(chunk) => chunk,
                    Err(keyring_core::Error::NoEntry) => return Err(VaultError::InvalidCredential),
                    Err(error) => return Err(error.into()),
                };
                data.extend(
                    STANDARD
                        .decode(chunk)
                        .map_err(|_| VaultError::InvalidCredential)?,
                );
            }
            String::from_utf8(data)
                .map(Some)
                .map_err(|_| VaultError::InvalidCredential)
        }
        Operation::Delete => {
            let count = match manifest() {
                Ok(Some(count)) => count,
                // A corrupt manifest cannot identify its chunks. Every credential
                // uses a random owned ID and at most 128 chunks, so clean only that
                // bounded namespace before removing its manifest.
                Ok(None) | Err(VaultError::InvalidCredential) => 128,
                Err(error) => return Err(error),
            };
            for index in 0..count {
                match entry(store, &format!("{id}/{index}"))?.delete_credential() {
                    Ok(()) | Err(keyring_core::Error::NoEntry) => (),
                    Err(error) => return Err(error.into()),
                }
            }
            match root.delete_credential() {
                Ok(()) | Err(keyring_core::Error::NoEntry) => (),
                Err(error) => return Err(error.into()),
            }
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_data_errors_are_permanent_without_retaining_secret_bytes() {
        for error in [
            keyring_core::Error::BadEncoding(b"secret-bytes".to_vec()),
            keyring_core::Error::BadDataFormat(
                b"secret-bytes".to_vec(),
                Box::new(std::io::Error::other("secret-detail")),
            ),
        ] {
            let error = VaultError::from(error);
            assert!(matches!(error, VaultError::InvalidCredential));
            assert!(!format!("{error:?}").contains("secret"));
        }
        assert!(matches!(
            VaultError::from(keyring_core::Error::NoDefaultStore),
            VaultError::Unavailable
        ));
    }

    #[test]
    #[ignore = "explicit native vault corruption/recovery smoke; random temporary credential only"]
    fn native_corrupt_manifest_is_classified_and_cleaned() {
        let store = open_store().unwrap();
        for corrupt in [true, false] {
            let id = format!("fixture-corrupt-{}", uuid::Uuid::new_v4());
            let root = entry(store.as_ref(), &id).unwrap();
            let chunk = entry(store.as_ref(), &format!("{id}/0")).unwrap();
            if corrupt {
                root.set_password("invalid-manifest").unwrap();
            }
            chunk.set_password("temporary-fixture-chunk").unwrap();
            let result = operate(store.as_ref(), &id, Operation::Read);
            let cleanup = operate(store.as_ref(), &id, Operation::Delete);
            if corrupt {
                assert!(matches!(result, Err(VaultError::InvalidCredential)));
            } else {
                assert!(matches!(result, Ok(None)));
            }
            cleanup.unwrap();
            assert!(matches!(
                root.get_password(),
                Err(keyring_core::Error::NoEntry)
            ));
            assert!(matches!(
                chunk.get_password(),
                Err(keyring_core::Error::NoEntry)
            ));
        }
    }
}
