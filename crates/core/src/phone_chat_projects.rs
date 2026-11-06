//! Phone-owned sidebar folders. A paired computer's chat IDs live in their
//! own namespace. Filing a chat never changes the workspace used to run it.
use std::collections::BTreeMap;

use rusqlite::{params, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::{db::Database, error::CoreError};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PhoneProjectAction {
    List,
    Create,
    Rename,
    Delete,
    Move,
}

#[derive(Debug, Serialize)]
pub struct PhoneChatProject {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct PhoneChatProjects {
    pub projects: Vec<PhoneChatProject>,
    pub memberships: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct PhoneProjectChat {
    pub id: String,
    pub title: String,
    pub model: String,
}

fn identifier(value: Option<&str>) -> Result<&str, CoreError> {
    value
        .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
        .ok_or_else(|| CoreError::InvalidInput("Invalid chat or project ID.".into()))
}

fn project_name(name: Option<&str>) -> Result<&str, CoreError> {
    name.map(str::trim)
        .filter(|name| {
            !name.is_empty() && name.chars().count() <= 100 && !name.chars().any(char::is_control)
        })
        .ok_or_else(|| {
            CoreError::InvalidInput("Enter a project name of 1 to 100 characters.".into())
        })
}

impl Database {
    /// Invoked when opening the phone's database, including existing installs.
    pub fn initialize_phone_chat_projects(&self) -> Result<(), CoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS phone_chat_projects (
                host TEXT NOT NULL,
                id TEXT NOT NULL,
                name TEXT NOT NULL,
                PRIMARY KEY (host, id)
             );
             CREATE TABLE IF NOT EXISTS phone_chat_project_members (
                host TEXT NOT NULL,
                conversation_id TEXT NOT NULL,
                project_id TEXT NOT NULL,
                PRIMARY KEY (host, conversation_id),
                FOREIGN KEY (host, project_id) REFERENCES phone_chat_projects(host, id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_phone_chat_project_members_project
                ON phone_chat_project_members(host, project_id);
             CREATE TRIGGER IF NOT EXISTS phone_chat_project_deleted_chat
             AFTER DELETE ON conversations BEGIN
                DELETE FROM phone_chat_project_members WHERE host='' AND conversation_id=OLD.id;
             END;",
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn phone_chat_projects(
        &self,
        host_id: Option<&str>,
        action: PhoneProjectAction,
        project_id: Option<&str>,
        name: Option<&str>,
        conversation_id: Option<&str>,
    ) -> Result<PhoneChatProjects, CoreError> {
        let host = host_id
            .map(|id| identifier(Some(id)))
            .transpose()?
            .unwrap_or("");
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(match action {
            PhoneProjectAction::List => TransactionBehavior::Deferred,
            _ => TransactionBehavior::Immediate,
        })?;
        match action {
            PhoneProjectAction::List => {}
            PhoneProjectAction::Create => {
                let id = match project_id {
                    Some(id) => identifier(Some(id))?.to_owned(),
                    None => uuid::Uuid::new_v4().to_string(),
                };
                tx.execute(
                    "INSERT INTO phone_chat_projects(host,id,name) VALUES(?1,?2,?3)
                     ON CONFLICT(host,id) DO NOTHING",
                    params![host, id, project_name(name)?],
                )?;
            }
            PhoneProjectAction::Rename => {
                if tx.execute(
                    "UPDATE phone_chat_projects SET name=?3 WHERE host=?1 AND id=?2",
                    params![host, identifier(project_id)?, project_name(name)?],
                )? == 0
                {
                    return Err(CoreError::NotFound("Project not found.".into()));
                }
            }
            PhoneProjectAction::Delete => {
                tx.execute(
                    "DELETE FROM phone_chat_projects WHERE host=?1 AND id=?2",
                    params![host, identifier(project_id)?],
                )?;
            }
            PhoneProjectAction::Move => {
                let chat = identifier(conversation_id)?;
                if let Some(id) = project_id {
                    let id = identifier(Some(id))?;
                    if host_id.is_none()
                        && !tx.query_row(
                            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1
                             AND parent_conversation_id IS NULL AND archived_at IS NULL)",
                            [chat],
                            |row| row.get::<_, bool>(0),
                        )?
                    {
                        return Err(CoreError::NotFound("Active conversation not found.".into()));
                    }
                    if tx.execute(
                        "INSERT INTO phone_chat_project_members(host,conversation_id,project_id)
                         SELECT host,?2,id FROM phone_chat_projects WHERE host=?1 AND id=?3
                         ON CONFLICT(host,conversation_id) DO UPDATE SET project_id=excluded.project_id",
                        params![host, chat, id],
                    )? == 0 {
                        return Err(CoreError::NotFound("Project not found.".into()));
                    }
                } else {
                    tx.execute("DELETE FROM phone_chat_project_members WHERE host=?1 AND conversation_id=?2", params![host, chat])?;
                }
            }
        }
        let projects = tx
            .prepare("SELECT id,name FROM phone_chat_projects WHERE host=?1 ORDER BY rowid")?
            .query_map([host], |row| {
                Ok(PhoneChatProject {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let memberships = tx
            .prepare(
                "SELECT conversation_id,project_id FROM phone_chat_project_members WHERE host=?1",
            )?
            .query_map([host], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        tx.commit()?;
        Ok(PhoneChatProjects {
            projects,
            memberships,
        })
    }

    /// Only a confirmed hard deletion on the paired host invalidates its
    /// link. Disconnection and archiving can both be followed by restoration.
    pub fn clear_deleted_paired_chat_membership(
        &self,
        host: &str,
        conversation_id: &str,
    ) -> Result<(), CoreError> {
        self.conn().execute(
            "DELETE FROM phone_chat_project_members WHERE host=?1 AND conversation_id=?2",
            params![identifier(Some(host))?, identifier(Some(conversation_id))?],
        )?;
        Ok(())
    }

    /// Local chat titles and archive status come from the conversation itself.
    /// The same IDs in a paired computer's namespace cannot match this join.
    pub fn local_phone_project_chats(
        &self,
        project_id: &str,
    ) -> Result<Vec<PhoneProjectChat>, CoreError> {
        let conn = self.conn();
        let mut query = conn.prepare(
            "SELECT c.id,c.title,c.model FROM phone_chat_project_members m
             JOIN conversations c ON c.id=m.conversation_id
             WHERE m.host='' AND m.project_id=?1 AND c.parent_conversation_id IS NULL AND c.archived_at IS NULL
             ORDER BY c.updated_at DESC,c.id DESC",
        )?;
        let chats = query
            .query_map([identifier(Some(project_id))?], |row| {
                Ok(PhoneProjectChat {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    model: row.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(chats)
    }
}

#[cfg(test)]
mod tests {
    use super::{Database, PhoneProjectAction::*};
    use crate::conversation::CreateConversationInput;

    #[test]
    fn local_project_history_uses_active_chat_order_and_keeps_chats_when_the_folder_is_removed() {
        let directory = tempfile::tempdir().unwrap();
        let db = Database::new(directory.path().join("phone.db")).unwrap();
        db.initialize_phone_chat_projects().unwrap();
        db.execute_batch_for_test(
            "INSERT INTO conversations(id,title,model,provider,updated_at) VALUES
             ('older','Old title','voice-a','custom','2026-01-01 00:00:00'),
             ('a','New title A','voice-b','custom','2026-01-02 00:00:00'),
             ('b','New title B','voice-c','custom','2026-01-02 00:00:00'),
             ('ungrouped','Other chat','voice-d','custom','2026-01-03 00:00:00');",
        )
        .unwrap();
        for host in [None, Some("paired")] {
            db.phone_chat_projects(host, Create, Some("project"), Some("Project"), None)
                .unwrap();
        }
        for chat in ["a", "older", "b"] {
            db.phone_chat_projects(None, Move, Some("project"), None, Some(chat))
                .unwrap();
        }
        db.phone_chat_projects(
            Some("paired"),
            Move,
            Some("project"),
            None,
            Some("ungrouped"),
        )
        .unwrap();
        let list = || db.local_phone_project_chats("project").unwrap();
        let ids = || list().into_iter().map(|chat| chat.id).collect::<Vec<_>>();
        assert_eq!(ids(), ["b", "a", "older"]);
        assert_eq!(list()[0].title, "New title B");
        assert_eq!(list()[0].model, "voice-c");
        db.archive_conversation("b").unwrap();
        assert_eq!(ids(), ["a", "older"]);
        db.unarchive_conversation("b").unwrap();
        assert_eq!(ids(), ["b", "a", "older"]);
        db.phone_chat_projects(None, Rename, Some("project"), Some("Renamed"), None)
            .unwrap();
        assert_eq!(list().len(), 3);
        db.phone_chat_projects(None, Delete, Some("project"), None, None)
            .unwrap();
        assert!(list().is_empty());
        assert_eq!(db.get_conversation("b").unwrap().title, "New title B");
    }

    #[test]
    fn moving_chats_is_durable_and_keeps_phone_and_paired_hosts_separate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("phone.db");
        let db = Database::new(&path).unwrap();
        let chat = db
            .create_conversation(&CreateConversationInput {
                provider: "custom".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        // An existing phone chat predates the folder schema.
        db.initialize_phone_chat_projects().unwrap();
        for host in [None, Some("desk-a"), Some("desk-b")] {
            let created = db
                .phone_chat_projects(host, Create, Some("same"), Some("  工作  "), None)
                .unwrap();
            assert_eq!(created.projects[0].name, "工作");
            let retried = db
                .phone_chat_projects(host, Create, Some("same"), Some("工作"), None)
                .unwrap();
            assert_eq!(retried.projects.len(), 1);
            db.phone_chat_projects(host, Move, Some("same"), None, Some(&chat.id))
                .unwrap();
        }
        db.phone_chat_projects(None, Create, Some("second"), Some("笔记"), None)
            .unwrap();
        db.phone_chat_projects(None, Move, Some("second"), None, Some(&chat.id))
            .unwrap();
        assert!(db.get_conversation(&chat.id).unwrap().project_id.is_none());
        drop(db);
        let db = Database::new(path).unwrap();
        db.initialize_phone_chat_projects().unwrap();
        assert_eq!(
            db.phone_chat_projects(None, List, None, None, None)
                .unwrap()
                .memberships[&chat.id],
            "second"
        );
        db.archive_conversation(&chat.id).unwrap();
        assert!(db
            .phone_chat_projects(None, Move, Some("same"), None, Some(&chat.id))
            .is_err());
        db.unarchive_conversation(&chat.id).unwrap();
        assert_eq!(
            db.phone_chat_projects(None, List, None, None, None)
                .unwrap()
                .memberships[&chat.id],
            "second"
        );
        db.archive_conversation(&chat.id).unwrap();
        db.delete_archived_conversations_batch(std::slice::from_ref(&chat.id))
            .unwrap();
        assert!(db
            .phone_chat_projects(None, List, None, None, None)
            .unwrap()
            .memberships
            .is_empty());
        assert_eq!(
            db.phone_chat_projects(Some("desk-a"), List, None, None, None)
                .unwrap()
                .memberships[&chat.id],
            "same"
        );
        db.phone_chat_projects(Some("desk-a"), Delete, Some("same"), None, None)
            .unwrap();
        assert!(db
            .phone_chat_projects(Some("desk-a"), List, None, None, None)
            .unwrap()
            .memberships
            .is_empty());
        assert_eq!(
            db.phone_chat_projects(Some("desk-b"), List, None, None, None)
                .unwrap()
                .memberships[&chat.id],
            "same"
        );
        assert!(db
            .phone_chat_projects(Some("desk-b"), Move, Some("second"), None, Some(&chat.id))
            .is_err());
        db.clear_deleted_paired_chat_membership("desk-b", &chat.id)
            .unwrap();
        assert!(db
            .phone_chat_projects(Some("desk-b"), List, None, None, None)
            .unwrap()
            .memberships
            .is_empty());
        db.phone_chat_projects(
            Some("desk-b"),
            Move,
            Some("same"),
            None,
            Some("another-chat"),
        )
        .unwrap();
        let removed = db
            .phone_chat_projects(Some("desk-b"), Move, None, None, Some("another-chat"))
            .unwrap();
        assert!(removed.memberships.is_empty());
        assert!(db
            .phone_chat_projects(None, Create, Some("new"), Some("  "), None)
            .is_err());
    }
}
