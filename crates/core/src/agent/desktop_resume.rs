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

pub(super) struct ExecutionResumeState {
    pub(super) workflow: WorkflowIr,
    pub(super) plan: Option<crate::intelligence::AgentTaskPlan>,
    pub(super) completed_tool_rounds: u32,
    pub(super) max_iterations: Option<u32>,
}

fn saved_plan(value: Option<&Value>) -> Option<crate::intelligence::AgentTaskPlan> {
    value
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .filter(|plan: &crate::intelligence::AgentTaskPlan| plan.version == 1)
}

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
            || target
                .observation_not_before_ms
                .is_some_and(|value| value <= 0)
        {
            return Err(invalid("saved desktop target identity is invalid"));
        }
    }
    Ok(Some(workflow))
}

/// The current run's plan covers interaction-response continuations and user
/// stops. A formally launched checkpoint can carry a newer live-turn snapshot.
pub(super) fn load_execution_resume_state(
    db: &Database,
    conversation_id: Option<&str>,
    turn_id: Option<&str>,
) -> Result<Option<ExecutionResumeState>, CoreError> {
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
    let mut selected_plan = saved_plan(run.plan.as_ref());
    let mut completed_tool_rounds = run
        .plan
        .as_ref()
        .and_then(|value| value.get("completedToolRounds"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32;
    let mut max_iterations = run
        .plan
        .as_ref()
        .and_then(|value| value.get("maxIterations"))
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());

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
        if let Some(count) = state
            .pointer("/liveTurnState/completedToolRounds")
            .and_then(Value::as_u64)
        {
            completed_tool_rounds =
                completed_tool_rounds.max(count.min(u64::from(u32::MAX)) as u32);
        }
        if let Some(limit) = state
            .pointer("/liveTurnState/maxIterations")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
        {
            max_iterations = Some(max_iterations.map_or(limit, |prior| prior.min(limit)));
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
                if let Some(current) = selected.as_ref() {
                    if current.id != candidate.id {
                        return Err(invalid("checkpoint workflow identity changed"));
                    }
                }
                selected_plan = saved_plan(state.pointer("/liveTurnState/taskPlan"))
                    .or_else(|| saved_plan(state.pointer("/run/plan")))
                    .or(selected_plan);
                selected = Some(candidate);
            }
        }
    }
    if let Some(saved) = selected.as_mut() {
        reconcile_late_desktop_receipts(db, conversation_id, turn_id, saved)?;
    }
    Ok(selected.map(|workflow| ExecutionResumeState {
        workflow,
        plan: selected_plan,
        completed_tool_rounds,
        max_iterations,
    }))
}

pub(super) fn load_desktop_resume_workflow(
    db: &Database,
    conversation: Option<&str>,
    turn: Option<&str>,
) -> Result<Option<WorkflowIr>, CoreError> {
    Ok(load_execution_resume_state(db, conversation, turn)?
        .map(|state| state.workflow)
        .filter(|workflow| {
            workflow.desktop_evidence_pending()
                || workflow
                    .completion_contract
                    .desktop_terminal_closure_evidence
        }))
}

/// Already paid reconnaissance is never launched again just because its parent
/// was interrupted. Late completed receipts win; uncertain attempts remain a
/// failed advisory with a parent takeover, not fabricated success.
pub(super) fn reconcile_resumed_reconnaissance(
    db: &Database,
    turn: Option<&str>,
    workflow: &mut WorkflowIr,
) -> Result<(), CoreError> {
    use crate::workflow_ir::{
        WorkflowDeliverable, WorkflowNodeOrigin, WorkflowNodeRequirement, WorkflowNodeStatus,
    };
    let Some(run) = turn
        .map(|turn| db.get_agent_task_run_by_turn(turn))
        .transpose()?
        .flatten()
    else {
        return Ok(());
    };
    let receipts = db.list_agent_subtask_runs(&run.id)?;
    let pending = workflow
        .nodes
        .iter()
        .filter(|node| {
            node.origin == WorkflowNodeOrigin::RuntimeReconnaissance
                && node.requirement == WorkflowNodeRequirement::Advisory
                && node.status != WorkflowNodeStatus::Succeeded
        })
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    for id in pending {
        let matching = receipts
            .iter()
            .filter(|row| {
                row.input
                    .as_ref()
                    .and_then(|input| input.get("workerId"))
                    .and_then(Value::as_str)
                    == Some(&id)
            })
            .collect::<Vec<_>>();
        let receipt = matching.last().copied();
        let node = workflow
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .unwrap();
        // An older checkpoint may precede admission while the durable worker
        // receipt already exists. Receipt ownership outranks its stale counter.
        node.attempts = node
            .attempts
            .max(u8::try_from(matching.len()).unwrap_or(u8::MAX));
        if node.attempts == 0 {
            continue;
        }
        if let Some(result) = receipt
            .filter(|row| row.status == "completed")
            .and_then(|row| row.output.as_ref())
            .and_then(|output| output.get("run"))
            .filter(|result| result.get("isError").and_then(Value::as_bool) == Some(false))
        {
            if let Some(node) = workflow.nodes.iter_mut().find(|node| node.id == id) {
                node.status = WorkflowNodeStatus::Running;
            }
            workflow.apply_reconnaissance_batch_result(
                &[id],
                Some(&serde_json::json!({"runs":[result]})),
                false,
                "Recovered completed reconnaissance",
            );
        } else if let Some(node) = workflow.nodes.iter_mut().find(|node| node.id == id) {
            node.status = WorkflowNodeStatus::Failed;
            node.deliverable.get_or_insert_with(|| WorkflowDeliverable { claims: Vec::new(), evidence: Vec::new(), files_touched: Vec::new(), tests: Vec::new(), uncertainties: vec!["The prior reconnaissance was interrupted; its effect was not replayed. Parent investigation may continue while its durable receipt remains available.".into()] });
        }
    }
    workflow.refresh_checkpoint();
    Ok(())
}

// A committed native worker can finish after the async turn was stopped and its
// checkpoint was saved. A modal receipt transfers the obligation to the exact
// owner; only an originally authorized terminal closure can discharge it.
fn reconcile_late_desktop_receipts(
    db: &Database,
    conversation_id: &str,
    turn_id: &str,
    saved: &mut WorkflowIr,
) -> Result<(), CoreError> {
    if saved.checkpoint.desktop_observation_targets.is_empty() {
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
        let matching_modal_receipt = detail
            .pointer("/modalOwnerHandoffReceipt/actionReceiptId")
            .and_then(Value::as_str)
            == Some(activity_id.as_str());
        let matching_terminal_receipt = saved.completion_contract.desktop_terminal_closure_evidence
            && detail
                .pointer("/terminalWindowReceipt/actionReceiptId")
                .and_then(Value::as_str)
                == Some(activity_id.as_str());
        if detail.get("actionReceiptId").and_then(Value::as_str) != Some(activity_id.as_str())
            || !(matching_modal_receipt || matching_terminal_receipt)
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
        let verified_handoff =
            crate::workflow_ir::verified_desktop_modal_owner_handoff(None, Some(&artifacts))
                .is_some();
        let verified_closure = matching_terminal_receipt
            && detail
                .get("modalOwnerHandoffReceipt")
                .is_none_or(Value::is_null)
            && crate::workflow_ir::verified_desktop_terminal_closure(None, Some(&artifacts))
                .is_some();
        if !verified_handoff && !verified_closure {
            continue;
        }
        saved.observe_tool_result_with_arguments(
            &activity_id,
            "computer_control",
            None,
            false,
            Some(&artifacts),
            "Reconciled the native worker's durable target-bound completion receipt after pause.",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
