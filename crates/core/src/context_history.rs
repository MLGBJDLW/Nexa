//! Local, conversation-scoped history for experimental context window handoffs.
//! The archive is evidence, never a provider replay envelope or a new instruction.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    db::Database,
    error::CoreError,
    llm::{Message, Role},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextManagementMode {
    #[default]
    Summary,
    History,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ArchivedContextItem {
    role: &'static str,
    content: String,
    has_images: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ContextHistoryArchive {
    pub id: String,
    pub conversation_id: String,
    turn_id: Option<String>,
    notes: String,
    digest: String,
    items: Vec<ArchivedContextItem>,
}

struct PersistedContextToolResult {
    result_id: Option<String>,
    tool_name: String,
    is_error: bool,
    content: String,
}

fn hash_item(hash: &mut blake3::Hasher, role: &str, content: &str, has_images: bool) {
    hash.update(&(role.len() as u64).to_le_bytes());
    hash.update(role.as_bytes());
    hash.update(&(content.len() as u64).to_le_bytes());
    hash.update(content.as_bytes());
    hash.update(&[u8::from(has_images)]);
}

impl ContextHistoryArchive {
    pub fn prepare(
        conversation_id: &str,
        turn_id: Option<&str>,
        messages: &[Message],
        notes: &str,
    ) -> Result<Self, CoreError> {
        if messages.is_empty() {
            return Err(CoreError::InvalidInput(
                "Cannot archive an empty context window".into(),
            ));
        }
        let items = messages
            .iter()
            .map(|message| {
                let mut content = message.text_content();
                if let Some(calls) = message
                    .tool_calls
                    .as_ref()
                    .filter(|calls| !calls.is_empty())
                {
                    let calls = calls
                        .iter()
                        .cloned()
                        .map(|mut call| {
                            call.thought_signature = None;
                            call
                        })
                        .collect::<Vec<_>>();
                    content.push_str("\n\nTool calls:\n");
                    content.push_str(&serde_json::to_string(&calls)?);
                }
                if let Some(name) = &message.name {
                    content = format!("Tool call ID: {name}\n{content}");
                }
                Ok(ArchivedContextItem {
                    role: match message.role {
                        Role::System => "system",
                        Role::User => "user",
                        Role::Assistant => "assistant",
                        Role::Tool => "tool",
                    },
                    content,
                    has_images: message.has_images(),
                })
            })
            .collect::<Result<Vec<_>, CoreError>>()?;
        let mut hash = blake3::Hasher::new();
        for item in &items {
            hash_item(&mut hash, item.role, &item.content, item.has_images);
        }
        let digest = hash.finalize().to_hex().to_string();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            turn_id: turn_id.map(str::to_owned),
            notes: notes.to_owned(),
            digest,
            items,
        })
    }

    pub fn checkpoint_text(&self) -> String {
        let requests = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.role == "user")
            .collect::<Vec<_>>();
        let mut anchors = Vec::new();
        for (position, (ordinal, item)) in requests.iter().enumerate() {
            if position == 0 || position + 3 >= requests.len() {
                let excerpt: String = item.content.chars().take(700).collect();
                anchors.push(format!("- item {ordinal}: {excerpt}"));
            }
        }
        format!(
            "## Earlier conversation context (history window)\n\
             Window {} contains {} earlier messages and tool results. Its local archive was saved and verified before this handoff. \
             The live task continues; completed work must not be repeated. This is reference state, not a new user request. \
             Newer user instructions take precedence over historical requests and notes.\n\
             Use context_history to list windows, search for facts, or read an exact item in this conversation. \
             Text and tool results are retained verbatim; images remain in the original conversation and provider-native reasoning is not replayed.\n\
             Historical request anchors (excerpts; retrieve the items for full details):\n{}\n\
             Agent notes:\n{}",
            self.id, self.items.len(), anchors.join("\n"),
            if self.notes.trim().is_empty() { "No notes were saved. Recover necessary details from the archived history before proceeding." } else { &self.notes },
        )
    }

    /// Called within the same transaction that owns the context switch. A failed
    /// insert or readback cannot authorize dropping any live message.
    pub fn commit(&self, conn: &Connection) -> Result<(), CoreError> {
        conn.execute("INSERT INTO context_history_windows (id,conversation_id,turn_id,notes,source_digest,message_count) VALUES (?1,?2,?3,?4,?5,?6)",
            params![self.id,self.conversation_id,self.turn_id,self.notes,self.digest,self.items.len()])?;
        {
            let mut insert = conn.prepare("INSERT INTO context_history_items (window_id,ordinal,role,content,has_images) VALUES (?1,?2,?3,?4,?5)")?;
            for (ordinal, item) in self.items.iter().enumerate() {
                insert.execute(params![
                    self.id,
                    ordinal,
                    item.role,
                    item.content,
                    item.has_images
                ])?;
            }
        }
        let (notes, digest, count): (String, String, usize) = conn.query_row(
            "SELECT w.notes,w.source_digest,(SELECT COUNT(*) FROM context_history_items i WHERE i.window_id=w.id) FROM context_history_windows w WHERE w.id=?1 AND w.conversation_id=?2",
            params![self.id,self.conversation_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        )?;
        let mut readback = blake3::Hasher::new();
        let mut query = conn.prepare("SELECT role,content,has_images FROM context_history_items WHERE window_id=?1 ORDER BY ordinal")?;
        let mut rows = query.query([&self.id])?;
        while let Some(row) = rows.next()? {
            hash_item(
                &mut readback,
                &row.get::<_, String>(0)?,
                &row.get::<_, String>(1)?,
                row.get(2)?,
            );
        }
        if notes != self.notes
            || digest != self.digest
            || count != self.items.len()
            || readback.finalize().to_hex().as_str() != self.digest
        {
            return Err(CoreError::Internal(
                "Context history verification failed; the working context was retained".into(),
            ));
        }
        Ok(())
    }
}

