use super::*;
use crate::workflow_authoring::{
    compile_workflow_recipe, WorkflowAuthoringPreview, WorkflowInputs, WorkflowRecipe,
    WorkflowRunOptions, WorkflowRunSnapshot,
};

pub(super) fn definition_digest(automation: &WorkflowAutomation) -> Result<String, CoreError> {
    let value = serde_json::json!({
        "name":automation.name,"description":automation.description,"template":automation.workflow_template_id,
        "prompt":automation.prompt,"recipe":automation.recipe,"trigger":automation.trigger,
        "sourceScope":automation.source_scope,"approvalPolicy":automation.approval_policy,"execution":automation.schedule_config,
        "revision":automation.definition_revision,
    });
    Ok(blake3::hash(&serde_json::to_vec(&value)?)
        .to_hex()
        .to_string())
}

pub(super) fn compile_saved_workflow(
    automation: &WorkflowAutomation,
    inputs: Option<&WorkflowInputs>,
) -> Result<WorkflowAuthoringPreview, CoreError> {
    let mut preview = if let Some(raw) = &automation.recipe {
        let recipe: WorkflowRecipe = serde_json::from_value(raw.clone())?;
        compile_workflow_recipe(&recipe, &automation.workflow_template_id, inputs)?
    } else {
        if inputs.is_some() {
            return Err(CoreError::InvalidInput("This legacy workflow stores a plain prompt; edit or copy it into a structured workflow before supplying typed inputs".into()));
        }
        WorkflowAuthoringPreview {
            prompt: automation.prompt.clone(),
            content_digest: String::new(),
            recipe: None,
            resolved_inputs: None,
            definition_revision: None,
        }
    };
    let mut resolved = automation.clone();
    resolved.prompt = preview.prompt;
    preview.prompt = automation_prompt(&resolved);
    preview.content_digest =
        blake3::hash(format!("{}\n{}", definition_digest(automation)?, preview.prompt).as_bytes())
            .to_hex()
            .to_string();
    preview.definition_revision = Some(automation.definition_revision);
    Ok(preview)
}

pub(super) fn prepare_run_snapshot(
    automation: &WorkflowAutomation,
    origin: WorkflowAutomationOccurrenceOrigin,
    options: &WorkflowRunOptions,
) -> Result<WorkflowRunSnapshot, CoreError> {
    if options
        .expected_revision
        .is_some_and(|revision| revision != automation.definition_revision)
    {
        return Err(CoreError::Conflict(
            "Workflow definition changed after preview; reload and preview again".into(),
        ));
    }
    let preview = compile_saved_workflow(automation, options.inputs.as_ref())?;
    if options
        .preview_digest
        .as_ref()
        .is_some_and(|digest| digest != &preview.content_digest)
    {
        return Err(CoreError::Conflict(
            "Workflow inputs or policy changed after preview; preview again before running".into(),
        ));
    }
    Ok(WorkflowRunSnapshot {
        version: 1,
        definition_digest: definition_digest(automation)?,
        automation: automation.clone(),
        resolved_inputs: preview.resolved_inputs,
        compiled_prompt: preview.prompt,
        origin,
    })
}

pub(super) fn save_run_snapshot(
    conn: &rusqlite::Connection,
    run_id: &str,
    snapshot: &WorkflowRunSnapshot,
) -> Result<(), CoreError> {
    conn.execute("INSERT INTO workflow_automation_run_snapshots (run_id, version, snapshot_json) VALUES (?1, 1, ?2)", rusqlite::params![run_id, serde_json::to_string(snapshot)?])?;
    Ok(())
}

pub(super) fn fetch_run_snapshot(
    conn: &rusqlite::Connection,
    run_id: &str,
) -> Result<Option<WorkflowRunSnapshot>, CoreError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT snapshot_json FROM workflow_automation_run_snapshots WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|raw| serde_json::from_str::<WorkflowRunSnapshot>(&raw).map_err(CoreError::from))
        .transpose()
}

pub(super) fn fetch_definition_revision(
    conn: &rusqlite::Connection,
    automation_id: &str,
    revision: i64,
) -> Result<WorkflowAutomation, CoreError> {
    conn.query_row("SELECT a.id,d.name,d.description,d.workflow_template_id,d.prompt,d.trigger_json,d.trigger_kind,d.source_scope_json,d.approval_policy_json,a.enabled,a.status,a.last_run_at,a.next_run_at,a.created_at,a.updated_at,d.schedule_config_json,d.recipe_json,d.revision FROM workflow_automations a JOIN workflow_automation_definition_revisions d ON d.automation_id=a.id WHERE a.id=?1 AND d.revision=?2", rusqlite::params![automation_id, revision], workflow_automation_from_row).map_err(CoreError::from)
}

