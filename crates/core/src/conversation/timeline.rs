//! Bounded desktop history reads. User anchors and final replies are cheap to open;
//! full turn/tool history is read only when the user asks for that entry's details.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::{
    conversation_message_for_display_with_turn_trace, conversation_turn_for_display, str_to_role,
    AgentTaskRun, Conversation, ConversationMessage, ConversationTurn, ImageAttachment,
};
use crate::{db::Database, error::CoreError, llm::ToolCallRequest};

const MAX_PAGE_SIZE: usize = 50;
const DEFAULT_PAGE_SIZE: usize = 20;
const ROOT_PREDICATE: &str = "role = 'user' AND COALESCE(display_artifact_kind, '') NOT IN ('steering', 'questionResponse', 'checkpointContinuation')";
type TimelineTurn = (ConversationTurn, bool, i64, Option<i64>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTimelineCursor {
    pub sort_order: i64,
    pub message_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::CreateConversationInput;

    fn fixture(count: usize) -> (Database, String) {
        let db = Database::open_memory().unwrap();
        let conversation = db
            .create_conversation(&CreateConversationInput {
                provider: "openai".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        {
            let connection = db.conn();
            let tx = connection.unchecked_transaction().unwrap();
            tx.execute_batch("CREATE TEMP TABLE timeline_numbers(n INTEGER PRIMARY KEY)")
                .unwrap();
            tx.execute("WITH RECURSIVE numbers(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM numbers WHERE n+1<?1) INSERT INTO timeline_numbers SELECT n FROM numbers", [count as i64]).unwrap();
            tx.execute("INSERT INTO messages(id,conversation_id,role,content,sort_order,created_at) SELECT printf('u%05d',n),?1,'user','question',n*10,'same-second' FROM timeline_numbers",[&conversation.id]).unwrap();
            tx.execute("INSERT INTO messages(id,conversation_id,role,content,sort_order,created_at) SELECT printf('tool%05d',n),?1,'tool',replace(hex(zeroblob(2048)),'0','x'),n*10+1,'same-second' FROM timeline_numbers",[&conversation.id]).unwrap();
            tx.execute("INSERT INTO messages(id,conversation_id,role,content,thinking,sort_order,created_at) SELECT printf('a%05d',n),?1,'assistant','answer','hidden thinking',n*10+2,'same-second' FROM timeline_numbers",[&conversation.id]).unwrap();
            // Malformed trace must not be parsed by a summary page. It fails only
            // when the explicit detail endpoint reads the affected entry.
            tx.execute("INSERT INTO conversation_turns(id,conversation_id,user_message_id,assistant_message_id,status,trace_json,created_at,updated_at) SELECT printf('t%05d',n),?1,printf('u%05d',n),printf('a%05d',n),'completed','{invalid trace','same-second','same-second' FROM timeline_numbers",[&conversation.id]).unwrap();
            // IDs deliberately oppose message order to catch timestamp/id paging.
            tx.execute("INSERT INTO agent_task_runs(id,conversation_id,turn_id,user_message_id,status,phase,title,provider,model,created_at,updated_at) SELECT printf('run%05d',10000-n),?1,printf('t%05d',n),printf('u%05d',n),'completed','done','task','openai','test','same-second','same-second' FROM timeline_numbers",[&conversation.id]).unwrap();
            tx.commit().unwrap();
        }
        (db, conversation.id)
    }

    #[test]
    fn timeline_unrelated_writes_do_not_reparse_traces_or_scale_with_trace_size() {
        use rusqlite::{functions::FunctionFlags, StatementStatus};
        let mut content_update_costs = Vec::new();
        for trace_size in [1, 1_000] {
            let (db, id) = fixture(1);
            let items = vec![serde_json::json!({"kind":"thinking","text":"private"}); trace_size];
            let legacy = serde_json::json!({"kind":"traceTimeline","items":items});
            let canonical = serde_json::json!({"kind":"turnTrace","items":items});
            let connection = db.conn();
            connection.execute("UPDATE messages SET content='private',thinking='private',artifacts_json=?1 WHERE id='a00000'", [legacy.to_string()]).unwrap();
            connection
                .execute(
                    "UPDATE conversation_turns SET trace_json=?1 WHERE id='t00000'",
                    [canonical.to_string()],
                )
                .unwrap();

            // Every trace was classified above. Unrelated writes and clearing
            // or retaining an existing payload must not enter a JSON parser.
            for (name, arity) in [
                ("json_valid", 1),
                ("json_extract", 2),
                ("json_type", 2),
                ("json_remove", -1),
            ] {
                connection
                    .create_scalar_function(
                        name,
                        arity,
                        FunctionFlags::SQLITE_UTF8
                            | FunctionFlags::SQLITE_DETERMINISTIC
                            | FunctionFlags::SQLITE_INNOCUOUS,
                        move |_| -> rusqlite::Result<i32> {
                            Err(rusqlite::Error::UserFunctionError(Box::new(
                                std::io::Error::other(format!(
                                    "unrelated write parsed trace through {name}"
                                )),
                            )))
                        },
                    )
                    .unwrap();
            }
            let sql = "UPDATE messages SET content='new reply' WHERE id='a00000'";
            let mut statement = connection.prepare(sql).unwrap();
            statement.execute([]).unwrap();
            content_update_costs.push(statement.get_status(StatementStatus::VmStep));
            let bytecode_functions = connection
                .prepare(&format!("EXPLAIN {sql}"))
                .unwrap()
                .query_map([], |row| row.get::<_, Option<String>>(5))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(
                !bytecode_functions
                    .iter()
                    .flatten()
                    .any(|name| name.starts_with("json_")),
                "content-only update prepared a JSON classifier"
            );
            assert!(!connection
                .query_row(
                    "SELECT display_reasoning_candidate FROM messages WHERE id='a00000'",
                    [],
                    |row| row.get::<_, bool>(0)
                )
                .unwrap());

            connection
                .execute("UPDATE messages SET role='tool' WHERE id='a00000'", [])
                .unwrap();
            connection
                .execute(
                    "UPDATE messages SET role='assistant',content=thinking WHERE id='a00000'",
                    [],
                )
                .unwrap();
            assert!(connection
                .query_row(
                    "SELECT display_reasoning_candidate FROM messages WHERE id='a00000'",
                    [],
                    |row| row.get::<_, bool>(0)
                )
                .unwrap());
            connection.execute("UPDATE messages SET content=content,thinking=thinking,artifacts_json=artifacts_json WHERE id='a00000'", []).unwrap();
            connection
                .execute(
                    "UPDATE conversation_turns SET trace_json=trace_json WHERE id='t00000'",
                    [],
                )
                .unwrap();
            assert_eq!(
                connection
                    .query_row(
                        "SELECT display_legacy_trace_flags FROM messages WHERE id='a00000'",
                        [],
                        |row| row.get::<_, Option<i64>>(0)
                    )
                    .unwrap(),
                Some(1)
            );
            assert_eq!(
                connection
                    .query_row(
                        "SELECT display_trace_flags FROM conversation_turns WHERE id='t00000'",
                        [],
                        |row| row.get::<_, Option<i64>>(0)
                    )
                    .unwrap(),
                Some(1)
            );

            connection
                .execute(
                    "UPDATE messages SET artifacts_json=NULL,thinking=NULL WHERE id='a00000'",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE conversation_turns SET trace_json=NULL WHERE id='t00000'",
                    [],
                )
                .unwrap();
            assert_eq!(connection.query_row("SELECT display_artifact_kind,display_legacy_trace_flags,display_reasoning_candidate FROM messages WHERE id='a00000'", [], |row| Ok((row.get::<_,Option<String>>(0)?,row.get::<_,Option<i64>>(1)?,row.get::<_,bool>(2)?))).unwrap(), (None,None,false));
            assert_eq!(
                connection
                    .query_row(
                        "SELECT display_trace_flags FROM conversation_turns WHERE id='t00000'",
                        [],
                        |row| row.get::<_, Option<i64>>(0)
                    )
                    .unwrap(),
                None
            );
            for (index, role) in ["user", "assistant", "tool"].iter().enumerate() {
                connection.execute("INSERT INTO messages(id,conversation_id,role,content,sort_order) VALUES(?1,?2,?3,'plain text',10)", params![format!("plain-{index}"),id,role]).unwrap();
            }
            connection.execute("INSERT INTO conversation_turns(id,conversation_id,user_message_id,status) VALUES('plain-turn',?1,'plain-0','running')", [&id]).unwrap();
        }
        assert_eq!(content_update_costs[0], content_update_costs[1]);
        eprintln!("content-only update VM steps for 1/1000 trace items: {content_update_costs:?}");
    }

    #[test]
    fn timeline_reasoning_only_summary_keeps_guard_without_private_text() {
        let (db, id) = fixture(2);
        let private = "private reasoning accidentally copied into reply";
        let trace =
            serde_json::json!({"kind":"turnTrace","items":[{"kind":"thinking","text":private}]});
        db.conn()
            .execute(
                "UPDATE messages SET content=?1,thinking=?1 WHERE id='a00001'",
                [private],
            )
            .unwrap();
        db.conn()
            .execute(
                "UPDATE conversation_turns SET trace_json=?1 WHERE id='t00001'",
                [trace.to_string()],
            )
            .unwrap();

        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        let message = page
            .messages
            .iter()
            .find(|message| message.id == "a00001")
            .unwrap();
        assert!(
            message.content.is_empty(),
            "summary exposed private reasoning: {}",
            message.content
        );
        assert_eq!(
            message
                .artifacts
                .as_ref()
                .and_then(|value| value.get("displayReasoningOnly")),
            Some(&serde_json::Value::Bool(true))
        );
        assert!(message.thinking.is_none());
        assert!(!serde_json::to_string(&page).unwrap().contains(private));

        let details = db.conversation_timeline_details(&id, "u00001").unwrap();
        let message = details
            .messages
            .iter()
            .find(|message| message.id == "a00001")
            .unwrap();
        assert_eq!(message.content, private);
        assert_eq!(message.thinking.as_deref(), Some(private));
        assert_eq!(details.turns[0].trace.as_ref(), Some(&trace));
    }

    #[test]
    fn timeline_details_keep_legacy_only_when_canonical_projection_is_unusable() {
        let (db, id) = fixture(1);
        let legacy = serde_json::json!({"kind":"traceTimeline","items":[{"kind":"thinking","text":"private"}]});
        db.conn().execute("UPDATE messages SET content='private',thinking='private',artifacts_json=?1 WHERE id='a00000'", [legacy.to_string()]).unwrap();
        let invalid =
            serde_json::json!({"kind":"turnTrace","items":[{"kind":"unknown","text":"ignored"}]});
        db.conn()
            .execute(
                "UPDATE conversation_turns SET trace_json=?1 WHERE id='t00000'",
                [invalid.to_string()],
            )
            .unwrap();
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert!(page
            .messages
            .iter()
            .find(|message| message.id == "a00000")
            .unwrap()
            .content
            .is_empty());
        let details = db.conversation_timeline_details(&id, "u00000").unwrap();
        let message = details
            .messages
            .iter()
            .find(|message| message.id == "a00000")
            .unwrap();
        assert_eq!(message.artifacts.as_ref(), Some(&legacy));
        assert_eq!(message.thinking.as_deref(), Some("private"));

        // Even empty reply/status items are valid canonical projections and
        // therefore prevent a legacy thinking-only trace from overriding them.
        let valid = serde_json::json!({"kind":"turnTrace","items":[{"kind":"status","text":""}]});
        db.conn()
            .execute(
                "UPDATE conversation_turns SET trace_json=?1 WHERE id='t00000'",
                [valid.to_string()],
            )
            .unwrap();
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .find(|message| message.id == "a00000")
                .unwrap()
                .content,
            "private"
        );
        let details = db.conversation_timeline_details(&id, "u00000").unwrap();
        assert!(details
            .messages
            .iter()
            .find(|message| message.id == "a00000")
            .unwrap()
            .artifacts
            .is_none());
        assert_eq!(details.turns[0].trace.as_ref(), Some(&valid));
    }

    #[test]
    fn timeline_reasoning_marker_is_derived_and_keeps_public_artifacts() {
        let (db, id) = fixture(1);
        let image = serde_json::json!({"kind":"generatedImage","dataUrl":"data:image/png;base64,fixture","displayReasoningOnly":true});
        let trace =
            serde_json::json!({"kind":"turnTrace","items":[{"kind":"thinking","text":"private"}]});
        db.conn().execute("UPDATE messages SET content='private',thinking='private',artifacts_json=?1 WHERE id='a00000'", [image.to_string()]).unwrap();
        db.conn()
            .execute(
                "UPDATE conversation_turns SET trace_json=?1 WHERE id='t00000'",
                [trace.to_string()],
            )
            .unwrap();
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        let message = page
            .messages
            .iter()
            .find(|message| message.id == "a00000")
            .unwrap();
        assert!(message.content.is_empty());
        assert_eq!(message.artifacts.as_ref(), Some(&image));

        // An identical marker persisted on an ordinary final response is not
        // evidence. Keep its public image payload and discard the forged field.
        db.conn()
            .execute(
                "UPDATE messages SET content='real final answer' WHERE id='a00000'",
                [],
            )
            .unwrap();
        let mut public_image = image.clone();
        public_image
            .as_object_mut()
            .unwrap()
            .remove("displayReasoningOnly");
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        let message = page
            .messages
            .iter()
            .find(|message| message.id == "a00000")
            .unwrap();
        assert_eq!(message.content, "real final answer");
        assert_eq!(message.artifacts.as_ref(), Some(&public_image));
        let details = db.conversation_timeline_details(&id, "u00000").unwrap();
        assert_eq!(
            details
                .messages
                .iter()
                .find(|message| message.id == "a00000")
                .unwrap()
                .artifacts
                .as_ref(),
            Some(&public_image)
        );
        db.conn()
            .execute(
                "UPDATE messages SET content='',thinking=NULL WHERE id='a00000'",
                [],
            )
            .unwrap();
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .find(|message| message.id == "a00000")
                .unwrap()
                .artifacts
                .as_ref(),
            Some(&public_image)
        );
    }

    #[test]
    fn timeline_reasoning_guard_tracks_writes_and_canonical_projection_precedence() {
        let (db, id) = fixture(2);
        let private = "first line\nsecond line";
        let legacy = serde_json::json!({"kind":"traceTimeline","items":[{"kind":"thinking","text":private}]});
        db.conn()
            .execute(
                "UPDATE messages SET content=?1,thinking=?2,artifacts_json=?3 WHERE id='a00001'",
                params![
                    "\u{a0}\tfirst line\r\nsecond line\u{3000}",
                    private,
                    legacy.to_string()
                ],
            )
            .unwrap();
        let cases = [
            (
                serde_json::json!([{"kind":"reply","text":"confirmed final answer"}]),
                false,
            ),
            (
                serde_json::json!([{"kind":"reply","text":"\t\r\n\u{a0}"}]),
                false,
            ),
            (
                serde_json::json!([{"kind":"thinking","text":"\t\r\n\u{a0}"}]),
                false,
            ),
            (serde_json::json!([{"kind":"status","text":""}]), false),
            (
                serde_json::json!([{"kind":"tool","toolCall":{"callId":"","toolName":""}}]),
                false,
            ),
            (
                serde_json::json!([{"kind":"skillSelection","skills":[{"id":"\t skill "}]}]),
                false,
            ),
            (
                serde_json::json!([{"kind":"unknown","text":"ignored"}]),
                true,
            ),
            (
                serde_json::json!([{"kind":"tool","toolCall":{"callId":1,"toolName":""}}]),
                true,
            ),
            (
                serde_json::json!([{"kind":"skillSelection","skills":[{"id":"\u{a0}\t"}]}]),
                true,
            ),
            (
                serde_json::json!(["not an item",{"kind":"reply","text":1}]),
                true,
            ),
            (
                serde_json::json!([{"kind":"thinking","text":private}]),
                true,
            ),
            (
                serde_json::json!([{"kind":"thinking","text":private},{"kind":"reply","text":"answer"}]),
                false,
            ),
        ];
        for (items, reasoning_only) in cases {
            let trace = serde_json::json!({"kind":"turnTrace","items":items});
            db.conn()
                .execute(
                    "UPDATE conversation_turns SET trace_json=?1 WHERE id='t00001'",
                    [trace.to_string()],
                )
                .unwrap();
            let page = db
                .conversation_timeline_page(&id, None, None, None, None)
                .unwrap();
            let message = page
                .messages
                .iter()
                .find(|message| message.id == "a00001")
                .unwrap();
            assert_eq!(message.content.is_empty(), reasoning_only, "trace {trace}");
            assert_eq!(
                message
                    .artifacts
                    .as_ref()
                    .and_then(|value| value.get("displayReasoningOnly"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                reasoning_only,
                "trace {trace}"
            );
        }
        // Legacy-only rows retain the same protection. A message edit must
        // invalidate the candidate independently from the trace classification.
        db.conn()
            .execute(
                "UPDATE conversation_turns SET trace_json=NULL WHERE id='t00001'",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "UPDATE messages SET content='a real final answer' WHERE id='a00001'",
                [],
            )
            .unwrap();
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .find(|message| message.id == "a00001")
                .unwrap()
                .content,
            "a real final answer"
        );
        db.conn()
            .execute("UPDATE messages SET content=thinking WHERE id='a00001'", [])
            .unwrap();
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert!(page
            .messages
            .iter()
            .find(|message| message.id == "a00001")
            .unwrap()
            .content
            .is_empty());
        db.conn()
            .execute(
                "UPDATE messages SET tool_calls_json=?1 WHERE id='a00001'",
                [r#"[{"id":"call-1","name":"read_file","arguments":"{}"}]"#],
            )
            .unwrap();
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .find(|message| message.id == "a00001")
                .unwrap()
                .content,
            private
        );
        db.conn()
            .execute(
                "UPDATE messages SET tool_calls_json=NULL,thinking=NULL WHERE id='a00001'",
                [],
            )
            .unwrap();
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .find(|message| message.id == "a00001")
                .unwrap()
                .content,
            private
        );
    }

    #[test]
    fn timeline_reasoning_guard_migration_backfills_and_tracks_inserted_records() {
        let connection = Connection::open_in_memory().unwrap();
        // A pre-v135 schema with existing records exercises the actual migration,
        // rather than emulating its write-time projections in the test.
        connection.execute_batch("CREATE TABLE messages(id TEXT PRIMARY KEY,conversation_id TEXT,role TEXT,content TEXT,thinking TEXT,artifacts_json TEXT,sort_order INTEGER);
            CREATE TABLE conversation_turns(id TEXT PRIMARY KEY,conversation_id TEXT,user_message_id TEXT,assistant_message_id TEXT,status TEXT,route_kind TEXT,trace_json TEXT,created_at TEXT);
            CREATE TABLE agent_task_runs(conversation_id TEXT,user_message_id TEXT,created_at TEXT,id TEXT);
            INSERT INTO messages VALUES('old','c','assistant','private','private','{\"kind\":\"traceTimeline\",\"items\":[{\"kind\":\"thinking\",\"text\":\"private\"}]}',0);
            INSERT INTO conversation_turns(id,trace_json) VALUES('old-turn','{\"kind\":\"turnTrace\",\"items\":[{\"kind\":\"thinking\",\"text\":\"private\"}]}');").unwrap();
        connection
            .execute_batch(include_str!(
                "../migrations/v135_conversation_timeline_indexes.sql"
            ))
            .unwrap();
        let read_candidate = |id: &str| {
            connection.query_row("SELECT display_reasoning_candidate,display_legacy_trace_flags FROM messages WHERE id=?1", [id], |row| Ok((row.get::<_,bool>(0)?,row.get::<_,Option<i64>>(1)?))).unwrap()
        };
        assert_eq!(read_candidate("old"), (true, Some(1)));
        assert_eq!(
            connection
                .query_row(
                    "SELECT display_trace_flags FROM conversation_turns WHERE id='old-turn'",
                    [],
                    |row| row.get::<_, Option<i64>>(0)
                )
                .unwrap(),
            Some(1)
        );
        connection.execute("INSERT INTO messages(id,role,content,thinking,artifacts_json) SELECT 'new',role,content,thinking,artifacts_json FROM messages WHERE id='old'", []).unwrap();
        assert_eq!(read_candidate("new"), (true, Some(1)));
        connection.execute("INSERT INTO conversation_turns(id,trace_json) SELECT 'new-turn',trace_json FROM conversation_turns WHERE id='old-turn'", []).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT display_trace_flags FROM conversation_turns WHERE id='new-turn'",
                    [],
                    |row| row.get::<_, Option<i64>>(0)
                )
                .unwrap(),
            Some(1)
        );
        connection.execute("UPDATE messages SET thinking='[reasoning content unavailable in local history]',content='[reasoning content unavailable in local history]',artifacts_json=NULL WHERE id='new'", []).unwrap();
        assert_eq!(read_candidate("new"), (false, None));
        connection.execute("INSERT INTO messages(id,role,content,thinking) VALUES('unicode-trim','assistant',?1,?2)", params!["\u{feff}first\r\nsecond\u{3000}","\u{85}first\nsecond\u{85}"]).unwrap();
        assert_eq!(read_candidate("unicode-trim"), (true, None));
        connection
            .execute(
                "UPDATE conversation_turns SET trace_json='{invalid' WHERE id='new-turn'",
                [],
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT display_trace_flags FROM conversation_turns WHERE id='new-turn'",
                    [],
                    |row| row.get::<_, Option<i64>>(0)
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn timeline_legacy_trace_summary_does_not_execute_json_parsers() {
        use rusqlite::functions::FunctionFlags;
        let (db, id) = fixture(2);
        let artifact = serde_json::json!({"kind":"traceTimeline","version":1,"items":[{"kind":"thinking","text":"large legacy trace ".repeat(100_000)}]});
        {
            let connection = db.conn();
            connection
                .execute("UPDATE conversation_turns SET trace_json=NULL", [])
                .unwrap();
            connection
                .execute(
                    "UPDATE messages SET artifacts_json=?1 WHERE id='a00001'",
                    [artifact.to_string()],
                )
                .unwrap();
            let kind: String = connection
                .query_row(
                    "SELECT display_artifact_kind FROM messages WHERE id='a00001'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(kind, "traceTimeline");
            // Any accidental json_valid/extract/remove on this read fails the
            // actual storage query. The derived kind was committed beforehand.
            for (name, arity) in [("json_valid", 1), ("json_extract", 2), ("json_remove", -1)] {
                connection
                    .create_scalar_function(
                        name,
                        arity,
                        FunctionFlags::SQLITE_UTF8
                            | FunctionFlags::SQLITE_DETERMINISTIC
                            | FunctionFlags::SQLITE_INNOCUOUS,
                        move |_| -> rusqlite::Result<i32> {
                            Err(rusqlite::Error::UserFunctionError(Box::new(
                                std::io::Error::other(format!(
                                    "summary parsed legacy trace through {name}"
                                )),
                            )))
                        },
                    )
                    .unwrap();
            }
        }
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert!(page
            .messages
            .iter()
            .all(|message| message.artifacts.is_none()));
        assert!(serde_json::to_vec(&page).unwrap().len() < 32_768);
        let detail = db.conversation_timeline_details(&id, "u00001").unwrap();
        assert_eq!(
            detail
                .messages
                .iter()
                .find(|message| message.id == "a00001")
                .unwrap()
                .artifacts
                .as_ref(),
            Some(&artifact)
        );
    }

    #[test]
    fn timeline_artifact_kind_tracks_writes_and_keeps_final_image_previews() {
        let (db, id) = fixture(2);
        db.conn()
            .execute("UPDATE conversation_turns SET trace_json=NULL", [])
            .unwrap();
        let image =
            serde_json::json!({"kind":"generatedImage","dataUrl":"data:image/png;base64,fixture"});
        db.conn()
            .execute(
                "UPDATE messages SET artifacts_json=?1 WHERE id='a00001'",
                [image.to_string()],
            )
            .unwrap();
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert_eq!(
            page.messages
                .iter()
                .find(|message| message.id == "a00001")
                .unwrap()
                .artifacts
                .as_ref(),
            Some(&image)
        );
        db.conn()
            .execute(
                "UPDATE messages SET artifacts_json='{broken trace' WHERE id='a00001'",
                [],
            )
            .unwrap();
        let kind: String = db
            .conn()
            .query_row(
                "SELECT display_artifact_kind FROM messages WHERE id='a00001'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "__invalid_json__");
        let page = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert!(page
            .messages
            .iter()
            .find(|message| message.id == "a00001")
            .unwrap()
            .artifacts
            .is_none());
        assert!(page.entries[1].has_details);
        assert!(db.conversation_timeline_details(&id, "u00001").is_err());
        db.conn()
            .execute(
                "UPDATE messages SET artifacts_json=NULL WHERE id='a00001'",
                [],
            )
            .unwrap();
        let kind: Option<String> = db
            .conn()
            .query_row(
                "SELECT display_artifact_kind FROM messages WHERE id='a00001'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(kind.is_none());
        db.conn().execute("INSERT INTO messages(id,conversation_id,role,content,artifacts_json,sort_order) VALUES('image-on-insert',?1,'assistant','image',?2,30)",params![id,image.to_string()]).unwrap();
        let kind: String = db
            .conn()
            .query_row(
                "SELECT display_artifact_kind FROM messages WHERE id='image-on-insert'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kind, "generatedImage");
    }

    #[test]
    fn timeline_early_entry_queries_seek_both_bounds_independently_of_history_size() {
        use rusqlite::{types::Value, StatementStatus};
        let queries = [
            controls_query(true),
            details_query(true),
            last_assistant_query(true),
            has_details_query(true),
            compaction_markers_query(true, true),
        ];
        let mut costs = Vec::new();
        for count in [100, 10_000] {
            let (db, id) = fixture(count);
            let connection = db.conn();
            let mut measurements = Vec::new();
            for sql in &queries {
                let mut arguments = vec![
                    Value::Text(id.clone()),
                    Value::Integer(10),
                    Value::Text("u00001".into()),
                    Value::Integer(20),
                    Value::Text("u00002".into()),
                ];
                if sql.contains("?6") {
                    arguments.push(Value::Text("a00001".into()));
                }
                let mut statement = connection.prepare(sql).unwrap();
                let rows = statement
                    .query_map(rusqlite::params_from_iter(arguments.iter()), |_| Ok(()))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                let steps = statement.get_status(StatementStatus::VmStep);
                assert_eq!(statement.get_status(StatementStatus::Sort), 0, "{sql}");
                let plan = connection
                    .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                    .unwrap()
                    .query_map(rusqlite::params_from_iter(arguments.iter()), |row| {
                        row.get::<_, String>(3)
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
                    .join(" ");
                assert!(
                    plan.contains('>') && plan.contains('<'),
                    "query must seek both entry bounds: {plan}; {sql}"
                );
                assert!(!plan.contains("TEMP B-TREE"), "{plan}");
                assert!(
                    steps < 256,
                    "early entry scanned beyond its range: turns={count}, steps={steps}; {sql}"
                );
                measurements.push((rows.len(), steps));
            }
            costs.push(measurements);
        }
        for (small, large) in costs[0].iter().zip(&costs[1]) {
            assert_eq!(small.0, large.0);
            assert!(
                large.1 <= small.1 + 32,
                "100 vs 10k turns: {small:?} -> {large:?}"
            );
        }
        println!(
            "timeline early-entry query (rows, VM steps), 100 turns: {:?}; 10k turns: {:?}",
            costs[0], costs[1]
        );
    }

    #[test]
    fn timeline_tail_cost_is_bounded_and_trace_is_lazy_at_10k_turns() {
        let (small, id) = fixture(100);
        let first = small
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        let first_bytes = serde_json::to_vec(&first).unwrap().len();
        let (large, id) = fixture(10_000);
        let tail = large
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert_eq!(tail.entries.len(), 20);
        assert_eq!(tail.messages.len(), 40);
        assert!(tail
            .messages
            .iter()
            .all(|message| message.thinking.is_none()));
        assert!(tail.turns.iter().all(|turn| turn.trace.is_none()));
        assert!(tail.entries.iter().all(|entry| entry.has_details));
        assert_eq!(tail.task_runs.last().unwrap().user_message_id, "u09999");
        assert!(
            serde_json::to_vec(&tail)
                .unwrap()
                .len()
                .abs_diff(first_bytes)
                < 128
        );
        assert!(large.conversation_timeline_details(&id, "u09999").is_err());
        let connection = large.conn();
        let plan = connection.prepare(&format!("EXPLAIN QUERY PLAN SELECT sort_order,id FROM messages INDEXED BY idx_messages_timeline_roots WHERE conversation_id=?1 AND {ROOT_PREDICATE} ORDER BY sort_order DESC,id DESC LIMIT 20")).unwrap().query_map([&id],|row|row.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap().join(" ");
        assert!(plan.contains("idx_messages_timeline_roots"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }

    #[test]
    fn timeline_orphan_import_pages_preserve_details_and_exclusive_ranges() {
        let db = Database::open_memory().unwrap();
        let conversation = db
            .create_conversation(&CreateConversationInput {
                provider: "openai".into(),
                model: "test".into(),
                system_prompt: None,
                collection_context: None,
                project_id: None,
                persona_id: None,
            })
            .unwrap();
        for index in 0..6 {
            db.conn().execute("INSERT INTO messages(id,conversation_id,role,content,thinking,sort_order) VALUES(?1,?2,'assistant','imported answer','imported thinking',?3)",params![format!("orphan{index}"),conversation.id,index]).unwrap();
        }
        let tail = db
            .conversation_timeline_page(&conversation.id, None, None, None, Some(2))
            .unwrap();
        assert_eq!(tail.messages.len(), 2);
        assert_eq!(
            tail.messages[0].thinking.as_deref(),
            Some("imported thinking")
        );
        assert!(tail.entries.is_empty());
        let before = db
            .conversation_timeline_page(
                &conversation.id,
                tail.oldest_cursor.as_ref(),
                None,
                None,
                Some(2),
            )
            .unwrap();
        assert_eq!(before.messages[0].id, "orphan2");
        assert_eq!(before.range.unwrap().before.unwrap().message_id, "orphan4");
        assert!(before.has_more_before && before.has_more_after);
    }

    #[test]
    fn timeline_revision_distinguishes_same_second_trace_writes() {
        let (db, id) = fixture(2);
        let before = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        db.conn()
            .execute(
                "UPDATE conversation_turns SET trace_json=NULL WHERE user_message_id='u00001'",
                [],
            )
            .unwrap();
        let after = db
            .conversation_timeline_page(&id, None, None, None, None)
            .unwrap();
        assert_eq!(before.turns[1].updated_at, after.turns[1].updated_at);
        assert_ne!(
            before.entries[1].detail_revision,
            after.entries[1].detail_revision
        );
        assert_eq!(
            before.entries[0].detail_revision,
            after.entries[0].detail_revision
        );
    }

    #[test]
    fn timeline_keysets_preserve_controls_and_reject_foreign_anchors() {
        let (db, id) = fixture(8);
        {
            let connection = db.conn();
            connection.execute("INSERT INTO messages(id,conversation_id,role,content,sort_order,artifacts_json) VALUES('steering',?1,'user','steer',65,'{\"kind\":\"steering\"}'),('question',?1,'user','reply',66,'{\"kind\":\"questionResponse\"}')",[&id]).unwrap();
            connection
                .execute(
                    "UPDATE conversation_turns SET trace_json=NULL WHERE user_message_id='u00006'",
                    [],
                )
                .unwrap();
        }
        let tail = db
            .conversation_timeline_page(&id, None, None, None, Some(2))
            .unwrap();
        assert_eq!(
            tail.entries
                .iter()
                .map(|entry| entry.anchor.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["u00006", "u00007"]
        );
        assert!(tail.messages.iter().any(|message| message.id == "steering"));
        assert!(tail.messages.iter().any(|message| message.id == "question"));
        let earlier = db
            .conversation_timeline_page(&id, tail.oldest_cursor.as_ref(), None, None, Some(2))
            .unwrap();
        assert_eq!(earlier.oldest_cursor.as_ref().unwrap().message_id, "u00004");
        assert_eq!(
            earlier
                .range
                .as_ref()
                .unwrap()
                .before
                .as_ref()
                .unwrap()
                .message_id,
            "u00006"
        );
        let suffix = db
            .conversation_timeline_page(&id, None, earlier.newest_cursor.as_ref(), None, Some(50))
            .unwrap();
        assert_eq!(suffix.oldest_cursor.as_ref().unwrap().message_id, "u00005");
        let around = db
            .conversation_timeline_page(&id, None, None, Some("tool00002"), Some(2))
            .unwrap();
        assert_eq!(around.oldest_cursor.as_ref().unwrap().message_id, "u00002");
        assert!(around.has_more_after);
        assert!(db
            .conversation_timeline_page(&id, None, None, Some("foreign"), None)
            .is_err());
        assert!(db
            .conversation_timeline_page(
                &id,
                tail.oldest_cursor.as_ref(),
                tail.newest_cursor.as_ref(),
                None,
                None
            )
            .is_err());
        let detail = db.conversation_timeline_details(&id, "u00006").unwrap();
        assert_eq!(detail.messages.len(), 5);
        assert!(detail
            .messages
            .iter()
            .any(|message| message.id == "tool00006" && message.content.len() == 4096));
        assert_eq!(
            detail
                .messages
                .iter()
                .find(|message| message.id == "a00006")
                .unwrap()
                .thinking
                .as_deref(),
            Some("hidden thinking")
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTimelineRange {
    pub from: ConversationTimelineCursor,
    pub before: Option<ConversationTimelineCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTimelineEntry {
    pub anchor: ConversationTimelineCursor,
    pub turn_id: Option<String>,
    pub has_details: bool,
    pub detail_revision: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTimelinePage {
    pub conversation: Conversation,
    pub messages: Vec<ConversationMessage>,
    pub turns: Vec<ConversationTurn>,
    pub entries: Vec<ConversationTimelineEntry>,
    pub task_runs: Vec<AgentTaskRun>,
    pub range: Option<ConversationTimelineRange>,
    pub oldest_cursor: Option<ConversationTimelineCursor>,
    pub newest_cursor: Option<ConversationTimelineCursor>,
    pub has_more_before: bool,
    pub has_more_after: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTimelineDetails {
    pub anchor_id: String,
    pub messages: Vec<ConversationMessage>,
    pub turns: Vec<ConversationTurn>,
    pub range: ConversationTimelineRange,
    pub detail_revision: String,
}

fn cursor_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationTimelineCursor> {
    Ok(ConversationTimelineCursor {
        sort_order: row.get(0)?,
        message_id: row.get(1)?,
    })
}

fn root_before(
    connection: &Connection,
    conversation_id: &str,
    cursor: &ConversationTimelineCursor,
    inclusive: bool,
) -> Result<Option<ConversationTimelineCursor>, CoreError> {
    let operator = if inclusive { "<=" } else { "<" };
    Ok(connection.query_row(&format!("SELECT sort_order,id FROM messages INDEXED BY idx_messages_timeline_roots WHERE conversation_id=?1 AND {ROOT_PREDICATE} AND (sort_order,id) {operator} (?2,?3) ORDER BY sort_order DESC,id DESC LIMIT 1"), params![conversation_id,cursor.sort_order,cursor.message_id], cursor_from_row).optional()?)
}

fn root_after(
    connection: &Connection,
    conversation_id: &str,
    cursor: &ConversationTimelineCursor,
) -> Result<Option<ConversationTimelineCursor>, CoreError> {
    Ok(connection.query_row(&format!("SELECT sort_order,id FROM messages INDEXED BY idx_messages_timeline_roots WHERE conversation_id=?1 AND {ROOT_PREDICATE} AND (sort_order,id) > (?2,?3) ORDER BY sort_order,id LIMIT 1"), params![conversation_id,cursor.sort_order,cursor.message_id], cursor_from_row).optional()?)
}

fn read_turn(
    connection: &Connection,
    conversation_id: &str,
    anchor_id: &str,
    details: bool,
) -> Result<Option<TimelineTurn>, CoreError> {
    let trace_column = if details { "trace_json" } else { "NULL" };
    let sql = format!("SELECT id,conversation_id,launch_project_id,user_message_id,assistant_message_id,status,route_kind,{trace_column},created_at,updated_at,finished_at,trace_json IS NOT NULL,display_revision,display_trace_flags FROM conversation_turns WHERE conversation_id=?1 AND user_message_id=?2 ORDER BY created_at DESC,id DESC LIMIT 1");
    let row = connection
        .query_row(&sql, params![conversation_id, anchor_id], |row| {
            Ok((
                ConversationTurn {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    launch_project_id: row.get(2)?,
                    user_message_id: row.get(3)?,
                    assistant_message_id: row.get(4)?,
                    status: row.get(5)?,
                    route_kind: row.get(6)?,
                    trace: None,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    finished_at: row.get(10)?,
                },
                row.get::<_, Option<String>>(7)?,
                row.get::<_, bool>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, Option<i64>>(13)?,
            ))
        })
        .optional()?;
    row.map(|(mut turn, trace, has_trace, revision, trace_flags)| {
        turn.trace = trace
            .map(|value| serde_json::from_str(&value))
            .transpose()?;
        Ok((
            conversation_turn_for_display(turn),
            has_trace,
            revision,
            trace_flags,
        ))
    })
    .transpose()
}

fn read_message(
    connection: &Connection,
    conversation_id: &str,
    message_id: &str,
    has_turn_trace: bool,
    canonical_trace_flags: Option<i64>,
    summary: bool,
) -> Result<Option<ConversationMessage>, CoreError> {
    let thinking = if summary { "NULL" } else { "thinking" };
    let artifacts = if summary {
        "CASE WHEN artifacts_json IS NULL OR display_artifact_kind IN ('traceTimeline','__invalid_json__') THEN NULL WHEN json_valid(artifacts_json) THEN json_remove(artifacts_json,'$.providerTurnEnvelope','$.providerReplayBoundary','$.reasoningEnvelope','$.llmContextContent') ELSE NULL END"
    } else {
        "artifacts_json"
    };
    let row = connection.query_row(&format!("SELECT id,conversation_id,role,content,tool_call_id,tool_calls_json,{artifacts},token_count,created_at,sort_order,{thinking},image_attachments_json,display_reasoning_candidate,display_legacy_trace_flags FROM messages WHERE conversation_id=?1 AND id=?2"), params![conversation_id,message_id], |row| Ok((
        row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?, row.get::<_,String>(3)?, row.get::<_,Option<String>>(4)?, row.get::<_,Option<String>>(5)?, row.get::<_,Option<String>>(6)?, row.get::<_,u32>(7)?, row.get::<_,String>(8)?, row.get::<_,i64>(9)?, row.get::<_,Option<String>>(10)?, row.get::<_,Option<String>>(11)?,
        row.get::<_,bool>(12)?, row.get::<_,Option<i64>>(13)?,
    ))).optional()?;
    row.map(
        |(
            id,
            conversation_id,
            role,
            content,
            tool_call_id,
            tool_calls,
            artifacts,
            token_count,
            created_at,
            sort_order,
            thinking,
            images,
            reasoning_candidate,
            legacy_trace_flags,
        )| {
            let mut message = conversation_message_for_display_with_turn_trace(
                ConversationMessage {
                    id,
                    conversation_id,
                    role: str_to_role(&role),
                    content,
                    tool_call_id,
                    tool_calls: tool_calls
                        .map(|value| serde_json::from_str::<Vec<ToolCallRequest>>(&value))
                        .transpose()?
                        .unwrap_or_default(),
                    artifacts: artifacts
                        .map(|value| serde_json::from_str(&value))
                        .transpose()?,
                    token_count,
                    created_at,
                    sort_order,
                    thinking: crate::llm::reasoning_replay::sanitize_reasoning_text(
                        thinking.as_deref(),
                    ),
                    image_attachments: images.and_then(|value| {
                        serde_json::from_str::<Vec<ImageAttachment>>(&value).ok()
                    }),
                },
                has_turn_trace || summary,
            );
            // A display-only marker is authoritative only when this projection
            // derives it. Never trust an identically named persisted field.
            if let Some(artifacts) = message
                .artifacts
                .as_mut()
                .and_then(serde_json::Value::as_object_mut)
            {
                artifacts.remove("displayReasoningOnly");
            }
            // The full renderer guard needs thinking and projected trace items.
            // Their write-time classification preserves that guard while summary
            // reads leave both large/private payloads in storage. A valid canonical
            // trace takes precedence, including one with an explicit reply.
            if summary
                && message.role == crate::llm::Role::Assistant
                && message.tool_calls.is_empty()
                && reasoning_candidate
                && canonical_trace_flags.or(legacy_trace_flags) == Some(1)
            {
                message.content.clear();
                let artifacts = message.artifacts.get_or_insert_with(
                    || serde_json::json!({"kind":"assistantArtifacts","version":2}),
                );
                if let Some(artifacts) = artifacts.as_object_mut() {
                    artifacts.insert("displayReasoningOnly".into(), serde_json::Value::Bool(true));
                }
            }
            Ok(message)
        },
    )
    .transpose()
}

fn insert_message(
    messages: &mut BTreeMap<(i64, String), ConversationMessage>,
    message: Option<ConversationMessage>,
) {
    if let Some(message) = message {
        messages.insert((message.sort_order, message.id.clone()), message);
    }
}

// Optional OR predicates prevent SQLite from seeking the upper bound, making
// an early entry walk every later message. Keep absent parameter slots bound
// without putting an OR around a real cursor comparison.
fn before_bound(has_before: bool) -> &'static str {
    if has_before {
        "(sort_order,id)<(?4,?5)"
    } else {
        "(?4 IS NULL AND ?5 IS NULL)"
    }
}

fn controls_query(has_before: bool) -> String {
    format!("SELECT id FROM messages WHERE conversation_id=?1 AND role='user' AND (sort_order,id)>(?2,?3) AND {} ORDER BY sort_order DESC,id DESC LIMIT 32", before_bound(has_before))
}

fn details_query(has_before: bool) -> String {
    format!("SELECT id FROM messages WHERE conversation_id=?1 AND (sort_order,id)>=(?2,?3) AND {} ORDER BY sort_order,id", before_bound(has_before))
}

fn last_assistant_query(has_before: bool) -> String {
    format!("SELECT id FROM messages WHERE conversation_id=?1 AND role='assistant' AND (tool_calls_json IS NULL OR tool_calls_json='[]') AND (sort_order,id)>(?2,?3) AND {} ORDER BY sort_order DESC,id DESC LIMIT 1", before_bound(has_before))
}

fn has_details_query(has_before: bool) -> String {
    format!("SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND (sort_order,id)>(?2,?3) AND {} AND (?6 IS NULL OR id<>?6) LIMIT 1)", before_bound(has_before))
}

fn compaction_markers_query(has_from: bool, has_before: bool) -> String {
    let from = if has_from {
        "(sort_order,id)>=(?2,?3)"
    } else {
        "(?2 IS NULL AND ?3 IS NULL)"
    };
    format!("SELECT id FROM messages WHERE conversation_id=?1 AND role='system' AND {from} AND {} AND (lower(content) LIKE '%earlier conversation context%' OR lower(content) LIKE '%auto-compacted%' OR lower(content) LIKE '%compacted context%') ORDER BY sort_order DESC,id DESC LIMIT 64", before_bound(has_before))
}

fn last_assistant(
    connection: &Connection,
    conversation_id: &str,
    from: &ConversationTimelineCursor,
    before: Option<&ConversationTimelineCursor>,
) -> Result<Option<String>, CoreError> {
    Ok(connection
        .query_row(
            &last_assistant_query(before.is_some()),
            params![
                conversation_id,
                from.sort_order,
                from.message_id,
                before.map(|cursor| cursor.sort_order),
                before.map(|cursor| cursor.message_id.as_str())
            ],
            |row| row.get(0),
        )
        .optional()?)
}

fn detail_revision(
    turn: Option<&TimelineTurn>,
    before: Option<&ConversationTimelineCursor>,
) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        turn.map(|(turn, _, _, _)| turn.id.as_str())
            .unwrap_or_default(),
        turn.map(|(_, _, revision, _)| *revision)
            .unwrap_or_default(),
        turn.map(|(turn, _, _, _)| turn.status.as_str())
            .unwrap_or_default(),
        turn.and_then(|(turn, _, _, _)| turn.assistant_message_id.as_deref())
            .unwrap_or_default(),
        before
            .map(|cursor| cursor.message_id.as_str())
            .unwrap_or_default()
    )
}

impl Database {
    /// Pages complete display entries, not provider replay. No trace JSON or tool
    /// output is decoded until conversation_timeline_details is requested.
    pub fn conversation_timeline_page(
        &self,
        conversation_id: &str,
        before: Option<&ConversationTimelineCursor>,
        after: Option<&ConversationTimelineCursor>,
        anchor_message_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<ConversationTimelinePage, CoreError> {
        if usize::from(before.is_some())
            + usize::from(after.is_some())
            + usize::from(anchor_message_id.is_some())
            > 1
        {
            return Err(CoreError::InvalidInput(
                "Choose one conversation page direction".into(),
            ));
        }
        let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(2, MAX_PAGE_SIZE);
        let conversation = self.get_conversation(conversation_id)?;
        let connection = self.conn();
        let transaction = connection.unchecked_transaction()?;
        let anchor = if let Some(id) = anchor_message_id {
            let cursor = transaction
                .query_row(
                    "SELECT sort_order,id FROM messages WHERE conversation_id=?1 AND id=?2",
                    params![conversation_id, id],
                    cursor_from_row,
                )
                .optional()?
                .ok_or_else(|| CoreError::NotFound(format!("Conversation message {id}")))?;
            root_before(&transaction, conversation_id, &cursor, true)?.or(Some(cursor))
        } else {
            None
        };
        let forward = after.or(anchor.as_ref());
        let (condition, order, cursor) = if let Some(cursor) = forward {
            ("AND (sort_order,id) >= (?2,?3)", "ASC", Some(cursor))
        } else if let Some(cursor) = before {
            ("AND (sort_order,id) < (?2,?3)", "DESC", Some(cursor))
        } else {
            ("AND (?2 IS NULL OR ?3 IS NULL)", "DESC", None)
        };
        let sql = format!("SELECT sort_order,id FROM messages INDEXED BY idx_messages_timeline_roots WHERE conversation_id=?1 AND {ROOT_PREDICATE} {condition} ORDER BY sort_order {order},id {order} LIMIT ?4");
        let mut roots = transaction
            .prepare(&sql)?
            .query_map(
                params![
                    conversation_id,
                    cursor.map(|cursor| cursor.sort_order),
                    cursor.map(|cursor| cursor.message_id.as_str()),
                    limit as i64
                ],
                cursor_from_row,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        if forward.is_none() {
            roots.reverse();
        }

        // Old imports can contain assistant-only history. Keep that case bounded
        // too, and retain ordinary keyset cursors instead of falling back to all rows.
        let orphan_page = roots.is_empty() && transaction.query_row(&format!("SELECT NOT EXISTS(SELECT 1 FROM messages INDEXED BY idx_messages_timeline_roots WHERE conversation_id=?1 AND {ROOT_PREDICATE} LIMIT 1)"), [conversation_id], |row| row.get::<_,bool>(0))?;
        if orphan_page {
            let sql = format!("SELECT sort_order,id FROM messages WHERE conversation_id=?1 {condition} ORDER BY sort_order {order},id {order} LIMIT ?4");
            roots = transaction
                .prepare(&sql)?
                .query_map(
                    params![
                        conversation_id,
                        cursor.map(|cursor| cursor.sort_order),
                        cursor.map(|cursor| cursor.message_id.as_str()),
                        limit as i64
                    ],
                    cursor_from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            if forward.is_none() {
                roots.reverse();
            }
        }
        let oldest_cursor = roots.first().cloned();
        let newest_cursor = roots.last().cloned();
        let mut messages = BTreeMap::new();
        let mut turns = Vec::new();
        let mut entries = Vec::new();
        let mut end_before = None;
        for root in &roots {
            let next = if orphan_page {
                transaction.query_row("SELECT sort_order,id FROM messages WHERE conversation_id=?1 AND (sort_order,id)>(?2,?3) ORDER BY sort_order,id LIMIT 1",params![conversation_id,root.sort_order,root.message_id],cursor_from_row).optional()?
            } else {
                root_after(&transaction, conversation_id, root)?
            };
            let turn = if orphan_page {
                None
            } else {
                read_turn(&transaction, conversation_id, &root.message_id, false)?
            };
            let has_trace = turn.as_ref().is_some_and(|(_, has_trace, _, _)| *has_trace);
            let final_id = if let Some((turn, _, _, _)) = turn.as_ref() {
                turn.assistant_message_id.clone()
            } else if orphan_page {
                None
            } else {
                last_assistant(&transaction, conversation_id, root, next.as_ref())?
            };
            // Assistant-only legacy imports have no expandable user entry. Read
            // their bounded message page in full so old reasoning/artifacts are
            // still available, without reading the rest of the conversation.
            insert_message(
                &mut messages,
                read_message(
                    &transaction,
                    conversation_id,
                    &root.message_id,
                    false,
                    None,
                    !orphan_page,
                )?,
            );
            let mut final_has_details = false;
            if let Some(id) = final_id.as_deref() {
                final_has_details = transaction.query_row("SELECT thinking IS NOT NULL OR artifacts_json IS NOT NULL FROM messages WHERE conversation_id=?1 AND id=?2",params![conversation_id,id],|row|row.get::<_,bool>(0)).optional()?.unwrap_or(false);
                insert_message(
                    &mut messages,
                    read_message(
                        &transaction,
                        conversation_id,
                        id,
                        has_trace,
                        turn.as_ref().and_then(|(_, _, _, flags)| *flags),
                        true,
                    )?,
                );
            }
            if !orphan_page {
                // Steering and question-response artifacts must retain their exact
                // durable position. The role/order index skips tool payload rows.
                // A very long single turn can have many steering replies. Keep
                // its latest controls in the summary; expansion restores all.
                let controls = transaction
                    .prepare(&controls_query(next.is_some()))?
                    .query_map(
                        params![
                            conversation_id,
                            root.sort_order,
                            root.message_id,
                            next.as_ref().map(|cursor| cursor.sort_order),
                            next.as_ref().map(|cursor| cursor.message_id.as_str())
                        ],
                        |row| row.get::<_, String>(0),
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                for id in controls {
                    insert_message(
                        &mut messages,
                        read_message(&transaction, conversation_id, &id, false, None, true)?,
                    );
                }
                let has_details = has_trace
                    || final_has_details
                    || transaction.query_row(
                        &has_details_query(next.is_some()),
                        params![
                            conversation_id,
                            root.sort_order,
                            root.message_id,
                            next.as_ref().map(|cursor| cursor.sort_order),
                            next.as_ref().map(|cursor| cursor.message_id.as_str()),
                            final_id
                        ],
                        |row| row.get::<_, bool>(0),
                    )?;
                entries.push(ConversationTimelineEntry {
                    anchor: root.clone(),
                    turn_id: turn.as_ref().map(|(turn, _, _, _)| turn.id.clone()),
                    has_details,
                    detail_revision: detail_revision(turn.as_ref(), next.as_ref()),
                });
            }
            if let Some((turn, _, _, _)) = turn {
                turns.push(turn);
            }
            end_before = next;
        }
        // Include visible compaction markers, including a leading marker on the
        // oldest page. Runtime system rows stay out of the initial display read.
        let has_more_before = if let Some(first) = oldest_cursor.as_ref() {
            if orphan_page {
                transaction.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND (sort_order,id)<(?2,?3) LIMIT 1)",params![conversation_id,first.sort_order,first.message_id],|row|row.get(0))?
            } else {
                root_before(&transaction, conversation_id, first, false)?.is_some()
            }
        } else {
            false
        };
        if let Some(first) = oldest_cursor.as_ref() {
            let system_ids = transaction
                .prepare(&compaction_markers_query(
                    has_more_before,
                    end_before.is_some(),
                ))?
                .query_map(
                    params![
                        conversation_id,
                        has_more_before.then_some(first.sort_order),
                        has_more_before.then_some(first.message_id.as_str()),
                        end_before.as_ref().map(|cursor| cursor.sort_order),
                        end_before.as_ref().map(|cursor| cursor.message_id.as_str())
                    ],
                    |row| row.get::<_, String>(0),
                )?
                .collect::<Result<Vec<_>, _>>()?;
            for id in system_ids {
                insert_message(
                    &mut messages,
                    read_message(&transaction, conversation_id, &id, false, None, true)?,
                );
            }
        }
        let has_more_after = if let Some(last) = newest_cursor.as_ref() {
            if orphan_page {
                transaction.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND (sort_order,id)>(?2,?3) LIMIT 1)",params![conversation_id,last.sort_order,last.message_id],|row|row.get(0))?
            } else {
                end_before.is_some()
            }
        } else {
            false
        };
        // Durable message order distinguishes turns created in the same second.
        // Seek the final two user anchors first so this remains bounded by the
        // displayed tail instead of sorting every historical task run.
        let run_ids = transaction.prepare(&format!("WITH latest_users AS (SELECT id,sort_order FROM messages INDEXED BY idx_messages_timeline_roots WHERE conversation_id=?1 AND {ROOT_PREDICATE} ORDER BY sort_order DESC,id DESC LIMIT 2) SELECT r.id FROM latest_users u JOIN agent_task_runs r ON r.conversation_id=?1 AND r.user_message_id=u.id ORDER BY u.sort_order DESC,u.id DESC,r.created_at DESC,r.id DESC LIMIT 2"))?.query_map([conversation_id], |row|row.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
        let mut task_runs = run_ids
            .into_iter()
            .map(|id| Database::get_agent_task_run_on_connection(&transaction, &id))
            .collect::<Result<Vec<_>, _>>()?;
        task_runs.reverse();
        let range = oldest_cursor.clone().map(|from| ConversationTimelineRange {
            from,
            before: end_before,
        });
        let page = ConversationTimelinePage {
            conversation,
            messages: messages.into_values().collect(),
            turns,
            entries,
            task_runs,
            range,
            oldest_cursor,
            newest_cursor,
            has_more_before,
            has_more_after,
        };
        transaction.commit()?;
        Ok(page)
    }

    pub fn conversation_timeline_details(
        &self,
        conversation_id: &str,
        anchor_message_id: &str,
    ) -> Result<ConversationTimelineDetails, CoreError> {
        let connection = self.conn();
        let transaction = connection.unchecked_transaction()?;
        let anchor = transaction.query_row(&format!("SELECT sort_order,id FROM messages WHERE conversation_id=?1 AND id=?2 AND {ROOT_PREDICATE}"),params![conversation_id,anchor_message_id],cursor_from_row).optional()?.ok_or_else(||CoreError::NotFound(format!("Conversation entry {anchor_message_id}")))?;
        let before = root_after(&transaction, conversation_id, &anchor)?;
        let turn = read_turn(&transaction, conversation_id, anchor_message_id, true)?;
        let has_trace = turn
            .as_ref()
            .is_some_and(|(_, _, _, flags)| flags.is_some());
        let message_ids = transaction
            .prepare(&details_query(before.is_some()))?
            .query_map(
                params![
                    conversation_id,
                    anchor.sort_order,
                    anchor.message_id,
                    before.as_ref().map(|cursor| cursor.sort_order),
                    before.as_ref().map(|cursor| cursor.message_id.as_str())
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let final_id = turn
            .as_ref()
            .and_then(|(turn, _, _, _)| turn.assistant_message_id.as_deref());
        let mut messages = Vec::new();
        for id in message_ids {
            if let Some(message) = read_message(
                &transaction,
                conversation_id,
                &id,
                has_trace && final_id == Some(id.as_str()),
                None,
                false,
            )? {
                messages.push(message);
            }
        }
        let detail_revision = detail_revision(turn.as_ref(), before.as_ref());
        let details = ConversationTimelineDetails {
            anchor_id: anchor_message_id.to_string(),
            messages,
            turns: turn.map(|(turn, _, _, _)| vec![turn]).unwrap_or_default(),
            range: ConversationTimelineRange {
                from: anchor,
                before,
            },
            detail_revision,
        };
        transaction.commit()?;
        Ok(details)
    }
}
