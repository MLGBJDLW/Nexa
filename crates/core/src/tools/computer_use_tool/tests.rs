use super::*;

#[tokio::test]
async fn desktop_stop_blocks_only_the_owned_turn_and_keeps_chat_token_live() {
    let conversation = format!("computer-stop-{}", uuid::Uuid::new_v4());
    let chat = crate::agent::CancellationToken::new();
    let session = control_session(Some(&conversation), Some("turn-a"));
    let other = control_session(Some(&conversation), Some("turn-b"));
    stop_desktop_session(&conversation, "turn-a");
    assert!(session.stopped.is_cancelled());
    assert!(session.input_stopped.load(AtomicOrdering::Acquire));
    assert!(!other.stopped.is_cancelled());
    assert!(!chat.is_cancelled());
    let db = crate::db::Database::open_memory().unwrap();
    for tool in [
        &ComputerObserveTool as &dyn Tool,
        &ComputerControlTool as &dyn Tool,
    ] {
        let context = crate::tools::ToolExecutionContext::new("blocked", "{}", &db, &[])
            .with_conversation_id(Some(&conversation))
            .with_turn_id(Some("turn-a"));
        let result = tool.execute(context).await.unwrap();
        assert!(result.is_error);
        assert!(result
            .llm_context_content()
            .contains("stopped Computer Use"));
    }
    release_desktop_session(&conversation, "turn-a");
    release_desktop_session(&conversation, "turn-b");
}

fn wait_capture_fixture(level: u8) -> CapturedWindow {
    let image = image::RgbImage::from_pixel(32, 32, image::Rgb([level; 3]));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    CapturedWindow {
        snapshot: serde_json::from_value(serde_json::json!({
            "id":42,"pid":7,"processStartedAt100ns":123,"executablePathHash":"fixture",
            "windowClass":"Editor","sessionId":1,"appName":"Editor","title":"Document",
            "x":0,"y":0,"width":32,"height":32,"minimized":false,"maximized":false,"focused":true,
        }))
        .unwrap(),
        png: png.into_inner(),
        image_width: 32,
        image_height: 32,
        native_image_width: 32,
        native_image_height: 32,
        elements: Vec::new(),
        semantic_enabled: false,
        semantic_error: None,
        annotated_png: None,
        observation_captured_at_ms: 0,
    }
}

fn wait_observed_fixture() -> ObservedWindow {
    let mut capture = wait_capture_fixture(0);
    capture.snapshot.input_epoch = super::super::computer_input::window_epoch(capture.snapshot.id);
    ObservedWindow {
        snapshot: capture.snapshot,
        image_width: Some(32),
        image_height: Some(32),
        native_image_width: Some(32),
        native_image_height: Some(32),
        screenshot_signature: screenshot_signature(&capture.png),
        screenshot_guard: None,
        elements: Vec::new(),
    }
}

#[test]
fn computer_receipt_requires_comparable_evidence_to_claim_unchanged() {
    use crate::observed_interaction::{InteractionCommit, ObservedEvidence};
    let commit = InteractionCommit::default();
    let mut outcome = ControlOutcome {
        summary: "Restored a minimized window".to_string(),
        capture: Some(wait_capture_fixture(0)),
        observation_error: None,
        cursor_position: None,
        target_verified: true,
        state_changed: false,
        verification: VisualVerification {
            before_hash: "unavailable_before_restore".to_string(),
            after_hash: Some("captured".to_string()),
            difference: None,
            stable_samples: 2,
            sampled_frames: 3,
            elapsed_ms: 200,
        },
        route: "foreground",
        delivery: "native",
        effect: "delivered_unverified",
        window_closed: false,
        modal_owner_handoff: None,
    };
    assert_eq!(
        outcome.interaction(&commit).evidence,
        ObservedEvidence::Unavailable
    );
    outcome.verification.difference = Some(VisualDifference {
        changed_pixel_ratio: 0.0,
        mean_absolute_difference: 0.0,
        materially_changed: false,
    });
    assert_eq!(
        outcome.interaction(&commit).evidence,
        ObservedEvidence::Unchanged
    );
    outcome.verification.difference = None;
    outcome.state_changed = true;
    assert_eq!(
        outcome.interaction(&commit).evidence,
        ObservedEvidence::Changed
    );
    outcome.capture = None;
    assert_eq!(
        outcome.interaction(&commit).evidence,
        ObservedEvidence::Unavailable
    );
    outcome.window_closed = true;
    assert_eq!(
        outcome.interaction(&commit).evidence,
        ObservedEvidence::TargetClosed
    );
}

#[test]
fn computer_delivery_fields_agree_after_noop_and_partial_semantic_failure() {
    use crate::observed_interaction::{ObservedEvidence, RequestedCondition};
    let tracker = ControlCommitTracker::default();
    tracker.mark_observation_consumed();
    let noop = tracker.interaction.completed(
        ObservedEvidence::Unchanged,
        RequestedCondition::NotRequested,
    );
    let fields = with_control_delivery(serde_json::json!({}), noop);
    assert_eq!(fields["interaction"]["delivery"], "notCommitted");
    assert_eq!(fields["inputDelivered"], false);
    assert_eq!(fields["deliveryStatus"], "not_committed");
    assert_eq!(fields["effectMayHaveOccurred"], false);

    tracker.mark().unwrap();
    assert!(tracker
        .delivered::<()>(Err(CoreError::Internal("provider failed".into())))
        .is_err());
    let uncertain = with_control_delivery(serde_json::json!({}), tracker.interaction.failure());
    assert!(uncertain["inputDelivered"].is_null());
    assert_eq!(uncertain["deliveryStatus"], "uncertain");
    tracker.delivered(Ok(())).unwrap();
    let failure = tracker
        .result::<()>(Err(CoreError::Internal("readback failed".into())))
        .unwrap_err();
    let sent = with_control_delivery(
        serde_json::json!({}),
        failure.interaction(&tracker.interaction),
    );
    assert_eq!(sent["interaction"]["delivery"], "inputSent");
    assert_eq!(sent["inputDelivered"], true);
    assert_eq!(sent["deliveryStatus"], "delivered");
    assert_eq!(sent["interaction"]["evidence"], "unavailable");
}

#[tokio::test]
async fn computer_wait_change_is_proved_by_the_returned_frame() {
    let mut probes = 0;
    let mut finals = 0;
    let options = CaptureOptions {
        include_elements: true,
        max_elements: 20,
        mode: CaptureMode::Raw,
    };
    let result = wait_for_window_change(
        &wait_observed_fixture(),
        Duration::from_secs(1),
        Duration::from_millis(10),
        options,
        |snapshot, options| {
            assert_eq!(snapshot.id, 42);
            let level = if options.include_elements {
                finals += 1;
                0
            } else {
                probes += 1;
                255
            };
            async move { Ok(wait_capture_fixture(level)) }
        },
    )
    .await
    .unwrap();
    assert_eq!((probes, finals), (1, 1));
    assert_eq!(result.sampled_frames, 1);
    assert!(result.transient_change_observed);
    assert!(
        !result.changed,
        "the changed intermediate frame was gone before final capture"
    );
    assert!(!result.difference.unwrap().materially_changed);
}

#[tokio::test]
async fn computer_wait_cancellation_stops_new_probes_and_final_capture() {
    use std::sync::atomic::AtomicUsize;
    let probes = AtomicUsize::new(0);
    let finals = AtomicUsize::new(0);
    let observed = wait_observed_fixture();
    let options = CaptureOptions {
        include_elements: true,
        max_elements: 20,
        mode: CaptureMode::Raw,
    };
    let waiting = wait_for_window_change(
        &observed,
        Duration::from_secs(10),
        Duration::from_millis(100),
        options,
        |_, options| {
            let probes = &probes;
            let finals = &finals;
            async move {
                if options.include_elements {
                    finals.fetch_add(1, AtomicOrdering::SeqCst);
                } else {
                    probes.fetch_add(1, AtomicOrdering::SeqCst);
                }
                Ok(wait_capture_fixture(0))
            }
        },
    );
    assert!(tokio::time::timeout(Duration::from_millis(30), waiting)
        .await
        .is_err());
    let after_cancellation = probes.load(AtomicOrdering::SeqCst);
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(after_cancellation, 1);
    assert_eq!(probes.load(AtomicOrdering::SeqCst), after_cancellation);
    assert_eq!(finals.load(AtomicOrdering::SeqCst), 0);
}

#[test]
fn released_physical_input_invalidates_an_unchanged_observation() {
    let mut observed = wait_observed_fixture();
    observed.snapshot.id = 0x7fff_1000;
    observed.snapshot.input_epoch =
        super::super::computer_input::window_epoch(observed.snapshot.id);
    let id = remember_observation(Some("physical-revision-test"), vec![observed.clone()]).unwrap();
    super::super::computer_input::record(observed.snapshot.id, false, false);
    assert!(
        claim_observed_window(Some("physical-revision-test"), &id, observed.snapshot.id)
            .unwrap_err()
            .to_string()
            .contains("user interacted")
    );
    observed.snapshot.input_epoch =
        super::super::computer_input::window_epoch(observed.snapshot.id);
    let id = remember_observation(Some("physical-revision-test"), vec![observed.clone()]).unwrap();
    assert!(
        claim_observed_window(Some("physical-revision-test"), &id, observed.snapshot.id).is_ok()
    );
}

#[test]
fn escape_stops_only_the_active_input_owner_and_preserves_other_sessions() {
    let active = ControlCommitTracker::default();
    let queued = ControlCommitTracker::default();
    active.escape_baseline.store(4, AtomicOrdering::Release);
    queued.escape_baseline.store(4, AtomicOrdering::Release);
    active
        .worker_state
        .store(WORKER_STARTED, AtomicOrdering::Release);
    assert!(!queued.stop_if_escape_changed(5));
    assert!(active.stop_if_escape_changed(5));
    assert!(active.session_cancel.is_cancelled());
    assert!(!queued.session_cancel.is_cancelled());
    assert!(active.session_stopped.load(AtomicOrdering::Acquire));
    assert!(!queued.session_stopped.load(AtomicOrdering::Acquire));
    let finished = ControlCommitTracker::default();
    finished
        .worker_state
        .store(WORKER_FINISHED, AtomicOrdering::Release);
    assert!(!finished.stop_if_escape_changed(5));
}

