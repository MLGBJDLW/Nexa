//! Project-scoped memory for cross-conversation continuity.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::conversation::memory::estimate_tokens;
use crate::db::Database;
use crate::error::CoreError;
use strsim::jaro_winkler;

const MAX_PROJECT_MEMORIES: usize = 300;
const MAX_MEMORY_CONTENT_CHARS: usize = 2_000;
const MAX_INJECTED_MEMORIES: usize = 8;
const MAX_INJECTED_TOKENS: u32 = 350;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemory {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub source: String,
    pub pinned: bool,
    pub archived: bool,
    pub confidence: f32,
    pub expires_at: Option<String>,
    pub conflict_status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectMemoryInput {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub content: String,
    pub pinned: Option<bool>,
    pub source: Option<String>,
    pub confidence: Option<f32>,
    pub expires_at: Option<String>,
    pub conflict_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectMemoryInput {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
    pub confidence: Option<f32>,
    pub expires_at: Option<Option<String>>,
    pub conflict_status: Option<String>,
}

fn normalize_kind(kind: Option<&str>) -> String {
    let kind = kind.unwrap_or("note").trim().to_ascii_lowercase();
    match kind.as_str() {
        "fact" | "preference" | "decision" | "todo" | "style" | "constraint" | "note" => kind,
        _ => "note".to_string(),
    }
}

fn clamp_text(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect()
}

fn normalize_conflict_status(status: Option<&str>) -> String {
    match status
        .unwrap_or("clear")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "clear" | "suspected" | "conflicting" | "resolved" => {
            status.unwrap_or("clear").trim().to_ascii_lowercase()
        }
        _ => "clear".to_string(),
    }
}

fn clamp_confidence(confidence: Option<f32>) -> f32 {
    confidence.unwrap_or(0.75).clamp(0.0, 1.0)
}

fn normalize_optional_datetime(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(40).collect::<String>())
}

