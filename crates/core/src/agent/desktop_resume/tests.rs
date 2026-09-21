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
