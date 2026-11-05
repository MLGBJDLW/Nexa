//! Reverse requests share the authenticated device channel. Reconnect never
//! replays input: outstanding requests finish with an uncertain outcome.
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub const PHONE_CONTROL_CAPABILITY: &str = "phone-control-v1";
const LOST: &str = "Phone control disconnected or was disabled. The action outcome may be unknown; observe after reconnecting before deciding what to do. Never repeat an uncertain send.";

struct Connection {
    session: String,
    enabled: bool,
    sender: mpsc::Sender<Value>,
}
struct Pending {
    owner: String,
    session: String,
    result: oneshot::Sender<Result<Value, String>>,
}
#[derive(Default)]
struct State {
    connections: HashMap<String, Connection>,
    pending: HashMap<String, Pending>,
}
#[derive(Default)]
pub(crate) struct DeviceControl {
    state: Mutex<State>,
}
struct RequestGuard {
    broker: Arc<DeviceControl>,
    id: String,
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.broker.cancel(&self.id);
    }
}

impl DeviceControl {
    pub fn attach(&self, owner: &str, session: &str) -> mpsc::Receiver<Value> {
        let (sender, receiver) = mpsc::channel(4);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        Self::clear_pending(&mut state, owner);
        state.connections.insert(
            owner.into(),
            Connection {
                session: session.into(),
                enabled: false,
                sender,
            },
        );
        receiver
    }
    fn clear_pending(state: &mut State, owner: &str) {
        let ids = state
            .pending
            .iter()
            .filter(|(_, pending)| pending.owner == owner)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(pending) = state.pending.remove(&id) {
                if let Some(connection) = state.connections.get(owner) {
                    let _ = connection
                        .sender
                        .try_send(json!({"event":"device:control-cancel","payload":{"id":id}}));
                }
                let _ = pending.result.send(Err(LOST.into()));
            }
        }
    }
    pub fn enable(&self, owner: &str, session: &str, enabled: bool) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(connection) = state
            .connections
            .get_mut(owner)
            .filter(|connection| connection.session == session)
        else {
            return false;
        };
        connection.enabled = enabled;
        if !enabled {
            Self::clear_pending(&mut state, owner);
        }
        true
    }
    pub fn detach(&self, owner: &str, session: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .connections
            .get(owner)
            .is_some_and(|connection| connection.session == session)
        {
            Self::clear_pending(&mut state, owner);
            state.connections.remove(owner);
        }
    }
    pub fn enabled(&self, owner: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .connections
            .get(owner)
            .is_some_and(|connection| connection.enabled)
    }
    pub fn complete(
        &self,
        owner: &str,
        session: &str,
        id: &str,
        value: Value,
    ) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state
            .pending
            .get(id)
            .is_some_and(|pending| pending.owner == owner && pending.session == session)
        {
            return Err("The phone request is no longer active for this device connection.".into());
        }
        let pending = state.pending.remove(id).expect("validated pending request");
        let _ = pending.result.send(Ok(value));
        Ok(())
    }
    fn cancel(&self, id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(pending) = state.pending.remove(id) {
            if let Some(connection) = state
                .connections
                .get(&pending.owner)
                .filter(|connection| connection.session == pending.session)
            {
                let _ = connection
                    .sender
                    .try_send(json!({"event":"device:control-cancel","payload":{"id":id}}));
            }
        }
    }
    pub async fn request(
        self: &Arc<Self>,
        owner: &str,
        arguments: Value,
        cancel: CancellationToken,
    ) -> Result<Value, String> {
        if cancel.is_cancelled() {
            return Err("Phone request cancelled before dispatch.".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let receiver = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.pending.values().any(|pending| pending.owner == owner) {
                return Err("This phone is busy with another operation. Wait for its result before continuing.".into());
            }
            let connection = state.connections.get(owner).filter(|connection|connection.enabled)
                .ok_or("Enable this computer's phone control in Nexa Mobile settings while the phone is connected and unlocked.")?;
            let session = connection.session.clone();
            let sender = connection.sender.clone();
            let (result, receiver) = oneshot::channel();
            state.pending.insert(
                id.clone(),
                Pending {
                    owner: owner.into(),
                    session,
                    result,
                },
            );
            if sender
                .try_send(
                    json!({"event":"device:control","payload":{"id":id,"arguments":arguments}}),
                )
                .is_err()
            {
                state.pending.remove(&id);
                return Err(
                    "The phone connection is busy or closed. No input was dispatched.".into(),
                );
            }
            receiver
        };
        let _guard = RequestGuard {
            broker: self.clone(),
            id,
        };
        tokio::select! {
            result = receiver => result.unwrap_or_else(|_|Err(LOST.into())),
            _ = cancel.cancelled() => Err("Phone request cancelled. Observe before deciding whether another action is needed; never repeat an uncertain send.".into()),
            _ = tokio::time::sleep(Duration::from_secs(30)) => Err("The phone did not return an outcome. Observe before deciding whether another action is needed; never repeat an uncertain send.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn responses_are_bound_to_device_and_connection() {
        let broker = Arc::new(DeviceControl::default());
        let mut socket = broker.attach("phone", "first");
        assert!(broker.enable("phone", "first", true));
        let task = tokio::spawn({
            let broker = broker.clone();
            async move {
                broker
                    .request(
                        "phone",
                        json!({"action":"observe"}),
                        CancellationToken::new(),
                    )
                    .await
            }
        });
        let event = socket.recv().await.unwrap();
        let id = event["payload"]["id"].as_str().unwrap();
        assert!(broker.complete("other", "first", id, json!({})).is_err());
        assert!(broker.complete("phone", "old", id, json!({})).is_err());
        broker
            .complete("phone", "first", id, json!({"observationId":"new"}))
            .unwrap();
        assert_eq!(task.await.unwrap().unwrap()["observationId"], "new");
        assert!(broker.complete("phone", "first", id, json!({})).is_err());
    }

    #[tokio::test]
    async fn reconnect_cancels_outstanding_input_without_replay() {
        let broker = Arc::new(DeviceControl::default());
        let mut old_socket = broker.attach("phone", "old");
        broker.enable("phone", "old", true);
        let task = tokio::spawn({
            let broker = broker.clone();
            async move {
                broker
                    .request(
                        "phone",
                        json!({"action":"set_text","text":"once"}),
                        CancellationToken::new(),
                    )
                    .await
            }
        });
        let event = old_socket.recv().await.unwrap();
        let mut new_socket = broker.attach("phone", "new");
        assert!(task
            .await
            .unwrap()
            .unwrap_err()
            .contains("outcome may be unknown"));
        assert_eq!(
            old_socket.recv().await.unwrap()["event"],
            "device:control-cancel"
        );
        assert!(new_socket.try_recv().is_err());
        assert!(!broker.enabled("phone"));
        assert!(!broker.enable("phone", "old", true));
        broker.enable("phone", "new", true);
        broker.detach("phone", "old");
        assert!(broker.enabled("phone"));
        assert!(broker
            .complete(
                "phone",
                "old",
                event["payload"]["id"].as_str().unwrap(),
                json!({})
            )
            .is_err());
    }

    #[tokio::test]
    async fn cancellation_releases_the_single_operation_slot() {
        let broker = Arc::new(DeviceControl::default());
        let mut socket = broker.attach("phone", "session");
        broker.enable("phone", "session", true);
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let broker = broker.clone();
            let cancel = cancel.clone();
            async move {
                broker
                    .request("phone", json!({"action":"tap"}), cancel)
                    .await
            }
        });
        socket.recv().await.unwrap();
        assert!(broker
            .request("phone", json!({"action":"tap"}), CancellationToken::new())
            .await
            .unwrap_err()
            .contains("busy"));
        cancel.cancel();
        assert!(task.await.unwrap().unwrap_err().contains("cancelled"));
        assert_eq!(
            socket.recv().await.unwrap()["event"],
            "device:control-cancel"
        );
        assert!(broker.state.lock().unwrap().pending.is_empty());
        broker.enable("phone", "session", false);
        assert!(broker
            .request("phone", json!({"action":"tap"}), CancellationToken::new())
            .await
            .is_err());
        assert!(socket.try_recv().is_err());
    }
}
