//! A non-activating desktop indicator projected from committed tool events.
use std::collections::BTreeMap;
use std::sync::Mutex;

use nexa_core::agent_run::{AgentRunEvent, AgentRunEventKind};
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewUrl,
    WebviewWindowBuilder,
};

const WINDOW_LABEL: &str = "desktop-control-status";
const EVENT_NAME: &str = "desktop-control:status";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopControlActivity {
    conversation_id: String,
    run_id: String,
    call_id: String,
    tool_name: String,
    window_id: Option<u64>,
    phase: DesktopControlPhase,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum DesktopControlPhase {
    Observing,
    Controlling,
    Waiting,
}

#[derive(Default)]
struct Projection {
    active: BTreeMap<String, DesktopControlActivity>,
}

impl Projection {
    fn apply(&mut self, conversation_id: &str, event: &AgentRunEvent) -> bool {
        if matches!(
            event.kind,
            AgentRunEventKind::Done | AgentRunEventKind::Error
        ) {
            let previous = self.active.len();
            self.active.remove(&event.run_id);
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
        let key = event.run_id.clone();
        if event.kind == AgentRunEventKind::ToolCompleted {
            if let Some(activity) = self
                .active
                .get_mut(&key)
                .filter(|activity| activity.call_id == call_id)
            {
                if activity.phase != DesktopControlPhase::Waiting {
                    activity.phase = DesktopControlPhase::Waiting;
                    return true;
                }
            }
            return false;
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
            phase: if tool_name == "computer_control" {
                DesktopControlPhase::Controlling
            } else {
                DesktopControlPhase::Observing
            },
            window_id: run
                .get("arguments")
                .and_then(serde_json::Value::as_str)
                .and_then(|args| serde_json::from_str::<serde_json::Value>(args).ok())
                .and_then(|args| args.get("window_id").and_then(serde_json::Value::as_u64)),
        };
        if self.active.get(&key) == Some(&activity) {
            return false;
        }
        self.active.insert(key, activity);
        true
    }

    fn snapshot(&self) -> Vec<DesktopControlActivity> {
        let mut values = self.active.values().cloned().collect::<Vec<_>>();
        values.sort_by_key(|item| match item.phase {
            DesktopControlPhase::Controlling => 0,
            DesktopControlPhase::Observing => 1,
            DesktopControlPhase::Waiting => 2,
        });
        values
    }
}

#[derive(Default)]
pub struct DesktopControlStatusState(Mutex<Projection>);

fn top_center(
    area: &PhysicalRect<i32, u32>,
    size: PhysicalSize<u32>,
    scale: f64,
) -> PhysicalPosition<i32> {
    PhysicalPosition::new(
        area.position
            .x
            .saturating_add((area.size.width.saturating_sub(size.width) / 2) as i32),
        area.position
            .y
            .saturating_add((12.0 * scale).round() as i32),
    )
}

#[cfg(target_os = "windows")]
fn target_center(window_id: Option<u64>) -> Option<(f64, f64)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut rect = RECT::default();
    unsafe { GetWindowRect(HWND(window_id? as usize as *mut _), &mut rect) }.ok()?;
    Some((
        (f64::from(rect.left) + f64::from(rect.right)) / 2.0,
        (f64::from(rect.top) + f64::from(rect.bottom)) / 2.0,
    ))
}

#[cfg(not(target_os = "windows"))]
fn target_center(_window_id: Option<u64>) -> Option<(f64, f64)> {
    None
}

fn place_status(window: &tauri::WebviewWindow, window_id: Option<u64>) {
    let monitor = target_center(window_id)
        .and_then(|(x, y)| window.monitor_from_point(x, y).ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let size = PhysicalSize::new(
            (350.0 * monitor.scale_factor()).round() as u32,
            (76.0 * monitor.scale_factor()).round() as u32,
        );
        let _ = window.set_size(size);
        let _ = window.set_position(top_center(
            monitor.work_area(),
            size,
            monitor.scale_factor(),
        ));
    }
}

pub fn initialize(app: &mut tauri::App) {
    app.manage(DesktopControlStatusState::default());
    // The renderer is warmed while hidden. Showing it never takes focus from
    // the approved target and no per-token event opens another WebView.
    match WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        WebviewUrl::App("desktop-control-status".into()),
    )
    .title("Nexa Computer Use")
    .inner_size(350.0, 76.0)
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
        Ok(window) => {
            #[cfg(target_os = "windows")]
            if let Ok(handle) = window.hwnd() {
                nexa_core::tools::computer_use_tool::register_desktop_status_window(
                    handle.0 as usize as u64,
                );
            }
            #[cfg(not(target_os = "windows"))]
            let _ = window;
        }
        Err(error) => log::warn!("Could not create computer-use status window: {error}"),
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
            place_status(
                &window,
                activities.first().and_then(|activity| activity.window_id),
            );
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
pub fn set_desktop_control_appearance_cmd(
    window: tauri::WebviewWindow,
    accent: [u8; 3],
    reduced_motion: bool,
) {
    // Secondary windows may hydrate an older local appearance before the
    // registry arrives. The main theme provider owns the native projection.
    if window.label() != "main" {
        return;
    }
    #[cfg(target_os = "windows")]
    nexa_core::tools::computer_use_tool::configure_desktop_feedback(accent, reduced_motion);
    #[cfg(not(target_os = "windows"))]
    let _ = (accent, reduced_motion);
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

    #[test]
    fn status_is_centered_on_target_work_area_with_negative_origins_and_dpi() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(-2560, 40),
            size: PhysicalSize::new(2560, 1400),
        };
        let position = top_center(&area, PhysicalSize::new(525, 114), 1.5);
        assert_eq!(position, PhysicalPosition::new(-1543, 58));
        assert_eq!(
            top_center(&area, PhysicalSize::new(3000, 114), 1.0).x,
            -2560
        );
    }

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
        assert_eq!(state.snapshot()[0].phase, DesktopControlPhase::Waiting);
        assert!(state.apply("chat-b", &event("b", AgentRunEventKind::Done, "", "")));
        assert!(state.snapshot().is_empty());
    }
}
