-- Every live outbox batch checks for an authoritative terminal. Index only true
-- terminal rows so an open run does not rescan all earlier streaming fragments.
-- Legacy done(status=paused) events remain non-closing suspension boundaries.
CREATE INDEX IF NOT EXISTS idx_agent_run_events_terminal
ON agent_run_events(run_id)
WHERE kind = 'error'
   OR (kind = 'done' AND COALESCE(status, '') <> 'paused');

-- Task Center reads metadata in timestamp/sequence order. Streaming payloads
-- and internal diagnostics never enter this small derived index. Keep the
-- julianday ordering used by existing histories, including mixed ISO formats.
CREATE INDEX IF NOT EXISTS idx_agent_run_events_history
ON agent_run_events(run_id, julianday(created_at) DESC, event_seq DESC)
WHERE kind NOT IN ('outputDelta', 'outputSnapshot', 'thinking', 'usageUpdated')
  AND visibility != 'internal';
