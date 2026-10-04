use super::{chat::ChatPrivacyPolicy, PrivacyConfig};
use crate::{db::Database, error::CoreError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio_util::sync::CancellationToken;

tokio::task_local! {
    static INVOCATION_REVOCATION: CancellationToken;
}

/// Called at physical request seams, including candidates selected inside an
/// automatic fallback in the same poll as a primary failure. An outer select
/// alone cannot intercept two immediately-ready nested provider futures.
pub(crate) fn ensure_invocation_current() -> Result<(), CoreError> {
    if INVOCATION_REVOCATION
        .try_with(CancellationToken::is_cancelled)
        .unwrap_or(false)
    {
        Err(CoreError::Cancelled(
            "Privacy settings changed before provider dispatch".into(),
        ))
    } else {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct PrivacyRuns {
    policy_change: Mutex<()>,
    active: Mutex<HashMap<String, (CancellationToken, CancellationToken)>>,
}

/// Owns a policy snapshot and a cancellation registration for one invocation.
/// The registration spans pre-summarization, provider retries and tool callbacks.
pub struct PrivacyLease {
    id: String,
    runs: Arc<PrivacyRuns>,
    revoked: CancellationToken,
    pub policy: ChatPrivacyPolicy,
}

impl PrivacyLease {
    pub(crate) fn release<T>(&self, deliver: impl FnOnce(bool) -> T) -> T {
        let _guard = self
            .runs
            .policy_change
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        deliver(self.is_revoked())
    }
    pub async fn scope<F: std::future::Future>(&self, future: F) -> F::Output {
        INVOCATION_REVOCATION
            .scope(self.revoked.clone(), future)
            .await
    }
    pub async fn cancelled(&self) {
        self.revoked.cancelled().await;
    }
    pub fn is_revoked(&self) -> bool {
        self.revoked.is_cancelled()
    }
    pub fn ensure_current(&self) -> Result<(), CoreError> {
        if self.is_revoked() {
            Err(CoreError::Cancelled(
                "Privacy settings changed; start a new turn to use the current policy".into(),
            ))
        } else {
            Ok(())
        }
    }
}

impl Drop for PrivacyLease {
    fn drop(&mut self) {
        self.runs
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.id);
    }
}

impl Database {
    pub fn privacy_revision(&self) -> Result<String, CoreError> {
        Ok(self.conn().query_row(
            "SELECT revision FROM privacy_chat_state WHERE id=1",
            [],
            |row| row.get(0),
        )?)
    }
    /// Register and capture the policy under the same database lock used by
    /// save_privacy_config. A writer can never revoke only half an invocation.
    pub fn privacy_lease(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<PrivacyLease, CoreError> {
        let _policy_guard = self.privacy_policy_guard();
        let conn = self.conn();
        let policy = ChatPrivacyPolicy::load(&conn)?;
        let id = uuid::Uuid::new_v4().to_string();
        let revoked = CancellationToken::new();
        self.privacy_runs
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.clone(), (revoked.clone(), cancellation.clone()));
        Ok(PrivacyLease {
            id,
            runs: self.privacy_runs.clone(),
            revoked,
            policy,
        })
    }

    pub(crate) fn revoke_privacy_runs(&self) {
        for (lease, cancellation) in self
            .privacy_runs
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
        {
            lease.cancel();
            cancellation.cancel();
        }
    }

    pub(crate) fn privacy_policy_guard(&self) -> MutexGuard<'_, ()> {
        self.privacy_runs
            .policy_change
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

impl ChatPrivacyPolicy {
    pub fn config(&self) -> &PrivacyConfig {
        &self.config
    }
}
