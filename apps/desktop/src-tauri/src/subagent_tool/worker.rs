use super::*;

pub(super) struct WorkerExecutionSlots {
    pub(super) lane: tokio::sync::OwnedSemaphorePermit,
    pub(super) batch: Option<tokio::sync::OwnedSemaphorePermit>,
    pub(super) queue_wait_ms: u64,
}

struct WorkerLifecycleOwner {
    runtime: DelegationRuntime,
    agent_id: String,
    cancellation: CancellationToken,
    settled: bool,
}

impl WorkerLifecycleOwner {
    async fn settle(
        &mut self,
        outcome: Result<&SubagentRunArtifact, &CoreError>,
    ) -> Result<(), CoreError> {
        settle_worker_lifecycle(
            &self.runtime.lifecycle,
            &self.agent_id,
            &self.cancellation,
            outcome,
        )
        .await?;
        self.settled = true;
        Ok(())
    }
}

impl Drop for WorkerLifecycleOwner {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let status = if self.cancellation.is_cancelled() {
            SubagentLifecycleStatus::Cancelled
        } else {
            SubagentLifecycleStatus::Failed
        };
        self.cancellation.cancel();
        if let Err(error) = self.runtime.lifecycle.finish_blocking(
            &self.agent_id,
            status,
            None,
            Some("Worker execution ended before its result was settled".into()),
        ) {
            warn!(
                "Failed to retire abandoned subagent {}: {error}",
                self.agent_id
            );
        }
    }
}

pub(super) async fn acquire_worker_execution_slots(
    runtime: &DelegationRuntime,
    call_label: &str,
    args: &SpawnSubagentArgs,
    batch_slots: Option<Arc<tokio::sync::Semaphore>>,
) -> Result<WorkerExecutionSlots, CoreError> {
    let started = Instant::now();
    let profile = resolve_role_profile(args.role_id.as_deref(), args.role.as_deref())?;
    let lane = runtime
        .budget
        .acquire_worker_slot(
            call_label,
            profile.is_some_and(|profile| profile.id == "verifier"),
            &runtime.cancel_token,
        )
        .await
        .map_err(|error| subagent_admission_failure(&error))?;
    // Preserve lane-first ordering: queued explorers must not occupy the
    // batch slots required by an independently reserved verifier lane.
    let batch = match batch_slots {
        Some(slots) => Some(
            acquire_batch_slot(
                slots,
                &runtime.cancel_token,
                call_label,
                started,
                runtime.budget.limits().await.queue_deadline_ms,
            )
            .await
            .map_err(|error| subagent_admission_failure(&error))?,
        ),
        None => None,
    };
    ensure_worker_not_cancelled(&runtime.cancel_token, call_label)?;
    Ok(WorkerExecutionSlots {
        lane,
        batch,
        queue_wait_ms: instant_elapsed_ms(started),
    })
}

fn ensure_worker_not_cancelled(
    cancellation: &CancellationToken,
    call_label: &str,
) -> Result<(), CoreError> {
    if cancellation.is_cancelled() {
        Err(CoreError::Agent(format!(
            "Delegated execution '{call_label}' was cancelled before starting."
        )))
    } else {
        Ok(())
    }
}

pub(super) async fn await_subagent_worker_completion<T, F>(
    call_label: &str,
    cancel_token: &CancellationToken,
    fatal_error_rx: &mut mpsc::UnboundedReceiver<String>,
    run_future: F,
    run_deadline_ms: Option<u64>,
) -> Result<T, CoreError>
where
    F: std::future::Future<Output = Result<T, CoreError>>,
{
    tokio::select! {
        biased;
        // A clean event-pump shutdown may precede final persistence. Only an
        // actual fatal event can preempt that successful worker completion.
        Some(error) = fatal_error_rx.recv() => Err(CoreError::Agent(format!(
            "Delegated execution '{call_label}' failed: {error}"
        ))),
        _ = cancel_token.cancelled() => Err(CoreError::Agent(format!(
            "Delegated execution '{call_label}' was cancelled by the parent turn."
        ))),
        result = with_optional_timeout(run_deadline_ms, run_future) => match result {
            Ok(result) => result,
            Err(_) => {
                cancel_token.cancel();
                Err(CoreError::Agent(format!(
                    "Delegated execution '{call_label}' timed out after {}ms.",
                    run_deadline_ms.unwrap_or_default()
                )))
            }
        }
    }
}

