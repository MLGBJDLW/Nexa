//! Real host-message-loop regression, with an isolated WebView2 profile.
use super::*;
use std::time::{Duration, Instant};
use tauri::Manager;

#[test]
#[ignore = "requires Windows ConPTY and installed PowerShell 5.1 and 7"]
fn native_powershell_input_and_paste_keep_protocol_bytes() {
    let pwsh = std::env::var("NEXA_TEST_PWSH").unwrap_or_else(|_| "pwsh.exe".into());
    for program in ["powershell.exe", pwsh.as_str()] {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(program);
        command.args(["-NoLogo", "-NoProfile"]);
        command.env("TERM", "xterm-256color");
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let response_writer = writer.clone();
        let output = Arc::new(Mutex::new(String::new()));
        let recorded = output.clone();
        thread::spawn(move || {
            let mut bytes = [0u8; 8192];
            let mut pending = String::new();
            while let Ok(n) = reader.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&bytes[..n]);
                pending.push_str(&text);
                if pending.contains("\x1b[6n") {
                    let mut writer = response_writer.lock().unwrap();
                    let _ = writer.write_all(b"\x1b[1;1R");
                    let _ = writer.flush();
                    pending.clear();
                }
                if pending.len() > 32 {
                    pending = pending
                        .chars()
                        .rev()
                        .take(8)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                }
                recorded.lock().unwrap().push_str(&text);
            }
        });
        let send = |text: &str| {
            let mut writer = writer.lock().unwrap();
            writer.write_all(text.as_bytes()).unwrap();
            writer.flush().unwrap();
        };
        let wait_for = |marker: &str| {
            let deadline = Instant::now() + Duration::from_secs(12);
            while Instant::now() < deadline {
                if output.lock().unwrap().contains(marker) {
                    return true;
                }
                thread::sleep(Duration::from_millis(20));
            }
            false
        };
        send(&shell_integration_bootstrap("PowerShell", program).unwrap());
        let initialized = wait_for("\x1b]633;A\x07");
        for ch in "Write-Output ('NEXA_' + 'TYPED_OK')\r".chars() {
            send(&ch.to_string());
            thread::sleep(Duration::from_millis(2));
        }
        let typed = wait_for("NEXA_TYPED_OK");
        send("Write-Output ('NEXA_' + 'PASTE_OK'); Write-Output ('多语言_' + '输入成功')\r");
        let pasted = wait_for("NEXA_PASTE_OK") && wait_for("多语言_输入成功");
        let _ = child.kill();
        let _ = child.wait();
        let captured = output.lock().unwrap().clone();
        assert!(
            initialized && typed && pasted,
            "{program}: initialize={initialized}, type={typed}, paste={pasted}: {captured}"
        );
        assert!(
            !captured.contains("e]633;"),
            "{program} leaked unsupported prompt escapes: {captured}"
        );
        drop(pair.master);
    }
}

