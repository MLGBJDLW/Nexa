//! Portable model connections. Hosts retain their own credential storage and
//! execution policy; only reviewed provider/model fields cross a paired link.
use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::{
    conversation::{AgentConfig, SaveAgentConfigInput},
    db::Database,
    error::CoreError,
    llm::ProviderStreamingConfig,
};

pub const CONNECTION_SYNC_VERSION: u16 = 1;
pub const CONNECTION_SYNC_PAGE_SIZE: usize = 64;

// Deliberately no Debug: this type carries a credential on the native wire.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharedModelConnection {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub model: String,
    pub provider_endpoint_id: Option<String>,
    pub temperature: Option<f64>,
    pub context_window: Option<i64>,
    pub reasoning_enabled: Option<bool>,
    pub thinking_budget: Option<i64>,
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub provider_streaming: ProviderStreamingConfig,
    #[serde(default)]
    pub is_default: bool,
}

impl SharedModelConnection {
    pub fn validate(&self) -> Result<(), CoreError> {
        uuid::Uuid::parse_str(&self.id)
            .map_err(|_| CoreError::InvalidInput("Invalid shared connection identity".into()))?;
        if self.name.trim().is_empty()
            || self.name.len() > 512
            || self.model.trim().is_empty()
            || self.model.len() > 4096
            || self.api_key.is_empty()
            || self.api_key.len() > 65_536
            || self.api_key.starts_with("enc:v1:")
            || !self.is_portable()
        {
            return Err(CoreError::InvalidInput(
                "This model connection cannot be transferred to another device".into(),
            ));
        }
        let input = self.save_input(None, false)?;
        crate::conversation::validate_agent_config_credential_contract(&input)?;
        crate::conversation::validate_agent_config_numeric_overrides(&input)
    }

    pub fn is_portable(&self) -> bool {
        if self.api_key.is_empty()
            || crate::provider_registry::provider_type_from_key(&self.provider).is_none()
            || crate::external_agent::is_agent_runtime(&self.provider)
            || matches!(
                self.provider.as_str(),
                "bedrock" | "vertex" | "github_copilot_api"
            )
        {
            return false;
        }
        self.base_url.as_deref().is_none_or(|url| {
            url.len() <= 8192
                && url::Url::parse(url).ok().is_some_and(|url| {
                    let host = url.host_str().unwrap_or_default();
                    url.scheme() == "https"
                        && !host.is_empty()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.fragment().is_none()
                        && host != "localhost"
                        && !host.ends_with(".localhost")
                        && host != "127.0.0.1"
                        && host != "[::1]"
                })
        })
    }

    pub fn revision(&self) -> Result<String, CoreError> {
        let mut contents = serde_json::to_value(self)?;
        let fields = contents.as_object_mut().expect("connection is an object");
        fields.remove("id");
        fields.remove("isDefault"); // Each device owns its active/default choice.
        Ok(blake3::hash(&serde_json::to_vec(&contents)?)
            .to_hex()
            .to_string())
    }

    pub fn from_config(id: String, config: &AgentConfig) -> Self {
        Self {
            id,
            name: config.name.clone(),
            provider: config.provider.clone(),
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            model: config.model.clone(),
            provider_endpoint_id: config.provider_endpoint_id.clone(),
            temperature: config.temperature,
            context_window: config.context_window,
            reasoning_enabled: config.reasoning_enabled,
            thinking_budget: config.thinking_budget,
            reasoning_effort: config.reasoning_effort.clone(),
            provider_streaming: config.provider_streaming,
            is_default: config.is_default,
        }
    }

