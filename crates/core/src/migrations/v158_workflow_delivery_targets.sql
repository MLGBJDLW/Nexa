-- A standalone schedule creates one conversation, atomically with this binding.
-- Keep a deleted target's identity: delivery must fail closed, not silently start
-- another conversation with a different history or source boundary.
CREATE TABLE workflow_automation_delivery_targets (
    automation_id TEXT PRIMARY KEY NOT NULL REFERENCES workflow_automations(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Recover the original chat for existing chat-authored schedules from successful
-- creation receipts. Read the small assistant tool-call envelope first, then seek
-- tool messages by conversation; never parse every large tool artifact at startup.
WITH calls AS MATERIALIZED (
    SELECT m.conversation_id, json_extract(t.value, '$.id') AS call_id
    FROM messages m, json_each(CASE WHEN json_valid(m.tool_calls_json) THEN m.tool_calls_json ELSE '[]' END) t
    WHERE m.role = 'assistant' AND m.tool_calls_json LIKE '%"automation"%'
      AND json_extract(t.value, '$.name') = 'automation'
), receipts AS MATERIALIZED (
    SELECT m.conversation_id, m.created_at,
           json_extract(m.artifacts_json, '$.result.task.id') AS automation_id
    FROM calls c JOIN messages m ON m.conversation_id = c.conversation_id
      AND m.role = 'tool' AND m.tool_call_id = c.call_id
    WHERE json_valid(m.artifacts_json)
      AND json_extract(m.artifacts_json, '$.kind') = 'automationResult'
      AND json_extract(m.artifacts_json, '$.result.created') = 1
)
INSERT OR IGNORE INTO workflow_automation_delivery_targets (automation_id, conversation_id)
SELECT r.automation_id, r.conversation_id FROM receipts r
JOIN workflow_automations a ON a.id = r.automation_id
JOIN conversations c ON c.id = r.conversation_id
ORDER BY r.created_at ASC;

-- A standalone/older schedule with no creation receipt keeps its first run chat.
INSERT OR IGNORE INTO workflow_automation_delivery_targets (automation_id, conversation_id)
SELECT r.automation_id, t.conversation_id FROM workflow_automation_runs r
JOIN agent_task_runs t ON t.id = r.task_run_id
JOIN conversations c ON c.id = t.conversation_id
ORDER BY r.created_at ASC;
