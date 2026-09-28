use super::*;
use nexa_core::{
    agent::{AgentEvent, ToolRunStatus},
    approval::ApprovalDecision,
};
use std::{
    process::Stdio,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

fn wire(mode: &str) -> Wire {
    wire_with_marker(mode, "")
}

#[test]
fn reusable_permission_identity_ignores_invocation_ids_but_binds_profile_and_action() {
    let a = json!({"toolCallId":"first","title":"Read file","kind":"read","rawInput":{"path":"file.txt","range":{"start":1,"end":9}}});
    let b: Value = serde_json::from_str(r#"{"rawInput":{"range":{"end":9,"start":1},"path":"file.txt"},"kind":"read","title":"Read file","toolCallId":"second"}"#).unwrap();
    let key = permission_target("profile-a:directory-a", "session-a", "first", &a);
    assert_eq!(
        key,
        permission_target("profile-a:directory-a", "session-b", "second", &b)
    );
    assert_ne!(
        key,
        permission_target("profile-b:directory-b", "session-a", "first", &a)
    );
    let mut changed = a.clone();
    changed["rawInput"]["path"] = json!("different.txt");
    assert_ne!(
        key,
        permission_target("profile-a:directory-a", "session-a", "first", &changed)
    );
    let store = nexa_core::approval::SessionApprovalStore::new();
    store.set(&key, ApprovalDecision::AllowSession);
    assert_eq!(
        store.get(&permission_target(
            "profile-a:directory-a",
            "session-b",
            "second",
            &b
        )),
        Some(ApprovalDecision::AllowSession)
    );
}

fn wire_with_marker(mode: &str, marker: &str) -> Wire {
    let mut command =
        tokio::process::Command::new(if cfg!(windows) { "python" } else { "python3" });
    command
        .args(["-u", "-c", include_str!("fixture.py"), mode, marker])
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Wire::spawn(command).unwrap()
}

#[tokio::test]
async fn completed_session_reuses_connection_without_resending_history_or_model_setup() {
    use nexa_core::llm::Role;
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("protocol.jsonl");
    let mut wire = wire_with_marker("record", log.to_str().unwrap());
    let session = Session::connect(&mut wire, "C:/workspace").await.unwrap();
    let mut connection = pool::Connected {
        wire,
        session,
        history: String::new(),
        context: String::new(),
    };
    let (mut first, _rx, _, _) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("gemini_cli"),
        "vendor/模型",
    );
    first
        .history
        .push(Message::text(Role::User, "OLD_HISTORY_SENTINEL"));
    let db = first.db.clone();
    let conversation = first.conversation_id.clone();
    run_initialized("gemini_cli", first, &mut connection)
        .await
        .unwrap();
    connection.history = history_fingerprint(&db, &conversation, None).unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    let fingerprint = connection.history.clone();
    pool::put(key.clone(), connection);
    assert!(pool::take(&format!("{key}:other-profile-or-workspace"), &fingerprint).is_none());
    let mut connection =
        pool::take(&key, &fingerprint).expect("completed connection remains reusable");
    let (mut next, _next_rx, _, _) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("gemini_cli"),
        "vendor/模型",
    );
    let mut user = db.get_messages(&conversation).unwrap()[0].clone();
    user.id = uuid::Uuid::new_v4().to_string();
    user.sort_order = 2;
    user.content = "SECOND_PROMPT_SENTINEL".into();
    db.add_message(&user).unwrap();
    next.turn_id = db
        .create_conversation_turn(&conversation, &user.id, None)
        .unwrap()
        .id;
    next.conversation_id = conversation.clone();
    next.db = db.clone();
    next.next_sort_order = 3;
    next.history = vec![Message::text(Role::User, "OLD_HISTORY_SENTINEL")];
    next.user_parts = vec![nexa_core::llm::ContentPart::Text { text: user.content }];
    assert_eq!(
        history_fingerprint(&db, &conversation, Some(2)).unwrap(),
        fingerprint
    );
    run_initialized("gemini_cli", next, &mut connection)
        .await
        .unwrap();
    let frames = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    for method in ["initialize", "session/new", "session/set_config_option"] {
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["method"] == method)
                .count(),
            1,
            "{method}"
        );
    }
    let prompts = frames
        .iter()
        .filter(|frame| frame["method"] == "session/prompt")
        .collect::<Vec<_>>();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[0].to_string().contains("OLD_HISTORY_SENTINEL"));
    assert!(!prompts[1].to_string().contains("OLD_HISTORY_SENTINEL"));
    assert!(prompts[1].to_string().contains("SECOND_PROMPT_SENTINEL"));
    pool::put(key.clone(), connection);
    assert!(pool::take(&key, "transcript-was-rewound").is_none());
    assert!(pool::take(&key, &fingerprint).is_none());
}

#[tokio::test]
async fn external_protocol_supports_config_only_and_legacy_catalogs_without_duplicate_tool_effects()
{
    for mode in ["config", "legacy"] {
        let (mut request, mut rx, effects, _) = super::super::tests::fixture(
            super::super::AgentRuntimeKind::Acp("gemini_cli"),
            "vendor/模型",
        );
        let db = request.db.clone();
        let conversation = request.conversation_id.clone();
        let approvals = Arc::new(AtomicUsize::new(0));
        let count = approvals.clone();
        request.approval = Arc::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { ApprovalDecision::AllowSession })
        });
        let answer = run_connected("gemini_cli", request, wire(mode), "C:/test")
            .await
            .unwrap();
        assert_eq!(answer.text_content(), "结果 ✓ 日本語 العربية");
        assert_eq!(
            effects.load(Ordering::SeqCst),
            0,
            "native reports are never dispatched as Nexa tools"
        );
        assert_eq!(approvals.load(Ordering::SeqCst), 1);
        let mut done = 0;
        let mut reported = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                AgentEvent::Done {
                    assistant_message_id: Some(id),
                    ..
                } => {
                    done += 1;
                    assert!(db
                        .get_messages(&conversation)
                        .unwrap()
                        .iter()
                        .any(
                            |message| message.id == id && message.content == answer.text_content()
                        ));
                }
                AgentEvent::ToolRunCompleted { run } => {
                    reported = true;
                    assert!(run.provider_executed);
                    assert_eq!(run.status, ToolRunStatus::Completed);
                }
                _ => {}
            }
        }
        assert!(reported);
        assert_eq!(done, 1);
    }
}

