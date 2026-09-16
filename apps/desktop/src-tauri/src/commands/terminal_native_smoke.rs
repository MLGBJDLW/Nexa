//! Real host-message-loop regression, with an isolated WebView2 profile.
use super::*;
use std::time::{Duration, Instant};
use tauri::Manager;

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
