//! Recover exact native targets at the hard-stop boundary from host receipts.

use std::collections::HashSet;
use std::time::Duration;

use nexa_core::activity::{ActivityEventKind, ActivityRuntime, ActivityState, ActivitySurface};
use nexa_core::agent_run::AgentRunEventKind;
use nexa_core::db::Database;
use nexa_core::error::CoreError;
use nexa_core::intelligence::{build_task_plan, AgentTaskPlan, TaskPlanningInput};
use nexa_core::quality_profile::{
    resolve_orchestration_profile, OrchestrationProfile, OrchestrationProfileInput,
};
use nexa_core::workflow_ir::{compile_workflow_ir, DesktopObservationTarget, WorkflowIr};
use serde_json::{json, Value};

pub(super) async fn preserve_pending_targets(
    db: &Database,
    activities: &ActivityRuntime,
    run_id: &str,
    turn_id: &str,
) -> Result<(), CoreError> {
    let run = db.get_agent_task_run(run_id)?;
    if run.turn_id != turn_id {
        return Err(CoreError::InvalidInput(
            "Stopped task does not own the supplied turn".into(),
        ));
    }
    let records = activities
        .list()
        .into_iter()
        .filter(|record| {
            record.surface == ActivitySurface::Desktop
                && record.owner_tool == "computer_control"
                && record.turn_id.as_deref() == Some(turn_id)
                && record.conversation_id.as_deref() == Some(run.conversation_id.as_str())
        })
        .collect::<Vec<_>>();
    let mut targets: Vec<(
        Option<String>,
        String,
        DesktopObservationTarget,
        Option<Value>,
    )> = Vec::new();
    for record in records {
        let observation = activities
            .observe(&record.activity_id, 0, Duration::ZERO)
            .await?;
        if observation.record.state == ActivityState::Failed
            && observation.events.last().is_some_and(|event| {
                event.kind == ActivityEventKind::Failed
                    && event.seq == observation.record.last_event_seq
                    && event
                        .payload
                        .pointer("/detail/inputDelivered")
                        .and_then(Value::as_bool)
                        == Some(false)
                    && event
                        .payload
                        .pointer("/detail/effectMayHaveOccurred")
                        .and_then(Value::as_bool)
                        == Some(false)
            })
        {
            // A host-proven precommit rejection adds no uncertainty. Skip it
            // before per-target replacement so an older real pending action
            // on this same window keeps its original consumed token.
            continue;
        }
        // The host writes prepared before spawning an OS worker, then carries
        // the same identity in claimed/observed/terminal receipts. Never parse
        // model text, resume reasons, or colon-delimited IDs for target authority.
        let target = observation
            .events
            .iter()
            .rev()
            .filter(|event| {
                matches!(
                    event.kind,
                    ActivityEventKind::Progress
                        | ActivityEventKind::DesktopObservation
                        | ActivityEventKind::Completed
                        | ActivityEventKind::Failed
                        | ActivityEventKind::TimedOut
                )
            })
            .find_map(|event| {
                target_from_receipt(event.payload.get("detail").unwrap_or(&event.payload))
            });
        if let Some(target) = target {
            let terminal_receipt = observation
                .events
                .iter()
                .rev()
                .filter(|event| event.kind == ActivityEventKind::Completed)
                .filter_map(|event| event.payload.get("detail"))
                .find(|detail| {
                    detail.get("actionReceiptId").and_then(Value::as_str)
                        == Some(record.activity_id.as_str())
                        && detail
                            .get("terminalWindowReceipt")
                            .is_some_and(Value::is_object)
                })
                .cloned();
            // The runtime orders records by start time. As in WorkflowIr, only
            // the latest action for the same window/process generation owns
            // its pending token; an older closure cannot settle a later action.
            targets.retain(|(_, _, previous, _)| {
                previous.window_id != target.window_id
                    || previous.target_identity != target.target_identity
            });
            targets.push((
                record.session_id,
                record.activity_id,
                target,
                terminal_receipt,
            ));
        }
    }
    if targets.is_empty() {
        return Ok(());
    }

    let task_plan = run
        .plan
        .as_ref()
        .and_then(|plan| serde_json::from_value::<AgentTaskPlan>(plan.clone()).ok())
        .unwrap_or_else(|| {
            build_task_plan(TaskPlanningInput::for_route(
                "Reconcile pending native desktop actions after a stopped turn.",
                "InteractionOperation",
                false,
                0,
            ))
        });
    let profile = resolve_orchestration_profile(OrchestrationProfileInput {
        profile: OrchestrationProfile::Balanced,
        custom: None,
        max_iterations: 0,
        max_parallel: Some(1),
        max_calls_per_turn: None,
        delegated_token_budget: None,
        verification_reserve_percent: None,
    });
    let mut workflow = match run
        .plan
        .as_ref()
        .and_then(|plan| plan.get("workflowIr"))
        .filter(|value| !value.is_null())
    {
        Some(saved) => serde_json::from_value::<WorkflowIr>(saved.clone()).map_err(|error| {
            CoreError::InvalidInput(format!("Cannot preserve stopped desktop targets: {error}"))
        })?,
        None => {
            compile_workflow_ir(&task_plan, &profile, false).map_err(CoreError::InvalidInput)?
        }
    };
    workflow.validate().map_err(CoreError::InvalidInput)?;

    // A result may have reached the durable outbox just before abort while the
    // executor had not yet updated run.plan. Replay native results in order so
    // completed verification/explicit closure is not reopened by older receipts.
    let mut completed_receipts = HashSet::new();
    let mut completed_targets = Vec::new();
    for event in db.list_agent_run_events(run_id)? {
        if event.kind != AgentRunEventKind::ToolCompleted {
            continue;
        }
        let Some(tool) = event.payload.get("run") else {
            continue;
        };
        let Some(name @ ("computer_control" | "computer_observe")) =
            tool.get("toolName").and_then(Value::as_str)
        else {
            continue;
        };
        let Some(call_id) = tool.get("callId").and_then(Value::as_str) else {
            continue;
        };
        if name == "computer_control" {
            if let Some(data) = tool.get("artifacts").and_then(|value| value.get("data")) {
                if let Some(receipt_id) = data.get("actionReceiptId").and_then(Value::as_str) {
                    completed_receipts.insert(receipt_id.to_string());
                }
                if let Some(target) = target_from_receipt(data) {
                    completed_targets.push((call_id.to_string(), target));
                }
            }
        }
        workflow.observe_tool_result_with_arguments(
            call_id,
            name,
            tool.get("arguments").and_then(Value::as_str),
            tool.get("isError").and_then(Value::as_bool).unwrap_or(true),
            tool.get("artifacts"),
            "",
        );
    }
    for (call_id, activity_id, target, terminal_receipt) in targets {
        if completed_receipts.contains(&activity_id) {
            continue;
        }
        if let Some(receipt) = terminal_receipt {
            // Only the workflow's original explicit-close contract may accept
            // the worker-owned closure proof; an editing task remains blocked.
            workflow.observe_tool_result(
                &activity_id,
                "computer_control",
                false,
                Some(&json!({
                    "artifacts":{"kind":"computerControl"}, "data":receipt,
                })),
                "Native worker completed with a target-bound terminal window receipt.",
            );
            continue;
        }
        // Provider call IDs may repeat in later model samples. Require an exact
        // host receipt or the complete target+consumed-token tuple to settle it.
        if call_id.as_ref().is_some_and(|call_id| {
            completed_targets
                .iter()
                .any(|(completed_call, completed_target)| {
                    completed_call == call_id && completed_target == &target
                })
        }) {
            continue;
        }
        let receipt = json!({
            "code":"computer_action_uncertain", "sideEffect":"may_have_occurred",
            "data":{"windowId":target.window_id,"targetIdentity":target.target_identity,
                "consumedObservationId":target.consumed_observation_id}
        });
        workflow.observe_tool_result(
            &activity_id,
            "computer_control",
            true,
            Some(&receipt),
            "Hard stop interrupted the native result; observe the exact target before completion.",
        );
    }
    let mut plan = run
        .plan
        .unwrap_or_else(|| serde_json::to_value(&task_plan).expect("task plan is serializable"));
    let Some(object) = plan.as_object_mut() else {
        return Err(CoreError::InvalidInput(
            "Cannot preserve stopped desktop targets in a non-object task plan".into(),
        ));
    };
    object.insert("workflowIr".into(), serde_json::to_value(workflow)?);
    db.update_agent_task_run_progress(run_id, None, None, None, None, Some(&plan), None)
}

