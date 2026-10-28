//! Recover legacy local verification debt only from complete host callback
//! ledgers. Provider-managed native tools and missing evidence stay opaque.
use crate::{
    db::Database,
    error::CoreError,
    workflow_ir::{BrowserObservationTarget, DesktopObservationTarget},
};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Default, serde::Serialize)]
pub(crate) struct LocalRecoveryTargets {
    pub desktop: Vec<DesktopObservationTarget>,
    pub browser: Vec<BrowserObservationTarget>,
}

impl LocalRecoveryTargets {
    pub fn is_empty(&self) -> bool {
        self.desktop.is_empty() && self.browser.is_empty()
    }
}

fn host_callback_runtime(provider: &str) -> bool {
    // These adapters disable ambient effect tools and execute Nexa callbacks.
    // ACP owns native tools and must never be inferred safe from this ledger.
    matches!(provider, "openai_codex" | "github_copilot")
}

fn canonical_arguments_match(
    tool: &str,
    canonical: Option<&Value>,
    audited: Option<&Value>,
) -> bool {
    let (Some(canonical), Some(audited)) = (
        canonical.and_then(Value::as_str),
        audited.and_then(Value::as_str),
    ) else {
        return false;
    };
    let (Ok(canonical), Ok(audited)) = (
        serde_json::from_str::<Value>(canonical),
        serde_json::from_str::<Value>(audited),
    ) else {
        return false;
    };
    canonical == audited
        || crate::tool_argument_projection::audit_safe_arguments(tool, &canonical) == audited
}

fn complete_callback_ledger(
    conn: &Connection,
    conversation: &str,
    turn: Option<&str>,
    events: &[(String, Value)],
) -> Result<Option<Vec<Value>>, CoreError> {
    let mut started = BTreeMap::new();
    let mut completed = BTreeMap::new();
    let mut ordered = Vec::new();
    for (phase, run) in events {
        let Some(id) = run
            .get("callId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            return Ok(None);
        };
        if run.get("providerExecuted").and_then(Value::as_bool) != Some(false) {
            return Ok(None);
        }
        if phase == "started" {
            if started.insert(id.to_string(), run).is_some() {
                return Ok(None);
            }
        } else {
            if !started.contains_key(id)
                || !matches!(
                    run.get("status").and_then(Value::as_str),
                    Some("completed" | "failed" | "declined" | "cancelled")
                )
                || completed.insert(id.to_string(), run).is_some()
            {
                return Ok(None);
            }
            ordered.push(run.clone());
        }
    }
    if started.is_empty() || started.len() != completed.len() || started.keys().ne(completed.keys())
    {
        return Ok(None);
    }
    if started.iter().any(|(id, start)| {
        let end = completed[id];
        start.get("toolName") != end.get("toolName")
            || start.get("arguments") != end.get("arguments")
    }) {
        return Ok(None);
    }
    let mut canonical = BTreeMap::new();
    let mut query = conn.prepare("SELECT message.tool_calls_json,message.sort_order,
        (SELECT MIN(next.sort_order) FROM messages next WHERE next.conversation_id=message.conversation_id AND next.sort_order>message.sort_order AND next.role IN ('assistant','user'))
        FROM messages message WHERE message.conversation_id=?1 AND message.role='assistant' AND message.tool_calls_json IS NOT NULL
        AND (?2 IS NULL OR json_extract(message.artifacts_json,'$.turnId')=?2)")?;
    let rows = query.query_map(rusqlite::params![conversation, turn], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<i64>>(2)?,
        ))
    })?;
    for row in rows {
        let (calls, order, next) = row?;
        let calls: Value = serde_json::from_str(&calls)?;
        for call in calls.as_array().into_iter().flatten() {
            let Some(id) = call.get("id").and_then(Value::as_str) else {
                return Ok(None);
            };
            if started.get(id).is_none_or(|run| {
                call.get("name") != run.get("toolName")
                    || !canonical_arguments_match(
                        call.get("name").and_then(Value::as_str).unwrap_or_default(),
                        call.get("arguments"),
                        run.get("arguments"),
                    )
            }) || canonical
                .insert(id.to_string(), (order, next, call.clone()))
                .is_some()
            {
                return Ok(None);
            }
        }
    }
    if canonical.len() != started.len() || !started.keys().all(|id| canonical.contains_key(id)) {
        return Ok(None);
    }
    let mut results = HashSet::new();
    let mut query = conn.prepare("SELECT tool_call_id,sort_order FROM messages WHERE conversation_id=?1 AND role='tool' AND tool_call_id IS NOT NULL")?;
    for row in query.query_map([conversation], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })? {
        let (id, order) = row?;
        if canonical
            .get(&id)
            .is_some_and(|(start, next, _)| order > *start && next.is_none_or(|end| order < end))
        {
            results.insert(id);
        }
    }
    if !canonical.keys().all(|id| results.contains(id)) {
        return Ok(None);
    }
    // Dispatch receives canonical arguments while durable callbacks contain the
    // mandatory audit projection. Recover target facts from the canonical call
    // only after proving the call ID, tool name, arguments and result ordering.
    for run in &mut ordered {
        let call = &canonical[run.get("callId").and_then(Value::as_str).unwrap()].2;
        run["arguments"] = call["arguments"].clone();
    }
    Ok(Some(ordered))
}