pub(super) fn consume_folder_occurrence_cutoff(
    conn: &rusqlite::Connection,
    occurrence_id: &str,
) -> Result<(), CoreError> {
    let stored: Option<(String,i64,String)> = conn.query_row("SELECT o.automation_id,o.definition_revision,g.folder_cutoff_at FROM workflow_automation_occurrences o JOIN workflow_automation_occurrence_origins g ON g.occurrence_id=o.id WHERE o.id=?1 AND g.origin='folder_event'", [occurrence_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    if let Some((automation_id, revision, cutoff)) = stored {
        let next = parse_utc_timestamp(&cutoff)
            .ok_or_else(|| CoreError::InvalidInput("Invalid folder event cutoff".into()))?;
        let previous: Option<String> = conn.query_row("SELECT observed_through FROM workflow_automation_folder_cursors WHERE automation_id=?1 AND definition_revision=?2", rusqlite::params![&automation_id,revision], |row| row.get(0))?;
        if previous
            .as_deref()
            .and_then(parse_utc_timestamp)
            .is_none_or(|previous| next > previous)
        {
            conn.execute("UPDATE workflow_automation_folder_cursors SET observed_through=?3,updated_at=datetime('now') WHERE automation_id=?1 AND definition_revision=?2", rusqlite::params![automation_id,revision,cutoff])?;
        }
    }
    Ok(())
}

impl Database {
    pub fn get_workflow_due_for_run(
        &self,
        run_id: &str,
    ) -> Result<WorkflowAutomationDueRun, CoreError> {
        let conn = self.conn();
        let run = fetch_workflow_run(&conn, run_id)?;
        let (automation, prompt, origin) =
            if let Some(snapshot) = fetch_run_snapshot(&conn, run_id)? {
                (
                    snapshot.automation,
                    snapshot.compiled_prompt,
                    snapshot.origin,
                )
            } else {
                let automation = fetch_definition_revision(
                    &conn,
                    &run.automation_id,
                    i64::from(run.definition_revision),
                )?;
                let raw: String = conn.query_row(
                "SELECT origin FROM workflow_automation_occurrence_origins WHERE occurrence_id=?1",
                rusqlite::params![run.occurrence_id],
                |row| row.get(0),
            )?;
                let prompt = automation_prompt(&automation);
                (
                    automation,
                    prompt,
                    WorkflowAutomationOccurrenceOrigin::parse(&raw)?,
                )
            };
        Ok(WorkflowAutomationDueRun {
            automation,
            prompt,
            origin,
            scheduled_for: run.scheduled_for,
            due_reason: "saved workflow run".into(),
        })
    }
    pub fn preview_workflow_authoring(
        &self,
        id: &str,
        inputs: Option<&WorkflowInputs>,
    ) -> Result<WorkflowAuthoringPreview, CoreError> {
        compile_saved_workflow(&self.get_workflow_automation(id)?, inputs)
    }

    pub fn get_workflow_run_snapshot(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowRunSnapshot>, CoreError> {
        fetch_run_snapshot(&self.conn(), run_id)
    }

    pub fn get_workflow_definition_for_run(
        &self,
        run_id: &str,
    ) -> Result<WorkflowAutomation, CoreError> {
        let conn = self.conn();
        if let Some(snapshot) = fetch_run_snapshot(&conn, run_id)? {
            return Ok(snapshot.automation);
        }
        let run = fetch_workflow_run(&conn, run_id)?;
        fetch_definition_revision(
            &conn,
            &run.automation_id,
            i64::from(run.definition_revision),
        )
    }

    pub fn list_workflow_run_history(
        &self,
        automation_id: &str,
        before: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, CoreError> {
        let conn = self.conn();
        let mut statement = conn.prepare("SELECT r.id,r.automation_id,r.task_run_id,r.status,r.summary,r.created_at,r.finished_at,r.occurrence_id,r.scheduled_for,r.definition_revision,r.attempt,t.conversation_id FROM workflow_automation_runs r LEFT JOIN agent_task_runs t ON t.id=r.task_run_id WHERE r.automation_id=?1 AND (?2 IS NULL OR r.rowid<(SELECT rowid FROM workflow_automation_runs WHERE id=?2)) ORDER BY r.rowid DESC LIMIT ?3")?;
        let rows = statement.query_map(
            rusqlite::params![automation_id, before, limit.clamp(1, 100) as i64],
            |row| {
                let run = workflow_automation_run_from_row(row)?;
                let mut value = serde_json::to_value(run)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                value["conversationId"] = serde_json::json!(row.get::<_, Option<String>>(11)?);
                Ok(value)
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(CoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_authoring::{
        WorkflowDeliverable, WorkflowDeliverableFormat, WorkflowRecipeKind, WorkflowRecording,
        WorkflowRecordingStep, WorkflowStepKind,
    };

    fn recorded_recipe() -> WorkflowRecipe {
        WorkflowRecipe {
            version: 1,
            kind: WorkflowRecipeKind::Recorded,
            template_snapshot: crate::workflow_catalog::workflow_catalog()
                .into_iter()
                .find(|template| template.id == "report_brief")
                .unwrap(),
            inputs: WorkflowInputs {
                goal: "Saved default goal".into(),
                context: "材料".repeat(4_000),
                constraints: vec!["TAIL_CONSTRAINT".into()],
                deliverable: WorkflowDeliverable {
                    format: WorkflowDeliverableFormat::Markdown,
                    instructions: "Final report".into(),
                },
                replay_values: vec!["period=default".into()],
            },
            custom_instructions: "Keep the saved procedure".into(),
            recording: Some(WorkflowRecording {
                variable_inputs: vec!["period".into()],
                steps: vec![WorkflowRecordingStep {
                    id: "first".into(),
                    kind: WorkflowStepKind::Action,
                    text: format!("{}TAIL_STEP", "完整演示".repeat(3_500)),
                }],
                preferences: vec![],
                success_criteria: vec!["TAIL_SUCCESS".into()],
                safety_notes: vec!["TAIL_SAFETY".into()],
            }),
        }
    }

    fn input() -> SaveWorkflowAutomationInput {
        SaveWorkflowAutomationInput {
            id: None,
            name: "Recorded report".into(),
            description: "Replay a complete recording".into(),
            workflow_template_id: "report_brief".into(),
            prompt: String::new(),
            trigger: WorkflowAutomationTrigger::Manual,
            source_scope: vec![],
            approval_policy: Default::default(),
            enabled: true,
        }
    }

    #[test]
    fn manual_recording_reopens_approves_once_and_pins_runtime_inputs_without_editing_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("workflows.db");
        let db = Database::new(&path).unwrap();
        let recipe = recorded_recipe();
        let recipe_json = serde_json::to_value(&recipe).unwrap();
        let mut config = WorkflowAutomationScheduleConfig::default();
        config.execution_policy.model = Some("saved-model".into());
        let saved = db
            .save_authored_workflow_automation(&input(), &config, Some(&recipe_json), None)
            .unwrap();
        assert!(saved.prompt.chars().count() > 12_000);
        let mut runtime_inputs = recipe.inputs.clone();
        runtime_inputs.goal = "Only this run".into();
        runtime_inputs.replay_values = vec!["period=2026Q4".into()];
        let preview = db
            .preview_workflow_authoring(&saved.id, Some(&runtime_inputs))
            .unwrap();
        let options = WorkflowRunOptions {
            inputs: Some(runtime_inputs),
            expected_revision: Some(saved.definition_revision),
            preview_digest: Some(preview.content_digest.clone()),
            client_request_id: Some(uuid::Uuid::new_v4().to_string()),
        };
        let now = Utc::now().to_rfc3339();
        let due = db
            .workflow_automation_run_now_due_at(&saved.id, &now)
            .unwrap();
        let prepared = crate::workflow_execution::prepare_workflow_launch_with_options(
            &db,
            due.clone(),
            &now,
            None,
            &options,
        )
        .unwrap();
        let crate::workflow_execution::ScheduledWorkflowLaunchPreparation::PendingApproval { run } =
            prepared
        else {
            panic!("default manual run must wait for approval")
        };
        assert_eq!(run.definition_revision as i64, saved.definition_revision);
        assert_eq!(
            db.list_workflow_automation_runs_waiting_approval()
                .unwrap()
                .len(),
            1
        );
        assert!(db
            .claim_workflow_automation_with_options(due, &now, None, &options)
            .unwrap()
            .run
            .is_none());
        let snapshot = db.get_workflow_run_snapshot(&run.id).unwrap().unwrap();
        assert_eq!(snapshot.compiled_prompt, preview.prompt);
        for text in [
            "TAIL_STEP",
            "TAIL_SUCCESS",
            "TAIL_SAFETY",
            "TAIL_CONSTRAINT",
            "Only this run",
            "period=2026Q4",
        ] {
            assert!(snapshot.compiled_prompt.contains(text));
        }
        assert_eq!(
            db.get_workflow_automation(&saved.id).unwrap().recipe,
            Some(recipe_json.clone())
        );
        drop(db);
        let db = Database::new(&path).unwrap();
        let reopened = db.get_workflow_automation(&saved.id).unwrap();
        assert_eq!(
            reopened.schedule_config.execution_policy.model.as_deref(),
            Some("saved-model")
        );
        assert_eq!(reopened.recipe, Some(recipe_json));
        let approved = db
            .approve_workflow_automation_run_at(&run.id, &now)
            .unwrap();
        assert_eq!(approved.due_run.prompt, preview.prompt);
        assert_eq!(
            crate::workflow_execution::workflow_launch_mode(&approved.due_run),
            crate::workflow_execution::WorkflowLaunchMode::Interactive
        );
        assert!(db
            .approve_workflow_automation_run_at(&run.id, &now)
            .is_err());
        assert_eq!(
            db.list_workflow_run_history(&saved.id, None, 25)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn edits_cancel_pending_revisions_and_legacy_callers_keep_recording_structure() {
        let db = Database::open_memory().unwrap();
        let recipe = serde_json::to_value(recorded_recipe()).unwrap();
        let mut input = input();
        let config = WorkflowAutomationScheduleConfig::default();
        let first = db
            .save_authored_workflow_automation(&input, &config, Some(&recipe), None)
            .unwrap();
        let now = Utc::now().to_rfc3339();
        let claim = db
            .claim_workflow_automation_due_run_at(
                db.workflow_automation_run_now_due_at(&first.id, &now)
                    .unwrap(),
                &now,
                None,
            )
            .unwrap();
        let run = claim.run.unwrap();
        db.mark_workflow_automation_run_waiting_approval(&run.id)
            .unwrap();
        input.id = Some(first.id.clone());
        input.name = "Updated name".into();
        input.prompt = first.prompt.clone();
        let second = db
            .save_authored_workflow_automation(
                &input,
                &config,
                None,
                Some(first.definition_revision),
            )
            .unwrap();
        assert_eq!(second.recipe, Some(recipe));
        assert!(second.definition_revision > first.definition_revision);
        assert!(db
            .save_authored_workflow_automation(
                &input,
                &config,
                None,
                Some(first.definition_revision)
            )
            .is_err());
        assert!(db
            .approve_workflow_automation_run_at(&run.id, &now)
            .is_err());
        assert_eq!(
            db.get_workflow_definition_for_run(&run.id).unwrap().name,
            first.name
        );
        assert_eq!(
            db.get_workflow_automation_run(&run.id).unwrap().status,
            WorkflowAutomationRunStatus::Cancelled
        );
    }

    #[test]
    fn folder_observation_cutoff_survives_approval_and_preserves_later_same_second_files() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("event.txt");
        fs_write_at(&file, "first", "2026-10-08T01:00:00.100Z");
        let db = Database::open_memory().unwrap();
        let mut input = input();
        input.prompt = "Inspect matching text files".into();
        input.trigger = WorkflowAutomationTrigger::Folder {
            path: temp.path().display().to_string(),
            pattern: "*.txt".into(),
        };
        let automation = db.save_workflow_automation(&input).unwrap();
        let due = db
            .list_due_workflow_automations("2026-10-08T01:00:00.200Z")
            .unwrap()
            .remove(0);
        let claim = db
            .claim_workflow_automation_due_run_at(due, "2026-10-08T01:00:00.200Z", None)
            .unwrap();
        let run = claim.run.unwrap();
        db.mark_workflow_automation_run_waiting_approval(&run.id)
            .unwrap();
        fs_write_at(&file, "second", "2026-10-08T01:00:00.300Z");
        db.deny_workflow_automation_run_at(&run.id, "2026-10-08T01:00:00.400Z")
            .unwrap();
        let next = db
            .list_due_workflow_automations("2026-10-08T01:00:00.400Z")
            .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].automation.id, automation.id);
        assert_eq!(
            next[0].origin,
            WorkflowAutomationOccurrenceOrigin::FolderEvent
        );
        assert_eq!(
            next[0].scheduled_for.as_deref(),
            Some("2026-10-08T01:00:00.400+00:00")
        );
    }

    fn fs_write_at(path: &std::path::Path, body: &str, time: &str) {
        std::fs::write(path, body).unwrap();
        let modified: std::time::SystemTime = DateTime::parse_from_rfc3339(time)
            .unwrap()
            .with_timezone(&Utc)
            .into();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }
}
