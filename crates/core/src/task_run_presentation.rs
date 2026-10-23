//! Small task projections for list, timeline, and live presentation.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::conversation::{AgentTaskArtifactSummary, AgentTaskRun};

pub(crate) const ARTIFACT_SECTION_KEYS: &[&str] = &[
    "files",
    "fileCheckpoints",
    "judgement",
    "plan",
    "report",
    "subtasks",
    "table",
    "trace",
    "verification",
];

/// Run-independent card fields computed when authoritative JSON changes.
/// `output` is the whole-value summary used by a delegated worker's output
/// card; task-run catalogs exclude it and use root/section entries instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ArtifactCatalogEntry {
    pub key: String,
    pub kind: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub paths: Vec<String>,
}

pub(crate) fn project_catalog(value: &Value) -> Value {
    use crate::conversation::{
        artifact_kind_for_key, artifact_paths, title_from_artifact_key, value_string_field,
    };
    let paths = artifact_paths(value);
    let summary = preview_summary(value);
    let mut entries = vec![ArtifactCatalogEntry {
        key: "output".into(),
        kind: "subtask_output".into(),
        title: None,
        summary: summary.clone(),
        paths: paths.clone(),
    }];
    if let Value::Object(map) = value {
        if let Some(kind) = value_string_field(value, "kind") {
            entries.push(ArtifactCatalogEntry {
                key: "root".into(),
                kind,
                title: preview_string(value, "title"),
                summary,
                paths,
            });
        }
        for &key in ARTIFACT_SECTION_KEYS {
            let Some(value) = map.get(key).filter(|value| !value.is_null()) else {
                continue;
            };
            entries.push(ArtifactCatalogEntry {
                key: key.into(),
                kind: artifact_kind_for_key(key, value),
                title: Some(
                    preview_string(value, "title").unwrap_or_else(|| title_from_artifact_key(key)),
                ),
                summary: preview_summary(value).or_else(|| {
                    value
                        .as_array()
                        .map(|items| format!("{} item(s)", items.len()))
                }),
                paths: artifact_paths(value),
            });
        }
    } else {
        entries.push(ArtifactCatalogEntry {
            key: "payload".into(),
            kind: "artifact".into(),
            title: None,
            summary,
            paths,
        });
    }
    serde_json::to_value(entries).expect("artifact catalog contains only serializable metadata")
}

fn preview_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(task_preview)
}

