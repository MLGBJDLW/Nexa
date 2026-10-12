//! Native adapter for the same generated-HTML sandbox used by remote chat.
use std::sync::Mutex;

use nexa_remote::html_preview::{
    host_document, HtmlPreviewStore, DOCUMENT_CSP, HOST_CSP, PERMISSIONS_POLICY,
};
use tauri::{
    http::{Request, Response, StatusCode},
    Manager,
};

#[derive(Default)]
pub struct ChatUiPreviewState(pub Mutex<HtmlPreviewStore>);

#[tauri::command]
pub fn create_chat_ui_preview_cmd(
    state: tauri::State<'_, ChatUiPreviewState>,
    window: tauri::WebviewWindow,
    html: String,
) -> Result<String, String> {
    let id = state
        .0
        .lock()
        .map_err(|_| "HTML preview state unavailable")?
        .create(window.label(), html)?;
    Ok(format!("/preview/{id}"))
}

#[tauri::command]
pub fn release_chat_ui_preview_cmd(
    state: tauri::State<'_, ChatUiPreviewState>,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    state
        .0
        .lock()
        .map_err(|_| "HTML preview state unavailable")?
        .release(window.label(), &id);
    Ok(())
}

pub fn respond(app: &tauri::AppHandle, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let mut parts = request.uri().path().trim_start_matches('/').split('/');
    let route = parts.next();
    let id = parts.next().unwrap_or("");
    let suffix = parts.next();
    let document = suffix == Some("document");
    let state = app.state::<ChatUiPreviewState>();
    let store = state.0.lock().unwrap_or_else(|error| error.into_inner());
    let content = if request.method() == "GET"
        && route == Some("preview")
        && (suffix.is_none() || document)
        && parts.next().is_none()
    {
        store.read(id).and_then(|(_, html)| {
            if document {
                Some(html.to_string())
            } else {
                host_document(id)
            }
        })
    } else {
        None
    };
    Response::builder()
        .status(if content.is_some() {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        })
        .header("Content-Type", "text/html; charset=utf-8")
        .header(
            "Content-Security-Policy",
            if document { DOCUMENT_CSP } else { HOST_CSP },
        )
        .header("Permissions-Policy", PERMISSIONS_POLICY)
        .header("Cache-Control", "no-store")
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff")
        .body(content.unwrap_or_default().into_bytes())
        .expect("fixed preview headers")
}

#[cfg(all(test, target_os = "windows"))]
mod native_tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows desktop and WebView2"]
    fn native_chat_ui_sandbox_has_scripts_but_no_parent_storage_or_ipc() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("profile");
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        context.config_mut().identifier = "com.nexa.chat-ui-acceptance".into();
        let app = tauri::Builder::default().any_thread()
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
                    result.ipc=typeof window.__TAURI_INTERNALS__!=='undefined';
                    result.value=Number(document.querySelector('output').textContent)+1;
                    top.postMessage({nexaAcceptance:result},'*');
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
                        window.eval(format!("window.addEventListener('message',e=>{{if(e.data?.nexaAcceptance)window.__acceptance=e.data.nexaAcceptance}}); const f=document.createElement('iframe');f.sandbox='allow-scripts';f.src={};f.style='width:100%;height:360px;border:0';document.body.appendChild(f);", serde_json::to_string(&url).unwrap())).map_err(|error| error.to_string())?;
                        for _ in 0..150 {
                            let value = evaluate(&window, "window.__acceptance || null").await?;
                            if !value.is_null() {
                                if value != serde_json::json!({"scripts":true,"parent":false,"storage":false,"ipc":false,"value":42}) {
                                    return Err(format!("Unexpected sandbox access: {value}"));
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
