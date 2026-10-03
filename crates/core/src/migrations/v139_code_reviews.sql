CREATE TABLE code_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    record_json TEXT NOT NULL
);
CREATE INDEX code_reviews_conversation ON code_reviews(conversation_id);
CREATE TABLE code_review_versions (
    review_id TEXT NOT NULL REFERENCES code_reviews(id) ON DELETE CASCADE,
    revision TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    PRIMARY KEY(review_id, revision)
);
CREATE TABLE conversation_code_reviews (
    conversation_id TEXT PRIMARY KEY NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    review_id TEXT NOT NULL REFERENCES code_reviews(id) ON DELETE CASCADE
);
