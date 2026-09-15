//! A non-activating desktop indicator projected from committed tool events.
use std::collections::BTreeMap;
use std::sync::Mutex;

use nexa_core::agent_run::{AgentRunEvent, AgentRunEventKind};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const WINDOW_LABEL: &str = "desktop-control-status";
const EVENT_NAME: &str = "desktop-control:status";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopControlActivity {
    conversation_id: String,
    run_id: String,
    call_id: String,
    tool_name: String,
}

#[derive(Default)]
struct Projection {
    active: BTreeMap<(String, String), DesktopControlActivity>,
}

impl Projection {
    fn apply(&mut self, conversation_id: &str, event: &AgentRunEvent) -> bool {
        if matches!(
            event.kind,
            AgentRunEventKind::Done | AgentRunEventKind::Error
        ) {
            let previous = self.active.len();
            self.active.retain(|(run_id, _), _| run_id != &event.run_id);
            return self.active.len() != previous;
        }
        if !matches!(
            event.kind,
            AgentRunEventKind::ToolStarted | AgentRunEventKind::ToolCompleted
        ) {
            return false;
        }
        let run = event.payload.get("run").unwrap_or(&event.payload);
        let Some(call_id) = run.get("callId").and_then(serde_json::Value::as_str) else {
            return false;
        };
        let key = (event.run_id.clone(), call_id.to_string());
        if event.kind == AgentRunEventKind::ToolCompleted {
            return self.active.remove(&key).is_some();
        }
        let Some(tool_name) = run.get("toolName").and_then(serde_json::Value::as_str) else {
            return false;
        };
        if !matches!(tool_name, "computer_control" | "computer_observe") {
            return false;
        }
        if self.active.len() >= 64 && !self.active.contains_key(&key) {
            return false;
        }
        let activity = DesktopControlActivity {
            conversation_id: conversation_id.into(),
            run_id: event.run_id.clone(),
            call_id: call_id.into(),
            tool_name: tool_name.into(),
        };
        if self.active.get(&key) == Some(&activity) {
            return false;
        }
        self.active.insert(key, activity);
        true
    }

    fn snapshot(&self) -> Vec<DesktopControlActivity> {
        let mut values = self.active.values().cloned().collect::<Vec<_>>();
        values.sort_by_key(|item| item.tool_name != "computer_control");
        values
    }
}

#[derive(Default)]
pub struct DesktopControlStatusState(Mutex<Projection>);

pub fn initialize(app: &mut tauri::App) {
    app.manage(DesktopControlStatusState::default());
    // The renderer is warmed while hidden. Showing it never takes focus from
    // the approved target and no per-token event opens another WebView.
    if let Err(error) = WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        WebviewUrl::App("desktop-control-status".into()),
    )
    .title("Nexa Computer Use")
    .inner_size(350.0, 76.0)
    .position(24.0, 24.0)
    .resizable(false)
    .decorations(false)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .visible(false)
    .build()
    {
        log::warn!("Could not create computer-use status window: {error}");
    }
}

pub fn observe_committed_event(app: &AppHandle, conversation_id: &str, event: &AgentRunEvent) {
    if !matches!(
        event.kind,
        AgentRunEventKind::ToolStarted
            | AgentRunEventKind::ToolCompleted
            | AgentRunEventKind::Done
            | AgentRunEventKind::Error
    ) {
        return;
    }
    let Some(state) = app.try_state::<DesktopControlStatusState>() else {
        return;
    };
    let changed = state
        .0
        .lock()
        .map(|mut state| state.apply(conversation_id, event))
        .unwrap_or(false);
    if !changed {
        return;
    }
    let handle = app.clone();
    // Never create/show native windows or wait on the main thread from the
    // durable outbox writer. Read the newest projection when the UI runs.
    if let Err(error) = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(WINDOW_LABEL) else {
            return;
        };
        let activities = snapshot(&handle);
        let _ = window.emit(EVENT_NAME, &activities);
        if activities.is_empty() {
            let _ = window.hide();
        } else {
            let _ = window.show();
        }
    }) {
        log::warn!("Could not update computer-use status window: {error}");
    }
}

fn snapshot(app: &AppHandle) -> Vec<DesktopControlActivity> {
    app.try_state::<DesktopControlStatusState>()
        .and_then(|state| state.0.lock().ok().map(|state| state.snapshot()))
        .unwrap_or_default()
}

#[tauri::command]
pub fn desktop_control_status_cmd(app: AppHandle) -> Vec<DesktopControlActivity> {
    snapshot(&app)
}

#[tauri::command]
pub async fn stop_desktop_control_cmd(
    state: tauri::State<'_, crate::commands::AppState>,
    agents: tauri::State<'_, crate::commands::AgentState>,
    approvals: tauri::State<'_, crate::commands::ApprovalState>,
    app: AppHandle,
    conversation_id: String,
) -> Result<(), String> {
    if !snapshot(&app)
        .iter()
        .any(|activity| activity.conversation_id == conversation_id)
    {
        return Ok(());
    }
    crate::commands::agent_stop_cmd(state, agents, approvals, app, conversation_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexa_core::agent_run::AgentRunPhase;

    fn event(run: &str, kind: AgentRunEventKind, call: &str, tool: &str) -> AgentRunEvent {
        let mut event = AgentRunEvent::status_update(
            run,
            None,
            1,
            AgentRunPhase::Tooling,
            tool,
            Some("running"),
            None,
        );
        event.kind = kind;
        event.payload = serde_json::json!({"run":{"callId":call,"toolName":tool}});
        event
    }

    #[test]
    fn desktop_indicator_tracks_actual_tools_and_releases_only_the_finished_run() {
        let mut state = Projection::default();
        assert!(!state.apply(
            "chat-a",
            &event("a", AgentRunEventKind::OutputDelta, "x", "computer_control")
        ));
        assert!(!state.apply(
            "chat-a",
            &event("a", AgentRunEventKind::ToolStarted, "x", "read_file")
        ));
        assert!(state.apply(
            "chat-a",
            &event(
                "a",
                AgentRunEventKind::ToolStarted,
                "control",
                "computer_control"
            )
        ));
        assert!(state.apply(
            "chat-b",
            &event(
                "b",
                AgentRunEventKind::ToolStarted,
                "observe",
                "computer_observe"
            )
        ));
        assert_eq!(state.snapshot()[0].tool_name, "computer_control");
        assert!(state.apply("chat-a", &event("a", AgentRunEventKind::Done, "", "")));
        assert_eq!(state.snapshot().len(), 1);
        assert_eq!(state.snapshot()[0].conversation_id, "chat-b");
        assert!(state.apply(
            "chat-b",
            &event(
                "b",
                AgentRunEventKind::ToolCompleted,
                "observe",
                "computer_observe"
            )
        ));
        assert!(state.snapshot().is_empty());
    }
}
