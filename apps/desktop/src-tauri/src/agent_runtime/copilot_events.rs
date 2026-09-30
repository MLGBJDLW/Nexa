//! A bounded duplicate window plus a durable replay cursor. The number of
//! already-delivered events is not a lifetime execution budget.
use super::{protocol_error, CoreError};
use futures::{Stream, StreamExt};
use github_copilot_sdk::SessionEvent;
use nexa_core::runtime_receipts::RuntimeReceipts;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::{sync::Notify, task::JoinHandle};

/// Keep the SDK's small broadcast queue draining while the UI or a tool's
/// transcript lock is busy. Unread events spill to a private temporary SQLite
/// database; consumed rows are reclaimed. Ephemeral usage/filter/idle events
/// receive the same ordering guarantee as persistent assistant messages.
pub(super) struct EventStream {
    buffer: Arc<Mutex<EventBuffer>>,
    ready: Arc<Notify>,
    task: JoinHandle<()>,
    cursor: u64,
}
struct EventBuffer {
    receipts: RuntimeReceipts,
    count: u64,
    closed: Option<String>,
}
impl Drop for EventStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl EventStream {
    pub(super) fn start(
        subscription: github_copilot_sdk::subscription::EventSubscription,
    ) -> Result<Self, CoreError> {
        Self::from_stream(subscription.map(|event| event.map_err(|error| error.to_string())))
    }
    fn from_stream(
        stream: impl Stream<Item = Result<SessionEvent, String>> + Send + 'static,
    ) -> Result<Self, CoreError> {
        let buffer = Arc::new(Mutex::new(EventBuffer {
            receipts: RuntimeReceipts::new()?,
            count: 0,
            closed: None,
        }));
        let ready = Arc::new(Notify::new());
        let producer = buffer.clone();
        let notify = ready.clone();
        let task = tokio::spawn(async move {
            let mut stream = Box::pin(stream);
            while let Some(event) = stream.next().await {
                let failed = {
                    let mut state = producer.lock().unwrap_or_else(|e| e.into_inner());
                    match event {
                        Ok(event) => {
                            let index = state.count;
                            match state.receipts.insert(&index.to_string(), "", &event) {
                                Ok(()) => {
                                    state.count += 1;
                                    false
                                }
                                Err(error) => {
                                    state.closed = Some(error.to_string());
                                    true
                                }
                            }
                        }
                        Err(error) => {
                            state.closed = Some(error);
                            true
                        }
                    }
                };
                notify.notify_one();
                if failed {
                    return;
                }
                tokio::task::yield_now().await;
            }
            producer.lock().unwrap_or_else(|e| e.into_inner()).closed =
                Some("Copilot event stream closed".into());
            notify.notify_one();
        });
        Ok(Self {
            buffer,
            ready,
            task,
            cursor: 0,
        })
    }
    pub(super) async fn recv(&mut self) -> Result<SessionEvent, CoreError> {
        loop {
            let notified = self.ready.notified();
            {
                let state = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
                if self.cursor < state.count {
                    let id = self.cursor.to_string();
                    let (_, event) = state
                        .receipts
                        .get(&id)?
                        .ok_or_else(|| protocol_error("Copilot event spool lost its next event"))?;
                    state.receipts.remove(&id)?;
                    self.cursor += 1;
                    return Ok(event);
                }
                if let Some(error) = &state.closed {
                    return Err(protocol_error(error));
                }
            }
            notified.await;
        }
    }
}

#[derive(Default)]
pub(super) struct EventLedger {
    recent: HashSet<String>,
    order: VecDeque<String>,
    durable: Option<String>,
}
impl EventLedger {
    pub(super) fn observe(&mut self, event: &SessionEvent) -> bool {
        if !self.recent.insert(event.id.clone()) {
            return false;
        }
        self.order.push_back(event.id.clone());
        if event.ephemeral != Some(true) {
            self.durable = Some(event.id.clone());
        }
        while self.order.len() > 8192 {
            if let Some(id) = self.order.pop_front() {
                self.recent.remove(&id);
            }
        }
        true
    }
    pub(super) fn backfill(
        &self,
        events: Vec<SessionEvent>,
    ) -> Result<Vec<SessionEvent>, CoreError> {
        let start = match &self.durable {
            Some(id) => events.iter().rposition(|event| &event.id == id).map(|index| index + 1)
                .ok_or_else(|| protocol_error("Copilot event history no longer contains the confirmed replay cursor; uncertain actions were not replayed"))?,
            None => 0,
        };
        Ok(events
            .into_iter()
            .skip(start)
            .filter(|event| event.ephemeral != Some(true))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn slow_consumer_retains_ephemeral_filter_usage_and_idle_in_order() {
        let mut events = (0..2000).map(|index| Ok(serde_json::from_value(serde_json::json!({"id":index.to_string(),"timestamp":"2026-09-30T00:00:00Z","type":"assistant.message_delta","ephemeral":true,"data":{"messageId":"answer","deltaContent":"x"}})).unwrap())).collect::<Vec<_>>();
        for (id, kind, data) in [
            (
                "usage",
                "session.usage_info",
                serde_json::json!({"currentTokens":8000,"tokenLimit":200000}),
            ),
            (
                "filter",
                "assistant.usage",
                serde_json::json!({"contentFilterTriggered":true}),
            ),
            ("idle", "session.idle", serde_json::json!({})),
        ] {
            events.push(Ok(serde_json::from_value(serde_json::json!({"id":id,"timestamp":"2026-09-30T00:00:00Z","type":kind,"ephemeral":true,"data":data})).unwrap()));
        }
        let mut stream = EventStream::from_stream(futures::stream::iter(events)).unwrap();
        // Force the producer to finish before the consumer catches up.
        loop {
            if stream.buffer.lock().unwrap().closed.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
        for index in 0..2000 {
            assert_eq!(stream.recv().await.unwrap().id, index.to_string());
        }
        for id in ["usage", "filter", "idle"] {
            assert_eq!(stream.recv().await.unwrap().id, id);
        }
    }
    #[test]
    fn long_event_stream_uses_a_cursor_without_replaying_evicted_ids() {
        let mut ledger = EventLedger::default();
        let mut history = Vec::new();
        for index in 0..20_000 {
            let event: SessionEvent = serde_json::from_value(serde_json::json!({"id":index.to_string(),"timestamp":"2026-09-30T00:00:00Z","type":"session.info","data":{}})).unwrap();
            if index < 19_999 {
                assert!(ledger.observe(&event));
            }
            history.push(event);
        }
        assert!(ledger.recent.len() <= 8192);
        let pending = ledger.backfill(history).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "19999");
    }
}
