//! Opt-in real WebView2 acceptance. Uses a non-activating, isolated fixture window and
//! profile, never the user's active browser or installed Nexa database.
use super::*;

#[test]
#[ignore = "requires an interactive Windows desktop and installed WebView2"]
fn native_dialog_and_download_complete_the_original_trusted_gesture() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let destination = root.path().join("export.txt");
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().identifier = "com.nexa.browser-fixture".into();
    let app = tauri::Builder::default().any_thread().setup(move |app| {
        let window = tauri::window::WindowBuilder::new(app, "main")
            .title("Nexa Browser regression fixture").visible(true).focused(false)
            .inner_size(640.0, 480.0).build()?;
        let webview = window.add_child(
            WebviewBuilder::new("fixture", WebviewUrl::External(Url::parse("about:blank")?))
                .data_directory(profile)
                .initialization_script(super::super::scripts::browser_init_script("fixture-pick"))
                .initialization_script(super::super::scripts::browser_takeover_script("fixture-input"))
                .on_navigation(|url| url.scheme() != "nexa-user-input")
                ,
            tauri::LogicalPosition::new(0.0, 0.0), tauri::LogicalSize::new(640.0, 480.0),
        )?;
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            let result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|error| error.to_string())?;
                let fixture_url = format!("http://{}/", listener.local_addr().map_err(|error| error.to_string())?);
                let server = tauri::async_runtime::spawn(async move {
                    let router = axum::Router::new().route("/", axum::routing::get(|| async { axum::response::Html("<!doctype html><html><body></body></html>") }));
                    let _ = axum::serve(listener, router).await;
                });
                webview.navigate(Url::parse(&fixture_url).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
                for _ in 0..100 {
                    if eval_json(&webview, "({url: location.href, ready: document.readyState, runtime: Boolean(window.__NEXA_BROWSER_RUNTIME__)})").await?.get("runtime").and_then(serde_json::Value::as_bool) == Some(true) { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                server.abort();
                let restricted = Arc::new(AtomicBool::new(true));
                let dialogs = Arc::new(super::super::dialogs::DialogPolicy::default());
                super::super::dialogs::install(&webview, dialogs.clone(), restricted.clone(), |_| {}).await?;
                let downloads = Arc::new(super::super::downloads::DownloadGate::default());
                super::super::downloads::install(&webview, downloads.clone(), restricted).await?;
                eval_json(&webview, r#"(() => {
                    document.body.innerHTML = '<button id="export">Export</button>';
                    window.events = [];
                    const button = document.getElementById('export');
                    button.onmousedown = () => { window.events.push('down'); window.accepted = confirm('Export this report?'); };
                    button.onmouseup = () => window.events.push('up');
                    button.onclick = () => {
                        window.events.push('click');
                        if (!window.accepted || prompt('Export name', '') !== 'report') return;
                        const link = document.createElement('a');
                        link.href = URL.createObjectURL(new Blob(['native download proof'], { type: 'text/plain' }));
                        link.download = 'export.txt'; link.click();
                    };
                    return true;
                })()"#).await?;
                let binding = eval_json(&webview, r#"(() => {
                    const bridge = window.__NEXA_BROWSER_RUNTIME__;
                    const element = document.getElementById('export');
                    const target = bridge.observe().elements.find(item => bridge.resolveTargetRef(item.ref) === element);
                    return { targetRef: target.ref, targetContext: bridge.targetContextFingerprint(element), x: target.bounds.x + target.bounds.width / 2, y: target.bounds.y + target.bounds.height / 2 };
                })()"#).await?;
                let responses: Vec<super::super::dialogs::DialogResponse> = serde_json::from_value(serde_json::json!([
                    { "kind": "confirm", "message": "Export this report?", "accept": true },
                    { "kind": "prompt", "message": "Export name", "accept": true, "promptText": "report" }
                ])).map_err(|error| error.to_string())?;
                let dialog_action = dialogs.arm(&fixture_url, &responses)?;
                let download = downloads.arm(&destination)?;
                let x = binding["x"].as_f64().ok_or("Missing x")?;
                let y = binding["y"].as_f64().ok_or("Missing y")?;
                let guard = BrowserTrustedInputGuard { webview: webview.clone(), token: Arc::from("fixture-input") };
                let armed = guard.arm(TrustedInputEventBudget::pointer_click(1, 0)?, TrustedInputMatch::Pointer { x, y, button: "left".into() }, binding["targetRef"].as_str().ok_or("Missing target")?, binding["targetContext"].as_str().ok_or("Missing context")?).await?;
                dispatch_trusted_pointer_click(&armed, x, y, "left", &[], 1).await?;
                armed.disarm().await?;
                let events = eval_json(&webview, "window.events").await?;
                if events != serde_json::json!(["down", "up", "click"]) { return Err(format!("Input was replayed or release was lost: {events}")); }
                let results = dialog_action.results();
                if results.len() != 2 || results.iter().any(|dialog| !dialog.accepted || !dialog.matched) { return Err(format!("Dialog responses did not match: {results:?}")); }
                let result = download.finish().await;
                if result.state != "completed" { return Err(format!("Download did not complete: {result:?}")); }
                let content = std::fs::read_to_string(&destination).map_err(|error| error.to_string())?;
                if content != "native download proof" || result.bytes != content.len() as u64 || result.content_hash.is_none() { return Err("Native download content verification failed".into()); }
                let binding = eval_json(&webview, r#"(() => {
                    const input = document.createElement('input'); input.type = 'file'; input.id = 'upload'; input.hidden = true;
                    document.body.appendChild(input);
                    const bridge = window.__NEXA_BROWSER_RUNTIME__;
                    const target = bridge.observe().elements.find(item => bridge.resolveTargetRef(item.ref) === input);
                    return { targetRef: target.ref, targetContext: bridge.targetContextFingerprint(input) };
                })()"#).await?;
                let upload = super::super::file_upload::PreparedUpload {
                    paths: vec![destination.canonicalize().map_err(|error| error.to_string())?.to_string_lossy().into()],
                    files: vec![nexa_core::browser_runtime::BrowserFileMetadata { name: "export.txt".into(), size: content.len() as u64 }],
                };
                let target_ref = binding["targetRef"].as_str().ok_or("Missing upload target")?;
                let target_context = binding["targetContext"].as_str().ok_or("Missing upload context")?;
                let armed = guard.arm(TrustedInputEventBudget::new(0, 0, 1)?, TrustedInputMatch::Files { files: upload.files.clone() }, target_ref, target_context).await?;
                set_trusted_files(&armed, target_ref, target_context, &upload).await?;
                armed.disarm().await?;
                let (upload_content, _) = call_devtools_protocol_method(&webview, "Runtime.evaluate", serde_json::json!({"expression":"document.getElementById('upload').files[0].text()", "awaitPromise":true, "returnByValue":true}), std::time::Duration::from_secs(5), None).await?;
                let upload_content: serde_json::Value = serde_json::from_str(&upload_content).map_err(|error| error.to_string())?;
                if upload_content["result"]["value"].as_str() != Some(content.as_str()) { return Err("Native upload file content did not match".into()); }
                // A later unsolicited dialog is dismissed immediately. It cannot
                // consume an old answer or leave page evaluation suspended.
                drop(dialog_action);
                if eval_json(&webview, "confirm('Export this report?')").await? != serde_json::json!(false) { return Err("A consumed dialog response was reused".into()); }
                let burst_policy = dialogs.arm(&fixture_url, &[])?;
                let burst = tokio::time::timeout(std::time::Duration::from_millis(500), eval_json(&webview, "(() => { for (let i = 0; i < 100; i++) confirm('burst'); return true; })()")).await;
                if burst.is_ok() || !dialogs.is_paused() || !burst_policy.results().iter().any(|dialog| dialog.dialog_limit_exceeded) { return Err("Dialog flood did not pause at its bound".into()); }
                let (ui_sender, ui_receiver) = tokio::sync::oneshot::channel();
                webview.app_handle().run_on_main_thread(move || { let _ = ui_sender.send(()); }).map_err(|error| error.to_string())?;
                tokio::time::timeout(std::time::Duration::from_secs(1), ui_receiver).await.map_err(|_| "Host UI froze during dialog flood")?.map_err(|_| "Host UI callback dropped")?;
                webview.close().map_err(|error| error.to_string())?;
                Ok::<_, String>(())
            }).await.unwrap_or_else(|_| Err("Native browser fixture exceeded 30 seconds".into()));
            let _ = sender.send(result);
            handle.exit(0);
        });
        Ok(())
    }).build(context).unwrap();
    app.run_return(|_, _| {});
    receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
}
