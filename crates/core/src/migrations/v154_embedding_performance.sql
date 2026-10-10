CREATE TABLE embedding_performance (
    space_id TEXT PRIMARY KEY NOT NULL,
    chars_per_second REAL NOT NULL CHECK(chars_per_second > 0),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
ALTER TABLE scan_errors ADD COLUMN input_fingerprint TEXT NOT NULL DEFAULT '';
