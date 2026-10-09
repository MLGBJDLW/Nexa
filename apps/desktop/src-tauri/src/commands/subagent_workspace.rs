use super::*;
use crate::subagent_lifecycle::SubagentLifecycleEventKind;
use nexa_core::activity::SubagentHistoryPage;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentWorkspacePage {
    agent_id: Option<String>,
    journal: Option<SubagentHistoryPage>,
    legacy_run: Option<AgentSubtaskRun>,
    status: String,
    can_control: bool,
    privacy_revision: String,
}

// Batch workerId is a task label. agentId is the lifecycle authority; only
// older rows without that field may use a verified workerId as their identity.
fn row_lifecycle_id(row: &AgentSubtaskRun) -> Option<&str> {
    let input = row.input.as_ref()?;
    input
        .get("agentId")
        .or_else(|| input.get("workerId"))?
        .as_str()
        .filter(|id| !id.trim().is_empty())
}

fn resolve_worker(
    db: &Database,
    conversation_id: &str,
    id: &str,
) -> Result<(Option<String>, Option<AgentSubtaskRun>), CoreError> {
    db.get_conversation(conversation_id)?;
    if let Ok(row) = db.get_agent_subtask_run(id) {
        let parent = db.get_agent_task_run(&row.parent_run_id)?;
        if parent.conversation_id != conversation_id {
            return Err(CoreError::NotFound("Subagent".into()));
        }
        let worker = row_lifecycle_id(&row).map(str::to_owned);
        if let Some(worker_id) = worker.as_deref() {
            match db.read_subagent_history(conversation_id, worker_id, u64::MAX) {
                Ok(_) => {}
                Err(CoreError::NotFound(_)) => return Ok((None, Some(row))),
                Err(error) => return Err(error),
            }
        }
        return Ok((worker, Some(row)));
    }
    // This checks the exact persisted owner and excludes arbitrary activities.
    db.read_subagent_history(conversation_id, id, u64::MAX)?;
    Ok((Some(id.to_string()), None))
}

#[tauri::command]
pub async fn read_subagent_workspace_cmd(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    id: String,
    after_seq: Option<u64>,
    wait_ms: Option<u64>,
) -> Result<SubagentWorkspacePage, String> {
    let resolution_revision = state
        .db
        .privacy_revision()
        .map_err(|error| error.to_string())?;
    let cid = conversation_id.clone();
    let (agent_id, legacy_run) = state
        .db_executor
        .read(move |db| resolve_worker(db, &cid, &id))
        .await
        .map_err(|error| error.to_string())?
        .value;
    let Some(agent_id) = agent_id else {
        let privacy_revision = state
            .db
            .privacy_revision()
            .map_err(|error| error.to_string())?;
        if privacy_revision != resolution_revision {
            return Err("Privacy settings changed; reload the subtask history".into());
        }
        let mut status = legacy_run
            .as_ref()
            .map(|run| run.status.clone())
            .unwrap_or_default();
        let live_id = legacy_run
            .as_ref()
            .and_then(row_lifecycle_id)
            .filter(|worker_id| {
                state
                    .subagent_lifecycle
                    .ensure_conversation(worker_id, Some(&conversation_id))
                    .is_ok()
            });
        if live_id.is_none()
            && matches!(
                status.as_str(),
                "queued" | "running" | "starting" | "cancelling"
            )
        {
            status = "orphaned".into();
        }
        return Ok(SubagentWorkspacePage {
            status,
            agent_id: None,
            journal: None,
            legacy_run,
            can_control: false,
            privacy_revision,
        });
    };
    let after = after_seq.unwrap_or(0);
    if state
        .subagent_lifecycle
        .ensure_conversation(&agent_id, Some(&conversation_id))
        .is_ok()
        && wait_ms.unwrap_or(0) > 0
    {
        // No model invocation; the lifecycle notification wakes this bounded read.
        let _ = state
            .subagent_lifecycle
            .observe(
                &agent_id,
                after,
                Duration::from_millis(wait_ms.unwrap_or(0).min(2500)),
            )
            .await;
    }
    let cid = conversation_id.clone();
    let aid = agent_id.clone();
    let journal = state
        .db_executor
        .read(move |db| db.read_subagent_history(&cid, &aid, after))
        .await
        .map_err(|error| error.to_string())?
        .value;
    let live = state
        .subagent_lifecycle
        .ensure_conversation(&agent_id, Some(&conversation_id))
        .ok()
        .and_then(|()| state.subagent_lifecycle.snapshot(&agent_id).ok());
    let status = live
        .as_ref()
        .map(|worker| {
            serde_json::to_value(worker.status)
                .unwrap_or_default()
                .as_str()
                .unwrap_or("unknown")
                .to_string()
        })
        .unwrap_or_else(|| {
            if journal.record.state.is_terminal() {
                format!("{:?}", journal.record.state).to_ascii_lowercase()
            } else {
                "orphaned".into()
            }
        });
    let can_control = live.is_some_and(|worker| {
        !worker.status.is_terminal()
            && worker.status != crate::subagent_lifecycle::SubagentLifecycleStatus::Cancelling
    });
    Ok(SubagentWorkspacePage {
        privacy_revision: journal.privacy_revision.clone(),
        agent_id: Some(agent_id),
        journal: Some(journal),
        legacy_run: None,
        status,
        can_control,
    })
}

