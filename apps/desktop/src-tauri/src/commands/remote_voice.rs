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
    transcript: String,
    sequence: u64,
    error: Option<String>,
}
impl Binding {
    fn record_event(&mut self, event: &Value) {
        let sequence = event["sequence"].as_u64().unwrap_or(0);
        if sequence <= self.sequence {
            return;
        }
        self.sequence = sequence;
        match event["kind"].as_str() {
            Some("interim" | "final") => {
                if let Some(text) = event["text"].as_str() {
                    self.transcript = text.to_owned();
                }
            }
            Some("error") => self.error = event["text"].as_str().map(str::to_owned),
            _ => {}
        }
    }
    fn snapshot(&self, id: &str) -> Value {
        json!({"sessionId":id,"sampleRate":self.sample_rate,"sequence":self.sequence,
            "text":self.final_text.as_ref().unwrap_or(&self.transcript),"error":self.error,
            "phase":if self.error.is_some() { "failed" } else if self.final_text.is_some() { "finished" }
                else if self.native_id.is_some() { "recording" } else { "starting" }})
    }
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
                transcript: String::new(),
                sequence: 0,
                error: None,
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
            {
                let state = callback_app.state::<RemoteVoiceState>();
                let mut sessions = state.sessions.lock().unwrap_or_else(|e| e.into_inner());
                let Some(binding) = sessions
                    .get_mut(&callback_id)
                    .filter(|binding| binding.owner == callback_owner)
                else {
                    return;
                };
                binding.record_event(&event);
            }
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

pub fn snapshot(app: &AppHandle, owner: &str, id: &str) -> Result<Value, String> {
    Ok(binding(app, owner, id)?.snapshot(id))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_snapshot_keeps_the_last_transcript_and_terminal_failure() {
        let mut binding = Binding {
            owner: "phone".into(),
            native_id: Some("native".into()),
            sample_rate: 24_000,
            final_text: None,
            transcript: String::new(),
            sequence: 0,
            error: None,
        };
        binding.record_event(&json!({"sequence":1,"kind":"interim","text":"保留文字"}));
        binding.record_event(&json!({"sequence":3,"kind":"final","text":"保留文字和结尾"}));
        binding.record_event(&json!({"sequence":2,"kind":"interim","text":"旧结果"}));
        assert_eq!(binding.snapshot("remote")["text"], "保留文字和结尾");
        binding.record_event(&json!({"sequence":4,"kind":"error","text":"Provider disconnected"}));
        let snapshot = binding.snapshot("remote");
        assert_eq!(snapshot["sequence"], 4);
        assert_eq!(snapshot["text"], "保留文字和结尾");
        assert_eq!(snapshot["phase"], "failed");
        assert_eq!(snapshot["error"], "Provider disconnected");
        assert!(snapshot.get("nativeId").is_none());
        assert!(snapshot.get("owner").is_none());
    }
}