fn preview_summary(value: &Value) -> Option<String> {
    // Match artifact_summary's priority without cloning an entire result only
    // to retain the short card preview.
    ["summary", "description", "result"]
        .into_iter()
        .find_map(|key| preview_string(value, key))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskArtifactMetadata {
    pub id: String,
    pub run_id: String,
    pub kind: String,
    pub title: String,
    pub summary: Option<String>,
    pub paths: Vec<String>,
    pub source: String,
    pub created_at: String,
}

impl From<AgentTaskArtifactSummary> for AgentTaskArtifactMetadata {
    fn from(artifact: AgentTaskArtifactSummary) -> Self {
        Self {
            id: artifact.id,
            run_id: artifact.run_id,
            kind: artifact.kind,
            title: artifact.title,
            summary: artifact.summary,
            paths: artifact.paths,
            source: artifact.source,
            created_at: artifact.created_at,
        }
    }
}

const TASK_PREVIEW_CHARS: usize = 512;

/// Live/task-list consumers need skill metadata and subtask previews, not the
/// executor configuration, context pack, or a second copy of the whole trace.
/// Full artifacts remain available through the explicit task/subagent detail
/// commands. This consumes a read projection; it never changes durable state.
pub fn prepare_task_run_for_frontend(mut task_run: AgentTaskRun) -> AgentTaskRun {
    task_run.artifacts = task_run.artifacts.as_ref().and_then(project_artifacts);
    task_run
}

/// Compute once from sanitized authoritative artifacts before snapshot offload.
/// Canonical inputs with an authoritative subtask array omit full transcripts.
pub(crate) fn project_artifacts(artifacts: &Value) -> Option<Value> {
    (!artifacts.is_null()).then(|| task_artifacts_for_frontend(artifacts))
}

fn task_artifacts_for_frontend(artifacts: &Value) -> Value {
    match artifacts {
        Value::Array(items) => {
            Value::Array(items.iter().map(task_artifacts_for_frontend).collect())
        }
        Value::Object(artifacts)
            if artifacts.get("kind").and_then(Value::as_str) == Some("agentTaskArtifacts")
                && artifacts.get("subtasks").is_some_and(Value::is_array) =>
        {
            let mut projection = Map::new();
            for key in ["kind", "version", "selectedSkills"] {
                if let Some(value) = artifacts.get(key) {
                    projection.insert(key.to_string(), value.clone());
                }
            }
            if let Some(subtasks) = artifacts.get("subtasks") {
                let subtasks = match subtasks {
                    Value::Array(rows) => {
                        Value::Array(rows.iter().map(subtask_for_frontend).collect())
                    }
                    other => other.clone(),
                };
                projection.insert("subtasks".to_string(), subtasks);
            }
            Value::Object(projection)
        }
        // Old/custom artifact envelopes may have their own presentation shape.
        // Only prune canonical containers with an authoritative subtask array.
        // Missing/malformed arrays still need the frontend's recursive fallback.
        other => other.clone(),
    }
}

fn subtask_for_frontend(subtask: &Value) -> Value {
    let Value::Object(row) = subtask else {
        return subtask.clone();
    };
    if !row.get("id").is_some_and(Value::is_string)
        || !row.get("parentRunId").is_some_and(Value::is_string)
    {
        return Value::Object(row.clone());
    }

    let mut projection = pick_preview_fields(
        row,
        &[
            "id",
            "parentRunId",
            "agentId",
            "workerId",
            "kind",
            "status",
            "phase",
            "tokenBudget",
            "createdAt",
            "updatedAt",
            "startedAt",
            "finishedAt",
        ],
        &["label", "name", "role", "errorMessage"],
    );
    if let Some(Value::Object(input)) = row.get("input") {
        projection.insert(
            "input".to_string(),
            Value::Object(pick_preview_fields(
                input,
                &["kind", "agentId", "workerId", "callLabel", "reservedTokens"],
                &["task", "name", "roleName", "role"],
            )),
        );
    }
    if let Some(Value::Object(output)) = row.get("output") {
        let mut preview = pick_preview_fields(output, &["kind"], &["error", "result", "summary"]);
        for key in ["run", "judgement"] {
            if let Some(Value::Object(run)) = output.get(key) {
                preview.insert(
                    key.to_string(),
                    Value::Object(pick_preview_fields(
                        run,
                        &["id", "agentId", "workerId", "status", "isError"],
                        &[
                            "task",
                            "name",
                            "role",
                            "roleName",
                            "result",
                            "summary",
                            "errorMessage",
                        ],
                    )),
                );
            }
        }
        projection.insert("output".to_string(), Value::Object(preview));
    }
    Value::Object(projection)
}

fn pick_preview_fields(
    source: &Map<String, Value>,
    identity_fields: &[&str],
    preview_fields: &[&str],
) -> Map<String, Value> {
    let mut projection = Map::new();
    for key in identity_fields {
        if let Some(value) = source.get(*key) {
            projection.insert((*key).to_string(), value.clone());
        }
    }
    for key in preview_fields {
        if let Some(value) = source.get(*key) {
            let preview = match value {
                Value::String(value) => Value::String(task_preview(value)),
                other => other.clone(),
            };
            projection.insert((*key).to_string(), preview);
        }
    }
    projection
}

pub(crate) fn task_preview(text: &str) -> String {
    let mut chars = text.char_indices();
    let Some((end, _)) = chars.nth(TASK_PREVIEW_CHARS - 1) else {
        return text.to_string();
    };
    if chars.next().is_none() {
        return text.to_string();
    }
    let mut preview = String::with_capacity(end + '…'.len_utf8());
    preview.push_str(&text[..end]);
    preview.push('…');
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{ConversationMessage, CreateConversationInput};
    use crate::db::Database;
    use crate::llm::Role;
    use serde_json::json;

    #[test]
    fn task_snapshot_preserves_preview_identity_and_keeps_full_details_durable() {
        let db = Database::open_memory().unwrap();
        let conversation = db
            .create_conversation(&CreateConversationInput {
                provider: "openai".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        let message = ConversationMessage {
            id: "snapshot-user-message".to_string(),
            conversation_id: conversation.id.clone(),
            role: Role::User,
            content: "Review the evidence".to_string(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            artifacts: None,
            token_count: 4,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        };
        db.add_message(&message).unwrap();
        let turn = db
            .create_conversation_turn(&conversation.id, &message.id, None)
            .unwrap();
        let mut run = db
            .create_agent_task_run(
                &conversation.id,
                &turn.id,
                &message.id,
                "Snapshot",
                Some("openai"),
                Some("test"),
            )
            .unwrap();
        let long_result = "Evidence 结果🦀. ".repeat(100_000);
        let selected_skills = json!({
            "kind": "selectedSkills", "skills": [{
                "id": "docs", "name": "Docs", "description": "Review documents", "enabled": true,
            }],
        });
        let artifacts = json!({
            "kind": "agentTaskArtifacts", "version": 1,
            "trace": { "trace": long_result },
            "runtimeSession": { "systemPrompt": long_result },
            "contextPack": { "content": long_result },
            "selectedSkills": selected_skills,
            "subtasks": [{
                "id": "row-1", "parentRunId": run.id, "label": "Review evidence", "role": "Reviewer",
                "status": "completed", "phase": "done", "tokenBudget": 4000,
                "input": { "kind": "subagent_input", "agentId": "lifecycle-1", "callLabel": "worker-1",
                    "task": "Review the complete evidence", "reservedTokens": 4000, "context": long_result },
                "output": { "kind": "subagent_result", "run": { "id": "worker-1", "name": "Reviewer",
                    "status": "done", "result": long_result, "toolEvents": [long_result] } },
            }],
        });
        let plan = json!({ "kind": "plan", "steps": [{ "id": "review", "title": "Review evidence", "status": "completed" }] });
        db.update_agent_task_run_progress(
            &run.id,
            Some("running"),
            Some("tooling"),
            None,
            Some("Reviewing evidence"),
            Some(&plan),
            Some(&artifacts),
        )
        .unwrap();
        run = db.get_agent_task_run(&run.id).unwrap();
        let before = serde_json::to_vec(&run).unwrap().len();
        let projected = prepare_task_run_for_frontend(run);
        let after = serde_json::to_vec(&projected).unwrap().len();
        println!("Task snapshot bytes: {before} -> {after}");
        assert!(before > 5_000_000);
        assert!(
            after < 8_192,
            "trace or worker transcript leaked into snapshot: {after}"
        );
        assert_eq!(projected.status, "running");
        assert_eq!(projected.plan, Some(plan));
        let preview = projected.artifacts.unwrap();
        assert_eq!(preview["selectedSkills"], selected_skills);
        assert!(preview.get("trace").is_none());
        assert!(preview.get("runtimeSession").is_none());
        assert!(preview.get("contextPack").is_none());
        let worker = &preview["subtasks"][0];
        assert_eq!(worker["id"], "row-1");
        assert_eq!(worker["input"]["agentId"], "lifecycle-1");
        assert_eq!(worker["input"]["callLabel"], "worker-1");
        assert_eq!(worker["output"]["run"]["id"], "worker-1");
        assert_eq!(worker["status"], "completed");
        assert_eq!(worker["tokenBudget"], 4000);
        let result = worker["output"]["run"]["result"].as_str().unwrap();
        assert_eq!(result.chars().count(), TASK_PREVIEW_CHARS);
        assert!(result.ends_with('…'));
        assert_eq!(
            db.get_agent_task_run(&projected.id).unwrap().artifacts,
            Some(artifacts)
        );
        let stored_preview = db.get_agent_task_run_presentation(&projected.id).unwrap();
        assert_eq!(stored_preview.artifacts, Some(preview));
        let child = db
            .create_agent_subtask_run(
                &projected.id,
                "Review evidence",
                "Reviewer",
                None,
                Some(4000),
            )
            .unwrap();
        db.finish_agent_subtask_run(
            &child.id,
            "completed",
            Some(&json!({ "kind": "subagent_result", "run": { "result": long_result } })),
            None,
        )
        .unwrap();
        let catalog = db.list_agent_task_artifact_metadata(&projected.id).unwrap();
        let legacy = db.list_agent_task_artifacts(&projected.id).unwrap();
        let expected = legacy
            .into_iter()
            .map(AgentTaskArtifactMetadata::from)
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::to_value(&catalog).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert!(serde_json::to_vec(&catalog).unwrap().len() < 4_096);
        let trace_id = format!("{}:trace", projected.id);
        let detail = db
            .get_agent_task_artifact_detail(&projected.id, &trace_id)
            .unwrap();
        assert_eq!(detail.payload["trace"], long_result);
        assert!(db
            .get_agent_task_artifact_detail("unrelated-run", &trace_id)
            .is_err());
        assert!(db
            .get_agent_task_artifact_detail(&projected.id, &format!("{}:unknown", projected.id))
            .is_err());
        let child_id = format!("{}:subtask:{}", projected.id, child.id);
        assert_eq!(
            db.get_agent_task_artifact_detail(&projected.id, &child_id)
                .unwrap()
                .payload["run"]["result"],
            long_result
        );
        assert!(db
            .get_agent_task_artifact_detail(
                "foreign-run",
                &format!("foreign-run:subtask:{}", child.id)
            )
            .is_err());
    }

    #[test]
    fn task_snapshot_keeps_empty_subtasks_authoritative_and_preserves_legacy_shapes() {
        let canonical = json!({ "kind": "agentTaskArtifacts", "version": 1, "subtasks": [],
            "trace": { "runs": [{ "id": "ordinary-tool", "label": "Not a subagent" }] } });
        let preview = task_artifacts_for_frontend(&canonical);
        assert_eq!(preview["subtasks"], json!([]));
        let legacy = json!({ "kind": "subagent_result", "id": "worker", "task": "Review", "status": "done" });
        assert_eq!(task_artifacts_for_frontend(&legacy), legacy);
        let mut legacy_canonical = json!({ "kind": "agentTaskArtifacts", "version": 1,
            "trace": { "artifacts": legacy } });
        assert_eq!(
            task_artifacts_for_frontend(&legacy_canonical),
            legacy_canonical
        );
        legacy_canonical["subtasks"] = json!({ "legacy": true });
        assert_eq!(
            task_artifacts_for_frontend(&legacy_canonical),
            legacy_canonical
        );
        assert_eq!(task_preview(&"🦀".repeat(512)), "🦀".repeat(512));
    }

    #[test]
    fn task_artifact_metadata_omits_body_without_losing_card_fields() {
        let artifact = AgentTaskArtifactSummary {
            id: "run:trace".into(),
            run_id: "run".into(),
            kind: "trace".into(),
            title: "Execution trace".into(),
            summary: Some("Evidence inspected".into()),
            paths: vec!["report.docx".into()],
            source: "task_run".into(),
            created_at: "now".into(),
            payload: json!({ "trace": "Evidence. ".repeat(100_000) }),
        };
        let before = serde_json::to_vec(&artifact).unwrap().len();
        let mut expected = serde_json::to_value(&artifact).unwrap();
        expected.as_object_mut().unwrap().remove("payload");
        let metadata = AgentTaskArtifactMetadata::from(artifact);
        let after = serde_json::to_vec(&metadata).unwrap().len();
        println!("Task artifact card bytes: {before} -> {after}");
        assert!(before > 1_000_000);
        assert!(after < 512);
        assert_eq!(serde_json::to_value(metadata).unwrap(), expected);
    }
}
