CREATE TABLE privacy_chat_state (
    id INTEGER PRIMARY KEY CHECK (id=1),
    revision TEXT NOT NULL,
    guard INTEGER NOT NULL DEFAULT 0 CHECK (guard IN (0,1))
);
INSERT INTO privacy_chat_state(id,revision) VALUES(1,'initial');
ALTER TABLE provider_turn_envelopes ADD COLUMN privacy_fingerprint TEXT;
ALTER TABLE agent_task_runs ADD COLUMN privacy_revision TEXT NOT NULL DEFAULT 'initial';
CREATE TRIGGER privacy_chat_run_revision AFTER INSERT ON agent_task_runs BEGIN
    UPDATE agent_task_runs SET privacy_revision=(SELECT revision FROM privacy_chat_state WHERE id=1) WHERE id=NEW.id;
END;

-- An AFTER redaction trigger may update the same row before or after the
-- original INSERT trigger. Always replace the projection from the CURRENT row,
-- never from the original NEW image, which can still contain revoked content.
DROP TRIGGER messages_fts_ai;
DROP TRIGGER messages_fts_au;
CREATE TRIGGER messages_fts_ai AFTER INSERT ON messages BEGIN
    DELETE FROM fts_messages WHERE message_id=NEW.id;
    INSERT INTO fts_messages(content,conversation_id,message_id,role)
        SELECT content,conversation_id,id,role FROM messages WHERE id=NEW.id
          AND role IN ('user','assistant') AND content!='';
END;
CREATE TRIGGER messages_fts_au AFTER UPDATE OF content ON messages BEGIN
    DELETE FROM fts_messages WHERE message_id=NEW.id;
    INSERT INTO fts_messages(content,conversation_id,message_id,role)
        SELECT content,conversation_id,id,role FROM messages WHERE id=NEW.id
          AND role IN ('user','assistant') AND content!='';
END;
DROP TRIGGER agent_procedural_memories_fts_ai;
DROP TRIGGER agent_procedural_memories_fts_au;
CREATE TRIGGER agent_procedural_memories_fts_ai AFTER INSERT ON agent_procedural_memories BEGIN
    DELETE FROM fts_agent_procedural_memories WHERE memory_id=NEW.id;
    INSERT INTO fts_agent_procedural_memories(title,content,tags,memory_id)
        SELECT title,content,tags_json,id FROM agent_procedural_memories WHERE id=NEW.id;
END;
CREATE TRIGGER agent_procedural_memories_fts_au AFTER UPDATE ON agent_procedural_memories BEGIN
    DELETE FROM fts_agent_procedural_memories WHERE memory_id=NEW.id;
    INSERT INTO fts_agent_procedural_memories(title,content,tags,memory_id)
        SELECT title,content,tags_json,id FROM agent_procedural_memories WHERE id=NEW.id;
END;
DELETE FROM fts_messages;
INSERT INTO fts_messages(content,conversation_id,message_id,role)
    SELECT content,conversation_id,id,role FROM messages WHERE role IN ('user','assistant') AND content!='';
DELETE FROM fts_agent_procedural_memories;
INSERT INTO fts_agent_procedural_memories(title,content,tags,memory_id)
    SELECT title,content,tags_json,id FROM agent_procedural_memories;
