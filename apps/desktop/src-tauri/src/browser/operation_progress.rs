use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum BrowserOperationPhase {
    Moving,
    Committing,
    Waiting,
    Verified,
    ObservedUnchanged,
    ObservedPending,
    Failed,
    Interrupted,
}

impl BrowserOperationPhase {
    fn is_terminal(self) -> bool {
        !matches!(self, Self::Moving | Self::Committing | Self::Waiting)
    }
}

/// Presentation identity belongs to a host invocation, never a provider's
/// reusable call ID. This guard neither grants nor releases input authority.
pub(super) struct BrowserOperationProgress<'a> {
    operation_id: String,
    session_id: &'a str,
    tab_id: &'a str,
    call_id: &'a str,
    action: &'a str,
    emit: &'a (dyn Fn(serde_json::Value) + Sync),
    terminal: bool,
}

impl<'a> BrowserOperationProgress<'a> {
    pub fn new(
        session_id: &'a str,
        tab_id: &'a str,
        call_id: &'a str,
        action: &'a str,
        emit: &'a (dyn Fn(serde_json::Value) + Sync),
    ) -> Self {
        Self {
            operation_id: format!("browser_op_{}", uuid::Uuid::new_v4().simple()),
            session_id,
            tab_id,
            call_id,
            action,
            emit,
            terminal: false,
        }
    }

    pub fn update(&mut self, phase: BrowserOperationPhase, detail: serde_json::Value) {
        if self.terminal {
            return;
        }
        let mut payload = match detail {
            serde_json::Value::Object(fields) => fields,
            _ => serde_json::Map::new(),
        };
        if let serde_json::Value::Object(identity) = serde_json::json!({
            "operationId": self.operation_id,
            "sessionId": self.session_id,
            "tabId": self.tab_id,
            "callId": self.call_id,
            "action": self.action,
            "phase": phase,
        }) {
            payload.extend(identity);
        }
        self.terminal = phase.is_terminal();
        (self.emit)(serde_json::Value::Object(payload));
    }
}

impl Drop for BrowserOperationProgress<'_> {
    fn drop(&mut self) {
        self.update(BrowserOperationPhase::Interrupted, serde_json::json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn repeated_provider_calls_have_distinct_operations_with_stable_phases() {
        let events = Mutex::new(Vec::new());
        let emit = |payload| events.lock().unwrap().push(payload);
        {
            let mut action =
                BrowserOperationProgress::new("session", "tab", "call_0", "click", &emit);
            action.update(
                BrowserOperationPhase::Moving,
                serde_json::json!({"targetRef":"e1"}),
            );
            action.update(BrowserOperationPhase::Committing, serde_json::json!({}));
            action.update(
                BrowserOperationPhase::Verified,
                serde_json::json!({"observationId":"obs-2"}),
            );
            action.update(BrowserOperationPhase::Moving, serde_json::json!({}));
        }
        {
            let mut repeated =
                BrowserOperationProgress::new("session", "tab", "call_0", "click", &emit);
            repeated.update(BrowserOperationPhase::Moving, serde_json::json!({}));
            repeated.update(BrowserOperationPhase::Failed, serde_json::json!({}));
        }
        let events = events.lock().unwrap();
        assert_eq!(
            events.len(),
            5,
            "terminal operations emit no progress or drop interruption"
        );
        assert!(events[..3]
            .iter()
            .all(|event| event["operationId"] == events[0]["operationId"]));
        assert_eq!(events[3]["operationId"], events[4]["operationId"]);
        assert_ne!(events[0]["operationId"], events[3]["operationId"]);
        assert!(events.iter().all(|event| event["callId"] == "call_0"));
    }

    #[test]
    fn cancelled_wait_retains_its_identity_and_detail_cannot_replace_host_scope() {
        let events = Mutex::new(Vec::new());
        let emit = |payload| events.lock().unwrap().push(payload);
        {
            let mut waiting =
                BrowserOperationProgress::new("session", "tab", "call_0", "wait_for", &emit);
            waiting.update(
                BrowserOperationPhase::Waiting,
                serde_json::json!({"operationId":"spoofed","callId":"other"}),
            );
        }
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["operationId"], events[1]["operationId"]);
        assert_ne!(events[0]["operationId"], "spoofed");
        assert_eq!(events[1]["callId"], "call_0");
        assert_eq!(events[1]["phase"], "interrupted");
    }

    #[tokio::test]
    async fn dropped_action_future_emits_interruption_for_the_same_operation() {
        let events = Mutex::new(Vec::new());
        let emit = |payload| events.lock().unwrap().push(payload);
        let action = async {
            let mut progress =
                BrowserOperationProgress::new("session", "tab", "call_0", "click", &emit);
            progress.update(BrowserOperationPhase::Moving, serde_json::json!({}));
            progress.update(BrowserOperationPhase::Committing, serde_json::json!({}));
            std::future::pending::<()>().await;
        };
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), action)
                .await
                .is_err()
        );
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(events
            .iter()
            .all(|event| event["operationId"] == events[0]["operationId"]));
        assert_eq!(events[2]["phase"], "interrupted");
    }
}
