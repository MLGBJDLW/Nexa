//! Connector-scoped MCP lifecycle and immutable catalog snapshots.
//!
//! The manager lock only publishes desired slots. Each slot owns its own I/O
//! admission, RPC client, catalog revision and cancellation epoch. A registry
//! is a read projection and never performs network discovery.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use super::client::McpClient;
use super::events::McpClientEvents;
use super::result::McpCallOutcome;
use super::{
    expand_managed_arg, find_free_port, parse_mcp_args, resolve_mcp_config_map,
    runtime_config_changed, McpServer, McpToolIdentity, McpToolInfo,
};
use crate::db::Database;
use crate::error::CoreError;
use crate::tools::mcp_tool::McpTool;
use crate::tools::ToolRegistry;

const MAX_PARALLEL_DISCOVERY: usize = 4;
const CATALOG_TTL: Duration = Duration::from_secs(60);
const CATALOG_NOTIFICATION_DEBOUNCE: Duration = Duration::from_millis(25);
const RECOVERY_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCatalogSnapshot {
    pub connector_id: String,
    pub connection_epoch: u64,
    pub catalog_revision: u64,
    pub complete: bool,
    pub tools: Vec<McpToolInfo>,
    pub diagnostics: Option<String>,
}

#[derive(Clone)]
struct DesiredConnector {
    server: McpServer,
    timeout_secs: Option<u64>,
    database: Option<Database>,
    authority_epoch: u64,
    cancelled: CancellationToken,
}

impl DesiredConnector {
    fn remains_authorized(&self) -> bool {
        if self.cancelled.is_cancelled() {
            return false;
        }
        self.database.as_ref().is_none_or(|db| {
            db.get_mcp_server(&self.server.id).is_ok_and(|current| {
                current.enabled && !runtime_config_changed(&current, &self.server)
            })
        })
    }
}

struct McpConnection {
    epoch: u64,
    client: Arc<Mutex<McpClient>>,
    events: Arc<McpClientEvents>,
    healthy: AtomicBool,
    managed_process: StdMutex<Option<Child>>,
}

impl McpConnection {
    async fn shutdown(&self) {
        let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
            self.client.lock().await.shutdown().await
        })
        .await;
        let child = self
            .managed_process
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(mut child) = child {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, child.wait()).await;
        }
    }
}

#[derive(Default)]
struct SlotState {
    desired: Option<DesiredConnector>,
    connection: Option<Arc<McpConnection>>,
    catalog: Option<McpCatalogSnapshot>,
    observed_notification_revision: u64,
    refreshed_at: Option<Instant>,
    diagnostics: Option<String>,
}

pub(crate) struct McpConnectorSlot {
    id: String,
    state: StdMutex<SlotState>,
    operation: Mutex<()>,
    refresh_scheduled: AtomicBool,
    registry_revision: Arc<AtomicU64>,
    next_connection_epoch: Arc<AtomicU64>,
    discovery_admission: Arc<Semaphore>,
}

impl McpConnectorSlot {
    fn new(id: String, manager: &McpManager) -> Self {
        Self {
            id,
            state: StdMutex::new(SlotState::default()),
            operation: Mutex::new(()),
            refresh_scheduled: AtomicBool::new(false),
            registry_revision: Arc::clone(&manager.inner.registry_revision),
            next_connection_epoch: Arc::clone(&manager.inner.next_epoch),
            discovery_admission: Arc::clone(&manager.inner.discovery_admission),
        }
    }

    fn configure(
        &self,
        server: Option<&McpServer>,
        timeout_secs: Option<u64>,
        database: Option<&Database>,
    ) -> Option<Arc<McpConnection>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let (Some(desired), Some(server)) = (&mut state.desired, server) {
            if !runtime_config_changed(&desired.server, server)
                && desired.timeout_secs == timeout_secs
            {
                if database.is_some() {
                    desired.database = database.cloned();
                }
                return None;
            }
        } else if state.desired.is_none() && server.is_none() {
            return None;
        }

