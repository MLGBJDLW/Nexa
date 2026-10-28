//! One model-facing control vocabulary over the existing registered adapters.
//!
//! This module only projects definitions and routes arguments. The registry
//! still owns admission, permissions and execution; no handler is called here.

use std::borrow::Cow;
use std::collections::BTreeSet;

use serde_json::{json, Map, Value};

use crate::llm::{ToolCallRequest, ToolDefinition};

pub const TOOL_NAME: &str = "computer_use";
pub const BACKEND_NAMES: &[&str] = &["browser_session", "computer_observe", "computer_control"];

#[derive(Debug, Clone, PartialEq)]
pub struct RoutedControlCall {
    pub tool_name: &'static str,
    pub arguments: Value,
}

pub fn is_backend(name: &str) -> bool {
    BACKEND_NAMES.contains(&name)
}

pub fn model_name(name: &str) -> &str {
    if is_backend(name) {
        TOOL_NAME
    } else {
        name
    }
}

/// Expand only against the adapters already present in the caller's scope.
/// This is an intersection, never a request to register or enable an adapter.
pub fn expand_allowed_names(requested: &[String], available: &[String]) -> Vec<String> {
    let mut expanded = Vec::new();
    for name in requested {
        for candidate in available {
            if (name == candidate || (name == TOOL_NAME && is_backend(candidate)))
                && !expanded.contains(candidate)
            {
                expanded.push(candidate.clone());
            }
        }
    }
    expanded
}

fn public_action(backend: &str, action: &str) -> String {
    match (backend, action) {
        ("computer_observe", "capture_window") => "observe",
        ("computer_control", "move_mouse") => "move",
        ("computer_control", "type_text") => "type",
        ("computer_control", "key") => "press",
        _ => action,
    }
    .to_string()
}

fn snake_name(name: &str) -> String {
    super::camel_to_snake_case(name)
}

fn canonical_public_value(value: &Value) -> Result<Value, String> {
    match value {
        Value::Object(object) => {
            let mut canonical = Map::new();
            for (key, value) in object {
                let key = snake_name(key);
                if canonical
                    .insert(key.clone(), canonical_public_value(value)?)
                    .is_some()
                {
                    return Err(
                        "Conflicting spellings of the same argument are not allowed.".into(),
                    );
                }
            }
            Ok(Value::Object(canonical))
        }
        Value::Array(values) => values
            .iter()
            .map(canonical_public_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        _ => Ok(value.clone()),
    }
}

fn browser_value(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    // The scheduler field is shared by every backend definition.
                    let key = if key == "wait_for_previous" {
                        key
                    } else {
                        super::snake_to_camel_case(&key)
                    };
                    (key, browser_value(value))
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(browser_value).collect()),
        value => value,
    }
}

