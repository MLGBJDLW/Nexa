//! Browser schema and receipts shared by native and optional headless hosts.

use super::ToolResult;

pub(super) const MAX_WAIT_MS: u64 = 120_000;

/// Opening has already succeeded. A failed capture must not invite the model to
/// create a duplicate tab/session or replay navigation to a page with side effects.
pub fn browser_open_observation_failure(
    call_id: &str,
    action: &str,
    session_id: &str,
    tab_id: &str,
    error: impl std::fmt::Display,
) -> ToolResult {
    let artifacts = serde_json::json!({
        "kind": "browserOpenReceipt",
        "action": action,
        "sessionId": session_id,
        "tabId": tab_id,
        "opened": true,
        "observationPending": true,
        "retrySafe": false,
        "sideEffect": "may_have_occurred",
        "recovery": { "action": "observe", "sessionId": session_id, "tabId": tab_id },
    });
    ToolResult {
        call_id: call_id.into(),
        content: format!(
            "Browser tab was opened, but its initial observation failed: {error}. Do not repeat {action}; use browser_session observe with sessionId={session_id} and tabId={tab_id} to inspect the existing page.\n{artifacts}"
        ),
        is_error: true,
        artifacts: Some(artifacts),
    }
}

/// Bind a completed opening capture to the same observation used for input.
/// The workflow accepts this receipt only for create_session/open_tab, never
/// a plain mutation acknowledgement or an arbitrary screenshot attachment.
pub fn mark_browser_open_observation(result: ToolResult, action: &str) -> ToolResult {
    let mut output = result.output_channels();
    if !result.is_error && matches!(action, "create_session" | "open_tab") {
        if let (Some(data), Some(artifacts)) = (&output.data, &mut output.artifacts) {
            artifacts["openingObservation"] = serde_json::json!({
                "action": action,
                "sessionId": data.get("sessionId"),
                "tabId": data.get("tabId"),
                "observationId": data.get("observationId"),
            });
        }
    }
    ToolResult::from_output(result.call_id, result.is_error, output)
}

pub fn browser_session_action_schema_variants(
    actions: &[&str],
    session_optional_actions: &[&str],
) -> serde_json::Value {
    let optional_actions = actions
        .iter()
        .copied()
        .filter(|action| session_optional_actions.contains(action))
        .collect::<Vec<_>>();
    let session_required_actions = actions
        .iter()
        .copied()
        .filter(|action| !session_optional_actions.contains(action) && *action != "close_tab")
        .collect::<Vec<_>>();
    serde_json::json!([
        {
            "type": "object",
            "properties": { "action": { "enum": optional_actions } }
        },
        {
            "type": "object",
            "properties": { "action": { "enum": session_required_actions } },
            "required": ["sessionId"]
        },
        {
            "type": "object",
            "properties": { "action": { "enum": ["close_tab"] } },
            "required": ["sessionId", "tabId"]
        }
    ])
}

pub fn browser_session_parameters_schema() -> serde_json::Value {
    let actions = [
        "create_session",
        "list_sessions",
        "list_tabs",
        "open_tab",
        "activate_tab",
        "navigate",
        "observe",
        "click",
        "type",
        "select",
        "press",
        "scroll",
        "wait_for",
        "close_tab",
        "close_session",
    ];
    let action_variants =
        browser_session_action_schema_variants(&actions, &["create_session", "list_sessions"]);
    serde_json::json!({
        "type": "object",
        "properties": {
            "action": { "type": "string", "enum": actions },
            "sessionId": { "type": "string", "description": "Explicit session target. Required for close_session and close_tab so terminal receipts bind the exact requested target." },
            "tabId": { "type": "string", "description": "Explicit tab target. Required for close_tab so a final-tab receipt binds the exact requested target." },
            "url": { "type": "string", "description": "URL for navigate, create_session, or open_tab. Creating with a URL returns the initial page observation; use its returned sessionId, tabId and observationId directly." },
            "observationId": { "type": "string" },
            "targetRef": { "type": "string" },
            "query": { "type": "string", "maxLength": 240, "description": "observe only: case-insensitive substring filter on accessible name, role, or tag. Use this to locate controls omitted from a large observation; the result returns fresh refs and coverage." },
            "offset": { "type": "integer", "minimum": 0, "maximum": 9007199254740991_u64, "description": "observe only: start at this matching-control offset, using observationCoverage.nextOffset to continue. Keep the same query and page viewport while paging." },
            "text": { "type": "string" },
            "value": { "type": "string" },
            "key": { "type": "string" },
            "scrollX": { "type": "integer", "default": 0 },
            "scrollY": { "type": "integer", "default": 0 },
            "condition": { "type": "object", "description": "Condition type: page_loaded, text_present, url_matches, selector_visible, selector_hidden, or console_error." },
            "afterDiagnosticCursor": { "type": "integer", "minimum": 0, "description": "Return only console/network diagnostics newer than this cursor." },
            "timeoutMs": { "type": "integer", "minimum": 1, "maximum": MAX_WAIT_MS, "default": 15000 }
        },
        "required": ["action"],
        "oneOf": action_variants,
        "additionalProperties": false
    })
}
