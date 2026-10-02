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

-- Preserve the existing reasoning-only display guard without selecting hidden
-- thinking or parsing full traces on every summary read. These are derived
-- scalars only; canonical message/trace payloads remain unchanged for details.
ALTER TABLE messages ADD COLUMN display_reasoning_candidate INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN display_legacy_trace_flags INTEGER;
ALTER TABLE conversation_turns ADD COLUMN display_trace_flags INTEGER;

-- NULL means no projected item, 0 means valid items without nonempty reasoning
-- or reply, bit 1 means nonempty thinking and bit 2 means a nonempty reply.
-- Match persistedTrace.ts: even an empty text item is a valid projection.
CREATE VIEW timeline_trace_display_flags AS
SELECT source, id, (
    SELECT max(CASE WHEN json_extract(item.value, '$.kind') = 'thinking'
                       AND json_type(item.value, '$.text') = 'text'
                       AND trim(json_extract(item.value, '$.text'), char(9,10,11,12,13,32,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)) <> ''
                    THEN 1 ELSE 0 END)
         + 2 * max(CASE WHEN json_extract(item.value, '$.kind') = 'reply'
                           AND json_type(item.value, '$.text') = 'text'
                           AND trim(json_extract(item.value, '$.text'), char(9,10,11,12,13,32,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)) <> ''
                        THEN 1 ELSE 0 END)
    FROM json_each(CASE WHEN json_valid(payload) THEN
        CASE WHEN json_extract(payload, '$.kind') = expected_kind
                   AND json_type(payload, '$.items') = 'array'
             THEN payload ELSE '{"items":[]}' END
        ELSE '{"items":[]}' END, '$.items') item
    WHERE CASE WHEN item.type = 'object' THEN
        CASE json_extract(item.value, '$.kind')
            WHEN 'thinking' THEN json_type(item.value, '$.text') = 'text'
            WHEN 'reply' THEN json_type(item.value, '$.text') = 'text'
            WHEN 'status' THEN json_type(item.value, '$.text') = 'text'
            WHEN 'tool' THEN CASE
                WHEN json_type(item.value, '$.toolCall') = 'object' THEN
                    json_type(item.value, '$.toolCall.callId') = 'text'
                    AND json_type(item.value, '$.toolCall.toolName') = 'text'
                WHEN json_type(item.value, '$.tool_call') = 'object' THEN
                    json_type(item.value, '$.tool_call.callId') = 'text'
                    AND json_type(item.value, '$.tool_call.toolName') = 'text'
                ELSE 0 END
            WHEN 'skillSelection' THEN EXISTS (
                SELECT 1 FROM json_each(CASE
                    WHEN json_type(item.value, '$.skills') = 'array'
                    THEN json_extract(item.value, '$.skills') ELSE '[]' END) skill
                WHERE CASE WHEN skill.type = 'object' THEN
                    (json_type(skill.value, '$.id') = 'text'
                     AND trim(json_extract(skill.value, '$.id'), char(9,10,11,12,13,32,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)) <> '')
                    OR (json_type(skill.value, '$.name') = 'text'
                        AND trim(json_extract(skill.value, '$.name'), char(9,10,11,12,13,32,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)) <> '')
                    OR (json_type(skill.value, '$.displayName') = 'text'
                        AND trim(json_extract(skill.value, '$.displayName'), char(9,10,11,12,13,32,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)) <> '')
                ELSE 0 END
            )
            ELSE 0 END
        ELSE 0 END
) AS flags
FROM (
    SELECT 'message' AS source, id, artifacts_json AS payload, 'traceTimeline' AS expected_kind FROM messages
    UNION ALL
    SELECT 'turn' AS source, id, trace_json AS payload, 'turnTrace' AS expected_kind FROM conversation_turns
);

CREATE VIEW timeline_message_reasoning_candidate AS
SELECT id, CASE WHEN role = 'assistant'
    AND thinking IS NOT NULL
    AND trim(thinking, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)) <> '[reasoning content unavailable in local history]'
    AND replace(trim(content, char(9,10,11,12,13,32,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)), char(13,10), char(10)) <> ''
    AND replace(trim(content, char(9,10,11,12,13,32,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)), char(13,10), char(10)) = replace(trim(trim(thinking, char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288)), char(9,10,11,12,13,32,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)), char(13,10), char(10))
    THEN 1 ELSE 0 END AS candidate
FROM messages;

UPDATE messages SET
    display_reasoning_candidate = (SELECT candidate FROM timeline_message_reasoning_candidate WHERE id = messages.id),
    display_legacy_trace_flags = (SELECT flags FROM timeline_trace_display_flags WHERE source = 'message' AND id = messages.id);
UPDATE conversation_turns SET display_trace_flags =
    (SELECT flags FROM timeline_trace_display_flags WHERE source = 'turn' AND id = conversation_turns.id);

CREATE TRIGGER messages_display_reasoning_insert AFTER INSERT ON messages
BEGIN
    UPDATE messages SET
        display_reasoning_candidate = (SELECT candidate FROM timeline_message_reasoning_candidate WHERE id = NEW.id),
        display_legacy_trace_flags = (SELECT flags FROM timeline_trace_display_flags WHERE source = 'message' AND id = NEW.id)
    WHERE id = NEW.id;
END;
CREATE TRIGGER messages_display_reasoning_update AFTER UPDATE OF role, content, thinking, artifacts_json ON messages
BEGIN
    UPDATE messages SET
        display_reasoning_candidate = (SELECT candidate FROM timeline_message_reasoning_candidate WHERE id = NEW.id),
        display_legacy_trace_flags = (SELECT flags FROM timeline_trace_display_flags WHERE source = 'message' AND id = NEW.id)
    WHERE id = NEW.id;
END;
CREATE TRIGGER conversation_turns_display_trace_flags_insert AFTER INSERT ON conversation_turns
BEGIN
    UPDATE conversation_turns SET display_trace_flags =
        (SELECT flags FROM timeline_trace_display_flags WHERE source = 'turn' AND id = NEW.id)
    WHERE id = NEW.id;
END;
CREATE TRIGGER conversation_turns_display_trace_flags_update AFTER UPDATE OF trace_json ON conversation_turns
BEGIN
    UPDATE conversation_turns SET display_trace_flags =
        (SELECT flags FROM timeline_trace_display_flags WHERE source = 'turn' AND id = NEW.id)
    WHERE id = NEW.id;
END;