/// Route a public request without granting authority or retrying another host.
/// Legacy backend calls are deliberately left untouched for replay compatibility.
pub fn route_call(name: &str, args: &Value) -> Result<Option<RoutedControlCall>, String> {
    if name != TOOL_NAME {
        return Ok(None);
    }
    let canonical = canonical_public_value(args)?;
    let mut body = canonical
        .as_object()
        .cloned()
        .ok_or_else(|| "computer_use arguments must be one object.".to_string())?;
    let action = body
        .get("action")
        .and_then(Value::as_str)
        .filter(|action| !action.is_empty())
        .ok_or_else(|| "computer_use requires an action.".to_string())?
        .to_ascii_lowercase();
    let mut target = body
        .remove("target")
        .and_then(|target| target.as_object().cloned())
        .ok_or_else(|| {
            "computer_use requires target.kind=browser or target.kind=window.".to_string()
        })?;
    let kind = target
        .remove("kind")
        .and_then(|kind| kind.as_str().map(str::to_ascii_lowercase))
        .ok_or_else(|| "computer_use target.kind must be browser or window.".to_string())?;
    if ["session_id", "tab_id", "window_id"]
        .iter()
        .any(|field| body.contains_key(*field))
    {
        return Err("Put the surface identity only inside target; do not mix target and top-level identities.".into());
    }
    if let Some(token) = body.get("observation_id") {
        if token.as_str().is_none_or(|token| token.trim().is_empty()) {
            return Err("observation_id must be a nonempty observation token.".into());
        }
    }
    let tool_name = match kind.as_str() {
        "browser" => {
            if target
                .keys()
                .any(|key| !matches!(key.as_str(), "session_id" | "tab_id"))
            {
                return Err("A browser target accepts only session_id and tab_id; window targets cannot be mixed into it.".into());
            }
            for (key, value) in target {
                if value.as_str().is_none_or(|value| value.trim().is_empty()) {
                    return Err(format!("target.{key} must be a nonempty identifier."));
                }
                body.insert(key, value);
            }
            if matches!(action.as_str(), "close_session" | "close_tab")
                && !body.contains_key("session_id")
            {
                return Err(
                    "Closing requires target.session_id for the exact browser session.".into(),
                );
            }
            if action == "close_tab" && !body.contains_key("tab_id") {
                return Err(
                    "Closing a tab requires target.tab_id for the exact browser tab.".into(),
                );
            }
            body.insert("action".into(), Value::String(action));
            return Ok(Some(RoutedControlCall {
                tool_name: "browser_session",
                arguments: browser_value(Value::Object(body)),
            }));
        }
        "window" => {
            if target.keys().any(|key| key != "window_id") {
                return Err("A window target accepts only window_id; browser identities cannot be mixed into it.".into());
            }
            if let Some(window) = target.remove("window_id") {
                let id = window
                    .as_u64()
                    .or_else(|| window.as_str().and_then(|text| text.parse().ok()))
                    .filter(|id| *id > 0)
                    .ok_or_else(|| {
                        "target.window_id must be a positive window identifier.".to_string()
                    })?;
                body.insert("window_id".into(), Value::from(id));
            }
            let (backend, action) = match action.as_str() {
                "observe" => ("computer_observe", "capture_window"),
                "list_windows" | "wait_for_window" | "wait_for_change" | "cursor_position" => ("computer_observe", action.as_str()),
                "type" => ("computer_control", "type_text"),
                "press" => ("computer_control", "key"),
                "move" => ("computer_control", "move_mouse"),
                "focus_window" | "click" | "drag" | "scroll" | "invoke" | "set_value" => ("computer_control", action.as_str()),
                _ => return Err("Unsupported action for target.kind=window. Choose an advertised window operation.".into()),
            };
            body.insert("action".into(), Value::String(action.to_string()));
            backend
        }
        _ => return Err(
            "target.kind must be browser or window; routing never falls back to another surface."
                .into(),
        ),
    };
    Ok(Some(RoutedControlCall {
        tool_name,
        arguments: Value::Object(body),
    }))
}

/// Parse a complete request without relaxing the execution argument contract.
pub fn route_call_str(name: &str, args: Option<&str>) -> Option<RoutedControlCall> {
    if name != TOOL_NAME {
        return None;
    }
    let value = super::parse_tool_arguments_value(args?).ok()?;
    route_call(name, &value).ok().flatten()
}

/// Resolve only receipt identity from a public call's durable, audit-safe args.
/// Input text and tokens may have been replaced by redaction objects. They are
/// neither execution arguments nor authority; host receipts retain consumed
/// tokens and process identities. Keep a surviving string token for old traces.
/// Execution and capability admission must continue to use `route_call`.
pub fn route_receipt_str(name: &str, args: Option<&str>) -> Option<RoutedControlCall> {
    if name != TOOL_NAME {
        return None;
    }
    let value = super::parse_tool_arguments_value(args?).ok()?;
    let canonical = canonical_public_value(&value).ok()?;
    let body = canonical.as_object()?;
    let mut identity = Map::new();
    // Retain forbidden top-level identities so malformed mixed targets cannot
    // become valid merely by projecting away the conflicting fields.
    for key in ["action", "target", "session_id", "tab_id", "window_id"] {
        if let Some(value) = body.get(key) {
            identity.insert(key.into(), value.clone());
        }
    }
    if let Some(token) = body
        .get("observation_id")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty() && *token != "<observation-token-redacted>")
    {
        identity.insert("observation_id".into(), json!(token));
    }
    route_call(name, &Value::Object(identity)).ok().flatten()
}