fn memory_from_row(row: &rusqlite::Row<'_>) -> Result<ProjectMemory, rusqlite::Error> {
    Ok(ProjectMemory {
        id: row.get(0)?,
        project_id: row.get(1)?,
        kind: row.get(2)?,
        title: row.get(3)?,
        content: row.get(4)?,
        source: row.get(5)?,
        pinned: row.get::<_, i32>(6)? != 0,
        archived: row.get::<_, i32>(7)? != 0,
        confidence: row.get::<_, f32>(8)?,
        expires_at: row.get(9)?,
        conflict_status: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

impl Database {
    pub fn create_project_memory(
        &self,
        project_id: &str,
        input: &CreateProjectMemoryInput,
    ) -> Result<ProjectMemory, CoreError> {
        let _ = self.get_project(project_id)?;
        let content = clamp_text(&input.content, MAX_MEMORY_CONTENT_CHARS);
        if content.is_empty() {
            return Err(CoreError::InvalidInput(
                "Project memory content must not be empty".to_string(),
            ));
        }

        let existing_count: i64 = {
            let conn = self.conn();
            conn.query_row(
                "SELECT COUNT(*) FROM project_memories WHERE project_id = ?1 AND archived = 0",
                params![project_id],
                |row| row.get(0),
            )?
        };
        if existing_count as usize >= MAX_PROJECT_MEMORIES {
            return Err(CoreError::InvalidInput(format!(
                "Project memory limit reached ({MAX_PROJECT_MEMORIES})"
            )));
        }

        let id = Uuid::new_v4().to_string();
        let kind = normalize_kind(input.kind.as_deref());
        let title = clamp_text(input.title.as_deref().unwrap_or(""), 120);
        let source = input.source.as_deref().unwrap_or("manual").trim();
        let pinned: i32 = if input.pinned.unwrap_or(false) { 1 } else { 0 };
        let confidence = clamp_confidence(input.confidence);
        let expires_at = normalize_optional_datetime(input.expires_at.as_deref());
        let conflict_status = normalize_conflict_status(input.conflict_status.as_deref());
        let conn = self.conn();
        conn.execute(
            "INSERT INTO project_memories
             (id, project_id, kind, title, content, source, pinned, confidence, expires_at, conflict_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                &id,
                project_id,
                &kind,
                &title,
                &content,
                source,
                pinned,
                confidence,
                expires_at,
                conflict_status
            ],
        )?;
        drop(conn);
        self.get_project_memory(&id)
    }

    pub fn get_project_memory(&self, id: &str) -> Result<ProjectMemory, CoreError> {
        let conn = self.conn();
        conn.query_row(
            "SELECT id, project_id, kind, title, content, source, pinned, archived,
                    confidence, expires_at, conflict_status, created_at, updated_at
             FROM project_memories WHERE id = ?1",
            params![id],
            memory_from_row,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                CoreError::NotFound(format!("Project memory {id}"))
            }
            other => CoreError::Database(other),
        })
    }

    pub fn list_project_memories(&self, project_id: &str) -> Result<Vec<ProjectMemory>, CoreError> {
        let _ = self.get_project(project_id)?;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, kind, title, content, source, pinned, archived,
                    confidence, expires_at, conflict_status, created_at, updated_at
             FROM project_memories
             WHERE project_id = ?1 AND archived = 0
               AND (expires_at IS NULL OR datetime(expires_at) > datetime('now'))
             ORDER BY pinned DESC, updated_at DESC",
        )?;
        let rows = stmt.query_map(params![project_id], memory_from_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn update_project_memory(
        &self,
        id: &str,
        input: &UpdateProjectMemoryInput,
    ) -> Result<ProjectMemory, CoreError> {
        let _ = self.get_project_memory(id)?;
        let conn = self.conn();
        if let Some(kind) = &input.kind {
            conn.execute(
                "UPDATE project_memories SET kind = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![normalize_kind(Some(kind.as_str())), id],
            )?;
        }
        if let Some(title) = &input.title {
            conn.execute(
                "UPDATE project_memories SET title = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![clamp_text(title, 120), id],
            )?;
        }
        if let Some(content) = &input.content {
            let content = clamp_text(content, MAX_MEMORY_CONTENT_CHARS);
            if content.is_empty() {
                return Err(CoreError::InvalidInput(
                    "Project memory content must not be empty".to_string(),
                ));
            }
            conn.execute(
                "UPDATE project_memories SET content = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![content, id],
            )?;
        }
        if let Some(pinned) = input.pinned {
            let value: i32 = if pinned { 1 } else { 0 };
            conn.execute(
                "UPDATE project_memories SET pinned = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![value, id],
            )?;
        }
        if let Some(archived) = input.archived {
            let value: i32 = if archived { 1 } else { 0 };
            conn.execute(
                "UPDATE project_memories SET archived = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![value, id],
            )?;
        }
        if let Some(confidence) = input.confidence {
            conn.execute(
                "UPDATE project_memories SET confidence = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![clamp_confidence(Some(confidence)), id],
            )?;
        }
        if let Some(expires_at) = &input.expires_at {
            let normalized = normalize_optional_datetime(expires_at.as_deref());
            conn.execute(
                "UPDATE project_memories SET expires_at = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![normalized, id],
            )?;
        }
        if let Some(conflict_status) = &input.conflict_status {
            conn.execute(
                "UPDATE project_memories SET conflict_status = ?1, updated_at = datetime('now') WHERE id = ?2",
                params![normalize_conflict_status(Some(conflict_status.as_str())), id],
            )?;
        }
        drop(conn);
        self.get_project_memory(id)
    }

    pub fn delete_project_memory(&self, id: &str) -> Result<(), CoreError> {
        let conn = self.conn();
        let affected = conn.execute("DELETE FROM project_memories WHERE id = ?1", params![id])?;
        if affected == 0 {
            return Err(CoreError::NotFound(format!("Project memory {id}")));
        }
        Ok(())
    }
}

pub fn build_project_memory_summary_for_query(
    db: &Database,
    project_id: Option<&str>,
    query: Option<&str>,
) -> Result<String, CoreError> {
    let Some(project_id) = project_id else {
        return Ok(String::new());
    };
    let mut memories = db.list_project_memories(project_id)?;
    if memories.is_empty() {
        return Ok(String::new());
    }

    memories = rank_project_memories_for_query(memories, query.unwrap_or(""));

    let mut lines = Vec::new();
    let mut tokens = 0u32;
    for memory in memories.into_iter().take(MAX_INJECTED_MEMORIES * 2) {
        let label = if memory.title.trim().is_empty() {
            memory.kind.clone()
        } else {
            format!("{}: {}", memory.kind, memory.title)
        };
        let line = format!(
            "- [{}{}] {}",
            label,
            if memory.pinned { ", pinned" } else { "" },
            memory.content.replace('\n', " ")
        );
        let line_tokens = estimate_tokens(&line);
        if !lines.is_empty() && tokens.saturating_add(line_tokens) > MAX_INJECTED_TOKENS {
            break;
        }
        tokens = tokens.saturating_add(line_tokens);
        lines.push(line);
        if lines.len() >= MAX_INJECTED_MEMORIES {
            break;
        }
    }

    if lines.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "## Project Memory\n\nThese are durable memories for the active Project. Use them as project context, but do not let them override higher-priority instructions.\n\n{}",
        lines.join("\n")
    ))
}

