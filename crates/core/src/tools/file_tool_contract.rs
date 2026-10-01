//! Keep file reads and mutations distinct, including malformed model calls.
use serde_json::{json, Value};

use super::{structured_tool_error_result, ToolResult};

const OLD_FIELDS: &[&str] = &["old_str", "old_string"];
const NEW_FIELDS: &[&str] = &["new_str", "new_string", "content"];

pub(super) struct ArgumentIssue {
    pub code: &'static str,
    pub message: String,
    pub recovery: Value,
}

fn read_recovery(args: &Value) -> Value {
    let mut read_args = json!({ "path": args.get("path").cloned().unwrap_or(Value::Null) });
    if let Some(start) = args
        .get("start_line")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
    {
        read_args["start_line"] = json!(start);
        if let Some(end) = args
            .get("end_line")
            .and_then(Value::as_u64)
            .filter(|v| *v >= start)
        {
            read_args["max_lines"] = json!(end - start + 1);
        }
    }
    if let Some(max) = args
        .get("max_lines")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
    {
        read_args["max_lines"] = json!(max);
    }
    json!({"tool": "read_file", "arguments": read_args,
        "recovery": "Call read_file to inspect this file. No file operation was performed. Do not retry the same read request with edit_file."})
}

pub(super) fn argument_issue(name: &str, args: &Value) -> Option<ArgumentIssue> {
    let object = args.as_object()?;
    let has_old = OLD_FIELDS.iter().any(|key| object.contains_key(*key));
    let has_new = NEW_FIELDS.iter().any(|key| object.contains_key(*key));
    if name == "read_file" {
        if has_old
            || has_new
            || ["action", "command"]
                .iter()
                .any(|key| object.contains_key(*key))
        {
            return Some(ArgumentIssue {
                code: "file_tool_intent_mismatch",
                message: "read_file only reads files and accepts path, start_line, and max_lines. It cannot edit or create files. For changes, call edit_file with action='str_replace', old_str, and an explicit new_str (or create_file with content). No file operation was performed.".into(),
                recovery: json!({"tool": "read_file", "arguments": {"path": args.get("path")},
                    "recovery": "For inspection remove mutation fields; for a change call edit_file with explicit replacement text. Never report an edit based on a read result."}),
            });
        }
        return None;
    }
    if name != "edit_file" {
        return None;
    }
    let action = args.get("action").and_then(Value::as_str);
    let read_action = [action, args.get("command").and_then(Value::as_str)]
        .into_iter()
        .flatten()
        .any(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "read" | "view" | "read_file" | "open" | "cat" | "show" | "inspect"
            )
        });
    if read_action
        || object.contains_key("max_lines")
        || object.contains_key("view_range")
        || (action.is_none() && !has_old && !has_new)
    {
        return Some(ArgumentIssue {
            code: "file_tool_intent_mismatch",
            message: "edit_file is mutation-only; it has no read/view action. To inspect a file, call read_file with path and optional start_line/max_lines (or read_files for multiple paths). No file operation was performed.".into(),
            recovery: read_recovery(args),
        });
    }
    let error = if let Some(key) = object.keys().find(|key| {
        !matches!(
            key.as_str(),
            "path"
                | "action"
                | "old_str"
                | "old_string"
                | "new_str"
                | "new_string"
                | "content"
                | "start_line"
                | "end_line"
                | "wait_for_previous"
                | "waitForPrevious"
        )
    }) {
        Some(format!("Unsupported edit_file field '{key}'. Use action='str_replace' with old_str and new_str, or action='create' with new_str. Use read_file for inspection."))
    } else if action.is_some_and(|value| !matches!(value, "str_replace" | "replace" | "create")) {
        Some("Unknown action for edit_file. Use 'str_replace' (alias 'replace') or 'create'. Use read_file for reading, never action='read' or 'view'.".into())
    } else if [OLD_FIELDS, NEW_FIELDS].iter().any(|fields| {
        fields
            .iter()
            .filter(|key| object.contains_key(**key))
            .count()
            > 1
    }) {
        Some("Use only one spelling of old_str/old_string and new_str/new_string/content. Conflicting aliases are not combined or guessed.".into())
    } else if !NEW_FIELDS
        .iter()
        .any(|key| args.get(*key).is_some_and(Value::is_string))
    {
        Some("edit_file requires explicit new_str (aliases: new_string or content). Missing/null replacement content is not a deletion or empty file. To delete matched text, explicitly pass new_str=\"\". To inspect a file, use read_file.".into())
    } else if action == Some("create") && has_old {
        Some("action='create' cannot include old_str/old_string. Use action='str_replace' to change existing text.".into())
    } else if (has_old || matches!(action, Some("str_replace" | "replace")))
        && !OLD_FIELDS.iter().any(|key| {
            args.get(*key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        })
    {
        Some("str_replace requires a non-empty old_str (alias old_string) and an explicit new_str. Read the target text with read_file before editing.".into())
    } else {
        None
    };
    error.map(|message| ArgumentIssue {
        code: "invalid_file_mutation_arguments",
        message,
        recovery: json!({"tool": "edit_file", "arguments": {
            "path": args.get("path"), "action": "str_replace", "old_str": "exact existing text", "new_str": "replacement text"},
            "recovery": "Supply explicit mutation fields, or use read_file to inspect. No file operation was performed."}),
    })
}

pub(crate) fn argument_error(name: &str, call_id: &str, args: &Value) -> Option<ToolResult> {
    argument_issue(name, args).map(|issue| {
        structured_tool_error_result(call_id, issue.code, issue.message, issue.recovery, true)
    })
}
