CREATE TABLE context_history_windows (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    turn_id TEXT,
    notes TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    message_count INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_context_history_conversation ON context_history_windows(conversation_id, created_at);
CREATE TABLE context_history_items (
    window_id TEXT NOT NULL REFERENCES context_history_windows(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    has_images INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (window_id, ordinal)
);
-- Edited/deleted history must not be resurrected by the archive tool.
CREATE TRIGGER invalidate_context_history_delete AFTER DELETE ON messages
BEGIN
    DELETE FROM context_history_windows WHERE conversation_id=OLD.conversation_id;
END;
CREATE TRIGGER invalidate_context_history_edit AFTER UPDATE OF content, role, tool_calls_json ON messages
WHEN OLD.content IS NOT NEW.content OR OLD.role IS NOT NEW.role OR OLD.tool_calls_json IS NOT NEW.tool_calls_json
BEGIN
    DELETE FROM context_history_windows WHERE conversation_id=OLD.conversation_id;
END;
