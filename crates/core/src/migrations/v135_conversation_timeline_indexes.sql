-- Display pagination seeks durable user anchors without reading tool/trace payloads.
-- The same predicate is used by conversation::timeline; this is a derived index,
-- never a second source of conversation or execution state.
-- Classify legacy artifacts once on migration/write. Summary reads must not
-- parse a complete traceTimeline merely to remove its hidden items afterward.
ALTER TABLE messages ADD COLUMN display_artifact_kind TEXT;
UPDATE messages SET display_artifact_kind = CASE
    WHEN artifacts_json IS NULL THEN NULL
    WHEN json_valid(artifacts_json) THEN COALESCE(json_extract(artifacts_json, '$.kind'), '')
    ELSE '__invalid_json__'
END;
CREATE TRIGGER messages_display_artifact_kind_insert AFTER INSERT ON messages
BEGIN
    UPDATE messages SET display_artifact_kind = CASE
        WHEN NEW.artifacts_json IS NULL THEN NULL
        WHEN json_valid(NEW.artifacts_json) THEN COALESCE(json_extract(NEW.artifacts_json, '$.kind'), '')
        ELSE '__invalid_json__'
    END WHERE id = NEW.id;
END;
CREATE TRIGGER messages_display_artifact_kind_update AFTER UPDATE OF artifacts_json ON messages
BEGIN
    UPDATE messages SET display_artifact_kind = CASE
        WHEN NEW.artifacts_json IS NULL THEN NULL
        WHEN json_valid(NEW.artifacts_json) THEN COALESCE(json_extract(NEW.artifacts_json, '$.kind'), '')
        ELSE '__invalid_json__'
    END WHERE id = NEW.id;
END;

CREATE INDEX IF NOT EXISTS idx_messages_timeline_roots
ON messages(conversation_id, sort_order, id)
WHERE role = 'user'
  AND COALESCE(display_artifact_kind, '')
      NOT IN ('steering', 'questionResponse', 'checkpointContinuation');

CREATE INDEX IF NOT EXISTS idx_messages_conversation_role_order
ON messages(conversation_id, role, sort_order, id);

CREATE INDEX IF NOT EXISTS idx_messages_conversation_order
ON messages(conversation_id, sort_order, id);

CREATE INDEX IF NOT EXISTS idx_conversation_turns_user
ON conversation_turns(conversation_id, user_message_id, created_at, id);

CREATE INDEX IF NOT EXISTS idx_agent_task_runs_conversation_user
ON agent_task_runs(conversation_id, user_message_id, created_at, id);

-- Timestamps have second precision. A monotonic scalar prevents two same-second
-- trace writes from sharing a lazy-detail version without reading the trace blob.
ALTER TABLE conversation_turns ADD COLUMN display_revision INTEGER NOT NULL DEFAULT 0;
CREATE TRIGGER conversation_turns_display_revision
AFTER UPDATE OF status, assistant_message_id, route_kind, trace_json ON conversation_turns
BEGIN
    UPDATE conversation_turns SET display_revision = OLD.display_revision + 1 WHERE id = NEW.id;
END;
