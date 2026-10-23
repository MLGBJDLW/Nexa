//! Ordered native events with bounded memory, independent of UI consumption.
//! The existing private SQLite receipt store spills unread events and reuses
//! consumed pages. Dropping the inbox closes storage; it is never user history.
use super::{protocol_error, CoreError};
use nexa_core::runtime_receipts::RuntimeReceipts;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc::error::TryRecvError, Notify};

struct State {
    receipts: RuntimeReceipts,
    written: u64,
    read: u64,
    closed: bool,
    receiver_closed: bool,
    failure: Option<CoreError>,
}

pub(super) struct EventInbox<T> {
    state: Arc<Mutex<State>>,
    ready: Arc<Notify>,
    item: PhantomData<T>,
}

pub(super) struct EventWriter<T> {
    state: Arc<Mutex<State>>,
    ready: Arc<Notify>,
    item: PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned> EventInbox<T> {
    pub(super) fn new() -> Result<(EventWriter<T>, Self), CoreError> {
        let state = Arc::new(Mutex::new(State {
            receipts: RuntimeReceipts::new()?,
            written: 0,
            read: 0,
            closed: false,
            receiver_closed: false,
            failure: None,
        }));
        let ready = Arc::new(Notify::new());
        Ok((
            EventWriter {
                state: state.clone(),
                ready: ready.clone(),
                item: PhantomData,
            },
            Self {
                state,
                ready,
                item: PhantomData,
            },
        ))
    }

    pub(super) fn is_closed(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed
    }

    pub(super) fn try_recv(&mut self) -> Result<Result<T, CoreError>, TryRecvError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.read < state.written {
            let id = state.read.to_string();
            let event = state
                .receipts
                .get(&id)
                .and_then(|row| {
                    row.map(|(_, value)| value)
                        .ok_or_else(|| protocol_error("Native event inbox lost its next event"))
                })
                .and_then(|event| {
                    state.receipts.remove(&id)?;
                    Ok(event)
                });
            state.read += 1;
            return Ok(event);
        }
        if let Some(error) = state.failure.take() {
            return Ok(Err(error));
        }
        Err(if state.closed {
            TryRecvError::Disconnected
        } else {
            TryRecvError::Empty
        })
    }

    pub(super) async fn recv(&mut self) -> Option<Result<T, CoreError>> {
        loop {
            let ready = self.ready.clone();
            let notified = ready.notified();
            match self.try_recv() {
                Ok(value) => return Some(value),
                Err(TryRecvError::Disconnected) => return None,
                Err(TryRecvError::Empty) => notified.await,
            }
        }
    }
}

impl<T: Serialize> EventWriter<T> {
    pub(super) fn send(&self, value: Result<T, CoreError>) -> Result<(), CoreError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.receiver_closed || state.closed {
            return Err(protocol_error("Native event inbox is closed"));
        }
        match value {
            Ok(value) => {
                let id = state.written.to_string();
                if let Err(error) = state.receipts.insert(&id, "", &value) {
                    state.failure = Some(protocol_error(error));
                    state.closed = true;
                    self.ready.notify_one();
                    return Err(protocol_error("Native event inbox storage failed"));
                }
                state.written += 1;
            }
            Err(error) => {
                state.failure = Some(error);
                state.closed = true;
            }
        }
        self.ready.notify_one();
        Ok(())
    }
}

impl<T> Drop for EventWriter<T> {
    fn drop(&mut self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        self.ready.notify_one();
    }
}

impl<T> Drop for EventInbox<T> {
    fn drop(&mut self) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .receiver_closed = true;
    }
}
