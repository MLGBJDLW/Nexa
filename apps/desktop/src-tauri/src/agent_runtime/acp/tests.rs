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

#[tokio::test]
async fn opaque_chat_reasoning_reaches_native_prompt_and_unadvertised_values_fail_closed() {
    for effort in ["deep", "invented"] {
        let (mut request, mut rx, _, _) = super::super::tests::fixture(
            super::super::AgentRuntimeKind::Acp("opencode"),
            "vendor/模型",
        );
        request.config.reasoning_effort = Some(nexa_core::llm::ReasoningEffort::High);
        request.external = Some(super::super::ExternalAgentBinding {
            profile_id: "fixture".into(),
            launch: Default::default(),
            reasoning_effort: Some(effort.into()),
        });
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = run_connected("opencode", request, wire("opaque_effort"), "fixture").await;
        assert_eq!(result.is_ok(), effort == "deep", "{effort}: {result:?}");
        drain.await.unwrap();
    }
}

#[tokio::test]
async fn custom_launch_overrides_arguments_and_environment_without_shell_interpretation() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("launch.json");
    let preset = nexa_core::external_agent::ExternalAgentPreset {
        command: "python".into(),
        args: vec!["--invalid-preset-argument".into()],
        env: std::collections::BTreeMap::from([("NEXA_ACP_TEST".into(), "preset".into())]),
        ..Default::default()
    };
    let launch = ExternalAgentLaunch {
        working_directory: directory.path().to_string_lossy().into(),
        args: Some(vec![
            "-u".into(),
            "-c".into(),
            include_str!("fixture.py").into(),
            "launch".into(),
            marker.to_string_lossy().into(),
            "literal & unicode 参数".into(),
        ]),
        env: std::collections::BTreeMap::from([(
            "NEXA_ACP_TEST".into(),
            "override=literal".into(),
        )]),
        ..Default::default()
    };
    let mut wire = Wire::start(&preset, &launch).unwrap();
    Session::connect(&mut wire, &launch.working_directory)
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(marker).unwrap()).unwrap();
    assert_eq!(value["argument"], "literal & unicode 参数");
    assert_eq!(value["environment"], "override=literal");
}

