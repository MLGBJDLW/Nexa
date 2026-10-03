CREATE TABLE source_index_state (
    source_id TEXT PRIMARY KEY NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    result_json TEXT NOT NULL CHECK(json_valid(result_json)),
    scanned_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