/// Preserve the original model call/call ID; provide resolved facts to receipt
/// consumers that already operate on the existing backend vocabulary.
pub fn effective_request(call: &ToolCallRequest) -> Cow<'_, ToolCallRequest> {
    let Some(routed) = route_call_str(&call.name, Some(&call.arguments)) else {
        return Cow::Borrowed(call);
    };
    let mut resolved = call.clone();
    resolved.name = routed.tool_name.to_string();
    resolved.arguments = routed.arguments.to_string();
    Cow::Owned(resolved)
}

fn public_property_schema(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let value = match key.as_str() {
                        "properties" => value
                            .as_object()
                            .map(|properties| {
                                Value::Object(
                                    properties
                                        .iter()
                                        .map(|(name, schema)| {
                                            (snake_name(name), public_property_schema(schema))
                                        })
                                        .collect(),
                                )
                            })
                            .unwrap_or_else(|| value.clone()),
                        "required" => value
                            .as_array()
                            .map(|fields| {
                                Value::Array(
                                    fields
                                        .iter()
                                        .map(|field| {
                                            field
                                                .as_str()
                                                .map(|field| Value::String(snake_name(field)))
                                                .unwrap_or_else(|| field.clone())
                                        })
                                        .collect(),
                                )
                            })
                            .unwrap_or_else(|| value.clone()),
                        "description" => value
                            .as_str()
                            .map(|text| {
                                Value::String(
                                    text.replace("browser_session", TOOL_NAME)
                                        .replace("computer_observe", TOOL_NAME)
                                        .replace("computer_control", TOOL_NAME)
                                        .replace("capture_window", "observe")
                                        .replace("type_text", "type")
                                        .replace("move_mouse", "move")
                                        .replace("Required for key.", "Required for press."),
                                )
                            })
                            .unwrap_or_else(|| value.clone()),
                        _ => public_property_schema(value),
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(public_property_schema).collect()),
        _ => value.clone(),
    }
}

fn merge_property(properties: &mut Map<String, Value>, name: String, schema: Value) {
    match properties.get_mut(&name) {
        None => {
            properties.insert(name, schema);
        }
        Some(existing) if *existing == schema => {}
        Some(existing) => {
            if let Some(variants) = existing.get_mut("anyOf").and_then(Value::as_array_mut) {
                if !variants.contains(&schema) {
                    variants.push(schema);
                }
            } else {
                *existing = json!({"anyOf":[existing.clone(),schema]});
            }
        }
    }
}

