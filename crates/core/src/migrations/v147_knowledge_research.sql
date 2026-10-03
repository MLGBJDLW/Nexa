CREATE TABLE knowledge_service_config (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE research_sets (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    questions_json TEXT NOT NULL CHECK(json_valid(questions_json)),
    revision INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE research_documents (
    set_id TEXT NOT NULL REFERENCES research_sets(id) ON DELETE CASCADE,
    document_id TEXT NOT NULL,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    path TEXT NOT NULL,
    reference_json TEXT NOT NULL CHECK(json_valid(reference_json)),
    PRIMARY KEY(set_id,document_id)
);
CREATE TABLE research_cells (
    set_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    question_index INTEGER NOT NULL,
    document_revision TEXT NOT NULL DEFAULT '',
    review_state TEXT NOT NULL DEFAULT 'pending' CHECK(review_state IN ('pending','needs_review','supported','not_found','conflict')),
    note TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(evidence_json)),
    PRIMARY KEY(set_id,document_id,question_index),
    FOREIGN KEY(set_id,document_id) REFERENCES research_documents(set_id,document_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX idx_knowledge_active_research ON knowledge_jobs(kind) WHERE kind='research' AND status='running';
