use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

type CatalogObserver = Arc<dyn Fn() + Send + Sync>;

/// Shared by the transport reader and connector slot. Notifications invalidate
/// catalogs even while no caller owns the RPC client mutex. Consumed id-less
/// notifications never fill the bounded queue reserved for request/response RPC.
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
        // Every id-bearing message must reach RPC handling, including server
        // requests and malformed null-id responses. Only id-less methods are
        // JSON-RPC notifications. The client currently has no progress/logging
        // consumer; its RPC handler already ignores all such unknown methods.
        if message.get("id").is_some() {
            return false;
        }
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return false;
        };
        if matches!(
            method,
            "notifications/tools/list_changed"
                | "notifications/resources/list_changed"
                | "notifications/prompts/list_changed"
                | "notifications/resources/updated"
        ) {
            self.catalog_revision.fetch_add(1, Ordering::AcqRel);
            let observer = self
                .observer
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            if let Some(observer) = observer {
                observer();
            }
        }
        // In particular, log/progress bursts must not block the idle reader
        // before it reaches a later list_changed notification.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn idle_notifications_are_consumed_without_invalidating_the_catalog() {
        let events = McpClientEvents::default();
        for method in [
            "notifications/message",
            "notifications/progress",
            "future/notification",
        ] {
            assert!(events.observe(&json!({"jsonrpc":"2.0","method":method,"params":{}})));
        }
        assert_eq!(events.catalog_revision(), 0);
        assert!(
            events.observe(&json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}))
        );
        assert_eq!(events.catalog_revision(), 1);
    }

    #[test]
    fn request_and_response_ids_remain_owned_by_rpc_handling() {
        let events = McpClientEvents::default();
        for message in [
            json!({"jsonrpc":"2.0","id":7,"method":"sampling/createMessage","params":{}}),
            json!({"jsonrpc":"2.0","id":7,"result":{"tools":[]}}),
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600}}),
            json!({"jsonrpc":"2.0","id":7,"method":"notifications/tools/list_changed"}),
        ] {
            assert!(!events.observe(&message), "{message}");
        }
        assert_eq!(events.catalog_revision(), 0);
    }
}
