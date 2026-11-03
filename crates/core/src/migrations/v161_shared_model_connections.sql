-- A stable portable identity survives device-local IDs. A null target is a
-- local deletion tombstone, preventing a later sync from resurrecting it.
CREATE TABLE shared_model_connections (
    sync_id TEXT PRIMARY KEY NOT NULL,
    config_id TEXT UNIQUE REFERENCES agent_configs(id) ON DELETE SET NULL
);
