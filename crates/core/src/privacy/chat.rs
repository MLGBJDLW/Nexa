//! Privacy projections for derived conversation data. Protocol identifiers and
//! authored user messages are not source text. Opaque provider replay is revoked
//! as a unit; rewriting a signed or encrypted payload would not be valid.

use super::{builtin_redact_rules, redaction_fingerprint, PrivacyConfig};
use crate::error::CoreError;
use crate::llm::provider_turn::{ProviderReplayPayload, ProviderTurnEnvelope};
use crate::llm::reasoning_profile::ReasoningCaptureStatus;
use crate::llm::{ContentPart, Message, ToolCallRequest};
use regex::Regex;
use rusqlite::{functions::FunctionFlags, Connection};
use serde_json::Value;
use std::sync::{Arc, Mutex};

struct Redactor {
    rules: Vec<(Regex, String)>,
    fingerprint: String,
    allow_legacy_replay: bool,
}

impl Redactor {
    fn new(config: &PrivacyConfig, revision: &str) -> Result<Self, CoreError> {
        let rules = if config.enabled {
            builtin_redact_rules()
                .into_iter()
                .chain(config.redact_patterns.clone())
                .map(|rule| {
                    Ok((
                        Regex::new(&rule.pattern).map_err(|error| {
                            CoreError::InvalidInput(format!("Invalid privacy expression: {error}"))
                        })?,
                        rule.replacement,
                    ))
                })
                .collect::<Result<Vec<_>, CoreError>>()?
        } else {
            Vec::new()
        };
        Ok(Self {
            rules,
            fingerprint: blake3::hash(
                format!("{}:{revision}", redaction_fingerprint(config)?).as_bytes(),
            )
            .to_hex()
            .to_string(),
            allow_legacy_replay: revision == "initial",
        })
    }

    fn text(&self, text: &str) -> String {
        self.rules
            .iter()
            .fold(text.to_owned(), |text, (pattern, replacement)| {
                pattern
                    .replace_all(&text, replacement.as_str())
                    .into_owned()
            })
    }

    fn arguments(&self, arguments: &str) -> String {
        match serde_json::from_str::<Value>(arguments) {
            Ok(mut value) => {
                self.data(&mut value);
                value.to_string()
            }
            Err(_) => self.text(arguments),
        }
    }

