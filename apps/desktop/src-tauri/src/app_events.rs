use log::warn;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub(crate) fn emit_window_event<R: Runtime, T: Serialize + ?Sized>(
    app_handle: &AppHandle<R>,
    window_label: &str,
    event: &str,
    payload: &T,
) {
    if window_label == "main" {
        if let Some(remote) = app_handle.try_state::<crate::remote::RemoteState>() {
            remote.publish(event, payload);
        }
    }
    // Adding a browser child makes `get_webview_window("main")` return None.
    // The chat webview still exists; address it directly and never broadcast
    // private application events to untrusted browser/preview children.
    let Some(webview) = app_handle.get_webview(window_label) else {
        return;
    };
    if webview.window().label() != window_label {
        return;
    }
    if let Err(err) = app_handle.emit_to(tauri::EventTarget::webview(window_label), event, payload)
    {
        let msg = err.to_string();
        let lower = msg.to_ascii_lowercase();
        if lower.contains("0x80070578")
            || lower.contains("invalid window handle")
            || lower.contains("invalid window")
        {
            return;
        }
        warn!("Failed to emit event '{event}' to window '{window_label}': {msg}");
    }
}

pub(crate) fn emit_main_window_event<R: Runtime, T: Serialize + ?Sized>(
    app_handle: &AppHandle<R>,
    event: &str,
    payload: &T,
) {
    emit_window_event(app_handle, "main", event, payload);
}

pub(crate) fn emit_app_event<R: Runtime, T: Serialize + ?Sized>(
    app_handle: &AppHandle<R>,
    event: &str,
    payload: &T,
) {
    for label in app_handle.windows().keys() {
        emit_window_event(app_handle, label, event, payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tauri::{Listener, WebviewBuilder, WebviewUrl, WebviewWindowBuilder};

    #[test]
    fn browser_child_keeps_main_stream_events_live_and_private() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let main = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
            .build()
            .unwrap();
        let browser = main
            .as_ref()
            .window()
            .add_child(
                WebviewBuilder::new("browser-tab", WebviewUrl::default()),
                tauri::LogicalPosition::new(0.0, 0.0),
                tauri::LogicalSize::new(400.0, 300.0),
            )
            .unwrap();
        // This is the actual Tauri distinction that opening browser_session changes.
        assert!(app.get_webview_window("main").is_none());
        assert!(app.get_webview("main").is_some());
        let delivered = Arc::new(Mutex::new(Vec::new()));
        for webview in [app.get_webview("main").unwrap(), browser] {
            let delivered = Arc::clone(&delivered);
            let label = webview.label().to_owned();
            webview.listen("agent://run-event", move |event| {
                delivered
                    .lock()
                    .unwrap()
                    .push((label.clone(), event.payload().to_owned()));
            });
        }
        for kind in ["toolCompleted", "outputDelta", "outputDelta"] {
            emit_main_window_event(app.handle(), "agent://run-event", &kind);
        }
        assert_eq!(*delivered.lock().unwrap(), vec![
            ("main".into(), "\"toolCompleted\"".into()),
            ("main".into(), "\"outputDelta\"".into()),
            ("main".into(), "\"outputDelta\"".into()),
        ], "text and tool completions reach the chat before browser/session termination, never the page");
        delivered.lock().unwrap().clear();
        emit_app_event(app.handle(), "agent://run-event", &"privacyChanged");
        assert_eq!(
            *delivered.lock().unwrap(),
            vec![("main".into(), "\"privacyChanged\"".into())]
        );
    }
}
