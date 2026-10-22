//! A non-activating desktop indicator projected from committed tool events.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

use nexa_core::agent_run::{AgentRunEvent, AgentRunEventKind, AgentRunPhase};
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
    turn_id: Option<String>,
    call_id: String,
    tool_name: String,
    window_id: Option<u64>,
    phase: DesktopControlPhase,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum DesktopControlPhase {
    Controlling,
    Moving,
    Committing,
    Verifying,
    Waiting,
}

struct ControlScope {
    current: DesktopControlActivity,
    pending: VecDeque<DesktopControlActivity>,
    yield_when_idle: bool,
}

#[derive(Default)]
struct Projection {
    active: BTreeMap<String, ControlScope>,
    stopped: BTreeSet<String>,
}

impl Projection {
    fn apply(&mut self, conversation_id: &str, event: &AgentRunEvent) -> bool {
        if matches!(
            event.kind,
            AgentRunEventKind::Done | AgentRunEventKind::Error
        ) {
            self.stopped.remove(&event.run_id);
            return self.active.remove(&event.run_id).is_some();
        }
        if self.stopped.contains(&event.run_id) {
            return false;
        }
        if matches!(
            event.phase,
            AgentRunPhase::Paused | AgentRunPhase::AwaitingUserInput
        ) {
            return self.active.remove(&event.run_id).is_some();
        }
        if !matches!(
            event.kind,
            AgentRunEventKind::ToolStarted
                | AgentRunEventKind::ToolProgress
                | AgentRunEventKind::ToolCompleted
        ) {
            return false;
        }
        let run = event.payload.get("run").unwrap_or(&event.payload);
        let Some(call_id) = run.get("callId").and_then(serde_json::Value::as_str) else {
            return false;
        };
        if event.kind == AgentRunEventKind::ToolCompleted {
            let Some(scope) = self.active.get_mut(&event.run_id) else {
                return false;
            };
            let Some(index) = scope
                .pending
                .iter()
                .position(|call| call.call_id == call_id)
            else {
                return false;
            };
            let completed = scope.pending.remove(index).expect("pending control exists");
            if scope.pending.is_empty() {
                let failed = matches!(
                    run.get("status")
                        .and_then(serde_json::Value::as_str)
                        .or(event.status.as_deref()),
                    Some("failed" | "declined" | "cancelled" | "timedOut" | "timed_out")
                );
                if failed || scope.yield_when_idle {
                    self.active.remove(&event.run_id);
                } else {
                    scope.current = completed;
                    scope.current.phase = DesktopControlPhase::Waiting;
                }
                return true;
            }
            if scope.current.call_id == call_id {
                scope.current = scope
                    .pending
                    .front()
                    .expect("another control remains")
                    .clone();
                return true;
            }
            return false;
        }
        // A streaming ToolStarted can contain only a preparing draft. The
        // complete Running transition arrives as ToolProgress after admission.
        if run
            .get("status")
            .and_then(serde_json::Value::as_str)
            .or(event.status.as_deref())
            != Some("running")
        {
            return false;
        }
        let Some(tool_name) = run.get("toolName").and_then(serde_json::Value::as_str) else {
            return false;
        };
        if tool_name == "computer_observe" {
            return false;
        }
        if tool_name != "computer_control" {
            if let Some(scope) = self.active.get_mut(&event.run_id) {
                if scope.pending.is_empty() {
                    return self.active.remove(&event.run_id).is_some();
                }
                // The scheduler announces a whole batch before executing it.
                // An unrelated announcement cannot hide Stop during input.
                scope.yield_when_idle = true;
            }
            return false;
        }
        if self.active.len() >= 64 && !self.active.contains_key(&event.run_id) {
            return false;
        }
        let arguments = run
            .get("arguments")
            .and_then(serde_json::Value::as_str)
            .and_then(|args| serde_json::from_str::<serde_json::Value>(args).ok());
        let activity = DesktopControlActivity {
            conversation_id: conversation_id.into(),
            run_id: event.run_id.clone(),
            turn_id: (!event.turn_id.is_empty()).then(|| event.turn_id.clone()),
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            phase: match run
                .pointer("/artifacts/activity/payload/phase")
                .and_then(serde_json::Value::as_str)
            {
                Some("moving") => DesktopControlPhase::Moving,
                Some("committing") => DesktopControlPhase::Committing,
                Some("waiting") => DesktopControlPhase::Verifying,
                _ => self
                    .active
                    .get(&event.run_id)
                    .filter(|scope| scope.current.call_id == call_id)
                    .map(|scope| scope.current.phase.clone())
                    .unwrap_or(DesktopControlPhase::Controlling),
            },
            window_id: arguments
                .and_then(|args| args.get("window_id").and_then(serde_json::Value::as_u64)),
        };
        if let Some(scope) = self.active.get_mut(&event.run_id) {
            let was_idle = scope.pending.is_empty();
            if let Some(pending) = scope
                .pending
                .iter_mut()
                .find(|call| call.call_id == call_id)
            {
                *pending = activity.clone();
            } else {
                scope.pending.push_back(activity.clone());
            }
            if was_idle {
                scope.yield_when_idle = false;
            }
            if was_idle || scope.current.call_id == call_id {
                let changed = scope.current != activity;
                scope.current = activity;
                return changed;
            }
            return false;
        }
        self.active.insert(
            event.run_id.clone(),
            ControlScope {
                current: activity.clone(),
                pending: VecDeque::from([activity]),
                yield_when_idle: false,
            },
        );
        true
    }

