//! Completion checks measure stalled repair attempts, not total task length.
//! Only new persisted file states or newly satisfied runtime obligations renew
//! patience. Prompt prose, elapsed time and changing check logs do not.

use std::collections::HashSet;

use crate::{db::Database, error::CoreError};

#[derive(Default)]
pub(super) struct CompletionRepairGuard {
    last_file_event: u64,
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
        let mut query = conn.prepare(
            "SELECT id,absolute_path,before_hash,after_hash FROM turn_file_change_events
             WHERE conversation_id=?1 AND turn_id=?2 AND id>?3
               AND mutation_id NOT LIKE 'hook:%' AND pending=0
               AND absolute_path IS NOT NULL AND before_hash IS NOT after_hash
             ORDER BY id",
        )?;
        let rows = query.query_map(
            rusqlite::params![conversation, turn, self.last_file_event],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )?;
        let mut progressed = false;
        for row in rows {
            let (id, path, before, after) = row?;
            // Seed the previous state too: reverting a failed candidate to
            // its original bytes is not a newly discovered repair state.
            self.seen_file_states
                .insert(blake3::hash(&serde_json::to_vec(&(&path, before))?));
            progressed |= self
                .seen_file_states
                .insert(blake3::hash(&serde_json::to_vec(&(&path, after))?));
            self.last_file_event = id;
        }
        Ok(progressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Database {
        let db = Database::open_memory().unwrap();
        db.execute_batch_for_test("INSERT INTO conversations(id,provider,model) VALUES('chat','custom','test');
            INSERT INTO messages(id,conversation_id,role,content) VALUES('user','chat','user','test');
            INSERT INTO conversation_turns(id,conversation_id,user_message_id) VALUES('turn','chat','user');").unwrap();
        db
    }

    fn revision(db: &Database, id: &str, path: &str, before: &str, after: &str) {
        let mut context = crate::tools::ToolExecutionContext::new(id, "{}", db, &[])
            .with_conversation_id(Some("chat"))
            .with_turn_id(Some("turn"));
        context.file_change_owner = Some(crate::turn_file_changes::FileChangeOwner {
            conversation_id: "chat".into(),
            turn_id: "turn".into(),
            mutation_namespace: id.starts_with("hook:").then(|| "hook".into()),
        });
        crate::turn_file_changes::FileChangeScope::from_context(&context)
            .unwrap()
            .record(
                id,
                path,
                path,
                crate::turn_file_changes::FileChangeContent {
                    hash: Some(before),
                    bytes: None,
                },
                crate::turn_file_changes::FileChangeContent {
                    hash: Some(after),
                    bytes: None,
                },
            )
            .unwrap();
    }

    #[test]
    fn productive_repairs_have_no_cumulative_retry_ceiling() {
        let db = fixture();
        let mut guard = CompletionRepairGuard::default();
        for index in 0..300 {
            revision(
                &db,
                &format!("edit-{index}"),
                "file",
                &format!("contents-{index}"),
                &format!("contents-{}", index + 1),
            );
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
        revision(&db, "edit-first", "file", "before", "unchanged");
        assert!(guard
            .allow_retry(&db, Some(("chat", "turn")), [], 2)
            .unwrap());
        revision(&db, "edit-no-op", "file", "unchanged", "unchanged");
        assert!(guard
            .allow_retry(&db, Some(("chat", "turn")), [], 2)
            .unwrap());
        revision(&db, "hook:changing-log", "log", "previous-log", "log-noise");
        revision(&db, "new-no-op-row", "untouched-file", "same", "same");
        assert!(!guard
            .allow_retry(&db, Some(("chat", "turn")), [], 2)
            .unwrap());
    }

    #[test]
    fn native_lifecycle_events_and_first_time_no_op_files_are_not_progress() {
        let db = fixture();
        let mut guard = CompletionRepairGuard::default();
        assert!(guard
            .allow_retry(&db, Some(("chat", "turn")), [], 1)
            .unwrap());
        db.execute_batch_for_test("INSERT INTO turn_file_change_events(conversation_id,turn_id,mutation_id) VALUES('chat','turn','finished:read-only-command');").unwrap();
        revision(
            &db,
            "untouched",
            "previously-untracked-file",
            "unchanged",
            "unchanged",
        );
        revision(&db, "hook:log", "log", "old-log", "new-log");
        assert!(!guard
            .allow_retry(&db, Some(("chat", "turn")), [], 1)
            .unwrap());
    }

    #[test]
    fn provenance_upgrade_preserves_legacy_events_without_inventing_effects() {
        let db = fixture();
        {
            let conn = db.conn();
            conn.execute_batch("ALTER TABLE turn_file_change_events DROP COLUMN absolute_path;
                ALTER TABLE turn_file_change_events DROP COLUMN before_hash;
                ALTER TABLE turn_file_change_events DROP COLUMN after_hash;
                DELETE FROM _migrations WHERE name='v149_file_change_progress';
                INSERT INTO turn_file_change_events(conversation_id,turn_id,mutation_id) VALUES('chat','turn','legacy');").unwrap();
            crate::migrations::run_migrations(&conn).unwrap();
            crate::migrations::run_migrations(&conn).unwrap();
            let legacy_unknown: bool = conn.query_row("SELECT absolute_path IS NULL AND before_hash IS NULL AND after_hash IS NULL FROM turn_file_change_events WHERE mutation_id='legacy'", [], |row| row.get(0)).unwrap();
            assert!(legacy_unknown);
        }
        let mut guard = CompletionRepairGuard::default();
        assert!(guard
            .allow_retry(&db, Some(("chat", "turn")), [], 1)
            .unwrap());
        assert!(!guard
            .allow_retry(&db, Some(("chat", "turn")), [], 1)
            .unwrap());
        revision(&db, "new-edit", "file", "before", "after");
        assert!(guard
            .allow_retry(&db, Some(("chat", "turn")), [], 1)
            .unwrap());
    }

    #[test]
    fn returning_to_an_old_file_state_is_not_new_progress() {
        let db = fixture();
        let mut guard = CompletionRepairGuard::default();
        for (before, state) in [("baseline", "candidate-a"), ("candidate-a", "candidate-b")] {
            revision(&db, state, "file", before, state);
            assert!(guard
                .allow_retry(&db, Some(("chat", "turn")), [], 1)
                .unwrap());
        }
        revision(&db, "revert", "file", "candidate-b", "candidate-a");
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
