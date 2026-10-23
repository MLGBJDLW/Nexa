//! Versioned, row-owned storage for large immutable JSON snapshots.
//!
//! Old TEXT/NULL values remain readable. New snapshots may be a bounded zlib
//! frame with an exact byte length and BLAKE3 digest. Public readers always
//! hydrate JSON; list projections use separately derived, small preview JSON.
//! Replacing/deleting a row naturally replaces/deletes its snapshot bytes.

use std::io::Write;

use flate2::{write::ZlibEncoder, Compression, Decompress, FlushDecompress, Status};
use rusqlite::{functions::FunctionFlags, types::ValueRef, Connection};

use crate::error::CoreError;

const MAGIC: &[u8; 8] = b"NEXASNP\0";
const VERSION: u8 = 1;
const HEADER_BYTES: usize = 8 + 1 + 8 + 32;
const MIN_BYTES: usize = 32 * 1024;
const MAX_BYTES: usize = 256 * 1024 * 1024;

fn eligible_size(bytes: usize) -> bool {
    (MIN_BYTES..=MAX_BYTES).contains(&bytes)
}

fn invalid_frame(reason: &'static str) -> rusqlite::Error {
    rusqlite::Error::UserFunctionError(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("Invalid stored snapshot: {reason}"),
    )))
}