#[test]
#[ignore = "requires Windows ConPTY and an installed WSL Bash distribution"]
fn native_wsl_terminal_close_reaps_job_control_children() {
    fn spawn(
        profile: &nexa_core::shell_environment::ShellProfile,
        cwd: &std::path::Path,
    ) -> (
        TerminalSession,
        Box<dyn portable_pty::Child + Send + Sync>,
        Arc<Mutex<String>>,
    ) {
        let (program, args) = profile.invocation(None, cwd).unwrap();
        let (args, lease) =
            nexa_core::shell_environment::wsl_process::prepare(&program, &args).unwrap();
        let lease = Arc::new(lease.expect("interactive WSL needs owned session cleanup"));
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(&program);
        command.args(args);
        command.cwd(cwd);
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
        let response_writer = writer.clone();
        let output = Arc::new(Mutex::new(String::new()));
        let recorded = output.clone();
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(n) = reader.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                // Match Xterm's cursor-position response during ConPTY startup.
                if buffer[..n].windows(4).any(|bytes| bytes == b"\x1b[6n") {
                    let mut writer = response_writer.lock().unwrap();
                    writer.write_all(b"\x1b[1;1R").unwrap();
                    writer.flush().unwrap();
                }
                recorded
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push_str(&String::from_utf8_lossy(&buffer[..n]));
            }
        });
        let session = TerminalSession {
            wsl: Some(lease),
            master: Arc::new(Mutex::new(pair.master)),
            writer,
            killer: Arc::new(Mutex::new(child.clone_killer())),
            shell: profile.label.clone(),
            cwd: cwd.display().to_string(),
            process_id: child.process_id(),
            conversation_id: None,
            output: Arc::new(Mutex::new(TerminalOutputBuffer::default())),
        };
        (session, child, output)
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let discovery = runtime.block_on(nexa_core::shell_environment::discover_shells());
    let profile = discovery.profiles.iter().find(|p| p.kind == "wsl").unwrap();
    let cwd = std::env::current_dir().unwrap();
    let state = TerminalState::default();
    let mut children = Vec::new();
    let markers = [
        format!("nexa-pty-job-{}", Uuid::new_v4()),
        format!("nexa-pty-job-{}", Uuid::new_v4()),
    ];
    for (id, marker) in ["wsl-smoke", "wsl-other"].into_iter().zip(&markers) {
        let (session, child, output) = spawn(profile, &cwd);
        children.push(child);
        state.sessions.lock().unwrap().insert(id.into(), session);
        state.write_session(id, &(format!(r#"printf 'NEXA_TTY_%s\n' READY; case $- in *m*) printf 'NEXA_JOB_%s\n' CONTROL_OK;; esac; bash -c 'trap "" HUP; exec -a {marker} sleep 90' &"#) + "\r")).unwrap();
        let started = Instant::now();
        while !output.lock().unwrap().contains("NEXA_JOB_CONTROL_OK")
            && started.elapsed() < Duration::from_secs(15)
        {
            thread::sleep(Duration::from_millis(50));
        }
        let captured = output.lock().unwrap().clone();
        assert!(
            captured.contains("NEXA_JOB_CONTROL_OK"),
            "PTY did not preserve job control: {captured}"
        );
    }
    let child_exists = |marker: &str| {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(&profile.program)
            .args([
                "--distribution",
                profile.distribution.as_deref().unwrap(),
                "--exec",
                "bash",
                "-c",
                &format!("pgrep -f '^{marker} ' >/dev/null"),
            ])
            .creation_flags(0x08000000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    assert!(child_exists(&markers[0]) && child_exists(&markers[1]));
    state.close_session("wsl-smoke").unwrap();
    let _ = children[0].wait();
    assert!(
        !child_exists(&markers[0]),
        "closed session left a background job running"
    );
    assert!(
        child_exists(&markers[1]),
        "closing one terminal killed an unrelated WSL terminal"
    );
    let errors = nexa_core::shell_environment::wsl_process::shutdown_all_blocking();
    assert!(
        errors.is_empty(),
        "host shutdown did not clean owned sessions: {errors:?}"
    );
    assert!(
        !child_exists(&markers[1]),
        "host shutdown left an owned background job running"
    );
    state.close_session("wsl-other").unwrap();
    let _ = children[1].wait();
    assert!(!child_exists(&markers[1]));
    assert!(state.list_sessions().unwrap().is_empty());
    let (program, args) = profile.invocation(None, &cwd).unwrap();
    assert!(nexa_core::shell_environment::wsl_process::prepare(&program, &args).is_err());
}

#[test]
#[ignore = "requires an interactive Windows desktop and installed WebView2"]
fn native_terminal_contention_does_not_block_window_messages() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = "com.nexa.responsiveness-fixture".into();
    let app = tauri::Builder::default()
        .any_thread()
        .manage(TerminalState::default())
        .invoke_handler(crate::command_dispatch::off_main_thread(
            tauri::generate_handler![terminal_list_sessions_cmd],
        ))
        .setup(move |app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External("about:blank".parse().unwrap()),
            )
            .title("Nexa responsiveness regression fixture")
            .visible(true)
            .focused(false)
            .data_directory(profile)
            .build()?;
            let handle = app.handle().clone();
            thread::spawn(move || {
                let state = handle.state::<TerminalState>().inner().clone();
                let control_window = window.clone();
                // Tool-side terminal work owns this same state. Hold it until
                // the window probe completes, with a hard escape for a red run.
                let (locked_tx, locked_rx) = std::sync::mpsc::channel();
                let (release_tx, release_rx) = std::sync::mpsc::channel();
                let worker = thread::spawn(move || {
                    let _guard = state.sessions.lock().unwrap();
                    locked_tx.send(()).unwrap();
                    let _ = release_rx.recv_timeout(Duration::from_secs(3));
                });
                locked_rx.recv_timeout(Duration::from_secs(1)).unwrap();
                let (entered_tx, entered_rx) = std::sync::mpsc::channel();
                let (reply_tx, reply_rx) = std::sync::mpsc::channel();
                let key = handle.invoke_key().to_string();
                handle
                    .run_on_main_thread(move || {
                        entered_tx.send(()).unwrap();
                        window.on_message(
                            tauri::webview::InvokeRequest {
                                cmd: "terminal_list_sessions_cmd".into(),
                                callback: tauri::ipc::CallbackFn(0),
                                error: tauri::ipc::CallbackFn(1),
                                url: "http://tauri.localhost".parse().unwrap(),
                                body: tauri::ipc::InvokeBody::default(),
                                headers: Default::default(),
                                invoke_key: key,
                            },
                            Box::new(move |_, _, response, _, _| {
                                let _ = reply_tx.send(response);
                            }),
                        );
                    })
                    .unwrap();
                entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                let (ping_tx, ping_rx) = std::sync::mpsc::channel();
                let start = Instant::now();
                handle
                    .run_on_main_thread(move || {
                        let _ = ping_tx.send(());
                    })
                    .unwrap();
                let responsive = ping_rx.recv_timeout(Duration::from_millis(500)).is_ok();
                let elapsed = start.elapsed();
                // A separate request must also resolve while the original
                // call is blocked, and unknown commands must still reject.
                let (unknown_tx, unknown_rx) = std::sync::mpsc::channel();
                let key = handle.invoke_key().to_string();
                handle
                    .run_on_main_thread(move || {
                        control_window.on_message(
                            tauri::webview::InvokeRequest {
                                cmd: "missing_responsiveness_fixture_command".into(),
                                callback: tauri::ipc::CallbackFn(0),
                                error: tauri::ipc::CallbackFn(1),
                                url: "http://tauri.localhost".parse().unwrap(),
                                body: tauri::ipc::InvokeBody::default(),
                                headers: Default::default(),
                                invoke_key: key,
                            },
                            Box::new(move |_, _, reply, _, _| {
                                let _ = unknown_tx.send(reply);
                            }),
                        );
                    })
                    .unwrap();
                let unknown_rejected = unknown_rx
                    .recv_timeout(Duration::from_millis(500))
                    .is_ok_and(|reply| matches!(reply, tauri::ipc::InvokeResponse::Err(_)));
                let _ = release_tx.send(());
                worker.join().unwrap();
                let reply = reply_rx.recv_timeout(Duration::from_secs(5));
                let _ = result_tx.send((
                    responsive,
                    elapsed,
                    reply.is_ok_and(|reply| matches!(reply, tauri::ipc::InvokeResponse::Ok(_))),
                    unknown_rejected,
                ));
                handle.exit(0);
            });
            Ok(())
        })
        .build(context)
        .unwrap();
    app.run_return(|_, _| {});
    let (responsive, elapsed, replied, unknown_rejected) =
        result_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    eprintln!("native host message latency during tool contention: {elapsed:?}");
    assert!(
        replied,
        "terminal IPC did not complete after contention ended"
    );
    assert!(
        responsive,
        "host window stopped processing input while tool terminal state was locked ({elapsed:?})"
    );
    assert!(
        unknown_rejected,
        "an unrelated/unknown command was blocked or lost its rejection"
    );
}
