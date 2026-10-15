//! Native Google Interactions audio envelopes shared by speech input and output.
use crate::error::CoreError;
use serde_json::{json, Value};

pub(crate) fn endpoint(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.ends_with("/interactions") {
        base.into()
    } else {
        format!("{base}/interactions")
    }
}

pub(crate) fn transcription_request(model: &str, audio: &str, languages: &[String]) -> Value {
    let mut transcription = json!({"mode": {"type": "verbatim"}});
    if !languages.is_empty() {
        transcription["language_codes"] = json!(languages);
    }
    json!({
        "model": model,
        "input": [{"type": "audio", "data": audio, "mime_type": "audio/wav"}],
        "generation_config": {"transcription_config": transcription},
        "store": false
    })
}

pub(crate) fn synthesis_request(model: &str, text: &str, voice: &str) -> Value {
    json!({
        "model": model,
        "input": [{"type": "user_input", "content": [{"type": "text", "text": text}]}],
        "response_format": {"type": "audio", "mime_type": "audio/wav"},
        "generation_config": {"speech_config": [{"voice": voice}]},
        "store": false
    })
}

pub(crate) fn output_content<'a>(
    value: &'a Value,
    kind: &str,
) -> Result<Vec<&'a Value>, CoreError> {
    if value["status"] != "completed" {
        let status = value["status"].as_str().unwrap_or("missing status");
        let cause = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("Interaction did not complete");
        return Err(CoreError::Llm(format!(
            "Google audio {status}: {}",
            cause.chars().take(500).collect::<String>()
        )));
    }
    let content = value["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|step| step["type"] == "model_output")
        .flat_map(|step| step["content"].as_array().into_iter().flatten())
        .filter(|content| content["type"] == kind)
        .collect::<Vec<_>>();
    if content.is_empty() {
        return Err(CoreError::Parse(format!(
            "Google audio response has no {kind} output"
        )));
    }
    Ok(content)
}

pub(crate) fn transcript(value: &Value) -> Result<String, CoreError> {
    let mut result = String::new();
    for content in output_content(value, "text")? {
        let text = content["text"]
            .as_str()
            .ok_or_else(|| CoreError::Parse("Google transcript text is missing".into()))?;
        if !result.is_empty() && !text.is_empty() {
            result.push(' ');
        }
        result.push_str(text);
    }
    Ok(result.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_uses_native_envelopes_and_only_completed_model_outputs() {
        let request = synthesis_request("gemini-3.8-flash-tts", "Hello", "Kore");
        assert_eq!(request["input"][0]["content"][0]["text"], "Hello");
        assert_eq!(request["response_format"]["mime_type"], "audio/wav");
        assert_eq!(request["store"], false);
        let asr = transcription_request("gemini-3.5-transcribe", "wav", &["zh-CN".into()]);
        assert_eq!(
            asr["generation_config"]["transcription_config"]["language_codes"][0],
            "zh-CN"
        );
        let response = json!({"status":"completed", "steps":[
            {"type":"user_input","content":[{"type":"text","text":"Do not return input"}]},
            {"type":"model_output","content":[{"type":"text","text":"First."}]},
            {"type":"model_output","content":[{"type":"text","text":"Second."}]}
        ]});
        assert_eq!(transcript(&response).unwrap(), "First. Second.");
        assert!(transcript(&json!({"status":"failed","error":{"message":"No audio"}})).is_err());
        assert!(transcript(&json!({"status":"completed","steps":[]})).is_err());
    }
}
