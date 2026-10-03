ALTER TABLE documents ADD COLUMN index_revision TEXT NOT NULL DEFAULT '';
UPDATE documents SET index_revision=content_hash;
CREATE TRIGGER assign_document_revision_insert AFTER INSERT ON documents BEGIN
    UPDATE documents SET index_revision=lower(hex(randomblob(16))) WHERE id=NEW.id;
END;
CREATE TRIGGER assign_document_revision_update AFTER UPDATE OF content_hash,mime_type ON documents
WHEN NEW.index_revision=OLD.index_revision BEGIN
    UPDATE documents SET index_revision=lower(hex(randomblob(16))) WHERE id=NEW.id;
END;

CREATE TABLE evidence_snapshots (
    chunk_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    revision TEXT NOT NULL,
    document_hash TEXT NOT NULL,
    block_hash TEXT NOT NULL,
    document_path TEXT NOT NULL,
    document_title TEXT,
    document_metadata TEXT NOT NULL,
    content TEXT NOT NULL,
    chunk_index INTEGER NOT NULL,
    kind TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    archived_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    PRIMARY KEY (chunk_id, revision, block_hash)
);
CREATE INDEX idx_evidence_snapshot_document ON evidence_snapshots(document_id, revision, chunk_index);

CREATE TRIGGER archive_deleted_chunk BEFORE DELETE ON chunks BEGIN
    INSERT OR IGNORE INTO evidence_snapshots
      (chunk_id,document_id,source_id,revision,document_hash,block_hash,document_path,document_title,document_metadata,content,chunk_index,kind,metadata_json)
    SELECT OLD.id,OLD.document_id,d.source_id,d.index_revision,d.content_hash,OLD.content_hash,d.path,d.title,COALESCE(d.metadata,'{}'),OLD.content,OLD.chunk_index,OLD.kind,COALESCE(OLD.metadata_json,'{}')
    FROM documents d JOIN sources s ON s.id=d.source_id WHERE d.id=OLD.document_id;
END;
CREATE TRIGGER archive_updated_chunk BEFORE UPDATE OF content,content_hash,metadata_json ON chunks
WHEN NEW.content != OLD.content OR NEW.content_hash != OLD.content_hash OR NEW.metadata_json != OLD.metadata_json BEGIN
    INSERT OR IGNORE INTO evidence_snapshots
      (chunk_id,document_id,source_id,revision,document_hash,block_hash,document_path,document_title,document_metadata,content,chunk_index,kind,metadata_json)
    SELECT OLD.id,OLD.document_id,d.source_id,d.index_revision,d.content_hash,OLD.content_hash,d.path,d.title,COALESCE(d.metadata,'{}'),OLD.content,OLD.chunk_index,OLD.kind,COALESCE(OLD.metadata_json,'{}')
    FROM documents d JOIN sources s ON s.id=d.source_id WHERE d.id=OLD.document_id;
END;
-- Cascades remove the document before its chunks' DELETE triggers can see it.
CREATE TRIGGER archive_document_revision BEFORE UPDATE OF content_hash,mime_type ON documents BEGIN
    INSERT OR IGNORE INTO evidence_snapshots
      (chunk_id,document_id,source_id,revision,document_hash,block_hash,document_path,document_title,document_metadata,content,chunk_index,kind,metadata_json)
    SELECT c.id,OLD.id,OLD.source_id,OLD.index_revision,OLD.content_hash,c.content_hash,OLD.path,OLD.title,COALESCE(OLD.metadata,'{}'),c.content,c.chunk_index,c.kind,COALESCE(c.metadata_json,'{}')
    FROM chunks c JOIN sources s ON s.id=OLD.source_id WHERE c.document_id=OLD.id;