fn pack(raw: &str) -> rusqlite::Result<rusqlite::types::Value> {
    if !eligible_size(raw.len()) {
        return Ok(rusqlite::types::Value::Text(raw.to_owned()));
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(raw.as_bytes())
        .map_err(|_| invalid_frame("compression failed"))?;
    let body = encoder
        .finish()
        .map_err(|_| invalid_frame("compression failed"))?;
    // Avoid storing a frame when its space saving cannot justify decoding.
    if body.len() + HEADER_BYTES >= raw.len() - raw.len() / 8 {
        return Ok(rusqlite::types::Value::Text(raw.to_owned()));
    }
    let mut frame = Vec::with_capacity(HEADER_BYTES + body.len());
    frame.extend_from_slice(MAGIC);
    frame.push(VERSION);
    frame.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    frame.extend_from_slice(blake3::hash(raw.as_bytes()).as_bytes());
    frame.extend_from_slice(&body);
    Ok(rusqlite::types::Value::Blob(frame))
}

fn frame_length(frame: &[u8]) -> rusqlite::Result<u64> {
    if frame.len() < HEADER_BYTES
        || frame.len() > MAX_BYTES + HEADER_BYTES
        || &frame[..8] != MAGIC
        || frame[8] != VERSION
    {
        return Err(invalid_frame("unsupported header or version"));
    }
    let expected = u64::from_le_bytes(frame[9..17].try_into().expect("fixed header slice"));
    if expected > MAX_BYTES as u64 {
        return Err(invalid_frame("declared size exceeds the decoding limit"));
    }
    Ok(expected)
}

fn unpack(frame: &[u8]) -> rusqlite::Result<String> {
    let expected = frame_length(frame)?;
    let body = &frame[HEADER_BYTES..];
    let mut decoder = Decompress::new(true);
    let mut bytes = Vec::with_capacity((expected as usize).min(1024 * 1024));
    let mut buffer = [0; 64 * 1024];
    loop {
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status = decoder
            .decompress(
                &body[before_in as usize..],
                &mut buffer,
                FlushDecompress::Finish,
            )
            .map_err(|_| invalid_frame("invalid compressed body"))?;
        if decoder.total_out() > expected {
            return Err(invalid_frame("decoded data exceeds the declared size"));
        }
        bytes.extend_from_slice(&buffer[..(decoder.total_out() - before_out) as usize]);
        if status == Status::StreamEnd {
            break;
        }
        if decoder.total_in() == before_in && decoder.total_out() == before_out {
            return Err(invalid_frame("incomplete compressed body"));
        }
    }
    if bytes.len() as u64 != expected || decoder.total_in() as usize != frame.len() - HEADER_BYTES {
        return Err(invalid_frame("length or compressed stream mismatch"));
    }
    if blake3::hash(&bytes).as_bytes() != &frame[17..49] {
        return Err(invalid_frame("content digest mismatch"));
    }
    String::from_utf8(bytes).map_err(|_| invalid_frame("content is not UTF-8"))
}

fn same_snapshot(left: ValueRef<'_>, right: ValueRef<'_>) -> rusqlite::Result<bool> {
    match (left, right) {
        (ValueRef::Null, ValueRef::Null) => Ok(true),
        (ValueRef::Text(left), ValueRef::Text(right))
        | (ValueRef::Blob(left), ValueRef::Blob(right)) => Ok(left == right),
        (ValueRef::Blob(frame), ValueRef::Text(raw))
        | (ValueRef::Text(raw), ValueRef::Blob(frame)) => {
            // A public read -> same-Value write must not reapply a->aa privacy
            // rules merely because its representation changed from BLOB to TEXT.
            // Compare the new bytes to the immutable frame identity without
            // inflating the previous body on this live write path.
            Ok(frame_length(frame)? == raw.len() as u64
                && blake3::hash(raw).as_bytes() == &frame[17..49])
        }
        _ => Ok(false),
    }
}

fn json(value: ValueRef<'_>) -> rusqlite::Result<Option<String>> {
    match value {
        ValueRef::Null => Ok(None),
        ValueRef::Text(raw) => std::str::from_utf8(raw)
            .map(|raw| Some(raw.to_owned()))
            .map_err(|_| invalid_frame("legacy content is not UTF-8")),
        ValueRef::Blob(frame) => unpack(frame).map(Some),
        _ => Err(invalid_frame("unexpected SQLite value type")),
    }
}

pub(crate) fn register(conn: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    conn.create_scalar_function("nexa_snapshot_json_v1", 1, flags, |context| {
        json(context.get_raw(0))
    })?;
    conn.create_scalar_function("nexa_snapshot_same_v1", 2, flags, |context| {
        same_snapshot(context.get_raw(0), context.get_raw(1))
    })?;
    conn.create_scalar_function("nexa_snapshot_pack_v1", 1, flags, |context| {
        match context.get_raw(0) {
            ValueRef::Null => Ok(rusqlite::types::Value::Null),
            ValueRef::Text(raw) => pack(
                std::str::from_utf8(raw)
                    .map_err(|_| invalid_frame("legacy content is not UTF-8"))?,
            ),
            // Maintenance reopens must be a cheap no-op for existing frames.
            ValueRef::Blob(frame) => Ok(rusqlite::types::Value::Blob(frame.to_vec())),
            _ => Err(invalid_frame("unexpected SQLite value type")),
        }
    })?;
    conn.create_scalar_function("nexa_task_artifacts_preview_v1", 1, flags, |context| {
        let Some(raw) = context.get::<Option<String>>(0)? else {
            return Ok(None);
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            // Legacy malformed text retains its existing explicit detail error;
            // it must not make the representation-only migration fail.
            return Ok(None);
        };
        Ok(Some(
            serde_json::json!({
                "display": crate::task_run_presentation::project_artifacts(&value),
                "catalog": crate::task_run_presentation::project_catalog(&value),
            })
            .to_string(),
        ))
    })?;
    conn.create_scalar_function("nexa_subtask_output_preview_v1", 1, flags, |context| {
        let Some(raw) = context.get::<Option<String>>(0)? else {
            return Ok(None);
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return Ok(None);
        };
        Ok(Some(
            crate::task_run_presentation::project_catalog(&value).to_string(),
        ))
    })?;
    Ok(())
}

pub(crate) fn enabled(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('agent_task_runs') WHERE name='artifacts_preview_json')",
        [], |row| row.get(0),
    )
}

pub(crate) fn packed_field(table: &str, column: &str) -> bool {
    matches!(
        (table, column),
        ("conversation_turns", "trace_json") | ("agent_task_runs", "artifacts_json")
    )
}

