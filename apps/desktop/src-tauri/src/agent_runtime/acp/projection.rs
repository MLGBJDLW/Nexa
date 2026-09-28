//! Agent-owned effects are reports, never commands to the Nexa dispatcher.
use super::{error, Result};
use nexa_core::{
    agent::{AgentEvent, ToolRunItem, ToolRunStatus},
    plugins::CapabilityOwner,
    tools::{ToolInputStreamingMode, ToolInterruptBehavior, ToolRenderKind, ToolRunCapabilities},
};
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc;

pub(super) struct ToolReports {
    provider: String,
    calls: HashMap<String, ToolRunItem>,
}

fn bounded(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

impl ToolReports {
    pub(super) fn new(provider: &str) -> Self {
        Self {
            provider: provider.into(),
            calls: HashMap::new(),
        }
    }

    pub(super) fn has_pending(&self) -> bool {
        self.calls.values().any(|run| {
            matches!(
                run.status,
                ToolRunStatus::Preparing | ToolRunStatus::Running | ToolRunStatus::ApprovalPending
            )
        })
    }

    pub(super) async fn update(
        &mut self,
        tx: &mpsc::Sender<AgentEvent>,
        value: &Value,
    ) -> Result<()> {
        let id = value["toolCallId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 1024)
            .ok_or_else(|| error("ACP tool report has no call id"))?;
        let started = !self.calls.contains_key(id);
        if started && self.calls.len() >= 512 {
            return Err(error("ACP tool report budget exceeded"));
        }
        let run = self
            .calls
            .entry(id.to_string())
            .or_insert_with(|| ToolRunItem {
                call_id: format!("acp:{}:{id}", self.provider),
                tool_name: "External tool".into(),
                owner: CapabilityOwner {
                    id: format!("external-agent:{}", self.provider),
                    name: self.provider.clone(),
                    capability: "external_agent_tool".into(),
                    description: "Reported by the external agent; executed by its native runtime."
                        .into(),
                },
                provider_executed: true,
                status: ToolRunStatus::Preparing,
                arguments: None,
                render_kind: ToolRenderKind::Generic,
                capabilities: ToolRunCapabilities {
                    input_streaming: ToolInputStreamingMode::None,
                    render_kind: ToolRenderKind::Generic,
                    read_only: false,
                    destructive: false,
                    concurrency_safe: false,
                    interrupt_behavior: ToolInterruptBehavior::Cancel,
                    resource_keys: vec![],
                },
                content: None,
                is_error: None,
                artifacts: None,
                progress_note: None,
                duration_ms: None,
            });
        if let Some(title) = value["title"].as_str() {
            run.tool_name = bounded(title, 200);
        }
        if let Some(kind) = value["kind"].as_str() {
            run.render_kind = match kind {
                "execute" => ToolRenderKind::CommandExecution,
                "edit" | "delete" | "move" => ToolRenderKind::FileChange,
                "search" => ToolRenderKind::Search,
                _ => ToolRenderKind::Generic,
            };
            run.capabilities.render_kind = run.render_kind;
            run.capabilities.read_only = matches!(kind, "read" | "search" | "think");
        }
        if let Some(arguments) = value.get("rawInput") {
            let safe = nexa_core::tool_argument_projection::audit_safe_arguments(
                &run.tool_name,
                arguments,
            );
            run.arguments = Some(bounded(&safe.to_string(), 16000));
        }
        if let Some(content) = value.get("content") {
            let text = content
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|block| match block["type"].as_str() {
                    Some("content") => block
                        .pointer("/content/text")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    Some("diff") => block["path"]
                        .as_str()
                        .map(|path| format!("Changed file: {path}")),
                    Some("terminal") => block["terminalId"]
                        .as_str()
                        .map(|id| format!("External terminal: {id}")),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            run.content = Some(bounded(&text, 16000));
        }
        if let Some(status) = value["status"].as_str() {
            run.status = match status {
                "pending" => ToolRunStatus::Preparing,
                "in_progress" => ToolRunStatus::Running,
                "completed" => ToolRunStatus::Completed,
                "failed" => ToolRunStatus::Failed,
                _ => return Err(error("Invalid ACP tool status")),
            };
        }
        let complete = matches!(run.status, ToolRunStatus::Completed | ToolRunStatus::Failed);
        run.is_error = complete.then_some(run.status == ToolRunStatus::Failed);
        let event = if complete {
            AgentEvent::ToolRunCompleted { run: run.clone() }
        } else if started {
            AgentEvent::ToolRunStarted { run: run.clone() }
        } else {
            AgentEvent::ToolRunUpdated { run: run.clone() }
        };
        tx.send(event).await.map_err(error)
    }

    pub(super) async fn close_pending(
        &mut self,
        tx: &mpsc::Sender<AgentEvent>,
        cancelled: bool,
    ) -> Result<()> {
        for run in self.calls.values_mut().filter(|run| {
            matches!(
                run.status,
                ToolRunStatus::Preparing | ToolRunStatus::Running | ToolRunStatus::ApprovalPending
            )
        }) {
            run.status = if cancelled {
                ToolRunStatus::Cancelled
            } else {
                ToolRunStatus::Failed
            };
            run.is_error = Some(true);
            run.progress_note = Some("External agent stopped before reporting completion".into());
            tx.send(AgentEvent::ToolRunCompleted { run: run.clone() })
                .await
                .map_err(error)?;
        }
        Ok(())
    }
}
