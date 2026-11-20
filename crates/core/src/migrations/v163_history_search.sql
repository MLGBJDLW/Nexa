-- One small entry per top-level conversation makes literal history pages stop
-- after enough matches. The message index and source rows already exist.
CREATE INDEX idx_conversations_history_page
ON conversations((archived_at IS NOT NULL), updated_at DESC, id DESC)
WHERE parent_conversation_id IS NULL;

-- Reproject only rows with adjoining scripts. Keep unmodified text in messages
-- for every snippet and consult the current row after privacy redaction.
UPDATE fts_messages SET content=nexa_chat_terms_v1(content)
WHERE content!=nexa_chat_terms_v1(content);
DROP TRIGGER messages_fts_ai;
DROP TRIGGER messages_fts_au;
CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
    DELETE FROM fts_messages WHERE message_id=NEW.id;
    INSERT INTO fts_messages(content,conversation_id,message_id,role)
        SELECT nexa_chat_terms_v1(content),conversation_id,id,role FROM messages WHERE id=NEW.id
          AND role IN ('user','assistant') AND content!='';
END;
CREATE TRIGGER messages_fts_au AFTER UPDATE OF content ON messages BEGIN
    DELETE FROM fts_messages WHERE message_id=NEW.id;
    INSERT INTO fts_messages(content,conversation_id,message_id,role)
        SELECT nexa_chat_terms_v1(content),conversation_id,id,role FROM messages WHERE id=NEW.id
          AND role IN ('user','assistant') AND content!='';
END;
