//! Keep observation handles and complete control state usable within model context.
//! Display/audit output retains the original observation; this is only its model view.

use serde_json::{json, Map, Value};

pub(super) fn project_observation(tool_name: &str, content: &str, budget: usize) -> Option<String> {
    let browser = tool_name == "browser_session";
    if !browser && !matches!(tool_name, "computer_observe" | "computer_control") {
        return None;
    }
    if content.len() <= budget {
        return None;
    }
    let start = if content.trim_start().starts_with('{') {
        content.len() - content.trim_start().len()
    } else {
        content.find("\n{")? + 1
    };
    let prefix = &content[..start];
    let mut stream = serde_json::Deserializer::from_str(&content[start..]).into_iter::<Value>();
    let value = stream.next()?.ok()?;
    let mut root = value.as_object()?.clone();
    if !valid_handle(root.get("observationId")) || !root.get("elements")?.is_array() {
        return None;
    }
    if browser {
        if !valid_handle(root.get("sessionId")) || !valid_handle(root.get("tabId")) {
            return None;
        }
    } else if !root
        .get("window")?
        .get("id")
        .is_some_and(|id| id.is_number() || valid_handle(Some(id)))
    {
        return None;
    }

    let original_suffix = &content[start + stream.byte_offset()..];
    // Download/dialog receipts appended by the browser stay exact when bounded.
    // Oversized trailing details are omitted as a unit, never byte-sliced JSON.
    let suffix = if original_suffix.len() <= budget / 8 {
        original_suffix
    } else {
        ""
    };
    let mut elements = root.remove("elements")?.as_array()?.clone();
    let input_count = elements.len();
    root.remove("accessibilityTree");
    for element in &mut elements {
        if let Some(object) = element.as_object_mut() {
            object.remove("locatorFingerprint");
        }
    }
    if browser {
        compact_frame_limitations(&mut root);
    }
    if !browser {
        // Windows UIA also reports static text; leave room for actionable controls.
        elements.sort_by_key(|element| !is_interactive(element));
    }
    let mut omitted_fields = Vec::new();
    let text_truncated = if let Some(Value::String(text)) = root.get_mut("text") {
        if text.len() > 1_536 {
            *text = excerpt(text, 1_536);
            true
        } else {
            false
        }
    } else {
        false
    };
    if let Some(window) = root.get_mut("window").and_then(Value::as_object_mut) {
        for key in ["untrustedTitle", "appName"] {
            if window
                .get(key)
                .is_some_and(|value| value.to_string().len() > 1_024)
            {
                window.remove(key);
                omitted_fields.push(format!("window.{key}"));
            }
        }
    }
    root.insert("elements".into(), json!([]));
    // Bound non-action metadata first. Opaque handles, window identity, hashes,
    // coordinates, ownership and observation lifetime are never string-truncated.
    while serialized_len(&root).saturating_add(prefix.len() + suffix.len()) > budget / 3 {
        let largest = root
            .iter()
            .filter(|(key, _)| !protected_metadata(key))
            .max_by_key(|(_, value)| value.to_string().len())
            .map(|(key, _)| key.clone());
        let Some(key) = largest else {
            break;
        };
        root.remove(&key);
        omitted_fields.push(key);
    }
    root.insert("contextProjection".into(), json!({
        "inputElements": input_count,
        "returnedElements": 0,
        "omittedElements": input_count,
        "oversizedElements": 0,
        "oversizedElementOffsets": [],
        "omittedMetadataFields": omitted_fields,
        "textExcerpt": text_truncated,
        "trailingDetailsOmitted": suffix.len() != original_suffix.len(),
        "recovery": if browser {
            "Only complete controls are included. Use observe with the same query and observationCoverage.nextOffset for later controls; use a narrower query or the screenshot for omitted oversized controls. Do not infer omitted control values or trailing download/dialog outcomes."
        } else {
            "Only complete controls are included, with interactive controls first. This is not a complete accessibility tree and has no pagination. Use the screenshot and a fresh observation after narrowing the visible window/view; do not infer omitted control state."
        },
    }));
    let coverage = if browser {
        root.get("observationCoverage").cloned()
    } else {
        None
    };
    let base_offset = coverage
        .as_ref()
        .and_then(|value| value.get("offset"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let empty_size = render(prefix, &root, suffix).len();
    // Reserve room for the bounded omission offsets and pagination counters.
    let entry_budget = budget.saturating_sub(empty_size).saturating_sub(256);
    let mut consumed = 0usize;
    let mut entry_bytes = 0usize;
    let mut oversized = 0usize;
    let mut oversized_offsets = Vec::new();
    for (index, element) in elements.into_iter().enumerate() {
        let bytes = element.to_string().len();
        if bytes > entry_budget {
            consumed = index + 1;
            oversized += 1;
            if oversized_offsets.len() < 8 {
                oversized_offsets.push(base_offset.saturating_add(index as u64));
            }
            continue;
        }
        // Each control is serialized once for its exact escaped UTF-8 size.
        // Avoid repeatedly cloning/serializing an ever-growing observation.
        let separator = usize::from(entry_bytes > 0);
        if entry_bytes.saturating_add(bytes).saturating_add(separator) > entry_budget {
            break;
        }
        entry_bytes = entry_bytes.saturating_add(bytes).saturating_add(separator);
        root.get_mut("elements")?.as_array_mut()?.push(element);
        consumed = index + 1;
    }
    update_counts(
        &mut root,
        coverage.as_ref(),
        input_count,
        consumed,
        oversized,
        &oversized_offsets,
    );
    let projected = render(prefix, &root, suffix);
    if projected.len() <= budget {
        return Some(projected);
    }

    // A malformed or pathologically large identity/coordinate object must not
    // produce partial actionable state. Keep exact handles and explicitly withhold
    // all controls rather than sending broken JSON or invented coordinates.
    let mut unavailable = Map::new();
    for key in ["observationId", "sessionId", "tabId", "targetIdentity"] {
        if let Some(value) = root.get(key) {
            if valid_handle(Some(value)) {
                unavailable.insert(key.into(), value.clone());
            }
        }
    }
    if let Some(id) = root.get("window").and_then(|window| window.get("id")) {
        unavailable.insert("window".into(), json!({ "id": id }));
    }
    unavailable.insert("elements".into(), json!([]));
    unavailable.insert("contextProjection".into(), json!({
        "status":"observation_metadata_exceeds_context_budget", "inputElements":input_count,
        "omittedElements":input_count, "recovery":"Observation metadata is too large. No control state is provided. Obtain a fresh narrower observation or inspect the screenshot before acting."
    }));
    let projected = render(prefix, &unavailable, "");
    (projected.len() <= budget).then_some(projected)
}

fn valid_handle(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty() && id.len() <= 1_024)
}

fn protected_metadata(key: &str) -> bool {
    matches!(
        key,
        "observationId"
            | "sessionId"
            | "tabId"
            | "elements"
            | "observationCoverage"
            | "frameLimitations"
            | "actionReceipt"
            | "targetIdentity"
            | "windowId"
            | "consoleAndNetworkCursor"
            | "blockedRequests"
            | "window"
            | "viewport"
            | "contentHash"
            | "controlOwner"
            | "readyState"
            | "imageSize"
            | "imageWidth"
            | "imageHeight"
            | "nativeImageWidth"
            | "nativeImageHeight"
            | "captureTransform"
            | "coordinateSpace"
            | "coordinateSpaces"
            | "screenshotHash"
            | "semanticHash"
            | "stateFingerprint"
            | "captureMode"
            | "semanticStatus"
            | "semanticObservation"
            | "singleUseForControl"
            | "expiresInSeconds"
            | "trust"
            | "schemaVersion"
    )
}

fn compact_frame_limitations(root: &mut Map<String, Value>) {
    let Some(limitations) = root
        .get_mut("frameLimitations")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let Some(Value::Array(frames)) = limitations.remove("frames") else {
        return;
    };
    if frames.is_empty() && limitations.get("unavailableCount").and_then(Value::as_u64) == Some(0) {
        limitations.insert("frames".into(), Value::Array(frames));
        return;
    }
    let reasons = frames
        .iter()
        .filter_map(|frame| frame.get("reason").and_then(Value::as_str))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut kept = Vec::new();
    let mut bytes = 0usize;
    let mut omitted = limitations
        .get("detailsOmitted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    for frame in frames {
        let size = frame.to_string().len();
        if bytes.saturating_add(size) <= 3_000 {
            kept.push(frame);
            bytes = bytes.saturating_add(size);
        } else {
            omitted = true;
        }
    }
    limitations.insert("frames".into(), Value::Array(kept));
    limitations.insert("detailsOmitted".into(), json!(omitted));
    limitations.insert("reasons".into(), json!(reasons));
    limitations.insert("recovery".into(), json!("Controls in these frames are unavailable to this DOM observation. Repeating or narrowing observe cannot expose them; inspect the screenshot and use another supported surface only within existing user authorization, or ask the user to complete that step."));
}

fn is_interactive(element: &Value) -> bool {
    element.get("interactive").and_then(Value::as_bool) == Some(true)
        || element
            .get("actions")
            .and_then(Value::as_array)
            .is_some_and(|actions| !actions.is_empty())
        || matches!(
            element.get("role").and_then(Value::as_str),
            Some(
                "button"
                    | "checkbox"
                    | "combobox"
                    | "textbox"
                    | "link"
                    | "menuitem"
                    | "radio"
                    | "slider"
                    | "tab"
            )
        )
}

fn excerpt(text: &str, max_bytes: usize) -> String {
    let mut end = max_bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}… [context excerpt; remaining page text omitted]",
        &text[..end]
    )
}

