//! DashScope's duplex task protocol, shared by Qwen Audio ASR and TTS.

use serde_json::{json, Value};
use url::Url;

pub fn task_endpoint(base_url: &str) -> Result<String, String> {
    let mut url = Url::parse(base_url.trim()).map_err(|e| format!("Invalid DashScope URL: {e}"))?;
    let scheme = match url.scheme() {
        "https" | "wss" => "wss",
        "http" | "ws" => "ws",
        _ => return Err("DashScope requires an HTTP or WebSocket URL".into()),
    };
    url.set_scheme(scheme)
        .map_err(|_| "Invalid DashScope scheme")?;
    let path = url.path().trim_end_matches('/');
    // Preserve the authority (including workspace domains and custom gateways).
    // Older saved TTS configurations used the HTTP service path.
    let path = if path.is_empty()
        || matches!(
            path,
            "/api/v1/services/audio/tts" | "/api/v1/services/audio/tts/SpeechSynthesizer"
        ) {
        "/api-ws/v1/inference".to_string()
    } else if path.ends_with("/inference") {
        path.to_string()
    } else {
        format!("{path}/inference")
    };
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

pub fn task_message(action: &str, task_id: &str, payload: Value) -> Value {
    json!({"header": {"action": action, "task_id": task_id, "streaming": "duplex"}, "payload": payload})
}

pub fn task_error(event: &Value) -> String {
    event
        .pointer("/header/error_message")
        .and_then(Value::as_str)
        .or_else(|| event.pointer("/payload/message").and_then(Value::as_str))
        .unwrap_or("DashScope speech task failed")
        .to_string()
}

pub fn is_streaming_asr_model(model: &str) -> bool {
    matches!(
        model.trim(),
        "qwen-audio-3.1-asr-flash-streaming"
            | "qwen-audio-3.0-asr-flash-streaming"
            | "qwen-audio-3.1-asr-flash-message"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_endpoints_preserve_region_and_migrate_legacy_tts_paths() {
        for (base, expected) in [
            (
                "https://dashscope.aliyuncs.com/api/v1/services/audio/tts",
                "wss://dashscope.aliyuncs.com/api-ws/v1/inference",
            ),
            (
                "https://workspace.ap-southeast-1.maas.aliyuncs.com/api-ws/v1",
                "wss://workspace.ap-southeast-1.maas.aliyuncs.com/api-ws/v1/inference",
            ),
            (
                "ws://localhost:1234/gateway/inference/",
                "ws://localhost:1234/gateway/inference",
            ),
        ] {
            assert_eq!(task_endpoint(base).unwrap(), expected);
        }
        assert!(task_endpoint("file:///tmp/speech").is_err());
    }
}