    fn save_input(
        &self,
        previous: Option<&AgentConfig>,
        make_default: bool,
    ) -> Result<SaveAgentConfigInput, CoreError> {
        let mut input = previous
            .map(serde_json::to_value)
            .transpose()?
            .unwrap_or(serde_json::json!({}));
        let fields = input.as_object_mut().expect("config is an object");
        let mut shared = serde_json::to_value(self)?;
        let shared = shared.as_object_mut().expect("connection is an object");
        shared.remove("id");
        shared.remove("isDefault");
        fields.extend(shared.clone());
        fields.insert(
            "id".into(),
            serde_json::json!(previous
                .map(|p| p.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())),
        );
        fields.insert("modelId".into(), serde_json::json!(&self.model));
        fields.insert(
            "isDefault".into(),
            serde_json::json!(previous.map_or(make_default, |p| p.is_default)),
        );
        Ok(serde_json::from_value(input)?)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharedConnectionPatch {
    pub connection: SharedModelConnection,
    pub base_revision: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionExchange {
    pub version: u16,
    #[serde(default)]
    pub connections: Vec<SharedConnectionPatch>,
    pub after: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionFork {
    pub source_id: String,
    pub copy_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionExchangeResult {
    pub version: u16,
    pub connections: Vec<SharedModelConnection>,
    pub next_after: Option<String>,
    pub created: usize,
    pub updated: usize,
    pub forks: Vec<ConnectionFork>,
    pub ignored_ids: Vec<String>,
    pub device_only_count: usize,
}

fn conflict_copy(
    connection: &SharedModelConnection,
    device: &str,
) -> Result<SharedModelConnection, CoreError> {
    let hash = blake3::hash(
        format!(
            "nexa-connection-conflict-v1:{}:{}",
            connection.id,
            connection.revision()?
        )
        .as_bytes(),
    );
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hash.as_bytes()[..16]);
    let mut copy = connection.clone();
    copy.id = uuid::Uuid::from_bytes(bytes).to_string();
    copy.name = format!(
        "{} · {}",
        connection.name.chars().take(96).collect::<String>(),
        device.chars().take(24).collect::<String>()
    );
    copy.is_default = false;
    Ok(copy)
}

impl Database {
    /// Allocate identities only for existing eligible configs. Secrets stay in
    /// the existing encrypted agent-config store, not in this identity table.
    pub fn shared_model_connections(
        &self,
    ) -> Result<(Vec<SharedModelConnection>, usize), CoreError> {
        let configs = self.list_agent_configs()?;
        let total = configs.len();
        let mut portable = configs
            .into_iter()
            .filter_map(|config| {
                let connection = SharedModelConnection::from_config(String::new(), &config);
                connection.is_portable().then_some((config.id, connection))
            })
            .collect::<Vec<_>>();
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (config_id, connection) in &mut portable {
            tx.execute(
                "INSERT OR IGNORE INTO shared_model_connections(sync_id,config_id) VALUES(?1,?2)",
                rusqlite::params![uuid::Uuid::new_v4().to_string(), config_id.as_str()],
            )?;
            connection.id = tx.query_row(
                "SELECT sync_id FROM shared_model_connections WHERE config_id=?1",
                [config_id.as_str()],
                |row| row.get(0),
            )?;
        }
        tx.commit()?;
        let mut connections = portable
            .into_iter()
            .map(|(_, connection)| connection)
            .collect::<Vec<_>>();
        connections.sort_by(|a, b| a.id.cmp(&b.id));
        let device_only = total - connections.len();
        Ok((connections, device_only))
    }

    fn shared_connection_target(&self, id: &str) -> Result<Option<Option<AgentConfig>>, CoreError> {
        let target: Option<Option<String>> = self
            .conn()
            .query_row(
                "SELECT config_id FROM shared_model_connections WHERE sync_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        target
            .map(|id| id.map(|id| self.get_agent_config(&id)).transpose())
            .transpose()
    }

    pub fn exchange_model_connections(
        &self,
        input: &ConnectionExchange,
        device: &str,
    ) -> Result<ConnectionExchangeResult, CoreError> {
        if input.version != CONNECTION_SYNC_VERSION
            || input.connections.len() > CONNECTION_SYNC_PAGE_SIZE
        {
            return Err(CoreError::InvalidInput(
                "Unsupported model connection sync version or page size".into(),
            ));
        }
        let mut seen = HashSet::new();
        for patch in &input.connections {
            patch.connection.validate()?;
            if !seen.insert(&patch.connection.id) {
                return Err(CoreError::InvalidInput(
                    "Duplicate shared connection in one page".into(),
                ));
            }
        }
        let mut result = ConnectionExchangeResult {
            version: CONNECTION_SYNC_VERSION,
            connections: vec![],
            next_after: None,
            created: 0,
            updated: 0,
            forks: vec![],
            ignored_ids: vec![],
            device_only_count: 0,
        };
        for patch in &input.connections {
            let incoming = &patch.connection;
            let previous = match self.shared_connection_target(&incoming.id)? {
                Some(None) => {
                    result.ignored_ids.push(incoming.id.clone());
                    continue;
                }
                Some(Some(previous)) => Some(previous),
                None => None,
            };
            let mut record = incoming.clone();
            let mut target = previous;
            if let Some(current) = &target {
                let current_revision =
                    SharedModelConnection::from_config(incoming.id.clone(), current).revision()?;
                let incoming_revision = incoming.revision()?;
                if current_revision == incoming_revision
                    || patch.base_revision.as_deref() == Some(&incoming_revision)
                {
                    continue;
                }
                if patch.base_revision.as_deref() != Some(&current_revision) {
                    record = conflict_copy(incoming, device)?;
                    result.forks.push(ConnectionFork {
                        source_id: incoming.id.clone(),
                        copy_id: record.id.clone(),
                    });
                    match self.shared_connection_target(&record.id)? {
                        Some(_) => continue, // Retry of an already preserved conflict.
                        None => target = None,
                    }
                }
            }
            let make_default = target.is_none() && self.list_agent_configs()?.is_empty();
            let saved = record.save_input(target.as_ref(), make_default)?;
            self.save_shared_agent_config(&saved, &record.id, target.as_ref())?;
            if target.is_none() {
                result.created += 1;
            } else {
                result.updated += 1;
            }
        }
        let (connections, device_only_count) = self.shared_model_connections()?;
        result.device_only_count = device_only_count;
        let mut page = connections
            .into_iter()
            .filter(|record| input.after.as_ref().is_none_or(|after| &record.id > after))
            .take(CONNECTION_SYNC_PAGE_SIZE + 1)
            .collect::<Vec<_>>();
        if page.len() > CONNECTION_SYNC_PAGE_SIZE {
            page.pop();
            result.next_after = page.last().map(|record| record.id.clone());
        }
        result.connections = page;
        Ok(result)
    }
}

/// Merge a native response against the request snapshot without overwriting a
/// phone edit made while the network request was in flight.
pub fn merge_connection_response(
    current: &mut HashMap<String, SharedModelConnection>,
    sent: &HashMap<String, String>,
    baselines: &mut HashMap<String, String>,
    ignored: &HashSet<String>,
    result: &ConnectionExchangeResult,
) -> Result<(), CoreError> {
    if result.version != CONNECTION_SYNC_VERSION {
        return Err(CoreError::InvalidInput(
            "Unsupported connection response".into(),
        ));
    }
    for remote in &result.connections {
        remote.validate()?;
        if ignored.contains(&remote.id) {
            continue;
        }
        let remote_revision = remote.revision()?;
        let local_revision = current
            .get(&remote.id)
            .map(SharedModelConnection::revision)
            .transpose()?;
        let sent_revision = sent.get(&remote.id);
        if local_revision.is_none()
            || local_revision.as_ref() == sent_revision
            || local_revision.as_deref() == Some(&remote_revision)
        {
            current.insert(remote.id.clone(), remote.clone());
            baselines.insert(remote.id.clone(), remote_revision);
        } else if sent_revision == Some(&remote_revision) {
            baselines.insert(remote.id.clone(), remote_revision);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(db: &Database, name: &str, key: &str) -> AgentConfig {
        db.save_agent_config(
            &serde_json::from_value(serde_json::json!({
                "name":name,"provider":"open_ai","apiKey":key,"baseUrl":"https://api.openai.com/v1",
                "model":"sync-test-model","isDefault":true,"maxIterations":7
            }))
            .unwrap(),
        )
        .unwrap()
    }
    fn request(
        records: &HashMap<String, SharedModelConnection>,
        baseline: &HashMap<String, String>,
    ) -> ConnectionExchange {
        ConnectionExchange {
            version: 1,
            after: None,
            connections: records
                .values()
                .map(|record| SharedConnectionPatch {
                    connection: record.clone(),
                    base_revision: baseline.get(&record.id).cloned(),
                })
                .collect(),
        }
    }
    fn sent(records: &HashMap<String, SharedModelConnection>) -> HashMap<String, String> {
        records
            .values()
            .map(|record| (record.id.clone(), record.revision().unwrap()))
            .collect()
    }

    #[test]
    fn bidirectional_connections_preserve_identity_credentials_policy_and_conflicts() {
        let db = Database::open_memory().unwrap();
        let original = config(&db, "Desktop connection", "fixture-desktop-key");
        let initial = db
            .exchange_model_connections(
                &ConnectionExchange {
                    version: 1,
                    connections: vec![],
                    after: None,
                },
                "Phone",
            )
            .unwrap();
        let shared_id = initial.connections[0].id.clone();
        let mut phone = HashMap::new();
        let mut baseline = HashMap::new();
        merge_connection_response(
            &mut phone,
            &HashMap::new(),
            &mut baseline,
            &HashSet::new(),
            &initial,
        )
        .unwrap();
        assert_eq!(phone[&shared_id].api_key, "fixture-desktop-key");
        let mut new_phone = phone[&shared_id].clone();
        new_phone.id = uuid::Uuid::new_v4().to_string();
        new_phone.name = "Phone connection".into();
        new_phone.api_key = "fixture-phone-key".into();
        phone.insert(new_phone.id.clone(), new_phone.clone());
        phone.get_mut(&shared_id).unwrap().api_key = "fixture-rotated-on-phone".into();
        let sent_state = sent(&phone);
        let exchanged = db
            .exchange_model_connections(&request(&phone, &baseline), "Phone")
            .unwrap();
        assert_eq!((exchanged.created, exchanged.updated), (1, 1));
        assert_eq!(
            db.get_agent_config(&original.id).unwrap().api_key,
            "fixture-rotated-on-phone"
        );
        assert_eq!(
            db.get_agent_config(&original.id).unwrap().max_iterations,
            Some(7)
        );
        assert_eq!(
            db.get_default_agent_config().unwrap().unwrap().id,
            original.id
        );
        let encrypted: String = db
            .conn()
            .query_row(
                "SELECT api_key FROM agent_configs WHERE id=?1",
                [&original.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(encrypted.starts_with("enc:v1:"));
        assert!(!encrypted.contains("fixture-rotated"));
        merge_connection_response(
            &mut phone,
            &sent_state,
            &mut baseline,
            &HashSet::new(),
            &exchanged,
        )
        .unwrap();

        let current = db.get_agent_config(&original.id).unwrap();
        let mut desktop_edit = SharedModelConnection::from_config(shared_id.clone(), &current);
        desktop_edit.model = "desktop-model-change".into();
        db.save_agent_config(&desktop_edit.save_input(Some(&current), false).unwrap())
            .unwrap();
        phone.get_mut(&shared_id).unwrap().api_key = "fixture-concurrent-phone-edit".into();
        let concurrent = request(&phone, &baseline);
        let sent_state = sent(&phone);
        let response = db.exchange_model_connections(&concurrent, "Phone").unwrap();
        assert_eq!(response.forks.len(), 1);
        let copy_id = response.forks[0].copy_id.clone();
        assert_eq!(
            db.get_agent_config(&original.id).unwrap().model,
            "desktop-model-change"
        );
        assert_eq!(
            db.get_agent_config(&original.id).unwrap().api_key,
            "fixture-rotated-on-phone"
        );
        merge_connection_response(
            &mut phone,
            &sent_state,
            &mut baseline,
            &HashSet::new(),
            &response,
        )
        .unwrap();
        assert_eq!(phone[&copy_id].api_key, "fixture-concurrent-phone-edit");
        assert_eq!(phone[&shared_id].model, "desktop-model-change");
        db.exchange_model_connections(&concurrent, "Phone").unwrap();
        assert_eq!(
            db.list_agent_configs().unwrap().len(),
            3,
            "Retry must not duplicate conflict copies"
        );

        let deleted = db
            .shared_connection_target(&copy_id)
            .unwrap()
            .unwrap()
            .unwrap();
        db.delete_agent_config(&deleted.id).unwrap();
        let response = db
            .exchange_model_connections(&request(&phone, &baseline), "Phone")
            .unwrap();
        assert!(response.ignored_ids.contains(&copy_id));
        assert_eq!(
            db.list_agent_configs().unwrap().len(),
            2,
            "Local deletion must not be resurrected by a peer"
        );
    }

    #[test]
    fn sync_guards_concurrent_saves_and_keeps_device_only_connections_local() {
        let db = Database::open_memory().unwrap();
        let original = config(&db, "Desktop", "fixture-key");
        let (records, _) = db.shared_model_connections().unwrap();
        let mut record = records[0].clone();
        let stale = record.save_input(Some(&original), false).unwrap();
        record.api_key = "fixture-new-key".into();
        db.save_agent_config(&record.save_input(Some(&original), false).unwrap())
            .unwrap();
        assert!(matches!(
            db.save_shared_agent_config(&stale, &record.id, Some(&original)),
            Err(CoreError::Conflict(_))
        ));
        assert_eq!(
            db.get_agent_config(&original.id).unwrap().api_key,
            "fixture-new-key"
        );
        record.base_url = Some("http://127.0.0.1:8108/v1".into());
        assert!(!record.is_portable());
        assert!(record.validate().is_err());
        db.save_agent_config(&record.save_input(Some(&original), false).unwrap())
            .unwrap();
        let (records, local_only) = db.shared_model_connections().unwrap();
        assert!(records.is_empty());
        assert_eq!(local_only, 1);
    }

    #[test]
    fn a_phone_edit_during_transfer_survives_the_acknowledgement() {
        let db = Database::open_memory().unwrap();
        config(&db, "Desktop", "fixture-key");
        let initial = db
            .exchange_model_connections(
                &ConnectionExchange {
                    version: 1,
                    connections: vec![],
                    after: None,
                },
                "Phone",
            )
            .unwrap();
        let mut phone = HashMap::new();
        let mut baseline = HashMap::new();
        merge_connection_response(
            &mut phone,
            &HashMap::new(),
            &mut baseline,
            &HashSet::new(),
            &initial,
        )
        .unwrap();
        let id = initial.connections[0].id.clone();
        let sent_state = sent(&phone);
        phone.get_mut(&id).unwrap().model = "edited-while-offline".into();
        merge_connection_response(
            &mut phone,
            &sent_state,
            &mut baseline,
            &HashSet::new(),
            &initial,
        )
        .unwrap();
        assert_eq!(phone[&id].model, "edited-while-offline");
        let next = db
            .exchange_model_connections(&request(&phone, &baseline), "Phone")
            .unwrap();
        assert!(next.forks.is_empty());
        assert_eq!(next.updated, 1);
    }
}
