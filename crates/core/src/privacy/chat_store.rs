//! Versioned storage boundary for machine-derived chat copies. The same field
//! projections backfill old rows and guard INSERT/UPDATE/restore/late writes.
use crate::error::CoreError;
use rusqlite::Connection;

const POLICY: &str = "(SELECT json_set(p.value,'$._chatRevision',s.revision) FROM privacy_config p CROSS JOIN privacy_chat_state s WHERE p.key='privacy_config' AND s.id=1)";

struct Projection {
    name: &'static str,
    table: &'static str,
    condition: &'static str,
    fields: &'static [(&'static str, &'static str)],
}

const PROJECTIONS: &[Projection] = &[
    Projection { name: "message_text", table: "messages", condition: "{row}role!='user'", fields: &[("content","text"),("thinking","text"),("tool_calls_json","calls")] },
    Projection { name: "message_artifacts", table: "messages", condition: "1", fields: &[("artifacts_json","json")] },
    Projection { name: "archive_text", table: "archived_messages", condition: "{row}role!='user'", fields: &[("content","text"),("tool_calls_json","calls")] },
    Projection { name: "archive_artifacts", table: "archived_messages", condition: "1", fields: &[("artifacts_json","json")] },
    Projection { name: "turn_trace", table: "conversation_turns", condition: "1", fields: &[("trace_json","json")] },
    Projection { name: "run", table: "agent_task_runs", condition: "1", fields: &[("summary","text"),("error_message","text"),("plan_json","json"),("artifacts_json","json")] },
    Projection { name: "run_event", table: "agent_run_events", condition: "1", fields: &[("label","text"),("payload_json","event_payload")] },
    Projection { name: "task_event", table: "agent_task_run_events", condition: "1", fields: &[("label","text"),("payload_json","json")] },
    Projection { name: "subtask", table: "agent_subtask_runs", condition: "1", fields: &[("input_json","json"),("output_json","json"),("error_message","text")] },
    Projection { name: "trace", table: "agent_traces", condition: "1", fields: &[("trace_json","json"),("error_message","text")] },
    Projection { name: "resume", table: "task_resume_checkpoints", condition: "1", fields: &[("state_json","json"),("resume_prompt","text")] },
    Projection { name: "compaction", table: "context_compactions", condition: "1", fields: &[("summary","text")] },
    Projection { name: "context_notes", table: "context_history_windows", condition: "1", fields: &[("notes","text")] },
    Projection { name: "context_item", table: "context_history_items", condition: "{row}role!='user'", fields: &[("content","text")] },
    Projection { name: "scratchpad", table: "agent_scratchpad", condition: "1", fields: &[("content","text")] },
    Projection { name: "task_artifact", table: "agent_task_artifacts", condition: "{row}source!='manual'", fields: &[("title","text"),("summary","text"),("content","text"),("payload_json","json")] },
    Projection { name: "artifact_version", table: "agent_task_artifact_versions", condition: "COALESCE((SELECT source FROM agent_task_artifacts a WHERE a.id={row}artifact_id),'manual')!='manual'", fields: &[("title","text"),("summary","text"),("content","text"),("payload_json","json")] },
    Projection { name: "trajectory", table: "agent_trajectories", condition: "1", fields: &[("trajectory_json","json")] },
    Projection { name: "episode", table: "conversation_episodes", condition: "1", fields: &[("summary","text"),("evidence_json","json")] },
    Projection { name: "project_event", table: "project_events", condition: "json_extract({row}provenance_json,'$.author')='assistant'", fields: &[("title","text"),("summary","text"),("provenance_json","json")] },
    Projection { name: "project_item", table: "project_workspace_items", condition: "json_extract({row}provenance_json,'$.author')='assistant'", fields: &[("title","unique_title"),("summary","text"),("evidence_json","json")] },
    Projection { name: "learned_success", table: "learned_successes", condition: "1", fields: &[("response_summary","text")] },
    Projection { name: "user_memory", table: "user_memories", condition: "{row}source!='manual'", fields: &[("content","text")] },
    Projection { name: "project_memory", table: "project_memories", condition: "{row}source!='manual'", fields: &[("title","text"),("content","text")] },
    Projection { name: "procedural_memory", table: "agent_procedural_memories", condition: "{row}source!='manual'", fields: &[("title","text"),("content","text"),("tags_json","json")] },
    Projection { name: "activity_event", table: "activity_events", condition: "1", fields: &[("event_json","json")] },
    Projection { name: "knowledge_excerpt", table: "knowledge_evidence", condition: "{row}document_id IS NOT NULL", fields: &[("excerpt","text")] },
];

