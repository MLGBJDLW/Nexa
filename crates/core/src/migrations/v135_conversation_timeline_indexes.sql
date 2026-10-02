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
WHEN NEW.artifacts_json IS NOT NULL
BEGIN
    UPDATE messages SET display_artifact_kind = CASE
        WHEN NEW.artifacts_json IS NULL THEN NULL
        WHEN json_valid(NEW.artifacts_json) THEN COALESCE(json_extract(NEW.artifacts_json, '$.kind'), '')
        ELSE '__invalid_json__'
    END WHERE id = NEW.id;
END;
CREATE TRIGGER messages_display_artifact_kind_update AFTER UPDATE OF artifacts_json ON messages
WHEN NEW.artifacts_json IS NOT OLD.artifacts_json
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
-- Keep source-specific views: a UNION view expands both JSON classifiers into
-- every write's prepared trigger program even when one branch cannot execute.
-- UTF-8 whitespace constants match JS trim (includes U+FEFF, excludes U+0085)
-- and Rust str::trim (includes U+0085, excludes U+FEFF). Literal constants avoid
-- rebuilding long char() argument lists inside each row's trigger program.
CREATE VIEW timeline_message_trace_flags AS
SELECT id, (
    SELECT max(CASE WHEN json_extract(item.value, '$.kind') = 'thinking'
                       AND json_type(item.value, '$.text') = 'text'
                       AND trim(json_extract(item.value, '$.text'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> ''
                    THEN 1 ELSE 0 END)
         + 2 * max(CASE WHEN json_extract(item.value, '$.kind') = 'reply'
                           AND json_type(item.value, '$.text') = 'text'
                           AND trim(json_extract(item.value, '$.text'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> ''
                        THEN 1 ELSE 0 END)
    FROM json_each(CASE WHEN json_valid(artifacts_json) THEN
        CASE WHEN json_extract(artifacts_json, '$.kind') = 'traceTimeline'
                   AND json_type(artifacts_json, '$.items') = 'array'
             THEN artifacts_json ELSE '{"items":[]}' END
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
                     AND trim(json_extract(skill.value, '$.id'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> '')
                    OR (json_type(skill.value, '$.name') = 'text'
                        AND trim(json_extract(skill.value, '$.name'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> '')
                    OR (json_type(skill.value, '$.displayName') = 'text'
                        AND trim(json_extract(skill.value, '$.displayName'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> '')
                ELSE 0 END
            )
            ELSE 0 END
        ELSE 0 END
) AS flags
FROM messages;

CREATE VIEW timeline_turn_trace_flags AS
SELECT id, (
    SELECT max(CASE WHEN json_extract(item.value, '$.kind') = 'thinking'
                       AND json_type(item.value, '$.text') = 'text'
                       AND trim(json_extract(item.value, '$.text'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> ''
                    THEN 1 ELSE 0 END)
         + 2 * max(CASE WHEN json_extract(item.value, '$.kind') = 'reply'
                           AND json_type(item.value, '$.text') = 'text'
                           AND trim(json_extract(item.value, '$.text'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> ''
                        THEN 1 ELSE 0 END)
    FROM json_each(CASE WHEN json_valid(trace_json) THEN
        CASE WHEN json_extract(trace_json, '$.kind') = 'turnTrace'
                   AND json_type(trace_json, '$.items') = 'array'
             THEN trace_json ELSE '{"items":[]}' END
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
                     AND trim(json_extract(skill.value, '$.id'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> '')
                    OR (json_type(skill.value, '$.name') = 'text'
                        AND trim(json_extract(skill.value, '$.name'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> '')
                    OR (json_type(skill.value, '$.displayName') = 'text'
                        AND trim(json_extract(skill.value, '$.displayName'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> '')
                ELSE 0 END
            )
            ELSE 0 END
        ELSE 0 END
) AS flags
FROM conversation_turns;

CREATE VIEW timeline_message_reasoning_candidate AS
SELECT id, CASE WHEN role = 'assistant'
    AND thinking IS NOT NULL
    AND trim(thinking, CAST(X'090A0B0C0D20C285C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080' AS TEXT)) <> '[reasoning content unavailable in local history]'
    AND replace(trim(content, CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)), char(13,10), char(10)) <> ''
    AND replace(trim(content, CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)), char(13,10), char(10)) = replace(trim(trim(thinking, CAST(X'090A0B0C0D20C285C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080' AS TEXT)), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)), char(13,10), char(10))
    THEN 1 ELSE 0 END AS candidate
FROM messages;

UPDATE messages SET
    display_reasoning_candidate = (SELECT candidate FROM timeline_message_reasoning_candidate WHERE id = messages.id)
WHERE role = 'assistant' AND thinking IS NOT NULL;
UPDATE messages SET
    display_legacy_trace_flags = (SELECT flags FROM timeline_message_trace_flags WHERE id = messages.id)
WHERE artifacts_json IS NOT NULL;
UPDATE conversation_turns SET display_trace_flags =
    (SELECT flags FROM timeline_turn_trace_flags WHERE id = conversation_turns.id)
WHERE trace_json IS NOT NULL;

CREATE TRIGGER messages_display_reasoning_insert AFTER INSERT ON messages
WHEN NEW.role = 'assistant' AND NEW.thinking IS NOT NULL
BEGIN
    UPDATE messages SET display_reasoning_candidate =
        (SELECT candidate FROM timeline_message_reasoning_candidate WHERE id = NEW.id)
    WHERE id = NEW.id;
END;
CREATE TRIGGER messages_display_reasoning_update AFTER UPDATE OF role, content, thinking ON messages
WHEN NEW.role IS NOT OLD.role OR NEW.content IS NOT OLD.content OR NEW.thinking IS NOT OLD.thinking
BEGIN
    UPDATE messages SET display_reasoning_candidate =
        (SELECT candidate FROM timeline_message_reasoning_candidate WHERE id = NEW.id)
    WHERE id = NEW.id;
END;
CREATE TRIGGER messages_display_legacy_trace_insert AFTER INSERT ON messages
WHEN NEW.artifacts_json IS NOT NULL
BEGIN
    UPDATE messages SET display_legacy_trace_flags =
        (SELECT flags FROM timeline_message_trace_flags WHERE id = NEW.id)
    WHERE id = NEW.id;
END;
CREATE TRIGGER messages_display_legacy_trace_update AFTER UPDATE OF artifacts_json ON messages
WHEN NEW.artifacts_json IS NOT OLD.artifacts_json
BEGIN
    UPDATE messages SET display_legacy_trace_flags = CASE WHEN NEW.artifacts_json IS NULL THEN NULL
        ELSE (SELECT flags FROM timeline_message_trace_flags WHERE id = NEW.id) END
    WHERE id = NEW.id;
END;
CREATE TRIGGER conversation_turns_display_trace_flags_insert AFTER INSERT ON conversation_turns
WHEN NEW.trace_json IS NOT NULL
BEGIN
    UPDATE conversation_turns SET display_trace_flags =
        (SELECT flags FROM timeline_turn_trace_flags WHERE id = NEW.id)
    WHERE id = NEW.id;
END;
CREATE TRIGGER conversation_turns_display_trace_flags_update AFTER UPDATE OF trace_json ON conversation_turns
WHEN NEW.trace_json IS NOT OLD.trace_json
BEGIN
    UPDATE conversation_turns SET display_trace_flags = CASE WHEN NEW.trace_json IS NULL THEN NULL
        ELSE (SELECT flags FROM timeline_turn_trace_flags WHERE id = NEW.id) END
    WHERE id = NEW.id;
END;
