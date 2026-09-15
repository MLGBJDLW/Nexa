use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Responses are authorized before the input that opens a modal. This lets the
/// native input sequence finish its mouse/key release without replaying input or
/// leaving a COM dialog deferral alive after cancellation of a tool future.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DialogResponse {
    pub kind: String,
    pub message: String,
    pub accept: bool,
    pub prompt_text: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DialogResult {
    pub kind: String,
    pub message: String,
    pub accepted: bool,
    pub matched: bool,
    pub dialog_limit_exceeded: bool,
}

#[derive(Default)]
pub(super) struct DialogPolicy(
    Mutex<Option<ActivePolicy>>,
    std::sync::atomic::AtomicUsize,
    std::sync::atomic::AtomicBool,
);

struct ActivePolicy {
    id: String,
    page_url: String,
    responses: VecDeque<DialogResponse>,
    results: Vec<DialogResult>,
}

pub(super) struct DialogAction {
    policy: Arc<DialogPolicy>,
    id: String,
}

impl DialogPolicy {
    pub fn is_paused(&self) -> bool {
        self.2.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn navigation_started(&self) {
        self.2.store(false, std::sync::atomic::Ordering::Release);
        self.1.store(0, std::sync::atomic::Ordering::Release);
    }

    pub fn arm(
        self: &Arc<Self>,
        page_url: &str,
        responses: &[DialogResponse],
    ) -> Result<DialogAction, String> {
        if self.is_paused() {
            return Err("Page paused after repeated dialogs. Close or reload this tab before another interaction; do not replay the previous input.".into());
        }
        if responses.len() > 4
            || responses.iter().any(|response| {
                !matches!(
                    response.kind.as_str(),
                    "alert" | "confirm" | "prompt" | "beforeunload"
                ) || response.message.len() > 8192
                    || response
                        .prompt_text
                        .as_ref()
                        .is_some_and(|text| text.len() > 8192)
                    || (response.kind != "prompt" && response.prompt_text.is_some())
            })
        {
            return Err("dialogResponses accepts up to four exact kind/message responses; promptText is only valid for prompt (8192 bytes per string)".into());
        }
        let mut active = self
            .0
            .lock()
            .map_err(|_| "Browser dialog policy is unavailable")?;
        if active.is_some() {
            return Err("A browser action already owns the dialog responses".into());
        }
        self.1.store(0, std::sync::atomic::Ordering::Release);
        let id = uuid::Uuid::new_v4().to_string();
        *active = Some(ActivePolicy {
            id: id.clone(),
            page_url: page_url.into(),
            responses: responses.iter().cloned().collect(),
            results: Vec::new(),
        });
        Ok(DialogAction {
            policy: self.clone(),
            id,
        })
    }

    pub fn respond(
        &self,
        url: &str,
        kind: &str,
        message: &str,
    ) -> (bool, Option<String>, DialogResult) {
        let count = self
            .1
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |count| Some(count.saturating_add(1).min(9)),
            )
            .unwrap_or(9);
        let mut active = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let response = active.as_mut().and_then(|active| {
            let matches = active.page_url == url
                && active
                    .responses
                    .front()
                    .is_some_and(|response| response.kind == kind && response.message == message);
            if matches {
                active.responses.pop_front()
            } else {
                // A mismatched dialog invalidates all remaining authorizations.
                active.responses.clear();
                None
            }
        });
        let result = DialogResult {
            kind: kind.into(),
            message: message.chars().take(8192).collect(),
            accepted: response.as_ref().is_some_and(|response| response.accept),
            matched: response.is_some(),
            dialog_limit_exceeded: count >= 8,
        };
        if result.dialog_limit_exceeded {
            self.2.store(true, std::sync::atomic::Ordering::Release);
        }
        if let Some(active) = active.as_mut() {
            if active.results.len() < 8 {
                active.results.push(result.clone());
            } else if result.dialog_limit_exceeded {
                active.results[7] = result.clone();
            }
        }
        (
            result.accepted,
            response.and_then(|response| response.prompt_text),
            result,
        )
    }
}