fn pending_targets(receipts: &[Value]) -> Option<LocalRecoveryTargets> {
    pending_targets_with_batches(receipts, &BTreeMap::new())
}

fn pending_targets_with_batches(
    receipts: &[Value],
    batches: &BTreeMap<String, LocalRecoveryTargets>,
) -> Option<LocalRecoveryTargets> {
    let mut targets = LocalRecoveryTargets::default();
    for run in receipts {
        if let Some(batch) = run
            .get("callId")
            .and_then(Value::as_str)
            .and_then(|id| batches.get(id))
        {
            for target in &batch.desktop {
                if !targets.desktop.contains(target) {
                    targets.desktop.push(target.clone());
                }
            }
            for target in &batch.browser {
                if !targets.browser.contains(target) {
                    targets.browser.push(target.clone());
                }
            }
            continue;
        }
        let tool = run.get("toolName")?.as_str()?;
        let args = run.get("arguments").and_then(Value::as_str);
        // Match the public callback ledger before resolving its backend facts.
        // Routing changes neither the original protocol identity nor authority.
        let route = crate::tools::unified_control::route_receipt_str(tool, args);
        let routed_arguments = route.as_ref().map(|route| route.arguments.to_string());
        let tool = route.as_ref().map_or(tool, |route| route.tool_name);
        let args = routed_arguments.as_deref().or(args);
        let artifacts = run.get("artifacts");
        let is_error = run
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(run.get("status")?.as_str()? != "completed");
        let no_effect = artifacts
            .and_then(|value| value.get("sideEffect"))
            .and_then(Value::as_str)
            == Some("not_started");
        let read_only = run
            .pointer("/capabilities/readOnly")
            .and_then(Value::as_bool)
            == Some(true);
        let file_only = matches!(tool, "edit_file" | "create_file")
            && run
                .pointer("/capabilities/resourceKeys")
                .and_then(Value::as_array)
                .is_some_and(|keys| {
                    !keys.is_empty()
                        && keys
                            .iter()
                            .all(|key| key.as_str().is_some_and(|key| key.starts_with("file:")))
                });
        let browser_target = (tool == "browser_session")
            .then(|| crate::workflow_ir::browser_control_target(args, artifacts))
            .flatten();
        let local_bookkeeping = matches!(tool, "update_goal" | "update_plan" | "update_scratchpad");
        let presentation_only = tool == "browser_session"
            && args
                .and_then(|args| serde_json::from_str::<Value>(args).ok())
                .and_then(|args| {
                    args.get("action")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .is_some_and(|action| {
                    matches!(action.as_str(), "show_workspace" | "hide_workspace")
                });
        // A cancelled/failed native command may still own an OS effect. A
        // settled file-edit mismatch cannot create desktop input uncertainty;
        // its failed task/result remains visible for ordinary file inspection.
        if is_error
            && run.get("status").and_then(Value::as_str) != Some("declined")
            && !no_effect
            && !read_only
            && !file_only
            && !local_bookkeeping
            && !presentation_only
            && tool != "computer_control"
            && browser_target.is_none()
        {
            return None;
        }
        if crate::workflow_ir::tool_result_effect_may_have_occurred(artifacts)
            && tool != "computer_control"
            && browser_target.is_none()
        {
            return None;
        }
        if crate::workflow_ir::tool_result_requires_desktop_observation(tool, is_error, artifacts) {
            let target = crate::workflow_ir::desktop_control_target(args, artifacts)?;
            if target.target_identity.is_none() || target.consumed_observation_id.is_none() {
                return None;
            }
            targets.desktop.retain(|prior| {
                prior.window_id != target.window_id
                    || prior.target_identity != target.target_identity
            });
            targets.desktop.push(target);
        }
        if let Some(mut target) = browser_target {
            // Canonical event ordering proves whether a later historical read
            // followed this action. Once returned, require a newly captured
            // observation after the recovery was imported instead.
            target.observation_not_before_ms = None;
            targets.browser.retain(|prior| {
                prior.session_id != target.session_id || prior.tab_id != target.tab_id
            });
            targets.browser.push(target);
        }
        if !is_error && tool == "browser_session" {
            targets.browser.retain(|target| {
                !crate::workflow_ir::browser_observation_matches(target, args, artifacts)
            });
        }
        if !is_error
            && ((tool == "computer_observe"
                && crate::workflow_ir::is_verified_desktop_observation(args, artifacts))
                || (tool == "computer_control"
                    && crate::workflow_ir::verified_post_action_desktop_observation(
                        args, artifacts,
                    )))
        {
            let data = artifacts?.get("data").or(artifacts)?;
            let observation = if tool == "computer_control" {
                data.get("observation").unwrap_or(data)
            } else {
                data
            };
            targets.desktop.retain(|target| {
                !crate::workflow_ir::desktop_observation_matches(target, observation)
            });
        }
    }
    for target in &mut targets.browser {
        target.observation_not_before_ms = Some(chrono::Utc::now().timestamp_millis());
    }
    Some(targets)
}

/// A failed aggregate is settled bookkeeping only when every requested worker
/// is bound through its exact batch, lifecycle terminal result and canonical
/// child callbacks. Do not exempt a delegation merely because of its tool name.
fn completed_batch_targets(
    conn: &Connection,
    run_id: &str,
    conversation: &str,
    turn: &str,
    callback: &Value,
) -> Result<Option<LocalRecoveryTargets>, CoreError> {
    let Some(call_id) = callback.get("callId").and_then(Value::as_str) else {
        return Ok(None);
    };
    let Some(artifacts) = callback.get("artifacts") else {
        return Ok(None);
    };
    let arguments = callback
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    let Some(tasks) = arguments
        .as_ref()
        .and_then(|args| args.get("tasks"))
        .and_then(Value::as_array)
        .filter(|tasks| !tasks.is_empty())
    else {
        return Ok(None);
    };
    let Some(runs) = artifacts.get("runs").and_then(Value::as_array) else {
        return Ok(None);
    };
    let Some(workers) = artifacts.get("lifecycleWorkers").and_then(Value::as_array) else {
        return Ok(None);
    };
    if artifacts.get("kind").and_then(Value::as_str) != Some("subagent_batch_result")
        || artifacts.get("batchId").and_then(Value::as_str)
            != Some(format!("{run_id}:{call_id}").as_str())
        || [
            "continuingWorkers",
            "unsettledWorkers",
            "pendingAtPolicyCompletion",
        ]
        .iter()
        .any(|key| artifacts.get(key).and_then(Value::as_u64) != Some(0))
        || runs.len() != tasks.len()
        || workers.len() != tasks.len()
        || artifacts
            .get("completedRuns")
            .and_then(Value::as_u64)
            .zip(artifacts.get("failedRuns").and_then(Value::as_u64))
            .is_none_or(|(completed, failed)| {
                completed.checked_add(failed) != Some(tasks.len() as u64)
            })
        || crate::workflow_ir::tool_result_effect_may_have_occurred(Some(artifacts))
    {
        return Ok(None);
    }
    let mut identities = HashSet::new();
    let mut agents = HashSet::new();
    let mut result = LocalRecoveryTargets::default();
    for (index, task) in tasks.iter().enumerate() {
        let Some(task_text) = task
            .get("task")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return Ok(None);
        };
        let id = task
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{call_id}-{}", index + 1));
        if !identities.insert(id.clone()) {
            return Ok(None);
        }
        let Some(worker) = workers
            .iter()
            .find(|worker| worker.get("workerId").and_then(Value::as_str) == Some(id.as_str()))
        else {
            return Ok(None);
        };
        let Some(agent) = worker
            .get("agentId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            return Ok(None);
        };
        if !agents.insert(agent) {
            return Ok(None);
        }
        let Some(child) = runs
            .iter()
            .find(|run| run.get("id").and_then(Value::as_str) == Some(id.as_str()))
        else {
            return Ok(None);
        };
        let session = task
            .get("task_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or(&id);
        if child.get("sessionId").and_then(Value::as_str) != Some(session)
            || Some(task_text) != child.get("task").and_then(Value::as_str)
        {
            return Ok(None);
        }
        let terminal: Option<(String,String)> = conn.query_row(
            "SELECT record.record_json,event.event_json FROM activity_records record JOIN activity_events event ON event.activity_id=record.activity_id AND event.seq=json_extract(record.record_json,'$.lastEventSeq') WHERE record.activity_id=?1 AND record.task_run_id=?2 AND record.conversation_id=?3 AND record.state IN ('completed','failed','cancelled')",
            rusqlite::params![agent,run_id,conversation], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        let Some((record, event)) = terminal else {
            return Ok(None);
        };
        let record: crate::activity::ActivityRecord = serde_json::from_str(&record)?;
        let event: crate::activity::ActivityEvent = serde_json::from_str(&event)?;
        if record.activity_id != agent
            || record.owner_tool != "spawn_subagent"
            || record.task_run_id.as_deref() != Some(run_id)
            || record.conversation_id.as_deref() != Some(conversation)
            || record.turn_id.as_deref() != Some(turn)
            || !record.state.is_terminal()
            || event.activity_id != agent
            || event.seq != record.last_event_seq
            || event.payload.get("agentId").and_then(Value::as_str) != Some(agent)
            || !matches!(
                event.payload.get("subagentEvent").and_then(Value::as_str),
                Some("completed" | "failed" | "cancelled")
            )
            || event.payload.pointer("/detail/result") != Some(child)
        {
            return Ok(None);
        }
        let mut query = conn.prepare("SELECT id,json_extract(output_json,'$.run') FROM agent_subtask_runs WHERE parent_run_id=?1 AND status IN ('completed','failed','cancelled','timed_out') AND json_extract(input_json,'$.callLabel')=?2 AND json_extract(input_json,'$.workerId')=?2 AND json_extract(output_json,'$.run.id')=?2 AND json_extract(output_json,'$.run.sessionId')=?3")?;
        let rows = query
            .query_map(rusqlite::params![run_id, id, session], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.len() != 1 || serde_json::from_str::<Value>(&rows[0].1)? != *child {
            return Ok(None);
        }
        let Some(targets) = legacy_subtask_targets(conn, run_id, &rows[0].0)? else {
            return Ok(None);
        };
        result.desktop.extend(targets.desktop);
        result.browser.extend(targets.browser);
    }
    Ok(Some(result))
}

pub(super) fn legacy_subtask_targets(
    conn: &Connection,
    run_id: &str,
    worker_id: &str,
) -> Result<Option<LocalRecoveryTargets>, CoreError> {
    let row = conn.query_row("SELECT parent.conversation_id,json_extract(worker.output_json,'$.run') FROM agent_subtask_runs worker JOIN agent_task_runs parent ON parent.id=worker.parent_run_id WHERE worker.id=?1 AND worker.parent_run_id=?2", rusqlite::params![worker_id,run_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?))).optional()?;
    let Some((parent, Some(raw))) = row else {
        return Ok(None);
    };
    let run: Value = serde_json::from_str(&raw)?;
    let Some(provider) = run
        .pointer("/runtimeIdentity/provider")
        .and_then(Value::as_str)
        .filter(|provider| host_callback_runtime(provider))
    else {
        return Ok(None);
    };
    let Some(session) = run.get("sessionId").and_then(Value::as_str) else {
        return Ok(None);
    };
    let identity = serde_json::to_vec(&(&parent, session, provider))?;
    let child = format!("subagent-{}", blake3::hash(&identity).to_hex());
    let belongs: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1 AND parent_conversation_id=?2 AND provider=?3)",rusqlite::params![child,parent,provider],|row|row.get(0))?;
    if !belongs {
        return Ok(None);
    }
    let Some(events) = run.get("toolEvents").and_then(Value::as_array) else {
        return Ok(None);
    };
    let events = events
        .iter()
        .filter_map(|event| match event.get("phase").and_then(Value::as_str) {
            Some("runStarted") => Some((
                "started".into(),
                event.get("run").cloned().unwrap_or(Value::Null),
            )),
            Some("runCompleted") => Some((
                "completed".into(),
                event.get("run").cloned().unwrap_or(Value::Null),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    Ok(complete_callback_ledger(conn, &child, None, &events)?
        .as_deref()
        .and_then(pending_targets))
}

fn native_run_targets(
    conn: &Connection,
    run_id: &str,
) -> Result<Option<LocalRecoveryTargets>, CoreError> {
    let (conversation, turn, provider) = conn.query_row(
        "SELECT conversation_id,turn_id,provider FROM agent_task_runs WHERE id=?1",
        [run_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )?;
    if !provider.as_deref().is_some_and(host_callback_runtime) {
        return Ok(None);
    }
    let mut query = conn.prepare("SELECT kind,json_extract(payload_json,'$.run') FROM agent_run_events WHERE run_id=?1 AND kind IN ('toolStarted','toolCompleted') ORDER BY event_seq")?;
    let events = query
        .query_map([run_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .map(|row| {
            let (kind, json) = row?;
            Ok((
                if kind == "toolStarted" {
                    "started"
                } else {
                    "completed"
                }
                .into(),
                serde_json::from_str(&json)?,
            ))
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let Some(receipts) = complete_callback_ledger(conn, &conversation, Some(&turn), &events)?
    else {
        return Ok(None);
    };
    let mut batches = BTreeMap::new();
    for callback in &receipts {
        if callback.get("toolName").and_then(Value::as_str) == Some("spawn_subagent_batch")
            && callback.pointer("/artifacts/kind").and_then(Value::as_str)
                == Some("subagent_batch_result")
        {
            let Some(targets) =
                completed_batch_targets(conn, run_id, &conversation, &turn, callback)?
            else {
                return Ok(None);
            };
            batches.insert(callback["callId"].as_str().unwrap().to_owned(), targets);
        }
    }
    Ok(pending_targets_with_batches(&receipts, &batches))
}

impl Database {
    pub(crate) fn native_run_recovery_targets(
        &self,
        run_id: &str,
    ) -> Result<Option<LocalRecoveryTargets>, CoreError> {
        native_run_targets(&self.conn(), run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{ConversationMessage, CreateConversationInput};
    use crate::llm::{Role, ToolCallRequest};
    use serde_json::json;

    #[test]
    fn public_audit_receipts_recover_exact_local_targets() {
        let desktop_arguments = crate::tool_argument_projection::audit_safe_arguments_string(
            "computer_use",
            r#"{"action":"type","target":{"kind":"window","window_id":42},"observation_id":"before","text":"private input"}"#,
        );
        let browser_arguments = crate::tool_argument_projection::audit_safe_arguments_string(
            "computer_use",
            r#"{"action":"type","target":{"kind":"browser","session_id":"session","tab_id":"tab"},"observation_id":"page-before","text":"private input"}"#,
        );
        let receipts = [
            json!({"callId":"desktop","toolName":"computer_use","arguments":desktop_arguments,"status":"failed","isError":true,
                "artifacts":{"code":"computer_action_uncertain","sideEffect":"may_have_occurred","data":{"windowId":42,"targetIdentity":"process-generation","consumedObservationId":"before"}}}),
            json!({"callId":"browser","toolName":"computer_use","arguments":browser_arguments,"status":"failed","isError":true,
                "artifacts":{"code":"browser_action_uncertain","sideEffect":"may_have_occurred"}}),
        ];
        let targets =
            pending_targets(&receipts).expect("audit arguments retain bound host effects");
        assert_eq!(targets.desktop.len(), 1);
        assert_eq!(targets.desktop[0].window_id, 42);
        assert_eq!(
            targets.desktop[0].target_identity.as_deref(),
            Some("process-generation")
        );
        assert_eq!(
            targets.desktop[0].consumed_observation_id.as_deref(),
            Some("before")
        );
        assert_eq!(targets.browser.len(), 1);
        assert_eq!(targets.browser[0].session_id, "session");
        assert_eq!(targets.browser[0].tab_id, "tab");
        assert_eq!(
            targets.browser[0].consumed_observation_id.as_deref(),
            Some("page-before")
        );
    }

    #[test]
    fn canonical_arguments_allow_only_exact_or_deterministic_audit_projection() {
        let canonical = json!(r#"{"action":"show_workspace","session_id":"s"}"#);
        let parsed: Value = serde_json::from_str(canonical.as_str().unwrap()).unwrap();
        let projected = json!(crate::tool_argument_projection::audit_safe_arguments(
            "browser_session",
            &parsed
        )
        .to_string());
        assert!(canonical_arguments_match(
            "browser_session",
            Some(&canonical),
            Some(&projected)
        ));
        let wrong_target = json!(r#"{"action":"show_workspace","session_id":"other"}"#);
        assert!(!canonical_arguments_match(
            "browser_session",
            Some(&wrong_target),
            Some(&projected)
        ));
        assert!(!canonical_arguments_match(
            "other_tool",
            Some(&canonical),
            Some(&projected)
        ));
    }

    #[test]
    fn failed_batch_recovery_requires_complete_exact_parent_lifecycle_and_child_binding() {
        for variant in [
            "complete",
            "known_desktop",
            "observed_desktop",
            "continuing",
            "unsettled",
            "wrong_batch",
            "wrong_parent",
            "wrong_lifecycle",
            "wrong_session",
            "child_gap",
            "unknown_child",
            "missing_lifecycle",
            "truncated_batch",
            "template_only",
            "unknown_parent",
        ] {
            // This fixture exercises the production SQL/ledger path without
            // unrelated migrations. The adjacent legacy test uses Database's
            // full schema and public conversation/subtask persistence APIs.
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE agent_task_runs(id TEXT,conversation_id TEXT,turn_id TEXT,provider TEXT);
                CREATE TABLE agent_subtask_runs(id TEXT,parent_run_id TEXT,status TEXT,input_json TEXT,output_json TEXT);
                CREATE TABLE conversations(id TEXT,parent_conversation_id TEXT,provider TEXT);
                CREATE TABLE messages(conversation_id TEXT,role TEXT,tool_calls_json TEXT,sort_order INTEGER,tool_call_id TEXT,artifacts_json TEXT);
                CREATE TABLE activity_records(activity_id TEXT,task_run_id TEXT,conversation_id TEXT,state TEXT,record_json TEXT);
                CREATE TABLE activity_events(activity_id TEXT,seq INTEGER,event_json TEXT);
                CREATE TABLE agent_run_events(run_id TEXT,kind TEXT,payload_json TEXT,event_seq INTEGER);
                INSERT INTO agent_task_runs VALUES('parent-run','parent','turn','openai_codex');").unwrap();
            let identity = serde_json::to_vec(&("parent", "worker", "openai_codex")).unwrap();
            let child_conversation = format!("subagent-{}", blake3::hash(&identity).to_hex());
            conn.execute(
                "INSERT INTO conversations VALUES(?1,'parent','openai_codex')",
                [&child_conversation],
            )
            .unwrap();
            let desktop = matches!(variant, "known_desktop" | "observed_desktop");
            let tool = if desktop {
                "computer_control"
            } else {
                "read_file"
            };
            let child_args = if desktop {
                r#"{"action":"click","window_id":42,"observation_id":"before"}"#
            } else {
                r#"{"path":"fixture.txt"}"#
            };
            let child_start = json!({"callId":"child-call","toolName":tool,"arguments":child_args,"providerExecuted":false,"status":"running"});
            let child_artifacts = if desktop {
                json!({"sideEffect":"may_have_occurred","data":{"windowId":42,"targetIdentity":"native42","consumedObservationId":"before"}})
            } else if variant == "unknown_child" {
                json!({"sideEffect":"may_have_occurred"})
            } else {
                json!({})
            };
            let child_end = json!({"callId":"child-call","toolName":tool,"arguments":child_args,"providerExecuted":false,"status":"completed","isError":false,"artifacts":child_artifacts});
            let child_events = if variant == "child_gap" {
                json!([{"phase":"runStarted","run":child_start}])
            } else {
                json!([{"phase":"runStarted","run":child_start},{"phase":"runCompleted","run":child_end}])
            };
            let child = json!({"id":"worker","sessionId":if variant == "wrong_session" {"other"} else {"worker"},"task":"inspect","status":"failed","isError":true,"runtimeIdentity":{"provider":"openai_codex","model":"fixture"},"toolEvents":child_events,"recoveryRequired":{"actionReconciliation":true,"message":"legacy verification debt"}});
            conn.execute(
                "INSERT INTO messages VALUES(?1,'assistant',?2,0,NULL,NULL)",
                rusqlite::params![
                    child_conversation,
                    json!([{"id":"child-call","name":tool,"arguments":child_args}]).to_string()
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages VALUES(?1,'tool',NULL,1,'child-call',NULL)",
                [&child_conversation],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO agent_subtask_runs VALUES('worker-row','parent-run','failed',?1,?2)",
                rusqlite::params![
                    json!({"callLabel":"worker","workerId":"worker"}).to_string(),
                    json!({"run":child}).to_string()
                ],
            )
            .unwrap();
            let activity_parent = if variant == "wrong_parent" {
                "other-parent"
            } else {
                "parent-run"
            };
            let record = json!({"activityId":"agent","sessionId":"agent","surface":"process","state":"failed","conversationId":"parent","turnId":"turn","taskRunId":activity_parent,"ownerTool":"spawn_subagent","startedAt":"2026-10-28T00:00:00Z","updatedAt":"2026-10-28T00:00:01Z","completedAt":"2026-10-28T00:00:01Z","lastEventSeq":1});
            conn.execute(
                "INSERT INTO activity_records VALUES('agent',?1,'parent','failed',?2)",
                rusqlite::params![activity_parent, record.to_string()],
            )
            .unwrap();
            if variant != "missing_lifecycle" {
                let event = json!({"activityId":"agent","seq":1,"timestamp":"2026-10-28T00:00:01Z","kind":"failed","payload":{"agentId":"agent","subagentEvent":"failed","detail":{"status":"failed","result":child}}});
                conn.execute(
                    "INSERT INTO activity_events VALUES('agent',1,?1)",
                    [event.to_string()],
                )
                .unwrap();
            }
            let args = if variant == "template_only" {
                json!({"workflow_template":"investigate","batch_goal":"inspect"})
            } else {
                json!({"tasks":[{"id":"worker","task":"inspect"}]})
            }
            .to_string();
            let batch = json!({"kind":"subagent_batch_result","batchId":if variant == "wrong_batch" {"parent-run:other-call"} else {"parent-run:batch-call"},"continuingWorkers":if variant == "continuing" {1} else {0},"unsettledWorkers":if variant == "unsettled" {1} else {0},"pendingAtPolicyCompletion":0,"completedRuns":0,"failedRuns":1,"runs":if variant == "truncated_batch" {json!([])} else {json!([child])},"lifecycleWorkers":[{"agentId":if variant == "wrong_lifecycle" {"other-agent"} else {"agent"},"workerId":"worker"}]});
            let mut calls =
                vec![json!({"id":"batch-call","name":"spawn_subagent_batch","arguments":args})];
            let mut receipts = vec![
                (
                    "toolStarted",
                    json!({"callId":"batch-call","toolName":"spawn_subagent_batch","arguments":args,"providerExecuted":false,"status":"running"}),
                ),
                (
                    "toolCompleted",
                    json!({"callId":"batch-call","toolName":"spawn_subagent_batch","arguments":args,"providerExecuted":false,"status":"failed","isError":true,"artifacts":batch}),
                ),
            ];
            conn.execute(
                "INSERT INTO messages VALUES('parent','tool',NULL,1,'batch-call',NULL)",
                [],
            )
            .unwrap();
            if variant == "observed_desktop" || variant == "unknown_parent" {
                let (name, arguments, artifacts, failed) = if variant == "observed_desktop" {
                    (
                        "computer_observe",
                        r#"{"action":"capture_window","window_id":42}"#,
                        json!({"data":{"kind":"computerObservationReceipt","windowId":42,"targetIdentity":"native42","observationId":"after","screenshotHash":"pixels"}}),
                        false,
                    )
                } else {
                    (
                        "browser_session",
                        r#"{"action":"create_session"}"#,
                        json!({"kind":"toolContractError","code":"invalid_tool_request"}),
                        true,
                    )
                };
                calls.push(json!({"id":"other-call","name":name,"arguments":arguments}));
                receipts.extend([("toolStarted",json!({"callId":"other-call","toolName":name,"arguments":arguments,"providerExecuted":false,"status":"running"})),("toolCompleted",json!({"callId":"other-call","toolName":name,"arguments":arguments,"providerExecuted":false,"status":if failed {"failed"} else {"completed"},"isError":failed,"artifacts":artifacts}))]);
                conn.execute(
                    "INSERT INTO messages VALUES('parent','tool',NULL,2,'other-call',NULL)",
                    [],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO messages VALUES('parent','assistant',?1,0,NULL,?2)",
                rusqlite::params![
                    json!(calls).to_string(),
                    json!({"turnId":"turn"}).to_string()
                ],
            )
            .unwrap();
            for (index, (kind, receipt)) in receipts.iter().enumerate() {
                conn.execute(
                    "INSERT INTO agent_run_events VALUES('parent-run',?1,?2,?3)",
                    rusqlite::params![kind, json!({"run":receipt}).to_string(), index as i64 + 1],
                )
                .unwrap();
            }
            let recovered = native_run_targets(&conn, "parent-run").unwrap();
            assert_eq!(
                recovered.is_some(),
                matches!(variant, "complete" | "known_desktop" | "observed_desktop"),
                "{variant}"
            );
            if let Some(targets) = recovered {
                assert_eq!(
                    targets.desktop.len(),
                    usize::from(variant == "known_desktop"),
                    "{variant}"
                );
            }
        }
    }

    #[test]
    #[ignore = "explicit read-only audit of a selected local legacy recovery receipt"]
    fn audit_selected_legacy_recovery_read_only() {
        let path = std::env::var("NEXA_RECOVERY_AUDIT_DB").expect("audit database path");
        let run = std::env::var("NEXA_RECOVERY_AUDIT_RUN").expect("audit parent run");
        let worker = std::env::var("NEXA_RECOVERY_AUDIT_WORKER").expect("audit worker");
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let targets = legacy_subtask_targets(&conn, &run, &worker).unwrap();
        let parent_targets = native_run_targets(&conn, &run).unwrap();
        let (conversation, turn): (String, String) = conn
            .query_row(
                "SELECT conversation_id,turn_id FROM agent_task_runs WHERE id=?1",
                [&run],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let events = conn.prepare("SELECT kind,json_extract(payload_json,'$.run') FROM agent_run_events WHERE run_id=?1 AND kind IN ('toolStarted','toolCompleted') ORDER BY event_seq").unwrap()
            .query_map([&run], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).unwrap()
            .map(|row| { let (kind, raw) = row.unwrap(); (if kind == "toolStarted" {"started"} else {"completed"}.to_string(), serde_json::from_str::<Value>(&raw).unwrap()) }).collect::<Vec<_>>();
        let ledger = complete_callback_ledger(&conn, &conversation, Some(&turn), &events).unwrap();
        let blockers = ledger.as_ref().map(|receipts| receipts.iter().filter(|receipt| pending_targets(&[(*receipt).clone()]).is_none())
            .map(|receipt| json!({"tool":receipt.get("toolName"),"callId":receipt.get("callId"),"status":receipt.get("status"),"code":receipt.pointer("/artifacts/code"),"sideEffect":receipt.pointer("/artifacts/sideEffect")})).collect::<Vec<_>>());
        let report = json!({"readOnly":true,"workerId":worker,"parentRunId":run,
            "classification":match &targets {Some(targets) if targets.is_empty()=>"complete_host_callbacks_no_pending_desktop_actions",Some(_)=>"bound_local_desktop_targets",None=>"opaque_or_incomplete_evidence"},
            "pendingTargets":targets,"parentHostCallbacksComplete":parent_targets.is_some(),"parentCanonicalLedgerComplete":ledger.is_some(),"blockingParentCallbacks":blockers,"parentPendingTargets":parent_targets,"taskCompletionClaimed":false});
        if let Ok(path) = std::env::var("NEXA_RECOVERY_AUDIT_OUTPUT") {
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        println!("{report}");
        assert!(targets.is_some(), "selected legacy receipt remains opaque");
        // The parent may independently have opaque effects. Auditing a worker
        // must not claim that its complete ledger reconciles the whole parent.
    }

    #[test]
    fn legacy_recovery_requires_complete_canonical_host_callbacks() {
        for variant in [
            "complete",
            "missing_result",
            "old_result",
            "reordered_events",
            "truncated_pair",
            "uncertain_effect",
            "acp",
            "known_desktop",
        ] {
            let db = Database::open_memory().unwrap();
            let parent = db
                .create_conversation(&CreateConversationInput {
                    provider: "openai_codex".into(),
                    model: "test".into(),
                    system_prompt: None,
                    collection_context: None,
                    project_id: None,
                    persona_id: None,
                })
                .unwrap();
            let user = ConversationMessage {
                id: "user".into(),
                conversation_id: parent.id.clone(),
                role: Role::User,
                content: "Work".into(),
                tool_call_id: None,
                tool_calls: vec![],
                artifacts: None,
                token_count: 0,
                created_at: String::new(),
                sort_order: 0,
                thinking: None,
                image_attachments: None,
            };
            db.add_message(&user).unwrap();
            let turn = db
                .create_conversation_turn(&parent.id, &user.id, None)
                .unwrap();
            let parent_run = db
                .create_agent_task_run(
                    &parent.id,
                    &turn.id,
                    &user.id,
                    "Work",
                    Some("openai_codex"),
                    Some("test"),
                )
                .unwrap();
            let worker = db
                .create_agent_subtask_run(&parent_run.id, "worker", "worker", None, None)
                .unwrap();
            let provider = if variant == "acp" {
                "external_acp"
            } else {
                "openai_codex"
            };
            let child = db
                .delegated_conversation(&parent.id, "session", provider, "test")
                .unwrap();
            let tool = if variant == "known_desktop" {
                "computer_control"
            } else {
                "read_file"
            };
            let arguments = r#"{"action":"click","window_id":42,"observation_id":"before"}"#;
            let call = ToolCallRequest {
                id: "call".into(),
                name: tool.into(),
                arguments: arguments.into(),
                thought_signature: None,
            };
            let mut assistant = user.clone();
            assistant.id = "assistant".into();
            assistant.conversation_id = child.id.clone();
            assistant.role = Role::Assistant;
            assistant.tool_calls = vec![call];
            assistant.content.clear();
            db.add_message(&assistant).unwrap();
            let mut result = assistant.clone();
            result.id = "result".into();
            result.role = Role::Tool;
            result.tool_call_id = Some("call".into());
            result.tool_calls.clear();
            result.sort_order = 1;
            if variant == "old_result" {
                result.sort_order = -1;
            }
            if variant != "missing_result" {
                db.add_message(&result).unwrap();
            }
            let mut prose = user.clone();
            prose.id = "prose".into();
            prose.conversation_id = child.id.clone();
            prose.role = Role::Assistant;
            prose.sort_order = 2;
            db.add_message(&prose).unwrap();
            let start = json!({"callId":"call","toolName":tool,"arguments":arguments,"providerExecuted":false,"status":"running"});
            let artifacts = if variant == "uncertain_effect" {
                json!({"sideEffect":"may_have_occurred"})
            } else if variant == "known_desktop" {
                json!({"sideEffect":"may_have_occurred","data":{"windowId":42,"targetIdentity":"process-instance","consumedObservationId":"before"}})
            } else {
                json!({})
            };
            let end = json!({"callId":"call","toolName":tool,"arguments":arguments,"providerExecuted":false,"status":"completed","isError":false,"artifacts":artifacts});
            if variant == "complete" {
                assert!(pending_targets(&[end.clone()]).is_some());
                assert!(
                    complete_callback_ledger(
                        &db.conn(),
                        &child.id,
                        None,
                        &[
                            ("started".into(), start.clone()),
                            ("completed".into(), end.clone())
                        ],
                    )
                    .unwrap()
                    .is_some(),
                    "synthetic canonical callback ledger should be complete"
                );
            }
            let events = if variant == "truncated_pair" {
                json!([])
            } else if variant == "reordered_events" {
                json!([{"phase":"runCompleted","run":end},{"phase":"runStarted","run":start}])
            } else {
                json!([{"phase":"runStarted","run":start},{"phase":"runCompleted","run":end}])
            };
            db.finish_agent_subtask_run(&worker.id,"failed",Some(&json!({"kind":"subagent_run","run":{"sessionId":"session","runtimeIdentity":{"provider":provider,"model":"test"},"toolEvents":events,"recoveryRequired":{"actionReconciliation":true,"message":"Legacy generic recovery"}}})),None).unwrap();
            if variant == "complete" {
                assert!(
                    legacy_subtask_targets(&db.conn(), &parent_run.id, &worker.id)
                        .unwrap()
                        .is_some(),
                    "complete callback ledger should resolve its deterministic child conversation"
                );
            }
            let receipts = db
                .agent_subtask_action_reconciliations(&parent_run.id)
                .unwrap();
            let receipt = &receipts[&worker.id];
            assert_eq!(
                receipt.action_reconciliation,
                variant != "complete",
                "{variant}"
            );
            assert_eq!(
                receipt.has_bound_desktop_targets(),
                variant == "known_desktop",
                "{variant}"
            );
        }
    }
}
