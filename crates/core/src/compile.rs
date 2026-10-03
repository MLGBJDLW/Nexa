//! Knowledge compilation layer — Karpathy-inspired "raw → compile → wiki" pipeline.
//! Automatically distills documents into structured summaries, entities, and relationships.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::CoreError;
use crate::llm::{CompletionRequest, LlmProvider, Message, ProviderType, Role};

#[path = "compile_sections.rs"]
mod sections;

// ── Types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub id: String,
    pub document_id: String,
    pub summary: String,
    pub key_points: Vec<String>,
    pub tags: Vec<String>,
    pub model_used: String,
    pub compiled_at: String,
    pub input_revision: String,
    pub stale: bool,
    pub coverage: CompileCoverage,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileCoverage {
    pub total_sections: usize,
    pub completed_sections: usize,
    pub total_chars: usize,
    pub covered_chars: usize,
    pub complete: bool,
    #[serde(default)]
    pub summary_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub entity_type: EntityType,
    pub description: String,
    pub first_seen_doc: Option<String>,
    pub mention_count: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    Concept,
    Person,
    Technology,
    Event,
    Organization,
    Place,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileResult {
    pub document_id: String,
    pub summary: DocumentSummary,
    pub entities_found: usize,
    pub links_created: usize,
    pub sections_compiled: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileStats {
    pub total_docs: i64,
    pub compiled_docs: i64,
    pub total_entities: i64,
    pub total_links: i64,
}

pub struct EntityLinkEvidence<'a> {
    pub strength: f64,
    pub evidence_doc: Option<&'a str>,
    pub evidence_snippet: Option<&'a str>,
    pub confidence: Option<f64>,
}

// ── LLM Response Parsing ──

#[derive(Clone, Serialize, Deserialize)]
struct LlmCompileOutput {
    summary: String,
    key_points: Vec<String>,
    tags: Vec<String>,
    entities: Vec<LlmEntity>,
}

#[derive(Clone, Serialize, Deserialize)]
struct LlmEntity {
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    entity_type: String,
    description: String,
    context: String,
    #[serde(default)]
    relations: Vec<LlmRelation>,
}

#[derive(Clone, Serialize, Deserialize)]
struct LlmRelation {
    target: String,
    relation_type: String,
    evidence: Option<String>,
    confidence: Option<f64>,
}

// ── Constants ──

// Bump when request semantics, output validation, or aggregation rules change.
// Prompt edits are additionally included verbatim in the persisted cache key.
const COMPILE_CONTRACT_VERSION: u32 = 1;
const COMPILE_SYSTEM_PROMPT: &str = include_str!("../prompts/compile.md");
const COMPILE_INPUT_CHAR_BUDGET: usize = 12_000;

// ── Core Functions ──

/// Compile a single document: generate summary + extract entities + build relationships.
pub async fn compile_document(
    db: &Database,
    doc_id: &str,
    provider: &dyn LlmProvider,
    model: &str,
    provider_type: Option<ProviderType>,
) -> Result<CompileResult, CoreError> {
    let mut budget = 8;
    sections::compile_document(db, doc_id, provider, model, provider_type, &mut budget).await
}

fn normalize_entity_lookup_name(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_lowercase()
}

fn normalize_entity_display_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_relation_type(relation_type: &str) -> String {
    let normalized = relation_type
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let collapsed = normalized
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if collapsed.is_empty() {
        "related_to".to_string()
    } else {
        collapsed
    }
}

/// Progress information emitted during compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileProgress {
    pub current: usize,
    pub total: usize,
    pub document_id: String,
    pub document_title: Option<String>,
    pub phase: String,
}

/// Compile all documents that haven't been compiled yet.
pub async fn compile_pending(
    db: &Database,
    provider: &dyn LlmProvider,
    model: &str,
    provider_type: Option<ProviderType>,
    limit: usize,
) -> Result<Vec<CompileResult>, CoreError> {
    compile_pending_with_progress(db, provider, model, provider_type, limit, |_| {}).await
}

