use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::{future::Future, time::Duration};

use serde_json::Value;

type CatalogObserver = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct McpCallProgress {
    token: i64,
    last: Mutex<(Option<f64>, tokio::time::Instant)>,
}

impl McpCallProgress {
    fn observe(&self, params: &Value) {
        if params["progressToken"].as_i64() != Some(self.token) {
            return;
        }
        let Some(progress) = params["progress"]
            .as_f64()
            .filter(|value| value.is_finite())
        else {
            return;
        };
        let mut last = self.last.lock().unwrap_or_else(|error| error.into_inner());
        if last.0.is_none_or(|previous| progress > previous) {
            *last = (Some(progress), tokio::time::Instant::now());
        }
    }

    fn deadline(&self, idle: Duration) -> tokio::time::Instant {
        self.last
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .1
            + idle
    }

    pub(crate) async fn wait<F: Future>(&self, idle: Duration, future: F) -> Result<F::Output, ()> {
        tokio::pin!(future);
        loop {
            tokio::select! {
                biased;
                result = &mut future => return Ok(result),
                _ = tokio::time::sleep_until(self.deadline(idle)) => {
                    // Readers may receive progress on another task. Recheck the
                    // clock before expiring; duplicate/unrelated notifications
                    // cannot extend it and no extra wakeup queue is required.
                    if self.deadline(idle) <= tokio::time::Instant::now() {
                        return Err(());
                    }
                }
            }
        }
    }
}

/// Shared by the transport reader and connector slot. Notifications invalidate
/// catalogs even while no caller owns the RPC client mutex. Consumed id-less
/// notifications never fill the bounded queue reserved for request/response RPC.
#[derive(Default)]
pub(crate) struct McpClientEvents {
    catalog_revision: AtomicU64,
    observer: Mutex<Option<CatalogObserver>>,
    active_progress: Mutex<Option<Weak<McpCallProgress>>>,
}

impl McpClientEvents {
    pub(crate) fn track_call(&self, token: i64) -> Arc<McpCallProgress> {
        let progress = Arc::new(McpCallProgress {
            token,
            last: Mutex::new((None, tokio::time::Instant::now())),
        });
        // RPC calls own the client exclusively. A weak reference also ends
        // tracking when a completion, error, or cancelled future drops its guard.
        *self
            .active_progress
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(Arc::downgrade(&progress));
        progress
    }

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
        // JSON-RPC notifications. Logging/unknown methods need no RPC consumer.
        if message.get("id").is_some() {
            return false;
        }
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return false;
        };
        if method == "notifications/progress" {
            let progress = self
                .active_progress
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .and_then(Weak::upgrade);
            if let Some(progress) = progress {
                progress.observe(&message["params"]);
            }
        }
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

    #[tokio::test(start_paused = true)]
    async fn only_matching_advancing_progress_renews_the_call_idle_deadline() {
        let events = Arc::new(McpClientEvents::default());
        let progress = events.track_call(7);
        let idle = Duration::from_secs(5);
        let deadline = progress.deadline(idle);
        for params in [
            json!({"progressToken":8,"progress":1}),
            json!({"progressToken":7,"progress":"1"}),
        ] {
            events.observe(&json!({"method":"notifications/progress","params":params}));
        }
        assert_eq!(progress.deadline(idle), deadline);
        let observer = events.clone();
        let result = progress.wait(idle, async move {
            for value in 0..20 {
                tokio::time::sleep(Duration::from_secs(4)).await;
                observer.observe(&json!({"method":"notifications/progress","params":{"progressToken":7,"progress":value}}));
            }
            42
        }).await;
        assert_eq!(result, Ok(42));
        let deadline = progress.deadline(idle);
        tokio::time::advance(Duration::from_secs(4)).await;
        for value in [19, 18] {
            events.observe(&json!({"method":"notifications/progress","params":{"progressToken":7,"progress":value}}));
        }
        assert_eq!(progress.deadline(idle), deadline);
        assert_eq!(
            progress.wait(idle, std::future::pending::<()>()).await,
            Err(())
        );
    }

    #[test]
    fn completed_or_cancelled_call_tokens_cannot_renew_a_later_request() {
        let events = McpClientEvents::default();
        let prior = events.track_call(1);
        drop(prior);
        assert!(events
            .active_progress
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .is_none());
        let active = events.track_call(2);
        events.observe(
            &json!({"method":"notifications/progress","params":{"progressToken":1,"progress":100}}),
        );
        assert!(active.last.lock().unwrap().0.is_none());
    }

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
