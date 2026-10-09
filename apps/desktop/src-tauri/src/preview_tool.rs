//! A preview request succeeds only after the current renderer acknowledges it.
use nexa_core::tools::open_in_nexa_tool::{
    NexaPreviewHost, PreviewBrowserTarget, PreviewOpenReceipt, PreviewOpenRequest,
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
    opening_browser: bool,
    browser: Option<PreviewBrowserTarget>,
}

fn preview_failure(error: &str, target: Option<&PreviewBrowserTarget>) -> String {
    let error = error.chars().take(1000).collect::<String>();
    match target {
        Some(target) => format!("{error} The HTML tab is preserved: sessionId={}, tabId={}. Use browser_session show_workspace and observe to continue with this target.", target.session_id, target.tab_id),
        None => error,
    }
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
                    opening_browser: false,
                    browser: None,
                },
            );
        }
        let _guard = PendingGuard {
            state: state.clone(),
            app: self.app.clone(),
            id: id.clone(),
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
            .map_err(|_| {
                let target =
                    state.pending.lock().ok().and_then(|pending| {
                        pending.get(&id).and_then(|entry| entry.browser.clone())
                    });
                preview_failure(
                    "Nexa did not acknowledge the preview. No external app was opened.",
                    target.as_ref(),
                )
            })?
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

/// Only an outstanding, file-authorized tool request can open an Agent tab.
/// Renderer input never supplies a path, actor, or conversation authority.
#[tauri::command]
pub async fn open_agent_html_preview_cmd(
    state: State<'_, PreviewBridgeState>,
    browser: State<'_, crate::browser::state::BrowserState>,
    request_id: String,
) -> Result<PreviewBrowserTarget, String> {
    let request = {
        let mut pending = state
            .pending
            .lock()
            .map_err(|_| "Preview bridge unavailable")?;
        let entry = pending
            .get_mut(&request_id)
            .ok_or("The preview request was cancelled")?;
        if entry.opening_browser {
            return Err("This browser preview is already opening".into());
        }
        if let Some(target) = &entry.browser {
            return Ok(target.clone());
        }
        let request = &entry.request.request;
        let extension = std::path::Path::new(&request.path)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if request.line.is_some()
            || !matches!(extension.to_ascii_lowercase().as_str(), "html" | "htm")
        {
            return Err("This request is not an HTML browser preview".into());
        }
        entry.opening_browser = true;
        request.clone()
    };
    let owner = request
        .conversation_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .ok_or(
            "An agent HTML preview needs an owning conversation for continuous browser access",
        )?;
    let prepared = browser
        .prepare_html_preview(request.path, owner.clone(), request.resource_paths)
        .await?;
    let require_pending = || -> Result<(), String> {
        if state
            .pending
            .lock()
            .map_err(|_| "Preview bridge unavailable")?
            .contains_key(&request_id)
        {
            Ok(())
        } else {
            Err("The preview request was cancelled".into())
        }
    };
    let opened = async {
        require_pending()?;
        if let Some(existing) = browser.active_session(&owner)? {
            browser
                .present_workspace(&existing.id, &request.call_id, true)
                .await?;
        }
        require_pending()?;
        let session = browser
            .create_session(
                Some(owner),
                None,
                Some(&prepared.url),
                true,
                crate::browser::policy::NavigationActor::Agent,
                None,
            )
            .await?;
        let tab_id = session
            .active_tab_id
            .clone()
            .ok_or("The preview has no browser tab")?;
        // Store the committed target before presentation: cancellation never closes
        // a tab the user can inspect or invalidates a still-live HTML server.
        let mut target = PreviewBrowserTarget {
            session_id: session.id.clone(),
            tab_id,
            readiness: "opened".into(),
        };
        if let Some(entry) = state
            .pending
            .lock()
            .map_err(|_| "Preview bridge unavailable")?
            .get_mut(&request_id)
        {
            entry.browser = Some(target.clone());
        }
        browser
            .present_workspace(&session.id, &request.call_id, true)
            .await?;
        require_pending()?;
        target.readiness = "presented".into();
        if let Some(entry) = state
            .pending
            .lock()
            .map_err(|_| "Preview bridge unavailable")?
            .get_mut(&request_id)
        {
            entry.opening_browser = false;
            entry.browser = Some(target.clone());
        }
        Ok(target)
    }
    .await;
    if opened.is_err() && !prepared.reused {
        browser.release_html_preview(&prepared.preview_id);
    }
    opened
}
#[tauri::command]
pub fn acknowledge_preview_request_cmd(
    state: State<'_, PreviewBridgeState>,
    result: PreviewAcknowledgement,
) -> Result<(), String> {
    acknowledge_preview_result(&state, result)
}

fn acknowledge_preview_result(
    state: &PreviewBridgeState,
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
    if result
        .receipt
        .as_ref()
        .is_some_and(|receipt| receipt.browser != request.browser)
    {
        return Err("Preview acknowledgement does not match the authorized browser target".into());
    }
    let request = pending.remove(&result.request_id).unwrap();
    drop(pending);
    let outcome = match (result.receipt, result.error) {
        (Some(receipt), None) => Ok(receipt),
        (_, Some(error)) => Err(preview_failure(&error, request.browser.as_ref())),
        _ => Err(preview_failure(
            "Preview did not return a result",
            request.browser.as_ref(),
        )),
    };
    let _ = request.response.send(outcome);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_failure_keeps_committed_target_after_renderer_timeout() {
        let state = PreviewBridgeState::default();
        let (response, mut receiver) = oneshot::channel();
        state.pending.lock().unwrap().insert(
            "request".into(),
            Pending {
                request: PendingPreviewRequest {
                    request_id: "request".into(),
                    request: PreviewOpenRequest {
                        path: "fixture.html".into(),
                        resource_paths: vec![],
                        line: None,
                        conversation_id: Some("owner".into()),
                        call_id: "call".into(),
                    },
                },
                response,
                opening_browser: true,
                browser: Some(PreviewBrowserTarget {
                    session_id: "session-owned".into(),
                    tab_id: "tab-owned".into(),
                    readiness: "opened".into(),
                }),
            },
        );
        acknowledge_preview_result(
            &state,
            PreviewAcknowledgement {
                request_id: "request".into(),
                receipt: None,
                error: Some("Renderer timeout ".repeat(200)),
            },
        )
        .unwrap();
        assert!(state.pending.lock().unwrap().is_empty());
        let error = receiver
            .try_recv()
            .unwrap()
            .err()
            .expect("presentation error remains an error");
        assert!(error.contains("sessionId=session-owned, tabId=tab-owned"));
        assert!(error.contains("show_workspace and observe"));
        assert!(error.len() < 1400);
    }
}
