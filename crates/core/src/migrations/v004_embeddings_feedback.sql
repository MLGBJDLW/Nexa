-- Embeddings table: stores vector representations of chunks
CREATE TABLE IF NOT EXISTS embeddings (
    id TEXT PRIMARY KEY,
    chunk_id TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    model TEXT NOT NULL DEFAULT 'tfidf-v1',
    vector BLOB NOT NULL,
    dimensions INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(chunk_id, model)
);

CREATE INDEX IF NOT EXISTS idx_embeddings_chunk ON embeddings(chunk_id);
CREATE INDEX IF NOT EXISTS idx_embeddings_model ON embeddings(model);

-- Feedback table: user feedback on evidence cards
CREATE TABLE IF NOT EXISTS feedback (
    id TEXT PRIMARY KEY,
    chunk_id TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    query_text TEXT NOT NULL,
    action TEXT NOT NULL CHECK(action IN ('upvote', 'downvote', 'pin')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_feedback_chunk ON feedback(chunk_id);
CREATE INDEX IF NOT EXISTS idx_feedback_query ON feedback(query_text);
