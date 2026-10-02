-- Display pagination seeks durable user anchors without reading tool/trace payloads.
-- The same predicate is used by conversation::timeline; this is a derived index,
-- never a second source of conversation or execution state.
CREATE INDEX IF NOT EXISTS idx_messages_timeline_roots
ON messages(conversation_id, sort_order, id)
WHERE role = 'user'
  AND COALESCE(CASE WHEN json_valid(artifacts_json) THEN json_extract(artifacts_json, '$.kind') END, '')
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
