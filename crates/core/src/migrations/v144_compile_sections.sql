CREATE TABLE document_section_compilations (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    revision TEXT NOT NULL,
    route_key TEXT NOT NULL,
    section_index INTEGER NOT NULL,
    input_hash TEXT NOT NULL,
    char_count INTEGER NOT NULL,
    output_json TEXT NOT NULL CHECK(json_valid(output_json)),
    compiled_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY(document_id,revision,route_key,section_index)
);
CREATE TRIGGER invalidate_section_compilations AFTER UPDATE OF index_revision ON documents
WHEN NEW.index_revision != OLD.index_revision BEGIN
    DELETE FROM document_section_compilations WHERE document_id=NEW.id AND revision!=NEW.index_revision;
END;
