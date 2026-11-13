//! A paired human viewer uses the existing capture, input, and receipt runtime.
//! Only local IPC can grant a source. Grants survive navigation, never restart.
use crate::{commands::AppState, remote::RemoteState};
use nexa_core::tools::{computer_use_tool, Tool, ToolExecutionContext};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Manager};

struct Share {
    id: String,
    source: String,
    control: bool,
    gate: tokio::sync::Mutex<()>,
    stopped: nexa_core::agent::CancellationToken,
    ready: AtomicBool,
}
impl Drop for Share {
    fn drop(&mut self) {
        self.stopped.cancel();
        computer_use_tool::release_desktop_session(&self.id, &self.id);
    }
}
#[derive(Default)]
pub struct RemoteScreenState {
    grants: Mutex<HashMap<String, Arc<Share>>>,
    phones: Mutex<HashMap<String, Arc<PhoneView>>>,
}
struct PhoneView {
    device: String,
    cancel: nexa_core::agent::CancellationToken,
}
impl RemoteScreenState {
    fn begin(&self, owner: &str, share: Arc<Share>) {
        if let Some(previous) = self
            .grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(owner.into(), share)
        {
            previous.stopped.cancel();
            computer_use_tool::stop_desktop_session(&previous.id, &previous.id);
        }
    }
    fn finish_grant(
        &self,
        owner: &str,
        share: &Arc<Share>,
        validated: Result<(), String>,
    ) -> Result<(), String> {
        let mut grants = self.grants.lock().unwrap_or_else(|e| e.into_inner());
        let current = grants
            .get(owner)
            .is_some_and(|current| Arc::ptr_eq(current, share));
        if validated.is_err() || share.stopped.is_cancelled() || !current {
            share.stopped.cancel();
            if current {
                grants.remove(owner);
            }
            return Err(validated
                .err()
                .unwrap_or_else(|| "Screen sharing stopped or the device disconnected.".into()));
        }
        share.ready.store(true, Ordering::Release);
        Ok(())
    }
    pub fn stop(&self, owner: &str) {
        self.phones
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, view| {
                if view.device == owner {
                    view.cancel.cancel();
                    false
                } else {
                    true
                }
            });
        if let Some(share) = self
            .grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(owner)
        {
            share.stopped.cancel();
            computer_use_tool::stop_desktop_session(&share.id, &share.id);
        }
    }
    pub fn stop_all(&self) {
        self.phones
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, view| {
                view.cancel.cancel();
                false
            });
        let owners = self
            .grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for owner in owners {
            self.stop(&owner);
        }
    }
    fn share(&self, owner: &str) -> Result<Arc<Share>, String> {
        self.grants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(owner)
            .filter(|share| share.ready.load(Ordering::Acquire) && !share.stopped.is_cancelled())
            .cloned()
            .ok_or(
                "Choose a window and enable screen sharing for this device on the computer.".into(),
            )
    }
    pub fn status(&self, owner: &str) -> Value {
        self.share(owner)
            .map(|share| json!({"active":true,"sessionId":share.id,"control":share.control}))
            .unwrap_or(json!({"active":false}))
    }
    pub fn statuses(&self) -> Value {
        json!(self.grants.lock().unwrap_or_else(|e|e.into_inner()).iter().filter(|(_, share)| share.ready.load(Ordering::Acquire) && !share.stopped.is_cancelled()).map(|(owner, share)|
            json!({"deviceId":owner,"active":true,"control":share.control,"source":share.source})).collect::<Vec<_>>())
    }
}

#[tauri::command]
pub fn remote_screen_status_cmd(app: AppHandle) -> Value {
    app.state::<RemoteScreenState>().statuses()
}