pub(super) async fn run_subagent_once(
    runtime: DelegationRuntime,
    db: Database,
    inherited_source_scope: Vec<String>,
    call_label: String,
    worker_id: Option<String>,
    args: SpawnSubagentArgs,
    execution_slots: Option<WorkerExecutionSlots>,
    steering_rx: Option<mpsc::UnboundedReceiver<AgentSteeringMessage>>,
    lifecycle_events: Option<SubagentEventBridge>,
) -> Result<SubagentRunArtifact, CoreError> {
    let launch_started = Instant::now();
    let execution_slots = match execution_slots {
        Some(slots) => slots,
        None => acquire_worker_execution_slots(&runtime, &call_label, &args, None).await?,
    };
    ensure_worker_not_cancelled(&runtime.cancel_token, &call_label)?;
    let mut prepared = prepare_subagent_worker(
        &runtime,
        &db,
        inherited_source_scope,
        &args,
        &call_label,
        worker_id.as_deref(),
    )
    .await?;
    if let Some(events) = lifecycle_events.as_ref() {
        prepared.subtask_input["agentId"] = serde_json::json!(events.agent_id());
    }
    let admitted = admit_subagent_worker(
        &runtime,
        &db,
        &call_label,
        &args,
        execution_slots,
        launch_started,
        &prepared,
    )
    .await?;
    let parent_task_run_id = runtime.parent_task_run_id.clone();
    let AdmittedSubagentWorker {
        mut subtask,
        subtask_run_id,
        _lane_permit,
        _batch_permit,
        mut reservation,
    } = admitted;
    if let Some(events) = lifecycle_events.as_ref() {
        runtime
            .lifecycle
            .set_status(events.agent_id(), SubagentLifecycleStatus::Running)?;
        emit_subagent_lifecycle_event(
            Some(events),
            SubagentLifecycleEventKind::Progress,
            serde_json::json!({ "status": "running" }),
        )
        .await;
    }
    let PreparedSubagentWorker {
        worker_cancel_token,
        role_profile,
        requested_task_id,
        session_id,
        previous_session,
        config,
        provider,
        effective_provider_type,
        effective_model,
        model_route_fallback,
        delegation_limits,
        context_snapshot,
        run_deadline_ms,
        effective_allowed_tools,
        effective_source_scope,
        preflight,
        evidence_handoff,
        enabled_skills,
        applied_skill_refs,
        tools,
        request_text,
        context_snapshot_artifact,
        effective_model_budgets,
        source_scope_applied,
        ..
    } = prepared;
    let (final_message, capture) = execute_subagent_worker(
        SubagentExecutionInput {
            runtime: &runtime,
            db: &db,
            call_label: &call_label,
            launch_started,
            parent_task_run_id: parent_task_run_id.clone(),
            subtask_run_id: subtask_run_id.clone(),
            session_id: session_id.clone(),
            worker_cancel_token: worker_cancel_token.clone(),
            effective_provider_type,
            effective_model: effective_model.clone(),
            worker_actual_token_limit: delegation_limits
                .max_actual_tokens_per_worker
                .and_then(|limit| u32::try_from(limit).ok()),
            run_deadline_ms,
            context_messages: context_snapshot.messages.as_ref().to_vec(),
            effective_source_scope: effective_source_scope.clone(),
            provider,
            tools,
            config,
            enabled_skills,
            request_text,
            steering_rx,
            lifecycle_events,
        },
        &mut subtask,
        &mut reservation,
    )
    .await?;
    let run = settle_subagent_artifact(
        &runtime,
        SubagentSettlementInput {
            worker_id,
            call_label,
            args,
            role_profile,
            requested_task_id,
            session_id,
            previous_session,
            effective_model,
            model_route_fallback,
            evidence_handoff,
            effective_source_scope,
            effective_allowed_tools,
            applied_skill_refs,
            preflight,
            context_snapshot_artifact,
            effective_model_budgets,
            source_scope_applied,
        },
        final_message,
        capture,
        &mut subtask,
    );
    Ok(run)
}
pub(super) fn isolated_subagent_runtime() -> Result<tokio::runtime::Runtime, CoreError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            CoreError::Internal(format!(
                "Failed to build isolated subagent runtime: {error}"
            ))
        })
}
pub(super) async fn settle_worker_lifecycle(
    lifecycle: &SubagentLifecycleRuntime,
    agent_id: &str,
    cancellation: &CancellationToken,
    outcome: Result<&SubagentRunArtifact, &CoreError>,
) -> Result<(), CoreError> {
    let (status, result, error) = match outcome {
        Ok(run) => (
            SubagentLifecycleStatus::Completed,
            serde_json::to_value(run).ok(),
            None,
        ),
        Err(error) => (
            if cancellation.is_cancelled() {
                SubagentLifecycleStatus::Cancelled
            } else {
                SubagentLifecycleStatus::Failed
            },
            None,
            Some(error.to_string()),
        ),
    };
    lifecycle.finish(agent_id, status, result, error).await?;
    Ok(())
}
#[allow(clippy::too_many_arguments)]
async fn run_admitted_worker(
    runtime: DelegationRuntime,
    db: Database,
    inherited_source_scope: Vec<String>,
    call_label: String,
    worker_id: Option<String>,
    args: SpawnSubagentArgs,
    execution_slots: WorkerExecutionSlots,
    registration: crate::subagent_lifecycle::SubagentWorkerRegistration,
) -> Result<SubagentRunArtifact, CoreError> {
    run_subagent_once(
        runtime.scoped_to_worker(registration.cancel_token),
        db,
        inherited_source_scope,
        call_label,
        worker_id,
        args,
        Some(execution_slots),
        Some(registration.steering_rx),
        Some(registration.events),
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_registered_subagent_isolated(
    runtime: DelegationRuntime,
    db: Database,
    inherited_source_scope: Vec<String>,
    call_label: String,
    worker_id: Option<String>,
    mut args: SpawnSubagentArgs,
    batch_slots: Option<Arc<tokio::sync::Semaphore>>,
    registration: crate::subagent_lifecycle::SubagentWorkerRegistration,
) -> Result<SubagentRunArtifact, CoreError> {
    let agent_id = registration.agent_id.clone();
    let cancellation = registration.cancel_token.clone();
    let runtime = runtime.scoped_to_worker(cancellation.clone());
    let mut owner = Some(WorkerLifecycleOwner {
        runtime: runtime.clone(),
        agent_id,
        cancellation: cancellation.clone(),
        settled: false,
    });
    let outcome = async {
        let workspace_lease = runtime.parent_conversation_id.as_deref().map(|id| nexa_core::chat_worktrees::activity(&db, id)).transpose()?;
        if let Some(id) = runtime.parent_conversation_id.as_deref() {
            let current = db.conversation_workspace(id)?;
            let inherited = runtime.tool_registry.lock().map_err(|_| CoreError::Internal("Subagent tools unavailable".into()))?.as_ref().and_then(|tools| tools.workspace().cloned());
            if db.chat_worktree(id)?.is_some() && current != inherited {
                return Err(CoreError::InvalidInput("The chat workspace changed before this worker started; launch the worker again".into()));
            }
        }
        registration.events.start().await?;
        if let Some(gate) = &args.stage_handoff.gate {
            emit_subagent_lifecycle_event(Some(&registration.events), SubagentLifecycleEventKind::Progress,
                serde_json::json!({"phase":"waiting_dependencies","dependsOn":args.stage_handoff.depends_on})).await;
            args.stage_handoff.results = runtime.wait_for_dependencies(gate, &cancellation).await?;
            if let Some(narrowed) = &args.source_ids {
                if args.stage_handoff.results.iter().any(|result| result.source_scope.is_empty() || result.source_scope.iter().any(|source| !narrowed.contains(source))) {
                    return Err(CoreError::InvalidInput("Dependency output is outside the receiving stage's explicit source scope; use compatible source scopes or an authorized evidence handoff".into()));
                }
            }
        }
        let execution_slots =
            acquire_worker_execution_slots(&runtime, &call_label, &args, batch_slots).await?;
        ensure_worker_not_cancelled(&cancellation, &call_label)?;
        // Provider/executor futures are kept on an isolated current-thread
        // runtime. Only admitted workers allocate this thread and reactor.
        let isolated_runtime = isolated_subagent_runtime()?;
        let (result_tx, result_rx) = oneshot::channel();
        let mut worker_owner = owner.take().expect("worker owner transferred only once");
        std::thread::Builder::new()
            .name("nexa-subagent-worker".to_string())
            .spawn(move || {
                let _workspace_lease = workspace_lease;
                let result = isolated_runtime.block_on(async move {
                    let result = run_admitted_worker(
                        runtime,
                        db,
                        inherited_source_scope,
                        call_label,
                        worker_id,
                        args,
                        execution_slots,
                        registration,
                    )
                    .await;
                    // The isolated worker owns settlement even when the parent
                    // drops/aborts its collector or the result receiver closes.
                    worker_owner.settle(result.as_ref()).await?;
                    result
                });
                let _ = result_tx.send(result);
            })
            .map_err(|error| {
                CoreError::Internal(format!("Failed to start isolated subagent thread: {error}"))
            })?;
        result_rx.await.map_err(|_| {
            CoreError::Agent("Isolated subagent thread exited without a result".to_string())
        })?
    }
    .await;
    if let Some(owner) = owner.as_mut() {
        owner.settle(outcome.as_ref()).await?;
    }
    outcome
}
pub(super) fn launch_detached_subagent(
    runtime: DelegationRuntime,
    db: Database,
    inherited_source_scope: Vec<String>,
    args: SpawnSubagentArgs,
    registration: crate::subagent_lifecycle::SubagentWorkerRegistration,
) {
    let agent_id = registration.agent_id.clone();
    tokio::spawn(async move {
        if let Err(error) = run_registered_subagent_isolated(
            runtime,
            db,
            inherited_source_scope,
            agent_id.clone(),
            Some(agent_id.clone()),
            args,
            None,
            registration,
        )
        .await
        {
            warn!("Detached subagent {agent_id} failed: {error}");
        }
    });
}
pub(super) fn failed_subagent_run_artifact(
    label: String,
    fallback: SpawnSubagentArgs,
    parallel_group: Option<String>,
    error: &CoreError,
) -> SubagentRunArtifact {
    SubagentRunArtifact {
        id: label.clone(),
        session_id: label,
        resumed_from_task_id: None,
        previous_session: None,
        status: "error".to_string(),
        depends_on: fallback.stage_handoff.depends_on.clone(),
        predecessor_results: fallback.stage_handoff.evidence(),
        task: fallback.task,
        role_id: fallback.role_id.clone(),
        role_name: resolve_role_profile(fallback.role_id.as_deref(), fallback.role.as_deref())
            .ok()
            .flatten()
            .map(|profile| profile.label.to_string()),
        role: fallback.role,
        model_policy: fallback.model_policy,
        effective_model: None,
        model_route_fallback: false,
        expected_output: fallback.expected_output,
        acceptance_criteria: fallback.acceptance_criteria,
        evidence_chunk_ids: fallback.evidence_chunk_ids,
        evidence_handoff: Vec::new(),
        requested_source_scope: fallback.source_ids,
        effective_source_scope: Vec::new(),
        requested_allowed_tools: fallback.allowed_tools,
        allowed_tools: Vec::new(),
        allowed_skills: Vec::new(),
        parallel_group,
        deliverable_style: fallback.deliverable_style,
        return_sections: fallback.return_sections,
        result: format!("Subagent failed: {error}"),
        finish_reason: None,
        usage_total: Usage::default(),
        tool_events: Vec::new(),
        thinking: None,
        source_scope_applied: false,
        is_error: true,
        error_message: Some(error.to_string()),
        preflight_failure: subagent_preflight_failure_from_error(error),
        preflight: None,
        context_snapshot: None,
        effective_model_budgets: None,
    }
}
pub(super) fn summarize_subagent_run(run: &SubagentRunArtifact) -> String {
    let role_suffix = run
        .role_name
        .as_deref()
        .or(run.role.as_deref())
        .map(|role| format!(" ({role})"))
        .unwrap_or_default();
    format!(
        "{}{}: {}",
        run.task,
        role_suffix,
        truncate_excerpt(&run.result, 220)
    )
}
