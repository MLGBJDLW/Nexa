//! Tauri delivery adapter for committed Run Events and task projections.

use nexa_core::agent_run::{AgentRunEvent, AgentRunEventKind};
use nexa_core::conversation::AgentTaskRun;
use nexa_core::run_event_outbox::AgentRunEventDelivery;
use tauri::{AppHandle, Emitter};

use crate::agent_stream::emit_agent_run_frontend_event;
use crate::agent_task_events::emit_agent_task_run_snapshot;

pub(crate) struct DesktopAgentRunEventDelivery {
    app_handle: AppHandle,
}

impl DesktopAgentRunEventDelivery {
    pub(crate) fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }
}

impl AgentRunEventDelivery for DesktopAgentRunEventDelivery {
    fn deliver_run_event(&self, conversation_id: &str, event: &AgentRunEvent) {
        emit_agent_run_frontend_event(&self.app_handle, conversation_id, event);
        if let Some(revision) = changed_appearance_revision(event) {
            // Run Event tool artifacts are compacted for display. Broadcast an
            // invalidation, never apply their potentially truncated theme body.
            let _ = self.app_handle.emit(
                "appearance://changed",
                serde_json::json!({ "revision": revision }),
            );
        }
        crate::desktop_control_status::observe_committed_event(
            &self.app_handle,
            conversation_id,
            event,
        );
    }

    fn deliver_task_run_snapshot(&self, conversation_id: &str, snapshot: AgentTaskRun) {
        emit_agent_task_run_snapshot(&self.app_handle, conversation_id, snapshot);
    }
}

fn changed_appearance_revision(event: &AgentRunEvent) -> Option<u64> {
    if event.kind != AgentRunEventKind::ToolCompleted {
        return None;
    }
    let run = event.payload.get("run")?;
    if run.get("toolName")?.as_str()? != "appearance" || run.get("status")?.as_str()? != "completed"
    {
        return None;
    }
    let artifacts = run.get("artifacts")?;
    if artifacts.get("kind")?.as_str()? != "appearanceRegistry" {
        return None;
    }
    artifacts
        .get("payload")?
        .get("registry")?
        .get("revision")?
        .as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_core::agent::AgentEvent;
    use nexa_core::db::Database;
    use nexa_core::tools::{appearance_tool::AppearanceTool, Tool, ToolExecutionContext};

    #[tokio::test]
    async fn completed_agent_appearance_reaches_native_change_notification() {
        let db = Database::open_memory().unwrap();
        let result = AppearanceTool
            .execute(ToolExecutionContext::new(
                "appearance-call",
                r#"{"action":"activate","themeId":"light"}"#,
                &db,
                &[],
            ))
            .await
            .unwrap();
        let mut event = AgentRunEvent::from_agent_event(&AgentEvent::ToolCallResult {
            call_id: result.call_id,
            tool_name: "appearance".into(),
            content: result.content,
            is_error: result.is_error,
            artifacts: result.artifacts,
        });
        assert_eq!(
            changed_appearance_revision(&event),
            Some(db.load_appearance_registry().unwrap().revision)
        );
        event.kind = AgentRunEventKind::ToolProgress;
        assert!(changed_appearance_revision(&event).is_none());
        event.kind = AgentRunEventKind::ToolCompleted;
        event.payload["run"]["status"] = serde_json::json!("failed");
        assert!(changed_appearance_revision(&event).is_none());
    }
}
