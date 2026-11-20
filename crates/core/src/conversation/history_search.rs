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
        let indexed = query.chars().all(|ch| {
            ch.is_whitespace() || ch.is_alphanumeric() && !crate::lexical::is_unsegmented_script(ch)
        }) && query.split_whitespace().count() <= 32;
        let folded = query.to_lowercase();
        let visible = "m.role IN ('user','assistant')
            AND coalesce(m.display_artifact_kind,'') NOT IN ('checkpointContinuation','replayableRuntimeContext','questionResponse')
            AND (m.display_reasoning_candidate = 0 OR CASE WHEN json_valid(m.tool_calls_json) THEN json_array_length(m.tool_calls_json) ELSE 0 END > 0
              OR coalesce((SELECT t.display_trace_flags FROM conversation_turns t
                  WHERE t.conversation_id=m.conversation_id AND t.assistant_message_id=m.id ORDER BY t.created_at DESC,t.id DESC LIMIT 1),
                  m.display_legacy_trace_flags,0) <> 1)";
        let (matches, lookup, terms) = if indexed {
            (
                format!(", hits AS (
                    SELECT m.conversation_id, max(m.rowid) AS message_row
                    FROM fts_messages JOIN messages m ON m.id=fts_messages.message_id
                    JOIN scope s ON s.id=m.conversation_id
                    WHERE fts_messages MATCH :terms AND {visible} GROUP BY m.conversation_id
                )"),
                "LEFT JOIN hits h ON h.conversation_id=s.id LEFT JOIN messages hit ON hit.rowid=h.message_row".into(),
                query.split_whitespace().map(|word| format!("\"{word}\"*")).collect::<Vec<_>>().join(" "),
            )
        } else {
            // Walk the recency index, stopping as soon as a page is full. Each
            // conversation seeks its newest visible literal hit in timeline order.
            let content = if query.chars().any(|c| c.is_lowercase() || c.is_uppercase()) {
                "nexa_lower(m.content)"
            } else {
                "m.content"
            };
            (
                String::new(),
                format!(
                    "LEFT JOIN messages hit ON hit.rowid=(
                    SELECT m.rowid FROM messages m WHERE m.conversation_id=s.id
                    AND {visible} AND instr({content},:terms)>0
                    ORDER BY m.sort_order DESC,m.id DESC LIMIT 1
                )"
                ),
                folded.clone(),
            )
        };
        let sql = format!(
            "WITH scope AS (
                SELECT id,title,updated_at FROM conversations
                WHERE parent_conversation_id IS NULL AND (archived_at IS NOT NULL) = :archived
                  AND (:date IS NULL OR (updated_at,id) < (:date,:id))
            ) {matches}
            SELECT s.id,coalesce(s.title,''),s.updated_at,hit.id,hit.sort_order,
                   substr(hit.content, max(1,instr(nexa_lower(hit.content),:needle)-40), 150)
            FROM scope s {lookup}
            WHERE hit.id IS NOT NULL OR instr(nexa_lower(s.title),:query)>0
            ORDER BY s.updated_at DESC,s.id DESC LIMIT 33"
        );
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&sql)?;
        let (date, id) = before
            .map(|[date, id]| (Some(date), Some(id)))
            .unwrap_or_default();
        let needle = if indexed {
            folded.split_whitespace().next().unwrap_or(&folded)
        } else {
            &folded
        };
        let mut rows = stmt.query(named_params! {
            ":archived": archived, ":date": date, ":id": id,
            ":terms": terms, ":query": folded, ":needle": needle,
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
            INSERT INTO messages(id,conversation_id,role,content,sort_order,artifacts_json) VALUES
                ('control','matched','user','private 控制内容',4,'{\"kind\":\"questionResponse\"}'),
                ('steer','matched','user','visible steering 正文',5,'{\"kind\":\"steering\"}');
            INSERT INTO messages(id,conversation_id,role,content,thinking,sort_order) VALUES
                ('reason','matched','assistant','private 旧思考','private 旧思考',6);
            INSERT INTO conversation_turns(id,conversation_id,user_message_id,assistant_message_id,status,trace_json) VALUES
                ('turn','matched','m1','reason','completed','{\"kind\":\"turnTrace\",\"items\":[{\"kind\":\"thinking\",\"text\":\"private 旧思考\"}]}');
        ").unwrap();
        for query in ["private", "旧思考", "控制内容"] {
            assert!(db
                .search_chat_history(query, false, None)
                .unwrap()
                .items
                .is_empty());
        }
        assert_eq!(
            db.search_chat_history("steering", false, None)
                .unwrap()
                .items[0]
                .message_id
                .as_deref(),
            Some("steer")
        );
        db.conn()
            .execute(
                "INSERT INTO messages(id,conversation_id,role,content,sort_order)
            VALUES ('mixed','matched','assistant','使用PKCE保护OAuth登录',8)",
                [],
            )
            .unwrap();
        for query in ["PKCE", "oauth", "保护OAuth"] {
            let hit = db
                .search_chat_history(query, false, None)
                .unwrap()
                .items
                .remove(0);
            assert_eq!(hit.message_id.as_deref(), Some("mixed"), "{query}");
            assert_eq!(hit.snippet.as_deref(), Some("使用PKCE保护OAuth登录"));
        }
        // Pre-upgrade rows held the unsplit display text. Verify the migration
        // repairs that projection without changing the message or its anchor.
        {
            let conn = db.conn();
            conn.execute_batch(
                "UPDATE fts_messages SET content='使用PKCE保护OAuth登录' WHERE message_id='mixed';
                DROP INDEX idx_conversations_history_page;
                DELETE FROM _migrations WHERE name='v163_history_search';",
            )
            .unwrap();
            crate::migrations::run_migrations(&conn).unwrap();
        }
        assert_eq!(
            db.search_chat_history("PKCE", false, None).unwrap().items[0]
                .message_id
                .as_deref(),
            Some("mixed")
        );
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
        db.conn()
            .execute(
                "UPDATE conversations SET title='ÜBERBLICK • ОТЧЁТ' WHERE id='title'",
                [],
            )
            .unwrap();
        for query in ["über", "отчёт"] {
            assert_eq!(
                db.search_chat_history(query, false, None).unwrap().items[0].id,
                "title"
            );
        }
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
