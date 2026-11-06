//! Admit due occurrences from the shared durable cron ledger to the phone executor.
use super::*;
use nexa_core::workflow_automation::WorkflowAutomationDueRunClaim;

pub(crate) struct ScheduledLaunch {
    pub run_id: String,
    pub allowed_tools: Vec<String>,
}

impl LocalRuntime {
    pub async fn tick_schedules(self: &Arc<Self>, app: AppHandle) -> Result<Value, String> {
        let Ok(_tick) = self.scheduler.try_lock() else {
            return Ok(Value::Null);
        };
        if self.active.lock().await.is_some() {
            return Ok(Value::Null);
        }
        let now = chrono::Utc::now().to_rfc3339();
        for due in self.db.list_due_workflow_automations(&now).map_err(err)? {
            let Some(chat) = due.automation.conversation_id.as_deref() else {
                continue;
            };
            if self.db.workflow_conversation_is_busy(chat).map_err(err)? {
                continue;
            }
            let claim = self
                .db
                .claim_workflow_automation_due_run_at(due, &now, None)
                .map_err(err)?;
            if let Some(run) = &claim.run {
                if claim.due_run.automation.approval_policy.require_before_run {
                    self.db
                        .mark_workflow_automation_run_waiting_approval(&run.id)
                        .map_err(err)?;
                    continue;
                }
                self.launch_schedule(app, claim).await?;
                break;
            }
        }
        Ok(Value::Null)
    }

    pub async fn decide_schedule(
        self: &Arc<Self>,
        app: AppHandle,
        chat: &str,
        run_id: &str,
        allow: bool,
    ) -> Result<Value, String> {
        let _guard = self.scheduler.lock().await;
        let due = self.db.get_workflow_due_for_run(run_id).map_err(err)?;
        if due.automation.conversation_id.as_deref() != Some(chat) {
            return Err("This occurrence belongs to another chat.".into());
        }
        let now = chrono::Utc::now().to_rfc3339();
        if allow {
            if self.active.lock().await.is_some()
                || self.db.workflow_conversation_is_busy(chat).map_err(err)?
            {
                return Err(
                    "Finish the active phone task before approving this occurrence.".into(),
                );
            }
            let claim = self
                .db
                .approve_workflow_automation_run_at(run_id, &now)
                .map_err(err)?;
            self.launch_schedule(app, claim).await?;
        } else {
            self.db
                .deny_workflow_automation_run_at(run_id, &now)
                .map_err(err)?;
        }
        Ok(Value::Null)
    }

    async fn launch_schedule(
        self: &Arc<Self>,
        app: AppHandle,
        claim: WorkflowAutomationDueRunClaim,
    ) -> Result<(), String> {
        let run = claim.run.ok_or("Missing scheduled attempt.")?;
        let launch: Result<(), String> = async {
            let due = self.db.get_workflow_due_for_run(&run.id).map_err(err)?;
            let task = due.automation;
            let chat = task
                .conversation_id
                .ok_or("Missing scheduled conversation.")?;
            let allowed = task.approval_policy.allowed_tools;
            if !task.source_scope.is_empty()
                || allowed
                    .iter()
                    .any(|tool| !matches!(tool.as_str(), "web_search" | "fetch_url"))
            {
                return Err(
                    "This phone schedule requires unsupported source or tool grants.".into(),
                );
            }
            self.start(
                app,
                Some(chat),
                due.prompt,
                run.id.clone(),
                None,
                TurnOptions::default(),
                Some(ScheduledLaunch {
                    run_id: run.id.clone(),
                    allowed_tools: allowed,
                }),
            )
            .await?;
            Ok(())
        }
        .await;
        if let Err(error) = &launch {
            self.db
                .mark_workflow_automation_launch_failed_for_retry(
                    &run.id,
                    error,
                    &chrono::Utc::now().to_rfc3339(),
                )
                .map_err(err)?;
        }
        launch
    }
}
