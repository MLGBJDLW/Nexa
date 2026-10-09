-- Full delegated histories follow the lifetime of the owning chat. Other
-- activity adapters retain their existing independent session lifecycles.
CREATE TRIGGER delete_subagent_history_with_conversation AFTER DELETE ON conversations
BEGIN
    DELETE FROM activity_records WHERE conversation_id=OLD.id
      AND CASE WHEN json_valid(record_json) THEN json_extract(record_json,'$.ownerTool')='spawn_subagent' ELSE 0 END;
END;