impl DialogAction {
    pub fn results(&self) -> Vec<DialogResult> {
        self.policy
            .0
            .lock()
            .ok()
            .and_then(|active| {
                active
                    .as_ref()
                    .filter(|active| active.id == self.id)
                    .map(|active| active.results.clone())
            })
            .unwrap_or_default()
    }
}

impl Drop for DialogAction {
    fn drop(&mut self) {
        if let Ok(mut active) = self.policy.0.lock() {
            if active.as_ref().is_some_and(|active| active.id == self.id) {
                *active = None;
            }
        }
    }
}

#[cfg(windows)]
pub(super) async fn install(
    webview: &tauri::Webview,
    policy: Arc<DialogPolicy>,
    restricted: Arc<std::sync::atomic::AtomicBool>,
    emit: impl Fn(DialogResult) + Send + Sync + 'static,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::{CoTaskMemPWSTR, ScriptDialogOpeningEventHandler};
    use windows_core_webview2::PWSTR;
    // with_webview runs on the main thread; initialization must report failure
    // through the callback channel before the tab can be used by an Agent.
    let emit = Arc::new(emit);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    webview
        .with_webview(move |platform| unsafe {
            let installed = (|| -> windows_core_webview2::Result<()> {
                let core = platform.controller().CoreWebView2()?;
                core.Settings()?
                    .SetAreDefaultScriptDialogsEnabled(!restricted.load(Ordering::Acquire))?;
                // DevTools-dispatched input can pause on a dialog without raising
                // WebView2's ScriptDialogOpening callback. Subscribe on the same CDP
                // session before input; never recover by repeating a pointer event.
                let cdp_policy = policy.clone();
                let cdp_restricted = restricted.clone();
                let cdp_emit = emit.clone();
                let event_name = CoTaskMemPWSTR::from("Page.javascriptDialogOpening");
                let receiver =
                    core.GetDevToolsProtocolEventReceiver(*event_name.as_ref().as_pcwstr())?;
                let cdp_handler = webview2_com::DevToolsProtocolEventReceivedEventHandler::create(
                    Box::new(move |core, args| {
                        if !cdp_restricted.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let (Some(core), Some(args)) = (core, args) else {
                            return Ok(());
                        };
                        let mut raw = PWSTR::null();
                        args.ParameterObjectAsJson(&mut raw)?;
                        let encoded = CoTaskMemPWSTR::from(raw).to_string();
                        let Ok(event) = serde_json::from_str::<serde_json::Value>(&encoded) else {
                            return Ok(());
                        };
                        // WebView2 reports hasBrowserHandler=true even when its native
                        // callback is queued behind the currently paused Input command.
                        // The CDP event must answer that dialog on this same session.
                        let (accept, prompt, result) = cdp_policy.respond(
                            event["url"].as_str().unwrap_or_default(),
                            event["type"].as_str().unwrap_or_default(),
                            event["message"].as_str().unwrap_or_default(),
                        );
                        if result.dialog_limit_exceeded {
                            // Keep one modal paused instead of acknowledging an
                            // endless loop. No further renderer events are produced;
                            // Nexa remains responsive and the user can close the tab.
                            cdp_emit(result);
                            return Ok(());
                        }
                        let method = CoTaskMemPWSTR::from("Page.handleJavaScriptDialog");
                        let mut params = serde_json::json!({ "accept": accept });
                        if let Some(prompt) = prompt {
                            params["promptText"] = serde_json::json!(prompt);
                        }
                        let params = params.to_string();
                        let params = CoTaskMemPWSTR::from(params.as_str());
                        let emit = cdp_emit.clone();
                        let complete =
                            webview2_com::CallDevToolsProtocolMethodCompletedHandler::create(
                                Box::new(move |status, _| {
                                    if let Err(error) = status {
                                        log::error!("Could not answer browser dialog: {error}");
                                    } else {
                                        emit(result.clone());
                                    }
                                    Ok(())
                                }),
                            );
                        core.CallDevToolsProtocolMethod(
                            *method.as_ref().as_pcwstr(),
                            *params.as_ref().as_pcwstr(),
                            &complete,
                        )?;
                        Ok(())
                    }),
                );
                let mut cdp_token = 0;
                receiver.add_DevToolsProtocolEventReceived(&cdp_handler, &mut cdp_token)?;
                let pending_native = std::cell::RefCell::new(None);
                let handler = ScriptDialogOpeningEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else {
                        return Ok(());
                    };
                    if !restricted.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let mut kind = COREWEBVIEW2_SCRIPT_DIALOG_KIND_ALERT;
                    args.Kind(&mut kind)?;
                    let kind = match kind {
                        COREWEBVIEW2_SCRIPT_DIALOG_KIND_CONFIRM => "confirm",
                        COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT => "prompt",
                        COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD => "beforeunload",
                        _ => "alert",
                    };
                    let mut raw_url = PWSTR::null();
                    args.Uri(&mut raw_url)?;
                    let url = CoTaskMemPWSTR::from(raw_url).to_string();
                    let mut raw_message = PWSTR::null();
                    args.Message(&mut raw_message)?;
                    let message = CoTaskMemPWSTR::from(raw_message).to_string();
                    let (accept, prompt, result) = policy.respond(&url, kind, &message);
                    if result.dialog_limit_exceeded {
                        *pending_native.borrow_mut() = Some(args.GetDeferral()?);
                        emit(result);
                        return Ok(());
                    }
                    if accept {
                        if let Some(prompt) = prompt {
                            let text = CoTaskMemPWSTR::from(prompt.as_str());
                            args.SetResultText(*text.as_ref().as_pcwstr())?;
                        }
                        args.Accept()?;
                    }
                    emit(result);
                    Ok(())
                }));
                let mut token = 0;
                core.add_ScriptDialogOpening(&handler, &mut token)?;
                Ok(())
            })();
            let _ = sender.send(installed.map_err(|error| error.to_string()));
        })
        .map_err(|error| error.to_string())?;
    tokio::time::timeout(std::time::Duration::from_secs(5), receiver)
        .await
        .map_err(|_| "Browser dialog initialization timed out".to_string())?
        .map_err(|_| "Browser dialog initialization was dropped".to_string())??;
    super::webview_host::call_devtools_protocol_method(
        webview,
        "Page.enable",
        serde_json::json!({}),
        std::time::Duration::from_secs(5),
        None,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn confirm(message: &str) -> DialogResponse {
        DialogResponse {
            kind: "confirm".into(),
            message: message.into(),
            accept: true,
            prompt_text: None,
        }
    }
    #[test]
    fn responses_are_exact_ordered_single_use_and_cancel_with_the_action() {
        let policy = Arc::new(DialogPolicy::default());
        let action = policy
            .arm("https://example.com/", &[confirm("Continue?")])
            .unwrap();
        assert!(
            policy
                .respond("https://example.com/", "confirm", "Continue?")
                .0
        );
        assert!(
            !policy
                .respond("https://example.com/", "confirm", "Continue?")
                .0
        );
        assert_eq!(action.results().len(), 2);
        drop(action);
        let action = policy
            .arm("https://example.com/", &[confirm("Continue?")])
            .unwrap();
        assert!(
            !policy
                .respond("https://other.example/", "confirm", "Continue?")
                .0
        );
        assert!(
            !policy
                .respond("https://example.com/", "confirm", "Continue?")
                .0
        );
        drop(action);
        drop(
            policy
                .arm("https://example.com/", &[confirm("Continue?")])
                .unwrap(),
        );
        assert!(
            !policy
                .respond("https://example.com/", "confirm", "Continue?")
                .0
        );
    }
}

// Read the shared flag on the UI thread so queued ownership changes cannot
// restore native modal dialogs over a newer Agent lease.
pub(super) fn sync_mode(webview: &tauri::Webview, restricted: Arc<std::sync::atomic::AtomicBool>) {
    #[cfg(windows)]
    if let Err(error) = webview.with_webview(move |platform| unsafe {
        let result = platform
            .controller()
            .CoreWebView2()
            .and_then(|core| core.Settings())
            .and_then(|settings| {
                settings.SetAreDefaultScriptDialogsEnabled(
                    !restricted.load(std::sync::atomic::Ordering::Acquire),
                )
            });
        if let Err(error) = result {
            log::error!("Could not update browser dialog mode: {error}");
        }
    }) {
        log::error!("Could not dispatch browser dialog mode: {error}");
    }
    #[cfg(not(windows))]
    let _ = (webview, restricted);
}
