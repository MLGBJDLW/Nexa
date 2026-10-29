//! Bounded native capture cleanup. Driver shutdown may outlive a tool timeout,
//! but it must not retain the input lane or create unlimited cleanup threads.
use std::sync::{mpsc, Arc, Mutex};
#[cfg(test)]
use std::time::Duration;

type Cleanup = Box<dyn FnOnce() + Send>;

pub(super) struct CaptureCleanupPool {
    slots: Arc<tokio::sync::Semaphore>,
    sender: mpsc::SyncSender<Cleanup>,
}

impl CaptureCleanupPool {
    pub fn new(capacity: usize) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel::<Cleanup>(capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        for index in 0..capacity {
            let receiver = Arc::clone(&receiver);
            std::thread::Builder::new()
                .name(format!("nexa-capture-cleanup-{index}"))
                .spawn(move || loop {
                    let task = match receiver.lock() {
                        Ok(receiver) => receiver.recv(),
                        Err(_) => return,
                    };
                    match task {
                        Ok(task) => task(),
                        Err(_) => return,
                    }
                })
                .map_err(|error| format!("Could not start capture cleanup worker: {error}"))?;
        }
        Ok(Self {
            slots: Arc::new(tokio::sync::Semaphore::new(capacity)),
            sender,
        })
    }

    pub fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
        Arc::clone(&self.slots).try_acquire_owned().map_err(|_| {
            "Native capture workers are still stopping; no additional capture was started.".into()
        })
    }

    /// A timed-out acknowledgement keeps the permit inside the cleanup job.
    #[cfg(test)]
    pub fn finish<F>(
        &self,
        permit: tokio::sync::OwnedSemaphorePermit,
        cleanup: F,
        timeout: Duration,
    ) -> Result<(), String>
    where
        F: FnOnce() -> Result<(), String> + Send + 'static,
    {
        self.dispatch(permit, cleanup)?
            .recv_timeout(timeout)
            .map_err(|_| "Native capture cleanup is still pending".to_string())?
    }

    /// Admission covers startup, frame production and teardown. A caller may
    /// time out while the job retains its slot until the driver actually exits.
    pub fn dispatch<F>(
        &self,
        permit: tokio::sync::OwnedSemaphorePermit,
        operation: F,
    ) -> Result<mpsc::Receiver<Result<(), String>>, String>
    where
        F: FnOnce() -> Result<(), String> + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(Box::new(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
                    .unwrap_or_else(|_| Err("Native capture cleanup panicked".into()));
                if result.is_err() {
                    // Failed teardown may have detached a still-running driver
                    // worker. Quarantine this slot instead of admitting more.
                    permit.forget();
                } else {
                    drop(permit);
                }
                let _ = sender.send(result);
            }))
            .map_err(|_| "Native capture cleanup queue is unavailable".to_string())?;
        Ok(receiver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_timeout_retains_the_same_admission_until_the_worker_exits() {
        let pool = CaptureCleanupPool::new(1).unwrap();
        let (release, waiting) = mpsc::channel();
        let completion = pool
            .dispatch(pool.acquire().unwrap(), move || {
                waiting.recv().unwrap();
                Ok(())
            })
            .unwrap();
        assert!(completion.recv_timeout(Duration::from_millis(10)).is_err());
        assert!(
            pool.acquire().is_err(),
            "startup must not escape capture admission"
        );
        release.send(()).unwrap();
        completion
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert!(pool.acquire().is_ok());
    }

    #[test]
    fn stalled_cleanup_returns_to_the_caller_and_keeps_capture_admission_bounded() {
        let pool = CaptureCleanupPool::new(2).unwrap();
        let (release_one, wait_one) = mpsc::channel();
        let (release_two, wait_two) = mpsc::channel();
        assert!(pool
            .finish(
                pool.acquire().unwrap(),
                move || {
                    wait_one.recv().unwrap();
                    Ok(())
                },
                Duration::from_millis(10)
            )
            .is_err());
        assert!(pool
            .finish(
                pool.acquire().unwrap(),
                move || {
                    wait_two.recv().unwrap();
                    Ok(())
                },
                Duration::from_millis(10)
            )
            .is_err());
        assert!(pool.acquire().is_err());
        release_one.send(()).unwrap();
        release_two.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while pool.slots.available_permits() != 2 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(pool.slots.available_permits(), 2);
        assert!(pool
            .finish(pool.acquire().unwrap(), || Ok(()), Duration::from_secs(1))
            .is_ok());
        assert!(pool
            .finish(
                pool.acquire().unwrap(),
                || Err("driver failed to stop".into()),
                Duration::from_secs(1)
            )
            .is_err());
        let remaining = pool.acquire().unwrap();
        assert!(pool.acquire().is_err());
        drop(remaining);
    }
}
