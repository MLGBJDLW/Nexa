//! Stable evidence identity and source-native locations. A historical reference
//! resolves to its stored text, never to new contents at an unchanged file path.
use crate::{db::Database, error::CoreError, models::EvidenceCard};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceContext {
    pub reference: EvidenceRef,
    pub cards: Vec<EvidenceCard>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSection {
    pub reference: EvidenceRef,
    pub chunk_index: i64,
    pub heading: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentOutline {
    pub sections: Vec<DocumentSection>,
    pub has_more: bool,
    pub next_index: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EvidenceLocator {
    Text {
        byte_start: u64,
        byte_end: u64,
        line_start: u32,
        line_end: u32,
    },
    Pdf {
        page: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bbox: Option<[f32; 4]>,
    },
    Document {
        part: String,
        paragraph: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        table: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        row: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        column: Option<u32>,
    },
    Sheet {
        sheet: String,
        range: String,
    },
    Slide {
        slide: u32,
    },
    Media {
        start_ms: i64,
        end_ms: i64,
    },
    Extracted {
        section: String,
    },
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRef {
    pub source_id: Uuid,
    pub document_id: Uuid,
    pub revision: String,
    pub document_hash: String,
    pub block_id: Uuid,
    pub content_hash: String,
    pub locator: EvidenceLocator,
    pub extraction_method: String,
    /// `current`, `historical`, or `missing` (file removed, source retained).
    pub status: String,
}

pub(crate) fn from_record(
    card: &crate::models::EvidenceCard,
    revision: String,
    document_hash: String,
    content_hash: String,
    metadata: &str,
    kind: &str,
    status: String,
) -> EvidenceRef {
    let metadata: serde_json::Value = serde_json::from_str(metadata).unwrap_or_default();
    EvidenceRef {
        source_id: card.source_id,
        document_id: card.document_id,
        revision,
        document_hash,
        block_id: card.chunk_id,
        content_hash,
        locator: metadata
            .get("locator")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default(),
        extraction_method: metadata
            .get("extraction_method")
            .and_then(|value| value.as_str())
            .unwrap_or(if kind == "summary" {
                "model_summary"
            } else {
                "legacy"
            })
            .to_string(),
        status,
    }
}

pub(crate) fn hydrate_references(
    db: &crate::db::Database,
    cards: &mut [crate::models::EvidenceCard],
) -> Result<(), crate::error::CoreError> {
    if cards.is_empty() {
        return Ok(());
    }
    let ids = cards
        .iter()
        .map(|card| card.chunk_id.to_string())
        .collect::<Vec<_>>();
    let placeholders = vec!["?"; ids.len()].join(",");
    let conn = db.conn();
    let mut statement = conn.prepare(&format!("SELECT chunk_id,revision,block_hash,metadata_json,kind,document_hash FROM evidence_records WHERE status='current' AND chunk_id IN ({placeholders})"))?;
    let records = statement
        .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let records = records
        .into_iter()
        .map(|(id, revision, hash, metadata, kind, document_hash)| {
            (id, (revision, hash, metadata, kind, document_hash))
        })
        .collect::<std::collections::HashMap<_, _>>();
    for card in cards {
        if let Some((revision, hash, metadata, kind, document_hash)) =
            records.get(&card.chunk_id.to_string())
        {
            card.evidence_ref = Some(from_record(
                card,
                revision.clone(),
                document_hash.clone(),
                hash.clone(),
                metadata,
                kind,
                "current".into(),
            ));
        }
    }
    Ok(())
}

pub fn context(
    db: &Database,
    reference: &EvidenceRef,
    radius: usize,
) -> Result<EvidenceContext, CoreError> {
    let target = crate::search::resolve_evidence_ref(db, reference)?;
    let reference = target
        .evidence_ref
        .clone()
        .ok_or_else(|| CoreError::NotFound("Evidence reference unavailable".into()))?;
    let radius = radius.min(5) as i64;
    let rows = {
        let conn = db.conn();
        let mut statement = conn.prepare("SELECT chunk_id,block_hash,chunk_index FROM evidence_records WHERE source_id=?1 AND document_id=?2 AND revision=?3 AND chunk_index BETWEEN ?4 AND ?5 ORDER BY chunk_index,(chunk_id=?6) DESC,(status='current') DESC,archived_at DESC LIMIT 128")?;
        let rows = statement
            .query_map(
                rusqlite::params![
                    reference.source_id.to_string(),
                    reference.document_id.to_string(),
                    reference.revision,
                    target.chunk_index - radius,
                    target.chunk_index + radius,
                    reference.block_id.to_string()
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let mut seen = std::collections::HashSet::new();
    let mut cards = Vec::new();
    let mut truncated = false;
    for (id, hash, index) in rows {
        if !seen.insert(index) {
            continue;
        }
        let mut neighbor = reference.clone();
        neighbor.block_id =
            Uuid::parse_str(&id).map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        neighbor.content_hash = hash;
        let mut card = crate::search::resolve_evidence_ref(db, &neighbor)?;
        let chars = card.content.chars().count();
        let allowance = if card.chunk_id == reference.block_id {
            32_000
        } else {
            32_000 / (radius as usize * 2).max(1)
        };
        if chars > allowance {
            card.content = card.content.chars().take(allowance).collect();
            card.snippet = None;
            truncated = true;
        }
        cards.push(card);
    }
    Ok(EvidenceContext {
        reference,
        cards,
        truncated,
    })
}

pub fn outline(
    db: &Database,
    reference: &EvidenceRef,
    after_index: Option<i64>,
) -> Result<DocumentOutline, CoreError> {
    let card = crate::search::resolve_evidence_ref(db, reference)?;
    let reference = card
        .evidence_ref
        .as_ref()
        .ok_or_else(|| CoreError::NotFound("Evidence reference unavailable".into()))?;
    let conn = db.conn();
    let mut statement = conn.prepare("SELECT chunk_id,block_hash,metadata_json,kind,status,chunk_index FROM evidence_records WHERE source_id=?1 AND document_id=?2 AND revision=?3 AND chunk_index>?4 ORDER BY chunk_index,(status='current') DESC,archived_at DESC LIMIT 1608")?;
    let rows = statement
        .query_map(
            rusqlite::params![
                reference.source_id.to_string(),
                reference.document_id.to_string(),
                reference.revision,
                after_index.unwrap_or(-1)
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen = std::collections::HashSet::new();
    let mut sections = Vec::new();
    for (id, hash, metadata, kind, status, index) in rows {
        if !seen.insert(index) {
            continue;
        }
        let meta: serde_json::Value = serde_json::from_str(&metadata)?;
        let mut evidence = from_record(
            &card,
            reference.revision.clone(),
            reference.document_hash.clone(),
            hash,
            &metadata,
            &kind,
            status,
        );
        evidence.block_id =
            Uuid::parse_str(&id).map_err(|error| CoreError::InvalidInput(error.to_string()))?;
        sections.push(DocumentSection {
            reference: evidence,
            chunk_index: index,
            heading: meta
                .get("heading_context")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        });
        if sections.len() == 201 {
            break;
        }
    }
    let has_more = sections.len() > 200;
    sections.truncate(200);
    let next_index = has_more.then(|| sections.last().unwrap().chunk_index);
    Ok(DocumentOutline {
        sections,
        has_more,
        next_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::Database,
        models::{SearchFilters, SearchQuery},
        parse::parse_file,
        search::{resolve_evidence_ref, search},
        sources::CreateSourceInput,
    };

    #[test]
    fn updates_keep_exact_historical_evidence_and_source_deletion_revokes_it() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("policy.md");
        std::fs::write(&path, "# 差旅\n报销上限是500元。\n").unwrap();
        let db = Database::open_memory().unwrap();
        let source = db
            .add_source(CreateSourceInput {
                root_path: folder.path().to_string_lossy().into(),
                include_globs: vec!["**/*.md".into()],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        let parse = || {
            parse_file(
                &path,
                None,
                #[cfg(feature = "video")]
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let document_id = db.insert_document(&source.id, &parse()).unwrap();
        let query = SearchQuery {
            text: "报销上限".into(),
            filters: SearchFilters::default(),
            limit: 10,
            offset: 0,
        };
        let first = search(&db, &query).unwrap().evidence_cards.remove(0);
        let reference = first.evidence_ref.unwrap();
        assert!(matches!(
            reference.locator,
            EvidenceLocator::Text { line_start: 1, .. }
        ));
        std::fs::write(&path, "# 差旅\n报销上限改为600元。\n").unwrap();
        db.update_document(&document_id, &parse()).unwrap();
        let historical = resolve_evidence_ref(&db, &reference).unwrap();
        assert!(historical.content.contains("500元"));
        assert_eq!(historical.evidence_ref.unwrap().status, "historical");
        let current = search(&db, &query).unwrap().evidence_cards.remove(0);
        assert!(current.content.contains("600元"));
        assert_ne!(
            current.evidence_ref.as_ref().unwrap().revision,
            reference.revision
        );
        let mut forged = reference.clone();
        forged.source_id = Uuid::new_v4();
        assert!(resolve_evidence_ref(&db, &forged).is_err());
        db.conn()
            .execute("DELETE FROM documents WHERE id=?1", [&document_id])
            .unwrap();
        assert_eq!(
            resolve_evidence_ref(&db, current.evidence_ref.as_ref().unwrap())
                .unwrap()
                .evidence_ref
                .unwrap()
                .status,
            "missing"
        );
        db.conn()
            .execute("DELETE FROM sources WHERE id=?1", [&source.id])
            .unwrap();
        assert!(resolve_evidence_ref(&db, &reference).is_err());
        assert!(db.integrity_check().unwrap());
    }

    #[test]
    fn overlapping_sources_force_reindex_and_claims_keep_independent_revisions() {
        let folder = tempfile::tempdir().unwrap();
        let child = folder.path().join("nested");
        std::fs::create_dir(&child).unwrap();
        let path = child.join("policy.md");
        std::fs::write(&path, "报销标准为500元。").unwrap();
        let db = Database::open_memory().unwrap();
        let sources = [folder.path(), child.as_path()].map(|root| {
            db.add_source(CreateSourceInput {
                root_path: root.to_string_lossy().into(),
                include_globs: vec!["**/*.md".into()],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap()
        });
        for source in &sources {
            crate::ingest::scan_source(&db, &source.id).unwrap();
        }
        let query = SearchQuery {
            text: "报销标准".into(),
            filters: Default::default(),
            limit: 10,
            offset: 0,
        };
        let cards = search(&db, &query).unwrap().evidence_cards;
        assert_eq!(cards.len(), 2);
        let parent = cards
            .iter()
            .find(|card| card.source_id.to_string() == sources[0].id)
            .unwrap()
            .evidence_ref
            .clone()
            .unwrap();
        let nested = cards
            .iter()
            .find(|card| card.source_id.to_string() == sources[1].id)
            .unwrap()
            .evidence_ref
            .clone()
            .unwrap();
        let claim=db.create_knowledge_claim(None,&serde_json::from_value(serde_json::json!({"subject":"lodging","predicate":"limit","object":"500","sourceRef":nested.block_id.to_string(),"sourceExcerpt":"500元","reviewState":"accepted"})).unwrap()).unwrap();
        crate::ingest::reindex_single_file(&db, &sources[1].id, &path).unwrap();
        assert_eq!(
            resolve_evidence_ref(&db, &parent)
                .unwrap()
                .evidence_ref
                .unwrap()
                .status,
            "current"
        );
        assert_eq!(
            resolve_evidence_ref(&db, &nested)
                .unwrap()
                .evidence_ref
                .unwrap()
                .status,
            "historical"
        );
        assert_eq!(
            db.get_knowledge_claim(&claim.id).unwrap().review_state,
            "needs_review"
        );
        let current = search(&db, &query)
            .unwrap()
            .evidence_cards
            .into_iter()
            .find(|card| card.source_id.to_string() == sources[1].id)
            .unwrap();
        assert_eq!(current.document_id, nested.document_id);
        assert_ne!(current.evidence_ref.unwrap().revision, nested.revision);
        let health = db.source_index_health().unwrap();
        assert_eq!(health.len(), 2);
        assert!(health.iter().all(|item| item.documents == 1
            && item.chunks == item.keyword_chunks
            && item.needs_reparse == 0));
    }
    #[test]
    fn later_exclusions_revoke_deleted_file_archives_and_saved_research() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("confidential.md");
        std::fs::write(&path, "Confidential budget is 500 yuan.").unwrap();
        let db = Database::open_memory().unwrap();
        let source = db
            .add_source(CreateSourceInput {
                root_path: dir.path().to_string_lossy().into(),
                include_globs: vec!["**/*.md".into()],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        crate::ingest::scan_source(&db, &source.id).unwrap();
        let reference = search(
            &db,
            &SearchQuery {
                text: "budget".into(),
                filters: Default::default(),
                limit: 2,
                offset: 0,
            },
        )
        .unwrap()
        .evidence_cards
        .remove(0)
        .evidence_ref
        .unwrap();
        let set = crate::research_workspace::create(
            &db,
            crate::research_workspace::CreateResearchSet {
                title: "Budget".into(),
                questions: vec!["budget".into()],
                documents: vec![reference.clone()],
            },
        )
        .unwrap();
        std::fs::remove_file(&path).unwrap();
        crate::ingest::scan_source(&db, &source.id).unwrap();
        assert_eq!(
            resolve_evidence_ref(&db, &reference)
                .unwrap()
                .evidence_ref
                .unwrap()
                .status,
            "missing"
        );
        let claim=db.create_knowledge_claim(None,&serde_json::from_value(serde_json::json!({"subject":"Budget","predicate":"is","object":"500 yuan","sourceRef":reference.block_id.to_string(),"reviewState":"accepted"})).unwrap()).unwrap();
        assert_eq!(claim.review_state, "needs_review");
        let bound:(String,String,i64)=db.conn().query_row("SELECT document_id,document_revision,stale FROM knowledge_evidence WHERE claim_id=?1",[&claim.id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(
            bound,
            (
                reference.document_id.to_string(),
                reference.revision.clone(),
                1
            )
        );
        let mut privacy = db.load_privacy_config().unwrap();
        privacy.exclude_patterns.push("**/confidential.md".into());
        db.save_privacy_config(&privacy).unwrap();
        let scan = crate::ingest::scan_source(&db, &source.id).unwrap();
        assert_eq!(scan.files_purged, 1);
        assert!(resolve_evidence_ref(&db, &reference).is_err());
        assert!(crate::research_workspace::get(&db, &set.summary.id)
            .unwrap()
            .documents
            .is_empty());
    }
}