impl Database {
    pub(crate) fn archive_context_history(
        &self,
        archive: &ContextHistoryArchive,
    ) -> Result<(), CoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        archive.commit(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_context_history(
        &self,
        conversation_id: &str,
        before_id: Option<&str>,
        limit: usize,
    ) -> Result<Value, CoreError> {
        let conn = self.conn();
        let mut query = conn.prepare(
            "SELECT id,message_count,created_at,substr(notes,1,500) FROM context_history_windows
             WHERE conversation_id=?1 AND (?2 IS NULL OR rowid < (SELECT rowid FROM context_history_windows WHERE id=?2 AND conversation_id=?1))
             ORDER BY rowid DESC LIMIT ?3")?;
        let limit = limit.clamp(1, 20);
        let mut items = query
            .query_map(params![conversation_id, before_id, limit + 1], |row| {
                Ok(json!({
                    "windowId":row.get::<_,String>(0)?,"messageCount":row.get::<_,usize>(1)?,
                    "createdAt":row.get::<_,String>(2)?,"notesPreview":row.get::<_,String>(3)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let more = items.len() > limit;
        items.truncate(limit);
        let next = more.then(|| items.last().unwrap()["windowId"].clone());
        Ok(json!({"windows":items,"nextBeforeId":next}))
    }

    pub fn read_context_history(
        &self,
        conversation_id: &str,
        window_id: &str,
        ordinal: usize,
        offset: usize,
        max_chars: usize,
    ) -> Result<Value, CoreError> {
        let conn = self.conn();
        conn.query_row(
            "SELECT i.role,substr(i.content,?4+1,?5),length(i.content),i.has_images
             FROM context_history_items i JOIN context_history_windows w ON w.id=i.window_id
             WHERE w.conversation_id=?1 AND w.id=?2 AND i.ordinal=?3",
            params![
                conversation_id,
                window_id,
                ordinal,
                offset,
                max_chars.clamp(1, 16000)
            ],
            |row| {
                Ok(json!({
                    "windowId":window_id,"item":ordinal,"role":row.get::<_,String>(0)?,
                    "text":row.get::<_,String>(1)?,"totalChars":row.get::<_,usize>(2)?,
                    "offset":offset,"hasImages":row.get::<_,bool>(3)?,
                }))
            },
        )
        .optional()?
        .ok_or_else(|| CoreError::NotFound("Context history item in this conversation".into()))
    }

    /// Read the already persisted, privacy-projected output of one completed
    /// local tool. The trace owns the original; context compaction owns only a
    /// projection. Neither tool arguments nor provider reasoning are exposed.
    pub fn read_context_tool_result(
        &self,
        conversation_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        result_id: Option<&str>,
        offset: usize,
        max_chars: usize,
    ) -> Result<Value, CoreError> {
        let result =
            self.load_context_tool_result(conversation_id, turn_id, tool_call_id, result_id)?;
        // Rust character slicing also preserves embedded NULs, which SQLite
        // text length/substr treat as a string terminator.
        let total_chars = result.content.chars().count();
        let text: String = result
            .content
            .chars()
            .skip(offset)
            .take(max_chars.clamp(1, 16000))
            .collect();
        let next = offset.saturating_add(text.chars().count());
        Ok(json!({
            "turnId":turn_id,"toolCallId":tool_call_id,"resultId":result.result_id,
            "toolName":result.tool_name,"isError":result.is_error,
            "text":text,"totalChars":total_chars,"offset":offset,
            "contentDigest":blake3::hash(result.content.as_bytes()).to_hex().to_string(),
            "nextOffset":(next < total_chars).then_some(next),
        }))
    }

    fn load_context_tool_result(
        &self,
        conversation_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        result_id: Option<&str>,
    ) -> Result<PersistedContextToolResult, CoreError> {
        let conn = self.conn();
        conn.query_row(
            &format!(
                "WITH outputs AS ({CONTEXT_TOOL_RESULT_QUERY})
                 SELECT tool_name,is_error,content,artifacts,result_id
                 FROM outputs WHERE (SELECT COUNT(*) FROM outputs)=1
                   AND status IN ('done','error') AND provider_executed=0
                   AND content_type='text' AND tool_name_type='text'"
            ),
            params![conversation_id, turn_id, tool_call_id, result_id],
            |row| {
                // Reuse the runtime's typed channel validation/fallback. These
                // artifacts have already passed privacy and ephemeral-screen
                // projection; never reconstruct discarded screen semantics.
                let output = crate::tools::ToolResult {
                    call_id: tool_call_id.into(),
                    content: row.get(2)?,
                    is_error: row.get(1)?,
                    artifacts: row
                        .get::<_, Option<String>>(3)?
                        .and_then(|raw| serde_json::from_str(&raw).ok()),
                };
                Ok(PersistedContextToolResult {
                    result_id: row.get(4)?,
                    tool_name: row.get(0)?,
                    is_error: output.is_error,
                    content: output.llm_context_content(),
                })
            },
        )
        .optional()?
        .ok_or_else(|| {
            CoreError::NotFound("Unique completed tool result in this conversation and turn".into())
        })
    }

    /// Advertise a readback only after the exact output has reached the owning
    /// trace. The runtime result ID survives reused provider IDs and trace
    /// projection. Intentionally ephemeral model channels have no exact hint.
    pub(crate) fn context_tool_result_is_persisted(
        &self,
        conversation_id: &str,
        turn_id: &str,
        tool_call_id: &str,
        result_id: Option<&str>,
        expected_content: &str,
    ) -> Result<bool, CoreError> {
        match self.load_context_tool_result(conversation_id, turn_id, tool_call_id, result_id) {
            Ok(result) => Ok(result.content == expected_content),
            Err(CoreError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn search_context_history(
        &self,
        conversation_id: &str,
        needle: &str,
        limit: usize,
    ) -> Result<Value, CoreError> {
        if needle.trim().is_empty() || needle.chars().count() > 256 {
            return Err(CoreError::InvalidInput(
                "History search requires 1 to 256 characters".into(),
            ));
        }
        let conn = self.conn();
        // Literal matching supports CJK phrases and IDs without FTS syntax or
        // wildcard interpretation. Only bounded excerpts leave SQLite.
        let mut query = conn.prepare(
            "SELECT w.id,i.ordinal,i.role,substr(i.content,max(1,instr(lower(i.content),lower(?2))-120),700),length(i.content)
             FROM context_history_windows w JOIN context_history_items i ON i.window_id=w.id
             WHERE w.conversation_id=?1 AND instr(lower(i.content),lower(?2))>0
             ORDER BY w.rowid DESC,i.ordinal ASC LIMIT ?3")?;
        let items = query.query_map(params![conversation_id,needle,limit.clamp(1,20)], |row| Ok(json!({
            "windowId":row.get::<_,String>(0)?,"item":row.get::<_,usize>(1)?,"role":row.get::<_,String>(2)?,
            "excerpt":row.get::<_,String>(3)?,"totalChars":row.get::<_,usize>(4)?,
        })))?.collect::<Result<Vec<_>,_>>()?;
        Ok(json!({"matches":items}))
    }
}

// Scope in SQL, before returning any output. Use the canonical tool item only:
// adjacent thinking, arguments and hosted replay are not readback. Only the
// validated ToolOutput model text may be selected from the tool's artifacts.
// Existing privacy projections and conversation/turn deletion govern this data.
const CONTEXT_TOOL_RESULT_QUERY: &str = "
    SELECT json_extract(i.value,'$.toolCall.toolName') AS tool_name,
           COALESCE(json_extract(i.value,'$.toolCall.isError'),0) AS is_error,
           json_extract(i.value,'$.toolCall.content') AS content,
           json_quote(json_extract(i.value,'$.toolCall.artifacts')) AS artifacts,
           json_extract(i.value,'$.toolCall.resultId') AS result_id,
           json_extract(i.value,'$.toolCall.status') AS status,
           COALESCE(json_extract(i.value,'$.toolCall.providerExecuted'),0) AS provider_executed,
           json_type(i.value,'$.toolCall.content') AS content_type,
           json_type(i.value,'$.toolCall.toolName') AS tool_name_type
    FROM conversation_turns t,json_each(t.trace_json,'$.items') i
    WHERE t.conversation_id=?1 AND t.id=?2
      AND json_extract(i.value,'$.kind')='tool'
      AND json_extract(i.value,'$.toolCall.callId')=?3
      AND (?4 IS NULL OR json_extract(i.value,'$.toolCall.resultId')=?4)
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{ConversationMessage, CreateConversationInput};

    fn conversation(db: &Database) -> String {
        db.create_conversation(&CreateConversationInput {
            provider: "custom".into(),
            model: "test".into(),
            system_prompt: None,
            collection_context: None,
            project_id: None,
            persona_id: None,
        })
        .unwrap()
        .id
    }

    fn tool_trace(db: &Database, conversation: &str, content: &str) -> (String, Value) {
        let user = uuid::Uuid::new_v4().to_string();
        db.conn().execute(
            "INSERT INTO messages(id,conversation_id,role,content) VALUES(?1,?2,'user','Read the output')",
            params![user, conversation],
        ).unwrap();
        let turn = db
            .create_conversation_turn(conversation, &user, None)
            .unwrap();
        let trace = json!({"kind":"turnTrace","items":[
            {"kind":"thinking","text":"private reasoning must not be read"},
            {"kind":"tool","toolCall":{
                "callId":"long-output","toolName":"read_file","status":"done",
                "arguments":"private arguments must not be read",
                "content":content,"isError":false,
                "artifacts":{"private":"artifacts must not be read"}
            }}
        ]});
        db.update_conversation_turn_trace(&turn.id, Some(&trace))
            .unwrap();
        (turn.id, trace)
    }

    #[test]
    fn tool_result_pages_preserve_unicode_nuls_and_scope_after_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tool-results.db");
        let db = Database::new(&path).unwrap();
        let id = conversation(&db);
        let text = format!(
            "{}\0middle-evidence\0{}",
            "证据🙂".repeat(9000),
            "末尾".repeat(100)
        );
        let (turn, _) = tool_trace(&db, &id, &text);
        assert!(db
            .context_tool_result_is_persisted(&id, &turn, "long-output", None, &text)
            .unwrap());
        assert!(!db
            .context_tool_result_is_persisted(&id, &turn, "long-output", None, "stale text")
            .unwrap());
        drop(db);
        let db = Database::new(path).unwrap();
        let mut restored = String::new();
        let mut offset = 0;
        loop {
            let page = db
                .read_context_tool_result(&id, &turn, "long-output", None, offset, 701)
                .unwrap();
            assert_eq!(
                page["contentDigest"],
                blake3::hash(text.as_bytes()).to_hex().as_str()
            );
            assert_eq!(page["totalChars"], text.chars().count());
            restored.push_str(page["text"].as_str().unwrap());
            match page["nextOffset"].as_u64() {
                Some(next) => {
                    assert!(next as usize > offset);
                    offset = next as usize;
                }
                None => break,
            }
        }
        assert_eq!(restored, text);
        for (chat, turn_id, call) in [
            ("other", turn.as_str(), "long-output"),
            (id.as_str(), "other", "long-output"),
            (id.as_str(), turn.as_str(), "other"),
        ] {
            assert!(db
                .read_context_tool_result(chat, turn_id, call, None, 0, 100)
                .is_err());
        }
        let end = db
            .read_context_tool_result(&id, &turn, "long-output", None, usize::MAX, 100)
            .unwrap();
        assert_eq!(end["text"], "");
        assert!(end["nextOffset"].is_null());
    }

    #[test]
    fn tool_result_readback_rejects_ambiguous_unfinished_and_hosted_results() {
        let db = Database::open_memory().unwrap();
        let id = conversation(&db);
        let (turn, original) = tool_trace(&db, &id, "visible text");
        for mode in ["duplicate", "running", "hosted"] {
            let mut trace = original.clone();
            match mode {
                "duplicate" => {
                    let duplicate = trace["items"][1].clone();
                    trace["items"].as_array_mut().unwrap().push(duplicate);
                }
                "running" => trace["items"][1]["toolCall"]["status"] = json!("running"),
                _ => trace["items"][1]["toolCall"]["providerExecuted"] = json!(true),
            }
            db.update_conversation_turn_trace(&turn, Some(&trace))
                .unwrap();
            assert!(
                db.read_context_tool_result(&id, &turn, "long-output", None, 0, 100)
                    .is_err(),
                "{mode}"
            );
            assert!(!db
                .context_tool_result_is_persisted(&id, &turn, "long-output", None, "visible text")
                .unwrap());
        }
        db.update_conversation_turn_trace(&turn, Some(&original))
            .unwrap();
        let page = db
            .read_context_tool_result(&id, &turn, "long-output", None, 0, 100)
            .unwrap();
        assert_eq!(page["text"], "visible text");
        assert!(!page.to_string().contains("private"));
        db.conn()
            .execute("DELETE FROM conversation_turns WHERE id=?1", [&turn])
            .unwrap();
        assert!(db
            .read_context_tool_result(&id, &turn, "long-output", None, 0, 100)
            .is_err());
    }

    #[tokio::test]
    async fn tool_result_readback_uses_current_privacy_and_rejects_stale_pages() {
        use crate::tools::{context_history_tool::ContextHistoryTool, Tool, ToolExecutionContext};
        let db = Database::open_memory().unwrap();
        let id = conversation(&db);
        let (turn, mut trace) = tool_trace(&db, &id, "retained privateCODE evidence");
        let result_id = "12345678-1234-1234-1234-123456789abc";
        trace["items"][1]["toolCall"]["resultId"] = json!(result_id);
        trace["items"][1]["toolCall"]["artifacts"] = crate::tools::ToolResult::from_output(
            "long-output",
            false,
            crate::tools::ToolOutput {
                llm_content: "model privateCODE evidence".into(),
                display_content: "display summary".into(),
                data: Some(json!({"resultId":result_id})),
                artifacts: None,
                attachments: Vec::new(),
            },
        )
        .artifacts
        .unwrap();
        db.update_conversation_turn_trace(&turn, Some(&trace))
            .unwrap();
        let before = db
            .read_context_tool_result(&id, &turn, "long-output", None, 0, 100)
            .unwrap();
        db.save_privacy_config(&crate::privacy::PrivacyConfig {
            enabled: true,
            redact_patterns: vec![crate::privacy::RedactRule {
                name: "test".into(),
                pattern: "privateCODE|12345678".into(),
                replacement: "[PRIVATE]".into(),
            }],
            ..Default::default()
        })
        .unwrap();
        let mut args = json!({"action":"read_tool_result", "turn_id":turn, "tool_call_id":"long-output", "result_id":result_id, "expected_digest":before["contentDigest"]});
        let result = ContextHistoryTool
            .execute(
                ToolExecutionContext::new("readback", &args.to_string(), &db, &[])
                    .with_conversation_id(Some(&id)),
            )
            .await;
        assert!(
            result.is_err(),
            "a privacy revision must not be combined with an earlier page"
        );
        args.as_object_mut().unwrap().remove("expected_digest");
        let result = ContextHistoryTool
            .execute(
                ToolExecutionContext::new("readback", &args.to_string(), &db, &[])
                    .with_conversation_id(Some(&id)),
            )
            .await
            .unwrap();
        assert!(!result.content.contains("privateCODE"));
        assert!(result.content.contains("[PRIVATE]"));
        assert!(result.content.contains("Historical evidence"));
        let projected = db.get_conversation_turn(&turn).unwrap().trace.unwrap();
        assert_eq!(projected["items"][1]["toolCall"]["resultId"], result_id);
        assert_ne!(
            projected["items"][1]["toolCall"]["artifacts"]["data"]["resultId"],
            result_id
        );
    }

    #[test]
    fn tool_result_readback_preserves_typed_channel_validation_and_ephemeral_policy() {
        use crate::tools::{ToolOutput, ToolResult};
        let db = Database::open_memory().unwrap();
        let id = conversation(&db);
        let (turn, mut trace) = tool_trace(&db, &id, "display summary");
        trace["items"][1]["toolCall"]["resultId"] = json!("result-1");
        let typed = ToolResult::from_output(
            "long-output",
            false,
            ToolOutput {
                llm_content: "full\0模型🙂 evidence".into(),
                display_content: "display summary".into(),
                data: None,
                artifacts: None,
                attachments: Vec::new(),
            },
        )
        .artifacts
        .unwrap();
        for artifacts in [
            typed.clone(),
            Value::Null,
            json!(42),
            json!(true),
            json!(typed.to_string()),
            json!({"toolOutput":{"llmContent":"malformed envelope"}}),
        ] {
            trace["items"][1]["toolCall"]["artifacts"] = artifacts.clone();
            db.update_conversation_turn_trace(&turn, Some(&trace))
                .unwrap();
            let expected = ToolResult {
                call_id: "long-output".into(),
                content: "display summary".into(),
                is_error: false,
                artifacts: Some(artifacts),
            }
            .llm_context_content();
            let page = db
                .read_context_tool_result(&id, &turn, "long-output", Some("result-1"), 0, 100)
                .unwrap();
            assert_eq!(page["text"], expected);
        }
        // Desktop persistence deliberately replaces its model channel with a
        // safe receipt. Readback cannot claim the omitted live screen is saved.
        trace["items"][1]["toolCall"]["artifacts"] = typed;
        trace["items"][1]["toolCall"]["artifacts"]["toolOutput"]["llmContent"] =
            json!("display summary");
        db.update_conversation_turn_trace(&turn, Some(&trace))
            .unwrap();
        assert!(!db
            .context_tool_result_is_persisted(
                &id,
                &turn,
                "long-output",
                Some("result-1"),
                "live ephemeral screen",
            )
            .unwrap());
    }

    #[test]
    fn result_identity_disambiguates_reused_call_ids_but_rejects_duplicate_records() {
        let db = Database::open_memory().unwrap();
        let id = conversation(&db);
        let (turn, mut trace) = tool_trace(&db, &id, "first output");
        trace["items"][1]["toolCall"]["resultId"] = json!("first-result");
        let mut second = trace["items"][1].clone();
        second["toolCall"]["resultId"] = json!("second-result");
        second["toolCall"]["content"] = json!("second output");
        trace["items"].as_array_mut().unwrap().push(second);
        db.update_conversation_turn_trace(&turn, Some(&trace))
            .unwrap();
        assert!(db
            .read_context_tool_result(&id, &turn, "long-output", None, 0, 100)
            .is_err());
        for (result, expected) in [
            ("first-result", "first output"),
            ("second-result", "second output"),
        ] {
            assert_eq!(
                db.read_context_tool_result(&id, &turn, "long-output", Some(result), 0, 100)
                    .unwrap()["text"],
                expected
            );
        }
        assert!(db
            .read_context_tool_result("other", &turn, "long-output", Some("first-result"), 0, 100)
            .is_err());
        assert!(db
            .read_context_tool_result(&id, &turn, "other", Some("first-result"), 0, 100)
            .is_err());
        for mode in ["done", "running", "hosted"] {
            let mut duplicate = trace.clone();
            duplicate["items"][2]["toolCall"]["resultId"] = json!("first-result");
            duplicate["items"][2]["toolCall"]["status"] =
                json!(if mode == "running" { "running" } else { "done" });
            duplicate["items"][2]["toolCall"]["providerExecuted"] = json!(mode == "hosted");
            db.update_conversation_turn_trace(&turn, Some(&duplicate))
                .unwrap();
            assert!(
                db.read_context_tool_result(
                    &id,
                    &turn,
                    "long-output",
                    Some("first-result"),
                    0,
                    100
                )
                .is_err(),
                "{mode}"
            );
        }
    }

    #[test]
    fn history_is_lossless_scoped_and_readable_after_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.db");
        let db = Database::new(&path).unwrap();
        let id = conversation(&db);
        let text = format!(
            "{}needle_百分比%_{}",
            "早期证据🙂".repeat(2500),
            "结尾".repeat(100)
        );
        let archive = ContextHistoryArchive::prepare(
            &id,
            None,
            &[Message::text(Role::Tool, &text)],
            "Continue the review; tests remain.",
        )
        .unwrap();
        db.archive_context_history(&archive).unwrap();
        drop(db);
        let db = Database::new(&path).unwrap();
        let found = db
            .search_context_history(&id, "needle_百分比%_", 5)
            .unwrap();
        assert_eq!(found["matches"][0]["windowId"], archive.id);
        let mut recovered = String::new();
        while recovered.chars().count() < text.chars().count() {
            let page = db
                .read_context_history(&id, &archive.id, 0, recovered.chars().count(), 700)
                .unwrap();
            recovered.push_str(page["text"].as_str().unwrap());
        }
        assert_eq!(recovered, text);
        assert!(db
            .read_context_history("other", &archive.id, 0, 0, 100)
            .is_err());
        assert_eq!(
            db.search_context_history("other", "needle", 5).unwrap()["matches"],
            json!([])
        );
    }

    #[test]
    fn archive_readback_failure_rolls_back_the_entire_window() {
        let db = Database::open_memory().unwrap();
        let id = conversation(&db);
        db.conn().execute_batch("CREATE TRIGGER drop_history_item BEFORE INSERT ON context_history_items BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        let archive = ContextHistoryArchive::prepare(
            &id,
            None,
            &[Message::text(Role::User, "Do not lose this request")],
            "Pending work",
        )
        .unwrap();
        assert!(db.archive_context_history(&archive).is_err());
        assert_eq!(
            db.list_context_history(&id, None, 5).unwrap()["windows"],
            json!([])
        );
    }

    #[test]
    fn editing_canonical_history_invalidates_archived_copies() {
        let db = Database::open_memory().unwrap();
        let id = conversation(&db);
        db.add_message(&ConversationMessage {
            id: "original".into(),
            conversation_id: id.clone(),
            role: Role::User,
            content: "old request".into(),
            tool_call_id: None,
            tool_calls: vec![],
            artifacts: None,
            token_count: 1,
            created_at: String::new(),
            sort_order: 0,
            thinking: None,
            image_attachments: None,
        })
        .unwrap();
        let archive = ContextHistoryArchive::prepare(
            &id,
            None,
            &[Message::text(Role::User, "old request")],
            "",
        )
        .unwrap();
        db.archive_context_history(&archive).unwrap();
        db.conn()
            .execute(
                "UPDATE messages SET content='new request' WHERE id='original'",
                [],
            )
            .unwrap();
        assert!(db
            .read_context_history(&id, &archive.id, 0, 0, 100)
            .is_err());
    }
}
