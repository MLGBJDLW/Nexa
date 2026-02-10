-- Playbooks
CREATE TABLE IF NOT EXISTS playbooks (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    body_md TEXT NOT NULL DEFAULT '',
    goal TEXT DEFAULT '',
    prerequisites TEXT DEFAULT '',
    notes TEXT DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_playbooks_title ON playbooks(title);

-- Playbook citations (links to chunks)
CREATE TABLE IF NOT EXISTS playbook_citations (
    id TEXT PRIMARY KEY NOT NULL,
    playbook_id TEXT NOT NULL REFERENCES playbooks(id) ON DELETE CASCADE,
    chunk_id TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    sort_order INTEGER NOT NULL DEFAULT 0,
    annotation TEXT DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(playbook_id, chunk_id)
);

CREATE INDEX IF NOT EXISTS idx_pb_citations_playbook ON playbook_citations(playbook_id);

-- Query logs (for analytics and feedback loop)
CREATE TABLE IF NOT EXISTS query_logs (
    id TEXT PRIMARY KEY NOT NULL,
    query_text TEXT NOT NULL,
    filters_json TEXT DEFAULT '{}',
    result_count INTEGER NOT NULL DEFAULT 0,
    clicked_chunk_ids TEXT DEFAULT '[]',  -- JSON array
    pinned_chunk_ids TEXT DEFAULT '[]',   -- JSON array
    excluded_chunk_ids TEXT DEFAULT '[]', -- JSON array
    duration_ms INTEGER DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_query_logs_created ON query_logs(created_at);
