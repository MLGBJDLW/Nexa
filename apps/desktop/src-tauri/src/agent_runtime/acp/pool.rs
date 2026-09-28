//! Only completed sessions enter this bounded idle cache. An active turn owns
//! its connection exclusively; dropping a failed/cancelled turn kills its tree.
use super::{catalog::Session, transport::Wire};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const IDLE_TTL: Duration = Duration::from_secs(300);
const MAX_IDLE: usize = 4;

pub(super) struct Connected {
    pub wire: Wire,
    pub session: Session,
    pub history: String,
    pub context: String,
}
struct Idle {
    connection: Connected,
    since: Instant,
}
fn cache() -> &'static Mutex<HashMap<String, Idle>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Idle>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}
pub(super) fn take(key: &str, history: &str) -> Option<Connected> {
    let mut entries = cache().lock().unwrap_or_else(|e| e.into_inner());
    entries.retain(|_, idle| idle.since.elapsed() < IDLE_TTL);
    let mut idle = entries.remove(key)?;
    (idle.connection.history == history && idle.connection.wire.is_alive())
        .then_some(idle.connection)
}
pub(super) fn put(key: String, connection: Connected) {
    let since = Instant::now();
    {
        let mut entries = cache().lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, idle| idle.since.elapsed() < IDLE_TTL);
        if entries.len() >= MAX_IDLE {
            let oldest = entries
                .iter()
                .min_by_key(|(_, idle)| idle.since)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert(key.clone(), Idle { connection, since });
    }
    tokio::spawn(async move {
        tokio::time::sleep(IDLE_TTL).await;
        let mut entries = cache().lock().unwrap_or_else(|e| e.into_inner());
        if entries.get(&key).is_some_and(|idle| idle.since == since) {
            entries.remove(&key);
        }
    });
}
pub(crate) fn shutdown() {
    cache().lock().unwrap_or_else(|e| e.into_inner()).clear();
}