fn serialized_len(root: &Map<String, Value>) -> usize {
    serde_json::to_string(root)
        .map(|value| value.len())
        .unwrap_or(usize::MAX)
}

fn render(prefix: &str, root: &Map<String, Value>, suffix: &str) -> String {
    format!("{prefix}{}{suffix}", Value::Object(root.clone()))
}

fn update_counts(
    root: &mut Map<String, Value>,
    coverage: Option<&Value>,
    input_count: usize,
    consumed: usize,
    oversized: usize,
    oversized_offsets: &[u64],
) {
    let returned = root
        .get("elements")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if let Some(projection) = root
        .get_mut("contextProjection")
        .and_then(Value::as_object_mut)
    {
        projection.insert("returnedElements".into(), json!(returned));
        projection.insert(
            "omittedElements".into(),
            json!(input_count.saturating_sub(returned)),
        );
        projection.insert("oversizedElements".into(), json!(oversized));
        projection.insert("oversizedElementOffsets".into(), json!(oversized_offsets));
    }
    if let Some(mut coverage) = coverage.cloned() {
        if let (Some(offset), Some(total)) = (
            coverage.get("offset").and_then(Value::as_u64),
            coverage.get("totalMatches").and_then(Value::as_u64),
        ) {
            let next = offset.saturating_add(consumed as u64);
            coverage["returned"] = json!(returned);
            coverage["hasMore"] = json!(next < total);
            coverage["nextOffset"] = if next < total {
                json!(next)
            } else {
                Value::Null
            };
            root.insert("observationCoverage".into(), coverage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_keeps_suffix_receipts_exact_and_does_not_intercept_error_results() {
        let value = json!({"observationId":"obs", "sessionId":"session", "tabId":"tab", "elements":[], "text":"x".repeat(30_000)});
        let suffix = "\nVerified download: {\"path\":\"C:/reports/final.pdf\",\"bytes\":125}";
        let prefix = "SECURITY NOTE: untrusted page data.\n\n";
        let source = format!("{prefix}{value}{suffix}");
        let output = project_observation("browser_session", &source, 24_000).unwrap();
        assert!(output.starts_with(prefix));
        assert!(output.ends_with(suffix));
        let json_text = output
            .strip_prefix(prefix)
            .unwrap()
            .strip_suffix(suffix)
            .unwrap();
        assert!(serde_json::from_str::<Value>(json_text).is_ok());
        assert!(project_observation("read_file", &source, 24_000).is_none());
        assert!(project_observation(
            "browser_session",
            &json!({"error":"closed", "details":"x".repeat(30_000)}).to_string(),
            24_000
        )
        .is_none());
        assert!(project_observation("computer_control", &json!({"observationId":"obs", "elements":[],"error":"unverified target", "details":"x".repeat(30_000)}).to_string(), 24_000).is_none());
    }

    #[test]
    fn projection_keeps_frame_capability_limit_when_details_exceed_budget() {
        let value = json!({
            "observationId":"obs", "sessionId":"session", "tabId":"tab", "elements":[], "text":"x".repeat(30_000),
            "frameLimitations":{"unavailableCount":2,"detailsOmitted":false,"frames":[
                {"reason":"cross_origin_or_sandboxed_frame","url":"https://example.org/frame","title":"t".repeat(30_000)},
                {"reason":"cross_origin_or_sandboxed_frame","url":"https://example.org/other","title":"Other"}
            ]}
        });
        let output = project_observation("browser_session", &value.to_string(), 24_000).unwrap();
        let projected: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(projected["frameLimitations"]["unavailableCount"], 2);
        assert_eq!(projected["frameLimitations"]["detailsOmitted"], true);
        assert_eq!(
            projected["frameLimitations"]["frames"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(projected["frameLimitations"]["recovery"]
            .as_str()
            .unwrap()
            .contains("cannot expose"));
    }

    #[test]
    fn projection_pages_at_the_first_whole_control_that_did_not_fit() {
        let elements = (0..200).map(|index| json!({
            "ref":format!("control-{index}"),"role":"button","name":"处理事项".repeat(50),
            "enabled":true,"visible":true,"bounds":{"x":10,"y":index*20,"width":100,"height":20}
        })).collect::<Vec<_>>();
        let value = json!({"observationId":"obs", "sessionId":"session", "tabId":"tab", "elements":elements,
            "observationCoverage":{"query":"处理", "offset":40,"returned":200,"totalMatches":400,"hasMore":true,"nextOffset":240}});
        let output = project_observation("browser_session", &value.to_string(), 24_000).unwrap();
        let projected: Value = serde_json::from_str(&output).unwrap();
        let returned = projected["elements"].as_array().unwrap().len();
        assert!((1..200).contains(&returned));
        assert!(output.len() <= 24_000);
        assert_eq!(
            projected["elements"][returned - 1]["ref"],
            format!("control-{}", returned - 1)
        );
        assert_eq!(projected["observationCoverage"]["returned"], returned);
        assert_eq!(
            projected["observationCoverage"]["nextOffset"],
            40 + returned
        );
        assert_eq!(projected["observationCoverage"]["totalMatches"], 400);
        assert_eq!(projected["observationCoverage"]["query"], "处理");
        assert_eq!(projected["contextProjection"]["oversizedElements"], 0);
    }

    #[test]
    fn oversized_required_metadata_is_explicitly_non_actionable_instead_of_partial_json() {
        let value = json!({"observationId":"obs", "sessionId":"session", "tabId":"tab", "elements":[{"ref":"e1"}],
            "viewport":{"badMetadata":"x".repeat(30_000)}});
        let output = project_observation("browser_session", &value.to_string(), 24_000).unwrap();
        let projected: Value = serde_json::from_str(&output).unwrap();
        assert!(output.len() <= 24_000);
        assert_eq!(projected["observationId"], "obs");
        assert_eq!(projected["sessionId"], "session");
        assert_eq!(projected["tabId"], "tab");
        assert_eq!(projected["elements"], json!([]));
        assert_eq!(
            projected["contextProjection"]["status"],
            "observation_metadata_exceeds_context_budget"
        );
    }

    #[test]
    fn standalone_browser_projection_keeps_session_binding_action_receipt_and_diagnostic_cursor() {
        // observe_tab produces this data; execute binds sessionId immediately
        // before constructing ToolOutput. Unlike the desktop backend it has no prefix.
        let mut captured = json!({
            "observationId":"obs_standalone", "tabId":"tab_standalone", "elements":[{"ref":"e1","index":0,"tag":"button","role":"button","name":"Submit","enabled":true,"visible":true,"bounds":[0,0,100,30]}],
            "text":"body".repeat(20_000),"consoleAndNetworkCursor":42,"blockedRequests":3,
            "diagnostics":[{"message":"x".repeat(20_000)}],
            "actionReceipt":{"callId":"standalone-action","action":"click","status":"observed_after_action","observationConsumed":true}
        });
        captured["sessionId"] = json!("session_standalone");
        let projected =
            project_observation("browser_session", &captured.to_string(), 24_000).unwrap();
        let value: Value = serde_json::from_str(&projected).unwrap();
        assert_eq!(value["sessionId"], "session_standalone");
        assert_eq!(value["actionReceipt"], captured["actionReceipt"]);
        assert_eq!(value["consoleAndNetworkCursor"], 42);
        assert_eq!(value["blockedRequests"], 3);
        assert_eq!(value["elements"][0]["ref"], "e1");
    }

    #[test]
    fn computer_projection_preserves_opaque_target_identity() {
        let observed = json!({"observationId":"obs", "window":{"id":77},"targetIdentity":"verified-process-generation-hash", "elements":[], "extraMetadata":"x".repeat(30_000)});
        let projected =
            project_observation("computer_observe", &observed.to_string(), 24_000).unwrap();
        let value: Value = serde_json::from_str(&projected).unwrap();
        assert_eq!(value["targetIdentity"], observed["targetIdentity"]);
    }

    #[test]
    fn frame_projection_preserves_native_absence_and_distinct_limitation_reasons() {
        for (limitations, expected_reasons) in [
            (
                json!({"unavailableCount":0,"detailsOmitted":false,"frames":[]}),
                None,
            ),
            (
                json!({"unavailableCount":1,"detailsOmitted":false,"frames":[{"reason":"frame_context_unavailable","url":"https://example.org/same-origin"}]}),
                Some(json!(["frame_context_unavailable"])),
            ),
        ] {
            let observed = json!({"observationId":"obs", "sessionId":"session", "tabId":"tab", "elements":[],"text":"x".repeat(30_000),"frameLimitations":limitations});
            let projected =
                project_observation("browser_session", &observed.to_string(), 24_000).unwrap();
            let value: Value = serde_json::from_str(&projected).unwrap();
            if let Some(reasons) = expected_reasons {
                assert_eq!(value["frameLimitations"]["reasons"], reasons);
                assert!(!projected.contains("cross_origin_or_sandboxed_frame"));
            } else {
                assert_eq!(value["frameLimitations"], observed["frameLimitations"]);
                assert!(value["frameLimitations"].get("recovery").is_none());
            }
        }
    }
}