#[tokio::test]
async fn automatic_acp_directory_is_shared_by_process_and_protocol() {
    let root = tempfile::tempdir().unwrap();
    let db = nexa_core::db::Database::new(root.path().join("nexa.db")).unwrap();
    let launch = ExternalAgentLaunch::default()
        .resolve(&db, None, Some("projectless-chat"))
        .unwrap();
    let marker = root.path().join("cwd.json");
    let preset = nexa_core::external_agent::ExternalAgentPreset {
        provider: "fixture".into(),
        name: "Fixture".into(),
        command: "python".into(),
        args: vec![
            "-u".into(),
            "-c".into(),
            include_str!("fixture.py").into(),
            "cwd".into(),
            marker.to_string_lossy().into(),
        ],
        env: Default::default(),
        docs_url: String::new(),
        ..Default::default()
    };
    let mut wire = Wire::start(&preset, &launch).unwrap();
    let session = Session::connect(&mut wire, &launch.working_directory)
        .await
        .unwrap();
    assert_eq!(session.cwd, launch.working_directory);
    let receipt: Value = serde_json::from_str(&std::fs::read_to_string(marker).unwrap()).unwrap();
    assert_eq!(receipt["sessionCwd"], launch.working_directory);
    assert_eq!(
        std::fs::canonicalize(receipt["processCwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(&launch.working_directory).unwrap()
    );
}

#[tokio::test]
async fn chat_model_switch_discards_old_model_options_and_discovery_survives_retirement() {
    let mut wire = wire("dependent");
    let mut session = Session::connect(&mut wire, "fixture").await.unwrap();
    let preferences = std::collections::BTreeMap::from([
        ("provider".into(), "B".into()),
        ("reasoning_effort".into(), "low".into()),
        ("fast".into(), "on".into()),
    ]);
    session
        .configure(
            &mut wire,
            Some("B2"),
            &preferences,
            None,
            Some("A1"),
            catalog::ConfigurationUse::Inference,
        )
        .await
        .unwrap();
    assert_eq!(session.models[0].id, "B2");
    session
        .configure(
            &mut wire,
            Some("retired"),
            &preferences,
            None,
            Some("A1"),
            catalog::ConfigurationUse::Discovery,
        )
        .await
        .unwrap();
    assert_eq!(session.models.len(), 2);
    assert!(
        session.select_model(&mut wire, "retired").await.is_err(),
        "Inference cannot use a retired selection"
    );
}

#[tokio::test]
async fn provider_switch_replaces_catalog_before_model_and_model_dependent_reasoning() {
    let mut wire = wire("dependent");
    let mut session = Session::connect(&mut wire, "fixture").await.unwrap();
    // Saved low is obsolete for B2; the explicit chat choice high owns reasoning.
    let preferences = std::collections::BTreeMap::from([
        ("provider".into(), "B".into()),
        ("reasoning_effort".into(), "low".into()),
    ]);
    session
        .configure(
            &mut wire,
            Some("B2"),
            &preferences,
            Some("high"),
            None,
            catalog::ConfigurationUse::Inference,
        )
        .await
        .unwrap();
    assert_eq!(
        session
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["B2", "B1"]
    );
    let effort = session
        .config_options
        .iter()
        .find(|option| option.id == "reasoning_effort")
        .unwrap();
    assert_eq!(effort.current_value, "high");
    assert_eq!(effort.options.len(), 2);
    assert_eq!(effort.options[0].value, "high");
    session
        .set_option(&mut wire, "reasoning_effort", "default")
        .await
        .unwrap();
    assert_eq!(
        session
            .config_options
            .iter()
            .find(|option| option.id == "reasoning_effort")
            .unwrap()
            .current_value,
        "high"
    );
}

#[tokio::test]
#[ignore = "uses the official Copilot CLI ACP login to edit one disposable temporary file"]
async fn native_copilot_acp_reads_and_edits_with_native_tools() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("native-acp-edit.txt");
    std::fs::write(&path, "old value\n").unwrap();
    let (mut request, mut rx, effects, nonce) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("github_copilot_acp"),
        "@nexa/agent-default",
    );
    let launch = ExternalAgentLaunch {
        executable: Some(
            crate::commands::subscription_accounts::resolve_copilot_binary()
                .unwrap()
                .to_string_lossy()
                .into(),
        ),
        working_directory: directory.path().to_string_lossy().into(),
        ..Default::default()
    };
    let catalog = probe("github_copilot_acp", &launch, &request.db, None)
        .await
        .unwrap();
    let model = catalog
        .models
        .iter()
        .find(|model| model.id == "claude-opus-5.5")
        .unwrap_or(&catalog.models[0])
        .id
        .clone();
    request.config.model = Some(model.clone());
    request.user_parts = vec![nexa_core::llm::ContentPart::Text { text: format!(
        "Read {} with your native tools, edit it by replacing old value with {}, then read it to verify. This is a disposable test file. Reply ACP_EDIT_VERIFIED and the exact new value.", path.display(), nonce) }];
    request.external = Some(super::super::ExternalAgentBinding {
        profile_id: uuid::Uuid::new_v4().to_string(),
        launch,
        reasoning_effort: None,
    });
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let answer = tokio::time::timeout(Duration::from_secs(180), run("github_copilot_acp", request))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap().trim(), nonce);
    assert!(answer.text_content().contains("ACP_EDIT_VERIFIED"));
    assert!(answer.text_content().contains(&nonce));
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    drain.await.unwrap();
    shutdown();
    eprintln!("Live Copilot ACP model={model}: native edit and final answer verified");
}

fn client_wire(root: &std::path::Path, mode: &str) -> Wire {
    let mut command =
        tokio::process::Command::new(if cfg!(windows) { "python" } else { "python3" });
    command
        .args([
            "-u",
            "-c",
            include_str!("client_fixture.py"),
            &root.to_string_lossy(),
            mode,
        ])
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    Wire::spawn(command).unwrap()
}

