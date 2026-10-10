-- Provider/model names alone are not unique across subscription, metered and
-- regional endpoints. Keep existing IDs for historical references; the next
-- registry synchronization creates endpoint-scoped definitions for new runs.
CREATE TABLE model_definitions_v153 (
    id TEXT PRIMARY KEY NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version = 2),
    provider_id TEXT NOT NULL,
    canonical_model_id TEXT NOT NULL,
    descriptor_json TEXT NOT NULL CHECK (json_valid(descriptor_json)),
    descriptor_hash TEXT NOT NULL,
    source TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT INTO model_definitions_v153 SELECT * FROM model_definitions;
DROP TABLE model_definitions;
ALTER TABLE model_definitions_v153 RENAME TO model_definitions;
CREATE INDEX idx_model_definitions_provider_model ON model_definitions(provider_id, canonical_model_id);
