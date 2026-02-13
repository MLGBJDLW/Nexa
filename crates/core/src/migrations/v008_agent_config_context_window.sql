-- Add optional context_window override to agent_configs.
-- NULL means auto-detect from model name.
ALTER TABLE agent_configs ADD COLUMN context_window INTEGER DEFAULT NULL;
