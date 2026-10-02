use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

type CatalogObserver = Arc<dyn Fn() + Send + Sync>;

/// Shared by the transport reader and connector slot. Notifications invalidate
/// catalogs even while no caller owns the RPC client mutex.
#[derive(Default)]
pub(crate) struct McpClientEvents {
    catalog_revision: AtomicU64,
    observer: Mutex<Option<CatalogObserver>>,
}

impl McpClientEvents {
    pub(crate) fn catalog_revision(&self) -> u64 {
        self.catalog_revision.load(Ordering::Acquire)
    }

    pub(crate) fn set_observer(&self, observer: CatalogObserver) {
        *self
            .observer
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(observer);
    }

    pub(crate) fn observe(&self, message: &Value) -> bool {
        if message.get("id").is_none()
            && message.get("method").and_then(Value::as_str)
                == Some("notifications/tools/list_changed")
        {
            self.catalog_revision.fetch_add(1, Ordering::AcqRel);
            let observer = self
                .observer
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            if let Some(observer) = observer {
                observer();
            }
            true
        } else {
            false
        }
    }
}
