//! Desktop adapter and native acceptance for the shared HTML preview host.
use nexa_remote::html_preview as store;
#[path = "../../../../shared/rust/native_chat_ui_preview.rs"]
mod native;
pub use native::*;

#[cfg(all(test, target_os = "windows"))]
use tauri::{http::Response, Manager};

#[cfg(all(test, target_os = "windows"))]
mod native_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PROBE_CALLS: AtomicUsize = AtomicUsize::new(0);
    #[tauri::command]
    fn chat_ui_acceptance_probe() -> &'static str {
        PROBE_CALLS.fetch_add(1, Ordering::SeqCst);
        "native-authorized"
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop and WebView2"]
    fn native_chat_ui_sandbox_has_scripts_but_no_parent_storage_or_ipc() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("profile");
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().identifier = "com.nexa.chat-ui-acceptance".into();
        PROBE_CALLS.store(0, Ordering::SeqCst);
        let app = tauri::Builder::default().any_thread()
            .invoke_system(crate::command_dispatch::main_frame_invoke_script())
            .invoke_handler(tauri::generate_handler![chat_ui_acceptance_probe])
            .manage(ChatUiPreviewState::default())
            .register_uri_scheme_protocol("nexa-ui", |context, request| respond(context.app_handle(), request))
            .register_uri_scheme_protocol("nexa-fixture", |_, _| Response::builder()
                .header("Content-Type", "text/html")
                .header("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'unsafe-inline'; frame-src http://nexa-ui.localhost")
                .body(b"<!doctype html><title>Nexa UI acceptance</title><h1>Isolated generated HTML</h1>".to_vec()).unwrap())
            .setup(move |app| {
                let html = r#"<!doctype html><h2>Generated content</h2><output>41</output><script>
                    const result={scripts:true};
                    try{result.parent=!!parent.document}catch{result.parent=false}
                    try{localStorage.setItem('probe','x');result.storage=true}catch{result.storage=false}
                    result.transport=typeof window.__TAURI_INTERNALS__?.postMessage!=='undefined';
                    result.value=Number(document.querySelector('output').textContent)+1;
                    Promise.resolve().then(()=>window.__TAURI_INTERNALS__.invoke('chat_ui_acceptance_probe'))
                    .then(()=>result.ipc=true,()=>result.ipc=false)
                    .finally(()=>top.postMessage({nexaAcceptance:result},'*'));
                    </script>"#;
                let id = app.state::<ChatUiPreviewState>().0.lock().unwrap().create("main", html.into()).unwrap();
                let url = format!("http://nexa-ui.localhost/preview/{id}");
                let window = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External("http://nexa-fixture.localhost/".parse()?))
                    .title("Nexa generated UI acceptance").visible(true).focused(false)
                    .inner_size(640.0, 480.0).data_directory(profile.clone()).build()?;
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let result = tokio::time::timeout(std::time::Duration::from_secs(20), async {
                        // Wait for the fixture document before mounting; the app
                        // CSP stays strict, only the preview response allows JS.
                        for _ in 0..100 {
                            if evaluate(&window, "document.readyState").await? == serde_json::json!("complete") { break; }
                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                        }
                        window.eval(format!("window.addEventListener('message',e=>{{if(e.data?.nexaAcceptance)window.__acceptance=e.data.nexaAcceptance}}); window.__TAURI_INTERNALS__.invoke('chat_ui_acceptance_probe').then(value=>window.__mainProbe=value);const f=document.createElement('iframe');f.sandbox='allow-scripts';f.src={};f.style='width:100%;height:360px;border:0';document.body.appendChild(f);", serde_json::to_string(&url).unwrap())).map_err(|error| error.to_string())?;
                        for _ in 0..150 {
                            let value = evaluate(&window, "window.__acceptance || null").await?;
                            if !value.is_null() {
                                if value != serde_json::json!({"scripts":true,"parent":false,"storage":false,"transport":false,"ipc":false,"value":42}) {
                                    return Err(format!("Unexpected sandbox access: {value}"));
                                }
                                if evaluate(&window, "window.__mainProbe").await? != serde_json::json!("native-authorized") || PROBE_CALLS.load(Ordering::SeqCst) != 1 {
                                    return Err("Native IPC must remain available only to the main frame".into());
                                }
                                return Ok::<_,String>(());
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                        Err("Generated script did not execute through the native custom protocol".into())
                    }).await.unwrap_or_else(|_| Err("Native UI sandbox acceptance timed out".into()));
                    let _ = sender.send(result); handle.exit(0);
                });
                Ok(())
            }).build(context).unwrap();
        app.run_return(|_, _| {});
        receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
    }

    async fn evaluate(
        window: &tauri::WebviewWindow,
        script: &str,
    ) -> Result<serde_json::Value, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = Mutex::new(Some(tx));
        window
            .eval_with_callback(script, move |value| {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(value);
                }
            })
            .map_err(|error| error.to_string())?;
        let value = rx.await.map_err(|error| error.to_string())?;
        serde_json::from_str(&value).map_err(|error| error.to_string())
    }
}
