//! A manual Fork is control-plane input, consumed only at a safe turn boundary.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::ContextCompactionResult;
use crate::activity::{ActivityEventKind, ActivityRuntime};
use crate::error::CoreError;

type ForkResult = Result<ContextCompactionResult, CoreError>;
type ForkCompletion = Arc<Mutex<Option<oneshot::Sender<ForkResult>>>>;

#[derive(Clone)]
pub struct ContextForkRequest {
    pub(crate) cancellation: CancellationToken,
    operation_id: String,
    activities: ActivityRuntime,
    pub(super) admission: Arc<ForkAdmission>,
    completion: ForkCompletion,
}

#[derive(Default)]
pub(super) struct ForkAdmission(AtomicU8);

impl ForkAdmission {
    fn claim(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(super) fn cancel_if_queued(&self) -> bool {
        self.0
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

impl std::fmt::Debug for ContextForkRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextForkRequest")
            .field("operation_id", &self.operation_id)
            .finish()
    }
}

impl ContextForkRequest {
    pub(crate) fn new(
        operation_id: String,
        activities: ActivityRuntime,
        cancellation: CancellationToken,
    ) -> (
        Self,
        oneshot::Receiver<Result<ContextCompactionResult, CoreError>>,
    ) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                operation_id,
                activities,
                cancellation,
                admission: Arc::new(ForkAdmission::default()),
                completion: Arc::new(Mutex::new(Some(sender))),
            },
            receiver,
        )
    }

    pub(crate) fn begin(&self) -> Result<(), CoreError> {
        if self.cancellation.is_cancelled() || !self.admission.claim() {
            return Err(CoreError::Cancelled("Fork cancelled before handoff".into()));
        }
        self.activities.append(
            &self.operation_id,
            ActivityEventKind::Progress,
            serde_json::json!({"phase": "committing", "progress": 0.5}),
        )?;
        Ok(())
    }

    pub(crate) fn complete(self, result: Result<ContextCompactionResult, CoreError>) {
        if let Some(sender) = self
            .completion
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = sender.send(result);
        }
    }
}