#[tokio::test]
async fn user_permission_does_not_block_concurrent_filesystem_requests() {
    let directory = tempfile::tempdir().unwrap();
    let (mut request, mut rx, _, _) =
        super::super::tests::fixture(super::super::AgentRuntimeKind::Acp("gemini_cli"), "native");
    request.dependencies.tools =
        request
            .dependencies
            .tools
            .with_workspace(Some(nexa_core::workspace::Workspace {
                roots: vec![directory.path().to_string_lossy().into()],
            }));
    request.approval = Arc::new(|_| {
        Box::pin(async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            ApprovalDecision::AllowOnce
        })
    });
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let answer = tokio::time::timeout(
        Duration::from_secs(10),
        run_connected(
            "gemini_cli",
            request,
            client_wire(directory.path(), "parallel_permission"),
            &directory.path().to_string_lossy(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(answer.text_content(), "Concurrent services verified");
    drain.await.unwrap();
}

#[tokio::test]
async fn native_services_create_missing_files_render_released_terminals_and_survive_long_polling() {
    let directory = tempfile::tempdir().unwrap();
    let (mut request, mut rx, effects, _) =
        super::super::tests::fixture(super::super::AgentRuntimeKind::Acp("gemini_cli"), "native");
    request.dependencies.tools =
        request
            .dependencies
            .tools
            .with_workspace(Some(nexa_core::workspace::Workspace {
                roots: vec![directory.path().to_string_lossy().into()],
            }));
    let db = request.db.clone();
    let drain = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    });
    tokio::time::timeout(
        Duration::from_secs(60),
        run_connected(
            "gemini_cli",
            request,
            client_wire(directory.path(), "services"),
            &directory.path().to_string_lossy(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let events = drain.await.unwrap();
    assert_eq!(
        effects.load(Ordering::SeqCst),
        0,
        "native reports are not replayed through Nexa tools"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("created-中文.txt")).unwrap(),
        "first\r\n中文🙂\nlast\n"
    );
    assert!(!directory.path().join("wrong-session.txt").exists());
    let checkpoints = db.list_file_checkpoints(None).unwrap().len();
    assert_eq!(
        checkpoints, 1,
        "duplicate write RPC must reuse the exact receipt"
    );
    assert!(events.iter().any(|event| matches!(event, AgentEvent::ToolRunCompleted { run } if run.call_id.ends_with("native-terminal") && run.content.as_deref().is_some_and(|text| text.contains("DONE")))));
    assert!(events.iter().any(|event| matches!(event, AgentEvent::ToolRunCompleted { run } if run.call_id.ends_with("native-edit") && run.artifacts.as_ref().is_some_and(|value| value["diffs"][0]["additions"] == 3))));
    let done = events
        .iter()
        .find(|event| matches!(event, AgentEvent::Done { .. }))
        .unwrap();
    let payload = nexa_core::agent_run::AgentRunEvent::from_agent_event(done).payload;
    assert_eq!(payload["lastPromptTokens"], 6400);
    assert_eq!(payload["contextBreakdown"]["contextWindow"], 64000);
    assert_eq!(payload["usageTotal"]["totalTokens"], 0);
}

#[tokio::test]
async fn queued_commands_keep_the_following_input_and_first_project_context() {
    let directory = tempfile::tempdir().unwrap();
    let (mut request, mut rx, _, _) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("github_copilot_acp"),
        "native",
    );
    request.user_parts = vec![nexa_core::llm::ContentPart::Text {
        text: "/context".into(),
    }];
    request.config.system_prompt = "PROJECT_CONTEXT_MUST_REACH_AGENT".into();
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    sender
        .send(nexa_core::agent::AgentSteeringMessage::text(
            "/mcp:server:command",
        ))
        .unwrap();
    sender
        .send(nexa_core::agent::AgentSteeringMessage::text(
            "Continue with the requested work",
        ))
        .unwrap();
    drop(sender);
    request.steering = receiver;
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let answer = run_connected(
        "github_copilot_acp",
        request,
        client_wire(directory.path(), "commands"),
        &directory.path().to_string_lossy(),
    )
    .await
    .unwrap();
    assert_eq!(answer.text_content(), "Native peer completed");
    drain.await.unwrap();
}

#[tokio::test]
async fn cold_native_command_preserves_raw_input_and_does_not_mark_project_context_delivered() {
    let directory = tempfile::tempdir().unwrap();
    let mut wire = client_wire(directory.path(), "commands");
    let session = Session::connect(&mut wire, &directory.path().to_string_lossy())
        .await
        .unwrap();
    let mut connection = pool::Connected {
        wire,
        session,
        history: String::new(),
        context: String::new(),
    };
    let (mut request, mut rx, _, _) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("github_copilot_acp"),
        "native",
    );
    request.user_parts = vec![nexa_core::llm::ContentPart::Text {
        text: "/mcp:server:command".into(),
    }];
    request.config.system_prompt = "PROJECT_CONTEXT_MUST_REACH_AGENT".into();
    let db = request.db.clone();
    let conversation = request.conversation_id.clone();
    let answer = run_initialized("github_copilot_acp", request, &mut connection)
        .await
        .unwrap();
    assert!(answer.text_content().is_empty());
    assert_eq!(
        db.get_messages(&conversation).unwrap().len(),
        1,
        "A native command receipt is not assistant dialogue"
    );
    let mut command_receipt = false;
    while let Ok(event) = rx.try_recv() {
        if let AgentEvent::ControllerStatus { code, content, .. } = event {
            if code == "external_agent_command_completed" {
                command_receipt = content == "/mcp:server:command";
            }
        }
    }
    assert!(command_receipt);
    assert!(connection.context.is_empty());
    connection.history = "cached-completed-command".into();
    let (mut request, _rx, _, _) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("github_copilot_acp"),
        "native",
    );
    request.config.system_prompt = "PROJECT_CONTEXT_MUST_REACH_AGENT".into();
    run_initialized("github_copilot_acp", request, &mut connection)
        .await
        .unwrap();
    assert!(!connection.context.is_empty());
}

