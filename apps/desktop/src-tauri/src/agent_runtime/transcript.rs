//! Every external runtime shares Nexa's durable tool and transcript lifecycle.
use nexa_core::{
    agent::{AgentSteeringMessage, ExternalToolSession, PersistedAssistantMessage},
    error::CoreError,
};
use std::sync::Arc;

pub(super) struct Transcript(pub(super) Arc<ExternalToolSession>);

impl Transcript {
    pub(super) async fn completion_recovery_error(&self) -> Option<CoreError> {
        self.0.completion_recovery_error().await
    }
    pub(super) async fn tool_budget_rejected(&self) -> bool {
        self.0.tool_budget_rejected().await
    }
    pub(super) async fn completion_repair_status(
        &self,
    ) -> Result<Option<(String, Vec<String>)>, CoreError> {
        self.0.completion_repair_status().await
    }
    pub(super) fn ensure_delegated_recovery_clear(&self) -> Result<(), CoreError> {
        self.0.ensure_delegated_recovery_clear()
    }
    pub(super) async fn persist_intermediate(
        &self,
        text: &str,
    ) -> Result<PersistedAssistantMessage, CoreError> {
        self.0.persist_intermediate(text).await
    }
    pub(super) fn nexa_tools(&self) -> &Arc<ExternalToolSession> {
        &self.0
    }
    pub(super) async fn persist_answer(
        &self,
        text: &str,
    ) -> Result<PersistedAssistantMessage, CoreError> {
        self.0.persist_answer(text).await
    }
    pub(super) async fn persist_steering(
        &self,
        message: &AgentSteeringMessage,
    ) -> Result<(), CoreError> {
        self.0.persist_steering(message).await
    }
}
