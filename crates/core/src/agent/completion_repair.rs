//! Completion checks measure stalled repair attempts, not total task length.
//! Only new persisted file states or newly satisfied runtime obligations renew
//! patience. Prompt prose, elapsed time and changing check logs do not.

use std::collections::HashSet;

use crate::{db::Database, error::CoreError};

#[derive(Default)]
pub(super) struct CompletionRepairGuard {
    file_revision: Option<u64>,
    seen_file_states: HashSet<blake3::Hash>,
    completed_milestones: HashSet<String>,
    stalled_retries: u8,
}

impl CompletionRepairGuard {
    pub(super) fn allow_retry(
        &mut self,
        db: &Database,
        scope: Option<(&str, &str)>,
        milestones: impl IntoIterator<Item = String>,
        retry_limit: u8,
    ) -> Result<bool, CoreError> {
        let mut progressed = false;
        for milestone in milestones {
            progressed |= self.completed_milestones.insert(milestone);
        }
        if let Some((conversation, turn)) = scope {
            progressed |= self.observe_file_progress(db, conversation, turn)?;
        }
        if progressed {
            self.stalled_retries = 0;
        }
        if self.stalled_retries >= retry_limit.max(1) {
            return Ok(false);
        }
        self.stalled_retries = self.stalled_retries.saturating_add(1);
        Ok(true)
    }

    fn observe_file_progress(
        &mut self,
        db: &Database,
        conversation: &str,
        turn: &str,
    ) -> Result<bool, CoreError> {
        let conn = db.conn();
        let (revision, pending): (u64, bool) = conn.query_row(
            "SELECT COALESCE(MAX(id),0),COALESCE(MAX(pending),0)
             FROM turn_file_change_events WHERE conversation_id=?1 AND turn_id=?2
             AND mutation_id NOT LIKE 'hook:%'",
            rusqlite::params![conversation, turn],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if revision == 0 || pending || self.file_revision == Some(revision) {
            return Ok(false);
        }
        self.file_revision = Some(revision);
        let mut query = conn.prepare(
            "SELECT absolute_path,after_hash,exists_after FROM turn_file_changes
             WHERE conversation_id=?1 AND turn_id=?2 ORDER BY absolute_path",
        )?;
        let rows = query.query_map(rusqlite::params![conversation, turn], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        let mut hash = blake3::Hasher::new();
        let mut has_files = false;
        for row in rows {
            let (path, content_hash, exists) = row?;
            has_files = true;
            let state = serde_json::to_vec(&(path, content_hash, exists))?;
            hash.update(&(state.len() as u64).to_le_bytes());
            hash.update(&state);
        }
        Ok(has_files && self.seen_file_states.insert(hash.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Database {
        let db = Database::open_memory().unwrap();
        db.execute_batch_for_test("INSERT INTO conversations(id,provider,model) VALUES('chat','custom','test');
            INSERT INTO messages(id,conversation_id,role,content) VALUES('user','chat','user','test');
            INSERT INTO conversation_turns(id,conversation_id,user_message_id) VALUES('turn','chat','user');
            INSERT INTO turn_file_changes(conversation_id,turn_id,absolute_path,display_path,after_hash,existed_before,exists_after,content_kind) VALUES('chat','turn','file','file','initial',0,1,'text');").unwrap();
        db
    }

    fn revision(db: &Database, id: &str, hash: &str) {
        db.conn().execute("INSERT INTO turn_file_change_events(conversation_id,turn_id,mutation_id) VALUES('chat','turn',?1)", [id]).unwrap();
        db.conn()
            .execute(
                "UPDATE turn_file_changes SET after_hash=?1 WHERE turn_id='turn'",
                [hash],
            )
            .unwrap();
    }

    #[test]
    fn productive_repairs_have_no_cumulative_retry_ceiling() {
        let db = fixture();
        let mut guard = CompletionRepairGuard::default();
        for index in 0..300 {
            revision(&db, &format!("edit-{index}"), &format!("contents-{index}"));
            assert!(guard
                .allow_retry(&db, Some(("chat", "turn")), [], 1)
                .unwrap());
        }
        assert!(!guard
            .allow_retry(&db, Some(("chat", "turn")), [], 1)
            .unwrap());
    }

    #[test]
    fn no_op_writes_and_hook_activity_do_not_reset_stall_detection() {
        let db = fixture();
        let mut guard = CompletionRepairGuard::default();
        revision(&db, "edit-first", "unchanged");
        assert!(guard
            .allow_retry(&db, Some(("chat", "turn")), [], 2)
            .unwrap());
        revision(&db, "edit-no-op", "unchanged");
        assert!(guard
            .allow_retry(&db, Some(("chat", "turn")), [], 2)
            .unwrap());
        revision(&db, "hook:changing-log", "log-noise");
        assert!(!guard
            .allow_retry(&db, Some(("chat", "turn")), [], 2)
            .unwrap());
    }

    #[test]
    fn returning_to_an_old_file_state_is_not_new_progress() {
        let db = fixture();
        let mut guard = CompletionRepairGuard::default();
        for state in ["candidate-a", "candidate-b"] {
            revision(&db, state, state);
            assert!(guard
                .allow_retry(&db, Some(("chat", "turn")), [], 1)
                .unwrap());
        }
        revision(&db, "revert", "candidate-a");
        assert!(!guard
            .allow_retry(&db, Some(("chat", "turn")), [], 1)
            .unwrap());
    }

    #[test]
    fn only_new_completed_milestones_renew_patience() {
        let db = Database::open_memory().unwrap();
        let mut guard = CompletionRepairGuard::default();
        for index in 0..100 {
            assert!(guard
                .allow_retry(&db, None, [format!("gate-{index}")], 1)
                .unwrap());
        }
        assert!(!guard.allow_retry(&db, None, ["gate-0".into()], 1).unwrap());
        assert!(!guard.allow_retry(&db, None, [], 1).unwrap());
    }
}
