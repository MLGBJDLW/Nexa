CREATE TABLE IF NOT EXISTS vector_store_config (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    owner_id TEXT NOT NULL,
    config_json TEXT NOT NULL,
    last_error TEXT
);
-- No chunk foreign key: receipts survive deletion until the remote tombstone
-- is acknowledged. Credentials live only in the encrypted configuration.
CREATE TABLE IF NOT EXISTS vector_store_receipts (
    store_id TEXT NOT NULL,
    space_id TEXT NOT NULL,
    chunk_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    embedding_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    PRIMARY KEY (store_id, space_id, chunk_id)
);