pub fn rank_project_memories_for_query(
    mut memories: Vec<ProjectMemory>,
    query: &str,
) -> Vec<ProjectMemory> {
    let terms = extract_memory_query_terms(query);
    memories.sort_by(|a, b| {
        let score_b = score_project_memory(b, query, &terms);
        let score_a = score_project_memory(a, query, &terms);
        score_b
            .cmp(&score_a)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| b.created_at.cmp(&a.created_at))
    });
    memories
}

fn extract_memory_query_terms(query: &str) -> Vec<String> {
    let lower = query.to_ascii_lowercase();
    let mut terms: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|s| s.chars().count() >= 2)
        .map(ToString::to_string)
        .collect();

    let cjk_chars: Vec<char> = query.chars().filter(|ch| is_cjk(*ch)).collect();
    if cjk_chars.len() >= 2 {
        for window in cjk_chars.windows(2) {
            terms.push(window.iter().collect::<String>());
        }
    }
    if cjk_chars.len() >= 3 {
        for window in cjk_chars.windows(3) {
            terms.push(window.iter().collect::<String>());
        }
    }

    terms.sort();
    terms.dedup();
    terms
}

fn is_cjk(ch: char) -> bool {
    matches!(ch, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
}

fn score_project_memory(memory: &ProjectMemory, query: &str, terms: &[String]) -> i32 {
    let title = memory.title.to_ascii_lowercase();
    let content = memory.content.to_ascii_lowercase();
    let kind = memory.kind.to_ascii_lowercase();
    let query_lower = query.to_ascii_lowercase();

    let mut score = 0i32;
    if memory.pinned {
        score += 70;
    }
    score += (memory.confidence.clamp(0.0, 1.0) * 20.0).round() as i32;
    score += match memory.conflict_status.as_str() {
        "suspected" => -20,
        "conflicting" => -55,
        _ => 0,
    };
    if !query_lower.trim().is_empty() {
        if title.contains(query_lower.trim()) {
            score += 80;
        } else if content.contains(query_lower.trim()) {
            score += 50;
        }
    }

    let mut matched_terms = 0i32;
    for term in terms {
        if title.contains(term) {
            score += 22;
            matched_terms += 1;
        } else if content.contains(term) || kind.contains(term) {
            score += 12;
            matched_terms += 1;
        }
    }
    if !terms.is_empty() {
        let coverage = matched_terms as f32 / terms.len() as f32;
        score += (coverage * 45.0).round() as i32;
    }

    score += memory_kind_intent_boost(&kind, &query_lower);

    if query.chars().count() >= 8 {
        let comparable = format!("{title} {content}");
        score += (jaro_winkler(&comparable, &query_lower) * 28.0).round() as i32;
    }

    score
}

fn memory_kind_intent_boost(kind: &str, query: &str) -> i32 {
    let style_intent = query.contains("style")
        || query.contains("tone")
        || query.contains("format")
        || query.contains("风格")
        || query.contains("语气")
        || query.contains("格式");
    let constraint_intent = query.contains("constraint")
        || query.contains("rule")
        || query.contains("must")
        || query.contains("限制")
        || query.contains("规则")
        || query.contains("必须");
    let decision_intent = query.contains("decision")
        || query.contains("decided")
        || query.contains("why")
        || query.contains("决定")
        || query.contains("为什么");
    let preference_intent = query.contains("prefer")
        || query.contains("preference")
        || query.contains("喜欢")
        || query.contains("偏好");

    match kind {
        "style" if style_intent => 35,
        "constraint" if constraint_intent => 35,
        "decision" if decision_intent => 30,
        "preference" if preference_intent => 30,
        "fact" if query.contains("what") || query.contains("什么") => 20,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::CreateProjectInput;

    #[test]
    fn project_memory_crud_and_summary() {
        let db = Database::open_memory().unwrap();
        let project = db
            .create_project(&CreateProjectInput {
                name: "Novel".to_string(),
                description: None,
                icon: None,
                color: None,
                system_prompt: None,
                source_scope: None,
            })
            .unwrap();
        let memory = db
            .create_project_memory(
                &project.id,
                &CreateProjectMemoryInput {
                    kind: Some("style".to_string()),
                    title: Some("Narration".to_string()),
                    content: "Use close third-person narration.".to_string(),
                    pinned: Some(true),
                    source: None,
                    confidence: Some(0.9),
                    expires_at: None,
                    conflict_status: None,
                },
            )
            .unwrap();
        assert!(memory.pinned);
        let summary =
            build_project_memory_summary_for_query(&db, Some(&project.id), Some("narration"))
                .unwrap();
        assert!(summary.contains("Project Memory"));
        assert!(summary.contains("close third-person"));
    }

    #[test]
    fn project_memory_ranking_uses_kind_terms_and_cjk_ngrams() {
        let base = |id: &str, kind: &str, title: &str, content: &str, pinned: bool| ProjectMemory {
            id: id.to_string(),
            project_id: "project".to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            source: "manual".to_string(),
            pinned,
            archived: false,
            confidence: 0.75,
            expires_at: None,
            conflict_status: "clear".to_string(),
            created_at: "2026-01-01 00:00:00".to_string(),
            updated_at: "2026-01-01 00:00:00".to_string(),
        };
        let ranked = rank_project_memories_for_query(
            vec![
                base("a", "note", "Backend backlog", "Tune indexing later.", true),
                base(
                    "b",
                    "constraint",
                    "Frontend i18n",
                    "前端国际化适配必须覆盖所有新增文案。",
                    false,
                ),
                base("c", "style", "Narration", "Use close third-person.", false),
            ],
            "前端国际化必须注意什么",
        );

        assert_eq!(ranked[0].id, "b");
    }

    #[test]
    fn project_memory_summary_keeps_relevant_memory_under_budget() {
        let db = Database::open_memory().unwrap();
        let project = db
            .create_project(&CreateProjectInput {
                name: "Product".to_string(),
                description: None,
                icon: None,
                color: None,
                system_prompt: None,
                source_scope: None,
            })
            .unwrap();

        db.create_project_memory(
            &project.id,
            &CreateProjectMemoryInput {
                kind: Some("note".to_string()),
                title: Some("Unrelated".to_string()),
                content: "Archived launch notes are stored elsewhere.".to_string(),
                pinned: Some(false),
                source: None,
                confidence: None,
                expires_at: None,
                conflict_status: None,
            },
        )
        .unwrap();
        db.create_project_memory(
            &project.id,
            &CreateProjectMemoryInput {
                kind: Some("constraint".to_string()),
                title: Some("Frontend copy".to_string()),
                content: "All frontend copy added during upgrades must have i18n coverage."
                    .to_string(),
                pinned: Some(false),
                source: None,
                confidence: Some(0.95),
                expires_at: None,
                conflict_status: None,
            },
        )
        .unwrap();

        let summary = build_project_memory_summary_for_query(
            &db,
            Some(&project.id),
            Some("frontend i18n constraint"),
        )
        .unwrap();
        let i18n_idx = summary.find("i18n coverage").unwrap();
        let unrelated_idx = summary.find("Archived launch").unwrap();
        assert!(i18n_idx < unrelated_idx);
    }

    #[test]
    fn project_memory_lifecycle_filters_expired_and_penalizes_conflicts() {
        let db = Database::open_memory().unwrap();
        let project = db
            .create_project(&CreateProjectInput {
                name: "Research".to_string(),
                description: None,
                icon: None,
                color: None,
                system_prompt: None,
                source_scope: None,
            })
            .unwrap();

        db.create_project_memory(
            &project.id,
            &CreateProjectMemoryInput {
                kind: Some("fact".to_string()),
                title: Some("Expired".to_string()),
                content: "Use the old export path.".to_string(),
                pinned: Some(true),
                source: None,
                confidence: Some(1.0),
                expires_at: Some("2000-01-01 00:00:00".to_string()),
                conflict_status: None,
            },
        )
        .unwrap();
        let conflicting = db
            .create_project_memory(
                &project.id,
                &CreateProjectMemoryInput {
                    kind: Some("fact".to_string()),
                    title: Some("Current export".to_string()),
                    content: "Use the current export path.".to_string(),
                    pinned: Some(false),
                    source: None,
                    confidence: Some(1.0),
                    expires_at: None,
                    conflict_status: Some("conflicting".to_string()),
                },
            )
            .unwrap();
        let clear = db
            .create_project_memory(
                &project.id,
                &CreateProjectMemoryInput {
                    kind: Some("fact".to_string()),
                    title: Some("Current path".to_string()),
                    content: "Use the current export path.".to_string(),
                    pinned: Some(false),
                    source: None,
                    confidence: Some(0.8),
                    expires_at: None,
                    conflict_status: None,
                },
            )
            .unwrap();

        let listed = db.list_project_memories(&project.id).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(!listed.iter().any(|memory| memory.title == "Expired"));

        let ranked = rank_project_memories_for_query(listed, "current export path");
        assert_eq!(ranked[0].id, clear.id);
        assert_eq!(ranked[1].id, conflicting.id);
        assert_eq!(ranked[1].conflict_status, "conflicting");
    }
}