    /// Arbitrary tool data has no protocol IDs. Parse JSON before replacing
    /// strings so quotes/backslashes in user rules cannot corrupt its syntax.
    fn data(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.text(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.data(item)),
            Value::Object(map) => {
                let original = std::mem::take(map);
                for (key, mut value) in original {
                    self.data(&mut value);
                    let base = self.text(&key);
                    let mut key = base.clone();
                    let mut suffix = 2;
                    while map.contains_key(&key) {
                        key = format!("{base} [redacted {suffix}]");
                        suffix += 1;
                    }
                    map.insert(key, value);
                }
            }
            _ => {}
        }
    }

    fn calls(&self, calls: &mut [ToolCallRequest], revoke: bool) -> bool {
        let mut changed = false;
        for call in calls {
            let arguments = self.arguments(&call.arguments);
            let redacted = serde_json::from_str::<Value>(&arguments).ok()
                != serde_json::from_str::<Value>(&call.arguments).ok()
                || (serde_json::from_str::<Value>(&arguments).is_err()
                    && arguments != call.arguments);
            if redacted {
                call.arguments = arguments;
                changed = true;
            }
            if revoke || redacted {
                call.thought_signature = None;
            }
        }
        changed
    }

    fn envelope(&self, envelope: &mut ProviderTurnEnvelope) {
        let stale = envelope.privacy_fingerprint.as_deref() != Some(&self.fingerprint)
            && !(self.allow_legacy_replay && envelope.privacy_fingerprint.is_none());
        let visible = self.text(&envelope.visible_content);
        let changed = visible != envelope.visible_content;
        envelope.visible_content = visible;
        let arguments_changed = self.calls(&mut envelope.tool_calls, stale || changed);
        if stale || changed || arguments_changed {
            for call in &mut envelope.tool_calls {
                call.thought_signature = None;
            }
            envelope.provider_items.clear();
            envelope.replay_payload = ProviderReplayPayload::None;
            envelope.capture_status = ReasoningCaptureStatus::Redacted;
            envelope.raw_response_digest = blake3::hash(
                &serde_json::to_vec(&serde_json::json!({
                    "route": &envelope.route,
                    "visibleContent": &envelope.visible_content,
                    "providerItems": &envelope.provider_items,
                    "toolCalls": &envelope.tool_calls,
                }))
                .expect("provider envelope is serializable"),
            )
            .to_hex()
            .to_string();
        }
        if stale || changed || arguments_changed || envelope.privacy_fingerprint.is_some() {
            envelope.privacy_fingerprint = Some(self.fingerprint.clone());
        }
    }

    fn structured(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.text(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.structured(item)),
            Value::Object(map) => {
                if map.contains_key("turnItemId") && map.contains_key("replayPayload") {
                    if let Ok(mut envelope) =
                        serde_json::from_value::<ProviderTurnEnvelope>(value.clone())
                    {
                        self.envelope(&mut envelope);
                        *value = serde_json::to_value(envelope)
                            .expect("provider envelope is serializable");
                    } else {
                        // An unrecognised replay object cannot be proven safe.
                        *value = Value::Null;
                    }
                    return;
                }
                if map.contains_key("arguments")
                    && map.contains_key("id")
                    && map.contains_key("name")
                {
                    if let Ok(mut call) = serde_json::from_value::<ToolCallRequest>(value.clone()) {
                        self.calls(std::slice::from_mut(&mut call), true);
                        *value = serde_json::to_value(call).expect("tool call is serializable");
                        return;
                    }
                }
                let Value::Object(map) = value else {
                    unreachable!()
                };
                if matches!(
                    map.get("kind").and_then(Value::as_str),
                    Some("outputDelta" | "outputSnapshot" | "thinking")
                ) {
                    if let Some(payload) = map.get_mut("payload") {
                        revoke_output_fragment(payload);
                    }
                }
                map.remove("reasoningEnvelope");
                map.remove("thoughtSignature");
                map.remove("thought_signature");
                let tool_discovery =
                    map.get("kind").and_then(Value::as_str) == Some("toolSearchResults");
                let mcp_result = map.get("kind").and_then(Value::as_str) == Some("mcpToolResult");
                for (key, value) in map.iter_mut() {
                    match key.as_str() {
                        // Only protocol metadata is exempt. Names in arbitrary
                        // tool data are processed by the data projection below.
                        "id" | "kind" | "type" | "role" | "status" | "phase" | "callId"
                        | "call_id" | "toolCallId" | "tool_call_id" | "toolName" | "tool_name"
                        | "conversationId" | "turnId" | "runId" | "subtaskRunId" | "messageId"
                        | "interactionId" | "documentId" | "sourceId" | "chunkId" | "revision"
                        | "route" | "providerEndpointId" | "modelId" => {}
                        "data" | "metadata" | "structuredContent" | "meta" | "_meta" => {
                            self.data(value)
                        }
                        "toolIdentity" if mcp_result => {}
                        "matches" if tool_discovery => {
                            if let Some(matches) = value.as_array_mut() {
                                for item in matches {
                                    let name = item.get("name").cloned();
                                    self.structured(item);
                                    if let (Some(name), Some(item)) = (name, item.as_object_mut()) {
                                        item.insert("name".into(), name);
                                    }
                                }
                            }
                        }
                        "arguments" if value.is_string() => {
                            *value =
                                Value::String(self.arguments(value.as_str().unwrap_or_default()));
                        }
                        _ => self.structured(value),
                    }
                }
            }
            _ => {}
        }
    }
}

/// Sanitize a model-facing copy, including older histories loaded before the
/// current invocation registered its cancellation lease. Durable user text is
/// deliberately handled separately by the database projections.
fn redact_context_messages(policy: &ChatPrivacyPolicy, messages: &mut [Message]) {
    if !policy.config.enabled {
        return;
    }
    let redactor = &policy.redactor;
    for message in messages {
        let mut replay_current = redactor.allow_legacy_replay;
        for part in &mut message.parts {
            match part {
                ContentPart::Text { text } => *text = redactor.text(text),
                ContentPart::ProviderTurn { envelope } => {
                    redactor.envelope(envelope);
                    replay_current = envelope.capture_status != ReasoningCaptureStatus::Redacted;
                }
                ContentPart::Image { .. } => {}
            }
        }
        if let Some(calls) = &mut message.tool_calls {
            redactor.calls(calls, !replay_current);
        }
        if let Some(reasoning) = &mut message.reasoning_content {
            if replay_current {
                *reasoning = redactor.text(reasoning);
            } else {
                message.reasoning_content = None;
            }
        }
    }
}

pub struct ChatPrivacyPolicy {
    pub(super) config: PrivacyConfig,
    redactor: Redactor,
    revision: String,
}

