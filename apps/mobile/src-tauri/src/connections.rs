//! Native-only connection library over the existing Android credential store.
use std::collections::{HashMap, HashSet};

use nexa_core::connection_sync::{
    merge_connection_response, ConnectionExchange, ConnectionExchangeResult, SharedConnectionPatch,
    SharedModelConnection, CONNECTION_SYNC_PAGE_SIZE, CONNECTION_SYNC_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::{local::LocalSettings, platform, remote};

static STORE: Mutex<()> = Mutex::const_new(());
static SYNC: Mutex<()> = Mutex::const_new(());

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub supported: bool,
    pub checked_at: String,
    pub connections: usize,
    pub received: usize,
    pub sent: usize,
    pub conflicts: usize,
    pub device_only: usize,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelStore {
    version: u16,
    #[serde(default)]
    connections: HashMap<String, SharedModelConnection>,
    active_id: Option<String>,
    #[serde(default)]
    baselines: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    ignored_ids: HashSet<String>,
    #[serde(default)]
    ignored_by_host: HashMap<String, HashSet<String>>,
    #[serde(default)]
    sync: HashMap<String, SyncSummary>,
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

async fn save(app: &AppHandle, store: &ModelStore) -> Result<(), String> {
    platform::save_secret(
        app,
        "local-provider",
        Some(&serde_json::to_value(store).map_err(message)?),
    )
    .await
}

async fn load(app: &AppHandle) -> Result<ModelStore, String> {
    let Some(saved) = platform::read_secret(app, "local-provider").await? else {
        return Ok(ModelStore {
            version: 2,
            ..Default::default()
        });
    };
    if saved["version"] == 2 {
        return serde_json::from_value(saved)
            .map_err(|_| "Saved model connections could not be read.".into());
    }
    // Upgrade the original single-provider store atomically under the same key.
    let settings: LocalSettings = serde_json::from_value(saved["settings"].clone())
        .map_err(|_| "The previous phone connection could not be read.")?;
    let record: SharedModelConnection = serde_json::from_value(json!({
        "id":uuid::Uuid::new_v4().to_string(),"name":settings.model,
        "provider":settings.provider_type,"model":settings.model,"baseUrl":settings.base_url,
        "apiKey":saved["apiKey"],"isDefault":true,
    }))
    .map_err(|_| "The previous phone connection could not be migrated.")?;
    let mut store = ModelStore {
        version: 2,
        active_id: Some(record.id.clone()),
        ..Default::default()
    };
    store.connections.insert(record.id.clone(), record);
    save(app, &store).await?;
    Ok(store)
}

pub fn settings(record: &SharedModelConnection) -> LocalSettings {
    LocalSettings {
        connection_id: Some(record.id.clone()),
        provider_type: record.provider.clone(),
        base_url: record.base_url.clone(),
        model: record.model.clone(),
    }
}

pub async fn public_state(app: &AppHandle) -> Result<Value, String> {
    let _guard = STORE.lock().await;
    let store = load(app).await?;
    let active = store
        .active_id
        .as_ref()
        .and_then(|id| store.connections.get(id));
    let mut records = store.connections.values().collect::<Vec<_>>();
    records.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(
        json!({"settings":active.map(settings),"activeConnectionId":store.active_id,
        "connections":records.iter().map(|record|json!({"id":record.id,"name":record.name,"provider":record.provider,"model":record.model,"baseUrl":record.base_url})).collect::<Vec<_>>(),
        "connectionSync":store.sync}),
    )
}

pub async fn get(app: &AppHandle, id: Option<&str>) -> Result<SharedModelConnection, String> {
    let _guard = STORE.lock().await;
    let store = load(app).await?;
    id.or(store.active_id.as_deref())
        .and_then(|id| store.connections.get(id))
        .cloned()
        .ok_or("Configure or synchronize a model connection first.".into())
}

pub async fn resolve_speech_credentials(
    app: &AppHandle,
    config: &mut nexa_core::app_settings::AppConfig,
    list_connections: bool,
) -> Result<Vec<crate::speech_connections::SpeechConnection>, String> {
    let _guard = STORE.lock().await;
    let store = load(app).await?;
    let mut sources = store.connections.values().collect::<Vec<_>>();
    sources.sort_by(|a, b| {
        (
            Some(&a.id) != store.active_id.as_ref(),
            !a.is_default,
            &a.id,
        )
            .cmp(&(
                Some(&b.id) != store.active_id.as_ref(),
                !b.is_default,
                &b.id,
            ))
    });
    Ok(crate::speech_connections::resolve(
        config,
        &sources,
        list_connections,
    ))
}

pub fn provider_config(
    saved: &SharedModelConnection,
) -> Result<nexa_core::llm::ProviderConfig, String> {
    Ok(nexa_core::llm::ProviderConfig {
        provider_type: nexa_core::provider_registry::provider_type_from_key(&saved.provider)
            .ok_or("Unsupported provider type.")?,
        base_url: saved.base_url.clone(),
        api_key: Some(saved.api_key.clone()),
        org_id: None,
        timeout_secs: None,
        streaming: saved.provider_streaming,
    })
}

pub async fn save_settings(
    app: &AppHandle,
    input: LocalSettings,
    api_key: String,
    name: Option<String>,
    create_new: bool,
) -> Result<(), String> {
    if input.model.trim().is_empty() {
        return Err("Enter a model ID.".into());
    }
    nexa_core::provider_registry::provider_type_from_key(&input.provider_type)
        .ok_or("Unsupported provider type.")?;
    if let Some(url) = &input.base_url {
        crate::validate_provider_url(url)?;
    }
    let _guard = STORE.lock().await;
    let mut store = load(app).await?;
    let selected = if create_new {
        None
    } else {
        input
            .connection_id
            .clone()
            .or_else(|| store.active_id.clone())
    };
    let previous = selected.as_ref().and_then(|id| store.connections.get(id));
    if selected.is_some() && previous.is_none() {
        return Err(
            "This connection was removed. Choose an existing connection or add one.".into(),
        );
    }
    let same_account = previous.is_some_and(|record| {
        record.provider == input.provider_type && record.base_url == input.base_url
    });
    let key = if api_key.is_empty() && same_account {
        previous
            .map(|record| record.api_key.clone())
            .unwrap_or_default()
    } else {
        api_key
    };
    if key.is_empty() {
        return Err("Enter an API key for this connection.".into());
    }
    let id = selected.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let name = name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            previous
                .map(|record| record.name.clone())
                .unwrap_or_else(|| input.model.clone())
        });
    if name.len() > 512 || key.len() > 65_536 {
        return Err("Connection name or credential is too large.".into());
    }
    let mut record = if same_account {
        previous.cloned().expect("same account exists")
    } else {
        serde_json::from_value(json!({"id":id,"name":name,"provider":input.provider_type,"apiKey":key,"baseUrl":input.base_url,"model":input.model,"isDefault":true})).map_err(|_|"Invalid phone connection.")?
    };
    record.id = id.clone();
    record.name = name;
    record.provider = input.provider_type;
    record.api_key = key;
    record.base_url = input.base_url;
    record.model = input.model;
    store.connections.insert(id.clone(), record);
    store.active_id = Some(id.clone());
    store.ignored_ids.remove(&id);
    save(app, &store).await?;
    let _ = app.emit("mobile:connections", json!({"changed":true}));
    Ok(())
}