fn target_from_receipt(receipt: &Value) -> Option<DesktopObservationTarget> {
    let window_id = receipt
        .get("windowId")?
        .as_u64()
        .filter(|value| *value > 0)?;
    let identity = receipt
        .get("targetIdentity")?
        .as_str()
        .filter(|value| !value.is_empty())?;
    let observation = receipt
        .get("consumedObservationId")?
        .as_str()
        .filter(|value| !value.is_empty())?;
    Some(DesktopObservationTarget {
        window_id,
        target_identity: Some(identity.into()),
        consumed_observation_id: Some(observation.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_core::activity::ActivitySpec;
    use nexa_core::agent::{restore_pending_desktop_evidence, AgentEvent, CancellationToken};
    use nexa_core::agent_run::AgentRunEvent;
    use nexa_core::conversation::{AgentTaskRun, ConversationMessage, CreateConversationInput};
    use nexa_core::db_executor::DatabaseExecutor;
    use nexa_core::llm::Role;
    use nexa_core::run_event_outbox::{AgentRunEventDelivery, AgentRunEventOutboxes};
    use nexa_core::runtime::{ActiveAgentTurn, AgentTurnHandle};
    use nexa_core::workflow_ir::VerificationGateKind;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    struct SlowDelivery(Arc<AtomicBool>);
    impl AgentRunEventDelivery for SlowDelivery {
        fn deliver_run_event(&self, _: &str, event: &AgentRunEvent) {
            if event.label == "hold old plan delivery" {
                self.0.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        fn deliver_task_run_snapshot(&self, _: &str, _: AgentTaskRun) {}
    }

    fn user_message(conversation_id: &str, content: &str) -> ConversationMessage {
        ConversationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: None,
            token_count: 0,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hard_stop_checkpoint_restores_exact_target_through_formal_resume() {
        assert_hard_stop_resume(false, false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hard_stop_replays_redacted_timeout_receipt_with_host_consumed_identity() {
        assert_hard_stop_resume(true, false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hard_stop_precommit_rejection_preserves_earlier_pending_target() {
        assert_hard_stop_resume(false, true).await;
    }

    async fn assert_hard_stop_resume(completed_timeout: bool, add_precommit_rejection: bool) {
        let db = Database::open_memory().unwrap();
        let conversation = db
            .create_conversation(&CreateConversationInput {
                provider: "test".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        let user = user_message(&conversation.id, "Fill the editor and verify its content");
        db.add_message(&user).unwrap();
        let turn = db
            .create_conversation_turn(&conversation.id, &user.id, None)
            .unwrap();
        let run = db
            .create_agent_task_run(
                &conversation.id,
                &turn.id,
                &user.id,
                &user.content,
                None,
                None,
            )
            .unwrap();
        db.mark_agent_task_run_started(&run.id, "tooling").unwrap();
        let plan = build_task_plan(TaskPlanningInput::for_route(
            &user.content,
            "InteractionOperation",
            false,
            0,
        ));
        let activities = ActivityRuntime::with_database(db.clone()).unwrap();
        for (id, owner, window) in [
            ("pending-native-A", conversation.id.as_str(), 42),
            ("other-owner", "another-conversation", 99),
        ] {
            activities
                .start(
                    ActivitySpec::new(ActivitySurface::Desktop, "computer_control")
                        .with_activity_id(id)
                        .with_session_id(id)
                        .with_conversation_id(owner)
                        .with_turn_id(&turn.id),
                )
                .unwrap();
            activities.append(id, ActivityEventKind::Progress, json!({
                "stage":"prepared", "windowId":window,"targetIdentity":"process-generation-A","consumedObservationId":"before-A"
            })).unwrap();
        }
        if add_precommit_rejection {
            activities
                .start(
                    ActivitySpec::new(ActivitySurface::Desktop, "computer_control")
                        .with_activity_id("rejected-before-input")
                        .with_session_id("rejected-call")
                        .with_conversation_id(&conversation.id)
                        .with_turn_id(&turn.id),
                )
                .unwrap();
            activities.append("rejected-before-input", ActivityEventKind::Progress, json!({
                "stage":"prepared","windowId":42,"targetIdentity":"process-generation-A","consumedObservationId":"never-consumed-new-token"
            })).unwrap();
            activities.transition("rejected-before-input", ActivityState::Failed, json!({
                "stage":"precommit_rejected","inputDelivered":false,"effectMayHaveOccurred":false,
                "windowId":42,"targetIdentity":"process-generation-A","consumedObservationId":"never-consumed-new-token"
            })).unwrap();
        }
        let entered = Arc::new(AtomicBool::new(false));
        activities
            .start(
                ActivitySpec::new(ActivitySurface::Desktop, "computer_control")
                    .with_activity_id("prior-native-receipt")
                    .with_session_id("pending-native-A")
                    .with_conversation_id(&conversation.id)
                    .with_turn_id(&turn.id),
            )
            .unwrap();
        activities.append("prior-native-receipt", ActivityEventKind::Progress, json!({
            "stage":"prepared", "windowId":77,"targetIdentity":"prior-process-B","consumedObservationId":"prior-before-B"
        })).unwrap();
        let outbox = AgentRunEventOutboxes::new(
            DatabaseExecutor::new(db.clone(), 8).unwrap(),
            Arc::new(SlowDelivery(entered.clone())),
        )
        .open(&conversation.id, &run.id)
        .await
        .unwrap();
        // A previous sample reused the same provider call ID for another target.
        let mut prior_result = AgentRunEvent::from_agent_event(&AgentEvent::ToolCallResult {
            call_id:"pending-native-A".into(), tool_name:"computer_control".into(), content:"Observed prior target".into(), is_error:false,
            artifacts:Some(json!({"artifacts":{"kind":"computerControl"},"data":{
                "windowId":77,"targetIdentity":"prior-process-B","consumedObservationId":"prior-before-B",
                "actionReceiptId":"prior-native-receipt","inputDelivered":true,"targetVerified":true,"deliveryStatus":"delivered",
                "observationId":"prior-after-B","observation":{"windowId":77,"targetIdentity":"prior-process-B","observationId":"prior-after-B","screenshotHash":"prior-pixels"}
            }})),
        }).with_context(Some(&run.id), Some(&turn.id), None);
        prior_result.payload["run"]["arguments"] = json!(
            r#"{"action":"invoke","window_id":77,"observation_id":"<observation-token-redacted>"}"#
        );
        outbox.submit(prior_result).unwrap();
        outbox
            .submit(
                AgentRunEvent::from_agent_event(&AgentEvent::Status {
                    content: "hold old plan delivery".into(),
                    tone: Some("running".into()),
                })
                .with_context(Some(&run.id), Some(&turn.id), None),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !entered.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // This queued pre-control plan must drain before stop merges A's receipt.
        outbox
            .submit(
                AgentRunEvent::from_agent_event(&AgentEvent::PlanUpdated {
                    plan: serde_json::to_value(&plan).unwrap(),
                    phase: Some("tooling".into()),
                    summary: Some("Old plan before native result".into()),
                })
                .with_context(Some(&run.id), Some(&turn.id), None),
            )
            .unwrap();
        if completed_timeout {
            let mut timeout = AgentRunEvent::from_agent_event(&AgentEvent::ToolCallResult {
                call_id:"pending-native-A".into(), tool_name:"computer_control".into(), content:"Native worker may still finish".into(), is_error:true,
                artifacts:Some(json!({"code":"computer_action_timeout_uncertain","sideEffect":"may_have_occurred","data":{
                    "windowId":42,"targetIdentity":"process-generation-A","consumedObservationId":"before-A"
                }})),
            }).with_context(Some(&run.id), Some(&turn.id), None);
            timeout.payload["run"]["arguments"] = json!(
                r#"{"action":"invoke","window_id":42,"observation_id":"<observation-token-redacted>"}"#
            );
            outbox.submit(timeout).unwrap();
        }
        let (steering_tx, _steering_rx) = tokio::sync::mpsc::unbounded_channel();
        let active = ActiveAgentTurn {
            handle: AgentTurnHandle::running(&conversation.id, &run.id, &turn.id),
            cancel_token: CancellationToken::new(),
            task: tokio::spawn(std::future::pending::<()>()),
            steering_tx,
            event_outbox: Arc::clone(&outbox),
            orchestrator_run_id: None,
            frontend_paint_recorded: AtomicBool::new(false),
        };
        let approvals = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        super::super::fence_and_checkpoint_desktop_agent_turn(active, &db, &approvals)
            .await
            .unwrap();
        let checkpoint = db.latest_task_resume_checkpoint(&run.id).unwrap().unwrap();
        let saved: WorkflowIr = serde_json::from_value(
            checkpoint
                .state
                .pointer("/run/plan/workflowIr")
                .unwrap()
                .clone(),
        )
        .unwrap();
        assert_eq!(saved.checkpoint.desktop_observation_targets.len(), 1);
        assert_eq!(
            saved.checkpoint.desktop_observation_targets[0].window_id,
            42
        );
        assert_eq!(
            saved.checkpoint.desktop_observation_targets[0]
                .target_identity
                .as_deref(),
            Some("process-generation-A")
        );
        assert_eq!(
            saved.checkpoint.desktop_observation_targets[0]
                .consumed_observation_id
                .as_deref(),
            Some("before-A")
        );

        let response = user_message(&conversation.id, &checkpoint.resume_prompt);
        db.resume_agent_turn_from_checkpoint(
            &response,
            None,
            None,
            "resume-stopped-native",
            &checkpoint.id,
        )
        .unwrap();
        let profile = resolve_orchestration_profile(OrchestrationProfileInput {
            profile: OrchestrationProfile::Balanced,
            custom: None,
            max_iterations: 0,
            max_parallel: Some(1),
            max_calls_per_turn: None,
            delegated_token_budget: None,
            verification_reserve_percent: None,
        });
        let mut restored = None;
        restore_pending_desktop_evidence(
            &db,
            Some(&conversation.id),
            Some(&turn.id),
            &mut restored,
            &plan,
            &profile,
            false,
        )
        .unwrap();
        let restored = restored.as_mut().unwrap();
        for (window, identity, token, remaining) in [
            (77, "process-generation-B", "after-B", 1),
            (42, "reused-hwnd-other-process", "after-reused-A", 1),
            (42, "process-generation-A", "before-A", 1),
            (42, "process-generation-A", "after-A", 0),
        ] {
            restored.observe_tool_result_with_arguments(token, "computer_observe", Some(r#"{"action":"capture_window"}"#), false, Some(&json!({
                "artifacts":{"kind":"computerObservation"},"data":{"windowId":window,"targetIdentity":identity,"observationId":token,"screenshotHash":"fresh-pixels"}
            })), "");
            assert_eq!(
                restored.checkpoint.desktop_observation_targets.len(),
                remaining
            );
            let gate = restored
                .verification_gates
                .iter()
                .find(|gate| gate.kind == VerificationGateKind::DesktopObservation)
                .unwrap();
            assert_eq!(gate.passed, Some(remaining == 0));
        }
    }
}
