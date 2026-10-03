//! Saved, bounded document-by-question comparisons with immutable evidence.
//! Retrieval proposes excerpts; support/conflict judgments remain reviewable.
use crate::{
    db::Database,
    error::CoreError,
    evidence::EvidenceRef,
    models::{EvidenceCard, SearchFilters, SearchQuery},
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateResearchSet {
    pub title: String,
    pub questions: Vec<String>,
    pub documents: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSetSummary {
    pub id: String,
    pub title: String,
    pub revision: u64,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSet {
    pub summary: ResearchSetSummary,
    pub questions: Vec<String>,
    pub documents: Vec<ResearchDocument>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchDocument {
    pub reference: EvidenceRef,
    pub title: String,
    pub path: String,
    pub cells: Vec<ResearchCell>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchCell {
    pub question_index: usize,
    pub review_state: String,
    pub note: String,
    pub stale: bool,
    pub evidence: Vec<EvidenceCard>,
}

fn changed(conn: &rusqlite::Connection, id: &str, revision: u64) -> Result<(), CoreError> {
    if conn.execute("UPDATE research_sets SET revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND revision=?2",params![id,revision])? != 1 {
        return Err(CoreError::Conflict("Research set changed; reload before saving".into()));
    }
    Ok(())
}
pub fn create(db: &Database, input: CreateResearchSet) -> Result<ResearchSet, CoreError> {
    if input.title.trim().is_empty()
        || input.title.chars().count() > 120
        || input.documents.is_empty()
        || input.documents.len() > 8
        || input.questions.is_empty()
        || input.questions.len() > 6
        || input
            .questions
            .iter()
            .any(|query| query.trim().is_empty() || query.chars().count() > 400)
    {
        return Err(CoreError::InvalidInput(
            "Research requires a title, 1–8 documents and 1–6 questions (400 characters each)"
                .into(),
        ));
    }
    let mut documents = Vec::new();
    let mut seen = HashSet::new();
    for reference in input.documents {
        let card = crate::search::resolve_evidence_ref(db, &reference)?;
        if card
            .evidence_ref
            .as_ref()
            .is_none_or(|reference| reference.status != "current")
        {
            return Err(CoreError::Conflict(
                "Select the current document version to create a research set".into(),
            ));
        }
        if seen.insert(reference.document_id) {
            documents.push(card);
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let mut conn = db.conn();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO research_sets(id,title,questions_json) VALUES(?1,?2,?3)",
        params![
            id,
            input.title.trim(),
            serde_json::to_string(&input.questions)?
        ],
    )?;
    for card in documents {
        let reference = card.evidence_ref.as_ref().expect("resolved evidence");
        let current: String = tx.query_row(
            "SELECT index_revision FROM documents WHERE id=?1 AND source_id=?2",
            params![card.document_id.to_string(), card.source_id.to_string()],
            |row| row.get(0),
        )?;
        if current != reference.revision {
            return Err(CoreError::Conflict(
                "Document changed while creating research".into(),
            ));
        }
        tx.execute("INSERT INTO research_documents(set_id,document_id,source_id,title,path,reference_json) VALUES(?1,?2,?3,?4,?5,?6)",params![id,card.document_id.to_string(),card.source_id.to_string(),card.document_title,card.document_path,serde_json::to_string(reference)?])?;
        for index in 0..input.questions.len() {
            tx.execute(
                "INSERT INTO research_cells(set_id,document_id,question_index) VALUES(?1,?2,?3)",
                params![id, card.document_id.to_string(), index],
            )?;
        }
    }
    tx.commit()?;
    drop(conn);
    get(db, &id)
}
pub fn list(db: &Database) -> Result<Vec<ResearchSetSummary>, CoreError> {
    let conn = db.conn();
    let mut statement = conn.prepare(
        "SELECT id,title,revision,updated_at FROM research_sets ORDER BY updated_at DESC LIMIT 100",
    )?;
    let result = statement
        .query_map([], |row| {
            Ok(ResearchSetSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                revision: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(result)
}
pub fn get(db: &Database, id: &str) -> Result<ResearchSet, CoreError> {
    let (summary, questions, documents) = {
        let conn = db.conn();
        let (summary, questions) = conn.query_row(
            "SELECT id,title,revision,updated_at,questions_json FROM research_sets WHERE id=?1",
            [id],
            |row| {
                Ok((
                    ResearchSetSummary {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        revision: row.get(2)?,
                        updated_at: row.get(3)?,
                    },
                    row.get::<_, String>(4)?,
                ))
            },
        )?;
        let mut statement=conn.prepare("SELECT reference_json,title,path FROM research_documents WHERE set_id=?1 ORDER BY title,document_id")?;
        let documents = statement
            .query_map([id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        (summary, serde_json::from_str(&questions)?, documents)
    };
    let mut result = ResearchSet {
        summary,
        questions,
        documents: Vec::new(),
    };
    for (reference, title, path) in documents {
        let reference: EvidenceRef = serde_json::from_str(&reference)?;
        let (current, cells) = {
            let conn = db.conn();
            let current: Option<String> = conn
                .query_row(
                    "SELECT index_revision FROM documents WHERE id=?1 AND source_id=?2",
                    params![
                        reference.document_id.to_string(),
                        reference.source_id.to_string()
                    ],
                    |row| row.get(0),
                )
                .optional()?;
            let mut statement=conn.prepare("SELECT question_index,document_revision,review_state,note,evidence_json FROM research_cells WHERE set_id=?1 AND document_id=?2 ORDER BY question_index")?;
            let cells = statement
                .query_map(params![id, reference.document_id.to_string()], |row| {
                    Ok((
                        row.get::<_, usize>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            (current, cells)
        };
        let mut materialized = Vec::new();
        for (index, revision, state, note, refs) in cells {
            let refs: Vec<EvidenceRef> = serde_json::from_str(&refs)?;
            let mut stale = current.as_deref() != Some(revision.as_str()) && state != "pending";
            let mut evidence = Vec::new();
            for reference in refs {
                match crate::search::resolve_evidence_ref(db, &reference) {
                    Ok(mut card) => {
                        card.content = card.content.chars().take(1600).collect();
                        evidence.push(card);
                    }
                    Err(_) => stale = true,
                }
            }
            materialized.push(ResearchCell {
                question_index: index,
                review_state: state,
                note,
                stale,
                evidence,
            });
        }
        result.documents.push(ResearchDocument {
            reference,
            title,
            path,
            cells: materialized,
        });
    }
    Ok(result)
}

/// At most twelve retrievals and thirty seconds of scheduling per action.
/// Each completed cell is committed independently; retry resumes unfinished cells.
pub fn refresh(
    db: &Database,
    id: &str,
    progress: impl Fn(usize, usize),
) -> Result<ResearchSet, CoreError> {
    let set = get(db, id)?;
    let start = std::time::Instant::now();
    let mut calls = 0;
    let total = set.documents.len() * set.questions.len();
    for document in set.documents {
        let current = {
            let conn = db.conn();
            conn.query_row("SELECT c.id FROM chunks c JOIN documents d ON d.id=c.document_id WHERE d.id=?1 AND d.source_id=?2 AND c.kind!='summary' ORDER BY c.chunk_index LIMIT 1",params![document.reference.document_id.to_string(),document.reference.source_id.to_string()],|row|row.get::<_,String>(0)).optional()?
        };
        let Some(current) = current else { continue };
        let card = crate::search::get_evidence_card(db, &current)?;
        let reference = card
            .evidence_ref
            .ok_or_else(|| CoreError::InvalidInput("Document has no evidence reference".into()))?;
        for cell in document.cells {
            if cell.review_state != "pending" && !cell.stale {
                continue;
            }
            if calls >= 12 || start.elapsed().as_secs() >= 30 {
                return get(db, id);
            }
            let query = SearchQuery {
                text: set.questions[cell.question_index].clone(),
                filters: SearchFilters {
                    source_ids: vec![reference.source_id],
                    document_ids: vec![reference.document_id],
                    ..Default::default()
                },
                limit: 3,
                offset: 0,
            };
            let result = crate::search::research_search(db, &query)?;
            calls += 1;
            let refs: Vec<EvidenceRef> = result
                .evidence_cards
                .iter()
                .filter_map(|card| card.evidence_ref.clone())
                .collect();
            let mut conn = db.conn();
            let tx = conn.transaction()?;
            let current: Option<String> = tx
                .query_row(
                    "SELECT index_revision FROM documents WHERE id=?1 AND source_id=?2",
                    params![
                        reference.document_id.to_string(),
                        reference.source_id.to_string()
                    ],
                    |row| row.get(0),
                )
                .optional()?;
            if current.as_deref() != Some(reference.revision.as_str())
                || refs.iter().any(|item| item.revision != reference.revision)
            {
                return Err(CoreError::Conflict(
                    "Document changed during research; retry to read its current version".into(),
                ));
            }
            tx.execute("UPDATE research_documents SET reference_json=?3 WHERE set_id=?1 AND document_id=?2",params![id,reference.document_id.to_string(),serde_json::to_string(&reference)?])?;
            tx.execute("UPDATE research_cells SET document_revision=?4,review_state=?5,evidence_json=?6 WHERE set_id=?1 AND document_id=?2 AND question_index=?3",params![id,reference.document_id.to_string(),cell.question_index,reference.revision,if refs.is_empty(){"not_found"}else{"needs_review"},serde_json::to_string(&refs)?])?;
            tx.execute("UPDATE research_sets SET revision=revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1",[id])?;
            tx.commit()?;
            drop(conn);
            progress(calls, total);
        }
    }
    get(db, id)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewResearchCell {
    pub set_id: String,
    pub document_id: String,
    pub question_index: usize,
    pub expected_revision: u64,
    pub review_state: String,
    pub note: String,
}
pub fn review(db: &Database, input: ReviewResearchCell) -> Result<ResearchSet, CoreError> {
    if !matches!(
        input.review_state.as_str(),
        "needs_review" | "supported" | "not_found" | "conflict"
    ) || input.note.chars().count() > 4000
    {
        return Err(CoreError::InvalidInput(
            "Invalid research review or note length".into(),
        ));
    }
    let mut conn = db.conn();
    let tx = conn.transaction()?;
    let (revision,refs):(String,String)=tx.query_row("SELECT document_revision,evidence_json FROM research_cells WHERE set_id=?1 AND document_id=?2 AND question_index=?3",params![input.set_id,input.document_id,input.question_index],|row|Ok((row.get(0)?,row.get(1)?)))?;
    let current: Option<String> = tx
        .query_row(
            "SELECT index_revision FROM documents WHERE id=?1",
            [&input.document_id],
            |row| row.get(0),
        )
        .optional()?;
    if current.as_deref() != Some(&revision) {
        return Err(CoreError::Conflict(
            "Refresh changed evidence before reviewing this cell".into(),
        ));
    }
    if matches!(input.review_state.as_str(), "supported" | "conflict")
        && serde_json::from_str::<Vec<EvidenceRef>>(&refs)?.is_empty()
    {
        return Err(CoreError::InvalidInput(
            "A supported or conflicting conclusion requires evidence".into(),
        ));
    }
    changed(&tx, &input.set_id, input.expected_revision)?;
    tx.execute("UPDATE research_cells SET review_state=?4,note=?5 WHERE set_id=?1 AND document_id=?2 AND question_index=?3",params![input.set_id,input.document_id,input.question_index,input.review_state,input.note])?;
    tx.commit()?;
    drop(conn);
    get(db, &input.set_id)
}
pub fn delete(db: &Database, id: &str) -> Result<(), CoreError> {
    db.conn()
        .execute("DELETE FROM research_sets WHERE id=?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ingest, sources::CreateSourceInput};
    #[test]
    fn matrix_retains_multiple_blocks_versions_reviews_and_source_revocation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.md");
        std::fs::write(
            &path,
            "# 住宿标准\n住宿标准为500元。\n\n# 审批\n住宿标准超额需要主管审批。\n",
        )
        .unwrap();
        let db = Database::open_memory().unwrap();
        let source = db
            .add_source(CreateSourceInput {
                root_path: dir.path().to_string_lossy().into(),
                include_globs: vec!["**/*.md".into()],
                exclude_globs: vec![],
                watch_enabled: false,
            })
            .unwrap();
        ingest::scan_source(&db, &source.id).unwrap();
        let query = SearchQuery {
            text: "住宿标准".into(),
            filters: Default::default(),
            limit: 5,
            offset: 0,
        };
        let card = crate::search::search(&db, &query)
            .unwrap()
            .evidence_cards
            .remove(0);
        let reference = card.evidence_ref.unwrap();
        let created = create(
            &db,
            CreateResearchSet {
                title: "规则对照".into(),
                questions: vec!["住宿标准".into(), "Unanswerable zebrafish".into()],
                documents: vec![reference.clone()],
            },
        )
        .unwrap();
        assert_eq!(created.documents[0].cells[0].review_state, "pending");
        let retrieved = refresh(&db, &created.summary.id, |_, _| {}).unwrap();
        assert_eq!(retrieved.documents[0].cells[0].evidence.len(), 2);
        assert_eq!(retrieved.documents[0].cells[1].review_state, "not_found");
        let reviewed = review(
            &db,
            ReviewResearchCell {
                set_id: created.summary.id.clone(),
                document_id: reference.document_id.to_string(),
                question_index: 0,
                expected_revision: retrieved.summary.revision,
                review_state: "supported".into(),
                note: "500元；超额须审批。".into(),
            },
        )
        .unwrap();
        assert_eq!(reviewed.documents[0].cells[0].review_state, "supported");
        assert!(review(
            &db,
            ReviewResearchCell {
                set_id: created.summary.id.clone(),
                document_id: reference.document_id.to_string(),
                question_index: 0,
                expected_revision: retrieved.summary.revision,
                review_state: "supported".into(),
                note: "old concurrent edit".into()
            }
        )
        .is_err());
        std::fs::write(&path, "# 住宿标准\n住宿标准改为600元。\n").unwrap();
        ingest::ingest_single_file(&db, &source.id, &path).unwrap();
        let changed = get(&db, &created.summary.id).unwrap();
        assert!(changed.documents[0].cells[0].stale);
        assert!(changed.documents[0].cells[0]
            .evidence
            .iter()
            .any(|card| card.content.contains("500元")));
        let updated = refresh(&db, &created.summary.id, |_, _| {}).unwrap();
        assert!(!updated.documents[0].cells[0].stale);
        assert_eq!(updated.documents[0].cells[0].review_state, "needs_review");
        assert!(updated.documents[0].cells[0].evidence[0]
            .content
            .contains("600元"));
        db.conn()
            .execute("DELETE FROM sources WHERE id=?1", [&source.id])
            .unwrap();
        assert!(get(&db, &created.summary.id).unwrap().documents.is_empty());
        assert_eq!(
            db.conn()
                .query_row("SELECT COUNT(*) FROM research_cells", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
