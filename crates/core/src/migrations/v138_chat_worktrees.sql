CREATE TABLE chat_worktrees (
    conversation_id TEXT PRIMARY KEY NOT NULL,
    record_json TEXT NOT NULL
);
-- Ownership survives deleting a chat. A checkout is never implicitly removed.
CREATE TRIGGER protect_worktree_project_move BEFORE UPDATE OF project_id ON conversations
WHEN NEW.project_id IS NOT OLD.project_id AND EXISTS (
    SELECT 1 FROM chat_worktrees WHERE conversation_id=OLD.id
)
BEGIN SELECT RAISE(ABORT, 'A chat with a managed worktree must keep its project'); END;
