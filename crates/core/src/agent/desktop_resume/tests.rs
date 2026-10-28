use super::*;
use crate::conversation::{AgentTaskRun, ConversationMessage, CreateConversationInput};
use crate::intelligence::{build_task_plan, AgentTaskPlan, TaskPlanningInput};
use crate::llm::Role;
use crate::quality_profile::{
    resolve_orchestration_profile, OrchestrationProfile, OrchestrationProfileInput,
    ResolvedOrchestrationProfile,
};
use crate::tool_visibility_policy::{resolve_turn_capability_requirements, ToolVisibilityInput};
use crate::workflow_ir::compile_workflow_ir;
use serde_json::json;

fn plan(objective: &str) -> AgentTaskPlan {
    let requirements = resolve_turn_capability_requirements(ToolVisibilityInput {
        query: objective,
        system_prompt: "",
        has_sources: false,
    });
    build_task_plan(TaskPlanningInput::for_requirements(
        objective,
        &requirements,
        false,
        0,
    ))
}

fn profile() -> ResolvedOrchestrationProfile {
    resolve_orchestration_profile(OrchestrationProfileInput {
        profile: OrchestrationProfile::Balanced,
        custom: None,
        max_iterations: 20,
        max_parallel: None,
        max_calls_per_turn: None,
        delegated_token_budget: None,
        verification_reserve_percent: None,
    })
}