/// Build from registered, already-scoped backend definitions. Disabled adapters
/// contribute neither actions nor parameter shapes to the public interface.
pub(crate) fn definition(backends: &[ToolDefinition]) -> Option<ToolDefinition> {
    if backends.is_empty() {
        return None;
    }
    let mut properties = Map::new();
    let mut targets = Map::new();
    let mut kinds = BTreeSet::new();
    let mut actions = BTreeSet::new();
    let mut variants = Vec::new();
    for backend in backends.iter().filter(|backend| is_backend(&backend.name)) {
        let kind = if backend.name == "browser_session" {
            "browser"
        } else {
            "window"
        };
        let schema = &backend.parameters;
        let Some(backend_actions) = schema
            .pointer("/properties/action/enum")
            .and_then(Value::as_array)
        else {
            continue;
        };
        let Some(backend_properties) = schema.get("properties").and_then(Value::as_object) else {
            continue;
        };
        let backend_actions = backend_actions
            .iter()
            .filter_map(Value::as_str)
            .map(|action| public_action(&backend.name, action))
            .collect::<Vec<_>>();
        kinds.insert(kind);
        actions.extend(backend_actions.iter().cloned());
        let mut target_properties = Map::new();
        target_properties.insert("kind".into(), json!({"type":"string","const":kind}));
        let mut target_required = vec![Value::String("kind".into())];
        let mut required = vec![
            Value::String("action".into()),
            Value::String("target".into()),
        ];
        let backend_required = schema
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (name, property) in backend_properties {
            if name == "action" {
                continue;
            }
            let public = snake_name(name);
            let property = public_property_schema(property);
            if matches!(public.as_str(), "session_id" | "tab_id" | "window_id") {
                target_properties.insert(public.clone(), property.clone());
                merge_property(&mut targets, public.clone(), property);
                if backend_required
                    .iter()
                    .any(|field| field.as_str() == Some(name))
                {
                    target_required.push(Value::String(public));
                }
            } else {
                merge_property(&mut properties, public.clone(), property);
                if backend_required
                    .iter()
                    .any(|field| field.as_str() == Some(name))
                {
                    required.push(Value::String(public));
                }
            }
        }
        variants.push(json!({"type":"object","properties":{
            "action":{"enum":backend_actions},
            "target":{"type":"object","properties":target_properties,"required":target_required,"additionalProperties":false}
        },"required":required}));
    }
    if kinds.is_empty() {
        return None;
    }
    let mut description = String::from("Observe and operate the explicitly selected surface. ");
    if kinds.contains("browser") {
        description.push_str("Browser pages use target.kind=browser with session_id/tab_id and fresh DOM target_ref evidence. Recover tabs with list_sessions/list_tabs; lifecycle and navigation actions remain in the schema. ");
    }
    if kinds.contains("window") {
        description.push_str("Native windows use target.kind=window with window_id and fresh UIA element_id or captured-image coordinates. ");
        if actions.contains("list_windows") {
            description.push_str("Discover windows with list_windows, then observe. ");
        }
    }
    let shared = ["observe", "click", "type", "scroll", "press", "move"]
        .into_iter()
        .filter(|action| actions.contains(*action))
        .collect::<Vec<_>>()
        .join(", ");
    description.push_str(&format!("Available shared actions: {shared}. Keep observation_id from the same target's latest observation. Input delivery or visible change alone does not prove the user goal. Never switch target kind or replay uncertain input automatically. Only currently authorized targets/actions appear in this schema."));
    targets.insert("kind".into(), json!({"type":"string","enum":kinds}));
    properties.insert("action".into(), json!({"type":"string","enum":actions,
        "description":"Choose one operation for the explicit target kind. Browser scrolling uses CSS pixels; window scrolling uses wheel steps. Browser refs and window UIA element IDs belong only to their own fresh observation."}));
    properties.insert("target".into(), json!({"type":"object","properties":targets,"required":["kind"],"additionalProperties":false}));
    Some(ToolDefinition {
        name: TOOL_NAME.into(),
        description,
        parameters: json!({"type":"object","properties":properties,"required":["action","target"],"oneOf":variants,"additionalProperties":false}),
    })
}

