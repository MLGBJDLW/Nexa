CREATE TABLE knowledge_jobs (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('scan', 'scan-all', 'embed', 'rebuild-embeddings', 'compile', 'research')),
    source_id TEXT REFERENCES sources(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed', 'interrupted')),
    progress_json TEXT NOT NULL DEFAULT '{}',
    error TEXT,
    revision INTEGER NOT NULL DEFAULT 1,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    finished_at TEXT
);
CREATE INDEX idx_knowledge_jobs_status ON knowledge_jobs(status, started_at DESC);
CREATE UNIQUE INDEX idx_knowledge_active_compile ON knowledge_jobs(kind) WHERE kind='compile' AND status='running';
