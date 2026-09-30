//! Agent-owned effects are reports, never commands to the Nexa dispatcher.
use super::{error, Result};
use nexa_core::{
    agent::{AgentEvent, ToolRunItem, ToolRunStatus},
    plugins::CapabilityOwner,
    tools::{ToolInputStreamingMode, ToolInterruptBehavior, ToolRenderKind, ToolRunCapabilities},
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;

pub(super) struct ToolReports {
    provider: String,
    calls: HashMap<String, ToolRunItem>,
    completed: nexa_core::runtime_receipts::RuntimeReceipts,
    terminal_bindings: HashMap<String, Vec<String>>,
}

fn bounded(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

impl ToolReports {
    pub(super) fn new(provider: &str) -> Result<Self> {
        Ok(Self {
            provider: provider.into(),
            calls: HashMap::new(),
            completed: nexa_core::runtime_receipts::RuntimeReceipts::new()?,
            terminal_bindings: HashMap::new(),
        })
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
        terminals: &super::terminals::Terminals,
    ) -> Result<()> {
        let id = value["toolCallId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 1024)
            .ok_or_else(|| error("ACP tool report has no call id"))?;
        let cached = if self.calls.contains_key(id) {
            None
        } else {
            self.completed.get::<ToolRunItem>(id)?.map(|(_, run)| run)
        };
        let started = !self.calls.contains_key(id) && cached.is_none();
        if let Some(content) = value["content"].as_array() {
            let ids = content
                .iter()
                .filter(|block| block["type"] == "terminal")
                .filter_map(|block| block["terminalId"].as_str().map(str::to_owned))
                .take(32)
                .collect::<Vec<_>>();
            if !ids.is_empty() {
                self.terminal_bindings.insert(id.into(), ids);
            }
        }
        if started && self.calls.len() >= 512 {
            return Err(error("ACP tool report budget exceeded"));
        }
        if let Some(cached) = cached {
            self.calls.insert(id.to_string(), cached);
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
        if let Some(locations) = value.get("locations").and_then(Value::as_array) {
            let paths = locations
                .iter()
                .filter_map(|location| location["path"].as_str())
                .take(64)
                .map(|path| bounded(path, 4096))
                .collect::<Vec<_>>();
            run.capabilities.resource_keys =
                paths.iter().map(|path| format!("file:{path}")).collect();
            if run.arguments.is_none() && !paths.is_empty() {
                run.arguments = Some(json!({"paths":paths}).to_string());
            }
        }
        if let Some(content) = value.get("content") {
            let diffs = content
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "diff")
                .take(32)
                .filter_map(|block| {
                    let path = block["path"].as_str()?;
                    let new = block["newText"].as_str()?;
                    let old = block["oldText"].as_str().unwrap_or_default();
                    // ACP supplies full replacement text; render bounded hunks
                    // through the same diff projection as ordinary file edits.
                    Some(nexa_core::tools::diff_stats::text_diff_artifact(
                        path,
                        if block["oldText"].is_null() {
                            "create"
                        } else {
                            "edit"
                        },
                        old,
                        new,
                    ))
                })
                .collect::<Vec<_>>();
            if !diffs.is_empty() {
                run.artifacts =
                    Some(json!({"kind":"fileChange","diffs":diffs,"providerExecuted":true}));
                run.render_kind = ToolRenderKind::FileChange;
                run.capabilities.render_kind = run.render_kind;
            }
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
                    Some("terminal") => None,
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            run.content = Some(bounded(&text, 16000));
        }
        if run.content.as_deref().is_none_or(str::is_empty) {
            if let Some(output) = value.get("rawOutput").filter(|value| !value.is_null()) {
                run.content = Some(bounded(
                    output
                        .as_str()
                        .or_else(|| output["formatted_output"].as_str())
                        .unwrap_or(&output.to_string()),
                    16000,
                ));
            }
        }
        if let Some(ids) = self.terminal_bindings.get(id) {
            if let Some(text) = terminal_text(terminals, ids)? {
                run.content = Some(text);
            }
            run.render_kind = ToolRenderKind::CommandExecution;
            run.capabilities.render_kind = run.render_kind;
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
        tx.send(event).await.map_err(error)?;
        if complete {
            self.completed.insert(id, "", run)?;
            self.calls.remove(id);
            self.terminal_bindings.remove(id);
        }
        Ok(())
    }

    pub(super) async fn refresh_terminals(
        &mut self,
        tx: &mpsc::Sender<AgentEvent>,
        terminals: &super::terminals::Terminals,
    ) -> Result<()> {
        for (call, ids) in &self.terminal_bindings {
            if let Some(run) = self.calls.get_mut(call) {
                if let Some(text) = terminal_text(terminals, ids)? {
                    if run.content.as_deref() != Some(&text) {
                        run.content = Some(text);
                        tx.send(AgentEvent::ToolRunUpdated { run: run.clone() })
                            .await
                            .map_err(error)?;
                    }
                }
            }
        }
        Ok(())
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

fn terminal_text(
    terminals: &super::terminals::Terminals,
    ids: &[String],
) -> Result<Option<String>> {
    let mut texts = Vec::new();
    for id in ids {
        if let Some(snapshot) = terminals.snapshot(id)? {
            let mut text = snapshot["output"].as_str().unwrap_or_default().to_string();
            if snapshot["truncated"] == true {
                text.push_str("\n[Earlier output was truncated]");
            }
            if let Some(status) = snapshot.get("exitStatus") {
                text.push_str(&format!("\nExit: {status}"));
            }
            if !text.is_empty() {
                texts.push(text);
            }
        }
    }
    Ok((!texts.is_empty()).then(|| bounded(&texts.join("\n\n"), 16000)))
}
