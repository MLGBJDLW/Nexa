use super::*;
use crate::conversation::{ConversationMessage, CreateConversationInput};
use crate::llm::{Role, ToolCallRequest};

fn enabled_policy() -> PrivacyConfig {
    PrivacyConfig {
        enabled: true,
        redact_patterns: vec![RedactRule {
            name: "private source".into(),
            pattern: "privateCODE".into(),
            replacement: "[PRIVATE]".into(),
        }],
        ..Default::default()
    }
}

fn conversation(db: &Database) -> String {
    db.create_conversation(&CreateConversationInput {
        provider: "open_ai".into(),
        model: "test".into(),
        system_prompt: None,
        collection_context: None,
        project_id: None,
        persona_id: None,
    })
    .unwrap()
    .id
}

fn seed_message(db: &Database, conversation: &str, role: Role, id: &str) {
    db.add_message(&ConversationMessage {
        id: id.into(),
        conversation_id: conversation.into(),
        role,
        content: "privateCODE".into(),
        tool_call_id: None,
        tool_calls: vec![],
        artifacts: None,
        token_count: 3,
        created_at: String::new(),
        sort_order: 0,
        thinking: None,
        image_attachments: None,
    })
    .unwrap();
}

#[test]
fn privacy_projects_claim_and_event_excerpts_when_document_provenance_is_bound() {
    let db = Database::open_memory().unwrap();
    let mut policy = enabled_policy();
    policy.redact_patterns[0].replacement = "privateCODEprivateCODE".into();
    db.save_privacy_config(&policy).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("allowance.md");
    std::fs::write(&path, "Allowance is 500.").unwrap();
    let source = db
        .add_source(crate::sources::CreateSourceInput {
            root_path: folder.path().to_string_lossy().into_owned(),
            include_globs: vec!["**/*.md".into()],
            exclude_globs: vec![],
            watch_enabled: false,
        })
        .unwrap();
    crate::ingest::scan_source(&db, &source.id).unwrap();
    let document = db
        .get_document_in_source(&source.id, &path.to_string_lossy())
        .unwrap()
        .unwrap();
    for source_ref in [&document.id, "manual-reference"] {
        db.create_knowledge_claim(
            None,
            &serde_json::from_value(serde_json::json!({
                "subject":"Allowance", "predicate":"is", "object":"500",
                "sourceRef":source_ref, "sourceExcerpt":"privateCODE"
            }))
            .unwrap(),
        )
        .unwrap();
        db.create_knowledge_event(
            None,
            &serde_json::from_value(serde_json::json!({
                "eventKind":"decision", "title":"Allowance agreed",
                "sourceRef":source_ref, "sourceExcerpt":"privateCODE"
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let conn = db.conn();
    // A repeated provenance write must not apply a non-idempotent rule twice.
    conn.execute("UPDATE knowledge_evidence SET document_id=document_id", [])
        .unwrap();
    let excerpts: Vec<(Option<String>, String)> = conn
        .prepare("SELECT document_id,excerpt FROM knowledge_evidence")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(excerpts.len(), 4);
    assert_eq!(excerpts.iter().filter(|(id, _)| id.is_some()).count(), 2);
    for (document_id, excerpt) in excerpts {
        if let Some(document_id) = document_id {
            assert_eq!(document_id, document.id);
            assert_eq!(excerpt, "privateCODEprivateCODE");
        } else {
            assert_eq!(
                excerpt, "privateCODE",
                "manual evidence remains authored content"
            );
        }
    }
}

#[tokio::test]
async fn privacy_revocation_reaches_file_database_read_lanes_and_never_revives_old_leases() {
    let folder = tempfile::tempdir().unwrap();
    let db = Database::new(folder.path().join("privacy.db")).unwrap();
    let executor = crate::db_executor::DatabaseExecutor::new(db.clone(), 4).unwrap();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let read_cancel = cancellation.clone();
    let lease = executor
        .read(move |reader| reader.privacy_lease(&read_cancel))
        .await
        .unwrap()
        .value;
    let original = lease.policy.fingerprint().to_string();
    executor
        .write(|writer| writer.save_privacy_config(&enabled_policy()))
        .await
        .unwrap();
    assert!(lease.is_revoked() && cancellation.is_cancelled());
    let current = db
        .privacy_lease(&tokio_util::sync::CancellationToken::new())
        .unwrap();
    let same_policy_fingerprint = current.policy.fingerprint().to_string();
    db.save_privacy_config(&enabled_policy()).unwrap();
    assert!(
        !current.is_revoked(),
        "saving identical policy must not cancel live work"
    );
    assert_eq!(same_policy_fingerprint, current.policy.fingerprint());
    db.save_privacy_config(&PrivacyConfig {
        enabled: false,
        ..Default::default()
    })
    .unwrap();
    db.save_privacy_config(&enabled_policy()).unwrap();
    let fresh = db
        .privacy_lease(&tokio_util::sync::CancellationToken::new())
        .unwrap();
    assert_ne!(fresh.policy.fingerprint(), same_policy_fingerprint);
    assert_ne!(fresh.policy.fingerprint(), original);
    assert!(current.is_revoked());
}

#[test]
fn privacy_revokes_opaque_replay_in_messages_and_independent_provider_ledger() {
    use crate::llm::provider_turn::{ProviderReplayPayload, ProviderTurnEnvelope, RouteSnapshot};
    use crate::llm::reasoning_profile::{ReasoningCaptureStatus, ReasoningReplayPolicy};
    let db = Database::open_memory().unwrap();
    db.save_privacy_config(&PrivacyConfig {
        enabled: false,
        ..Default::default()
    })
    .unwrap();
    let cid = conversation(&db);
    let lease = db
        .privacy_lease(&tokio_util::sync::CancellationToken::new())
        .unwrap();
    let call = ToolCallRequest {
        id: "privateCODE-call".into(),
        name: "search".into(),
        arguments: r#"{"query":"privateCODE"}"#.into(),
        thought_signature: Some("opaque-signature".into()),
    };
    let mut envelope = ProviderTurnEnvelope::capture_with_replay_payload(
        "turn-item",
        "sample",
        RouteSnapshot::unknown("test", "model", ReasoningReplayPolicy::RequiredAlways),
        "privateCODE answer",
        None,
        None,
        vec![call.clone()],
        true,
        Some(ProviderReplayPayload::DeepSeekReasoningContent(
            "opaque privateCODE reasoning".into(),
        )),
    );
    envelope.privacy_fingerprint = Some(lease.policy.fingerprint().into());
    let mut message = ConversationMessage {
        id: "sample-message".into(),
        conversation_id: cid.clone(),
        role: Role::Assistant,
        content: envelope.visible_content.clone(),
        tool_call_id: None,
        tool_calls: vec![call],
        artifacts: crate::conversation::merge_provider_turn_envelope_artifact(None, &envelope),
        token_count: 5,
        created_at: String::new(),
        sort_order: 0,
        thinking: None,
        image_attachments: None,
    };
    let scope = crate::conversation::ProviderTurnPersistenceScope {
        scope_id: "scope",
        conversation_id: Some(&cid),
        conversation_turn_id: None,
        run_id: None,
        subtask_run_id: None,
    };
    db.persist_provider_turn(Some(&message), &envelope, scope)
        .unwrap();
    db.save_privacy_config(&enabled_policy()).unwrap();
    let saved = db.get_messages(&cid).unwrap().remove(0);
    let safe = crate::conversation::conversation_message_provider_turn(&saved).unwrap();
    assert_eq!(safe.capture_status, ReasoningCaptureStatus::Redacted);
    assert_eq!(safe.replay_payload, ProviderReplayPayload::None);
    assert!(safe.provider_items.is_empty());
    assert_eq!(safe.tool_calls[0].id, "privateCODE-call");
    assert_eq!(safe.tool_calls[0].name, "search");
    assert_eq!(safe.tool_calls[0].thought_signature, None);
    let ledger: (String, String, String, String) = db.conn().query_row(
        "SELECT visible_content,replay_payload_json,tool_calls_json,raw_response_digest FROM provider_turn_envelopes WHERE turn_item_id='turn-item'", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(ledger.0, safe.visible_content);
    assert_eq!(
        serde_json::from_str::<ProviderReplayPayload>(&ledger.1).unwrap(),
        ProviderReplayPayload::None
    );
    assert_eq!(
        serde_json::from_str::<Vec<ToolCallRequest>>(&ledger.2).unwrap(),
        safe.tool_calls
    );
    assert_eq!(ledger.3, safe.raw_response_digest);
    // A stale worker's complete envelope must not receive the new policy stamp.
    envelope.turn_item_id = "late-turn-item".into();
    envelope.sample_id = "late-sample".into();
    message.id = "late-message".into();
    message.artifacts = crate::conversation::merge_provider_turn_envelope_artifact(None, &envelope);
    db.persist_provider_turn(Some(&message), &envelope, scope)
        .unwrap();
    let late = db
        .get_messages(&cid)
        .unwrap()
        .into_iter()
        .find(|m| m.id == "late-message")
        .unwrap();
    let late = crate::conversation::conversation_message_provider_turn(&late).unwrap();
    assert_eq!(late.capture_status, ReasoningCaptureStatus::Redacted);
    assert_eq!(late.replay_payload, ProviderReplayPayload::None);
    // Even a caller holding an old in-memory history gets a fresh safe projection.
    let mut stale = crate::llm::Message::text(Role::Assistant, "privateCODE answer");
    stale.set_provider_turn(envelope);
    let policy = db
        .privacy_lease(&tokio_util::sync::CancellationToken::new())
        .unwrap();
    policy
        .policy
        .redact_context_messages(std::slice::from_mut(&mut stale));
    assert!(!stale.text_content().contains("privateCODE"));
    assert!(stale.parts.iter().all(|part| match part {
        crate::llm::ContentPart::ProviderTurn { envelope } =>
            envelope.replay_payload == ProviderReplayPayload::None,
        _ => true,
    }));
}

#[test]
fn privacy_update_masks_derived_memory_and_trace_but_preserves_authored_content() {
    let db = Database::open_memory().unwrap();
    db.save_privacy_config(&PrivacyConfig {
        enabled: false,
        ..Default::default()
    })
    .unwrap();
    let cid = conversation(&db);
    seed_message(&db, &cid, Role::User, "manual-message");
    let turn = db
        .create_conversation_turn(&cid, "manual-message", None)
        .unwrap();
    db.conn().execute("UPDATE conversation_turns SET trace_json=?2 WHERE id=?1", params![turn.id,
        serde_json::json!({"kind":"traceTimeline","items":[{"kind":"tool","callId":"privateCODE-call","toolName":"search","content":"privateCODE","artifacts":{"data":{"secret":"privateCODE"}}}]}).to_string()]).unwrap();
    db.conn().execute_batch("INSERT INTO user_memories(id,content,source) VALUES ('manual','privateCODE','manual'),('generated','privateCODE','agent');").unwrap();
    db.conn()
        .execute(
            "INSERT INTO agent_scratchpad(conversation_id,content) VALUES (?1,'privateCODE')",
            [&cid],
        )
        .unwrap();
    let archive = crate::context_history::ContextHistoryArchive::prepare(
        &cid,
        None,
        &[crate::llm::Message::text(
            Role::Assistant,
            "privateCODE source",
        )],
        "privateCODE scratchpad",
    )
    .unwrap();
    {
        let mut conn = db.conn();
        let tx = conn.transaction().unwrap();
        archive.commit(&tx).unwrap();
        tx.commit().unwrap();
    }
    db.save_privacy_config(&enabled_policy()).unwrap();
    assert_eq!(db.get_messages(&cid).unwrap()[0].content, "privateCODE");
    let conn = db.conn();
    let manual: String = conn
        .query_row(
            "SELECT content FROM user_memories WHERE id='manual'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let derived: String = conn
        .query_row(
            "SELECT content FROM user_memories WHERE id='generated'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(manual, "privateCODE");
    assert_eq!(derived, "[PRIVATE]");
    let trace: String = conn
        .query_row(
            "SELECT trace_json FROM conversation_turns WHERE id=?1",
            [&turn.id],
            |r| r.get(0),
        )
        .unwrap();
    let trace: serde_json::Value = serde_json::from_str(&trace).unwrap();
    assert_eq!(trace["items"][0]["callId"], "privateCODE-call");
    assert_eq!(trace["items"][0]["toolName"], "search");
    assert_eq!(trace["items"][0]["content"], "[PRIVATE]");
    assert_eq!(
        trace["items"][0]["artifacts"]["data"]["secret"],
        "[PRIVATE]"
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM context_history_windows", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    // A stale archive cannot pass its original digest check and drop live history.
    {
        let mut conn = db.conn();
        let tx = conn.transaction().unwrap();
        assert!(archive.commit(&tx).is_err());
    }
    assert_eq!(
        db.conn()
            .query_row("SELECT count(*) FROM context_history_windows", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn privacy_upgrade_backfills_an_already_enabled_policy_and_is_idempotent() {
    let db = Database::open_memory().unwrap();
    let cid = conversation(&db);
    // Reconstruct the previous shipped schema, including a policy that will not
    // change when the user upgrades. This bypasses the new write guards only in
    // the fixture, before applying the versioned production migration.
    let triggers = db
        .conn()
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'privacy_chat_%'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for trigger in triggers {
        db.conn()
            .execute_batch(&format!("DROP TRIGGER {trigger}"))
            .unwrap();
    }
    db.conn().execute_batch("DROP TABLE privacy_chat_state; ALTER TABLE provider_turn_envelopes DROP COLUMN privacy_fingerprint; ALTER TABLE agent_task_runs DROP COLUMN privacy_revision; DELETE FROM _migrations WHERE name='v148_chat_privacy';").unwrap();
    db.conn()
        .execute(
            "INSERT OR REPLACE INTO privacy_config(key,value) VALUES ('privacy_config',?1)",
            [serde_json::to_string(&enabled_policy()).unwrap()],
        )
        .unwrap();
    seed_message(&db, &cid, Role::Assistant, "legacy");
    assert_eq!(db.get_messages(&cid).unwrap()[0].content, "privateCODE");
    crate::migrations::run_migrations(&db.conn()).unwrap();
    assert_eq!(db.get_messages(&cid).unwrap()[0].content, "[PRIVATE]");
    let revision = db
        .privacy_lease(&tokio_util::sync::CancellationToken::new())
        .unwrap()
        .policy
        .revision()
        .to_string();
    crate::migrations::run_migrations(&db.conn()).unwrap();
    db.save_privacy_config(&enabled_policy()).unwrap();
    assert_eq!(
        revision,
        db.privacy_lease(&tokio_util::sync::CancellationToken::new())
            .unwrap()
            .policy
            .revision()
    );
}

#[test]
fn privacy_default_without_a_saved_config_is_backfilled_and_guards_new_writes() {
    let db = Database::open_memory().unwrap();
    let cid = conversation(&db);
    let triggers = db
        .conn()
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='trigger' AND name LIKE 'privacy_chat_%'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for trigger in triggers {
        db.conn()
            .execute_batch(&format!("DROP TRIGGER {trigger}"))
            .unwrap();
    }
    db.conn().execute_batch("DROP TABLE privacy_chat_state; ALTER TABLE provider_turn_envelopes DROP COLUMN privacy_fingerprint; ALTER TABLE agent_task_runs DROP COLUMN privacy_revision; DELETE FROM _migrations WHERE name='v148_chat_privacy'; DELETE FROM privacy_config WHERE key='privacy_config';").unwrap();
    db.conn().execute("INSERT INTO messages(id,conversation_id,role,content) VALUES ('legacy-default',?1,'assistant','alice@example.com')",[&cid]).unwrap();
    crate::migrations::run_migrations(&db.conn()).unwrap();
    assert_eq!(db.get_messages(&cid).unwrap()[0].content, "[EMAIL]");
    db.save_privacy_config(&PrivacyConfig::default()).unwrap();
    db.conn().execute("INSERT INTO messages(id,conversation_id,role,content,sort_order) VALUES ('late-default',?1,'assistant','bob@example.com',1)",[&cid]).unwrap();
    assert_eq!(db.get_messages(&cid).unwrap()[1].content, "[EMAIL]");
    let fresh = Database::open_memory().unwrap();
    let cid = conversation(&fresh);
    fresh.conn().execute("INSERT INTO messages(id,conversation_id,role,content) VALUES ('fresh-default',?1,'assistant','carol@example.com')",[&cid]).unwrap();
    assert_eq!(fresh.get_messages(&cid).unwrap()[0].content, "[EMAIL]");
}

#[test]
fn privacy_provider_projection_applies_non_idempotent_rules_once_per_copy() {
    use crate::llm::provider_turn::{ProviderTurnEnvelope, RouteSnapshot};
    use crate::llm::reasoning_profile::ReasoningReplayPolicy;
    let db = Database::open_memory().unwrap();
    db.save_privacy_config(&PrivacyConfig {
        enabled: false,
        ..Default::default()
    })
    .unwrap();
    let cid = conversation(&db);
    let mut envelope = ProviderTurnEnvelope::capture(
        "first-turn",
        "first-sample",
        RouteSnapshot::unknown("test", "model", ReasoningReplayPolicy::NotRequired),
        "privateCODE",
        None,
        None,
        vec![ToolCallRequest {
            id: "stable-call".into(),
            name: "search".into(),
            arguments: r#"{"query":"privateCODE"}"#.into(),
            thought_signature: Some("opaque".into()),
        }],
        false,
    );
    let mut message = ConversationMessage {
        id: "first".into(),
        conversation_id: cid.clone(),
        role: Role::Assistant,
        content: envelope.visible_content.clone(),
        tool_call_id: None,
        tool_calls: envelope.tool_calls.clone(),
        artifacts: crate::conversation::merge_provider_turn_envelope_artifact(None, &envelope),
        token_count: 1,
        created_at: String::new(),
        sort_order: 0,
        thinking: None,
        image_attachments: None,
    };
    let scope = crate::conversation::ProviderTurnPersistenceScope {
        scope_id: "scope",
        conversation_id: Some(&cid),
        conversation_turn_id: None,
        run_id: None,
        subtask_run_id: None,
    };
    db.persist_provider_turn(Some(&message), &envelope, scope)
        .unwrap();
    let policy = PrivacyConfig {
        enabled: true,
        redact_patterns: vec![RedactRule {
            name: "twice".into(),
            pattern: "privateCODE".into(),
            replacement: "privateCODE privateCODE".into(),
        }],
        ..Default::default()
    };
    db.save_privacy_config(&policy).unwrap();
    envelope.turn_item_id = "late-turn".into();
    envelope.sample_id = "late-sample".into();
    message.id = "late".into();
    message.sort_order = 1;
    message.artifacts = crate::conversation::merge_provider_turn_envelope_artifact(None, &envelope);
    db.persist_provider_turn(Some(&message), &envelope, scope)
        .unwrap();
    for saved in db.get_messages(&cid).unwrap() {
        let embedded = crate::conversation::conversation_message_provider_turn(&saved).unwrap();
        let (text,calls,digest):(String,String,String)=db.conn().query_row(
            "SELECT visible_content,tool_calls_json,raw_response_digest FROM provider_turn_envelopes WHERE message_id=?1",[&saved.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(saved.content, "privateCODE privateCODE");
        assert_eq!(embedded.visible_content, saved.content);
        assert_eq!(text, saved.content);
        assert_eq!(saved.tool_calls, embedded.tool_calls);
        assert_eq!(
            serde_json::from_str::<Vec<ToolCallRequest>>(&calls).unwrap(),
            embedded.tool_calls
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved.tool_calls[0].arguments).unwrap()
                ["query"],
            "privateCODE privateCODE"
        );
        assert_eq!(digest, embedded.raw_response_digest);
    }
}

#[test]
fn privacy_storage_projection_preserves_json_and_does_not_recurse_or_rewrite_unchanged_columns() {
    let db = Database::open_memory().unwrap();
    let config = PrivacyConfig {
        enabled: true,
        redact_patterns: vec![
            RedactRule {
                name: "non-idempotent".into(),
                pattern: "privateCODE".into(),
                replacement: "privateCODE privateCODE".into(),
            },
            RedactRule {
                name: "quoted".into(),
                pattern: "SECRET".into(),
                replacement: "a\"b\\c".into(),
            },
        ],
        ..Default::default()
    };
    db.save_privacy_config(&config).unwrap();
    db.conn()
        .execute_batch("PRAGMA recursive_triggers=ON")
        .unwrap();
    let cid = conversation(&db);
    seed_message(&db, &cid, Role::Assistant, "once");
    assert_eq!(
        db.get_messages(&cid).unwrap()[0].content,
        "privateCODE privateCODE"
    );
    db.conn()
        .execute(
            "UPDATE messages SET thinking='new thought',artifacts_json=?1 WHERE id='once'",
            [r#"{"kind":"toolOutput","data":{"value":"SECRET"}}"#],
        )
        .unwrap();
    let message = db.get_messages(&cid).unwrap().remove(0);
    assert_eq!(message.content, "privateCODE privateCODE");
    assert_eq!(message.artifacts.unwrap()["data"]["value"], "a\"b\\c");
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT count(*) FROM fts_messages WHERE message_id='once'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn privacy_change_revokes_search_copies_from_chat_archives_and_fts() {
    let db = Database::open_memory().unwrap();
    db.save_privacy_config(&PrivacyConfig {
        enabled: false,
        ..Default::default()
    })
    .unwrap();
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("policy.md"), "allowance privateCODE 500").unwrap();
    let source = db
        .add_source(crate::sources::CreateSourceInput {
            root_path: folder.path().to_string_lossy().into_owned(),
            include_globs: vec!["**/*.md".into()],
            exclude_globs: vec![],
            watch_enabled: false,
        })
        .unwrap();
    crate::ingest::scan_source(&db, &source.id).unwrap();
    let result = crate::search::search(
        &db,
        &crate::models::SearchQuery {
            text: "allowance".into(),
            filters: Default::default(),
            limit: 10,
            offset: 0,
        },
    )
    .unwrap();
    let cards = serde_json::to_value(&result.evidence_cards).unwrap();
    assert!(cards.to_string().contains("privateCODE"));
    let conv = db
        .create_conversation(&CreateConversationInput {
            provider: "open_ai".into(),
            model: "test".into(),
            system_prompt: None,
            collection_context: None,
            project_id: None,
            persona_id: None,
        })
        .unwrap();
    let base = ConversationMessage {
        id: "user".into(),
        conversation_id: conv.id.clone(),
        role: Role::User,
        content: "My manual note privateCODE".into(),
        tool_call_id: None,
        tool_calls: vec![],
        artifacts: None,
        token_count: 9,
        created_at: String::new(),
        sort_order: 0,
        thinking: None,
        image_attachments: None,
    };
    db.add_message(&base).unwrap();
    let tool = ConversationMessage {
        id: "tool".into(),
        role: Role::Tool,
        content: cards.to_string(),
        tool_call_id: Some("privateCODE-call".into()),
        artifacts: Some(
            serde_json::json!({"kind":"searchResults","evidenceCards":cards,
            "llmContextContent":"allowance privateCODE", "data":{"nested":["privateCODE"]}}),
        ),
        sort_order: 2,
        ..base.clone()
    };
    let assistant = ConversationMessage {
        id: "assistant".into(),
        role: Role::Assistant,
        content: "The source says privateCODE".into(),
        thinking: Some("privateCODE from the source".into()),
        tool_calls: vec![ToolCallRequest {
            id: "privateCODE-call".into(),
            name: "search".into(),
            arguments: r#"{"query":"privateCODE"}"#.into(),
            thought_signature: Some("opaque-privateCODE".into()),
        }],
        sort_order: 1,
        ..base.clone()
    };
    db.add_message(&assistant).unwrap();
    db.add_message(&tool).unwrap();
    let checkpoint = db
        .create_checkpoint_with_messages(
            &conv.id,
            "before privacy",
            30,
            &[base.clone(), assistant.clone(), tool.clone()],
        )
        .unwrap();
    db.save_privacy_config(&enabled_policy()).unwrap();
    for message in db.get_messages(&conv.id).unwrap() {
        if message.role == Role::User {
            assert_eq!(message.content, base.content);
        } else {
            assert!(
                !message.content.contains("privateCODE"),
                "persisted {:?} still contains source content",
                message.role
            );
            assert!(!message
                .artifacts
                .unwrap_or_default()
                .to_string()
                .contains("privateCODE"));
            assert!(!message.thinking.unwrap_or_default().contains("privateCODE"));
            for call in message.tool_calls {
                assert_eq!(call.id, "privateCODE-call");
                assert_eq!(call.name, "search");
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&call.arguments).unwrap()["query"],
                    "[PRIVATE]"
                );
                assert_eq!(call.thought_signature, None);
            }
        }
    }
    let archived: Vec<(String, String, Option<String>)> = db
        .conn()
        .prepare("SELECT role,content,artifacts_json FROM archived_messages WHERE checkpoint_id=?1")
        .unwrap()
        .query_map([checkpoint], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(archived.len(), 3);
    assert!(archived
        .iter()
        .filter(|r| r.0 != "user")
        .all(|r| !r.1.contains("privateCODE")
            && !r.2.as_deref().unwrap_or_default().contains("privateCODE")));
    // An already running worker can complete after the revocation transaction.
    let late = ConversationMessage {
        id: "late".into(),
        sort_order: 3,
        ..assistant
    };
    db.add_message(&late).unwrap();
    assert!(!db
        .get_messages(&conv.id)
        .unwrap()
        .last()
        .unwrap()
        .content
        .contains("privateCODE"));
    let fts_hits: i64 = db.conn().query_row(
        "SELECT count(*) FROM fts_messages WHERE fts_messages MATCH 'privateCODE' AND role!='user'", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(
        fts_hits, 0,
        "the FTS copy must not retain revoked tool-derived text"
    );
}
