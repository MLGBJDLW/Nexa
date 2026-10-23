//! Card metadata is materialized with sanitized artifact writes. Listing cards
//! must never hydrate the full trace or each worker's output.

use super::{
    artifact_kind_for_key, artifact_paths, artifact_summary, title_from_artifact_key,
    value_string_field, AgentTaskArtifactSummary,
};
use crate::db::Database;
use crate::error::CoreError;
use crate::task_run_presentation::{
    task_preview, AgentTaskArtifactMetadata, ArtifactCatalogEntry, ARTIFACT_SECTION_KEYS,
};
use rusqlite::params;
use serde_json::Value;

fn missing_artifact() -> CoreError {
    CoreError::NotFound("Task artifact".to_string())
}

fn read_catalog(
    encoded: Option<String>,
    has_artifacts: bool,
) -> Result<Vec<ArtifactCatalogEntry>, CoreError> {
    match encoded {
        Some(encoded) => Ok(serde_json::from_str(&encoded)?),
        None if !has_artifacts => Ok(Vec::new()),
        None => Err(CoreError::Internal(
            "Task artifact metadata is unavailable".to_string(),
        )),
    }
}

impl Database {
    pub fn list_agent_task_artifact_metadata(
        &self,
        run_id: &str,
    ) -> Result<Vec<AgentTaskArtifactMetadata>, CoreError> {
        let connection = self.conn();
        let (title, summary, updated_at, catalog, has_artifacts): (String, Option<String>, String, Option<String>, bool) = connection.query_row(
            "SELECT title, summary, updated_at, json_extract(artifacts_preview_json, '$.catalog'), artifacts_json IS NOT NULL
             FROM agent_task_runs WHERE id=?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound(format!("Agent task run {run_id}")),
            other => CoreError::Database(other),
        })?;
        let mut result = read_catalog(catalog, has_artifacts)?
            .into_iter()
            .filter(|entry| entry.key != "output")
            .map(|entry| {
                let is_root = matches!(entry.key.as_str(), "root" | "payload");
                AgentTaskArtifactMetadata {
                    id: format!("{run_id}:{}", entry.key),
                    run_id: run_id.to_string(),
                    kind: entry.kind,
                    title: entry.title.unwrap_or_else(|| task_preview(&title)),
                    summary: entry.summary.or_else(|| {
                        is_root
                            .then(|| summary.as_deref().map(task_preview))
                            .flatten()
                    }),
                    paths: entry.paths,
                    source: "task_run".to_string(),
                    created_at: updated_at.clone(),
                }
            })
            .collect::<Vec<_>>();

        let mut statement = connection.prepare(
            "SELECT id, role, label, error_message, updated_at, output_preview_json
             FROM agent_subtask_runs WHERE parent_run_id=?1 AND output_json IS NOT NULL
             ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map([run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        for row in rows {
            let (id, role, label, error, updated_at, catalog) = row?;
            let output = read_catalog(catalog, true)?
                .into_iter()
                .find(|entry| entry.key == "output")
                .ok_or_else(|| {
                    CoreError::Internal("Subtask output metadata is unavailable".to_string())
                })?;
            result.push(AgentTaskArtifactMetadata {
                id: format!("{run_id}:subtask:{id}"),
                run_id: run_id.to_string(),
                kind: "subtask_output".to_string(),
                title: task_preview(&format!("{role}: {label}")),
                summary: output
                    .summary
                    .or_else(|| error.as_deref().map(task_preview)),
                paths: output.paths,
                source: id,
                created_at: updated_at,
            });
        }
        Ok(result)
    }

    /// Hydrate just the requested artifact owner and move the selected payload
    /// into the response. Do not build the root+trace+subtask full-payload list.
    pub fn get_agent_task_artifact_detail(
        &self,
        run_id: &str,
        artifact_id: &str,
    ) -> Result<AgentTaskArtifactSummary, CoreError> {
        let prefix = format!("{run_id}:");
        let key = artifact_id
            .strip_prefix(&prefix)
            .ok_or_else(missing_artifact)?;
        if let Some(subtask_id) = key.strip_prefix("subtask:") {
            let owned: bool = self.conn().query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_subtask_runs WHERE id=?1 AND parent_run_id=?2)",
                params![subtask_id, run_id],
                |row| row.get(0),
            )?;
            if !owned {
                return Err(missing_artifact());
            }
            let subtask = self.get_agent_subtask_run(subtask_id)?;
            if subtask.parent_run_id != run_id {
                return Err(missing_artifact());
            }
            let output = subtask.output.ok_or_else(missing_artifact)?;
            return Ok(AgentTaskArtifactSummary {
                id: artifact_id.to_string(),
                run_id: run_id.to_string(),
                kind: "subtask_output".to_string(),
                title: format!("{}: {}", subtask.role, subtask.label),
                summary: artifact_summary(&output).or(subtask.error_message),
                paths: artifact_paths(&output),
                source: subtask.id,
                created_at: subtask.updated_at,
                payload: output,
            });
        }
        if !matches!(key, "root" | "payload") && !ARTIFACT_SECTION_KEYS.contains(&key) {
            return Err(missing_artifact());
        }
        let run = self.get_agent_task_run(run_id)?;
        let root = run.artifacts.ok_or_else(missing_artifact)?;
        let is_root = matches!(key, "root" | "payload");
        let payload = match key {
            "root" if root.is_object() && value_string_field(&root, "kind").is_some() => root,
            "payload" if !root.is_object() => root,
            "root" | "payload" => return Err(missing_artifact()),
            _ => match root {
                Value::Object(mut object) => object
                    .remove(key)
                    .filter(|value| !value.is_null())
                    .ok_or_else(missing_artifact)?,
                _ => return Err(missing_artifact()),
            },
        };
        Ok(AgentTaskArtifactSummary {
            id: artifact_id.to_string(),
            run_id: run_id.to_string(),
            kind: if key == "payload" {
                "artifact".to_string()
            } else {
                artifact_kind_for_key(key, &payload)
            },
            title: value_string_field(&payload, "title").unwrap_or_else(|| {
                if is_root {
                    run.title
                } else {
                    title_from_artifact_key(key)
                }
            }),
            summary: artifact_summary(&payload).or_else(|| {
                if is_root {
                    run.summary
                } else {
                    payload
                        .as_array()
                        .map(|items| format!("{} item(s)", items.len()))
                }
            }),
            paths: artifact_paths(&payload),
            source: "task_run".to_string(),
            created_at: run.updated_at,
            payload,
        })
    }
}
