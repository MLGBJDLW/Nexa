//! A bounded duplicate window plus a durable replay cursor. The number of
//! already-delivered events is not a lifetime execution budget.
use super::{event_inbox::EventInbox, protocol_error, CoreError};
use futures::{Stream, StreamExt};
use github_copilot_sdk::SessionEvent;
use std::collections::{HashSet, VecDeque};
use tokio::task::JoinHandle;

/// Drain the SDK subscriber independently of UI/database work, preserving even
/// ephemeral usage, filter and idle events in the shared native-event inbox.
pub(super) struct EventStream {
    inbox: EventInbox<SessionEvent>,
    task: JoinHandle<()>,
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
        let (writer, inbox) = EventInbox::new()?;
        let task = tokio::spawn(async move {
            let mut stream = Box::pin(stream);
            while let Some(event) = stream.next().await {
                let failed = event.is_err();
                if writer
                    .send(event.map_err(CoreError::StreamIncomplete))
                    .is_err()
                    || failed
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
            let _ = writer.send(Err(CoreError::StreamIncomplete(
                "Copilot event stream closed".into(),
            )));
        });
        Ok(Self { inbox, task })
    }
    pub(super) async fn recv(&mut self) -> Result<SessionEvent, CoreError> {
        self.inbox.recv().await.unwrap_or_else(|| {
            Err(CoreError::StreamIncomplete(
                "Copilot event stream closed".into(),
            ))
        })
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
    async fn subscription_loss_becomes_reconcilable_recovery() {
        for events in [vec![], vec![Err("subscription lagged".to_string())]] {
            let mut stream = EventStream::from_stream(futures::stream::iter(events)).unwrap();
            let error = stream.recv().await.unwrap_err();
            assert!(matches!(
                super::super::recover_interrupted_runtime(error),
                CoreError::RecoveryRequired {
                    action_reconciliation: true,
                    ..
                }
            ));
        }
    }
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
            if stream.inbox.is_closed() {
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