impl ChatPrivacyPolicy {
    pub(crate) fn load(conn: &Connection) -> Result<Self, CoreError> {
        let config = super::load_config_on(conn)?;
        let revision: String = conn.query_row(
            "SELECT revision FROM privacy_chat_state WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let redactor = Redactor::new(&config, &revision)?;
        Ok(Self {
            config,
            redactor,
            revision,
        })
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn fingerprint(&self) -> &str {
        &self.redactor.fingerprint
    }
    pub fn redact_text(&self, text: &str) -> String {
        if self.config.enabled {
            self.redactor.text(text)
        } else {
            text.to_owned()
        }
    }
    pub fn redact_context_messages(&self, messages: &mut [Message]) {
        redact_context_messages(self, messages);
    }
}

pub(crate) fn redact_tool_output(
    config: &PrivacyConfig,
    content: &mut String,
    context: &mut String,
    artifacts: &mut Option<Value>,
) -> Result<(), CoreError> {
    if !config.enabled {
        return Ok(());
    }
    let redactor = Redactor::new(config, "initial")?;
    *content = redactor.arguments(content);
    *context = redactor.arguments(context);
    if let Some(artifacts) = artifacts {
        redactor.structured(artifacts);
    }
    Ok(())
}

/// SQL passes the policy value from its own transaction. No connection access
/// or mutable global policy is allowed inside a SQLite callback.
pub(crate) fn register(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        "nexa_chat_privacy_policy_v1",
        2,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let raw = context.get::<String>(0)?;
            let revision = context.get::<String>(1)?;
            let mut policy: Value = serde_json::from_str(&raw)
                .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
            policy["_chatRevision"] = Value::String(revision);
            Ok(policy.to_string())
        },
    )?;
    type Cache = Option<(String, Arc<Redactor>)>;
    let cache = Mutex::<Cache>::new(None);
    conn.create_scalar_function(
        "nexa_chat_privacy_v1",
        4,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        move |context| {
            let policy = context.get::<Option<String>>(0)?;
            let kind = context.get::<String>(1)?;
            let raw = context.get::<Option<String>>(2)?;
            let envelope_artifacts = context.get::<Option<String>>(3)?;
            let Some(raw) = raw else {
                return Ok(None);
            };
            let Some(policy) = policy else {
                return Ok(Some(raw));
            };
            let redactor = {
                let mut cached = cache.lock().unwrap_or_else(|error| error.into_inner());
                if cached.as_ref().is_none_or(|(key, _)| key != &policy) {
                    let config: PrivacyConfig = serde_json::from_str(&policy)
                        .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                    if !config.enabled {
                        return Ok(Some(raw));
                    }
                    let value: Value = serde_json::from_str(&policy)
                        .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                    let revision = value
                        .get("_chatRevision")
                        .and_then(Value::as_str)
                        .unwrap_or("initial");
                    let redactor = Redactor::new(&config, revision)
                        .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                    *cached = Some((policy, Arc::new(redactor)));
                }
                cached.as_ref().expect("privacy policy cached").1.clone()
            };
            let projected = match kind.as_str() {
                "unique_title" => {
                    let safe = redactor.text(&raw);
                    if safe == raw {
                        raw
                    } else {
                        format!("{safe} [{}]", envelope_artifacts.unwrap_or_default())
                    }
                }
                "calls" => {
                    let mut calls: Vec<ToolCallRequest> = serde_json::from_str(&raw)
                        .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                    let envelope = envelope_artifacts
                        .as_deref()
                        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                        .and_then(|value| {
                            serde_json::from_value::<ProviderTurnEnvelope>(
                                value.get("providerTurnEnvelope")?.clone(),
                            )
                            .ok()
                        });
                    let current = envelope
                        .map(|mut envelope| {
                            redactor.envelope(&mut envelope);
                            envelope.capture_status != ReasoningCaptureStatus::Redacted
                        })
                        .unwrap_or(redactor.allow_legacy_replay);
                    redactor.calls(&mut calls, !current);
                    serde_json::to_string(&calls).expect("tool calls are serializable")
                }
                "json" | "event_payload" => {
                    // Old diagnostic traces can be malformed. Redact their raw
                    // text without interpreting it as a valid protocol object;
                    // keep the existing explicit detail-read error observable.
                    let Ok(mut value) = serde_json::from_str::<Value>(&raw) else {
                        return Ok(Some(redactor.text(&raw)));
                    };
                    redactor.structured(&mut value);
                    if kind == "event_payload" {
                        let scope = envelope_artifacts
                            .as_deref()
                            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
                        if scope.as_ref().is_some_and(|scope| {
                            scope["stale"] == 1
                                && matches!(
                                    scope["kind"].as_str(),
                                    Some("outputDelta" | "outputSnapshot" | "thinking")
                                )
                        }) {
                            revoke_output_fragment(&mut value);
                        }
                    }
                    value.to_string()
                }
                _ => redactor.text(&raw),
            };
            Ok(Some(projected))
        },
    )
}

fn revoke_output_fragment(value: &mut Value) {
    if let Some(map) = value.as_object_mut() {
        for key in ["delta", "text", "content"] {
            if map.contains_key(key) {
                map.insert(key.into(), Value::String(String::new()));
            }
        }
        if map.contains_key("offset") {
            map.insert("offset".into(), Value::from(0));
        }
    }
}