#[tauri::command]
pub async fn remote_screen_share_cmd(
    app: AppHandle,
    device_id: String,
    source: Option<String>,
    control: bool,
) -> Result<Value, String> {
    let screens = app.state::<RemoteScreenState>();
    screens.stop(&device_id);
    if let Some(source) = source {
        let server = app.state::<RemoteState>().server()?;
        if !server.connected_device_ids().contains(&device_id) {
            return Err("Connect this paired device before sharing.".into());
        }
        let id = format!("remote-screen:{}", uuid::Uuid::new_v4());
        let share = Arc::new(Share {
            id,
            source,
            control,
            gate: tokio::sync::Mutex::new(()),
            stopped: Default::default(),
            ready: AtomicBool::new(false),
        });
        // Publish a pending, unusable grant before awaiting capture, so Stop,
        // disconnect, replacement, and revoke invalidate in-progress starts.
        screens.begin(&device_id, share.clone());
        let validated = tokio::select! {
            _ = share.stopped.cancelled() => Err("Screen sharing was stopped before it started.".into()),
            result = computer_use_tool::observe_user_share_window(&share.source, &share.id) => result.map(|_| ()).map_err(|e|e.to_string()),
        };
        if server.shutdown.is_cancelled() || !server.connected_device_ids().contains(&device_id) {
            share.stopped.cancel();
        }
        screens.finish_grant(&device_id, &share, validated)?;
    }
    crate::app_events::emit_main_window_event(&app, "remote:screen-changed", &());
    Ok(screens.status(&device_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending() -> Arc<Share> {
        Arc::new(Share {
            id: uuid::Uuid::new_v4().to_string(),
            source: "test-window".into(),
            control: true,
            gate: tokio::sync::Mutex::new(()),
            stopped: Default::default(),
            ready: AtomicBool::new(false),
        })
    }
    #[test]
    fn stop_invalidates_pending_grant_and_phone_view() {
        let screens = RemoteScreenState::default();
        let share = pending();
        screens.begin("phone", share.clone());
        let view = Arc::new(PhoneView {
            device: "phone".into(),
            cancel: Default::default(),
        });
        screens
            .phones
            .lock()
            .unwrap()
            .insert("viewer".into(), view.clone());
        assert!(screens.share("phone").is_err());
        screens.stop("phone");
        assert!(share.stopped.is_cancelled());
        assert!(view.cancel.is_cancelled());
        assert!(screens.finish_grant("phone", &share, Ok(())).is_err());
        assert_eq!(screens.status("phone")["active"], false);
        assert!(screens.phones.lock().unwrap().is_empty());
    }
    #[test]
    fn replaced_start_cannot_publish_or_remove_newer_grant() {
        let screens = RemoteScreenState::default();
        let old = pending();
        let new = pending();
        screens.begin("phone", old.clone());
        screens.begin("phone", new.clone());
        assert!(old.stopped.is_cancelled());
        assert!(screens.finish_grant("phone", &old, Ok(())).is_err());
        screens.finish_grant("phone", &new, Ok(())).unwrap();
        assert!(Arc::ptr_eq(&screens.share("phone").unwrap(), &new));
        screens.stop_all();
        assert!(new.stopped.is_cancelled());
        assert!(screens.share("phone").is_err());
    }
}

pub async fn frame(app: &AppHandle, owner: &str) -> Result<Value, String> {
    let share = app.state::<RemoteScreenState>().share(owner)?;
    let _gate = share
        .gate
        .try_lock()
        .map_err(|_| "The previous screen operation is still finishing.")?;
    let mut result = tokio::select! {
        _ = share.stopped.cancelled() => return Err("Screen sharing stopped.".into()),
        result = computer_use_tool::observe_user_share_window(&share.source, &share.id) => result.map_err(|e|e.to_string())?,
    };
    result["controlAvailable"] = json!(share.control && result["controlAvailable"] == true);
    Ok(result)
}

pub async fn input(app: &AppHandle, owner: &str, mut input: Value) -> Result<Value, String> {
    let share = app.state::<RemoteScreenState>().share(owner)?;
    if !share.control {
        return Err("This share is view only. Enable control on the computer.".into());
    }
    let _gate = share
        .gate
        .try_lock()
        .map_err(|_| "The previous screen operation is still finishing.")?;
    let fields = input.as_object_mut().ok_or("Invalid screen input")?;
    if fields.keys().any(|key| {
        ![
            "action",
            "observation_id",
            "window_id",
            "x",
            "y",
            "text",
            "key_sequence",
            "scroll_y",
            "scroll_x",
            "button",
        ]
        .contains(&key.as_str())
    }) || !matches!(
        fields.get("action").and_then(Value::as_str),
        Some("click" | "type_text" | "key" | "scroll" | "focus_window")
    ) {
        return Err("Unsupported screen input".into());
    }
    fields.insert("coordinate_space".into(), json!("image_pixels"));
    fields.insert("include_elements".into(), json!(false));
    let state = app.state::<AppState>();
    let runtime = nexa_core::activity::ActivityRuntime::with_database((*state.db).clone())
        .map_err(|e| e.to_string())?;
    let args = input.to_string();
    let call = uuid::Uuid::new_v4().to_string();
    let context = ToolExecutionContext::new(&call, &args, &state.db, &[])
        .with_conversation_id(Some(&share.id))
        .with_turn_id(Some(&share.id))
        .with_activity_runtime(&runtime);
    let result = tokio::select! {
        _ = share.stopped.cancelled() => return Err("Screen control stopped. The outcome may be unknown; refresh before acting again.".into()),
        result = computer_use_tool::ComputerControlTool.execute(context) => result.map_err(|e|e.to_string())?,
    };
    if result.is_error {
        return Err(result.content);
    }
    Ok(json!({"accepted":true}))
}

#[tauri::command]
pub fn remote_phone_screen_open_cmd(app: AppHandle, device_id: String) -> Result<String, String> {
    let server = app.state::<RemoteState>().server()?;
    if !server.phone_control_devices()["devices"]
        .as_array()
        .is_some_and(|devices| {
            devices.iter().any(|device| {
                device["id"] == device_id
                    && device["connected"] == true
                    && device["controlEnabled"] == true
            })
        })
    {
        return Err("Connect and enable this computer's phone control on Android first.".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    app.state::<RemoteScreenState>()
        .phones
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(
            id.clone(),
            Arc::new(PhoneView {
                device: device_id,
                cancel: Default::default(),
            }),
        );
    Ok(id)
}

#[tauri::command]
pub fn remote_phone_screen_stop_cmd(app: AppHandle, session_id: String) {
    if let Some(view) = app
        .state::<RemoteScreenState>()
        .phones
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&session_id)
    {
        view.cancel.cancel();
    }
}

#[tauri::command]
pub async fn remote_phone_screen_cmd(
    app: AppHandle,
    session_id: String,
    mut arguments: Value,
) -> Result<Value, String> {
    let action = arguments["action"]
        .as_str()
        .ok_or("Specify a phone action")?;
    if !matches!(
        action,
        "screenshot" | "tap" | "set_text" | "back" | "home" | "scroll_forward" | "scroll_backward"
    ) {
        return Err("Unsupported phone screen action".into());
    }
    let view = app
        .state::<RemoteScreenState>()
        .phones
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&session_id)
        .cloned()
        .ok_or("This phone viewer was stopped. Open it again before sending input.")?;
    if action == "screenshot" {
        return app
            .state::<RemoteState>()
            .server()?
            .control_phone(&view.device, arguments, view.cancel.clone())
            .await;
    }
    // Human-driven input uses the canonical phone receipt path as agent input.
    arguments["deviceId"] = json!(view.device);
    let state = app.state::<AppState>();
    let runtime = nexa_core::activity::ActivityRuntime::with_database((*state.db).clone())
        .map_err(|e| e.to_string())?;
    let args = arguments.to_string();
    let call = uuid::Uuid::new_v4().to_string();
    let mut context = ToolExecutionContext::new(&call, &args, &state.db, &[])
        .with_conversation_id(Some(&session_id))
        .with_turn_id(Some(&session_id))
        .with_execution_id(&call)
        .with_activity_runtime(&runtime);
    context.cancel_token = Some(&view.cancel);
    let tool = nexa_core::tools::phone_control_tool::PhoneControlTool(Arc::new(
        crate::phone_control::PairedPhoneHost(app.clone()),
    ));
    let result = tool.execute(context).await.map_err(|e| e.to_string())?;
    if result.is_error {
        return Err(result.content);
    }
    Ok(json!({"accepted":true}))
}
