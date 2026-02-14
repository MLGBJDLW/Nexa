-- Add reasoning/thinking fields to agent_configs.
ALTER TABLE agent_configs ADD COLUMN reasoning_enabled BOOLEAN DEFAULT NULL;
ALTER TABLE agent_configs ADD COLUMN thinking_budget INTEGER DEFAULT NULL;
ALTER TABLE agent_configs ADD COLUMN reasoning_effort TEXT DEFAULT NULL;
