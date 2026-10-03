ALTER TABLE mcp_servers ADD COLUMN oauth_epoch INTEGER NOT NULL DEFAULT 0;
CREATE TABLE mcp_oauth (
    connector_id TEXT PRIMARY KEY REFERENCES mcp_servers(id) ON DELETE CASCADE,
    config_json TEXT NOT NULL,
    credential_id TEXT,
    expires_at INTEGER,
    scopes TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'signed_out',
    detail TEXT,
    login_id TEXT
);
CREATE TABLE mcp_oauth_cleanup (credential_id TEXT PRIMARY KEY);
CREATE TRIGGER mcp_oauth_secret_replaced BEFORE UPDATE OF credential_id ON mcp_oauth
WHEN OLD.credential_id IS NOT NULL AND OLD.credential_id IS NOT NEW.credential_id
BEGIN INSERT OR IGNORE INTO mcp_oauth_cleanup VALUES (OLD.credential_id); END;
CREATE TRIGGER mcp_oauth_secret_deleted BEFORE DELETE ON mcp_oauth
WHEN OLD.credential_id IS NOT NULL
BEGIN INSERT OR IGNORE INTO mcp_oauth_cleanup VALUES (OLD.credential_id); END;
CREATE TRIGGER mcp_oauth_configuration_changed AFTER UPDATE OF transport, command, args, url, env_json, headers_json, enabled ON mcp_servers
WHEN OLD.transport IS NOT NEW.transport OR OLD.command IS NOT NEW.command
  OR OLD.args IS NOT NEW.args OR OLD.url IS NOT NEW.url OR OLD.env_json IS NOT NEW.env_json
  OR OLD.headers_json IS NOT NEW.headers_json OR OLD.enabled IS NOT NEW.enabled
BEGIN
    UPDATE mcp_servers SET oauth_epoch = oauth_epoch + 1 WHERE id = NEW.id AND EXISTS(SELECT 1 FROM mcp_oauth WHERE connector_id = NEW.id);
    UPDATE mcp_oauth SET credential_id = NULL, expires_at = NULL, status = 'signed_out', detail = NULL, login_id = NULL WHERE connector_id = NEW.id;
END;