#[tokio::test]
async fn external_protocol_rejects_false_success_and_unconfirmed_model_changes() {
    for mode in [
        "auth",
        "bad_version",
        "unconfirmed",
        "wrong_session",
        "eof",
        "partial",
        "unfinished",
    ] {
        let (request, mut rx, effects, _) = super::super::tests::fixture(
            super::super::AgentRuntimeKind::Acp("opencode"),
            "vendor/模型",
        );
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_connected("opencode", request, wire(mode), "C:/test"),
        )
        .await
        .unwrap();
        let failure = result.unwrap_err().to_string();
        assert!(!failure.contains("credential-secret"));
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        while let Ok(event) = rx.try_recv() {
            assert!(
                !matches!(event, AgentEvent::Done { .. }),
                "{mode} must not finish successfully"
            );
        }
    }
}

#[tokio::test]
async fn external_protocol_cancellation_terminates_a_pending_prompt() {
    let (request, mut rx, _, _) =
        super::super::tests::fixture(super::super::AgentRuntimeKind::Acp("hermes"), "vendor/模型");
    let cancellation = request.cancellation.clone();
    let run = tokio::spawn(run_connected("hermes", request, wire("hang"), "C:/test"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    cancellation.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    while let Ok(event) = rx.try_recv() {
        assert!(!matches!(event, AgentEvent::Done { .. }));
    }
}

#[tokio::test]
async fn external_protocol_stop_kills_native_descendants_and_retains_partial_text() {
    let directory = tempfile::Builder::new()
        .prefix("acp-多語-Δ-")
        .tempdir()
        .unwrap();
    let marker = directory.path().join("orphan.txt");
    let (request, mut rx, _, _) =
        super::super::tests::fixture(super::super::AgentRuntimeKind::Acp("hermes"), "vendor/模型");
    let db = request.db.clone();
    let conversation = request.conversation_id.clone();
    let cancellation = request.cancellation.clone();
    let run = tokio::spawn(run_connected(
        "hermes",
        request,
        wire_with_marker("tree", &marker.to_string_lossy()),
        "C:/test",
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(event) = rx.recv().await {
            if matches!(event, AgentEvent::StreamBlockDelta { ref delta, .. } if delta == "child started") { return; }
        }
        panic!("No child-start receipt");
    }).await.unwrap();
    cancellation.cancel();
    assert!(tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert!(
        !marker.exists(),
        "cancelled agent left an active child process"
    );
    assert!(db
        .get_messages(&conversation)
        .unwrap()
        .iter()
        .any(|message| message.content == "child started"));
}

#[tokio::test]
async fn external_protocol_permission_denial_is_not_promoted_to_an_allow_option() {
    let (mut request, mut rx, _, _) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("opencode"),
        "vendor/模型",
    );
    request.approval = Arc::new(|_| Box::pin(async { ApprovalDecision::Deny }));
    run_connected("opencode", request, wire("config"), "C:/test")
        .await
        .unwrap();
    let mut denied = false;
    while let Ok(event) = rx.try_recv() {
        if let AgentEvent::ToolRunCompleted { run } = event {
            denied = run.status == ToolRunStatus::Failed;
        }
    }
    assert!(denied, "native tool did not observe the denied permission");
}

#[tokio::test]
async fn external_protocol_prompt_does_not_borrow_nexa_desktop_workflow_gates() {
    let (mut request, _rx, _, _) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("opencode"),
        "vendor/模型",
    );
    request.user_parts = vec![nexa_core::llm::ContentPart::Text {
        text: "Read the visible window and describe the screen.".into(),
    }];
    // Native output remains externally owned evidence; it cannot be assigned
    // Nexa's computer-observe receipt contract just by constructing a transcript.
    run_connected("opencode", request, wire("config"), "C:/test")
        .await
        .unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn external_protocol_launches_installed_cmd_shims_in_unicode_directories() {
    use nexa_core::external_agent::{ExternalAgentLaunch, ExternalAgentPreset};
    let directory = tempfile::Builder::new()
        .prefix("acp-项目 & 日本語-")
        .tempdir()
        .unwrap();
    let script = directory.path().join("fixture.py");
    std::fs::write(&script, include_str!("fixture.py")).unwrap();
    let launcher = directory.path().join("agent.cmd");
    std::fs::write(
        &launcher,
        "@echo off\r\npython -u \"%~dp0fixture.py\" legacy\r\n",
    )
    .unwrap();
    let preset = ExternalAgentPreset {
        provider: "fixture".into(),
        name: "Fixture".into(),
        command: "unused".into(),
        args: vec![],
        docs_url: String::new(),
    };
    let launch = ExternalAgentLaunch {
        executable: Some(launcher.to_string_lossy().into_owned()),
        working_directory: directory.path().to_string_lossy().into_owned(),
    };
    let mut wire = Wire::start(&preset, &launch).unwrap();
    let mut session = Session::connect(&mut wire, &launch.working_directory)
        .await
        .unwrap();
    session
        .select_model(&mut wire, "vendor/模型")
        .await
        .unwrap();
}
