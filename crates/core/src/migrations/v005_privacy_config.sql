-- Privacy configuration key-value storage
CREATE TABLE IF NOT EXISTS privacy_config (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