/// Convert existing already-projected snapshots without applying privacy again.
/// The caller's migration transaction owns guard restoration on every failure.
pub(crate) fn backfill(conn: &Connection) -> Result<(), CoreError> {
    if conn.is_autocommit() {
        return Err(CoreError::Conflict(
            "Snapshot migration requires an owning transaction".into(),
        ));
    }
    // Representation changes are not semantic trace edits. Preserve the exact
    // previous revision/flags and avoid decoding large traces just to reproduce
    // those values. SQLite's transaction restores these triggers on any failure.
    let display_triggers = {
        let mut statement = conn.prepare(
            "SELECT name,sql FROM sqlite_master WHERE type='trigger' AND name IN
             ('conversation_turns_display_revision','conversation_turns_display_trace_flags_update')",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (name, _) in &display_triggers {
        conn.execute_batch(&format!("DROP TRIGGER {name}"))?;
    }
    conn.execute("UPDATE privacy_chat_state SET guard=1 WHERE id=1", [])?;
    conn.execute(
        "UPDATE agent_task_runs SET artifacts_preview_json=nexa_task_artifacts_preview_v1(nexa_snapshot_json_v1(artifacts_json)),
          artifacts_json=nexa_snapshot_pack_v1(artifacts_json)
         WHERE artifacts_json IS NOT NULL",
        [],
    )?;
    conn.execute(
        "UPDATE agent_subtask_runs SET output_preview_json=nexa_subtask_output_preview_v1(output_json)
         WHERE output_json IS NOT NULL",
        [],
    )?;
    conn.execute(
        "UPDATE conversation_turns SET trace_json=nexa_snapshot_pack_v1(trace_json)
         WHERE typeof(trace_json)='text' AND octet_length(trace_json)>=32768",
        [],
    )?;
    conn.execute("UPDATE privacy_chat_state SET guard=0 WHERE id=1", [])?;
    for (_, sql) in display_triggers {
        conn.execute_batch(&sql)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::CreateConversationInput;
    use crate::db::Database;
    use rusqlite::params;
    use serde_json::{json, Value};

    fn fixture(db: &Database) -> (String, String, String) {
        let conversation = db
            .create_conversation(&CreateConversationInput {
                provider: "test".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        let message = uuid::Uuid::new_v4().to_string();
        db.conn()
            .execute(
                "INSERT INTO messages(id,conversation_id,role,content) VALUES(?1,?2,'user','Task')",
                params![message, conversation.id],
            )
            .unwrap();
        let turn = db
            .create_conversation_turn(&conversation.id, &message, None)
            .unwrap();
        let run = db
            .create_agent_task_run(
                &conversation.id,
                &turn.id,
                &message,
                "Snapshot",
                Some("test"),
                Some("test"),
            )
            .unwrap();
        db.mark_agent_task_run_started(&run.id, "responding")
            .unwrap();
        (conversation.id, turn.id, run.id)
    }

    fn trace(text: &str) -> Value {
        json!({"kind":"turnTrace","items":[{"kind":"reply","text":text}]})
    }

    fn store(db: &Database, turn: &str, run: &str, trace: &Value) -> Value {
        let artifacts = json!({"kind":"agentTaskArtifacts","trace":trace,"subtasks":[],"selectedSkills":{"kind":"selectedSkills","skills":[]}});
        db.update_conversation_turn_trace(turn, Some(trace))
            .unwrap();
        db.update_agent_task_run_progress(
            run,
            None,
            None,
            None,
            Some("Snapshot"),
            None,
            Some(&artifacts),
        )
        .unwrap();
        artifacts
    }

    #[test]
    fn snapshots_keep_checkpoint_versions_and_row_owned_lifetimes() {
        let directory = tempfile::tempdir().unwrap();
        let db = Database::new(directory.path().join("snapshots.db")).unwrap();
        let (conversation, turn, run) = fixture(&db);
        let first = trace(&"Evidence 结果🦀\n".repeat(5000));
        let first_artifacts = store(&db, &turn, &run, &first);
        let reader = db.read_only_lane().unwrap();
        assert_eq!(
            reader.get_conversation_turn(&turn).unwrap().trace,
            Some(first.clone())
        );
        assert_eq!(
            reader.get_agent_task_run(&run).unwrap().artifacts,
            Some(first_artifacts.clone())
        );
        let checkpoint = db
            .create_task_resume_checkpoint(&run, "snapshot test")
            .unwrap();
        assert_eq!(checkpoint.state["run"]["artifacts"], first_artifacts);
        let captured = db.get_agent_task_run(&run).unwrap();
        let second = trace(&"Later independent result\n".repeat(5000));
        let second_artifacts = store(&db, &turn, &run, &second);
        assert_eq!(captured.artifacts, Some(first_artifacts.clone()));
        assert_eq!(
            db.get_task_resume_checkpoint(&checkpoint.id).unwrap().state["run"]["artifacts"],
            first_artifacts
        );
        assert_eq!(
            reader.get_agent_task_run(&run).unwrap().artifacts,
            Some(second_artifacts)
        );
        let storage: (String,String) = db.conn().query_row(
            "SELECT typeof(t.trace_json),typeof(r.artifacts_json) FROM conversation_turns t JOIN agent_task_runs r ON r.turn_id=t.id WHERE t.id=?1",
            [&turn], |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert_eq!(storage, ("blob".into(), "blob".into()));
        db.update_conversation_turn_trace(&turn, None).unwrap();
        assert!(db.get_conversation_turn(&turn).unwrap().trace.is_none());
        db.conn()
            .execute("DELETE FROM conversations WHERE id=?1", [&conversation])
            .unwrap();
        let remaining: i64 = db.conn().query_row("SELECT (SELECT COUNT(*) FROM conversation_turns)+(SELECT COUNT(*) FROM agent_task_runs)+(SELECT COUNT(*) FROM task_resume_checkpoints)", [], |row| row.get(0)).unwrap();
        assert_eq!(
            remaining, 0,
            "row-owned frames cannot become orphaned shared blobs"
        );
    }

    #[test]
    fn snapshot_failure_rolls_back_payload_preview_and_privacy_guard() {
        let db = Database::open_memory().unwrap();
        let (_, turn, run) = fixture(&db);
        let original = store(&db, &turn, &run, &trace(&"Original\n".repeat(5000)));
        let before: (Vec<u8>, String) = db
            .conn()
            .query_row(
                "SELECT artifacts_json,artifacts_preview_json FROM agent_task_runs WHERE id=?1",
                [&run],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        db.conn().execute_batch("CREATE TRIGGER reject_packed_snapshot AFTER UPDATE OF artifacts_json ON agent_task_runs WHEN typeof(NEW.artifacts_json)='blob' BEGIN SELECT RAISE(ABORT,'fixture snapshot rejection'); END;").unwrap();
        let result = db.update_agent_task_run_progress(
            &run,
            None,
            None,
            None,
            None,
            None,
            Some(
                &json!({"kind":"agentTaskArtifacts","trace":trace(&"Replacement\n".repeat(5000))}),
            ),
        );
        assert!(result.is_err());
        let after: (Vec<u8>,String,i64) = db.conn().query_row(
            "SELECT artifacts_json,artifacts_preview_json,(SELECT guard FROM privacy_chat_state WHERE id=1) FROM agent_task_runs WHERE id=?1", [&run],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).unwrap();
        assert_eq!(after, (before.0, before.1, 0));
        assert_eq!(
            db.get_agent_task_run(&run).unwrap().artifacts,
            Some(original)
        );
    }

    #[test]
    fn snapshot_presentation_does_not_decode_details_and_corruption_fails_closed() {
        let db = Database::open_memory().unwrap();
        let (conversation, turn, run) = fixture(&db);
        store(
            &db,
            &turn,
            &run,
            &trace(&"Independent details\n".repeat(5000)),
        );
        // Simulate on-disk body corruption without altering the derived metadata.
        db.conn()
            .execute_batch("UPDATE privacy_chat_state SET guard=1 WHERE id=1;")
            .unwrap();
        db.conn()
            .execute(
                "UPDATE agent_task_runs SET artifacts_json=x'00' WHERE id=?1",
                [&run],
            )
            .unwrap();
        db.conn()
            .execute_batch("UPDATE privacy_chat_state SET guard=0 WHERE id=1;")
            .unwrap();
        assert!(db.get_agent_task_run(&run).is_err());
        let preview = db.get_agent_task_run_presentation(&run).unwrap();
        assert!(preview.artifacts.as_ref().unwrap().get("trace").is_none());
        assert_eq!(
            db.get_agent_task_run_presentations_for_conversation(&conversation)
                .unwrap()
                .len(),
            1
        );
        assert!(!db
            .list_agent_task_artifact_metadata(&run)
            .unwrap()
            .is_empty());
        db.update_agent_task_run_progress(
            &run,
            Some("running"),
            None,
            None,
            Some("Metadata only"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            db.get_agent_task_run_presentation(&run)
                .unwrap()
                .summary
                .as_deref(),
            Some("Metadata only")
        );
        assert!(
            db.get_agent_task_run(&run).is_err(),
            "metadata reads must not silently repair a damaged authoritative frame"
        );
    }

    #[test]
    fn snapshot_upgrade_preserves_non_idempotent_privacy_and_display_revisions() {
        let db = Database::open_memory().unwrap();
        // Reconstruct the last schema capability before v157. The old privacy
        // hooks must keep TEXT while the preview columns do not exist yet.
        {
            let conn = db.conn();
            for name in ["turn_trace", "run", "subtask"] {
                conn.execute_batch(&format!("DROP TRIGGER privacy_chat_{name}_INSERT; DROP TRIGGER privacy_chat_{name}_UPDATE;")).unwrap();
            }
            conn.execute_batch(
                "ALTER TABLE agent_task_runs DROP COLUMN artifacts_preview_json;
                ALTER TABLE agent_subtask_runs DROP COLUMN output_preview_json;
                DELETE FROM _migrations WHERE name='v157_owned_snapshot_storage';",
            )
            .unwrap();
            crate::privacy::chat_store::install_snapshot_projections(&conn).unwrap();
        }
        let mut policy = crate::privacy::PrivacyConfig::default();
        policy.redact_patterns = vec![crate::privacy::RedactRule {
            name: "non-idempotent".into(),
            pattern: "a".into(),
            replacement: "aa".into(),
        }];
        db.save_privacy_config(&policy).unwrap();
        let (_, turn, run) = fixture(&db);
        store(&db, &turn, &run, &trace(&"a\n".repeat(20000)));
        // Full pre-storage reads still use the legacy TEXT passthrough codec.
        let before_turn = db.get_conversation_turn(&turn).unwrap();
        let before_run = db.get_agent_task_run(&run).unwrap();
        assert_eq!(
            before_turn.trace.as_ref().unwrap()["items"][0]["text"]
                .as_str()
                .unwrap(),
            "aa\n".repeat(20000)
        );
        let before_flags: (i64, Option<i64>) = db
            .conn()
            .query_row(
                "SELECT display_revision,display_trace_flags FROM conversation_turns WHERE id=?1",
                [&turn],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT typeof(trace_json) FROM conversation_turns WHERE id=?1",
                    [&turn],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "text"
        );
        db.conn().execute_batch("CREATE TRIGGER reject_snapshot_upgrade BEFORE UPDATE OF artifacts_json ON agent_task_runs WHEN (SELECT guard FROM privacy_chat_state WHERE id=1)=1 BEGIN SELECT RAISE(ABORT,'fixture upgrade rejection'); END;").unwrap();
        assert!(crate::migrations::run_migrations(&db.conn()).is_err());
        assert!(
            !enabled(&db.conn()).unwrap(),
            "failed migration must roll back its columns"
        );
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT guard FROM privacy_chat_state WHERE id=1",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            db.get_conversation_turn(&turn).unwrap().trace,
            before_turn.trace
        );
        let restored_triggers: i64 = db.conn().query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name IN ('conversation_turns_display_revision','conversation_turns_display_trace_flags_update')", [], |row|row.get(0)).unwrap();
        assert_eq!(restored_triggers, 2);
        db.conn()
            .execute_batch("DROP TRIGGER reject_snapshot_upgrade")
            .unwrap();
        crate::migrations::run_migrations(&db.conn()).unwrap();
        crate::migrations::run_migrations(&db.conn()).unwrap();
        assert_eq!(
            db.get_conversation_turn(&turn).unwrap().trace,
            before_turn.trace
        );
        assert_eq!(
            db.get_agent_task_run(&run).unwrap().artifacts,
            before_run.artifacts
        );
        let after_flags: (i64, Option<i64>) = db
            .conn()
            .query_row(
                "SELECT display_revision,display_trace_flags FROM conversation_turns WHERE id=?1",
                [&turn],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(after_flags, before_flags);
        assert_eq!(
            db.conn()
                .query_row(
                    "SELECT guard FROM privacy_chat_state WHERE id=1",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        // Hydrate -> unchanged write must retain the packed representation and
        // must not apply a non-idempotent privacy rule a second time.
        let before_packed: (Vec<u8>, Vec<u8>, String) = db.conn().query_row(
            "SELECT t.trace_json,r.artifacts_json,r.artifacts_preview_json FROM conversation_turns t JOIN agent_task_runs r ON r.turn_id=t.id WHERE t.id=?1",
            [&turn], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        db.update_conversation_turn_trace(&turn, before_turn.trace.as_ref())
            .unwrap();
        db.update_agent_task_run_progress(
            &run,
            None,
            None,
            None,
            None,
            None,
            before_run.artifacts.as_ref(),
        )
        .unwrap();
        assert_eq!(
            db.get_conversation_turn(&turn).unwrap().trace,
            before_turn.trace
        );
        assert_eq!(
            db.get_agent_task_run(&run).unwrap().artifacts,
            before_run.artifacts
        );
        let after_packed: (Vec<u8>, Vec<u8>, String) = db.conn().query_row(
            "SELECT t.trace_json,r.artifacts_json,r.artifacts_preview_json FROM conversation_turns t JOIN agent_task_runs r ON r.turn_id=t.id WHERE t.id=?1",
            [&turn], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(after_packed, before_packed);
        // An actual policy change still reprojects each stored copy exactly once.
        policy.redact_patterns.push(crate::privacy::RedactRule {
            name: "revision".into(),
            pattern: "never-match".into(),
            replacement: "removed".into(),
        });
        db.save_privacy_config(&policy).unwrap();
        let after_revoke = db.get_conversation_turn(&turn).unwrap().trace.unwrap();
        assert_eq!(
            after_revoke["items"][0]["text"].as_str().unwrap(),
            "aaaa\n".repeat(20000)
        );
    }

    #[test]
    fn snapshot_frame_roundtrips_unicode_and_rejects_corruption() {
        let raw = serde_json::json!({"kind":"turnTrace","text":"Result 结果🦀\n".repeat(5000)})
            .to_string();
        let rusqlite::types::Value::Blob(frame) = pack(&raw).unwrap() else {
            panic!("repeated large snapshot should be packed");
        };
        assert_eq!(unpack(&frame).unwrap(), raw);
        assert!(frame.len() < raw.len() / 8);
        for index in [0, 8, 17, HEADER_BYTES] {
            let mut corrupt = frame.clone();
            corrupt[index] ^= 0xff;
            assert!(unpack(&corrupt).is_err(), "accepted corruption at {index}");
        }
        let mut oversized = frame.clone();
        oversized[9..17].copy_from_slice(&((MAX_BYTES as u64) + 1).to_le_bytes());
        assert!(unpack(&oversized).is_err());
        let mut wrong_length = frame.clone();
        wrong_length[9..17].copy_from_slice(&1_u64.to_le_bytes());
        assert!(unpack(&wrong_length).is_err());
        assert!(unpack(&frame[..frame.len() - 1]).is_err());
        let mut trailing = frame.clone();
        trailing.push(0);
        assert!(unpack(&trailing).is_err());
        assert_eq!(
            pack("{} ").unwrap(),
            rusqlite::types::Value::Text("{} ".into())
        );
        assert_eq!(json(ValueRef::Null).unwrap(), None);
        assert_eq!(
            json(ValueRef::Text(b"{legacy malformed")).unwrap(),
            Some("{legacy malformed".into())
        );
        assert!(json(ValueRef::Blob(b"{\"not\":\"a frame\"}")).is_err());
        assert!(eligible_size(MAX_BYTES));
        assert!(
            !eligible_size(MAX_BYTES + 1),
            "oversized legacy JSON stays inline rather than being truncated or rejected"
        );
        assert!(!eligible_size(MIN_BYTES - 1));
    }
}