    fn snapshot(&self) -> Vec<DesktopControlActivity> {
        let mut values = self
            .active
            .values()
            .map(|scope| scope.current.clone())
            .collect::<Vec<_>>();
        values.sort_by_key(|item| match item.phase {
            DesktopControlPhase::Controlling
            | DesktopControlPhase::Moving
            | DesktopControlPhase::Committing
            | DesktopControlPhase::Verifying => 0,
            DesktopControlPhase::Waiting => 1,
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
    .transparent(true)
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
    if matches!(
        event.kind,
        AgentRunEventKind::Done | AgentRunEventKind::Error
    ) {
        if !event.turn_id.is_empty() {
            nexa_core::tools::computer_use_tool::release_desktop_session(
                conversation_id,
                &event.turn_id,
            );
        }
    }
    if !matches!(
        event.kind,
        AgentRunEventKind::ToolStarted
            | AgentRunEventKind::ToolCompleted
            | AgentRunEventKind::ToolProgress
            | AgentRunEventKind::Done
            | AgentRunEventKind::Error
            | AgentRunEventKind::Status
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
    publish_status(app);
}

fn publish_status(app: &AppHandle) {
    let handle = app.clone();
    // Never create/show native windows or wait on the main thread from the
    // durable outbox writer. Read the newest projection when the UI runs.
    if let Err(error) = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(WINDOW_LABEL) else {
            return;
        };
        let activities = snapshot(&handle);
        #[cfg(target_os = "windows")]
        nexa_core::tools::computer_use_tool::set_desktop_control_targets(
            &activities
                .iter()
                .map(|activity| activity.window_id.unwrap_or_default())
                .collect::<Vec<_>>(),
        );
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
    nexa_core::agent_pointer::configure(accent, reduced_motion);
    if let Some(browser) = window
        .app_handle()
        .try_state::<crate::browser::state::BrowserState>()
    {
        browser.refresh_pointer_appearance();
    }
    publish_status(window.app_handle());
}

#[tauri::command]
pub async fn stop_desktop_control_cmd(
    app: AppHandle,
    approvals: tauri::State<'_, crate::commands::ApprovalState>,
    conversation_id: String,
    run_id: String,
) -> Result<(), String> {
    let state = app.state::<DesktopControlStatusState>();
    {
        let mut projection = state
            .0
            .lock()
            .map_err(|_| "Computer Use status is unavailable")?;
        let Some(activity) = projection
            .active
            .get(&run_id)
            .map(|scope| &scope.current)
            .filter(|activity| activity.conversation_id == conversation_id)
        else {
            return Ok(());
        };
        let turn_id = activity
            .turn_id
            .as_deref()
            .ok_or("Computer Use turn identity is unavailable")?;
        nexa_core::tools::computer_use_tool::stop_desktop_session(&conversation_id, turn_id);
        projection.active.remove(&run_id);
        projection.stopped.insert(run_id.clone());
    }
    // A pending desktop permission belongs to the same stopped scope. Resolve
    // only those requests; shell/file approvals and the chat stay alive.
    let mut pending = approvals.pending.lock().await;
    let ids = pending
        .iter()
        .filter(|(_, approval)| {
            approval.task_run_id == run_id
                && matches!(
                    approval.request.tool_name.as_str(),
                    "computer_control" | "computer_observe"
                )
        })
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in ids {
        if let Some(approval) = pending.remove(&id) {
            let _ = approval
                .sender
                .send(nexa_core::approval::ApprovalDecision::Deny);
        }
    }
    drop(pending);
    publish_status(&app);
    Ok(())
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
    fn observation_and_browser_tools_do_not_start_desktop_control() {
        for tool in ["computer_observe", "browser", "browser_action", "read_file"] {
            let mut state = Projection::default();
            assert!(
                !state.apply(
                    "chat",
                    &event("run", AgentRunEventKind::ToolStarted, "call", tool)
                ),
                "{tool} must not announce desktop takeover"
            );
            assert!(state.snapshot().is_empty());
        }
    }

    #[test]
    fn worker_phases_survive_heartbeats_and_terminal_completion() {
        let mut state = Projection::default();
        let mut update = event(
            "run",
            AgentRunEventKind::ToolProgress,
            "call",
            "computer_control",
        );
        update.payload["run"]["artifacts"] =
            serde_json::json!({"activity":{"payload":{"phase":"moving"}}});
        assert!(state.apply("chat", &update));
        assert_eq!(state.snapshot()[0].phase, DesktopControlPhase::Moving);
        update.payload["run"]["artifacts"] = serde_json::Value::Null;
        state.apply("chat", &update);
        assert_eq!(state.snapshot()[0].phase, DesktopControlPhase::Moving);
        update.payload["run"]["artifacts"] =
            serde_json::json!({"activity":{"payload":{"phase":"waiting"}}});
        state.apply("chat", &update);
        assert_eq!(state.snapshot()[0].phase, DesktopControlPhase::Verifying);
        state.apply(
            "chat",
            &event(
                "run",
                AgentRunEventKind::ToolCompleted,
                "call",
                "computer_control",
            ),
        );
        assert_eq!(state.snapshot()[0].phase, DesktopControlPhase::Waiting);
    }

    #[test]
    fn mixed_batch_keeps_stop_visible_until_all_control_calls_complete() {
        let mut state = Projection::default();
        assert!(state.apply(
            "chat",
            &event(
                "run",
                AgentRunEventKind::ToolStarted,
                "first",
                "computer_control"
            )
        ));
        state.apply(
            "chat",
            &event(
                "run",
                AgentRunEventKind::ToolStarted,
                "second",
                "computer_control",
            ),
        );
        state.apply(
            "chat",
            &event("run", AgentRunEventKind::ToolStarted, "read", "read_file"),
        );
        assert_eq!(
            state.snapshot().len(),
            1,
            "a mixed batch cannot remove the active Stop surface"
        );
        state.apply(
            "chat",
            &event(
                "run",
                AgentRunEventKind::ToolCompleted,
                "first",
                "computer_control",
            ),
        );
        assert_eq!(state.snapshot()[0].phase, DesktopControlPhase::Controlling);
        state.apply(
            "chat",
            &event(
                "run",
                AgentRunEventKind::ToolCompleted,
                "second",
                "computer_control",
            ),
        );
        assert!(
            state.snapshot().is_empty(),
            "hide once actual control has ended and another tool took over"
        );
    }

    #[test]
    fn streaming_preparation_does_not_take_over_until_running_with_target_arguments() {
        let mut state = Projection::default();
        let mut preparing = event(
            "run",
            AgentRunEventKind::ToolStarted,
            "control",
            "computer_control",
        );
        preparing.payload["run"]["status"] = serde_json::json!("preparing");
        assert!(!state.apply("chat", &preparing));
        let mut running = event(
            "run",
            AgentRunEventKind::ToolProgress,
            "control",
            "computer_control",
        );
        running.payload["run"]["status"] = serde_json::json!("running");
        running.payload["run"]["arguments"] =
            serde_json::json!("{\"window_id\":1234,\"action\":\"click\"}");
        assert!(state.apply("chat", &running));
        assert_eq!(state.snapshot()[0].window_id, Some(1234));
    }

    #[test]
    fn desktop_control_ends_on_other_tools_failed_input_and_suspension() {
        for end in [
            event("run", AgentRunEventKind::ToolStarted, "browser", "browser"),
            event("run", AgentRunEventKind::ToolStarted, "shell", "shell"),
            AgentRunEvent::status_update(
                "run",
                None,
                2,
                AgentRunPhase::Paused,
                "Paused",
                Some("paused"),
                None,
            ),
            AgentRunEvent::status_update(
                "run",
                None,
                2,
                AgentRunPhase::AwaitingUserInput,
                "Waiting",
                Some("awaiting_user_input"),
                None,
            ),
        ] {
            let mut state = Projection::default();
            assert!(state.apply(
                "chat",
                &event(
                    "run",
                    AgentRunEventKind::ToolStarted,
                    "control",
                    "computer_control"
                )
            ));
            if end.kind == AgentRunEventKind::ToolStarted {
                state.apply(
                    "chat",
                    &event(
                        "run",
                        AgentRunEventKind::ToolCompleted,
                        "control",
                        "computer_control",
                    ),
                );
            }
            assert!(state.apply("chat", &end));
            assert!(state.snapshot().is_empty());
        }
        let mut state = Projection::default();
        state.apply(
            "chat",
            &event(
                "run",
                AgentRunEventKind::ToolStarted,
                "control",
                "computer_control",
            ),
        );
        let mut failed = event(
            "run",
            AgentRunEventKind::ToolCompleted,
            "control",
            "computer_control",
        );
        failed.payload["run"]["status"] = serde_json::json!("failed");
        assert!(state.apply("chat", &failed));
        assert!(state.snapshot().is_empty());
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
                "control-b",
                "computer_control"
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
                "control-b",
                "computer_control"
            )
        ));
        assert_eq!(state.snapshot()[0].phase, DesktopControlPhase::Waiting);
        assert!(state.apply("chat-b", &event("b", AgentRunEventKind::Done, "", "")));
        assert!(state.snapshot().is_empty());
    }
}
