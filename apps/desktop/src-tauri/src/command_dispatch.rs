//! Application IPC must never execute synchronous work on the window thread.
//!
//! Tauri's synchronous command wrappers run in the invoking WebView2 callback.
//! Even a read-only query can then stop native input while waiting for SQLite,
//! a filesystem, or a tool-owned mutex. Dispatch the generated handler on the
//! blocking pool; genuinely async commands still use Tauri's async executor.
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::ipc::Invoke;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct DispatchCapacity {
    ordinary: Arc<Semaphore>,
    control: Arc<Semaphore>,
}

impl Default for DispatchCapacity {
    fn default() -> Self {
        Self {
            ordinary: Arc::new(Semaphore::new(64)),
            control: Arc::new(Semaphore::new(8)),
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
    move |invoke| {
        let handler = handler.clone();
        let command = invoke.message.command().to_string();
        let resolver = invoke.resolver.clone();
        let permit = match capacity.admit(&command) {
            Ok(permit) => permit,
            Err(error) => {
                resolver.reject(error);
                return true;
            }
        };
        let queued_at = Instant::now();
        tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            let started_at = Instant::now();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(invoke)));
            match result {
                Ok(true) => {}
                Ok(false) => resolver.reject(format!("Command {command} not found")),
                Err(_) => resolver.reject(format!("Command {command} failed unexpectedly")),
            }
            let elapsed = started_at.elapsed();
            let queue_wait = started_at.duration_since(queued_at);
            if elapsed >= Duration::from_millis(250) || queue_wait >= Duration::from_millis(250) {
                // Deliberately exclude arguments, returned values, and paths.
                log::warn!(
                    "Slow command dispatch: {command}, queue={queue_wait:?}, execution={elapsed:?}"
                );
            }
        });
        // We own the response, including unknown-command and panic errors.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        drop((requests, stop, close));
        assert!(capacity.admit("search").is_ok());
    }
}