#[test]
fn cancelled_queued_computer_observation_never_captures() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let captures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    runtime.block_on(async {
        let (release, blocked) = std::sync::mpsc::channel();
        let (started, ready) = tokio::sync::oneshot::channel();
        let occupied = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            blocked.recv_timeout(Duration::from_secs(3)).unwrap();
        });
        ready.await.unwrap();
        let counted = Arc::clone(&captures);
        let observation = blocking(move || {
            counted.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(())
        });
        assert!(tokio::time::timeout(Duration::from_millis(20), observation)
            .await
            .is_err());
        release.send(()).unwrap();
        occupied.await.unwrap();
        // A barrier behind the cancelled job proves the worker drained it.
        tokio::task::spawn_blocking(|| ()).await.unwrap();
    });
    assert_eq!(captures.load(AtomicOrdering::SeqCst), 0);
}

#[test]
fn semantic_values_are_unicode_bounded_and_keep_full_freshness_private() {
    let text = "中文🙂".repeat(600);
    let mut remaining = 1_500;
    let first = UiElementValue::bounded(&text, &mut remaining);
    let second = UiElementValue::bounded(&text, &mut remaining);
    assert_eq!(first.text.chars().count(), 1_024);
    assert_eq!(second.text.chars().count(), 476);
    assert_eq!(remaining, 0);
    assert_eq!(first.total_characters, 1_800);
    assert!(first.truncated && second.truncated);
    assert_eq!(first.fingerprint, second.fingerprint);
    assert!(!serde_json::to_string(&first)
        .unwrap()
        .contains(&first.fingerprint));
    let cleared = UiElementValue::bounded("", &mut remaining);
    assert!(!cleared.truncated);
    assert_eq!(cleared.total_characters, 0);
}

#[tokio::test]
async fn window_inventory_pages_filtered_results_and_waits_for_the_requested_process() {
    let args: ObserveArgs = serde_json::from_value(serde_json::json!({
        "action":"wait_for_window", "process_id":7, "app_name":"editor", "max_results":1, "timeout_ms":200
    })).unwrap();
    let target = WindowSnapshot {
        input_epoch: Default::default(),
        id: 42,
        pid: 7,
        process_started_at_100ns: 123,
        executable_path_hash: "hash".into(),
        window_class: "EditorWindow".into(),
        session_id: 1,
        app_name: "Editor".into(),
        title: "年度预算 RÉSUMÉ 2026 - Editor".into(),
        x: 0,
        y: 0,
        width: 800,
        height: 600,
        minimized: false,
        maximized: false,
        focused: false,
        modal_owner: None,
    };
    let mut other = target.clone();
    other.pid = 8;
    let mut calls = 0;
    let found = matching_window_inventory(&args, true, || {
        calls += 1;
        std::future::ready(Ok(if calls == 1 {
            vec![other.clone()]
        } else {
            vec![other.clone(), target.clone()]
        }))
    })
    .await
    .unwrap();
    assert_eq!(found.windows, vec![target.clone()]);
    assert_eq!(calls, 2);
    let mut windows = vec![other];
    windows.extend((0..151).map(|index| {
        let mut window = target.clone();
        window.id = 100 + index;
        window
    }));
    for (offset, expected_count, expected_next) in
        [(0, 100, Some(100)), (100, 51, None), (151, 0, None)]
    {
        let args: ObserveArgs = serde_json::from_value(serde_json::json!({
            "action":"list_windows", "process_id":7, "app_name":"editor", "max_results":100, "offset":offset
        })).unwrap();
        let page =
            matching_window_inventory(&args, false, || std::future::ready(Ok(windows.clone())))
                .await
                .unwrap();
        assert_eq!(page.windows.len(), expected_count);
        assert_eq!(page.total_matches, 151);
        assert_eq!(
            page.coverage()["nextOffset"],
            serde_json::json!(expected_next)
        );
        assert_eq!(page.coverage()["hasMore"], expected_next.is_some());
        if let Some(first) = page.windows.first() {
            assert_eq!(
                first.id,
                100 + offset as u64,
                "filtering must precede pagination"
            );
        }
    }
    let args: ObserveArgs = serde_json::from_value(
        serde_json::json!({"action":"wait_for_window","process_id":7,"offset":1}),
    )
    .unwrap();
    assert!(matching_window_inventory(&args, true, || async {
        panic!("wait cannot use an inventory cursor")
    })
    .await
    .is_err());
}

