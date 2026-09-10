//! A preview request succeeds only after the current renderer acknowledges it.
use nexa_core::tools::open_in_nexa_tool::{
    NexaPreviewHost, PreviewOpenReceipt, PreviewOpenRequest,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager, State};
use tokio::sync::oneshot;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPreviewRequest {
    pub request_id: String,
    #[serde(flatten)]
    pub request: PreviewOpenRequest,
}
struct Pending {
    request: PendingPreviewRequest,
    response: oneshot::Sender<Result<PreviewOpenReceipt, String>>,
}
#[derive(Clone, Default)]
pub struct PreviewBridgeState {
    pending: Arc<Mutex<HashMap<String, Pending>>>,
}
pub struct NativeNexaPreviewHost {
    app: AppHandle,
}
impl NativeNexaPreviewHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}
struct PendingGuard {
    state: PreviewBridgeState,
    app: AppHandle,
    id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        if self
            .state
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id)
            .is_some()
        {
            crate::app_events::emit_main_window_event(
                &self.app,
                "preview:cancel",
                &serde_json::json!({"requestId":self.id}),
            );
        }
    }
}
#[async_trait::async_trait]
impl NexaPreviewHost for NativeNexaPreviewHost {
    async fn open(&self, request: PreviewOpenRequest) -> Result<PreviewOpenReceipt, String> {
        let state = self.app.state::<PreviewBridgeState>().inner().clone();
        let id = uuid::Uuid::new_v4().to_string();
        let (response, receiver) = oneshot::channel();
        let request = PendingPreviewRequest {
            request_id: id.clone(),
            request,
        };
        {
            let mut pending = state.pending.lock().unwrap_or_else(|e| e.into_inner());
            if !pending.is_empty() {
                return Err("Another preview is opening. Wait for it to finish before requesting a different file.".into());
            }
            pending.insert(
                id.clone(),
                Pending {
                    request: request.clone(),
                    response,
                },
            );
        }
        let _guard = PendingGuard {
            state,
            app: self.app.clone(),
            id,
        };
        let window = self
            .app
            .get_webview_window("main")
            .ok_or("The Nexa preview window is unavailable")?;
        window
            .show()
            .map_err(|_| "The Nexa preview window could not be shown")?;
        window
            .unminimize()
            .map_err(|_| "The Nexa preview window could not be restored")?;
        crate::app_events::emit_main_window_event(&self.app, "preview:open", &request);
        tokio::time::timeout(Duration::from_secs(25), receiver)
            .await
            .map_err(|_| "Nexa did not acknowledge the preview. No external app was opened.")?
            .map_err(|_| "The Nexa preview request was cancelled")?
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewAcknowledgement {
    request_id: String,
    receipt: Option<PreviewOpenReceipt>,
    error: Option<String>,
}
#[tauri::command]
pub fn pending_preview_requests_cmd(
    state: State<'_, PreviewBridgeState>,
) -> Vec<PendingPreviewRequest> {
    state
        .pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .map(|pending| pending.request.clone())
        .collect()
}
#[tauri::command]
pub fn acknowledge_preview_request_cmd(
    state: State<'_, PreviewBridgeState>,
    result: PreviewAcknowledgement,
) -> Result<(), String> {
    let mut pending = state.pending.lock().unwrap_or_else(|e| e.into_inner());
    let Some(request) = pending.get(&result.request_id) else {
        return Ok(());
    };
    if result
        .receipt
        .as_ref()
        .is_some_and(|receipt| receipt.path != request.request.request.path)
    {
        return Err("Preview acknowledgement does not match the requested file".into());
    }
    let request = pending.remove(&result.request_id).unwrap();
    drop(pending);
    let outcome = match (result.receipt, result.error) {
        (Some(receipt), None) => Ok(receipt),
        (_, Some(error)) => Err(error.chars().take(1000).collect()),
        _ => Err("Preview did not return a result".into()),
    };
    let _ = request.response.send(outcome);
    Ok(())
}
