use super::{CoreError, Database};
use rusqlite::named_params;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryHit {
    pub id: String,
    pub title: String,
    pub message_id: Option<String>,
    pub message_order: Option<i64>,
    pub snippet: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySearchPage {
    pub items: Vec<HistoryHit>,
    pub next: Option<[String; 2]>,
}

impl Database {
    /// One visible message per conversation. Reuse the existing word index.
    /// unicode61 doesn't segment CJK or index punctuation, so those searches
    /// use literal substrings on visible message bodies.
    pub fn search_chat_history(
        &self,
        query: &str,
        archived: bool,
        before: Option<&[String; 2]>,
    ) -> Result<HistorySearchPage, CoreError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(HistorySearchPage {
                items: vec![],
                next: None,
            });
        }
        let indexed = query
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace())
            && query.split_whitespace().count() <= 32;
        let (source, predicate, terms) = if indexed {
            (
                "fts_messages JOIN messages m ON m.id = fts_messages.message_id",
                "fts_messages MATCH :terms",
                query
                    .split_whitespace()
                    .map(|word| format!("\"{word}\"*"))
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        } else {
            (
                "messages m",
                "instr(lower(m.content), lower(:terms)) > 0",
                query.to_owned(),
            )
        };
        let sql = format!(
            "WITH scope AS (
                SELECT id,title,updated_at FROM conversations
                WHERE parent_conversation_id IS NULL AND (archived_at IS NOT NULL) = :archived
                  AND (:date IS NULL OR (updated_at,id) < (:date,:id))
            ), hits AS (
                SELECT m.conversation_id, max(m.rowid) AS message_row
                FROM {source} JOIN scope s ON s.id = m.conversation_id
                WHERE {predicate} AND m.role IN ('user','assistant')
                  AND coalesce(m.display_artifact_kind,'') NOT IN ('checkpointContinuation','replayableRuntimeContext')
                GROUP BY m.conversation_id
            )
            SELECT s.id,coalesce(s.title,''),s.updated_at,m.id,m.sort_order,
                   substr(m.content, max(1,instr(lower(m.content),lower(:needle))-40), 150)
            FROM scope s LEFT JOIN hits h ON h.conversation_id=s.id
            LEFT JOIN messages m ON m.rowid=h.message_row
            WHERE h.message_row IS NOT NULL OR instr(lower(s.title),lower(:query)) > 0
            ORDER BY s.updated_at DESC,s.id DESC LIMIT 33"
        );
        let reader = self.read_only_lane()?;
        let conn = reader.conn();
        let mut stmt = conn.prepare_cached(&sql)?;
        let (date, id) = before
            .map(|[date, id]| (Some(date), Some(id)))
            .unwrap_or_default();
        let needle = if indexed {
            query.split_whitespace().next().unwrap_or(query)
        } else {
            query
        };
        let mut rows = stmt.query(named_params! {
            ":archived": archived, ":date": date, ":id": id,
            ":terms": terms, ":query": query, ":needle": needle,
        })?;
        let mut items = Vec::new();
        let mut last_date = String::new();
        let mut next = None;
        while let Some(row) = rows.next()? {
            if items.len() == 32 {
                next = items
                    .last()
                    .map(|hit: &HistoryHit| [last_date, hit.id.clone()]);
                break;
            }
            last_date = row.get(2)?;
            items.push(HistoryHit {
                id: row.get(0)?,
                title: row.get(1)?,
                message_id: row.get(3)?,
                message_order: row.get(4)?,
                snippet: row.get(5)?,
            });
        }
        Ok(HistorySearchPage { items, next })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_search_returns_visible_anchored_hits_and_pages_by_conversation() {
        let db = Database::open_memory().unwrap();
        db.conn().execute_batch("
            INSERT INTO conversations(id,title,provider,model) VALUES
                ('matched','Release','local','test'),('title','Needle title','local','test'),
                ('old','Old','local','test'),('child','Child','local','test');
            UPDATE conversations SET archived_at='2026-11-20' WHERE id='old';
            UPDATE conversations SET parent_conversation_id='matched' WHERE id='child';
            INSERT INTO messages(id,conversation_id,role,content,sort_order) VALUES
                ('m1','matched','user','needle in a haystack',1),
                ('m2','matched','assistant','another needle',2),
                ('m3','matched','user','我们需要查找这段历史，保留 100%_ 的原文',3),
                ('m4','child','user','hidden needle',1),('m5','old','assistant','archived needle',1),
                ('m6','title','tool','invisible tool content',1);
        ").unwrap();
        let hits = db.search_chat_history("need", false, None).unwrap().items;
        assert_eq!(hits.len(), 2);
        let body = hits.iter().find(|h| h.id == "matched").unwrap();
        assert_eq!(body.message_id.as_deref(), Some("m2"));
        assert!(hits
            .iter()
            .find(|h| h.id == "title")
            .unwrap()
            .message_id
            .is_none());
        for query in ["这段历史", "100%_"] {
            assert_eq!(
                db.search_chat_history(query, false, None).unwrap().items[0].message_order,
                Some(3)
            );
        }
        assert!(db
            .search_chat_history("invisible", false, None)
            .unwrap()
            .items
            .is_empty());
        assert_eq!(
            db.search_chat_history("needle", true, None).unwrap().items[0].id,
            "old"
        );
        db.conn().execute_batch("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<36)
            INSERT INTO conversations(id,title,provider,model) SELECT printf('page%03d',i),'paginate','local','test' FROM n").unwrap();
        let first = db.search_chat_history("paginate", false, None).unwrap();
        let second = db
            .search_chat_history("paginate", false, first.next.as_ref())
            .unwrap();
        assert_eq!((first.items.len(), second.items.len()), (32, 4));
        assert!(second.next.is_none());
        assert!(first
            .items
            .iter()
            .all(|a| second.items.iter().all(|b| a.id != b.id)));
    }
}
