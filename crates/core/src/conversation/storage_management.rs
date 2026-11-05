use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStorageCursor {
    pub updated_at: String,
    pub id: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStorageEntry {
    pub id: String,
    pub title: String,
    pub archived_at: Option<String>,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStoragePage {
    pub entries: Vec<ConversationStorageEntry>,
    pub next: Option<ConversationStorageCursor>,
}

pub(super) fn ensure_idle(conn: &rusqlite::Connection, ids: &[&str]) -> Result<(), CoreError> {
    for chunk in ids.chunks(CONVERSATION_DELETE_BIND_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!("WITH RECURSIVE selected(id) AS (
            SELECT id FROM conversations WHERE id IN ({placeholders}) UNION
            SELECT c.id FROM conversations c JOIN selected s ON c.parent_conversation_id=s.id)
            SELECT EXISTS(SELECT 1 FROM agent_task_runs r JOIN selected s ON r.conversation_id=s.id
              WHERE r.status IN ('queued','running','waiting_approval','awaiting_user_input','cancelling'))");
        let busy: bool = conn.query_row(&sql, rusqlite::params_from_iter(chunk.iter()), |row| {
            row.get(0)
        })?;
        if busy {
            return Err(CoreError::InvalidInput(
                "Stop running tasks before managing their conversations.".into(),
            ));
        }
    }
    Ok(())
}

impl Database {
    pub fn conversation_storage_page(
        &self,
        scope: &str,
        cursor: Option<&ConversationStorageCursor>,
    ) -> Result<ConversationStoragePage, CoreError> {
        if !matches!(scope, "active" | "archived" | "all") {
            return Err(CoreError::InvalidInput("Invalid conversation scope".into()));
        }
        let conn = self.conn();
        let mut query = conn.prepare("SELECT id,title,archived_at,updated_at FROM conversations WHERE parent_conversation_id IS NULL
            AND (?1='all' OR (?1='active' AND archived_at IS NULL) OR (?1='archived' AND archived_at IS NOT NULL))
            AND (?2 IS NULL OR updated_at < ?2 OR (updated_at=?2 AND id < ?3)) ORDER BY updated_at DESC,id DESC LIMIT 101")?;
        let mut entries = query
            .query_map(
                rusqlite::params![
                    scope,
                    cursor.map(|cursor| &cursor.updated_at),
                    cursor.map(|cursor| &cursor.id)
                ],
                |row| {
                    Ok(ConversationStorageEntry {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        archived_at: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = entries.len() > 100;
        entries.truncate(100);
        let next = more
            .then(|| {
                entries.last().map(|entry| ConversationStorageCursor {
                    id: entry.id.clone(),
                    updated_at: entry.updated_at.clone(),
                })
            })
            .flatten();
        Ok(ConversationStoragePage { entries, next })
    }

    pub fn archive_conversations_batch(&self, ids: &[String]) -> Result<usize, CoreError> {
        let ids = ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn();
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_idle(&transaction, &ids)?;
        let mut changed = 0;
        for chunk in ids.chunks(CONVERSATION_DELETE_BIND_CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            changed += transaction.execute(&format!("UPDATE conversations SET archived_at=datetime('now') WHERE archived_at IS NULL AND id IN ({placeholders})"), rusqlite::params_from_iter(chunk.iter()))?;
        }
        transaction.commit()?;
        Ok(changed)
    }
}
