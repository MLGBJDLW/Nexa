-- A conversation owns its model choice; connections continue to own secrets.
ALTER TABLE conversations ADD COLUMN agent_config_id TEXT;
ALTER TABLE conversations ADD COLUMN model_selection_json TEXT;