#[tokio::test]
async fn native_questions_require_the_selected_option_and_never_infer_the_first_answer() {
    for (decision, expected) in [
        (ApprovalDecision::SelectOption(1), "choice-b"),
        (ApprovalDecision::AllowOnce, "deny"),
    ] {
        let (mut request, _rx, _, _) =
            super::super::tests::fixture(super::super::AgentRuntimeKind::Acp("opencode"), "native");
        request.approval = Arc::new(move |request| {
            assert_eq!(request.choices, vec!["First answer", "Second answer"]);
            Box::pin(async move { decision })
        });
        let turn = request.prepare(false).unwrap();
        let params = json!({"sessionId":"session","toolCall":{"toolCallId":"question","kind":"other","title":"Which target?"},"options":[{"optionId":"choice-a","kind":"allow_once","name":"First answer"},{"optionId":"choice-b","kind":"allow_once","name":"Second answer"},{"optionId":"deny","kind":"reject_once","name":"Skip"}]});
        assert_eq!(
            permission("opencode", &turn, "session", &params)
                .await
                .unwrap()["outcome"]["optionId"],
            expected
        );
    }
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
async fn clearing_reasoning_override_reconnects_and_restores_native_default() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("effective-effort.txt");
    let python = std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let launch = ExternalAgentLaunch {
        executable: Some(String::from_utf8(python.stdout).unwrap().trim().into()),
        working_directory: directory.path().to_string_lossy().into(),
        args: Some(vec![
            "-u".into(),
            "-c".into(),
            include_str!("fixture.py").into(),
            "opaque_record".into(),
            log.to_string_lossy().into(),
        ]),
        ..Default::default()
    };
    let binding = super::super::ExternalAgentBinding {
        profile_id: uuid::Uuid::new_v4().to_string(),
        launch,
        reasoning_effort: Some("deep".into()),
    };
    let (mut first, _rx, _, _) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("opencode"),
        "vendor/模型",
    );
    let db = first.db.clone();
    let conversation = first.conversation_id.clone();
    first.external = Some(binding.clone());
    run("opencode", first).await.unwrap();

    let (mut next, _next_rx, _, _) = super::super::tests::fixture(
        super::super::AgentRuntimeKind::Acp("opencode"),
        "vendor/模型",
    );
    let mut user = db.get_messages(&conversation).unwrap()[0].clone();
    user.id = uuid::Uuid::new_v4().to_string();
    user.sort_order = db
        .get_messages(&conversation)
        .unwrap()
        .iter()
        .map(|message| message.sort_order)
        .max()
        .unwrap()
        + 1;
    user.content = "Use the native default for this turn".into();
    db.add_message(&user).unwrap();
    next.turn_id = db
        .create_conversation_turn(&conversation, &user.id, None)
        .unwrap()
        .id;
    next.next_sort_order = user.sort_order + 1;
    next.db = db;
    next.conversation_id = conversation;
    next.external = Some(super::super::ExternalAgentBinding {
        reasoning_effort: None,
        ..binding
    });
    run("opencode", next).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["deep", "balanced"]
    );
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
        env: Default::default(),
        docs_url: String::new(),
        ..Default::default()
    };
    let launch = ExternalAgentLaunch {
        executable: Some(launcher.to_string_lossy().into_owned()),
        working_directory: directory.path().to_string_lossy().into_owned(),
        ..ExternalAgentLaunch::default()
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