pub async fn select(app: &AppHandle, id: &str) -> Result<(), String> {
    let _guard = STORE.lock().await;
    let mut store = load(app).await?;
    if !store.connections.contains_key(id) {
        return Err("This model connection is unavailable.".into());
    }
    store.active_id = Some(id.to_string());
    save(app, &store).await?;
    let _ = app.emit("mobile:connections", json!({"changed":false}));
    Ok(())
}

pub async fn remove(app: &AppHandle, id: &str) -> Result<(), String> {
    let _guard = STORE.lock().await;
    let mut store = load(app).await?;
    if store.connections.remove(id).is_none() {
        return Err("This model connection was already removed.".into());
    }
    store.ignored_ids.insert(id.to_string());
    if store.active_id.as_deref() == Some(id) {
        store.active_id = store.connections.keys().min().cloned();
    }
    save(app, &store).await?;
    let _ = app.emit("mobile:connections", json!({"changed":false}));
    Ok(())
}

pub async fn forget_host(app: &AppHandle, id: &str) -> Result<(), String> {
    let _guard = STORE.lock().await;
    let mut store = load(app).await?;
    store.baselines.remove(id);
    store.ignored_by_host.remove(id);
    store.sync.remove(id);
    save(app, &store).await
}

pub async fn synchronize(app: &AppHandle, host_id: &str) -> Result<SyncSummary, String> {
    let _sync = SYNC.lock().await;
    if !remote::supports_model_sync(app, host_id).await? {
        return Ok(SyncSummary {
            checked_at: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        });
    }
    let (mut outgoing, baseline, sent, device_only) = {
        let _guard = STORE.lock().await;
        let store = load(app).await?;
        let blocked = store.ignored_by_host.get(host_id);
        let mut outgoing = store
            .connections
            .values()
            .filter(|record| {
                record.is_portable() && !blocked.is_some_and(|ids| ids.contains(&record.id))
            })
            .cloned()
            .collect::<Vec<_>>();
        for record in &mut outgoing {
            record.is_default = store.active_id.as_deref() == Some(&record.id);
        }
        let sent = store
            .connections
            .values()
            .map(|record| Ok((record.id.clone(), record.revision().map_err(message)?)))
            .collect::<Result<HashMap<_, _>, String>>()?;
        let device_only = store
            .connections
            .values()
            .filter(|record| !record.is_portable())
            .count();
        (
            outgoing,
            store.baselines.get(host_id).cloned().unwrap_or_default(),
            sent,
            device_only,
        )
    };
    outgoing.sort_by(|a, b| b.is_default.cmp(&a.is_default).then(a.id.cmp(&b.id)));
    let mut combined = ConnectionExchangeResult {
        version: CONNECTION_SYNC_VERSION,
        connections: vec![],
        next_after: None,
        created: 0,
        updated: 0,
        forks: vec![],
        ignored_ids: vec![],
        device_only_count: 0,
    };
    let mut records = HashMap::new();
    let mut after = None;
    let chunks = outgoing
        .chunks(CONNECTION_SYNC_PAGE_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    let chunks = if chunks.is_empty() {
        vec![vec![]]
    } else {
        chunks
    };
    for chunk in chunks {
        let input = ConnectionExchange {
            version: CONNECTION_SYNC_VERSION,
            connections: chunk
                .into_iter()
                .map(|connection| SharedConnectionPatch {
                    base_revision: baseline.get(&connection.id).cloned(),
                    connection,
                })
                .collect(),
            after: None,
        };
        let result: ConnectionExchangeResult = serde_json::from_value(
            remote::rpc(
                app,
                host_id,
                "sync.connections".into(),
                Some(json!({"input":input})),
            )
            .await?,
        )
        .map_err(|_| "The computer returned an invalid connection sync response.")?;
        if result.version != CONNECTION_SYNC_VERSION {
            return Err("Update both devices to use model connection sync.".into());
        }
        after = result.next_after.clone();
        combined.created += result.created;
        combined.updated += result.updated;
        combined.forks.extend(result.forks);
        combined.ignored_ids.extend(result.ignored_ids);
        combined.device_only_count = result.device_only_count;
        for record in result.connections {
            record.validate().map_err(message)?;
            records.insert(record.id.clone(), record);
        }
    }
    let mut cursors = HashSet::new();
    while let Some(cursor) = after {
        if !cursors.insert(cursor.clone()) {
            return Err("The connection sync cursor did not advance.".into());
        }
        let input = ConnectionExchange {
            version: CONNECTION_SYNC_VERSION,
            connections: vec![],
            after: Some(cursor),
        };
        let result: ConnectionExchangeResult = serde_json::from_value(
            remote::rpc(
                app,
                host_id,
                "sync.connections".into(),
                Some(json!({"input":input})),
            )
            .await?,
        )
        .map_err(|_| "The computer returned an invalid connection sync response.")?;
        if result.version != CONNECTION_SYNC_VERSION {
            return Err("Update both devices to use model connection sync.".into());
        }
        after = result.next_after;
        for record in result.connections {
            record.validate().map_err(message)?;
            records.insert(record.id.clone(), record);
        }
    }
    combined.connections = records.into_values().collect();
    let _guard = STORE.lock().await;
    let mut store = load(app).await?;
    // Forgetting a host while its request was in flight must not retain its new credentials.
    let _paired = remote::paired_guard(app, host_id).await?;
    let before = store.connections.len();
    let active_copy = store
        .active_id
        .as_ref()
        .and_then(|active| combined.forks.iter().find(|fork| &fork.source_id == active))
        .filter(|fork| {
            store
                .connections
                .get(&fork.source_id)
                .and_then(|record| record.revision().ok())
                .as_ref()
                == sent.get(&fork.source_id)
        })
        .map(|fork| fork.copy_id.clone());
    let mut baseline = store.baselines.remove(host_id).unwrap_or_default();
    merge_connection_response(
        &mut store.connections,
        &sent,
        &mut baseline,
        &store.ignored_ids,
        &combined,
    )
    .map_err(message)?;
    store.baselines.insert(host_id.into(), baseline);
    store
        .ignored_by_host
        .entry(host_id.into())
        .or_default()
        .extend(combined.ignored_ids.iter().cloned());
    if let Some(copy) = active_copy.filter(|copy| store.connections.contains_key(copy)) {
        store.active_id = Some(copy);
    }
    if store.active_id.is_none() {
        store.active_id = combined
            .connections
            .iter()
            .find(|record| record.is_default && store.connections.contains_key(&record.id))
            .or_else(|| {
                combined
                    .connections
                    .iter()
                    .find(|record| store.connections.contains_key(&record.id))
            })
            .map(|record| record.id.clone());
    }
    let summary = SyncSummary {
        supported: true,
        checked_at: chrono::Utc::now().to_rfc3339(),
        connections: combined.connections.len(),
        received: store.connections.len().saturating_sub(before),
        sent: combined.created + combined.updated,
        conflicts: combined.forks.len(),
        device_only: device_only + combined.device_only_count,
    };
    store.sync.insert(host_id.into(), summary.clone());
    save(app, &store).await?;
    let _ = app.emit("mobile:connections", json!({"changed":false}));
    Ok(summary)
}