pub(crate) fn collapse_definitions(definitions: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
    let (backends, mut other): (Vec<_>, Vec<_>) = definitions
        .into_iter()
        .partition(|definition| is_backend(&definition.name));
    if let Some(definition) = definition(&backends) {
        other.push(definition);
    }
    other
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::error::CoreError;
    use crate::tools::{Tool, ToolCategory, ToolExecutionContext, ToolRegistry, ToolResult};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    type RecordedCalls = Arc<Mutex<Vec<(String, Value)>>>;

    struct RecordingBackend {
        name: &'static str,
        calls: RecordedCalls,
    }

    #[async_trait]
    impl Tool for RecordingBackend {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "Control adapter fixture"
        }
        fn parameters_schema(&self) -> Value {
            match self.name {
                "computer_observe" => {
                    crate::tools::computer_use_tool::ComputerObserveTool.parameters_schema()
                }
                "computer_control" => {
                    crate::tools::computer_use_tool::ComputerControlTool.parameters_schema()
                }
                _ => crate::tools::browser_session_tool::browser_session_parameters_schema(),
            }
        }
        fn categories(&self) -> &'static [ToolCategory] {
            if self.name == "browser_session" {
                &[ToolCategory::BrowserInteract]
            } else {
                &[ToolCategory::DesktopInteract]
            }
        }
        fn requires_confirmation(&self, args: &Value) -> bool {
            self.name == "computer_control"
                || (self.name == "browser_session"
                    && !matches!(
                        args.get("action").and_then(Value::as_str),
                        Some("list_sessions" | "list_tabs" | "observe")
                    ))
        }
        async fn execute(
            &self,
            context: ToolExecutionContext<'_>,
        ) -> Result<ToolResult, CoreError> {
            let args: Value = serde_json::from_str(context.arguments)?;
            if args["action"] == "wait_for" {
                return Err(CoreError::InvalidInput(
                    "fixture condition requires correction".into(),
                ));
            }
            self.calls
                .lock()
                .unwrap()
                .push((self.name.into(), args.clone()));
            Ok(ToolResult {
                call_id: context.call_id.into(),
                content: args.to_string(),
                is_error: false,
                artifacts: None,
            })
        }
    }

    fn registry() -> (ToolRegistry, RecordedCalls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut registry = ToolRegistry::new();
        for name in BACKEND_NAMES {
            registry.register(Box::new(RecordingBackend {
                name,
                calls: Arc::clone(&calls),
            }));
        }
        (registry, calls)
    }

    #[test]
    fn one_model_definition_retains_exact_backend_authority_and_conditional_schema() {
        let (registry, _) = registry();
        assert_eq!(registry.model_tool_names(), [TOOL_NAME]);
        assert_eq!(
            registry
                .definitions()
                .iter()
                .map(|definition| definition.name.as_str())
                .collect::<Vec<_>>(),
            [TOOL_NAME]
        );
        assert_eq!(registry.tool_names().len(), 3);
        assert!(registry.contains(TOOL_NAME));
        assert!(
            registry.get(TOOL_NAME).is_none(),
            "there is no duplicate execution handler"
        );
        let definition = registry.definition(TOOL_NAME).unwrap();
        assert_eq!(
            definition.parameters["properties"]["target"]["properties"]["kind"]["enum"],
            json!(["browser", "window"])
        );
        assert!(definition.parameters["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .any(|variant| {
                variant["properties"]["target"]["properties"]["kind"]["const"] == "window"
                    && variant["required"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("observation_id"))
            }));
        let serialized = definition.parameters.to_string();
        assert!(
            !BACKEND_NAMES.iter().any(|name| serialized.contains(name)),
            "public guidance must not revive old tool names"
        );
        let tokens = crate::conversation::memory::estimate_tokens_for_model(
            "gpt-4o",
            &serde_json::to_string(&definition).unwrap(),
        );
        assert!(
            tokens < 12_000,
            "one control schema must remain within the existing stable tool target: {tokens}"
        );
        println!(
            "unified_control_schema_bytes={} estimated_tokens={tokens}",
            serialized.len()
        );
    }

    #[tokio::test]
    async fn readonly_scope_and_plan_mode_cannot_reenable_mutation_through_unified_name() {
        let (registry, calls) = registry();
        let readonly = registry.filtered(&["computer_observe".into()]);
        let advertised = readonly.definition(TOOL_NAME).unwrap();
        let actions = advertised.parameters["properties"]["action"]["enum"]
            .as_array()
            .unwrap();
        assert!(actions.contains(&json!("observe")) && !actions.contains(&json!("click")));
        assert_eq!(
            advertised.parameters["properties"]["target"]["properties"]["kind"]["enum"],
            json!(["window"])
        );
        let explicit = readonly.filtered(&[TOOL_NAME.into()]);
        assert_eq!(explicit.tool_names(), ["computer_observe"]);
        assert!(!explicit.contains("computer_control"));
        let planned = registry.plan_mode_filtered();
        assert!(planned.contains("computer_observe"));
        assert!(!planned.contains("computer_control") && !planned.contains("browser_session"));
        let db = Database::open_memory().unwrap();
        let args = json!({"action":"click","target":{"kind":"window","window_id":42},"observation_id":"fresh","x":1,"y":2}).to_string();
        let rejected = explicit
            .execute(
                TOOL_NAME,
                ToolExecutionContext::new("blocked", &args, &db, &[]),
            )
            .await
            .unwrap();
        assert!(rejected.is_error);
        assert_eq!(rejected.artifacts.unwrap()["sideEffect"], "not_started");
        assert!(calls.lock().unwrap().is_empty());
        assert!(!registry
            .without_names(&[TOOL_NAME])
            .contains("computer_observe"));
    }

    #[tokio::test]
    async fn canonical_execution_preserves_observation_target_call_identity_and_legacy_aliases() {
        let (registry, calls) = registry();
        let db = Database::open_memory().unwrap();
        for (public, backend) in [
            ("observe", "capture_window"),
            ("click", "click"),
            ("type", "type_text"),
            ("press", "key"),
            ("move", "move_mouse"),
            ("scroll", "scroll"),
        ] {
            let args = json!({"action":public,"target":{"kind":"window","window_id":42},"observation_id":"fresh"}).to_string();
            let result = registry
                .execute(
                    TOOL_NAME,
                    ToolExecutionContext::new(public, &args, &db, &[]),
                )
                .await
                .unwrap();
            assert!(!result.is_error, "{public}: {}", result.content);
            assert_eq!(result.call_id, public);
            let actual: Value = serde_json::from_str(&result.content).unwrap();
            assert_eq!(actual["action"], backend);
            assert_eq!(actual["window_id"], 42);
            assert_eq!(actual["observation_id"], "fresh");
            assert!(actual.get("target").is_none());
        }
        let browser = json!({"action":"click","target":{"kind":"browser","session_id":"session","tab_id":"tab"},"observation_id":"page-fresh","target_ref":"e_2"}).to_string();
        let result = registry
            .execute(
                TOOL_NAME,
                ToolExecutionContext::new("browser", &browser, &db, &[]),
            )
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.content);
        let actual: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(actual["sessionId"], "session");
        assert_eq!(actual["tabId"], "tab");
        assert_eq!(actual["observationId"], "page-fresh");
        assert_eq!(actual["targetRef"], "e_2");
        let legacy = r#"{"action":"list_windows"}"#;
        let old = registry
            .execute(
                "computer_observe",
                ToolExecutionContext::new("legacy", legacy, &db, &[]),
            )
            .await
            .unwrap();
        assert!(!old.is_error);
        assert_eq!(calls.lock().unwrap().len(), 8);
    }

    #[tokio::test]
    async fn mixed_targets_never_dispatch_or_fall_back_to_another_backend() {
        let (registry, calls) = registry();
        let db = Database::open_memory().unwrap();
        for args in [
            json!({"action":"observe","target":{"kind":"browser","session_id":"session","window_id":42}}),
            json!({"action":"observe","target":{"kind":"window","window_id":42,"tab_id":"tab"}}),
            json!({"action":"observe","target":{"kind":"window","window_id":42},"session_id":"session"}),
            json!({"action":"navigate","target":{"kind":"window","window_id":42},"url":"https://example.test"}),
            json!({"action":"close_tab","target":{"kind":"browser","session_id":"session"}}),
        ] {
            let rejected = registry
                .execute(
                    TOOL_NAME,
                    ToolExecutionContext::new("mixed", &args.to_string(), &db, &[]),
                )
                .await
                .unwrap();
            assert!(rejected.is_error, "{args}");
            let artifacts = rejected.artifacts.unwrap();
            assert_eq!(artifacts["sideEffect"], "not_started");
            assert_eq!(artifacts["observationConsumed"], false);
        }
        assert!(calls.lock().unwrap().is_empty());
        let error = route_call(
            TOOL_NAME,
            &json!({"action":"PRIVATE_ACTION_VALUE","target":{"kind":"window"}}),
        )
        .unwrap_err();
        assert!(!error.contains("PRIVATE_ACTION_VALUE"));
    }

    #[tokio::test]
    async fn public_error_repair_uses_the_public_target_shape_while_legacy_stays_compatible() {
        let (registry, _) = registry();
        let db = Database::open_memory().unwrap();
        for args in [
            json!({"action":"wait_for","target":{"kind":"browser","session_id":"session"},"condition":{"type":"page_loaded"}}),
            json!({"action":"click","target":{"kind":"window","window_id":42},"observation_id":"fresh","unexpected":true}),
        ] {
            let error = registry
                .execute(
                    TOOL_NAME,
                    ToolExecutionContext::new("repair", &args.to_string(), &db, &[]),
                )
                .await
                .unwrap();
            assert!(error.is_error);
            let expected = &error.artifacts.as_ref().unwrap()["expectedFormat"];
            assert_eq!(expected["tool"], TOOL_NAME);
            assert!(expected["arguments"]["properties"].get("target").is_some());
            assert!(expected["arguments"]["properties"]
                .get("window_id")
                .is_none());
        }
        let legacy = registry.execute("browser_session",ToolExecutionContext::new("legacy",r#"{"action":"wait_for","sessionId":"session","condition":{"type":"page_loaded"}}"#,&db,&[])).await.unwrap();
        assert!(legacy.is_error);
        assert_eq!(
            legacy.artifacts.unwrap()["expectedFormat"]["tool"],
            "browser_session"
        );
    }

    #[test]
    fn receipt_projection_does_not_rewrite_original_or_legacy_calls() {
        let original = ToolCallRequest { id:"same-id".into(), name:TOOL_NAME.into(), arguments:json!({"action":"observe","target":{"kind":"window","windowId":"42"},"observationId":"fresh"}).to_string(), thought_signature:Some("provider-proof".into()) };
        let resolved = effective_request(&original);
        assert_eq!(resolved.name, "computer_observe");
        assert_eq!(resolved.id, original.id);
        assert_eq!(resolved.thought_signature, original.thought_signature);
        assert_eq!(original.name, TOOL_NAME);
        assert_eq!(
            serde_json::from_str::<Value>(&resolved.arguments).unwrap()["window_id"],
            42
        );
        let legacy = ToolCallRequest {
            name: "computer_observe".into(),
            ..original.clone()
        };
        assert!(matches!(effective_request(&legacy), Cow::Borrowed(_)));
        assert!(route_call_str(TOOL_NAME, Some("not JSON")).is_none());
    }

    #[test]
    fn durable_receipts_resolve_redacted_inputs_without_relaxing_live_calls() {
        for (args, backend, action) in [
            (
                json!({"action":"type","target":{"kind":"window","window_id":42},"observation_id":"native-before","text":"private text"}),
                "computer_control",
                "type_text",
            ),
            (
                json!({"action":"observe","target":{"kind":"window","window_id":42},"observation_id":"native-before"}),
                "computer_observe",
                "capture_window",
            ),
            (
                json!({"action":"type","target":{"kind":"browser","session_id":"session","tab_id":"tab"},"observation_id":"browser-before","text":"private text"}),
                "browser_session",
                "type",
            ),
        ] {
            let audit = crate::tool_argument_projection::audit_safe_arguments(TOOL_NAME, &args);
            assert!(!audit.to_string().contains("private text"));
            let route = route_receipt_str(TOOL_NAME, Some(&audit.to_string())).unwrap();
            assert_eq!(route.tool_name, backend);
            assert_eq!(route.arguments["action"], action);
            assert!(route.arguments.get("text").is_none());
            assert!(route.arguments.get("observation_id").is_none());
            if backend == "browser_session" {
                assert_eq!(route.arguments["sessionId"], "session");
                assert_eq!(route.arguments["tabId"], "tab");
                assert_eq!(route.arguments["observationId"], "browser-before");
            } else {
                if backend == "computer_observe" {
                    assert!(route_call(TOOL_NAME, &audit).is_err());
                }
                assert!(route.arguments.get("observationId").is_none());
                assert_eq!(route.arguments["window_id"], 42);
            }
            let live = route_call(TOOL_NAME, &args).unwrap().unwrap();
            let token_key = if backend == "browser_session" {
                "observationId"
            } else {
                "observation_id"
            };
            assert!(live.arguments[token_key].is_string());
        }
        for args in [
            json!({"action":"type","target":{"kind":"window","window_id":42},"window_id":99}),
            json!({"action":"type","target":{"kind":"window","window_id":42,"tab_id":"tab"}}),
            json!({"action":"type","target":{"kind":"window","window_id":42,"windowId":99}}),
            json!({"action":"close_tab","target":{"kind":"browser","session_id":"session"}}),
        ] {
            assert!(route_receipt_str(TOOL_NAME, Some(&args.to_string())).is_none());
        }
    }
}
