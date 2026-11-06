//! Admit due occurrences from the shared durable cron ledger to the phone executor.
use super::*;
use nexa_core::workflow_automation::WorkflowAutomationApprovalState;
use nexa_core::workflow_automation::WorkflowAutomationDueRunClaim;

pub(crate) struct ScheduledLaunch {
    pub run_id: String,
    pub allowed_tools: Vec<String>,
}

impl LocalRuntime {
    pub(crate) fn next_wake_at(&self) -> Result<Option<String>, String> {
        let raw = self.db.next_phone_schedule_wake().map_err(err)?;
        let Some(raw) = raw else { return Ok(None) };
        let at = chrono::DateTime::parse_from_rfc3339(&raw)
            .map_err(err)?
            .to_utc();
        let now = chrono::Utc::now();
        // A due task can be waiting for another turn, an unexpired claim, or
        // retry backoff. Keep a single delayed wake in that case.
        Ok(Some(
            if at <= now {
                now + chrono::Duration::minutes(1)
            } else {
                at
            }
            .to_rfc3339(),
        ))
    }

    pub(crate) async fn arm_schedule(&self, app: &AppHandle) {
        let _guard = self.arm_lock.lock().await;
        if let Ok(snapshot) = self.schedule_wake_snapshot() {
            let _ = platform::call(app, "scheduleWake", snapshot).await;
        }
    }

    /// Read under arm_lock in both the Activity and JobService. Android can
    /// receive their JNI callbacks in a different order after the read.
    pub(crate) fn schedule_wake_snapshot(&self) -> Result<Value, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        Ok(json!({
            "nextWakeAt": self.next_wake_at()?,
            "pendingApprovals": self.db.pending_phone_schedule_approvals().map_err(err)?,
            "approvalSequence": SEQUENCE.fetch_add(1, Ordering::Relaxed)
        }))
    }

    pub async fn tick_schedules(self: &Arc<Self>, host: RunHost) -> Result<Value, String> {
        let Ok(_tick) = self.scheduler.try_lock() else {
            return Ok(json!({"status":"idle"}));
        };
        if self.active.lock().await.is_some() {
            return Ok(json!({"status":"idle"}));
        }
        let now = chrono::Utc::now().to_rfc3339();
        for due in self.db.list_due_phone_schedules(&now).map_err(err)? {
            host.check_worker()?;
            let claim = self
                .db
                .claim_workflow_automation_due_run_at(due, &now, None)
                .map_err(err)?;
            if let Some(run) = &claim.run {
                let approved = run
                    .occurrence_id
                    .as_deref()
                    .map(|id| self.db.workflow_automation_occurrence_approval_state(id))
                    .transpose()
                    .map_err(err)?;
                if claim.due_run.automation.approval_policy.require_before_run
                    && approved != Some(WorkflowAutomationApprovalState::Approved)
                {
                    let created = self
                        .db
                        .mark_workflow_automation_run_waiting_approval(&run.id)
                        .map_err(err)?;
                    if created {
                        return Ok(
                            json!({"status":"waiting_approval","conversationId":claim.due_run.automation.conversation_id,"runId":run.id}),
                        );
                    }
                    continue;
                }
                self.launch_schedule(host, claim).await?;
                return Ok(json!({"status":"running"}));
            }
        }
        Ok(json!({"status":"idle"}))
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
            nexa_core::tools::automation_tool::validate_phone_schedule_grants(&due.automation)
                .map_err(err)?;
            self.db
                .validate_workflow_conversation(&due.automation, chat)
                .map_err(err)?;
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
            self.launch_schedule(RunHost::Foreground(app), claim)
                .await?;
        } else {
            self.db
                .deny_workflow_automation_run_at(run_id, &now)
                .map_err(err)?;
        }
        Ok(Value::Null)
    }

    async fn launch_schedule(
        self: &Arc<Self>,
        host: RunHost,
        claim: WorkflowAutomationDueRunClaim,
    ) -> Result<(), String> {
        let run = claim.run.ok_or("Missing scheduled attempt.")?;
        let launch: Result<(), String> = async {
            let due = self.db.get_workflow_due_for_run(&run.id).map_err(err)?;
            let task = due.automation;
            let chat = task
                .conversation_id
                .as_deref()
                .ok_or("Missing scheduled conversation.")?;
            nexa_core::tools::automation_tool::validate_phone_schedule_grants(&task)
                .map_err(err)?;
            self.db
                .validate_workflow_conversation(&task, chat)
                .map_err(err)?;
            if !task.source_scope.is_empty()
                || !task
                    .schedule_config
                    .execution_policy
                    .source_scope_is_explicit
            {
                return Err(
                    "This phone schedule requires unsupported source or tool grants.".into(),
                );
            }
            let package =
                nexa_core::package_host::database_backed_builtin_package_host_snapshot(&self.db)
                    .map_err(err)?;
            if !package.runtime_components().iter().any(|component| {
                component.kind == nexa_core::package_host::PackageSurfaceKind::Workflow
                    && component.id == task.workflow_template_id
            }) {
                return Err("This scheduled workflow template is disabled or unavailable.".into());
            }
            // The phone editor supports its active connection. Reject foreign
            // pinned routes rather than running their prompts on another account.
            let policy = &task.schedule_config.execution_policy;
            if policy.agent_config_id.is_some()
                || policy.provider.is_some()
                || policy.provider_endpoint_id.is_some()
                || policy.model.is_some()
            {
                return Err("Run this account-pinned task on its original host.".into());
            }
            self.start(
                host,
                Some(chat.to_owned()),
                due.prompt,
                run.id.clone(),
                None,
                // The saved occurrence grants are already narrowed to the two
                // supported read-only tools; use the desktop scheduled policy.
                TurnOptions {
                    tool_approval_mode: ToolApprovalMode::AllowAll,
                    ..Default::default()
                },
                Some(ScheduledLaunch {
                    run_id: run.id.clone(),
                    allowed_tools: task.approval_policy.allowed_tools,
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
