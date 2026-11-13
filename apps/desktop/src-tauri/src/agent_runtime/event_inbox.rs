//! Ordered native events with bounded memory, independent of UI consumption.
//! A small memory FIFO avoids database work for normal streaming. The existing
//! private SQLite receipt store spills unread events and reuses
//! consumed pages. Dropping the inbox closes storage; it is never user history.
use super::{protocol_error, CoreError};
use nexa_core::runtime_receipts::RuntimeEventQueue;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
};
use tokio::sync::{mpsc::error::TryRecvError, Notify};

struct State {
    events: RuntimeEventQueue,
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
            events: RuntimeEventQueue::new()?,
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
        match state.events.pop() {
            Ok(Some(event)) => return Ok(Ok(event)),
            Err(error) => return Ok(Err(error)),
            Ok(None) => {}
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
                if let Err(error) = state.events.push(&value) {
                    state.failure = Some(protocol_error(error));
                    state.closed = true;
                    self.ready.notify_one();
                    return Err(protocol_error("Native event inbox storage failed"));
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_events_drain_before_terminal_failure_or_writer_close() {
        for failed in [false, true] {
            let (writer, mut inbox) = EventInbox::new().unwrap();
            // Exercise both the memory prefix and the spilled tail.
            for index in 0..300_u32 {
                writer.send(Ok(index)).unwrap();
            }
            if failed {
                writer
                    .send(Err(CoreError::StreamIncomplete("stream closed".into())))
                    .unwrap();
                assert!(writer.send(Ok(300)).is_err());
            }
            drop(writer);
            assert!(inbox.is_closed());
            for index in 0..300_u32 {
                assert_eq!(inbox.recv().await.unwrap().unwrap(), index);
            }
            if failed {
                assert!(matches!(
                    inbox.recv().await.unwrap(),
                    Err(CoreError::StreamIncomplete(_))
                ));
            }
            assert!(inbox.recv().await.is_none());
        }
    }
}
