-- Versioned row-owned snapshots retain legacy TEXT/NULL reads. The Rust codec
-- and privacy projections are installed in the same migration transaction.
ALTER TABLE agent_task_runs ADD COLUMN artifacts_preview_json TEXT;
ALTER TABLE agent_subtask_runs ADD COLUMN output_preview_json TEXT;

-- Decode once for the selected turn, before the existing trace classification.
-- The outer view remains seekable by id; no global materialized history scan.
DROP VIEW timeline_turn_trace_flags;
CREATE VIEW timeline_turn_trace_flags AS
SELECT id, (
    WITH decoded AS MATERIALIZED (
        SELECT nexa_snapshot_json_v1(conversation_turns.trace_json) AS trace_json
    )
    SELECT max(CASE WHEN json_extract(item.value, '$.kind') = 'thinking'
                       AND json_type(item.value, '$.text') = 'text'
                       AND trim(json_extract(item.value, '$.text'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> ''
                    THEN 1 ELSE 0 END)
         + 2 * max(CASE WHEN json_extract(item.value, '$.kind') = 'reply'
                           AND json_type(item.value, '$.text') = 'text'
                           AND trim(json_extract(item.value, '$.text'), CAST(X'090A0B0C0D20C2A0E19A80E28080E28081E28082E28083E28084E28085E28086E28087E28088E28089E2808AE280A8E280A9E280AFE2819FE38080EFBBBF' AS TEXT)) <> ''
                        THEN 1 ELSE 0 END)
    FROM decoded, json_each(CASE WHEN json_valid(trace_json) THEN
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
