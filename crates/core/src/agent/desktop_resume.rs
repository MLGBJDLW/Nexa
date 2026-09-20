//! Restore desktop evidence obligations from host-owned continuation state.
//! Prompt text and model-provided JSON are never a checkpoint authority.

use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::activity::{
    ActivityEvent, ActivityEventKind, ActivityRecord, ActivityState, ActivitySurface,
};
use crate::db::Database;
use crate::error::CoreError;
use crate::workflow_ir::WorkflowIr;

/// Restore host-owned desktop completion obligations for an existing task turn.
/// This never reads checkpoint claims from user/model text or grants input rights.
pub fn restore_pending_desktop_evidence(
    db: &Database,
    conversation_id: Option<&str>,
    turn_id: Option<&str>,
    current: &mut Option<WorkflowIr>,
    plan: &crate::intelligence::AgentTaskPlan,
    profile: &crate::quality_profile::ResolvedOrchestrationProfile,
    nexus_enabled: bool,
) -> Result<(), CoreError> {
    if let Some(saved) = load_desktop_resume_workflow(db, conversation_id, turn_id)? {
        if saved.desktop_evidence_pending()
            || saved.completion_contract.desktop_terminal_closure_evidence
        {
            crate::workflow_ir::ensure_runtime_desktop_observation_gate(
                current,
                plan,
                profile,
                nexus_enabled,
            )
            .map_err(CoreError::InvalidInput)?;
        }
        if let Some(current) = current.as_mut() {
            current.restore_desktop_checkpoint(&saved);
        }
    }
    Ok(())
}

fn invalid(detail: &str) -> CoreError {
    CoreError::InvalidInput(format!(
        "Cannot restore desktop evidence checkpoint: {detail}"
    ))
}

fn workflow(value: Option<&Value>) -> Result<Option<WorkflowIr>, CoreError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let workflow: WorkflowIr = serde_json::from_value(value.clone())
        .map_err(|_| invalid("saved workflow is malformed"))?;
    workflow
        .validate()
        .map_err(|_| invalid("saved workflow is invalid"))?;
    for target in &workflow.checkpoint.desktop_observation_targets {
        if target.window_id == 0
            || target
                .target_identity
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
            || target
                .consumed_observation_id
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err(invalid("saved desktop target identity is invalid"));
        }
    }
    Ok(Some(workflow))
}