fn assignments(projection: &Projection, row: &str, update: bool) -> String {
    projection
        .fields
        .iter()
        .map(|(column, kind)| {
            let extra = match *kind {
                "calls" => format!("{row}artifacts_json"),
                "unique_title" => format!("{row}id"),
                "event_payload" => format!("json_object('kind',{row}kind,'stale',COALESCE((SELECT privacy_revision FROM agent_task_runs WHERE id={row}run_id),'initial')!=(SELECT revision FROM privacy_chat_state WHERE id=1))"),
                _ => "NULL".to_owned(),
            };
            let safe = format!("nexa_chat_privacy_v1({POLICY},'{kind}',{row}{column},{extra})");
            if update {
                format!("{column}=CASE WHEN OLD.{column} IS NEW.{column} THEN NEW.{column} ELSE {safe} END")
            } else { format!("{column}={safe}") }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn ledger_assignments(row: &str) -> String {
    let fields = [
        ("turnItemId", "turn_item_id"),
        ("sampleId", "sample_id"),
        ("visibleContent", "visible_content"),
        ("captureStatus", "capture_status"),
        ("requestId", "request_id"),
        ("responseId", "response_id"),
        ("rawResponseDigest", "raw_response_digest"),
        ("privacyFingerprint", "privacy_fingerprint"),
    ]
    .iter()
    .map(|(key, column)| format!("'{key}',{row}{column}"));
    let route = [
        ("providerEndpointId", "provider_endpoint_id"),
        ("providerFamily", "provider_family"),
        ("apiStyle", "api_style"),
        ("modelId", "model_id"),
        ("reasoningProfileId", "reasoning_profile_id"),
        ("reasoningProfileVersion", "reasoning_profile_version"),
        ("replayPolicy", "replay_policy"),
    ]
    .iter()
    .map(|(key, column)| format!("'{key}',{row}{column}"))
    .collect::<Vec<_>>()
    .join(",");
    let json_fields = [
        ("providerItems", "provider_items_json"),
        ("replayPayload", "replay_payload_json"),
        ("toolCalls", "tool_calls_json"),
    ]
    .iter()
    .map(|(key, column)| format!("'{key}',json({row}{column})"))
    .collect::<Vec<_>>()
    .join(",");
    let envelope = format!(
        "json_object({},'route',json_object({route}),{json_fields})",
        fields.collect::<Vec<_>>().join(",")
    );
    let safe = format!("nexa_chat_privacy_v1({POLICY},'json',{envelope},NULL)");
    [
        ("visible_content", "visibleContent"),
        ("provider_items_json", "providerItems"),
        ("replay_payload_json", "replayPayload"),
        ("tool_calls_json", "toolCalls"),
        ("capture_status", "captureStatus"),
        ("raw_response_digest", "rawResponseDigest"),
        ("privacy_fingerprint", "privacyFingerprint"),
    ]
    .iter()
    .map(|(column, key)| format!("{column}=json_extract({safe},'$.{key}')"))
    .collect::<Vec<_>>()
    .join(",")
}

fn install_trigger(
    conn: &Connection,
    name: &str,
    table: &str,
    event: &str,
    condition: &str,
    assignments: &str,
) -> Result<(), CoreError> {
    // The guard belongs to the statement transaction. It prevents recursive
    // re-application even for valid non-idempotent replacements such as a->aa.
    // Errors roll back both the payload mutation and this guard.
    conn.execute_batch(&format!(
        "CREATE TRIGGER privacy_chat_{name}_{event} AFTER {event} ON {table}
        WHEN (SELECT guard FROM privacy_chat_state WHERE id=1)=0
          AND json_extract({POLICY},'$.enabled')=1 AND ({condition})
        BEGIN
          UPDATE privacy_chat_state SET guard=1 WHERE id=1;
          UPDATE {table} SET {assignments} WHERE rowid=NEW.rowid;
          UPDATE privacy_chat_state SET guard=0 WHERE id=1;
        END;"
    ))?;
    Ok(())
}

pub(crate) fn install(conn: &Connection) -> Result<(), CoreError> {
    for projection in PROJECTIONS {
        let condition = projection.condition.replace("{row}", "NEW.");
        let insert_assignments = assignments(projection, "NEW.", false);
        install_trigger(
            conn,
            projection.name,
            projection.table,
            "INSERT",
            &condition,
            &insert_assignments,
        )?;
        let changed = projection
            .fields
            .iter()
            .map(|(column, _)| format!("OLD.{column} IS NOT NEW.{column}"))
            .collect::<Vec<_>>()
            .join(" OR ");
        install_trigger(
            conn,
            projection.name,
            projection.table,
            "UPDATE",
            &format!("({condition}) AND ({changed})"),
            &assignments(projection, "NEW.", true),
        )?;
    }
    for event in ["INSERT", "UPDATE"] {
        install_trigger(
            conn,
            "provider_ledger",
            "provider_turn_envelopes",
            event,
            "1",
            &ledger_assignments("NEW."),
        )?;
    }
    // Upgrade users whose policy is already enabled need the same backfill even
    // when they never open Settings or save that identical policy again.
    let configured: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM privacy_config WHERE key='privacy_config' AND json_extract(value,'$.enabled')=1)", [], |row| row.get(0))?;
    if configured {
        revoke(conn)?;
    }
    Ok(())
}

pub(crate) fn revoke(conn: &Connection) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE privacy_chat_state SET revision=lower(hex(randomblob(16))),guard=1 WHERE id=1",
        [],
    )?;
    for projection in PROJECTIONS {
        conn.execute(
            &format!(
                "UPDATE {} SET {} WHERE {}",
                projection.table,
                assignments(projection, "", false),
                projection
                    .condition
                    .replace("{row}", &format!("{}.", projection.table))
            ),
            [],
        )?;
    }
    conn.execute(
        &format!(
            "UPDATE provider_turn_envelopes SET {}",
            ledger_assignments("")
        ),
        [],
    )?;
    // These views carry content digests and must be rebuilt from the redacted
    // messages, not silently retain provenance for a different input snapshot.
    conn.execute("DELETE FROM context_history_windows", [])?;
    conn.execute("UPDATE context_compactions SET status='invalidated'", [])?;
    conn.execute(
        "UPDATE conversations SET active_context_compaction_id=NULL",
        [],
    )?;
    conn.execute("UPDATE privacy_chat_state SET guard=0 WHERE id=1", [])?;
    Ok(())
}
