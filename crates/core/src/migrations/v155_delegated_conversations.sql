-- Official runtimes persist child transcripts through the same conversation
-- store. The parent owns their lifetime; they never appear as top-level chats.
ALTER TABLE conversations ADD COLUMN parent_conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE;
CREATE INDEX idx_conversations_parent ON conversations(parent_conversation_id);
