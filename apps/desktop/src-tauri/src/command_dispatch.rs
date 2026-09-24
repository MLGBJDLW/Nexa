//! Application IPC must never execute synchronous work on the window thread.
//!
//! Tauri's synchronous command wrappers run in the invoking WebView2 callback.
//! Even a read-only query can then stop native input while waiting for SQLite,
//! a filesystem, or a tool-owned mutex. Dispatch the generated handler on the
//! blocking pool; genuinely async commands still use Tauri's async executor.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::ipc::Invoke;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

type DispatchJob = Box<dyn FnOnce() + Send + 'static>;
type DispatchQueues = HashMap<String, VecDeque<DispatchJob>>;

#[derive(Clone, Default)]
struct OrderedDispatch {
    queues: Arc<Mutex<DispatchQueues>>,
}

impl OrderedDispatch {
    fn submit(&self, key: String, job: DispatchJob) {
        {
            let mut queues = self
                .queues
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(queue) = queues.get_mut(&key) {
                queue.push_back(job);
                return;
            }
            queues.insert(key.clone(), VecDeque::from([job]));
        }
        let queues = self.queues.clone();
        tauri::async_runtime::spawn_blocking(move || loop {
            let job = {
                let mut queues = queues.lock().unwrap_or_else(|error| error.into_inner());
                match queues.get_mut(&key).and_then(VecDeque::pop_front) {
                    Some(job) => job,
                    None => {
                        queues.remove(&key);
                        break;
                    }
                }
            };
            // A rejected or panicking call must not strand the later requests.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        });
    }
}

struct DispatchCapacity {
    ordinary: Arc<Semaphore>,
    control: Arc<Semaphore>,
    terminal_input: Arc<Semaphore>,
}

impl Default for DispatchCapacity {
    fn default() -> Self {
        Self {
            ordinary: Arc::new(Semaphore::new(64)),
            control: Arc::new(Semaphore::new(8)),
            terminal_input: Arc::new(Semaphore::new(16)),
        }
    }
}

impl DispatchCapacity {
    fn admit(&self, command: &str) -> Result<OwnedSemaphorePermit, String> {
        // A backlog of slow reads must leave capacity to stop work, dismiss
        // approvals, close a blocked pipe, or return browser control.
        let pool = if matches!(
            command,
            "agent_stop_cmd"
                | "approve_tool_call_cmd"
                | "stop_desktop_control_cmd"
                | "end_desktop_share_cmd"
                | "terminal_close_session_cmd"
                | "browser_stop_cmd"
                | "browser_acquire_control_cmd"
                | "cancel_model_download_cmd"
        ) {
            &self.control
        } else if matches!(
            command,
            "terminal_write_session_cmd" | "terminal_resize_session_cmd"
        ) {
            // A background database/tool backlog must not reject typing or
            // PTY cursor replies. UI writes are serialized per session.
            &self.terminal_input
        } else {
            &self.ordinary
        };
        pool.clone().try_acquire_owned().map_err(|_| {
            "Nexa is busy processing earlier requests; retry after they complete.".into()
        })
    }
}

pub fn off_main_thread(
    handler: impl Fn(Invoke) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    let handler = Arc::new(handler);
    let capacity = DispatchCapacity::default();
    let ordered = OrderedDispatch::default();
    move |invoke| {
        let handler = handler.clone();
        let command = invoke.message.command().to_string();
        let key = command_resource_key(&command, invoke.message.payload());
        let resolver = invoke.resolver.clone();
        let permit = match capacity.admit(&command) {
            Ok(permit) => permit,
            Err(error) => {
                resolver.reject(error);
                return true;
            }
        };
        let queued_at = Instant::now();
        ordered.submit(
            key,
            Box::new(move || {
                let _permit = permit;
                let started_at = Instant::now();
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(invoke)));
                match result {
                    Ok(true) => {}
                    Ok(false) => resolver.reject(format!("Command {command} not found")),
                    Err(_) => resolver.reject(format!("Command {command} failed unexpectedly")),
                }
                let elapsed = started_at.elapsed();
                let queue_wait = started_at.duration_since(queued_at);
                if elapsed >= Duration::from_millis(250) || queue_wait >= Duration::from_millis(250)
                {
                    // Deliberately exclude arguments, returned values, and paths.
                    log::warn!(
                    "Slow command dispatch: {command}, queue={queue_wait:?}, execution={elapsed:?}"
                );
                }
            }),
        );
        // We own the response, including unknown-command and panic errors.
        true
    }
}

fn command_resource_key(command: &str, body: &tauri::ipc::InvokeBody) -> String {
    // Preserve native synchronous mutation order (resize, input, settings)
    // without putting another session or Stop behind a blocked read. Async
    // command implementations retain their own transaction/revision fences.
    let resource = match body {
        tauri::ipc::InvokeBody::Json(value) => {
            ["sessionId", "conversationId", "runId", "id", "windowLabel"]
                .iter()
                .find_map(|key| value.get(key).and_then(serde_json::Value::as_str))
        }
        _ => None,
    };
    format!("{command}:{}", resource.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sync_mutations_keep_resource_order_without_blocking_other_resources() {
        let dispatch = OrderedDispatch::default();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        dispatch.submit(
            "resize:one".into(),
            Box::new(move || {
                let _ = entered_tx.send(());
                let _ = release_rx.recv_timeout(Duration::from_secs(3));
            }),
        );
        entered_rx.await.unwrap();
        let (second_tx, mut second_rx) = tokio::sync::oneshot::channel();
        dispatch.submit(
            "resize:one".into(),
            Box::new(move || {
                let _ = second_tx.send(());
            }),
        );
        let (other_tx, other_rx) = tokio::sync::oneshot::channel();
        dispatch.submit(
            "resize:two".into(),
            Box::new(move || {
                let _ = other_tx.send(());
            }),
        );
        let independent = tokio::time::timeout(Duration::from_millis(500), other_rx).await;
        let reordered = tokio::time::timeout(Duration::from_millis(100), &mut second_rx)
            .await
            .is_ok();
        let _ = release_tx.send(());
        assert!(
            independent.is_ok(),
            "another terminal must remain responsive"
        );
        assert!(
            !reordered,
            "a later resize ran before an earlier resize finished"
        );
        tokio::time::timeout(Duration::from_secs(1), second_rx)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn repeated_slow_requests_leave_stop_capacity_and_release_their_slots() {
        let capacity = DispatchCapacity::default();
        let requests = (0..64)
            .map(|_| capacity.admit("search").unwrap())
            .collect::<Vec<_>>();
        assert!(capacity.admit("search").is_err());
        let stop = capacity
            .admit("agent_stop_cmd")
            .expect("slow reads must not exclude cancellation");
        let close = capacity.admit("terminal_close_session_cmd").unwrap();
        let input = capacity
            .admit("terminal_write_session_cmd")
            .expect("slow reads must not reject terminal typing");
        let resize = capacity.admit("terminal_resize_session_cmd").unwrap();
        drop((requests, stop, close, input, resize));
        assert!(capacity.admit("search").is_ok());
    }
}