        if let Some(previous) = state.desired.take() {
            previous.cancelled.cancel();
        }
        let previous_connection = state.connection.take();
        state.catalog = None;
        state.refreshed_at = None;
        state.observed_notification_revision = 0;
        state.diagnostics = None;
        state.desired = server.map(|server| DesiredConnector {
            server: server.clone(),
            timeout_secs,
            database: database.cloned(),
            authority_epoch: self.next_connection_epoch.fetch_add(1, Ordering::AcqRel) + 1,
            cancelled: CancellationToken::new(),
        });
        self.registry_revision.fetch_add(1, Ordering::AcqRel);
        previous_connection
    }

    fn desired(&self) -> Option<DesiredConnector> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .desired
            .clone()
    }

    fn snapshot(&self) -> Option<McpCatalogSnapshot> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.desired.as_ref()?;
        let mut snapshot = state.catalog.clone().unwrap_or_else(|| McpCatalogSnapshot {
            connector_id: self.id.clone(),
            connection_epoch: 0,
            catalog_revision: 0,
            complete: false,
            tools: Vec::new(),
            diagnostics: None,
        });
        snapshot.diagnostics = state.diagnostics.clone();
        Some(snapshot)
    }

    fn is_fresh(state: &SlotState) -> bool {
        state
            .catalog
            .as_ref()
            .is_some_and(|catalog| catalog.complete)
            && state.connection.as_ref().is_some_and(|connection| {
                connection.healthy.load(Ordering::Acquire)
                    && connection.events.catalog_revision() == state.observed_notification_revision
            })
            && state
                .refreshed_at
                .is_some_and(|at| at.elapsed() < CATALOG_TTL)
    }

    fn invalidate_catalog(self: &Arc<Self>, authority_epoch: u64, connection_epoch: u64) {
        let changed = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state
                .desired
                .as_ref()
                .is_none_or(|desired| desired.authority_epoch != authority_epoch)
                || state
                    .connection
                    .as_ref()
                    .is_some_and(|connection| connection.epoch != connection_epoch)
            {
                return;
            }
            let changed = state
                .catalog
                .as_ref()
                .is_some_and(|catalog| catalog.complete);
            if let Some(catalog) = state.catalog.as_mut() {
                catalog.complete = false;
            }
            changed
        };
        if changed {
            self.registry_revision.fetch_add(1, Ordering::AcqRel);
        }
        self.queue_refresh();
    }

    fn expire_catalog(self: &Arc<Self>) {
        let expired = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state
                .desired
                .as_ref()
                .zip(state.connection.as_ref())
                .and_then(|(desired, connection)| {
                    state
                        .refreshed_at
                        .filter(|at| at.elapsed() >= CATALOG_TTL)
                        .map(|_| (desired.authority_epoch, connection.epoch))
                })
        };
        if expired.is_some() {
            self.queue_refresh();
        }
    }

    fn queue_refresh(self: &Arc<Self>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self
            .refresh_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let slot = Arc::clone(self);
        runtime.spawn(async move {
            tokio::time::sleep(CATALOG_NOTIFICATION_DEBOUNCE).await;
            let succeeded = slot.refresh(false, true).await.is_ok();
            slot.refresh_scheduled.store(false, Ordering::Release);
            // A notification may arrive while a paginated list is in flight.
            // Only a successful refresh may schedule that newer revision. A
            // failed server is retried by a new call/configuration/notification,
            // never by an unbounded reconnect loop.
            let changed_during_refresh = succeeded && {
                let state = slot.state.lock().unwrap_or_else(|error| error.into_inner());
                state.desired.is_some() && !Self::is_fresh(&state)
            };
            if changed_during_refresh {
                slot.queue_refresh();
            }
        });
    }

    fn bind_events(
        self: &Arc<Self>,
        events: &McpClientEvents,
        desired: &DesiredConnector,
        connection_epoch: u64,
    ) {
        let slot = Arc::downgrade(self);
        let authority_epoch = desired.authority_epoch;
        events.set_observer(Arc::new(move || {
            if let Some(slot) = slot.upgrade() {
                slot.invalidate_catalog(authority_epoch, connection_epoch);
            }
        }));
    }

    async fn refresh(
        self: &Arc<Self>,
        force: bool,
        wait: bool,
    ) -> Result<McpCatalogSnapshot, CoreError> {
        let desired = self
            .desired()
            .ok_or_else(|| CoreError::Mcp(format!("MCP connector {} is disabled", self.id)))?;
        if !desired.remains_authorized() {
            self.revoke();
            return Err(CoreError::Mcp(format!(
                "MCP connector {} configuration is no longer authorized",
                self.id
            )));
        }
        if !force {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if Self::is_fresh(&state) {
                return Ok(state.catalog.clone().expect("fresh catalog"));
            }
        }
        let _operation = if wait {
            tokio::select! {
                biased;
                _ = desired.cancelled.cancelled() => return Err(CoreError::Mcp("MCP connector changed while waiting for discovery".into())),
                guard = self.operation.lock() => guard,
            }
        } else {
            self.operation.try_lock().map_err(|_| {
                CoreError::Mcp(format!(
                    "MCP connector {} catalog refresh is in progress",
                    self.id
                ))
            })?
        };
        let existing = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state
                .desired
                .as_ref()
                .is_none_or(|current| current.authority_epoch != desired.authority_epoch)
            {
                return Err(CoreError::Mcp(
                    "MCP discovery was superseded by configuration change".into(),
                ));
            }
            if !force && Self::is_fresh(&state) {
                return Ok(state.catalog.clone().expect("fresh catalog"));
            }
            state
                .connection
                .as_ref()
                .filter(|connection| connection.healthy.load(Ordering::Acquire))
                .cloned()
        };
        let discovery = async {
            let _admission = self
                .discovery_admission
                .acquire()
                .await
                .map_err(|_| CoreError::Mcp("MCP discovery admission is closed".into()))?;
            if let Some(connection) = existing {
                let observed = connection.events.catalog_revision();
                let tools = connection.client.lock().await.list_tools().await?;
                Ok((connection, tools, observed, false))
            } else {
                let epoch = self.next_connection_epoch.fetch_add(1, Ordering::AcqRel) + 1;
                let (mut client, process) =
                    connect_client(&desired.server, desired.timeout_secs).await?;
                let events = client.events();
                self.bind_events(&events, &desired, epoch);
                let observed = events.catalog_revision();
                let tools = client.list_tools().await?;
                Ok((
                    Arc::new(McpConnection {
                        epoch,
                        client: Arc::new(Mutex::new(client)),
                        events,
                        healthy: AtomicBool::new(true),
                        managed_process: StdMutex::new(process),
                    }),
                    tools,
                    observed,
                    true,
                ))
            }
        };
        let result = tokio::select! {
            biased;
            _ = desired.cancelled.cancelled() => return Err(CoreError::Mcp("MCP discovery was cancelled by configuration change".into())),
            result = tokio::time::timeout(Duration::from_secs(desired.timeout_secs.unwrap_or(60).max(1)), discovery) => {
                result.unwrap_or_else(|_| Err(CoreError::McpTransport("MCP connector discovery deadline expired".into())))
            }
        };
        let (connection, mut tools, observed, created) = match result {
            Ok(result) => result,
            Err(error) => {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if state
                    .desired
                    .as_ref()
                    .is_some_and(|current| current.authority_epoch == desired.authority_epoch)
                {
                    if let Some(catalog) = state.catalog.as_mut() {
                        catalog.complete = false;
                    }
                    if matches!(error, CoreError::McpTransport(_)) {
                        if let Some(connection) = &state.connection {
                            connection.healthy.store(false, Ordering::Release);
                        }
                    }
                    state.diagnostics = Some(error.to_string());
                    self.registry_revision.fetch_add(1, Ordering::AcqRel);
                }
                return Err(error);
            }
        };
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        if !desired.remains_authorized() {
            self.revoke();
            if created {
                connection.shutdown().await;
            }
            return Err(CoreError::Mcp(
                "MCP discovery finished after its authorization expired".into(),
            ));
        }
        let committed = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state
                .desired
                .as_ref()
                .is_none_or(|current| current.authority_epoch != desired.authority_epoch)
            {
                None
            } else {
                let changed = state.catalog.as_ref().is_none_or(|catalog| {
                    catalog.tools != tools || catalog.connection_epoch != connection.epoch
                });
                let revision = state
                    .catalog
                    .as_ref()
                    .map(|catalog| catalog.catalog_revision)
                    .unwrap_or(0)
                    + u64::from(changed);
                let snapshot = McpCatalogSnapshot {
                    connector_id: self.id.clone(),
                    connection_epoch: connection.epoch,
                    catalog_revision: revision,
                    complete: connection.events.catalog_revision() == observed,
                    tools,
                    diagnostics: None,
                };
                let previous = state.connection.replace(Arc::clone(&connection));
                state.catalog = Some(snapshot.clone());
                state.observed_notification_revision = observed;
                state.refreshed_at = Some(Instant::now());
                state.diagnostics = None;
                self.registry_revision.fetch_add(1, Ordering::AcqRel);
                Some((
                    snapshot,
                    previous.filter(|old| !Arc::ptr_eq(old, &connection)),
                ))
            }
        };
        let Some((snapshot, previous)) = committed else {
            if created {
                connection.shutdown().await;
            }
            return Err(CoreError::Mcp(
                "MCP catalog commit was superseded by configuration change".into(),
            ));
        };
        if let Some(previous) = previous {
            dispose_connection(previous);
        }
        if !snapshot.complete {
            self.queue_refresh();
        }
        Ok(snapshot)
    }

    fn revoke(&self) {
        if let Some(connection) = self.configure(None, None, None) {
            dispose_connection(connection);
        }
    }

    fn call_connection(
        &self,
        identity: &McpToolIdentity,
        authority_epoch: u64,
        expected: &McpToolInfo,
    ) -> Result<(Arc<McpConnection>, DesiredConnector), CoreError> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let desired = state.desired.as_ref().filter(|desired| desired.authority_epoch == authority_epoch)
            .ok_or_else(|| CoreError::Mcp("MCP tool belongs to a disabled or replaced connector; refresh the tool catalog".into()))?;
        if McpToolIdentity::new(&desired.server, &expected.name) != *identity {
            return Err(CoreError::Mcp(
                "MCP tool authorization no longer matches its connector configuration".into(),
            ));
        }
        let catalog = state
            .catalog
            .as_ref()
            .filter(|catalog| catalog.complete)
            .ok_or_else(|| CoreError::Mcp("MCP tool catalog is incomplete or refreshing".into()))?;
        if !catalog.tools.iter().any(|tool| tool == expected) {
            return Err(CoreError::Mcp("MCP tool was removed or its definition changed; begin a new model turn with the refreshed catalog".into()));
        }
        let connection = state
            .connection
            .as_ref()
            .filter(|connection| connection.healthy.load(Ordering::Acquire))
            .ok_or_else(|| CoreError::Mcp("MCP connector is recovering".into()))?;
        Ok((Arc::clone(connection), desired.clone()))
    }

    pub(crate) async fn call(
        self: &Arc<Self>,
        identity: &McpToolIdentity,
        authority_epoch: u64,
        expected: &McpToolInfo,
        arguments: Value,
        cancelled: Option<&CancellationToken>,
    ) -> Result<McpCallOutcome, CoreError> {
        let desired = self
            .desired()
            .filter(|desired| desired.authority_epoch == authority_epoch)
            .ok_or_else(|| {
                CoreError::Mcp("MCP tool belongs to a disabled or replaced connector".into())
            })?;
        let caller_cancelled = cancelled.cloned().unwrap_or_default();
        // Reject stale authority before discovery and honor cancellation while
        // waiting. Refresh never replays the effectful tools/call request.
        tokio::select! {
            biased;
            _ = desired.cancelled.cancelled() => return Err(CoreError::Mcp("MCP connector changed before execution".into())),
            _ = caller_cancelled.cancelled() => return Err(CoreError::Mcp("MCP tool cancelled before execution".into())),
            result = tokio::time::timeout(RECOVERY_WAIT_TIMEOUT, self.refresh(false, true)) => {
                result.map_err(|_| CoreError::Mcp("MCP connection recovery is still in progress".into()))??;
            }
        }
        let (connection, desired) = self.call_connection(identity, authority_epoch, expected)?;
        if !desired.remains_authorized() {
            self.revoke();
            return Err(CoreError::Mcp(
                "MCP connector was disabled before execution".into(),
            ));
        }
        let mut client = tokio::select! {
            biased;
            _ = desired.cancelled.cancelled() => return Err(CoreError::Mcp("MCP connector was disabled before execution".into())),
            _ = caller_cancelled.cancelled() => return Err(CoreError::Mcp("MCP tool cancelled before execution".into())),
            guard = connection.client.lock() => guard,
        };
        let (current, current_desired) =
            self.call_connection(identity, authority_epoch, expected)?;
        if !Arc::ptr_eq(&current, &connection) || !current_desired.remains_authorized() {
            return Err(CoreError::Mcp(
                "MCP connector changed before execution; no call was sent".into(),
            ));
        }
        let mut result = tokio::select! {
            biased;
            _ = desired.cancelled.cancelled() => Err(CoreError::McpTransport("MCP connector disabled during execution; effects may have occurred and the call was not replayed".into())),
            _ = caller_cancelled.cancelled() => Err(CoreError::McpTransport("MCP call cancelled; effects may have occurred and the call was not replayed".into())),
            result = client.call_tool(&identity.id.tool_name, arguments) => result,
        };
        drop(client);
        if matches!(result, Err(CoreError::McpTransport(_))) {
            let recover = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if state
                    .connection
                    .as_ref()
                    .is_some_and(|active| Arc::ptr_eq(active, &connection))
                {
                    connection.healthy.store(false, Ordering::Release);
                    if let Some(catalog) = state.catalog.as_mut() {
                        catalog.complete = false;
                    }
                    state.diagnostics =
                        Some("Connection failed; the effectful call was not replayed".into());
                    self.registry_revision.fetch_add(1, Ordering::AcqRel);
                    state.desired.is_some()
                } else {
                    false
                }
            };
            if recover && !caller_cancelled.is_cancelled() {
                self.queue_refresh();
                if let Err(CoreError::McpTransport(error)) = &mut result {
                    error.push_str(
                        "; recovery scheduled for subsequent calls; this call was not replayed",
                    );
                }
            }
        }
        result
    }
}