/// Compile all documents that haven't been compiled yet, with progress reporting.
pub async fn compile_pending_with_progress<F>(
    db: &Database,
    provider: &dyn LlmProvider,
    model: &str,
    provider_type: Option<ProviderType>,
    limit: usize,
    on_progress: F,
) -> Result<Vec<CompileResult>, CoreError>
where
    F: Fn(&CompileProgress),
{
    let pending_ids = db.get_uncompiled_document_ids(limit)?;
    let total = pending_ids.len();
    let mut results = Vec::new();
    let mut budget = 8;
    let mut errors = Vec::new();

    for (i, doc_id) in pending_ids.iter().enumerate() {
        if budget == 0 {
            break;
        }
        let title = db.get_document_title(doc_id).ok().flatten();
        on_progress(&CompileProgress {
            current: i + 1,
            total,
            document_id: doc_id.clone(),
            document_title: title.clone(),
            phase: "compiling".to_string(),
        });

        match sections::compile_document(db, doc_id, provider, model, provider_type, &mut budget)
            .await
        {
            Ok(result) => results.push(result),
            Err(e) => {
                tracing::warn!("compile doc {doc_id}: {e}");
                errors.push(format!("{}: {e}", title.as_deref().unwrap_or(doc_id)));
                on_progress(&CompileProgress {
                    current: i + 1,
                    total,
                    document_id: doc_id.clone(),
                    document_title: title.clone(),
                    phase: "error".to_string(),
                });
            }
        }
    }

    if errors.is_empty() {
        Ok(results)
    } else {
        Err(CoreError::Llm(format!(
            "Some documents could not be compiled. Completed sections were saved. {}",
            errors.join("; ")
        )))
    }
}

pub fn parse_entity_type(s: &str) -> EntityType {
    match s.to_lowercase().as_str() {
        "concept" => EntityType::Concept,
        "person" => EntityType::Person,
        "technology" => EntityType::Technology,
        "event" => EntityType::Event,
        "organization" => EntityType::Organization,
        "place" => EntityType::Place,
        _ => EntityType::Other,
    }
}

fn entity_type_key(entity_type: &EntityType) -> &'static str {
    match entity_type {
        EntityType::Concept => "concept",
        EntityType::Person => "person",
        EntityType::Technology => "technology",
        EntityType::Event => "event",
        EntityType::Organization => "organization",
        EntityType::Place => "place",
        EntityType::Other => "other",
    }
}

