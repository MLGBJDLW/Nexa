//! Owner-bound remote dictation sessions; provider credentials never leave the host.
use super::{realtime_transcription::*, AppState, RealtimeTranscriptionState};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Manager};

#[derive(Clone)]
struct Binding {
    owner: String,
    native_id: Option<String>,
    sample_rate: u32,
    final_text: Option<String>,
}
#[derive(Default)]
pub struct RemoteVoiceState {
    sessions: Arc<Mutex<HashMap<String, Binding>>>,
}
struct PendingStart {
    sessions: Arc<Mutex<HashMap<String, Binding>>>,
    id: String,
    committed: bool,
}
impl Drop for PendingStart {
    fn drop(&mut self) {
        if !self.committed {
            self.sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.id);
        }
    }
}

fn binding(app: &AppHandle, owner: &str, id: &str) -> Result<Binding, String> {
    app.state::<RemoteVoiceState>()
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .filter(|binding| binding.owner == owner)
        .cloned()
        .ok_or_else(|| "remote_voice_missing".into())
}

pub async fn start(app: &AppHandle, owner: &str, id: String) -> Result<Value, String> {
    uuid::Uuid::parse_str(&id).map_err(|_| "Invalid dictation request ID")?;
    let state = app.state::<RemoteVoiceState>();
    {
        let mut sessions = state.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = sessions.get(&id) {
            if existing.owner != owner {
                return Err("remote_voice_missing".into());
            }
            if existing.native_id.is_some() {
                return Ok(json!({"sessionId":id,"sampleRate":existing.sample_rate}));
            }
            return Err("remote_voice_starting".into());
        }
        if sessions
            .values()
            .any(|binding| binding.owner == owner && binding.final_text.is_none())
        {
            return Err("remote_voice_active".into());
        }
        sessions.retain(|_, binding| binding.owner != owner);
        sessions.insert(
            id.clone(),
            Binding {
                owner: owner.into(),
                native_id: None,
                sample_rate: 0,
                final_text: None,
            },
        );
    }
    let mut pending = PendingStart {
        sessions: state.sessions.clone(),
        id: id.clone(),
        committed: false,
    };
    let config = app
        .state::<AppState>()
        .db_executor
        .write(|db| Ok(db.load_app_config()?.speech_to_text))
        .await
        .map_err(|e| e.to_string())?
        .value;
    if !config.is_configured() {
        return Err("remote_voice_setup_required".into());
    }
    let callback_app = app.clone();
    let callback_owner = owner.to_owned();
    let callback_id = id.clone();
    let (native_id, sample_rate) = start_remote_transcription(
        config,
        &app.state::<RealtimeTranscriptionState>(),
        Arc::new(move |mut event| {
            event["sessionId"] = Value::String(callback_id.clone());
            callback_app
                .state::<crate::remote::RemoteState>()
                .publish_voice(&callback_owner, &event);
        }),
    )
    .await?;
    let committed = {
        let mut sessions = state.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(binding) = sessions
            .get_mut(&id)
            .filter(|binding| binding.owner == owner)
        {
            binding.native_id = Some(native_id.clone());
            binding.sample_rate = sample_rate;
            true
        } else {
            false
        }
    };
    if !committed {
        cancel_realtime_transcription_cmd(native_id, app.state()).await?;
        return Err("remote_voice_cancelled".into());
    }
    pending.committed = true;
    Ok(json!({"sessionId":id,"sampleRate":sample_rate}))
}

pub async fn audio(app: &AppHandle, owner: &str, id: &str, data: String) -> Result<Value, String> {
    let binding = binding(app, owner, id)?;
    if binding.final_text.is_some() {
        return Err("remote_voice_closed".into());
    }
    let native = binding.native_id.ok_or("remote_voice_starting")?;
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| "Invalid dictation audio")?;
    append_live_transcription_audio(&app.state::<RealtimeTranscriptionState>(), &native, bytes)
        .await?;
    Ok(Value::Null)
}
pub async fn finish(app: &AppHandle, owner: &str, id: &str) -> Result<Value, String> {
    let binding = binding(app, owner, id)?;
    if let Some(text) = binding.final_text {
        return Ok(json!({"text":text}));
    }
    let native = binding.native_id.ok_or("remote_voice_starting")?;
    let text = match finish_realtime_transcription_cmd(native, app.state()).await {
        Ok(text) => text,
        Err(error) => {
            let _ = cancel(app, owner, id).await;
            return Err(error);
        }
    };
    if let Some(binding) = app
        .state::<RemoteVoiceState>()
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(id)
    {
        binding.final_text = Some(text.clone());
    }
    Ok(json!({"text":text}))
}
pub async fn cancel(app: &AppHandle, owner: &str, id: &str) -> Result<Value, String> {
    let removed = {
        let state = app.state::<RemoteVoiceState>();
        let mut sessions = state.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if sessions
            .get(id)
            .is_some_and(|binding| binding.owner != owner)
        {
            return Err("remote_voice_missing".into());
        }
        sessions.remove(id)
    };
    if let Some(native) = removed.and_then(|binding| binding.native_id) {
        cancel_realtime_transcription_cmd(native, app.state()).await?;
    }
    Ok(Value::Null)
}
pub async fn stop_owner(app: &AppHandle, owner: &str) {
    let ids = app
        .state::<RemoteVoiceState>()
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(_, binding)| binding.owner == owner)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    for id in ids {
        let _ = cancel(app, owner, &id).await;
    }
}