#[tokio::test]
async fn window_wait_times_out_even_when_inventory_stalls_and_rejects_unscoped_waits() {
    let args: ObserveArgs = serde_json::from_value(serde_json::json!({
        "action":"wait_for_window", "process_id":7, "timeout_ms":100
    }))
    .unwrap();
    let found = tokio::time::timeout(
        Duration::from_millis(500),
        matching_window_inventory(&args, true, || std::future::pending()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(found.windows.is_empty());
    let args: ObserveArgs =
        serde_json::from_value(serde_json::json!({"action":"wait_for_window"})).unwrap();
    assert!(matching_window_inventory(&args, true, || async {
        panic!("unscoped wait must not enumerate")
    })
    .await
    .is_err());
}

#[tokio::test]
async fn computer_control_fails_closed_without_persistent_action_receipts() {
    let db = crate::db::Database::open_memory().unwrap();
    let runtime = crate::activity::ActivityRuntime::new();
    let source_scope = Vec::new();
    let result = ComputerControlTool
        .execute(
            crate::tools::ToolExecutionContext::new("call", "{}", &db, &source_scope)
                .with_activity_runtime(&runtime),
        )
        .await
        .expect("pre-commit rejection is a structured tool result");
    assert!(result.is_error);
    let artifacts = result.artifacts.expect("typed failure artifacts");
    assert_eq!(artifacts["code"], "computer_control_runtime_unavailable");
    assert_eq!(artifacts["sideEffect"], "not_started");
    assert!(artifacts.get("effectMayHaveOccurred").is_none());
    assert!(artifacts.get("failurePhase").is_none());
    assert_eq!(artifacts["observationConsumed"], false);
}

#[test]
fn invalid_image_coordinates_are_rejected_without_consuming_the_observation() {
    let fixture = wait_observed_fixture();
    let token = remember_observation(Some("coordinate-preflight"), vec![fixture.clone()]).unwrap();
    let args: ControlArgs = serde_json::from_value(serde_json::json!({
        "action": "click", "observation_id": token, "window_id": fixture.snapshot.id,
        "x": fixture.image_width.unwrap(), "y": 0,
    }))
    .unwrap();
    let observation =
        observed_window(Some("coordinate-preflight"), &token, fixture.snapshot.id).unwrap();
    let error = validate_observed_coordinates(&args, &observation).unwrap_err();
    let failure = ControlFailure::pre_commit_as(PreCommitFailureKind::InvalidCoordinates, error);
    let result = control_failure_result("coordinates", &failure, &Default::default());
    assert_eq!(
        result.artifacts.as_ref().unwrap()["code"],
        "computer_coordinates_out_of_bounds"
    );
    assert_eq!(
        result.artifacts.as_ref().unwrap()["observationConsumed"],
        false
    );
    assert!(
        claim_observed_window(Some("coordinate-preflight"), &token, fixture.snapshot.id).is_ok()
    );
}

#[test]
fn control_commit_phase_is_explicit_monotonic_and_independent_of_error_text() {
    let tracker = ControlCommitTracker::default();
    let before = tracker.failure_as(
        PreCommitFailureKind::RuntimeUnavailable,
        CoreError::Internal("effect is uncertain but no input was sent".to_string()),
    );
    let before_result = control_failure_result("before", &before, &tracker.interaction);
    let before_artifacts = before_result.artifacts.expect("pre-commit artifacts");
    assert_eq!(
        before_artifacts["code"],
        "computer_control_runtime_unavailable"
    );
    assert_eq!(before_artifacts["sideEffect"], "not_started");

    tracker.mark_observation_consumed();
    let consumed = tracker.failure_as(
        PreCommitFailureKind::RuntimeUnavailable,
        CoreError::Internal("worker stopped before OS input".to_string()),
    );
    let consumed_artifacts = control_failure_result("consumed", &consumed, &tracker.interaction)
        .artifacts
        .expect("consumed observation artifacts");
    assert_eq!(consumed_artifacts["sideEffect"], "not_started");
    assert_eq!(consumed_artifacts["observationConsumed"], true);

    tracker.mark().unwrap();
    let after = tracker.failure_as(
        PreCommitFailureKind::InvalidAction,
        CoreError::InvalidInput("no input was sent".to_string()),
    );
    let after_result = control_failure_result("after", &after, &tracker.interaction);
    let after_artifacts = after_result.artifacts.expect("post-commit artifacts");
    assert_eq!(after_artifacts["code"], "computer_action_uncertain");
    assert_eq!(after_artifacts["sideEffect"], "may_have_occurred");
    assert!(after_artifacts.get("effectMayHaveOccurred").is_none());
    assert!(after_artifacts.get("failurePhase").is_none());

    tracker.interaction.confirm_input_sent();
    let confirmed = control_failure_result("confirmed", &after, &tracker.interaction)
        .artifacts
        .unwrap();
    assert_eq!(
        confirmed["expectedFormat"]["interaction"]["delivery"],
        "inputSent"
    );
    assert_eq!(
        confirmed["expectedFormat"]["interaction"]["evidence"],
        "unavailable"
    );
    assert_eq!(confirmed["sideEffect"], "may_have_occurred");

    tracker.mark().unwrap();
    assert!(tracker.effect_may_have_occurred());
}

#[test]
fn provider_call_ids_are_namespaced_by_turn_and_observation_for_persistent_activities() {
    let first =
        computer_control_activity_id(Some("conversation"), Some("turn-a"), "call_0", "obs-a");
    let second =
        computer_control_activity_id(Some("conversation"), Some("turn-b"), "call_0", "obs-a");
    let next_round =
        computer_control_activity_id(Some("conversation"), Some("turn-a"), "call_0", "obs-b");
    let retry =
        computer_control_activity_id(Some("conversation"), Some("turn-a"), "call_0", "obs-a");

    assert_ne!(first, second);
    assert_ne!(first, next_round);
    assert_eq!(first, retry);
    assert!(first.contains("turn-a"));
}

#[test]
fn observation_tokens_are_scoped_to_the_listed_window() {
    let snapshot = WindowSnapshot {
        input_epoch: Default::default(),
        id: 42,
        pid: 7,
        process_started_at_100ns: 123,
        executable_path_hash: "exe-hash".to_string(),
        window_class: "EditorWindow".to_string(),
        session_id: 1,
        app_name: "Editor".to_string(),
        title: "Document".to_string(),
        x: 10,
        y: 20,
        width: 800,
        height: 600,
        minimized: false,
        maximized: false,
        focused: true,
        modal_owner: None,
    };
    let observation_id = remember_observation(
        Some("conversation-1"),
        vec![ObservedWindow {
            snapshot: snapshot.clone(),
            image_width: Some(800),
            image_height: Some(600),
            native_image_width: Some(800),
            native_image_height: Some(600),
            screenshot_signature: None,
            screenshot_guard: None,
            elements: Vec::new(),
        }],
    )
    .unwrap();

    assert_eq!(
        observed_window(Some("conversation-1"), &observation_id, snapshot.id)
            .unwrap()
            .snapshot,
        snapshot
    );
    assert!(observed_window(Some("conversation-1"), &observation_id, 99).is_err());
    assert!(observed_window(Some("conversation-2"), &observation_id, snapshot.id).is_err());
}

#[test]
fn approval_preview_is_exact_conversation_scoped_and_hides_typed_text() {
    let snapshot = WindowSnapshot {
        input_epoch: Default::default(),
        id: 420,
        pid: 7,
        process_started_at_100ns: 123,
        executable_path_hash: "exe-hash".to_string(),
        window_class: "EditorWindow".to_string(),
        session_id: 1,
        app_name: "Editor".to_string(),
        title: "Quarterly plan".to_string(),
        x: 10,
        y: 20,
        width: 800,
        height: 600,
        minimized: false,
        maximized: false,
        focused: true,
        modal_owner: None,
    };
    let observation_id = remember_observation(
        Some("approval-conversation"),
        vec![ObservedWindow {
            snapshot: snapshot.clone(),
            image_width: Some(800),
            image_height: Some(600),
            native_image_width: Some(800),
            native_image_height: Some(600),
            screenshot_signature: Some(vec![0; 256]),
            screenshot_guard: None,
            elements: vec![UiElementSnapshot {
                id: "e1".to_string(),
                role: "textbox".to_string(),
                name: "Plan title".to_string(),
                automation_id: "title".to_string(),
                bounds: ElementBounds {
                    x: 40,
                    y: 50,
                    width: 240,
                    height: 32,
                },
                enabled: true,
                focused: false,
                keyboard_focusable: true,
                interactive: true,
                password: false,
                actions: vec!["type_text".to_string()],
                state: UiElementState::default(),
                screen_bounds: ElementBounds {
                    x: 50,
                    y: 70,
                    width: 240,
                    height: 32,
                },
            }],
        }],
    )
    .unwrap();
    let control_args = serde_json::json!({
        "action": "type_text",
        "observation_id": observation_id,
        "window_id": snapshot.id,
        "element_id": "e1",
        "text": "secret"
    });
    let message = ComputerControlTool
        .confirmation_message_in_context(&control_args, Some("approval-conversation"))
        .unwrap()
        .unwrap();
    for expected in [
        "Editor",
        "Quarterly plan",
        "Plan title",
        "[40, 50, 240x32]",
        "6 character(s)",
    ] {
        assert!(message.contains(expected), "missing {expected}: {message}");
    }
    assert!(!message.contains("secret"));
    let durable_message = ComputerControlTool
        .confirmation_message_for_persistence_in_context(
            &control_args,
            Some("approval-conversation"),
        )
        .unwrap()
        .unwrap();
    assert!(durable_message.contains("Editor"));
    assert!(durable_message.contains("window title is redacted"));
    assert!(durable_message.contains("element name is redacted"));
    assert!(durable_message.contains("[40, 50, 240x32]"));
    assert!(durable_message.contains("6 character(s)"));
    assert!(!durable_message.contains("Quarterly plan"));
    assert!(!durable_message.contains("Plan title"));
    assert!(!durable_message.contains("secret"));
    assert!(ComputerControlTool
        .confirmation_message_in_context(&control_args, Some("other-conversation"))
        .is_err());

    let invalid_click = serde_json::json!({
        "action": "click",
        "observation_id": control_args["observation_id"],
        "window_id": snapshot.id,
        "element_id": "e1"
    });
    let error = ComputerControlTool
        .confirmation_message_in_context(&invalid_click, Some("approval-conversation"))
        .expect_err("unadvertised semantic action must fail before approval");
    assert!(error.to_string().contains("did not advertise 'click'"));

    let observe_args = serde_json::json!({
        "action": "capture_window",
        "observation_id": control_args["observation_id"],
        "window_id": snapshot.id
    });
    let capture_message = ComputerObserveTool
        .confirmation_message_in_context(&observe_args, Some("approval-conversation"))
        .unwrap()
        .unwrap();
    assert!(capture_message.contains("Editor"));
    assert!(capture_message.contains("Quarterly plan"));
}

#[test]
fn observation_token_is_single_use_for_control() {
    let snapshot = WindowSnapshot {
        input_epoch: super::super::computer_input::window_epoch(84),
        id: 84,
        pid: 9,
        process_started_at_100ns: 456,
        executable_path_hash: "exe-hash-2".to_string(),
        window_class: "TestWindow".to_string(),
        session_id: 1,
        app_name: "TestApp".to_string(),
        title: "Test".to_string(),
        x: 0,
        y: 0,
        width: 100,
        height: 100,
        minimized: false,
        maximized: false,
        focused: false,
        modal_owner: None,
    };
    let observation_id = remember_observation(
        Some("conversation-once"),
        vec![ObservedWindow {
            snapshot: snapshot.clone(),
            image_width: Some(100),
            image_height: Some(100),
            native_image_width: Some(100),
            native_image_height: Some(100),
            screenshot_signature: Some(vec![0; 256]),
            screenshot_guard: None,
            elements: Vec::new(),
        }],
    )
    .unwrap();

    assert!(claim_observed_window(Some("conversation-once"), &observation_id, snapshot.id).is_ok());
    let replay =
        claim_observed_window(Some("conversation-once"), &observation_id, snapshot.id).unwrap_err();
    assert!(replay.to_string().contains("already used"));
}

#[tokio::test(start_paused = true)]
async fn desktop_input_arbiter_serializes_and_bounds_independent_runs() {
    let first = std::sync::Arc::clone(crate::browser_runtime::desktop_input_arbiter())
        .lock_owned()
        .await;
    assert!(crate::browser_runtime::desktop_input_arbiter()
        .try_lock()
        .is_err());
    let blocked = tokio::spawn(crate::browser_runtime::acquire_desktop_input_permit());
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(5)).await;
    assert!(blocked
        .await
        .expect("bounded arbiter task")
        .expect_err("busy input must fail instead of hanging")
        .to_string()
        .contains("after 5 seconds"));
    drop(first);
    assert!(crate::browser_runtime::desktop_input_arbiter()
        .try_lock()
        .is_ok());
}

#[test]
fn pointer_motion_has_visible_intermediate_frames_and_exact_scaled_endpoints() {
    for (from, to) in [((-1800, 240), (820, -500)), ((80, 10), (100, 35))] {
        let (duration, points) = pointer_motion_path(from, to, 1.5);
        assert!(duration >= Duration::from_millis(100));
        assert!(duration <= Duration::from_millis(360));
        assert_eq!(points.last(), Some(&to));
        assert!(points.iter().any(|point| *point != from && *point != to));
        for point in points {
            assert!((from.0.min(to.0)..=from.0.max(to.0)).contains(&point.0));
            assert!((from.1.min(to.1)..=from.1.max(to.1)).contains(&point.1));
        }
    }
    assert_eq!(
        pointer_motion_path((10, 10), (10, 10), 1.0),
        (Duration::ZERO, vec![(10, 10)])
    );
    assert_eq!(
        pointer_motion_path((0, 0), (600, 0), 2.0).0,
        pointer_motion_path((0, 0), (300, 0), 1.0).0
    );
}

#[test]
fn stopping_an_inflight_worker_signals_motion_without_erasing_committed_effects() {
    let state = Arc::new(AtomicU8::new(WORKER_STARTED));
    let tracker = ControlCommitTracker::default();
    let mut pending = PendingWorkerCancellation::new(Arc::clone(&state));
    pending.running_control = Some(Arc::clone(&tracker.cancelled));
    tracker.mark().unwrap();
    let worker = tracker.clone();
    drop(pending);
    assert_eq!(state.load(AtomicOrdering::Acquire), WORKER_STARTED);
    let failure = worker.failure(
        worker
            .ensure_running()
            .expect_err("remaining frames must stop"),
    );
    assert_eq!(
        failure.pre_commit_kind,
        Some(PreCommitFailureKind::Cancelled)
    );
    assert!(worker.effect_may_have_occurred());
}

#[test]
fn cancelled_pending_worker_cannot_transition_to_started() {
    let state = std::sync::Arc::new(AtomicU8::new(WORKER_PENDING));
    drop(PendingWorkerCancellation::new(std::sync::Arc::clone(
        &state,
    )));
    assert_eq!(state.load(AtomicOrdering::Acquire), WORKER_CANCELLED);
    assert!(state
        .compare_exchange(
            WORKER_PENDING,
            WORKER_STARTED,
            AtomicOrdering::AcqRel,
            AtomicOrdering::Acquire,
        )
        .is_err());
}

#[test]
fn physical_input_interruption_is_not_reported_as_an_explicit_stop() {
    let tracker = ControlCommitTracker::default();
    let window = 0xabc90;
    *tracker.input_watch.lock().unwrap() = Some((
        window,
        super::super::computer_input::window_epoch(window),
        false,
    ));
    tracker.mark_observation_consumed();
    super::super::computer_input::record(window, false, false);
    let error = tracker
        .ensure_running()
        .expect_err("new physical input invalidates the approved snapshot");
    let failure = tracker.failure(error);
    assert_eq!(
        control_failure_contract(&failure),
        ("computer_user_takeover", false)
    );
    assert!(!failure.phase.effect_may_have_occurred());
    assert!(failure.observation_consumed);
    // An explicit Stop still wins over a coincident input change.
    tracker.session_stopped.store(true, AtomicOrdering::Release);
    assert_eq!(
        control_failure_contract(&tracker.failure(tracker.ensure_running().unwrap_err())),
        ("computer_control_cancelled", false)
    );
}

#[test]
fn cancelled_unsettled_worker_never_claims_no_effect_and_blocks_late_commit() {
    for started in [false, true] {
        let tracker = ControlCommitTracker::default();
        if started {
            tracker
                .worker_state
                .store(WORKER_STARTED, AtomicOrdering::Release);
        }
        let failure = tracker.cancel_unsettled_worker(CoreError::Cancelled("stop".into()));
        assert_eq!(failure.phase.effect_may_have_occurred(), started);
        assert!(
            tracker.mark().is_err(),
            "UIA resolution must not commit after stop"
        );
        assert!(tracker
            .worker_state
            .compare_exchange(
                WORKER_PENDING,
                WORKER_STARTED,
                AtomicOrdering::AcqRel,
                AtomicOrdering::Acquire
            )
            .is_err());
    }
}

#[test]
fn dropping_one_call_does_not_poison_the_turn_but_explicit_stop_does() {
    let session = ControlSession::default();
    let tracker = ControlCommitTracker {
        session_stopped: session.input_stopped.clone(),
        ..Default::default()
    };
    let mut pending = PendingWorkerCancellation::new(tracker.worker_state.clone());
    pending.running_control = Some(tracker.cancelled.clone());
    drop(pending);
    assert!(tracker.ensure_running().is_err());
    let next = ControlCommitTracker {
        session_stopped: session.input_stopped.clone(),
        ..Default::default()
    };
    assert!(next.ensure_running().is_ok());
    session.input_stopped.store(true, AtomicOrdering::Release);
    assert!(next.mark().is_err());
}

#[test]
fn control_validation_accepts_normalized_targets_and_rejects_missing_semantic_ids() {
    let click = ControlArgs {
        action: "click".to_string(),
        approval_scope: None,
        delivery: None,
        drag_duration_ms: None,
        observation_id: "00000000-0000-4000-8000-000000000001".to_string(),
        window_id: 1,
        element_id: None,
        to_element_id: None,
        coordinate_space: Some("normalized_0_1".to_string()),
        x: Some(0.5),
        y: Some(1.0),
        to_x: None,
        to_y: None,
        button: None,
        click_count: Some(1),
        scroll_x: None,
        scroll_y: None,
        text: None,
        key_sequence: None,
        reason: None,
        include_elements: None,
        max_elements: None,
        capture_mode: None,
        wait_for_previous: None,
    };
    assert!(validate_control_args(&click, ControlAction::Click).is_ok());

    let mut invoke = click;
    invoke.action = "invoke".to_string();
    assert!(validate_control_args(&invoke, ControlAction::Invoke).is_err());

    invoke.action = "key".to_string();
    invoke.key_sequence = Some("win+l".to_string());
    assert!(validate_control_args(&invoke, ControlAction::Key).is_err());
}

#[test]
fn semantic_value_input_can_clear_fields_and_replace_multiline_documents() {
    for text in [String::new(), "中文🙂 line\r\n".repeat(120)] {
        let args: ControlArgs = serde_json::from_value(serde_json::json!({
            "action": "set_value", "observation_id": "00000000-0000-4000-8000-000000000001",
            "window_id": 1, "element_id": "e1", "text": text
        }))
        .unwrap();
        assert!(validate_control_args(&args, ControlAction::SetValue).is_ok());
    }
}

#[test]
fn background_delivery_never_accepts_coordinate_or_double_click_fallbacks() {
    let args = serde_json::json!({"action":"click", "observation_id":"00000000-0000-4000-8000-000000000001", "window_id":1, "delivery":"background", "element_id":"e1"});
    let valid: ControlArgs = serde_json::from_value(args.clone()).unwrap();
    assert!(validate_control_args(&valid, ControlAction::Click).is_ok());
    for patch in [
        serde_json::json!({"click_count":2}),
        serde_json::json!({"button":"right"}),
        serde_json::json!({"element_id":null,"x":10,"y":10}),
    ] {
        let mut invalid = args.clone();
        invalid
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let invalid: ControlArgs = serde_json::from_value(invalid).unwrap();
        assert!(validate_control_args(&invalid, ControlAction::Click).is_err());
    }
}

#[test]
fn reusable_grants_bind_verified_process_window_and_task_not_observation_tokens() {
    let observe = |started| {
        remember_observation(
            Some("window-scope-test"),
            vec![ObservedWindow {
                snapshot: WindowSnapshot {
                    input_epoch: Default::default(),
                    id: 400,
                    pid: 7,
                    process_started_at_100ns: started,
                    executable_path_hash: "editor-binary".into(),
                    window_class: "Editor".into(),
                    session_id: 1,
                    app_name: "Editor".into(),
                    title: "Untitled".into(),
                    x: 0,
                    y: 0,
                    width: 640,
                    height: 480,
                    minimized: false,
                    maximized: false,
                    focused: false,
                    modal_owner: None,
                },
                image_width: None,
                image_height: None,
                native_image_width: None,
                native_image_height: None,
                screenshot_signature: None,
                screenshot_guard: None,
                elements: vec![],
            }],
        )
        .unwrap()
    };
    let bind = |token: &str, task| {
        let args = serde_json::json!({"action":"focus_window", "window_id":400,"observation_id":token,"approval_scope":"window_session"});
        let mut request = crate::approval::ApprovalRequest::new(
            "req",
            "computer_control",
            &args,
            crate::approval::ApprovalRisk::High,
            "Focus editor",
        );
        bind_window_session_approval(&mut request, &args, Some("window-scope-test"), Some(task))
            .unwrap();
        assert_eq!(request.target_kind, "desktop_window_task");
        request.permission_key
    };
    let first = observe(123);
    let next = observe(123);
    assert_eq!(bind(&first, "task-a"), bind(&next, "task-a"));
    assert_ne!(bind(&first, "task-a"), bind(&first, "task-b"));
    assert_ne!(bind(&first, "task-a"), bind(&observe(456), "task-a"));
}

#[test]
fn control_actions_always_require_confirmation() {
    let tool = ComputerControlTool;
    for action in [
        "focus_window",
        "move_mouse",
        "click",
        "drag",
        "scroll",
        "type_text",
        "key",
        "invoke",
        "set_value",
    ] {
        assert!(tool.requires_confirmation(&serde_json::json!({
            "action": action,
            "observation_id": "observation",
            "window_id": 1
        })));
    }
    let profile = tool.access_profile(&serde_json::json!({
        "action": "click",
        "observation_id": "observation",
        "window_id": 1,
        "x": 1,
        "y": 1
    }));
    assert!(profile.needs_approval);
    assert!(profile.can_access_network);
}

#[test]
fn perceptual_screenshot_validation_tolerates_incidental_pixels() {
    let expected = vec![100; (SCREENSHOT_SIGNATURE_EDGE * SCREENSHOT_SIGNATURE_EDGE * 3) as usize];
    let mut cursor_moved = expected.clone();
    cursor_moved[42] = 255;
    assert!(screenshot_signatures_match(&expected, &cursor_moved));

    let materially_changed = vec![220; expected.len()];
    assert!(!screenshot_signatures_match(&expected, &materially_changed));
}

#[test]
fn target_patch_guard_detects_local_changes_without_rejecting_distant_changes() {
    let expected = ScreenshotGuard {
        width: 32,
        height: 32,
        rgb: vec![100; 32 * 32 * 3],
    };
    let mut target_changed = expected.clone();
    for y in 12..20 {
        for x in 12..20 {
            let offset = (y * 32 + x) * 3;
            target_changed.rgb[offset..offset + 3].fill(220);
        }
    }
    assert!(!screenshot_guard_patch_matches(
        &expected,
        &target_changed,
        0.5,
        0.5
    ));

    let mut distant_change = expected.clone();
    distant_change.rgb[0..12].fill(220);
    assert!(screenshot_guard_patch_matches(
        &expected,
        &distant_change,
        0.5,
        0.5
    ));
}

#[test]
fn computer_observation_is_read_only_but_capture_requires_egress_consent() {
    let tool = ComputerObserveTool;
    assert!(!tool.requires_confirmation(&serde_json::json!({
        "action": "capture_window"
    })));
    assert!(tool.is_read_only(&serde_json::json!({
        "action": "capture_window"
    })));
    let profile = tool.access_profile(&serde_json::json!({
        "action": "capture_window"
    }));
    assert!(profile.needs_approval);
    assert!(profile.can_access_network);
    assert!(!profile.can_write);
    assert!(!profile.can_execute);

    let list_profile = tool.access_profile(&serde_json::json!({
        "action": "list_windows"
    }));
    assert!(!list_profile.needs_approval);
    assert!(!list_profile.can_access_network);

    let noncanonical_capture = serde_json::json!({ "action": " CAPTURE_WINDOW " });
    let noncanonical_profile = tool.access_profile(&noncanonical_capture);
    assert!(noncanonical_profile.needs_approval);
    assert!(noncanonical_profile.can_access_network);
    assert!(tool.confirmation_message(&noncanonical_capture).is_some());
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
#[test]
#[ignore = "requires an interactive Windows desktop"]
fn windows_computer_control_helper_window() {
    if std::env::var_os("NEXA_COMPUTER_USE_HELPER").is_none() {
        return;
    }
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{GetSysColorBrush, ScreenToClient, COLOR_WINDOW};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetDlgItem, GetMessageW,
        GetWindowLongPtrW, RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowLongPtrW,
        SetWindowTextW, ShowWindow, TranslateMessage, BM_GETCHECK, BN_CLICKED, BS_AUTOCHECKBOX,
        CW_USEDEFAULT, ES_AUTOVSCROLL, ES_MULTILINE, ES_PASSWORD, ES_READONLY, GWLP_USERDATA,
        HMENU, MSG, SW_SHOW, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND,
        WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    unsafe extern "system" fn helper_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, KillTimer, SetTimer, WM_APP, WM_TIMER,
        };
        if message == WM_APP + 99 {
            unsafe {
                SetTimer(Some(window), 909, 10, None);
            }
            return LRESULT(0);
        }
        if message == WM_TIMER && wparam.0 == 909 {
            let mut cursor = windows::Win32::Foundation::POINT::default();
            if unsafe { GetCursorPos(&mut cursor) }.is_ok()
                && unsafe { ScreenToClient(window, &mut cursor) }.as_bool()
                && cursor.y >= 0
            {
                let _ = unsafe { KillTimer(Some(window), 909) };
                // A hover-triggered overlay inside the same top-level HWND.
                let _ = unsafe {
                    CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        windows::core::w!("STATIC"),
                        windows::core::w!("Changed hover target"),
                        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(4 /* SS_BLACKRECT */),
                        16,
                        12,
                        260,
                        56,
                        Some(window),
                        None,
                        None,
                        None,
                    )
                };
            }
            return LRESULT(0);
        }
        if message == WM_COMMAND
            && (wparam.0 & 0xffff) == 101
            && (wparam.0 >> 16) == BN_CLICKED as usize
        {
            if let (Ok(checkbox), Ok(label)) = (unsafe { GetDlgItem(Some(window), 101) }, unsafe {
                GetDlgItem(Some(window), 102)
            }) {
                if checkbox.0 as isize == lparam.0 {
                    let count = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } + 1;
                    unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, count) };
                    let checked = unsafe {
                        SendMessageW(checkbox, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0)))
                    }
                    .0;
                    let state = if checked == 1 { "on" } else { "off" };
                    let text: Vec<u16> = format!("Checkbox business event: {count}, state {state}")
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect();
                    let _ = unsafe { SetWindowTextW(label, PCWSTR(text.as_ptr())) };
                }
            }
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let title = wide(
        &std::env::var("NEXA_COMPUTER_USE_HELPER_TITLE").expect("helper title must be supplied"),
    );
    let helper_class = windows::core::w!("NexaComputerControlSmoke");
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None) }.unwrap().0);
    let class = WNDCLASSW {
        lpfnWndProc: Some(helper_proc),
        lpszClassName: helper_class,
        hInstance: instance,
        hbrBackground: unsafe { GetSysColorBrush(COLOR_WINDOW) },
        ..Default::default()
    };
    assert_ne!(
        unsafe { RegisterClassW(&class) },
        0,
        "register isolated helper class"
    );
    let edit_class = wide("EDIT");
    let initial = wide("Initial text");
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            helper_class,
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            640,
            330,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .expect("create isolated computer-use smoke window");
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(edit_class.as_ptr()),
            PCWSTR(initial.as_ptr()),
            WS_CHILD
                | WS_VISIBLE
                | WS_BORDER
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32),
            32,
            64,
            560,
            110,
            Some(window),
            None,
            None,
            None,
        )
    }
    .expect("create isolated editable target");
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::w!("BUTTON"),
            windows::core::w!("Enable option"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
            32,
            24,
            180,
            28,
            Some(window),
            Some(HMENU(101_usize as *mut _)),
            None,
            None,
        )
    }
    .expect("create isolated checkbox target");
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::w!("STATIC"),
            windows::core::w!("Checkbox business event: 0, state off"),
            WS_CHILD | WS_VISIBLE,
            32,
            262,
            560,
            22,
            Some(window),
            Some(HMENU(102_usize as *mut _)),
            None,
            None,
        )
    }
    .expect("create isolated checkbox business feedback");
    if std::env::var_os("NEXA_COMPUTER_USE_HELPER_SEMANTICS").is_some() {
        for (value, style, y) in [
            ("Read-only evidence", ES_READONLY, 184),
            ("Nexa-private-password-fixture", ES_PASSWORD, 222),
        ] {
            let value = wide(value);
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR(edit_class.as_ptr()),
                    PCWSTR(value.as_ptr()),
                    WS_CHILD | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(style as u32),
                    32,
                    y,
                    560,
                    28,
                    Some(window),
                    None,
                    None,
                    None,
                )
            }
            .expect("create isolated semantic state target");
        }
    }
    let background = std::env::var_os("NEXA_COMPUTER_USE_HELPER_BACKGROUND").is_some();
    let _ = unsafe {
        ShowWindow(
            window,
            if background {
                SW_SHOWNOACTIVATE
            } else {
                SW_SHOW
            },
        )
    };
    if !background {
        let _ = unsafe { SetForegroundWindow(window) };
    }
    if std::env::var_os("NEXA_COMPUTER_USE_HELPER_MODAL").is_some() {
        create_owned_modal_smoke_window(window, &title);
    }
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
        let _ = unsafe { TranslateMessage(&message) };
        unsafe { DispatchMessageW(&message) };
    }
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
fn create_owned_modal_smoke_window(owner: windows::Win32::Foundation::HWND, owner_title: &[u16]) {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, GetWindow, RegisterClassW,
        SetWindowTextW, ShowWindow, GW_OWNER, HMENU, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
        WM_COMMAND, WNDCLASSW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_VISIBLE,
    };
    unsafe extern "system" fn modal_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_COMMAND && (wparam.0 & 0xffff) == 1 {
            if let Ok(owner) = unsafe { GetWindow(window, GW_OWNER) } {
                let _ = unsafe { EnableWindow(owner, true) };
                if let Ok(edit) =
                    unsafe { FindWindowExW(Some(owner), None, w!("EDIT"), PCWSTR::null()) }
                {
                    let _ = unsafe { SetWindowTextW(edit, w!("Confirmed through modal")) };
                }
            }
            let _ = unsafe { DestroyWindow(window) };
            return LRESULT(0);
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }
    let class_name = w!("NexaOwnedModalSmoke");
    let instance = HINSTANCE(unsafe { GetModuleHandleW(None) }.unwrap().0);
    let class = WNDCLASSW {
        lpfnWndProc: Some(modal_proc),
        lpszClassName: class_name,
        hInstance: instance,
        ..Default::default()
    };
    assert_ne!(
        unsafe { RegisterClassW(&class) },
        0,
        "register isolated modal class"
    );
    let mut title = owner_title[..owner_title.len() - 1].to_vec();
    title.extend(" dialog".encode_utf16());
    title.push(0);
    let modal = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_BORDER,
            160,
            160,
            360,
            180,
            Some(owner),
            None,
            Some(instance),
            None,
        )
    }
    .expect("create isolated owned modal window");
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("OK"),
            WS_CHILD | WS_VISIBLE,
            100,
            65,
            120,
            32,
            Some(modal),
            Some(HMENU(1_usize as *mut _)),
            None,
            None,
        )
    }
    .expect("create isolated modal confirmation button");
    let _ = unsafe { EnableWindow(owner, false) };
    let _ = unsafe { ShowWindow(modal, SW_SHOWNOACTIVATE) };
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
#[test]
#[ignore = "requires an interactive Windows desktop and confirms an owned modal fixture"]
fn windows_owned_modal_handoff_smoke_test() {
    use std::process::{Command, Stdio};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};
    let original_foreground = unsafe { GetForegroundWindow() };
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("nexa-owned-modal-helper.exe");
    let current = std::env::current_exe().unwrap();
    std::fs::copy(&current, &executable).unwrap();
    let title = format!("Nexa Modal Owner {}", uuid::Uuid::new_v4());
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(current.parent().unwrap().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .unwrap();
    let mut child = Command::new(executable)
        .args([
            "--ignored",
            "--exact",
            "tools::computer_use_tool::tests::windows_computer_control_helper_window",
            "--nocapture",
        ])
        .env("NEXA_COMPUTER_USE_HELPER", "1")
        .env("NEXA_COMPUTER_USE_HELPER_TITLE", &title)
        .env("NEXA_COMPUTER_USE_HELPER_BACKGROUND", "1")
        .env("NEXA_COMPUTER_USE_HELPER_MODAL", "1")
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let result = (|| -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let (owner, modal) = loop {
            let windows = platform::list_windows().map_err(|error| error.to_string())?;
            let owner = windows.iter().find(|window| window.title == title);
            let modal = windows
                .iter()
                .find(|window| window.title == format!("{title} dialog"));
            if let (Some(owner), Some(modal)) = (owner, modal) {
                break (owner.clone(), modal.clone());
            }
            if Instant::now() >= deadline {
                return Err("owned modal fixture did not become capturable".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let options = CaptureOptions {
            include_elements: true,
            max_elements: 120,
            mode: CaptureMode::Raw,
        };
        let capture =
            platform::capture_window(&modal, options).map_err(|error| error.to_string())?;
        let button = capture
            .elements
            .iter()
            .find(|element| element.role == "button" && element.name == "OK")
            .ok_or("modal OK control was not observed")?
            .id
            .clone();
        let token = remember_observation(
            None,
            vec![ObservedWindow {
                snapshot: capture.snapshot.clone(),
                image_width: Some(capture.image_width),
                image_height: Some(capture.image_height),
                native_image_width: Some(capture.native_image_width),
                native_image_height: Some(capture.native_image_height),
                screenshot_signature: screenshot_signature(&capture.png),
                screenshot_guard: screenshot_guard(&capture.png),
                elements: capture.elements.clone(),
            }],
        )
        .map_err(|error| error.to_string())?;
        let db = crate::db::Database::open_memory().map_err(|error| error.to_string())?;
        let activities = crate::activity::ActivityRuntime::with_database(db.clone())
            .map_err(|error| error.to_string())?;
        let arguments = serde_json::json!({"action":"invoke", "delivery":"background", "window_id":modal.id, "observation_id":token, "element_id":button}).to_string();
        let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
        runtime.block_on(async {
            let result = ComputerControlTool
                .execute(
                    crate::tools::ToolExecutionContext::new(
                        "modal-confirm",
                        &arguments,
                        &db,
                        &[],
                    )
                    .with_activity_runtime(&activities),
                )
                .await
                .map_err(|error| error.to_string())?;
            if result.is_error {
                return Err(format!("modal confirmation failed: {}", result.content));
            }
            let mut owner_capture =
                platform::capture_window(&owner, options).map_err(|error| error.to_string())?;
            if !semantic_observation_for_llm("owner-after", &owner_capture)
                .contains("Confirmed through modal")
            {
                return Err(
                    "modal confirmation did not update the actual owner document".into(),
                );
            }
            let data = result
                .artifacts
                .as_ref()
                .and_then(|value| value.get("data"))
                .ok_or("missing control data")?;
            let handoff = data
                .get("modalOwnerHandoffReceipt")
                .ok_or("closed modal did not return an exact owner handoff receipt")?;
            if handoff["kind"] != "computerModalOwnerHandoff"
                || handoff["windowId"] != modal.id
                || handoff["owner"]["windowId"] != owner.id
                || handoff["owner"]["targetIdentity"] != desktop_target_identity(&owner)
                || handoff["consumedObservationId"] != token
            {
                return Err("modal handoff did not bind the observed source and owner".into());
            }
            let boundary = handoff["ownerObservationNotBeforeMs"]
                .as_u64()
                .ok_or("modal handoff omitted the host observation boundary")?;
            // Windows can report the same clock tick for the returned
            // receipt and an immediate capture. Follow the real workflow's
            // fresh-observation retry without weakening its strict fence.
            let retry_started = Instant::now();
            while owner_capture.observation_captured_at_ms <= boundary
                && retry_started.elapsed() < Duration::from_millis(100)
            {
                std::thread::sleep(Duration::from_millis(2));
                owner_capture = platform::capture_window(&owner, options)
                    .map_err(|error| error.to_string())?;
            }
            let owner_data = capture_data("owner-after", &owner_capture);
            let captured = owner_data["observationCapturedAtMs"]
                .as_u64()
                .ok_or("owner capture omitted its host start time")?;
            if captured <= boundary {
                return Err(format!(
                    "owner recapture did not follow the host handoff boundary: captured={captured}, boundary={boundary}"
                ));
            }
            if !semantic_observation_for_llm("owner-after", &owner_capture)
                .contains("Confirmed through modal")
            {
                return Err("fresh owner capture lost the confirmed document state".into());
            }
            let activity_id = data["actionReceiptId"]
                .as_str()
                .ok_or("missing action receipt")?;
            let observation = activities
                .observe(activity_id, 0, Duration::ZERO)
                .await
                .map_err(|error| error.to_string())?;
            let durable = observation
                .events
                .iter()
                .rev()
                .find(|event| event.kind == crate::activity::ActivityEventKind::Completed)
                .and_then(|event| event.payload.pointer("/detail/modalOwnerHandoffReceipt"));
            if durable != Some(handoff) {
                return Err("durable modal handoff differs from the live tool result".into());
            }
            Ok(())
        })
    })();
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    let _ = unsafe { SetForegroundWindow(original_foreground) };
    if let Err(error) = result {
        panic!(
            "{error}; helper stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
#[test]
#[ignore = "requires an interactive Windows desktop and sends input to an isolated helper"]
fn windows_capture_control_recapture_smoke_test() {
    windows_control_smoke(false, false, false, false);
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
#[test]
#[ignore = "requires an interactive Windows desktop; replaces an isolated target while approaching it"]
fn windows_hover_target_change_prevents_click_smoke_test() {
    windows_control_smoke(false, false, false, true);
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
#[test]
#[ignore = "requires an interactive Windows desktop and uses UI Automation on an isolated helper"]
fn windows_background_controls_smoke_test() {
    windows_control_smoke(true, false, false, false);
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
#[test]
#[ignore = "requires an interactive Windows desktop and verifies agent-visible UI Automation state"]
fn windows_semantic_observations_are_actionable_smoke_test() {
    windows_control_smoke(true, true, false, false);
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
#[test]
#[ignore = "requires an interactive Windows desktop and closes only an isolated helper window"]
fn windows_window_closure_receipt_smoke_test() {
    windows_control_smoke(false, false, true, false);
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
fn windows_control_smoke(
    background_only: bool,
    verify_semantic_state: bool,
    verify_closure: bool,
    verify_hover_change: bool,
) {
    let _physical_pixels =
        platform::PhysicalPixels::enter().expect("physical test input coordinates");
    use std::process::{Command, Stdio};
    use windows::Win32::Foundation::{LPARAM, POINT, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetLastInputInfo, SendInput, INPUT, INPUT_0, INPUT_MOUSE, LASTINPUTINFO,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, GetCursorPos, GetDlgItem, GetForegroundWindow, PostMessageW, SendMessageW,
        SetCursorPos, SetForegroundWindow, WM_CLOSE,
    };
    let original_foreground = unsafe { GetForegroundWindow() };
    fn input_tick() -> Option<u32> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetLastInputInfo(&mut info) }
            .as_bool()
            .then_some(info.dwTime)
    }

    fn activate_isolated_window(target: &WindowSnapshot) -> Result<POINT, String> {
        let mut original = POINT::default();
        unsafe { GetCursorPos(&mut original) }
            .map_err(|error| format!("read original cursor position: {error}"))?;
        let point = (
            target.x.saturating_add((target.width / 2) as i32),
            target.y.saturating_add(16),
        );
        unsafe { SetCursorPos(point.0, point.1) }
            .map_err(|error| format!("move cursor to isolated helper: {error}"))?;
        let inputs = [
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: MOUSEEVENTF_LEFTDOWN,
                        ..Default::default()
                    },
                },
            },
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: MOUSEEVENTF_LEFTUP,
                        ..Default::default()
                    },
                },
            },
        ];
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent != inputs.len() as u32 {
            return Err(format!(
                "Windows delivered {sent}/{} helper activation inputs",
                inputs.len()
            ));
        }
        std::thread::sleep(Duration::from_millis(150));
        Ok(original)
    }

    let helper_dir = tempfile::tempdir().expect("create helper directory");
    let helper_path = helper_dir.path().join("nexa-cua-smoke-helper.exe");
    let current_exe = std::env::current_exe().expect("resolve test executable");
    std::fs::copy(&current_exe, &helper_path).expect("copy isolated helper executable");
    let helper_title = format!("Nexa Computer Use Smoke {}", uuid::Uuid::new_v4().simple());
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let helper_path_env = std::env::join_paths(
        std::iter::once(
            current_exe
                .parent()
                .expect("test executable has a parent")
                .to_path_buf(),
        )
        .chain(std::env::split_paths(&original_path)),
    )
    .expect("build helper PATH");
    let mut command = Command::new(&helper_path);
    if background_only {
        command.env("NEXA_COMPUTER_USE_HELPER_BACKGROUND", "1");
    }
    if verify_semantic_state {
        command.env("NEXA_COMPUTER_USE_HELPER_SEMANTICS", "1");
    }
    let mut child = command
        .args([
            "--ignored",
            "--exact",
            "tools::computer_use_tool::tests::windows_computer_control_helper_window",
            "--nocapture",
        ])
        .env("NEXA_COMPUTER_USE_HELPER", "1")
        .env("NEXA_COMPUTER_USE_HELPER_TITLE", &helper_title)
        .env("PATH", helper_path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("launch isolated helper window");

    let mut original_cursor = None;
    let result = (|| -> Result<(u64, ControlOutcome), String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let target = loop {
            if let Some(target) = platform::list_windows()
                .map_err(|error| error.to_string())?
                .into_iter()
                .find(|window| window.title == helper_title)
            {
                break target;
            }
            if Instant::now() >= deadline {
                return Err("isolated helper window did not become visible".to_string());
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        if !background_only {
            original_cursor = Some(activate_isolated_window(&target)?);
        }
        {
            use base64::Engine;
            let sources = list_user_share_windows().map_err(|error| error.to_string())?;
            let source = sources
                .iter()
                .find(|source| source.title == helper_title)
                .ok_or("helper missing from native share picker")?;
            let jpeg = capture_user_share_window(&source.id).map_err(|error| error.to_string())?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(jpeg)
                .map_err(|error| error.to_string())?;
            let frame = image::load_from_memory(&bytes).map_err(|error| error.to_string())?;
            if frame.width() == 0
                || frame.height() == 0
                || frame.width().max(frame.height()) > crate::media::MAX_LLM_IMAGE_DIMENSION
            {
                return Err("native shared window frame exceeded its image envelope".into());
            }
            let mut stale = target.clone();
            stale.pid += 1;
            let stale = base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_vec(&stale).map_err(|error| error.to_string())?);
            if capture_user_share_window(&stale).is_ok() {
                return Err("native sharing accepted a changed window owner".into());
            }
        }
        let mut capture = platform::capture_window(
            &target,
            CaptureOptions {
                include_elements: true,
                max_elements: 120,
                mode: CaptureMode::SetOfMarks,
            },
        )
        .map_err(|error| error.to_string())?;
        if !background_only {
            let focus: ControlArgs = serde_json::from_value(serde_json::json!({
                "action":"focus_window", "window_id":target.id,
                "observation_id":uuid::Uuid::new_v4().to_string(),
            }))
            .map_err(|error| error.to_string())?;
            // An activation request is not proof that Windows granted focus.
            // Establish it through the real adapter, then test the repeated no-op.
            let mut observed = ObservedWindow {
                snapshot: capture.snapshot.clone(),
                image_width: None,
                image_height: None,
                native_image_width: None,
                native_image_height: None,
                screenshot_signature: None,
                screenshot_guard: None,
                elements: Vec::new(),
            };
            let focused = platform::control_window(
                ControlAction::FocusWindow,
                &focus,
                &observed,
                CaptureOptions::from_control(&focus).map_err(|error| error.to_string())?,
                &ControlCommitTracker::default(),
            )
            .map_err(|error| format!("establish fixture focus: {error:?}"))?;
            observed.snapshot = focused
                .capture
                .ok_or("fixture focus had no fresh capture")?
                .snapshot;
            if !observed.snapshot.focused {
                return Err("fixture lost foreground before the no-op check".into());
            }
            let tracker = ControlCommitTracker::default();
            let outcome = platform::control_window(
                ControlAction::FocusWindow,
                &focus,
                &observed,
                CaptureOptions::from_control(&focus).map_err(|error| error.to_string())?,
                &tracker,
            )
            .map_err(|error| {
                format!(
                    "focus no-op failed: {error:?}; expected {:?}; physical {:?}",
                    *tracker.input_watch.lock().unwrap(),
                    super::super::computer_input::physical_epoch()
                )
            })?;
            let receipt = with_control_delivery(
                serde_json::json!({}),
                outcome.interaction(&tracker.interaction),
            );
            if receipt["inputDelivered"] != false
                || receipt["interaction"]["delivery"] != "notCommitted"
            {
                return Err(format!(
                    "already-focused window claimed dispatched input: {receipt}"
                ));
            }
            capture = outcome.capture.ok_or("focus no-op had no fresh capture")?;
            eprintln!(
                "Already-focused native window: no dispatched input in shared and legacy receipts"
            );
        }
        if verify_semantic_state {
            let model_view = semantic_observation_for_llm("initial", &capture);
            let model_view: serde_json::Value =
                serde_json::from_str(&model_view).map_err(|error| error.to_string())?;
            let elements = model_view["elements"]
                .as_array()
                .ok_or("missing elements")?;
            if !elements.iter().any(|element| {
                element
                    .pointer("/state/value/text")
                    .and_then(serde_json::Value::as_str)
                    == Some("Initial text")
            }) {
                return Err("Agent-visible observation omitted the existing editable value".into());
            }
            if !elements.iter().any(|element| {
                element["role"] == "checkbox" && element["state"]["toggleState"] == "off"
            }) {
                return Err(
                    "Agent cannot tell whether the checkbox already meets the request".into(),
                );
            }
            let read_only = elements
                .iter()
                .find(|element| {
                    element
                        .pointer("/state/value/text")
                        .and_then(serde_json::Value::as_str)
                        == Some("Read-only evidence")
                })
                .ok_or("read-only value was not observed")?;
            if read_only["state"]["readOnly"] != true
                || read_only["actions"]
                    .as_array()
                    .is_some_and(|actions| actions.iter().any(|action| action == "set_value"))
            {
                return Err("read-only control advertised an unusable set_value action".into());
            }
            let password = elements
                .iter()
                .find(|element| element["password"] == true)
                .ok_or("password fixture missing")?;
            if !password["state"]["value"].is_null()
                || model_view
                    .to_string()
                    .contains("Nexa-private-password-fixture")
            {
                return Err("password value leaked into model-visible semantics".into());
            }
        }
        let element = capture
            .elements
            .iter()
            .find(|element| element.keyboard_focusable && element.role == "edit")
            .ok_or_else(|| {
                format!(
                    "isolated editable UI Automation target was not captured; observed {:?}",
                    capture
                        .elements
                        .iter()
                        .map(|element| (
                            element.id.as_str(),
                            element.role.as_str(),
                            element.name.as_str(),
                            element.keyboard_focusable,
                            element.actions.as_slice(),
                        ))
                        .collect::<Vec<_>>()
                )
            })?
            .id
            .clone();
        let observed = ObservedWindow {
            snapshot: capture.snapshot.clone(),
            image_width: Some(capture.image_width),
            image_height: Some(capture.image_height),
            native_image_width: Some(capture.native_image_width),
            native_image_height: Some(capture.native_image_height),
            screenshot_signature: screenshot_signature(&capture.png),
            screenshot_guard: screenshot_guard(&capture.png),
            elements: capture.elements.clone(),
        };
        if !background_only {
            // Exercise the screenshot coordinate route, not only UIA element ids.
            // WGC and Win32 window bounds must describe the same physical surface.
            let geometry = (capture.native_image_width, capture.native_image_height);
            let bounds = (capture.snapshot.width, capture.snapshot.height);
            if geometry != bounds {
                return Err(format!("captured-image click geometry mismatch: screenshot {geometry:?}, screen bounds {bounds:?}"));
            }
            let _feedback =
                super::super::computer_pointer_feedback::PointerFeedback::begin(target.id, false);
            super::super::computer_pointer_feedback::show_at((
                capture.snapshot.x + (capture.snapshot.width / 2) as i32,
                capture.snapshot.y + (capture.snapshot.height / 2) as i32,
            ));
            let with_pointer =
                platform::capture_window(&capture.snapshot, CaptureOptions::pixels_only())
                    .map_err(|error| error.to_string())?;
            if !screenshot_signatures_match(
                &screenshot_signature(&capture.png).unwrap(),
                &screenshot_signature(&with_pointer.png).unwrap(),
            ) {
                return Err("Nexa pointer feedback contaminated model screenshot evidence".into());
            }
        }
        let initial_action = if background_only {
            ControlAction::SetValue
        } else {
            ControlAction::TypeText
        };
        let args = ControlArgs {
            action: initial_action.label().to_string(),
            approval_scope: None,
            delivery: None,
            drag_duration_ms: None,
            observation_id: uuid::Uuid::new_v4().to_string(),
            window_id: target.id,
            element_id: Some(element),
            to_element_id: None,
            coordinate_space: None,
            x: None,
            y: None,
            to_x: None,
            to_y: None,
            button: None,
            click_count: None,
            scroll_x: None,
            scroll_y: None,
            text: Some(" - Nexa 中文🙂\r\n第二行".to_string()),
            key_sequence: None,
            reason: Some("interactive smoke test".to_string()),
            include_elements: Some(true),
            max_elements: Some(120),
            capture_mode: Some("som".to_string()),
            wait_for_previous: None,
        };
        validate_control_args(&args, initial_action).map_err(|error| error.to_string())?;
        let commit_tracker = ControlCommitTracker::default();
        let outcome = platform::control_window(
            initial_action,
            &args,
            &observed,
            CaptureOptions::from_control(&args).map_err(|error| error.to_string())?,
            &commit_tracker,
        )
        .map_err(|error| format!("{error:?}"))?;
        if outcome.capture.is_none() || outcome.verification.after_hash.is_none() {
            return Err("post-action capture was not returned".to_string());
        }
        if !background_only {
            let _ = unsafe { SetForegroundWindow(original_foreground) };
        }
        let initial_input = input_tick();
        let mut cursor = POINT::default();
        unsafe { GetCursorPos(&mut cursor) }.map_err(|e| e.to_string())?;
        let hwnd = windows::Win32::Foundation::HWND(target.id as usize as *mut _);
        let edit = unsafe {
            FindWindowExW(
                Some(hwnd),
                None,
                windows::core::w!("EDIT"),
                windows::core::PCWSTR::null(),
            )
        }
        .map_err(|e| e.to_string())?;
        let checkbox = unsafe {
            FindWindowExW(
                Some(hwnd),
                None,
                windows::core::w!("BUTTON"),
                windows::core::PCWSTR::null(),
            )
        }
        .map_err(|e| e.to_string())?;
        let mut last = outcome;
        let typed_length =
            unsafe { SendMessageW(edit, 0x000e, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as usize;
        let mut typed = vec![0_u16; typed_length + 1];
        let typed_count = unsafe {
            SendMessageW(
                edit,
                0x000d,
                Some(WPARAM(typed.len())),
                Some(LPARAM(typed.as_mut_ptr() as isize)),
            )
        }
        .0 as usize;
        let typed = String::from_utf16_lossy(&typed[..typed_count]);
        if !typed.contains("中文🙂") || !typed.contains("第二行") {
            return Err("Unicode or multiline keyboard input was lost".into());
        }
        let captured = last.capture.as_ref().ok_or("missing capture")?;
        last.capture = Some(
            platform::capture_window(
                &captured.snapshot,
                CaptureOptions::from_control(&args).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
        );
        for text in ["中文🙂 line\r\n".repeat(120), String::new()] {
            let fresh = last.capture.as_ref().ok_or("missing capture")?;
            let observed = ObservedWindow {
                snapshot: fresh.snapshot.clone(),
                image_width: Some(fresh.image_width),
                image_height: Some(fresh.image_height),
                native_image_width: Some(fresh.native_image_width),
                native_image_height: Some(fresh.native_image_height),
                screenshot_signature: screenshot_signature(&fresh.png),
                screenshot_guard: screenshot_guard(&fresh.png),
                elements: fresh.elements.clone(),
            };
            let element = fresh
                .elements
                .iter()
                .find(|element| element.role == "edit")
                .ok_or("missing edit element")?;
            let input: ControlArgs = serde_json::from_value(serde_json::json!({"action":"set_value", "delivery":"background", "observation_id":uuid::Uuid::new_v4().to_string(), "window_id":target.id,"element_id":element.id,"text":text})).map_err(|e| e.to_string())?;
            validate_control_args(&input, ControlAction::SetValue).map_err(|e| e.to_string())?;
            let foreground = unsafe { GetForegroundWindow() };
            last = platform::control_window(
                ControlAction::SetValue,
                &input,
                &observed,
                CaptureOptions::from_control(&input).map_err(|e| e.to_string())?,
                &ControlCommitTracker::default(),
            )
            .map_err(|e| format!("{e:?}"))?;
            let length =
                unsafe { SendMessageW(edit, 0x000e, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as usize;
            let mut value = vec![0_u16; length + 1];
            let len = unsafe {
                SendMessageW(
                    edit,
                    0x000d,
                    Some(WPARAM(value.len())),
                    Some(LPARAM(value.as_mut_ptr() as isize)),
                )
            }
            .0 as usize;
            if String::from_utf16_lossy(&value[..len]) != text {
                return Err("background value input did not preserve the exact document".into());
            }
            if verify_semantic_state {
                let observed = last.capture.as_ref().ok_or("missing semantic recapture")?;
                let model_view: serde_json::Value =
                    serde_json::from_str(&semantic_observation_for_llm("updated", observed))
                        .map_err(|error| error.to_string())?;
                let value = model_view["elements"]
                    .as_array()
                    .ok_or("missing elements")?
                    .iter()
                    .find(|element| {
                        element["role"] == "edit"
                            && element["password"] != true
                            && element["state"]["readOnly"] != true
                    })
                    .and_then(|element| element.pointer("/state/value"))
                    .ok_or("Agent-visible post-action observation omitted the edited value")?;
                let visible = value["text"].as_str().ok_or("missing visible value")?;
                if !text.starts_with(visible)
                    || value["totalCharacters"].as_u64() != Some(text.chars().count() as u64)
                    || value["truncated"].as_bool() != Some(visible != text)
                    || visible.chars().count() > 1024
                {
                    return Err(
                        "Agent-visible value readback lost bounded Unicode or truncation evidence"
                            .into(),
                    );
                }
            }
            if unsafe { GetForegroundWindow() } != foreground
                && unsafe { GetForegroundWindow() } == hwnd
            {
                return Err(format!("background edit changed foreground focus: before={foreground:?}, after={:?}, target={}", unsafe { GetForegroundWindow() }, target.id));
            }
        }
        let fresh = last.capture.as_ref().ok_or("missing capture")?;
        let observed = ObservedWindow {
            snapshot: fresh.snapshot.clone(),
            image_width: Some(fresh.image_width),
            image_height: Some(fresh.image_height),
            native_image_width: Some(fresh.native_image_width),
            native_image_height: Some(fresh.native_image_height),
            screenshot_signature: screenshot_signature(&fresh.png),
            screenshot_guard: screenshot_guard(&fresh.png),
            elements: fresh.elements.clone(),
        };
        let element = fresh
            .elements
            .iter()
            .find(|element| element.role == "checkbox")
            .ok_or("missing checkbox")?;
        let click: ControlArgs = serde_json::from_value(serde_json::json!({"action":"click", "delivery":"auto", "observation_id":uuid::Uuid::new_v4().to_string(), "window_id":target.id,"element_id":element.id})).map_err(|e| e.to_string())?;
        let foreground = unsafe { GetForegroundWindow() };
        if foreground == hwnd {
            return Err("background checkbox fixture was already foreground before input; background focus preservation was not tested".into());
        }
        last = platform::control_window(
            ControlAction::Click,
            &click,
            &observed,
            CaptureOptions::from_control(&click).map_err(|e| e.to_string())?,
            &ControlCommitTracker::default(),
        )
        .map_err(|e| format!("{e:?}"))?;
        eprintln!(
            "Checkbox execution: route={}, delivery={}, before={foreground:?}, after={:?}, target={}",
            last.route, last.delivery, unsafe { GetForegroundWindow() }, target.id
        );
        // BM_GETCHECK: verify the native checkbox, not just the tool receipt.
        if unsafe { SendMessageW(checkbox, 0x00f0, Some(WPARAM(0)), Some(LPARAM(0))) }.0 != 1 {
            return Err("semantic auto click did not toggle the checkbox".into());
        }
        let business_label = unsafe { GetDlgItem(Some(hwnd), 102) }
            .map_err(|error| format!("missing checkbox business label: {error}"))?;
        let mut business_text = [0_u16; 256];
        let business_length = unsafe {
            SendMessageW(
                business_label,
                0x000d,
                Some(WPARAM(business_text.len())),
                Some(LPARAM(business_text.as_mut_ptr() as isize)),
            )
        }
        .0 as usize;
        let business_text = String::from_utf16_lossy(&business_text[..business_length]);
        if business_text != "Checkbox business event: 1, state on" {
            return Err(format!("checkbox did not deliver exactly one business notification with the new state: {business_text}"));
        }
        if verify_semantic_state {
            let observed = last.capture.as_ref().ok_or("missing checkbox recapture")?;
            if !semantic_observation_for_llm("toggled", observed)
                .contains("Checkbox business event: 1, state on")
            {
                return Err(
                    "Agent-visible observation omitted the actual checkbox business result".into(),
                );
            }
            let model_view: serde_json::Value =
                serde_json::from_str(&semantic_observation_for_llm("toggled", observed))
                    .map_err(|error| error.to_string())?;
            if !model_view["elements"]
                .as_array()
                .ok_or("missing elements")?
                .iter()
                .any(|element| {
                    element["role"] == "checkbox" && element["state"]["toggleState"] == "on"
                })
            {
                return Err("Agent-visible checkbox state did not acknowledge the action".into());
            }
            if !last.state_changed || last.effect != "observed_change" {
                return Err("verified checkbox change was reported as unverified delivery".into());
            }
        }
        if unsafe { GetForegroundWindow() } != foreground
            && unsafe { GetForegroundWindow() } == hwnd
        {
            return Err(format!("background click changed foreground focus: route={}, delivery={}, before={foreground:?}, after={:?}, target={}", last.route, last.delivery, unsafe { GetForegroundWindow() }, target.id));
        }
        let mut after = POINT::default();
        unsafe { GetCursorPos(&mut after) }.map_err(|e| e.to_string())?;
        if initial_input != input_tick() || initial_input.is_none() {
            eprintln!("Native value/checkbox effects verified; pointer preservation was not assessed because user input occurred or input history was unavailable.");
        } else if (after.x, after.y) != (cursor.x, cursor.y) {
            return Err("background controls moved the pointer".into());
        }
        if !background_only && !verify_closure {
            activate_isolated_window(&target)?;
            let fresh = platform::capture_window(
                &target,
                CaptureOptions::from_control(&args).map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            let element = fresh
                .elements
                .iter()
                .find(|element| element.role == "checkbox")
                .ok_or("missing checkbox coordinate target")?;
            let observed = ObservedWindow {
                snapshot: fresh.snapshot.clone(),
                image_width: Some(fresh.image_width),
                image_height: Some(fresh.image_height),
                native_image_width: Some(fresh.native_image_width),
                native_image_height: Some(fresh.native_image_height),
                screenshot_signature: screenshot_signature(&fresh.png),
                screenshot_guard: screenshot_guard(&fresh.png),
                elements: fresh.elements.clone(),
            };
            let click: ControlArgs = serde_json::from_value(serde_json::json!({
                "action":"click", "delivery":"foreground", "observation_id":uuid::Uuid::new_v4().to_string(),
                "window_id":target.id, "coordinate_space":"captured_image_pixels",
                "x": element.bounds.x + (element.bounds.width / 2) as i32,
                "y": element.bounds.y + (element.bounds.height / 2) as i32,
            })).map_err(|error| error.to_string())?;
            let sampling = Arc::new(AtomicBool::new(true));
            let sampler_flag = Arc::clone(&sampling);
            let sampler = std::thread::spawn(move || {
                let mut points = Vec::new();
                while sampler_flag.load(AtomicOrdering::Acquire) {
                    if let Ok(point) = platform::cursor_position() {
                        if points.last() != Some(&point) {
                            points.push(point);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
                points
            });
            if verify_hover_change {
                unsafe {
                    SendMessageW(
                        hwnd,
                        windows::Win32::UI::WindowsAndMessaging::WM_APP + 99,
                        Some(WPARAM(0)),
                        Some(LPARAM(0)),
                    )
                };
            }
            let click_tracker = ControlCommitTracker::default();
            let clicked = platform::control_window(
                ControlAction::Click,
                &click,
                &observed,
                CaptureOptions::from_control(&click).map_err(|error| error.to_string())?,
                &click_tracker,
            );
            sampling.store(false, AtomicOrdering::Release);
            let trajectory = sampler.join().map_err(|_| "pointer sampler failed")?;
            if verify_hover_change {
                let failure = match clicked {
                    Err(failure) => failure,
                    Ok(_) => return Err("a hover-replaced coordinate target was clicked".into()),
                };
                if failure.pre_commit_kind != Some(PreCommitFailureKind::ObservationStale)
                    || failure.phase != ControlFailurePhase::EffectMayHaveOccurred
                    || failure.interaction(&click_tracker.interaction).delivery
                        != crate::observed_interaction::InputDelivery::InputSent
                    || unsafe { SendMessageW(checkbox, 0x00f0, Some(WPARAM(0)), Some(LPARAM(0))) }.0
                        != 1
                {
                    return Err(format!("hover guard did not retain the original control and uncertain pointer effect: {failure:?}"));
                }
                eprintln!("Same-window hover replacement was rejected after movement without sending a click");
                return Ok((target.id, last));
            }
            last = clicked.map_err(|error| format!("coordinate click failed: {error:?}"))?;
            if last.interaction(&click_tracker.interaction).delivery
                != crate::observed_interaction::InputDelivery::InputSent
            {
                return Err("successful coordinate input was not retained in the receipt".into());
            }
            if trajectory.len() < 4 {
                return Err(format!(
                    "physical pointer jumped instead of moving through frames: {trajectory:?}"
                ));
            }
            if unsafe { SendMessageW(checkbox, 0x00f0, Some(WPARAM(0)), Some(LPARAM(0))) }.0 != 0 {
                return Err(
                    "captured-image coordinate click did not toggle the checkbox off".into(),
                );
            }
            eprintln!("Physical coordinate click verified against {}x{} screenshot and {}x{} visible bounds with {} distinct cursor positions", fresh.native_image_width, fresh.native_image_height, fresh.snapshot.width, fresh.snapshot.height, trajectory.len());
        }
        if verify_closure {
            let fresh = last.capture.as_ref().ok_or("missing pre-close capture")?;
            let observed = ObservedWindow {
                snapshot: fresh.snapshot.clone(),
                image_width: Some(fresh.image_width),
                image_height: Some(fresh.image_height),
                native_image_width: Some(fresh.native_image_width),
                native_image_height: Some(fresh.native_image_height),
                screenshot_signature: screenshot_signature(&fresh.png),
                screenshot_guard: screenshot_guard(&fresh.png),
                elements: fresh.elements.clone(),
            };
            let close: ControlArgs = serde_json::from_value(serde_json::json!({
                "action":"key", "observation_id":"isolated-helper-close",
                "window_id":target.id, "key_sequence":"alt+f4",
            }))
            .map_err(|error| error.to_string())?;
            last = platform::control_window(
                ControlAction::Key,
                &close,
                &observed,
                CaptureOptions::from_control(&close).map_err(|error| error.to_string())?,
                &ControlCommitTracker::default(),
            )
            .map_err(|error| format!("{error:?}"))?;
            if !last.window_closed || last.capture.is_some() || last.effect != "window_closed" {
                return Err(
                    "closing the isolated window did not produce verified terminal state".into(),
                );
            }
        }
        Ok((target.id, last))
    })();

    if let Ok((window_id, _)) = &result {
        let _ = unsafe {
            PostMessageW(
                Some(windows::Win32::Foundation::HWND(
                    *window_id as usize as *mut _,
                )),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
    let exit_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < exit_deadline {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(original) = original_cursor {
        let _ = unsafe { SetCursorPos(original.x, original.y) };
    }
    if !background_only {
        let _ = unsafe { SetForegroundWindow(original_foreground) };
    }
    let (_, outcome) = result.expect("capture-control-recapture must succeed");
    assert!(outcome.target_verified);
    if verify_closure {
        assert!(outcome.window_closed);
        assert!(outcome.capture.is_none());
    } else {
        assert!(outcome.capture.is_some());
        assert!(outcome.verification.sampled_frames > 0);
    }
}

#[cfg(all(target_os = "windows", feature = "desktop-control"))]
#[test]
#[ignore = "requires an interactive Windows desktop"]
fn windows_window_capture_smoke_test() {
    let windows = platform::list_windows().expect("enumerate Windows windows");
    let target = windows
        .into_iter()
        .find(|window| !window.minimized)
        .expect("at least one non-minimized capturable window");
    let capture = platform::capture_window(
        &target,
        CaptureOptions {
            include_elements: true,
            max_elements: 120,
            mode: CaptureMode::SetOfMarks,
        },
    )
    .expect("capture Windows window");
    assert!(!capture.png.is_empty());
    assert!(capture.image_width > 0);
    assert!(capture.image_height > 0);
    assert!(capture.semantic_error.is_none());
    assert!(capture.annotated_png.is_some());
}
