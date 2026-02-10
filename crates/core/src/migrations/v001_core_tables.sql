-- Sources table
CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL DEFAULT 'local_folder',
    root_path TEXT NOT NULL UNIQUE,
    include_globs TEXT NOT NULL DEFAULT '[]',  -- JSON array of glob patterns
    exclude_globs TEXT NOT NULL DEFAULT '[]',  -- JSON array of glob patterns
    watch_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Documents table
CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY NOT NULL,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    title TEXT,
    mime_type TEXT NOT NULL DEFAULT 'text/plain',
    file_size INTEGER NOT NULL DEFAULT 0,
    modified_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    indexed_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(source_id, path)
);

CREATE INDEX IF NOT EXISTS idx_documents_source ON documents(source_id);
CREATE INDEX IF NOT EXISTS idx_documents_path ON documents(path);
CREATE INDEX IF NOT EXISTS idx_documents_modified ON documents(modified_at);

-- Chunks table
CREATE TABLE IF NOT EXISTS chunks (
    id TEXT PRIMARY KEY NOT NULL,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    kind TEXT NOT NULL DEFAULT 'text',  -- text, heading, code, log_entry
    content TEXT NOT NULL,
    start_offset INTEGER NOT NULL,  -- byte offset in original file
    end_offset INTEGER NOT NULL,    -- byte offset in original file
    line_start INTEGER NOT NULL,    -- 1-based line number
    line_end INTEGER NOT NULL,      -- 1-based line number
    content_hash TEXT NOT NULL,
    metadata_json TEXT DEFAULT '{}',  -- extra metadata per chunk type
    UNIQUE(document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_chunks_document ON chunks(document_id);
CREATE INDEX IF NOT EXISTS idx_chunks_hash ON chunks(content_hash);