END;
CREATE TRIGGER archive_deleted_document BEFORE DELETE ON documents BEGIN
    INSERT OR IGNORE INTO evidence_snapshots
      (chunk_id,document_id,source_id,revision,document_hash,block_hash,document_path,document_title,document_metadata,content,chunk_index,kind,metadata_json)
    SELECT c.id,OLD.id,OLD.source_id,OLD.index_revision,OLD.content_hash,c.content_hash,OLD.path,OLD.title,COALESCE(OLD.metadata,'{}'),c.content,c.chunk_index,c.kind,COALESCE(c.metadata_json,'{}')
    FROM chunks c JOIN sources s ON s.id=OLD.source_id WHERE c.document_id=OLD.id;
END;

CREATE VIEW evidence_records AS
SELECT c.id AS chunk_id,c.document_id,d.source_id,d.index_revision AS revision,d.content_hash AS document_hash,c.content_hash AS block_hash,
       d.path AS document_path,d.title AS document_title,COALESCE(d.metadata,'{}') AS document_metadata,
       c.content,c.chunk_index,c.kind,COALESCE(c.metadata_json,'{}') AS metadata_json,
       'current' AS status,'' AS archived_at,s.root_path
FROM chunks c JOIN documents d ON d.id=c.document_id JOIN sources s ON s.id=d.source_id
UNION ALL
SELECT e.chunk_id,e.document_id,e.source_id,e.revision,e.document_hash,e.block_hash,e.document_path,e.document_title,e.document_metadata,
       e.content,e.chunk_index,e.kind,e.metadata_json,
       CASE WHEN d.id IS NULL THEN 'missing' ELSE 'historical' END,e.archived_at,s.root_path
FROM evidence_snapshots e JOIN sources s ON s.id=e.source_id LEFT JOIN documents d ON d.id=e.document_id;

ALTER TABLE document_summaries ADD COLUMN input_revision TEXT NOT NULL DEFAULT '';
ALTER TABLE document_summaries ADD COLUMN coverage_json TEXT NOT NULL DEFAULT '{}';
-- Existing summaries cannot prove which revision they read; queue them again.
ALTER TABLE knowledge_evidence ADD COLUMN document_id TEXT;
ALTER TABLE knowledge_evidence ADD COLUMN document_revision TEXT;
ALTER TABLE knowledge_evidence ADD COLUMN stale INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_knowledge_evidence_revision ON knowledge_evidence(document_id,document_revision);

CREATE TRIGGER invalidate_document_knowledge AFTER UPDATE OF index_revision ON documents
WHEN NEW.index_revision != OLD.index_revision BEGIN
    DELETE FROM document_entities WHERE document_id=NEW.id;
    DELETE FROM chunks WHERE document_id=NEW.id AND kind='summary';
    UPDATE knowledge_evidence SET stale=1 WHERE document_id=NEW.id AND document_revision!=NEW.index_revision;
    UPDATE knowledge_claims SET review_state='needs_review',provenance_json=json_set(provenance_json,'$.staleEvidence',1)
      WHERE id IN (SELECT claim_id FROM knowledge_evidence WHERE document_id=NEW.id AND stale=1);
    UPDATE knowledge_events SET review_state='needs_review',provenance_json=json_set(provenance_json,'$.staleEvidence',1)
      WHERE id IN (SELECT event_id FROM knowledge_evidence WHERE document_id=NEW.id AND stale=1);
END;
CREATE TRIGGER invalidate_deleted_document_knowledge BEFORE DELETE ON documents BEGIN
    UPDATE knowledge_evidence SET stale=1 WHERE document_id=OLD.id;
    UPDATE knowledge_claims SET review_state='needs_review',provenance_json=json_set(provenance_json,'$.staleEvidence',1)
      WHERE id IN (SELECT claim_id FROM knowledge_evidence WHERE document_id=OLD.id);
    UPDATE knowledge_events SET review_state='needs_review',provenance_json=json_set(provenance_json,'$.staleEvidence',1)
      WHERE id IN (SELECT event_id FROM knowledge_evidence WHERE document_id=OLD.id);
END;