fn dispose_connection(connection: Arc<McpConnection>) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            connection.shutdown().await;
        });
    }
    // Without a live runtime, dropping the final client still aborts reader
    // tasks and kills owned processes through their Drop/kill_on_drop contracts.
}

struct McpManagerInner {
    slots: StdMutex<BTreeMap<String, Arc<McpConnectorSlot>>>,
    registry_revision: Arc<AtomicU64>,
    next_epoch: Arc<AtomicU64>,
    discovery_admission: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct McpManager {
    inner: Arc<McpManagerInner>,
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(McpManagerInner {
                slots: StdMutex::new(BTreeMap::new()),
                registry_revision: Arc::new(AtomicU64::new(0)),
                next_epoch: Arc::new(AtomicU64::new(0)),
                discovery_admission: Arc::new(Semaphore::new(MAX_PARALLEL_DISCOVERY)),
            }),
        }
    }

    fn slots(&self) -> Vec<Arc<McpConnectorSlot>> {
        self.inner
            .slots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect()
    }

    fn slot(&self, id: &str) -> Option<Arc<McpConnectorSlot>> {
        self.inner
            .slots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .cloned()
    }

    fn configured_slot(
        &self,
        server: &McpServer,
        timeout_secs: Option<u64>,
        database: Option<&Database>,
    ) -> Arc<McpConnectorSlot> {
        let slot = {
            let mut slots = self
                .inner
                .slots
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            Arc::clone(
                slots
                    .entry(server.id.clone())
                    .or_insert_with(|| Arc::new(McpConnectorSlot::new(server.id.clone(), self))),
            )
        };
        if let Some(previous) =
            slot.configure(server.enabled.then_some(server), timeout_secs, database)
        {
            dispose_connection(previous);
        }
        slot
    }

    pub fn registry_generation(&self) -> u64 {
        for slot in self.slots() {
            slot.expire_catalog();
        }
        self.inner.registry_revision.load(Ordering::Acquire)
    }

    pub fn catalog_snapshot(&self, connector_id: &str) -> Option<McpCatalogSnapshot> {
        self.slot(connector_id)?.snapshot()
    }

    pub async fn connect_server(
        &self,
        server: &McpServer,
        timeout_secs: Option<u64>,
    ) -> Result<Vec<McpToolInfo>, CoreError> {
        self.configured_slot(server, timeout_secs, None)
            .refresh(true, true)
            .await
            .map(|snapshot| snapshot.tools)
    }

    pub async fn probe_server(
        server: &McpServer,
        timeout_secs: Option<u64>,
    ) -> Result<Vec<McpToolInfo>, CoreError> {
        let manager = Self::new();
        let mut probe = server.clone();
        probe.enabled = true;
        let result = manager.connect_server(&probe, timeout_secs).await;
        manager.shutdown().await;
        result
    }

    pub async fn refresh_server(&self, connector_id: &str) -> Result<Vec<McpToolInfo>, CoreError> {
        let slot = self
            .slot(connector_id)
            .ok_or_else(|| CoreError::NotFound(format!("MCP connector {connector_id}")))?;
        slot.refresh(true, true)
            .await
            .map(|snapshot| snapshot.tools)
    }

    pub async fn sync_servers(
        &self,
        servers: &[McpServer],
        timeout_secs: Option<u64>,
    ) -> HashMap<String, String> {
        self.sync_servers_inner(servers, timeout_secs, None).await
    }

    pub async fn sync_from_database(
        &self,
        db: &Database,
        timeout_secs: Option<u64>,
    ) -> Result<HashMap<String, String>, CoreError> {
        // Read immediately before publishing desired authority. Refresh commits
        // and calls additionally check this same database after awaiting I/O.
        let servers = db.get_enabled_mcp_servers()?;
        Ok(self
            .sync_servers_inner(&servers, timeout_secs, Some(db))
            .await)
    }

    pub async fn sync_server_from_database(
        &self,
        db: &Database,
        connector_id: &str,
        timeout_secs: Option<u64>,
    ) -> Result<Vec<McpToolInfo>, CoreError> {
        let server = db.get_mcp_server(connector_id)?;
        self.configured_slot(&server, timeout_secs, Some(db))
            .refresh(false, true)
            .await
            .map(|snapshot| snapshot.tools)
    }

    async fn sync_servers_inner(
        &self,
        servers: &[McpServer],
        timeout_secs: Option<u64>,
        database: Option<&Database>,
    ) -> HashMap<String, String> {
        let desired = servers
            .iter()
            .filter(|server| server.enabled)
            .map(|server| (server.id.as_str(), server))
            .collect::<BTreeMap<_, _>>();
        for slot in self.slots() {
            if !desired.contains_key(slot.id.as_str()) {
                slot.revoke();
            }
        }
        let slots = desired
            .values()
            .map(|server| self.configured_slot(server, timeout_secs, database))
            .collect::<Vec<_>>();
        stream::iter(slots)
            .map(|slot| async move {
                let id = slot.id.clone();
                slot.refresh(false, false)
                    .await
                    .err()
                    .map(|error| (id, error.to_string()))
            })
            .buffer_unordered(MAX_PARALLEL_DISCOVERY)
            .filter_map(|error| async move { error })
            .collect()
            .await
    }

    pub async fn disconnect_server(&self, connector_id: &str) -> Result<(), CoreError> {
        if let Some(slot) = self.slot(connector_id) {
            if let Some(connection) = slot.configure(None, None, None) {
                connection.shutdown().await;
            }
        }
        Ok(())
    }

    pub async fn disconnect_all(&self) {
        let connections = self
            .slots()
            .into_iter()
            .filter_map(|slot| slot.configure(None, None, None))
            .collect::<Vec<_>>();
        stream::iter(connections)
            .map(|connection| async move {
                connection.shutdown().await;
            })
            .buffer_unordered(MAX_PARALLEL_DISCOVERY)
            .collect::<Vec<_>>()
            .await;
    }

    pub async fn shutdown(&self) {
        self.disconnect_all().await;
    }

    pub fn register_tools(&self, registry: &mut ToolRegistry) -> Result<(), CoreError> {
        let mut errors = Vec::new();
        for slot in self.slots() {
            let Some(desired) = slot.desired() else {
                continue;
            };
            if !desired.remains_authorized() {
                slot.revoke();
                continue;
            }
            slot.expire_catalog();
            let Some(catalog) = slot.snapshot() else {
                continue;
            };
            if !catalog.complete {
                errors.push(format!(
                    "MCP connector {}: {}",
                    slot.id,
                    catalog
                        .diagnostics
                        .as_deref()
                        .unwrap_or("catalog refresh is incomplete")
                ));
                continue;
            }
            for tool in catalog.tools {
                let identity = McpToolIdentity::new(&desired.server, &tool.name);
                let tool = McpTool::new(
                    tool,
                    Arc::clone(&slot),
                    identity,
                    desired.authority_epoch,
                    &desired.server.name,
                );
                if let Err(error) = registry.try_register(Box::new(tool)) {
                    errors.push(error.to_string());
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(CoreError::Mcp(errors.join("; ")))
        }
    }
}

async fn connect_client(
    server: &McpServer,
    timeout_secs: Option<u64>,
) -> Result<(McpClient, Option<Child>), CoreError> {
    let mut managed_process = None;
    let mut managed_url = None;
    if server.builtin_id.is_some() && server.command.is_some() && server.transport != "stdio" {
        let (child, port) = start_managed_process(server).await?;
        managed_process = Some(child);
        managed_url = Some(format!(
            "http://127.0.0.1:{port}/{}",
            if server.transport == "sse" {
                "sse"
            } else {
                "mcp"
            }
        ));
    }
    let mut client = match server.transport.as_str() {
        "stdio" => {
            let command = server.command.as_deref().ok_or_else(|| {
                CoreError::InvalidInput("stdio transport requires a command".into())
            })?;
            let args = server
                .args
                .as_deref()
                .map(parse_mcp_args)
                .transpose()?
                .unwrap_or_default();
            let environment = server
                .env_json
                .as_deref()
                .map(|raw| resolve_mcp_config_map("envJson", raw))
                .transpose()?;
            McpClient::connect_stdio(command, &args, environment.as_ref(), &server.name).await?
        }
        "sse" | "streamable_http" => {
            let url = managed_url
                .as_deref()
                .or(server.url.as_deref())
                .ok_or_else(|| {
                    CoreError::InvalidInput("Remote MCP transport requires a URL".into())
                })?;
            let headers = server
                .headers_json
                .as_deref()
                .map(|raw| resolve_mcp_config_map("headersJson", raw))
                .transpose()?;
            if server.transport == "sse" {
                McpClient::connect_sse(url, headers.as_ref(), &server.name).await?
            } else {
                McpClient::connect_streamable_http(url, headers.as_ref(), &server.name).await?
            }
        }
        other => {
            return Err(CoreError::InvalidInput(format!(
                "Unsupported MCP transport {other}"
            )))
        }
    };
    if let Some(timeout) = timeout_secs {
        client.set_call_timeout(Duration::from_secs(timeout));
    }
    Ok((client, managed_process))
}

async fn start_managed_process(server: &McpServer) -> Result<(Child, u16), CoreError> {
    let command = server
        .command
        .as_deref()
        .ok_or_else(|| CoreError::Mcp("Built-in connector missing command".into()))?;
    let port = find_free_port()?;
    let arguments = server
        .args
        .as_deref()
        .map(parse_mcp_args)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(|argument| expand_managed_arg(&argument, port))
        .collect::<Vec<_>>();
    let mut environment = server
        .env_json
        .as_deref()
        .map(|raw| resolve_mcp_config_map("envJson", raw))
        .transpose()?
        .unwrap_or_default();
    environment.insert("PORT".into(), port.to_string());
    #[cfg(windows)]
    let command = if ["npx", "npm", "yarn", "pnpm", "bunx"]
        .contains(&command.to_ascii_lowercase().as_str())
    {
        format!("{command}.cmd")
    } else {
        command.into()
    };
    let mut process = Command::new(command);
    process
        .args(arguments)
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::background_process::configure_tokio_background(&mut process);
    let mut child = process.spawn().map_err(|error| {
        CoreError::Mcp(format!(
            "Failed to launch managed MCP connector {}: {error}",
            server.name
        ))
    })?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok((child, port));
        }
        if let Some(status) = child.try_wait()? {
            return Err(CoreError::Mcp(format!(
                "Managed MCP connector exited with {status}"
            )));
        }
        if Instant::now() >= deadline {
            return Err(CoreError::Mcp(
                "Managed MCP connector startup timed out".into(),
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ttl_refreshes_silent_catalog_changes_before_stale_tool_execution() {
        let remote = super::super::integration_tests::peer("ttl", "TTL", &["old"]).await;
        let manager = McpManager::new();
        manager
            .connect_server(&remote.server, Some(3))
            .await
            .unwrap();
        let mut registry = ToolRegistry::new();
        manager.register_tools(&mut registry).unwrap();
        let before = manager.catalog_snapshot(&remote.server.id).unwrap();
        *remote.tools.write().unwrap() =
            vec![serde_json::json!({"name":"new","inputSchema":{"type":"object"}})];
        let slot = manager.slot(&remote.server.id).unwrap();
        slot.state.lock().unwrap().refreshed_at =
            Some(Instant::now() - CATALOG_TTL - Duration::from_secs(1));
        let db = Database::open_memory().unwrap();
        let tool = registry
            .get(&super::super::CanonicalToolId::new(&remote.server.id, "old").model_alias())
            .unwrap();
        let result = tool
            .execute(crate::tools::ToolExecutionContext::new(
                "ttl-old",
                "{}",
                &db,
                &[],
            ))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("removed or its definition changed"),
            "{}",
            result.content
        );
        let after = manager.catalog_snapshot(&remote.server.id).unwrap();
        assert!(after.complete);
        assert!(after.catalog_revision > before.catalog_revision);
        assert_eq!(after.connection_epoch, before.connection_epoch);
        assert_eq!(after.tools[0].name, "new");
        manager.shutdown().await;
    }
}
