use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

const PAIR_LIFETIME: Duration = Duration::from_secs(180);
const MAX_PAIR_ATTEMPTS: u8 = 5;
const MAX_DEVICES: usize = 12;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub created_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct StoredDevice {
    device: Device,
    token_hash: String,
}
#[derive(Serialize, Deserialize)]
struct StoredAuth {
    server_id: String,
    devices: Vec<StoredDevice>,
}
struct PairChallenge {
    code_hash: blake3::Hash,
    expires: Instant,
    attempts: u8,
    issued: Option<(String, PairedDevice)>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedDevice {
    pub device: Device,
    pub token: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub code: String,
    pub expires_at: String,
}

pub struct AuthStore {
    path: PathBuf,
    stored: StoredAuth,
    challenge: Option<PairChallenge>,
}
impl AuthStore {
    pub fn open(directory: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(directory)
            .map_err(|_| "Unable to create remote settings directory")?;
        let path = directory.join("devices.json");
        let stored = match std::fs::read(&path) {
            Ok(data) => {
                if data.len() > 128 * 1024 {
                    return Err("Remote device registry is too large".into());
                }
                serde_json::from_slice(&data).map_err(|_| "Remote device registry is invalid")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => StoredAuth {
                server_id: uuid::Uuid::new_v4().to_string(),
                devices: vec![],
            },
            Err(_) => return Err("Unable to read remote device registry".into()),
        };
        let store = Self {
            path,
            stored,
            challenge: None,
        };
        store.persist()?;
        Ok(store)
    }
    pub fn server_id(&self) -> &str {
        &self.stored.server_id
    }
    pub fn devices(&self) -> Vec<Device> {
        self.stored
            .devices
            .iter()
            .map(|entry| entry.device.clone())
            .collect()
    }
    pub fn pairing_device_id(&self) -> Option<String> {
        self.challenge
            .as_ref()
            .and_then(|challenge| challenge.issued.as_ref())
            .map(|(_, paired)| paired.device.id.clone())
    }
    fn persist(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec(&self.stored)
            .map_err(|_| "Unable to encode remote device registry")?;
        let temp = self.path.with_extension("tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| "Unable to write remote device registry")?;
        use std::io::Write;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Unable to save remote device registry")?;
        drop(file);
        std::fs::rename(temp, &self.path)
            .map_err(|_| "Unable to replace remote device registry".to_string())
    }
    pub fn rotate_pairing(&mut self) -> PairingCode {
        let random = uuid::Uuid::new_v4();
        let number = u32::from_le_bytes(random.as_bytes()[..4].try_into().unwrap()) % 1_000_000;
        let code = format!("{number:06}");
        self.challenge = Some(PairChallenge {
            code_hash: blake3::hash(code.as_bytes()),
            expires: Instant::now() + PAIR_LIFETIME,
            attempts: 0,
            issued: None,
        });
        PairingCode {
            code,
            expires_at: (chrono::Utc::now() + chrono::Duration::seconds(180)).to_rfc3339(),
        }
    }
    pub fn pair(&mut self, code: &str, name: &str, nonce: &str) -> Result<PairedDevice, String> {
        if name.trim().is_empty()
            || name.chars().count() > 80
            || uuid::Uuid::parse_str(nonce).is_err()
        {
            return Err("Invalid device name or pairing request".into());
        }
        let challenge = self
            .challenge
            .as_mut()
            .ok_or("Generate a new pairing code on the desktop")?;
        if Instant::now() > challenge.expires || challenge.attempts >= MAX_PAIR_ATTEMPTS {
            return Err(
                "Pairing code expired or locked; generate a new code on the desktop".into(),
            );
        }
        challenge.attempts += 1;
        if code.len() != 6
            || !bool::from(
                challenge
                    .code_hash
                    .as_bytes()
                    .ct_eq(blake3::hash(code.as_bytes()).as_bytes()),
            )
        {
            return Err("Incorrect pairing code".into());
        }
        if let Some((previous_nonce, paired)) = &challenge.issued {
            return if previous_nonce == nonce {
                Ok(paired.clone())
            } else {
                Err("This pairing code has already been used".into())
            };
        }
        if self.stored.devices.len() >= MAX_DEVICES {
            return Err("Remove an unused paired device before adding another".into());
        }
        let device = Device {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.trim().into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let paired = PairedDevice {
            device: device.clone(),
            token: token.clone(),
        };
        self.stored.devices.push(StoredDevice {
            device,
            token_hash: blake3::hash(token.as_bytes()).to_hex().to_string(),
        });
        if let Err(error) = self.persist() {
            self.stored.devices.pop();
            return Err(error);
        }
        self.challenge.as_mut().unwrap().issued = Some((nonce.into(), paired.clone()));
        Ok(paired)
    }
    pub fn authenticate(&self, token: &str) -> Option<Device> {
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let hash = blake3::hash(token.as_bytes()).to_hex();
        self.stored
            .devices
            .iter()
            .find(|entry| bool::from(entry.token_hash.as_bytes().ct_eq(hash.as_bytes())))
            .map(|entry| entry.device.clone())
    }
    pub fn has_device(&self, id: &str) -> bool {
        self.stored
            .devices
            .iter()
            .any(|entry| entry.device.id == id)
    }
    pub fn revoke(&mut self, id: &str) -> Result<(), String> {
        let previous = self.stored.devices.clone();
        self.stored.devices.retain(|entry| entry.device.id != id);
        if let Err(error) = self.persist() {
            self.stored.devices = previous;
            return Err(error);
        }
        self.challenge = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairing_is_one_time_idempotent_revocable_and_never_persists_raw_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AuthStore::open(dir.path()).unwrap();
        let code = store.rotate_pairing().code;
        let nonce = uuid::Uuid::new_v4().to_string();
        let paired = store.pair(&code, "Phone", &nonce).unwrap();
        assert_eq!(
            store.pair(&code, "Phone", &nonce).unwrap().token,
            paired.token
        );
        assert!(store
            .pair(&code, "Other", &uuid::Uuid::new_v4().to_string())
            .is_err());
        assert!(!std::fs::read_to_string(dir.path().join("devices.json"))
            .unwrap()
            .contains(&paired.token));
        let mut reopened = AuthStore::open(dir.path()).unwrap();
        assert!(reopened.authenticate(&paired.token).is_some());
        reopened.revoke(&paired.device.id).unwrap();
        assert!(reopened.authenticate(&paired.token).is_none());
        assert!(AuthStore::open(dir.path())
            .unwrap()
            .authenticate(&paired.token)
            .is_none());
    }
    #[test]
    fn incorrect_codes_lock_and_expired_codes_cannot_pair() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AuthStore::open(dir.path()).unwrap();
        let code = store.rotate_pairing().code;
        let nonce = uuid::Uuid::new_v4().to_string();
        let incorrect = if code == "000000" { "000001" } else { "000000" };
        for _ in 0..5 {
            assert!(store.pair(incorrect, "Phone", &nonce).is_err());
        }
        assert!(store.pair(&code, "Phone", &nonce).is_err());
        let code = store.rotate_pairing().code;
        store.challenge.as_mut().unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert!(store.pair(&code, "Phone", &nonce).is_err());
    }
}