/// The current run's plan covers interaction-response continuations and user
/// stops. A formally launched checkpoint can carry a newer live-turn snapshot.
pub(super) fn load_desktop_resume_workflow(
    db: &Database,
    conversation_id: Option<&str>,
    turn_id: Option<&str>,
) -> Result<Option<WorkflowIr>, CoreError> {
    let (Some(conversation_id), Some(turn_id)) = (conversation_id, turn_id) else {
        return Ok(None);
    };
    let Some(run) = db.get_agent_task_run_by_turn(turn_id)? else {
        return Ok(None);
    };
    if run.conversation_id != conversation_id {
        return Err(invalid("task turn belongs to another conversation"));
    }
    let mut selected = workflow(run.plan.as_ref().and_then(|plan| plan.get("workflowIr")))?;
    let launched = {
        let conn = db.conn();
        conn.query_row(
            "SELECT checkpoint.id, checkpoint.state_json, checkpoint.resume_prompt,
                    checkpoint.launch_idempotency_key, response.content, response.artifacts_json,
                    response.conversation_id, response.role
             FROM task_resume_checkpoints checkpoint
             LEFT JOIN messages response ON response.id = checkpoint.response_message_id
             WHERE checkpoint.run_id = ?1 AND checkpoint.response_message_id IS NOT NULL
             ORDER BY checkpoint.rowid DESC LIMIT 1",
            [&run.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()?
    };
    if let Some((id, state, prompt, launch_key, response, artifacts, owner, role)) = launched {
        let artifacts: Value = artifacts
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok())
            .ok_or_else(|| invalid("checkpoint continuation metadata is missing"))?;
        if launch_key
            .as_deref()
            .is_none_or(|key| key.trim().is_empty())
            || response.as_deref() != Some(prompt.as_str())
            || owner.as_deref() != Some(conversation_id)
            || role.as_deref() != Some("user")
            || artifacts.get("kind").and_then(Value::as_str) != Some("checkpointContinuation")
            || artifacts.get("version").and_then(Value::as_u64) != Some(1)
            || artifacts.get("checkpointId").and_then(Value::as_str) != Some(id.as_str())
        {
            return Err(invalid(
                "checkpoint continuation does not match its durable launch",
            ));
        }
        let state: Value = serde_json::from_str(&state)
            .map_err(|_| invalid("checkpoint state is not valid JSON"))?;
        if state.pointer("/run/id").and_then(Value::as_str) != Some(run.id.as_str())
            || state.pointer("/run/turnId").and_then(Value::as_str) != Some(turn_id)
            || state.pointer("/run/conversationId").and_then(Value::as_str) != Some(conversation_id)
        {
            return Err(invalid("checkpoint state belongs to another task turn"));
        }
        let saved = state
            .pointer("/liveTurnState/workflowIr")
            .filter(|value| !value.is_null())
            .or_else(|| state.pointer("/run/plan/workflowIr"));
        if let Some(candidate) = workflow(saved)? {
            // Restoring preserves the revision floor. A later reconciliation
            // persisted in run.plan must not resurrect the old pending targets.
            if selected
                .as_ref()
                .is_none_or(|current| candidate.checkpoint.revision >= current.checkpoint.revision)
            {
                selected = Some(candidate);
            }
        }
    }
    if let Some(saved) = selected.as_mut() {
        reconcile_late_desktop_closures(db, conversation_id, turn_id, saved)?;
    }
    Ok(selected.filter(|workflow| {
        workflow.desktop_evidence_pending()
            || workflow
                .completion_contract
                .desktop_terminal_closure_evidence
    }))
}

// A committed native worker can finish after the async turn was stopped and its
// checkpoint was saved. Only its exact durable terminal receipt can discharge
// that saved obligation; a resumed prompt cannot grant closure permission.
fn reconcile_late_desktop_closures(
    db: &Database,
    conversation_id: &str,
    turn_id: &str,
    saved: &mut WorkflowIr,
) -> Result<(), CoreError> {
    if !saved.completion_contract.desktop_terminal_closure_evidence
        || saved.checkpoint.desktop_observation_targets.is_empty()
    {
        return Ok(());
    }
    let rows = {
        let conn = db.conn();
        let mut statement = conn.prepare(
            "SELECT record.activity_id, record.record_json, event.seq, event.event_json
             FROM activity_records record
             JOIN activity_events event ON event.activity_id = record.activity_id
             WHERE record.conversation_id = ?1 AND record.state = 'completed'
               AND event.kind = 'completed'
               AND CASE WHEN json_valid(record.record_json)
                   THEN json_extract(record.record_json, '$.turnId') END = ?2",
        )?;
        let rows = statement.query_map([conversation_id, turn_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (activity_id, record, sequence, event) in rows {
        let (Ok(record), Ok(event)) = (
            serde_json::from_str::<ActivityRecord>(&record),
            serde_json::from_str::<ActivityEvent>(&event),
        ) else {
            continue;
        };
        if record.activity_id != activity_id
            || record.surface != ActivitySurface::Desktop
            || record.owner_tool != "computer_control"
            || record.state != ActivityState::Completed
            || record.conversation_id.as_deref() != Some(conversation_id)
            || record.turn_id.as_deref() != Some(turn_id)
            || event.activity_id != activity_id
            || event.kind != ActivityEventKind::Completed
            || event.seq != sequence
            || event.seq != record.last_event_seq
            || event.payload.get("state").and_then(Value::as_str) != Some("completed")
        {
            continue;
        }
        let Some(detail) = event.payload.get("detail") else {
            continue;
        };
        let identity = detail.get("targetIdentity").and_then(Value::as_str);
        let token = detail.get("consumedObservationId").and_then(Value::as_str);
        let window = detail.get("windowId").and_then(Value::as_u64);
        if detail.get("actionReceiptId").and_then(Value::as_str) != Some(activity_id.as_str())
            || detail
                .pointer("/terminalWindowReceipt/actionReceiptId")
                .and_then(Value::as_str)
                != Some(activity_id.as_str())
            || identity.is_none_or(|value| value.trim().is_empty())
            || token.is_none_or(|value| {
                value.trim().is_empty() || value == "<observation-token-redacted>"
            })
            || !saved
                .checkpoint
                .desktop_observation_targets
                .iter()
                .any(|target| {
                    Some(target.window_id) == window
                        && target.target_identity.as_deref() == identity
                        && target.consumed_observation_id.as_deref() == token
                })
        {
            continue;
        }
        let artifacts = serde_json::json!({"artifacts":{"kind":"computerControl"}, "data":detail});
        saved.observe_tool_result_with_arguments(
            &activity_id,
            "computer_control",
            None,
            false,
            Some(&artifacts),
            "Reconciled the native worker's durable target-bound terminal receipt after pause.",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