fn message(conversation_id: &str, content: &str) -> ConversationMessage {
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

fn run(db: &Database, conversation_id: &str, content: &str) -> AgentTaskRun {
    let user = message(conversation_id, content);
    db.add_message(&user).unwrap();
    let turn = db
        .create_conversation_turn(conversation_id, &user.id, None)
        .unwrap();
    db.create_agent_task_run(conversation_id, &turn.id, &user.id, content, None, None)
        .unwrap()
}

fn fixture() -> (Database, AgentTaskRun, AgentTaskPlan, WorkflowIr) {
    fixture_for("Capture this app window and click Save")
}

fn fixture_for(objective: &str) -> (Database, AgentTaskRun, AgentTaskPlan, WorkflowIr) {
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
    let run = run(&db, &conversation.id, objective);
    let plan = plan(&run.title);
    let mut workflow = compile_workflow_ir(&plan, &profile(), false).unwrap();
    for window in [42, 43] {
        workflow.observe_tool_result_with_arguments("control", "computer_control", Some(&json!({"action":"invoke","window_id":window,"observation_id":format!("before-{window}")}).to_string()), false,
            Some(&json!({"data":{"windowId":window,"targetIdentity":format!("target-{window}"),"inputDelivered":true}})), "Input delivered without a post-action capture");
    }
    (db, run, plan, workflow)
}

fn late_closure_detail(activity_id: &str) -> Value {
    json!({"windowId":42,"targetIdentity":"target-42","consumedObservationId":"before-42",
        "actionReceiptId":activity_id,"targetVerified":true,"inputDelivered":true,
        "deliveryStatus":"delivered","effect":"window_closed",
        "terminalWindowReceipt":{"kind":"computerWindowClosure","windowId":42,
            "targetIdentity":"target-42","actionReceiptId":activity_id,
            "windowExists":false,"inputDelivered":true}})
}

fn late_modal_handoff_detail(activity_id: &str) -> Value {
    let mut detail = late_closure_detail(activity_id);
    detail["modalOwnerHandoffReceipt"] = json!({"kind":"computerModalOwnerHandoff", "windowId":42,
        "targetIdentity":"target-42", "consumedObservationId":"before-42", "actionReceiptId":activity_id,
        "windowExists":false,"inputDelivered":true,"ownerRelationshipVerified":true,
        "ownerObservedBeforeAction":true,"ownerObservationNotBeforeMs":1000,
        "owner":{"windowId":77,"targetIdentity":"exact-owner"}});
    detail
}

#[test]
fn late_native_modal_handoff_restores_owner_obligation_without_granting_close_permission() {
    use crate::activity::{ActivityRuntime, ActivitySpec};
    let (db, run, _, mut saved) = fixture();
    observe(&mut saved, 43, "target-43", "after-43");
    let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
    let activity = runtime
        .start(
            ActivitySpec::new(ActivitySurface::Desktop, "computer_control")
                .with_conversation_id(&run.conversation_id)
                .with_turn_id(&run.turn_id),
        )
        .unwrap();
    resume(&db, &run, &saved);
    runtime
        .transition(
            &activity.activity_id,
            ActivityState::Completed,
            late_modal_handoff_detail(&activity.activity_id),
        )
        .unwrap();
    let resumed_plan = plan("Continue the task");
    let mut resumed = None;
    restore_pending_desktop_evidence(
        &db,
        Some(&run.conversation_id),
        Some(&run.turn_id),
        &mut resumed,
        &resumed_plan,
        &profile(),
        false,
    )
    .unwrap();
    let resumed = resumed.as_mut().unwrap();
    assert!(
        !resumed
            .completion_contract
            .desktop_terminal_closure_evidence
    );
    assert_eq!(resumed.checkpoint.desktop_observation_targets.len(), 1);
    assert_eq!(
        resumed.checkpoint.desktop_observation_targets[0].window_id,
        77
    );
    assert_eq!(
        resumed.checkpoint.desktop_observation_targets[0].observation_not_before_ms,
        Some(1000)
    );
    for (window, identity, time, expected) in [
        (78, "exact-owner", 1001, false),
        (77, "other-owner", 1001, false),
        (77, "exact-owner", 1000, false),
        (77, "exact-owner", 1001, true),
    ] {
        resumed.observe_tool_result_with_arguments("owner", "computer_observe",
            Some(&json!({"action":"capture_window","window_id":window}).to_string()), false,
            Some(&json!({"data":{"kind":"computerObservationReceipt","windowId":window,
                "targetIdentity":identity,"observationId":format!("owner-{time}"),"observationCapturedAtMs":time,
                "screenshotHash":"owner-pixels"}})), "host owner capture");
        assert_eq!(resumed.completion_allowed(), expected);
    }
}

#[test]
fn late_modal_handoff_cannot_transfer_a_different_pending_control_or_forged_relationship() {
    use crate::activity::{ActivityRuntime, ActivitySpec};
    for mismatch in ["token", "activity", "relation", "turn"] {
        let (db, run, _, saved) = fixture();
        let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
        let activity = runtime
            .start(
                ActivitySpec::new(ActivitySurface::Desktop, "computer_control")
                    .with_conversation_id(&run.conversation_id)
                    .with_turn_id(if mismatch == "turn" {
                        "other"
                    } else {
                        &run.turn_id
                    }),
            )
            .unwrap();
        resume(&db, &run, &saved);
        let mut detail = late_modal_handoff_detail(&activity.activity_id);
        match mismatch {
            "token" => {
                detail["consumedObservationId"] = json!("other-token");
                detail["modalOwnerHandoffReceipt"]["consumedObservationId"] = json!("other-token");
            }
            "activity" => {
                detail["modalOwnerHandoffReceipt"]["actionReceiptId"] = json!("other-action")
            }
            "relation" => {
                detail["modalOwnerHandoffReceipt"]["ownerRelationshipVerified"] = json!(false)
            }
            _ => {}
        }
        runtime
            .transition(&activity.activity_id, ActivityState::Completed, detail)
            .unwrap();
        let resumed =
            load_desktop_resume_workflow(&db, Some(&run.conversation_id), Some(&run.turn_id))
                .unwrap()
                .unwrap();
        assert_eq!(
            resumed.checkpoint.desktop_observation_targets,
            saved.checkpoint.desktop_observation_targets,
            "{mismatch}"
        );
    }
}

#[test]
fn late_native_worker_closure_is_reconciled_at_the_real_checkpoint_resume_boundary() {
    use crate::activity::{ActivityRuntime, ActivitySpec};
    let (db, run, _, mut saved) =
        fixture_for("Capture this app window, close the window, then verify it");
    observe(&mut saved, 43, "target-43", "after-43");
    let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
    let activity = runtime
        .start(
            ActivitySpec::new(ActivitySurface::Desktop, "computer_control")
                .with_conversation_id(&run.conversation_id)
                .with_turn_id(&run.turn_id),
        )
        .unwrap();
    resume(&db, &run, &saved);
    // The worker finishes only after Stop has already persisted its checkpoint.
    runtime
        .transition(
            &activity.activity_id,
            ActivityState::Completed,
            late_closure_detail(&activity.activity_id),
        )
        .unwrap();
    let resumed_plan = plan("Continue the task");
    let mut resumed = None;
    restore_pending_desktop_evidence(
        &db,
        Some(&run.conversation_id),
        Some(&run.turn_id),
        &mut resumed,
        &resumed_plan,
        &profile(),
        false,
    )
    .unwrap();
    let resumed = resumed.unwrap();
    assert!(resumed.checkpoint.desktop_observation_targets.is_empty());
    assert!(!resumed.desktop_evidence_pending());
    assert!(resumed.completion_allowed());
    assert!(
        resumed
            .completion_contract
            .desktop_terminal_closure_evidence
    );
}

#[test]
fn late_closure_reconciliation_rejects_other_actions_targets_turns_and_ungranted_closure() {
    use crate::activity::{ActivityRuntime, ActivitySpec};
    for mismatch in [
        "identity",
        "token",
        "receipt",
        "turn",
        "conversation",
        "surface",
        "tool",
        "record",
        "no-contract",
    ] {
        let objective = if mismatch == "no-contract" {
            "Capture this app window and click Save"
        } else {
            "Capture this app window, close the window, then verify it"
        };
        let (db, run, _, saved) = fixture_for(objective);
        let runtime = ActivityRuntime::with_database(db.clone()).unwrap();
        let activity = runtime
            .start(
                ActivitySpec::new(
                    if mismatch == "surface" {
                        ActivitySurface::Browser
                    } else {
                        ActivitySurface::Desktop
                    },
                    if mismatch == "tool" {
                        "browser_session"
                    } else {
                        "computer_control"
                    },
                )
                .with_conversation_id(if mismatch == "conversation" {
                    "other"
                } else {
                    &run.conversation_id
                })
                .with_turn_id(if mismatch == "turn" {
                    "other"
                } else {
                    &run.turn_id
                }),
            )
            .unwrap();
        resume(&db, &run, &saved);
        let mut detail = late_closure_detail(&activity.activity_id);
        match mismatch {
            "identity" => {
                detail["targetIdentity"] = json!("recycled-42");
                detail["terminalWindowReceipt"]["targetIdentity"] = json!("recycled-42");
            }
            "token" => detail["consumedObservationId"] = json!("other-control-token"),
            "receipt" => detail["terminalWindowReceipt"]["actionReceiptId"] = json!("other-action"),
            "record" => {
                detail["actionReceiptId"] = json!("other-action");
                detail["terminalWindowReceipt"]["actionReceiptId"] = json!("other-action");
            }
            _ => {}
        }
        runtime
            .transition(&activity.activity_id, ActivityState::Completed, detail)
            .unwrap();
        let resumed =
            load_desktop_resume_workflow(&db, Some(&run.conversation_id), Some(&run.turn_id))
                .unwrap()
                .unwrap();
        assert_eq!(
            resumed.checkpoint.desktop_observation_targets,
            saved.checkpoint.desktop_observation_targets,
            "{mismatch}"
        );
        assert!(resumed.desktop_evidence_pending(), "{mismatch}");
    }
}

fn resume(db: &Database, run: &AgentTaskRun, workflow: &WorkflowIr) -> String {
    let checkpoint = db
        .create_task_resume_checkpoint_with_state(
            &run.id,
            "user_pause",
            Some(&json!({"kind":"longTaskLiveState","workflowIr":workflow})),
        )
        .unwrap();
    db.update_agent_task_run_progress(
        &run.id,
        Some("paused"),
        Some("paused"),
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let response = message(&run.conversation_id, &checkpoint.resume_prompt);
    let launch = db
        .resume_agent_turn_from_checkpoint(&response, None, None, "test-resume-key", &checkpoint.id)
        .unwrap();
    assert_eq!(launch.turn_id, run.turn_id);
    checkpoint.id
}

fn observe(workflow: &mut WorkflowIr, window: u64, identity: &str, token: &str) {
    workflow.observe_tool_result_with_arguments("capture", "computer_observe", Some(&json!({"action":"capture_window","window_id":window}).to_string()), false,
        Some(&json!({"data":{"kind":"computerObservationReceipt","windowId":window,"targetIdentity":identity,"observationId":token,"screenshotHash":"fresh-pixels"}})), "fresh target capture");
}

#[test]
fn delegated_local_verification_resumes_exact_targets_without_an_opaque_deadlock() {
    let (db, run, _, worker_workflow) = fixture();
    let worker = db
        .create_agent_subtask_run(&run.id, "native-worker", "worker", None, None)
        .unwrap();
    let error = CoreError::LocalVerificationRequired {
        browser_targets: Vec::new(),
        message: "Verify the native windows".into(),
        targets: worker_workflow
            .checkpoint
            .desktop_observation_targets
            .clone(),
    };
    db.finish_agent_subtask_run(
        &worker.id,
        "failed",
        Some(&json!({
            "kind":"subagent_run", "run":{"recoveryRequired":error.recovery_requirement()}
        })),
        Some(&error.to_string()),
    )
    .unwrap();
    let parent_plan = plan("Review completed work");
    let parent_workflow = compile_workflow_ir(&parent_plan, &profile(), false).unwrap();
    resume(&db, &run, &parent_workflow);
    let saved = load_execution_resume_state(&db, Some(&run.conversation_id), Some(&run.turn_id))
        .unwrap()
        .unwrap();
    assert!(!saved.has_opaque_subtask_recoveries);
    assert!(saved.checkpointed_subtask_recoveries.contains(&worker.id));
    let mut resumed = None;
    restore_pending_desktop_evidence(
        &db,
        Some(&run.conversation_id),
        Some(&run.turn_id),
        &mut resumed,
        &parent_plan,
        &profile(),
        false,
    )
    .unwrap();
    let resumed = resumed.as_mut().unwrap();
    assert_eq!(resumed.checkpoint.desktop_observation_targets.len(), 2);
    observe(resumed, 42, "recycled-owner", "fresh-wrong");
    observe(resumed, 42, "target-42", "before-42");
    assert_eq!(resumed.checkpoint.desktop_observation_targets.len(), 2);
    observe(resumed, 42, "target-42", "after-42");
    assert!(resumed.desktop_evidence_pending());
    observe(resumed, 43, "target-43", "after-43");
    assert!(!resumed.desktop_evidence_pending());
    db.update_agent_task_run_progress(
        &run.id,
        None,
        None,
        None,
        None,
        Some(&resumed.task_plan_checkpoint(&parent_plan)),
        None,
    )
    .unwrap();
    let mut reopened = None;
    restore_pending_desktop_evidence(
        &db,
        Some(&run.conversation_id),
        Some(&run.turn_id),
        &mut reopened,
        &parent_plan,
        &profile(),
        false,
    )
    .unwrap();
    assert!(
        reopened.is_none(),
        "resolved worker receipt must not resurrect on reopen"
    );
}

#[test]
fn delegated_browser_recovery_requires_exact_fresh_tab_and_does_not_resurrect() {
    let (db, run, _, _) = fixture();
    let worker = db
        .create_agent_subtask_run(&run.id, "browser-worker", "worker", None, None)
        .unwrap();
    let target = crate::workflow_ir::BrowserObservationTarget {
        session_id: "session".into(),
        tab_id: "tab".into(),
        consumed_observation_id: Some("before".into()),
        opened_without_observation: false,
        observation_not_before_ms: Some(100),
    };
    let error = CoreError::LocalVerificationRequired {
        message: "Observe the existing tab".into(),
        targets: Vec::new(),
        browser_targets: vec![target],
    };
    db.finish_agent_subtask_run(
        &worker.id,
        "failed",
        Some(&json!({"run":{"recoveryRequired":error.recovery_requirement()}})),
        Some(&error.to_string()),
    )
    .unwrap();
    let parent_plan = plan("Review completed work");
    let parent_workflow = compile_workflow_ir(&parent_plan, &profile(), false).unwrap();
    resume(&db, &run, &parent_workflow);
    let mut resumed = None;
    restore_pending_desktop_evidence(
        &db,
        Some(&run.conversation_id),
        Some(&run.turn_id),
        &mut resumed,
        &parent_plan,
        &profile(),
        false,
    )
    .unwrap();
    let resumed = resumed.as_mut().unwrap();
    assert!(resumed.has_pending_local_targets());
    for (session, tab, token, time) in [
        ("other-session", "tab", "fresh", 200),
        ("session", "new-tab", "fresh", 200),
        ("session", "tab", "before", 200),
        ("session", "tab", "old-pixels", 99),
    ] {
        resumed.observe_tool_result_with_arguments("observe", "browser_session",
            Some(&json!({"action":"observe","session_id":session,"tab_id":tab}).to_string()), false,
            Some(&json!({"kind":"browserObservation","data":{"sessionId":session,"tabId":tab,"observationId":token,"capturedAtMs":time,"screenshotHash":"pixels"}})), "observation");
        assert!(
            resumed.has_pending_local_targets(),
            "wrong, reused or old evidence cannot clear the target"
        );
    }
    resumed.observe_tool_result_with_arguments("replayed-open", "browser_session", Some(r#"{"action":"create_session"}"#), false,
        Some(&json!({"kind":"browserOpenReceipt","sessionId":"session","tabId":"tab","opened":true,"observationPending":true})), "opened");
    assert!(
        resumed.has_pending_local_targets(),
        "an opening receipt cannot verify itself"
    );
    resumed.observe_tool_result_with_arguments("fresh", "browser_session", Some(r#"{"action":"observe","session_id":"session","tab_id":"tab"}"#), false,
        Some(&json!({"kind":"browserObservation","data":{"sessionId":"session","tabId":"tab","observationId":"after","capturedAtMs":200,"screenshotHash":"new-pixels"}})), "fresh");
    assert!(!resumed.has_pending_local_targets());
    assert!(resumed
        .completion_progress_milestones()
        .iter()
        .any(|milestone| milestone == "browser-target:session:tab"));
    db.update_agent_task_run_progress(
        &run.id,
        None,
        None,
        None,
        None,
        Some(&resumed.task_plan_checkpoint(&parent_plan)),
        None,
    )
    .unwrap();
    let mut reopened = None;
    restore_pending_desktop_evidence(
        &db,
        Some(&run.conversation_id),
        Some(&run.turn_id),
        &mut reopened,
        &parent_plan,
        &profile(),
        false,
    )
    .unwrap();
    assert!(
        reopened.is_none(),
        "verified imported browser debt must not resurrect"
    );
}

#[test]
fn exact_host_browser_absence_releases_input_but_preserves_unverified_completion() {
    let mut workflow = compile_workflow_ir(&plan("Review work"), &profile(), false).unwrap();
    let target = crate::workflow_ir::BrowserObservationTarget {
        session_id: "gone-session".into(),
        tab_id: "gone-tab".into(),
        consumed_observation_id: None,
        opened_without_observation: true,
        observation_not_before_ms: Some(100),
    };
    workflow.import_desktop_recovery("worker", &[], &[target]);
    for (action, session, tab, time) in [
        ("list_sessions", "gone-session", "gone-tab", 200),
        ("list_tabs", "other-session", "gone-tab", 200),
        ("list_tabs", "gone-session", "other-tab", 200),
        ("list_tabs", "gone-session", "gone-tab", 99),
    ] {
        workflow.observe_tool_result_with_arguments("presence", "browser_session",
            Some(&json!({"action":action,"sessionId":session,"tabId":tab}).to_string()), false,
            Some(&json!({"kind":"browserTargetPresence","sessionId":session,"tabId":tab,"present":false,"checkedAtMs":time,"goalVerified":false})), "not present");
        assert!(workflow.has_pending_local_targets());
    }
    workflow.observe_tool_result_with_arguments("presence", "browser_session",
        Some(r#"{"action":"list_tabs","sessionId":"gone-session","tabId":"gone-tab"}"#), false,
        Some(&json!({"kind":"browserTargetPresence","sessionId":"gone-session","tabId":"gone-tab","present":false,"checkedAtMs":200,"goalVerified":false})), "not present");
    assert!(
        !workflow.has_pending_local_targets(),
        "exact host absence permits replanning without replay"
    );
    assert!(
        workflow.local_evidence_pending(),
        "missing historical target does not verify the requested goal"
    );
    let mut reopened = compile_workflow_ir(&plan("Review work"), &profile(), false).unwrap();
    reopened.restore_desktop_checkpoint(&workflow);
    assert!(!reopened.has_pending_local_targets());
    assert!(
        reopened.local_evidence_pending(),
        "reopening must keep the unverified completion gate"
    );
}

#[test]
fn durable_checkpoint_continuation_restores_every_pending_target_at_real_start_boundary() {
    let (db, run, _, saved) = fixture();
    resume(&db, &run, &saved);
    let resumed_plan = plan("Continue the task");
    let mut workflow = None;
    restore_pending_desktop_evidence(
        &db,
        Some(&run.conversation_id),
        Some(&run.turn_id),
        &mut workflow,
        &resumed_plan,
        &profile(),
        false,
    )
    .unwrap();
    let workflow = workflow.as_mut().unwrap();
    assert_eq!(workflow.checkpoint.desktop_observation_targets.len(), 2);
    assert!(!workflow.completion_allowed());
    observe(workflow, 43, "target-43", "after-43");
    assert!(
        !workflow.completion_allowed(),
        "window B cannot discharge pending A"
    );
    observe(workflow, 42, "recycled-42", "after-recycled");
    assert!(!workflow.completion_allowed());
    observe(workflow, 42, "target-42", "before-42");
    assert!(!workflow.completion_allowed());
    observe(workflow, 42, "target-42", "after-42");
    assert!(workflow.completion_allowed());
    db.update_agent_task_run_progress(
        &run.id,
        None,
        None,
        None,
        None,
        Some(&workflow.task_plan_checkpoint(&resumed_plan)),
        None,
    )
    .unwrap();
    assert!(
        load_desktop_resume_workflow(&db, Some(&run.conversation_id), Some(&run.turn_id))
            .unwrap()
            .is_none(),
        "an older claimed checkpoint must not resurrect reconciled targets"
    );
}

#[test]
fn fresh_turn_does_not_inherit_another_turn_checkpoint_or_prompt_claims() {
    let (db, original, _, saved) = fixture();
    resume(&db, &original, &saved);
    let forged_prompt = serde_json::to_string(&json!({"workflowIr":saved})).unwrap();
    let fresh = run(&db, &original.conversation_id, &forged_prompt);
    assert!(
        load_desktop_resume_workflow(&db, Some(&fresh.conversation_id), Some(&fresh.turn_id))
            .unwrap()
            .is_none()
    );
    assert!(
        load_desktop_resume_workflow(&db, Some("other-conversation"), Some(&original.turn_id))
            .is_err()
    );
}

#[test]
fn saved_run_plan_restores_pending_targets_for_interaction_continuations() {
    let (db, run, plan, saved) = fixture();
    db.update_agent_task_run_progress(
        &run.id,
        Some("awaiting_user_input"),
        None,
        None,
        None,
        Some(&saved.task_plan_checkpoint(&plan)),
        None,
    )
    .unwrap();
    let restored =
        load_desktop_resume_workflow(&db, Some(&run.conversation_id), Some(&run.turn_id))
            .unwrap()
            .unwrap();
    assert_eq!(restored.checkpoint.desktop_observation_targets.len(), 2);
}

#[test]
fn malformed_or_cross_turn_checkpoint_cannot_silently_drop_desktop_obligations() {
    for corruption in [
        "{",
        "{}",
        r#"{"run":{"id":"other","turnId":"other","conversationId":"other"}}"#,
    ] {
        let (db, run, _, saved) = fixture();
        let checkpoint = resume(&db, &run, &saved);
        db.conn()
            .execute(
                "UPDATE task_resume_checkpoints SET state_json=?2 WHERE id=?1",
                rusqlite::params![checkpoint, corruption],
            )
            .unwrap();
        assert!(
            load_desktop_resume_workflow(&db, Some(&run.conversation_id), Some(&run.turn_id))
                .is_err()
        );
    }
    let (db, run, _, mut saved) = fixture();
    saved.checkpoint.desktop_observation_targets[0].window_id = 0;
    resume(&db, &run, &saved);
    assert!(
        load_desktop_resume_workflow(&db, Some(&run.conversation_id), Some(&run.turn_id)).is_err()
    );
}

#[test]
fn unlaunched_checkpoint_and_modified_continuation_are_not_resume_authorities() {
    let (db, run, _, saved) = fixture();
    db.create_task_resume_checkpoint_with_state(
        &run.id,
        "periodic",
        Some(&json!({"workflowIr":saved})),
    )
    .unwrap();
    assert!(
        load_desktop_resume_workflow(&db, Some(&run.conversation_id), Some(&run.turn_id))
            .unwrap()
            .is_none()
    );
    let checkpoint = resume(&db, &run, &saved);
    db.conn().execute("UPDATE messages SET content='changed' WHERE id=(SELECT response_message_id FROM task_resume_checkpoints WHERE id=?1)", [&checkpoint]).unwrap();
    assert!(
        load_desktop_resume_workflow(&db, Some(&run.conversation_id), Some(&run.turn_id)).is_err()
    );
}

#[test]
fn resume_preserves_original_close_contract_instead_of_prompt_derived_permission() {
    let (db, run, _, mut saved) = fixture();
    saved.completion_contract.desktop_terminal_closure_evidence = true;
    resume(&db, &run, &saved);
    let next_plan = plan("Continue reviewing");
    let mut next = None;
    restore_pending_desktop_evidence(
        &db,
        Some(&run.conversation_id),
        Some(&run.turn_id),
        &mut next,
        &next_plan,
        &profile(),
        false,
    )
    .unwrap();
    assert!(
        next.unwrap()
            .completion_contract
            .desktop_terminal_closure_evidence
    );
}

#[test]
fn nexus_resume_keeps_successful_reconnaissance_and_recovers_late_owned_receipts() {
    use crate::workflow_ir::{WorkflowDeliverable, WorkflowNodeStatus};
    let (db, run, _, _) = fixture_for("Implement and verify a complex cross-module refactor");
    let task = build_task_plan(TaskPlanningInput::for_route(
        &run.title,
        "CodebaseOperation",
        false,
        0,
    ));
    let mut saved = compile_workflow_ir(&task, &profile(), true).unwrap();
    let source = crate::workflow_verification::VerificationSourceState {
        digest: "current".into(),
        pending: false,
    };
    let ids = saved.ready_node_ids();
    assert_eq!(ids.len(), 2);
    saved.start_node(&ids[0]).unwrap();
    saved
        .complete_node(
            &ids[0],
            WorkflowDeliverable {
                claims: vec!["Observed implementation".into()],
                evidence: vec!["source.rs".into()],
                files_touched: vec![],
                tests: vec![],
                uncertainties: vec![],
            },
        )
        .unwrap();
    saved.start_node(&ids[1]).unwrap();
    let mut checkpoint = saved.task_plan_checkpoint(&task);
    checkpoint["completedToolRounds"] = json!(1);
    checkpoint["maxIterations"] = json!(2);
    db.update_agent_task_run_progress(
        &run.id,
        Some("running"),
        None,
        None,
        None,
        Some(&checkpoint),
        None,
    )
    .unwrap();
    let row = db
        .create_agent_subtask_run(
            &run.id,
            &ids[1],
            "Research",
            Some(&json!({"kind":"subagent_run","workerId":ids[1]})),
            None,
        )
        .unwrap();
    db.finish_agent_subtask_run(&row.id, "completed", Some(&json!({"run":{"id":ids[1],"isError":false,"result":"Late evidence retained","evidenceHandoff":[{"path":"late.rs"}],"verificationSource":source}})), None).unwrap();
    // Simulate a pre-upgrade crash whose checkpoint predates worker admission.
    checkpoint["workflowIr"]["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|node| node["id"] == ids[1])
        .for_each(|node| {
            node["attempts"] = json!(0);
            node["status"] = json!("pending");
        });
    db.update_agent_task_run_progress(&run.id, None, None, None, None, Some(&checkpoint), None)
        .unwrap();
    let mut restored =
        load_execution_resume_state(&db, Some(&run.conversation_id), Some(&run.turn_id))
            .unwrap()
            .unwrap();
    assert_eq!(restored.workflow.as_ref().unwrap().id, saved.id);
    assert_eq!(restored.completed_tool_rounds, 1);
    assert_eq!(restored.max_iterations, Some(2));
    assert_eq!(restored.plan.as_ref().unwrap().objective, task.objective);
    reconcile_resumed_reconnaissance(
        &db,
        Some(&run.turn_id),
        restored.workflow.as_mut().unwrap(),
        &source,
    )
    .unwrap();
    assert!(restored
        .workflow
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter(|node| ids.contains(&node.id))
        .all(|node| node.status == WorkflowNodeStatus::Succeeded && node.attempts == 1));
    assert!(restored
        .workflow
        .as_ref()
        .unwrap()
        .reconnaissance_batch_arguments(&task.objective)
        .is_none());
}

#[test]
fn unresolved_resumed_reconnaissance_is_not_replayed_or_reported_as_success() {
    use crate::workflow_ir::WorkflowNodeStatus;
    let (db, run, _, _) = fixture_for("Implement and verify a complex cross-module refactor");
    let task = build_task_plan(TaskPlanningInput::for_route(
        &run.title,
        "CodebaseOperation",
        false,
        0,
    ));
    let mut workflow = compile_workflow_ir(&task, &profile(), true).unwrap();
    let ids = workflow.ready_node_ids();
    workflow.start_node(&ids[0]).unwrap();
    reconcile_resumed_reconnaissance(
        &db,
        Some(&run.turn_id),
        &mut workflow,
        &crate::workflow_verification::VerificationSourceState {
            digest: "current".into(),
            pending: false,
        },
    )
    .unwrap();
    let interrupted = workflow
        .nodes
        .iter()
        .find(|node| node.id == ids[0])
        .unwrap();
    assert_eq!(interrupted.status, WorkflowNodeStatus::Failed);
    assert_eq!(interrupted.attempts, 1);
    assert!(!workflow.ready_node_ids().contains(&ids[0]));
    assert!(workflow.ready_node_ids().contains(&ids[1]));
}

#[tokio::test]
async fn late_reconnaissance_requires_the_current_authoritative_workspace() {
    use crate::{workflow_ir::WorkflowNodeStatus, workspace::Workspace};
    for missing in [false, true] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source.rs"), "A").unwrap();
        let workspace = Workspace {
            roots: vec![root.path().to_string_lossy().into()],
        };
        let (db, run, _, _) = fixture_for("Implement and verify a complex cross-module refactor");
        let task = build_task_plan(TaskPlanningInput::for_route(
            &run.title,
            "CodebaseOperation",
            false,
            0,
        ));
        let mut workflow = compile_workflow_ir(&task, &profile(), true).unwrap();
        let ids = workflow.ready_node_ids();
        let before = crate::workflow_verification::capture(
            &db,
            Some(&run.conversation_id),
            Some(&run.turn_id),
            &[],
            Some(&workspace),
        )
        .await
        .unwrap();
        for id in &ids {
            workflow.start_node(id).unwrap();
        }
        let row = db
            .create_agent_subtask_run(
                &run.id,
                &ids[0],
                "Research",
                Some(&json!({"workerId":ids[0]})),
                None,
            )
            .unwrap();
        db.finish_agent_subtask_run(&row.id, "completed", Some(&json!({"run":{"id":ids[0],"isError":false,"result":"Stale advice","evidenceHandoff":[{"path":"source.rs"}],"verificationSource":if missing {None} else {Some(&before)}}})), None).unwrap();
        std::fs::write(root.path().join("source.rs"), "B").unwrap();
        let current = crate::workflow_verification::capture(
            &db,
            Some(&run.conversation_id),
            Some(&run.turn_id),
            &[],
            Some(&workspace),
        )
        .await
        .unwrap();
        assert_ne!(before, current);
        let row = db
            .create_agent_subtask_run(
                &run.id,
                &ids[1],
                "Research",
                Some(&json!({"workerId":ids[1]})),
                None,
            )
            .unwrap();
        db.finish_agent_subtask_run(&row.id, "completed", Some(&json!({"run":{"id":ids[1],"isError":false,"result":"Current advice","verificationSource":current}})), None).unwrap();
        reconcile_resumed_reconnaissance(&db, Some(&run.turn_id), &mut workflow, &current).unwrap();
        let stale = workflow
            .nodes
            .iter()
            .find(|node| node.id == ids[0])
            .unwrap();
        assert_eq!(stale.status, WorkflowNodeStatus::Failed);
        assert!(stale.deliverable.as_ref().unwrap().claims.is_empty());
        assert_eq!(
            workflow
                .nodes
                .iter()
                .find(|node| node.id == ids[1])
                .unwrap()
                .status,
            WorkflowNodeStatus::Succeeded
        );
        assert!(!workflow
            .evidence_ledger
            .iter()
            .any(|entry| entry.node_id == ids[0]));
        assert!(workflow
            .reconnaissance_batch_arguments(&task.objective)
            .is_none());
    }
}
