use crate::app_settings::DreamingConfig;

/// Merge a caller-provided dreaming scope with the configured allowlist.
///
/// When a configured allowlist is present, requested source/project IDs are
/// constrained to the intersection. If the caller does not request IDs, the
/// configured allowlist becomes the effective scope. An empty intersection is
/// kept as an explicit empty list so a disallowed request cannot become
/// unrestricted by omission.
pub fn merge_configured_dream_scope(
    config: &DreamingConfig,
    base_scope: serde_json::Value,
) -> serde_json::Value {
    let mut scope = match base_scope {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let source_scope_present = scope.contains_key("sourceIds") || scope.contains_key("sourceId");
    let project_scope_present = scope.contains_key("projectIds") || scope.contains_key("projectId");
    let source_ids = constrained_scope_ids(&scope, "sourceIds", "sourceId", &config.source_ids);
    let project_ids = constrained_scope_ids(&scope, "projectIds", "projectId", &config.project_ids);
    // Canonical arrays are the only authority after intersection. Leaving an
    // unfiltered single-ID alias would let downstream readers re-add that ID.
    scope.remove("sourceId");
    scope.remove("projectId");

    scope.insert(
        "dreamingLocalOnly".to_string(),
        serde_json::json!(config.local_only),
    );
    scope.insert(
        "dreamingMaxRunsPerDay".to_string(),
        serde_json::json!(config.max_runs_per_day),
    );

    if source_scope_present || !config.source_ids.is_empty() {
        scope.insert("sourceIds".to_string(), serde_json::json!(source_ids));
    }
    if project_scope_present || !config.project_ids.is_empty() {
        scope.insert("projectIds".to_string(), serde_json::json!(project_ids));
    }

    serde_json::Value::Object(scope)
}

pub fn constrained_scope_ids(
    scope: &serde_json::Map<String, serde_json::Value>,
    array_key: &str,
    single_key: &str,
    configured_ids: &[String],
) -> Vec<String> {
    let requested = scope_ids(scope, array_key, single_key);
    if configured_ids.is_empty() {
        return requested;
    }
    if !scope.contains_key(array_key) && !scope.contains_key(single_key) {
        return configured_ids.to_vec();
    }
    requested
        .into_iter()
        .filter(|id| configured_ids.iter().any(|configured| configured == id))
        .collect()
}

fn scope_ids(
    scope: &serde_json::Map<String, serde_json::Value>,
    array_key: &str,
    single_key: &str,
) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(single) = scope.get(single_key).and_then(serde_json::Value::as_str) {
        let trimmed = single.trim();
        if !trimmed.is_empty() {
            ids.push(trimmed.to_string());
        }
    }
    if let Some(items) = scope.get(array_key).and_then(serde_json::Value::as_array) {
        for item in items {
            let Some(raw) = item.as_str() else {
                continue;
            };
            let trimmed = raw.trim();
            if trimmed.is_empty() || ids.iter().any(|id| id == trimmed) {
                continue;
            }
            ids.push(trimmed.to_string());
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_allowlist_is_applied_when_request_is_unscoped() {
        let config = DreamingConfig {
            source_ids: vec!["source-a".to_string()],
            project_ids: vec!["project-a".to_string()],
            ..DreamingConfig::default()
        };

        let scope = merge_configured_dream_scope(&config, serde_json::json!({ "surface": "test" }));

        assert_eq!(scope["sourceIds"], serde_json::json!(["source-a"]));
        assert_eq!(scope["projectIds"], serde_json::json!(["project-a"]));
    }

    #[test]
    fn disallowed_requested_ids_remain_explicitly_empty() {
        let config = DreamingConfig {
            source_ids: vec!["allowed".to_string()],
            project_ids: vec!["allowed-project".to_string()],
            ..DreamingConfig::default()
        };

        for requested in [
            serde_json::json!({ "sourceIds": ["blocked"], "projectIds": ["blocked-project"] }),
            serde_json::json!({ "sourceId": "blocked", "projectId": "blocked-project" }),
            serde_json::json!({ "sourceIds": [], "projectIds": [] }),
        ] {
            let scope = merge_configured_dream_scope(&config, requested);
            assert_eq!(scope["sourceIds"], serde_json::json!([]));
            assert_eq!(scope["projectIds"], serde_json::json!([]));
            assert!(scope.get("sourceId").is_none());
            assert!(scope.get("projectId").is_none());
            assert_eq!(
                merge_configured_dream_scope(&config, scope.clone()),
                scope,
                "a canonical empty intersection must not expand on a second merge"
            );
        }
    }
}