fn entity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entity> {
    Ok(Entity {
        id: row.get(0)?,
        name: row.get(1)?,
        entity_type: parse_entity_type(&row.get::<_, String>(2)?),
        description: row.get(3)?,
        first_seen_doc: row.get(4)?,
        mention_count: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn normalized_entity_aliases(name: &str, aliases: &[String]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    std::iter::once(name)
        .chain(aliases.iter().map(String::as_str))
        .filter_map(|alias| {
            let display = normalize_entity_display_name(alias);
            let normalized = normalize_entity_lookup_name(&display);
            if display.is_empty() || normalized.is_empty() || !seen.insert(normalized.clone()) {
                None
            } else {
                Some((display, normalized))
            }
        })
        .collect()
}

fn find_entity_by_aliases(
    conn: &rusqlite::Connection,
    aliases: &[(String, String)],
    entity_type: &str,
) -> Result<Option<Entity>, CoreError> {
    for (_, normalized_alias) in aliases {
        let found = conn.query_row(
            "SELECT DISTINCT e.id, e.name, e.entity_type, e.description, e.first_seen_doc, e.mention_count, e.created_at
             FROM entities e
             LEFT JOIN entity_aliases ea ON ea.entity_id = e.id
             WHERE e.entity_type = ?2
               AND (ea.normalized_alias = ?1 OR lower(trim(e.name)) = ?1)
             ORDER BY e.mention_count DESC, e.name COLLATE NOCASE
             LIMIT 1",
            rusqlite::params![normalized_alias, entity_type],
            entity_from_row,
        );
        match found {
            Ok(entity) => return Ok(Some(entity)),
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Ok(None)
}

fn insert_entity_aliases(
    conn: &rusqlite::Connection,
    entity_id: &str,
    entity_type: &str,
    aliases: &[(String, String)],
) -> Result<(), CoreError> {
    for (alias, normalized_alias) in aliases {
        conn.execute(
            "INSERT OR IGNORE INTO entity_aliases (entity_id, alias, normalized_alias, entity_type)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![entity_id, alias, normalized_alias, entity_type],
        )?;
    }
    Ok(())
}

fn upsert_entity_on(
    conn: &rusqlite::Connection,
    name: &str,
    aliases: &[String],
    entity_type: &EntityType,
    description: &str,
    first_doc: &str,
) -> Result<Entity, CoreError> {
    let type_str = entity_type_key(entity_type);
    let now = chrono::Utc::now().to_rfc3339();
    let canonical_name = normalize_entity_display_name(name);
    let alias_pairs = normalized_entity_aliases(&canonical_name, aliases);

    if canonical_name.is_empty() {
        return Err(CoreError::InvalidInput(
            "Entity name cannot be empty".into(),
        ));
    }

    let existing = find_entity_by_aliases(conn, &alias_pairs, type_str)?;

    match existing {
        Some(mut entity) => {
            conn.execute(
                    "UPDATE entities
                     SET mention_count = mention_count + 1,
                         description = CASE WHEN length(?1) > length(description) THEN ?1 ELSE description END,
                         updated_at = ?2
                     WHERE id = ?3",
                    rusqlite::params![description, now, entity.id],
                )?;
            insert_entity_aliases(conn, &entity.id, type_str, &alias_pairs)?;
            entity.mention_count += 1;
            if description.len() > entity.description.len() {
                entity.description = description.to_string();
            }
            Ok(entity)
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                    "INSERT INTO entities (id, name, entity_type, description, first_seen_doc, mention_count, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
                    rusqlite::params![id, canonical_name, type_str, description, first_doc, now],
                )?;
            insert_entity_aliases(conn, &id, type_str, &alias_pairs)?;
            Ok(Entity {
                id,
                name: canonical_name,
                entity_type: entity_type.clone(),
                description: description.to_string(),
                first_seen_doc: Some(first_doc.to_string()),
                mention_count: 1,
                created_at: now,
            })
        }
    }
}

fn find_entity_on(conn: &rusqlite::Connection, name: &str) -> Result<Entity, CoreError> {
    let normalized = normalize_entity_lookup_name(name);
    conn.query_row(
            "SELECT DISTINCT e.id, e.name, e.entity_type, e.description, e.first_seen_doc, e.mention_count, e.created_at
             FROM entities e
             LEFT JOIN entity_aliases ea ON ea.entity_id = e.id
             WHERE ea.normalized_alias = ?1 OR lower(trim(e.name)) = ?1
             ORDER BY e.mention_count DESC, e.name COLLATE NOCASE
             LIMIT 1",
            rusqlite::params![normalized],
            entity_from_row,
        )
        .map_err(|_| CoreError::NotFound("Entity not found".into()))
}

fn upsert_entity_link_on(
    conn: &rusqlite::Connection,
    source_id: &str,
    target_id: &str,
    relation_type: &str,
    evidence: EntityLinkEvidence<'_>,
) -> Result<(), CoreError> {
    if let Some(document_id) = evidence.evidence_doc {
        conn.execute("INSERT INTO entity_link_support(source_entity_id,target_entity_id,relation_type,document_id,revision,strength,snippet,confidence) SELECT ?1,?2,?3,id,index_revision,?5,?6,?7 FROM documents WHERE id=?4 ON CONFLICT(source_entity_id,target_entity_id,relation_type,document_id) DO UPDATE SET revision=excluded.revision,strength=excluded.strength,snippet=excluded.snippet,confidence=excluded.confidence", rusqlite::params![source_id,target_id,normalize_relation_type(relation_type),document_id,evidence.strength.clamp(0.0,1.0),evidence.evidence_snippet.unwrap_or(""),evidence.confidence.map(|value|value.clamp(0.0,1.0))])?;
        return Ok(());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let normalized_relation_type = normalize_relation_type(relation_type);
    let clamped_strength = evidence.strength.clamp(0.0, 1.0);
    let clamped_confidence = evidence.confidence.map(|value| value.clamp(0.0, 1.0));
    let evidence_snippet = evidence.evidence_snippet.unwrap_or("").trim();
    conn.execute(
            "INSERT INTO entity_links (
                id, source_entity_id, target_entity_id, relation_type, strength,
                evidence_doc_id, evidence_snippet, confidence
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(source_entity_id, target_entity_id, relation_type) DO UPDATE SET
                strength = MIN(1.0, MAX(entity_links.strength, excluded.strength) + 0.1),
                evidence_doc_id = COALESCE(excluded.evidence_doc_id, entity_links.evidence_doc_id),
                evidence_snippet = CASE
                    WHEN length(excluded.evidence_snippet) > length(COALESCE(entity_links.evidence_snippet, ''))
                    THEN excluded.evidence_snippet
                    ELSE entity_links.evidence_snippet
                END,
                confidence = CASE
                    WHEN entity_links.confidence IS NULL THEN excluded.confidence
                    WHEN excluded.confidence IS NULL THEN entity_links.confidence
                    ELSE MAX(entity_links.confidence, excluded.confidence)
                END",
            rusqlite::params![
                id,
                source_id,
                target_id,
                normalized_relation_type,
                clamped_strength,
                evidence.evidence_doc,
                evidence_snippet,
                clamped_confidence
            ],
        )?;
    Ok(())
}

// ── Database Methods ──

fn store_summary_on(
    conn: &rusqlite::Connection,
    mut summary: DocumentSummary,
) -> Result<DocumentSummary, CoreError> {
    summary.id = conn.query_row(
        "INSERT INTO document_summaries(id,document_id,summary,key_points,tags,model_used,compiled_at,updated_at,input_revision,coverage_json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?7,?8,?9)
         ON CONFLICT(document_id) DO UPDATE SET summary=excluded.summary,key_points=excluded.key_points,tags=excluded.tags,
         model_used=excluded.model_used,compiled_at=excluded.compiled_at,updated_at=excluded.updated_at,input_revision=excluded.input_revision,coverage_json=excluded.coverage_json RETURNING id",
        rusqlite::params![summary.id,summary.document_id,summary.summary,serde_json::to_string(&summary.key_points)?,serde_json::to_string(&summary.tags)?,summary.model_used,summary.compiled_at,summary.input_revision,serde_json::to_string(&summary.coverage)?], |row| row.get(0),
    )?;
    Ok(summary)
}

fn store_summary_chunk_on(
    conn: &rusqlite::Connection,
    doc_id: &str,
    summary: &str,
    key_points: &[String],
    tags: &[String],
) -> Result<(), CoreError> {
    let content = [summary.to_string(), key_points.join("\n"), tags.join(", ")]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    conn.execute(
        "DELETE FROM chunks WHERE document_id=?1 AND kind='summary'",
        [doc_id],
    )?;
    if content.trim().is_empty() {
        return Ok(());
    }
    conn.execute("INSERT INTO chunks(id,document_id,chunk_index,kind,content,start_offset,end_offset,line_start,line_end,content_hash,metadata_json) VALUES(?1,?2,-1,'summary',?3,0,?4,0,0,?5,?6)", rusqlite::params![uuid::Uuid::new_v4().to_string(),doc_id,content,content.len() as i64,blake3::hash(content.as_bytes()).to_hex().to_string(),r#"{"extraction_method":"model_summary","locator":{"kind":"extracted","section":"compiled summary"}}"#])?;
    Ok(())
}

impl Database {
    pub fn get_document_full_text(&self, doc_id: &str) -> Result<String, CoreError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT content FROM chunks WHERE document_id = ? ORDER BY chunk_index ASC")?;
        let chunks: Vec<String> = stmt
            .query_map(rusqlite::params![doc_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(chunks.join("\n\n"))
    }

    pub fn upsert_document_summary(
        &self,
        doc_id: &str,
        summary: &str,
        key_points: &[String],
        tags: &[String],
        model: &str,
    ) -> Result<DocumentSummary, CoreError> {
        let conn = self.conn();
        let revision: String = conn.query_row(
            "SELECT index_revision FROM documents WHERE id=?1",
            [doc_id],
            |row| row.get(0),
        )?;
        store_summary_on(
            &conn,
            DocumentSummary {
                id: uuid::Uuid::new_v4().to_string(),
                document_id: doc_id.into(),
                summary: summary.into(),
                key_points: key_points.to_vec(),
                tags: tags.to_vec(),
                model_used: model.into(),
                compiled_at: chrono::Utc::now().to_rfc3339(),
                input_revision: revision,
                stale: false,
                coverage: CompileCoverage {
                    total_sections: 1,
                    completed_sections: 1,
                    complete: true,
                    ..CompileCoverage::default()
                },
            },
        )
    }

    pub fn upsert_entity(
        &self,
        name: &str,
        entity_type: &EntityType,
        description: &str,
        first_doc: &str,
    ) -> Result<Entity, CoreError> {
        self.upsert_entity_with_aliases(name, &[], entity_type, description, first_doc)
    }

    pub fn upsert_entity_with_aliases(
        &self,
        name: &str,
        aliases: &[String],
        entity_type: &EntityType,
        description: &str,
        first_doc: &str,
    ) -> Result<Entity, CoreError> {
        upsert_entity_on(
            &self.conn(),
            name,
            aliases,
            entity_type,
            description,
            first_doc,
        )
    }

    pub fn find_entity_by_name(&self, name: &str) -> Result<Entity, CoreError> {
        find_entity_on(&self.conn(), name)
    }

    pub fn link_document_entity(
        &self,
        doc_id: &str,
        entity_id: &str,
        relevance: f64,
        context: &str,
    ) -> Result<(), CoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO document_entities (document_id, entity_id, relevance, context_snippet) VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO UPDATE SET relevance = excluded.relevance, context_snippet = excluded.context_snippet",
            rusqlite::params![doc_id, entity_id, relevance, context],
        )?;
        Ok(())
    }

    pub fn upsert_entity_link(
        &self,
        source_id: &str,
        target_id: &str,
        relation_type: &str,
        strength: f64,
        evidence_doc: Option<&str>,
    ) -> Result<(), CoreError> {
        self.upsert_entity_link_with_evidence(
            source_id,
            target_id,
            relation_type,
            EntityLinkEvidence {
                strength,
                evidence_doc,
                evidence_snippet: None,
                confidence: None,
            },
        )
    }

    pub fn upsert_entity_link_with_evidence(
        &self,
        source_id: &str,
        target_id: &str,
        relation_type: &str,
        evidence: EntityLinkEvidence<'_>,
    ) -> Result<(), CoreError> {
        upsert_entity_link_on(&self.conn(), source_id, target_id, relation_type, evidence)
    }

    pub fn get_uncompiled_document_ids(&self, limit: usize) -> Result<Vec<String>, CoreError> {
        self.get_uncompiled_document_ids_scoped(limit, &[])
    }

    pub fn get_uncompiled_document_ids_scoped(
        &self,
        limit: usize,
        source_ids: &[String],
    ) -> Result<Vec<String>, CoreError> {
        let conn = self.conn();
        let mut statement = conn.prepare("SELECT d.id FROM documents d LEFT JOIN document_summaries ds ON d.id=ds.document_id WHERE (ds.id IS NULL OR ds.input_revision!=d.index_revision OR COALESCE(json_extract(ds.coverage_json,'$.complete'),0)=0) AND (?2='[]' OR d.source_id IN (SELECT value FROM json_each(?2))) ORDER BY d.indexed_at,d.id LIMIT ?1")?;
        let ids = statement
            .query_map(
                rusqlite::params![limit.min(1000), serde_json::to_string(source_ids)?],
                |row| row.get(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    pub fn get_document_title(&self, doc_id: &str) -> Result<Option<String>, CoreError> {
        let conn = self.conn();
        let result = conn.query_row(
            "SELECT title FROM documents WHERE id = ?1",
            rusqlite::params![doc_id],
            |row| row.get::<_, String>(0),
        );
        match result {
            Ok(title) => Ok(Some(title)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_document_summary(&self, doc_id: &str) -> Result<Option<DocumentSummary>, CoreError> {
        let conn = self.conn();
        let result = conn.query_row(
            "SELECT ds.id, ds.document_id, ds.summary, ds.key_points, ds.tags, ds.model_used, ds.compiled_at, ds.input_revision, ds.input_revision != d.index_revision, ds.coverage_json FROM document_summaries ds JOIN documents d ON d.id=ds.document_id WHERE ds.document_id = ?1",
            rusqlite::params![doc_id],
            |row| {
                let kp: String = row.get(3)?;
                let tags: String = row.get(4)?;
                Ok(DocumentSummary {
                    id: row.get(0)?,
                    document_id: row.get(1)?,
                    summary: row.get(2)?,
                    key_points: serde_json::from_str(&kp).unwrap_or_default(),
                    tags: serde_json::from_str(&tags).unwrap_or_default(),
                    model_used: row.get(5)?,
                    compiled_at: row.get(6)?,
                    input_revision: row.get(7)?,
                    stale: row.get(8)?,
                    coverage: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
                })
            },
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_entities_for_document(&self, doc_id: &str) -> Result<Vec<Entity>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT e.id, e.name, e.entity_type, e.description, e.first_seen_doc, e.mention_count, e.created_at FROM entities e JOIN document_entities de ON e.id = de.entity_id WHERE de.document_id = ?1 ORDER BY de.relevance DESC",
        )?;
        let entities = stmt
            .query_map(rusqlite::params![doc_id], |row| {
                Ok(Entity {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    entity_type: parse_entity_type(&row.get::<_, String>(2)?),
                    description: row.get(3)?,
                    first_seen_doc: row.get(4)?,
                    mention_count: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(entities)
    }

    pub fn get_compile_stats(&self) -> Result<CompileStats, CoreError> {
        self.get_compile_stats_scoped(&[])
    }

    pub fn get_compile_stats_scoped(
        &self,
        source_ids: &[String],
    ) -> Result<CompileStats, CoreError> {
        let conn = self.conn();
        Ok(conn.query_row("WITH selected AS (SELECT id,index_revision FROM documents WHERE ?1='[]' OR source_id IN (SELECT value FROM json_each(?1)))
          SELECT (SELECT COUNT(*) FROM selected),
          (SELECT COUNT(*) FROM document_summaries ds JOIN selected d ON d.id=ds.document_id WHERE ds.input_revision=d.index_revision AND COALESCE(json_extract(ds.coverage_json,'$.complete'),0)=1),
          (SELECT COUNT(*) FROM entities e WHERE ?1='[]' OR EXISTS(SELECT 1 FROM document_entities de JOIN selected d ON d.id=de.document_id WHERE de.entity_id=e.id)),
          (SELECT COUNT(*) FROM entity_links e WHERE (?1='[]' AND e.evidence_doc_id IS NULL)
            OR EXISTS(SELECT 1 FROM entity_link_support els JOIN selected d ON d.id=els.document_id AND d.index_revision=els.revision
              WHERE els.source_entity_id=e.source_entity_id AND els.target_entity_id=e.target_entity_id AND els.relation_type=e.relation_type))",
          [serde_json::to_string(source_ids)?], |row| Ok(CompileStats { total_docs: row.get(0)?, compiled_docs: row.get(1)?, total_entities: row.get(2)?, total_links: row.get(3)? }))?)
    }

    /// Insert (or replace) a synthetic chunk containing the compiled summary,
    /// key-points and tags so that FTS5 triggers make them searchable.
    /// Uses `chunk_index = -1` and `kind = 'summary'` to distinguish from
    /// content chunks.
    pub fn upsert_summary_chunk(
        &self,
        doc_id: &str,
        summary: &str,
        key_points: &[String],
        tags: &[String],
    ) -> Result<(), CoreError> {
        let mut conn = self.conn();
        let transaction = conn.transaction()?;
        store_summary_chunk_on(&transaction, doc_id, summary, key_points, tags)?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{CompletionResponse, FinishReason, Usage};
    use crate::sources::CreateSourceInput;

    struct RecordingCompiler {
        seen: std::sync::Mutex<Vec<String>>,
        change: Option<(Database, String)>,
    }
    #[async_trait::async_trait]
    impl LlmProvider for RecordingCompiler {
        fn name(&self) -> &str {
            "recording"
        }
        async fn list_models(&self) -> Result<Vec<String>, CoreError> {
            Ok(vec!["test".into()])
        }
        async fn health_check(&self) -> Result<(), CoreError> {
            Ok(())
        }
        async fn stream_events(
            &self,
            _: &CompletionRequest,
        ) -> Result<futures::stream::BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError>
        {
            unreachable!()
        }
        async fn complete(
            &self,
            request: &CompletionRequest,
        ) -> Result<CompletionResponse, CoreError> {
            self.seen
                .lock()
                .unwrap()
                .push(serde_json::to_string(&request.messages).unwrap());
            if let Some((db, doc_id)) = &self.change {
                db.conn().execute(
                    "UPDATE documents SET content_hash='updated-while-compiling' WHERE id=?1",
                    [doc_id],
                )?;
            }
            StaticLlmProvider { content: r#"{"summary":"A bounded section summary.","key_points":[],"tags":[],"entities":[]}"#.into() }.complete(request).await
        }
    }

    #[tokio::test]
    async fn long_compilation_resumes_complete_coverage_with_a_shared_call_budget() {
        let db = Database::open_memory().unwrap();
        let content = format!("{}APPENDIX_REQUIRED", "中英混合Budget。".repeat(13_000));
        let doc_id = insert_compile_doc(&db, &content);
        let provider = RecordingCompiler {
            seen: Default::default(),
            change: None,
        };
        let first = compile_document(&db, &doc_id, &provider, "test", None)
            .await
            .unwrap();
        assert_eq!(first.sections_compiled, 8);
        assert!(!first.summary.coverage.complete);
        assert!(db
            .get_uncompiled_document_ids(20)
            .unwrap()
            .contains(&doc_id));
        let second = compile_document(&db, &doc_id, &provider, "test", None)
            .await
            .unwrap();
        assert!(second.summary.coverage.complete);
        assert_eq!(
            second.summary.coverage.covered_chars,
            content.chars().count()
        );
        assert_eq!(
            provider.seen.lock().unwrap().len(),
            second.summary.coverage.total_sections
        );
        assert!(provider
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|input| input.contains("APPENDIX_REQUIRED")));
        assert!(!db
            .get_uncompiled_document_ids(20)
            .unwrap()
            .contains(&doc_id));
    }

    #[tokio::test]
    async fn compilation_replaces_sections_from_older_compiler_contracts() {
        for stale_contract in ["unversioned", "schema", "prompt"] {
            let db = Database::open_memory().unwrap();
            let doc_id = insert_compile_doc(&db, "A document about a pending decision.");
            let provider = RecordingCompiler {
                seen: Default::default(),
                change: None,
            };
            compile_document(&db, &doc_id, &provider, "test", None)
                .await
                .unwrap();
            let route = provider.route_snapshot(&CompletionRequest {
                model: "test".into(),
                ..Default::default()
            });
            let previous_contract = match stale_contract {
                "unversioned" => serde_json::to_vec(&route).unwrap(),
                "schema" => serde_json::to_vec(&(0, COMPILE_SYSTEM_PROMPT, &route)).unwrap(),
                _ => serde_json::to_vec(&(
                    COMPILE_CONTRACT_VERSION,
                    "Previous compiler prompt",
                    &route,
                ))
                .unwrap(),
            };
            let previous_key = blake3::hash(&previous_contract).to_hex().to_string();
            db.conn().execute(
                "UPDATE document_section_compilations SET route_key=?2,output_json=?3 WHERE document_id=?1",
                rusqlite::params![doc_id, previous_key, r#"{"summary":"OUTDATED CONTRACT RESULT","key_points":[],"tags":[],"entities":[]}"#],
            ).unwrap();
            let refreshed = compile_document(&db, &doc_id, &provider, "test", None)
                .await
                .unwrap();
            assert_eq!(
                refreshed.sections_compiled, 1,
                "reused {stale_contract} section"
            );
            assert!(!refreshed
                .summary
                .summary
                .contains("OUTDATED CONTRACT RESULT"));
            let resumed = compile_document(&db, &doc_id, &provider, "test", None)
                .await
                .unwrap();
            assert_eq!(
                resumed.sections_compiled, 0,
                "current contract should resume from cache"
            );
            assert_eq!(resumed.summary.summary, refreshed.summary.summary);
            assert_eq!(provider.seen.lock().unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn document_change_during_model_call_cannot_commit_old_knowledge() {
        let db = Database::open_memory().unwrap();
        let doc_id = insert_compile_doc(&db, "A document about a pending decision.");
        let provider = RecordingCompiler {
            seen: Default::default(),
            change: Some((db.clone(), doc_id.clone())),
        };
        assert!(matches!(
            compile_document(&db, &doc_id, &provider, "test", None).await,
            Err(CoreError::Conflict(_))
        ));
        assert!(db.get_document_summary(&doc_id).unwrap().is_none());
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT COUNT(*) FROM document_section_compilations",
                    [],
                    |row| row.get::<_, usize>(0)
                )
                .unwrap(),
            0
        );
    }

    struct StaticLlmProvider {
        content: String,
    }

    #[async_trait::async_trait]
    impl LlmProvider for StaticLlmProvider {
        fn name(&self) -> &str {
            "static"
        }

        async fn list_models(&self) -> Result<Vec<String>, CoreError> {
            Ok(vec!["test-model".to_string()])
        }

        async fn complete(
            &self,
            request: &CompletionRequest,
        ) -> Result<CompletionResponse, CoreError> {
            // A reasoning model may need more tokens than the compact JSON
            // it eventually returns. Exercise that shared output allowance.
            if request.max_tokens.is_some_and(|limit| limit < 8_000) {
                return Err(CoreError::Llm(
                    "reasoning exhausted the output allowance before document JSON".into(),
                ));
            }
            Ok(CompletionResponse {
                content: self.content.clone(),
                tool_calls: None,
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                thinking: None,
                provider_replay: None,
            })
        }

        async fn stream_events(
            &self,
            _request: &CompletionRequest,
        ) -> Result<futures::stream::BoxStream<'_, crate::llm::ProviderStreamEvent>, CoreError>
        {
            crate::llm::provider_events_from_chunk_stream(Box::pin(futures::stream::empty()))
        }

        async fn health_check(&self) -> Result<(), CoreError> {
            Ok(())
        }
    }

    fn insert_compile_doc(db: &Database, content: &str) -> String {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = db
            .add_source(CreateSourceInput {
                root_path: dir.path().to_string_lossy().to_string(),
                include_globs: vec![],
                exclude_globs: vec![],
                watch_enabled: true,
            })
            .expect("add source");
        let doc_id = uuid::Uuid::new_v4().to_string();
        let doc_path = dir.path().join("chapter.md").to_string_lossy().to_string();
        db.conn()
            .execute(
                "INSERT INTO documents (id, source_id, path, title, mime_type, file_size, modified_at, content_hash)
                 VALUES (?1, ?2, ?3, 'Chapter', 'text/markdown', 100, datetime('now'), 'doc-hash')",
                rusqlite::params![doc_id, source.id, doc_path],
            )
            .expect("insert document");
        db.conn()
            .execute(
                "INSERT INTO chunks (id, document_id, chunk_index, kind, content, start_offset, end_offset, line_start, line_end, content_hash)
                 VALUES (?1, ?2, 0, 'text', ?3, 0, ?4, 1, 1, 'chunk-hash')",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    doc_id,
                    content,
                    content.len() as i64
                ],
            )
            .expect("insert chunk");
        doc_id
    }

    #[tokio::test]
    async fn compile_document_links_relations_to_entities_later_in_same_output() {
        let db = Database::open_memory().expect("open memory");
        let doc_id = insert_compile_doc(&db, "Princess meets Dragon.");
        let provider = StaticLlmProvider {
            content: serde_json::json!({
                "summary": "A princess meets a dragon.",
                "key_points": ["Princess and Dragon are in the same scene."],
                "tags": ["novel"],
                "entities": [
                    {
                        "name": "Princess",
                        "entity_type": "person",
                        "description": "A protagonist.",
                        "context": "Princess meets Dragon.",
                        "relations": [
                            { "target": "Dragon", "relation_type": "meets", "evidence": "Princess meets Dragon." }
                        ]
                    },
                    {
                        "name": "Dragon",
                        "entity_type": "person",
                        "description": "A rival.",
                        "context": "Princess meets Dragon.",
                        "relations": []
                    }
                ]
            })
            .to_string(),
        };

        let result = compile_document(&db, &doc_id, &provider, "test-model", None)
            .await
            .expect("compile document");

        assert_eq!(result.entities_found, 2);
        assert_eq!(result.links_created, 1);
        let document_entities: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM document_entities", [], |row| {
                row.get(0)
            })
            .expect("document entity count");
        assert_eq!(document_entities, 2);
        let entity_links: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM entity_links", [], |row| row.get(0))
            .expect("entity link count");
        assert_eq!(entity_links, 1);
    }

    #[tokio::test]
    async fn compile_document_resolves_aliases_and_stores_relation_evidence() {
        let db = Database::open_memory().expect("open memory");
        let doc_id = insert_compile_doc(&db, "PKCE protects OAuth login.");
        let provider = StaticLlmProvider {
            content: serde_json::json!({
                "summary": "PKCE protects OAuth login.",
                "key_points": ["Proof Key for Code Exchange is used by OAuth."],
                "tags": ["security"],
                "entities": [
                    {
                        "name": "Proof Key for Code Exchange",
                        "aliases": ["PKCE"],
                        "entity_type": "technology",
                        "description": "An OAuth security extension.",
                        "context": "PKCE protects OAuth login.",
                        "relations": [
                            {
                                "target": "OAuth",
                                "relation_type": "protects",
                                "evidence": "PKCE protects OAuth login",
                                "confidence": 0.92
                            }
                        ]
                    },
                    {
                        "name": "OAuth",
                        "entity_type": "technology",
                        "description": "An authorization protocol.",
                        "context": "PKCE protects OAuth login.",
                        "relations": []
                    }
                ]
            })
            .to_string(),
        };

        let result = compile_document(&db, &doc_id, &provider, "test-model", None)
            .await
            .expect("compile document");

        assert_eq!(result.entities_found, 2);
        let pkce = db.find_entity_by_name("PKCE").expect("alias lookup");
        assert_eq!(pkce.name, "Proof Key for Code Exchange");
        let (snippet, confidence): (String, f64) = db
            .conn()
            .query_row(
                "SELECT evidence_snippet, confidence FROM entity_links WHERE relation_type = 'protects'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("relation evidence");
        assert_eq!(snippet, "PKCE protects OAuth login");
        assert!((confidence - 0.92).abs() < f64::EPSILON);
    }
    #[tokio::test]
    async fn a_relation_keeps_other_document_support_after_one_revision_changes() {
        let db = Database::open_memory().unwrap();
        let first = insert_compile_doc(&db, "PKCE protects OAuth login.");
        let second = insert_compile_doc(&db, "PKCE protects OAuth login.");
        let provider=StaticLlmProvider {content:serde_json::json!({"summary":"PKCE protects OAuth login.","key_points":[],"tags":[],"entities":[
            {"name":"PKCE","entity_type":"concept","description":"","context":"PKCE protects OAuth login.","relations":[{"target":"OAuth","relation_type":"protects","evidence":"PKCE protects OAuth login."}]},
            {"name":"OAuth","entity_type":"concept","description":"","context":"PKCE protects OAuth login.","relations":[]}
        ]}).to_string()};
        for document in [&first, &second, &first] {
            compile_document(&db, document, &provider, "fixture", None)
                .await
                .unwrap();
        }
        assert_eq!(
            db.conn()
                .query_row("SELECT COUNT(*) FROM entity_link_support", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.conn()
                .query_row("SELECT MIN(mention_count) FROM entities", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        db.conn()
            .execute(
                "UPDATE documents SET content_hash='changed' WHERE id=?1",
                [&first],
            )
            .unwrap();
        assert_eq!(
            db.conn()
                .query_row("SELECT evidence_doc_id FROM entity_links", [], |row| row
                    .get::<_, String>(
                    0
                ))
                .unwrap(),
            second
        );
        db.conn()
            .execute("DELETE FROM documents WHERE id=?1", [&second])
            .unwrap();
        assert_eq!(
            db.conn()
                .query_row("SELECT COUNT(*) FROM entity_links", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
