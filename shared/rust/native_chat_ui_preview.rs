//! Native adapter for the same generated-HTML sandbox used by remote chat.
use std::sync::Mutex;

use super::store::{host_document, HtmlPreviewStore, DOCUMENT_CSP, HOST_CSP, PERMISSIONS_POLICY};
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