#[tauri::command]
pub async fn control_subagent_workspace_cmd(
    state: tauri::State<'_, AppState>,
    conversation_id: String,
    agent_id: String,
    action: String,
    input: Option<String>,
) -> Result<(), String> {
    state
        .db
        .get_conversation(&conversation_id)
        .map_err(|error| error.to_string())?;
    state
        .subagent_lifecycle
        .ensure_conversation(&agent_id, Some(&conversation_id))
        .map_err(|error| error.to_string())?;
    match action.as_str() {
        "input" => {
            let input = input
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "An instruction is required".to_string())?;
            if input.len() > 64 * 1024 {
                return Err("Instruction exceeds 64 KiB".into());
            }
            let bridge = state
                .subagent_lifecycle
                .send_input(&agent_id, input.to_owned())
                .map_err(|error| error.to_string())?;
            bridge
                .emit(
                    SubagentLifecycleEventKind::InputQueued,
                    serde_json::json!({
                        "content": input, "bytes": input.len(), "state": "queued", "source": "user",
                        "acknowledgement": "channel_enqueue_only"
                    }),
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        "cancel" => {
            state
                .subagent_lifecycle
                .cancel(&agent_id)
                .map_err(|error| error.to_string())?
                .emit(
                    SubagentLifecycleEventKind::Progress,
                    serde_json::json!({ "status": "cancelling" }),
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        _ => return Err("Unknown subagent action".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_core::activity::{ActivityRuntime, ActivitySpec, ActivitySurface};
    use nexa_core::conversation::{ConversationMessage, CreateConversationInput};
    use nexa_core::llm::Role;

    #[test]
    fn subagent_workspace_resolves_batch_rows_by_agent_identity_before_task_labels() {
        let db = Database::open_memory().unwrap();
        let conv = db
            .create_conversation(&CreateConversationInput {
                provider: "openai".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        let user = ConversationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conv.id.clone(),
            role: Role::User,
            content: "batch".into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: None,
            token_count: 1,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        };
        db.add_message(&user).unwrap();
        let turn = db
            .create_conversation_turn(&conv.id, &user.id, None)
            .unwrap();
        let parent = db
            .create_agent_task_run(&conv.id, &turn.id, &user.id, "batch", None, None)
            .unwrap();
        let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
        for id in ["actual-agent-uuid", "research-label"] {
            runtime
                .start(
                    ActivitySpec::new(ActivitySurface::Process, "spawn_subagent")
                        .with_activity_id(id)
                        .with_conversation_id(&conv.id),
                )
                .unwrap();
        }
        let row = db
            .create_agent_subtask_run(
                &parent.id,
                "Review",
                "reviewer",
                Some(
                    &serde_json::json!({"workerId":"research-label","agentId":"actual-agent-uuid"}),
                ),
                None,
            )
            .unwrap();
        assert_eq!(
            resolve_worker(&db, &conv.id, &row.id).unwrap().0.as_deref(),
            Some("actual-agent-uuid")
        );
        let legacy = db
            .create_agent_subtask_run(
                &parent.id,
                "Old worker",
                "reviewer",
                Some(&serde_json::json!({"workerId":"actual-agent-uuid"})),
                None,
            )
            .unwrap();
        assert_eq!(
            resolve_worker(&db, &conv.id, &legacy.id)
                .unwrap()
                .0
                .as_deref(),
            Some("actual-agent-uuid")
        );
        let missing = db
            .create_agent_subtask_run(
                &parent.id,
                "Missing",
                "reviewer",
                Some(&serde_json::json!({"workerId":"research-label","agentId":"missing-agent"})),
                None,
            )
            .unwrap();
        assert!(
            resolve_worker(&db, &conv.id, &missing.id)
                .unwrap()
                .0
                .is_none(),
            "a present agent identity must not redirect to a similarly named different worker"
        );
    }
}
