CREATE TABLE entity_link_support (
    source_entity_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    target_entity_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    relation_type TEXT NOT NULL,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    revision TEXT NOT NULL,
    strength REAL NOT NULL,
    snippet TEXT NOT NULL,
    confidence REAL,
    PRIMARY KEY(source_entity_id,target_entity_id,relation_type,document_id)
);
-- Legacy edges cannot prove a revision and are intentionally queued for compilation.
DELETE FROM entity_links WHERE evidence_doc_id IS NOT NULL;
CREATE TRIGGER project_entity_support_insert AFTER INSERT ON entity_link_support BEGIN

    INSERT INTO entity_links(id,source_entity_id,target_entity_id,relation_type,strength,evidence_doc_id,evidence_snippet,confidence)
    SELECT lower(hex(randomblob(16))),source_entity_id,target_entity_id,relation_type,strength,document_id,snippet,confidence
    FROM entity_link_support WHERE source_entity_id=NEW.source_entity_id AND target_entity_id=NEW.target_entity_id AND relation_type=NEW.relation_type
    ORDER BY strength DESC,document_id LIMIT 1
    ON CONFLICT(source_entity_id,target_entity_id,relation_type) DO UPDATE SET strength=excluded.strength,evidence_doc_id=excluded.evidence_doc_id,evidence_snippet=excluded.evidence_snippet,confidence=excluded.confidence;
END;
CREATE TRIGGER project_entity_support_update AFTER UPDATE ON entity_link_support BEGIN

    INSERT INTO entity_links(id,source_entity_id,target_entity_id,relation_type,strength,evidence_doc_id,evidence_snippet,confidence)
    SELECT lower(hex(randomblob(16))),source_entity_id,target_entity_id,relation_type,strength,document_id,snippet,confidence
    FROM entity_link_support WHERE source_entity_id=NEW.source_entity_id AND target_entity_id=NEW.target_entity_id AND relation_type=NEW.relation_type
    ORDER BY strength DESC,document_id LIMIT 1
    ON CONFLICT(source_entity_id,target_entity_id,relation_type) DO UPDATE SET strength=excluded.strength,evidence_doc_id=excluded.evidence_doc_id,evidence_snippet=excluded.evidence_snippet,confidence=excluded.confidence;
END;
CREATE TRIGGER project_entity_support_delete AFTER DELETE ON entity_link_support BEGIN
    DELETE FROM entity_links WHERE source_entity_id=OLD.source_entity_id AND target_entity_id=OLD.target_entity_id AND relation_type=OLD.relation_type AND evidence_doc_id IS NOT NULL;

    INSERT INTO entity_links(id,source_entity_id,target_entity_id,relation_type,strength,evidence_doc_id,evidence_snippet,confidence)
    SELECT lower(hex(randomblob(16))),source_entity_id,target_entity_id,relation_type,strength,document_id,snippet,confidence
    FROM entity_link_support WHERE source_entity_id=OLD.source_entity_id AND target_entity_id=OLD.target_entity_id AND relation_type=OLD.relation_type
    ORDER BY strength DESC,document_id LIMIT 1
    ON CONFLICT(source_entity_id,target_entity_id,relation_type) DO UPDATE SET strength=excluded.strength,evidence_doc_id=excluded.evidence_doc_id,evidence_snippet=excluded.evidence_snippet,confidence=excluded.confidence;
END;
CREATE TRIGGER invalidate_entity_support AFTER UPDATE OF index_revision ON documents
WHEN NEW.index_revision!=OLD.index_revision BEGIN
    DELETE FROM entity_link_support WHERE document_id=NEW.id;
END;
CREATE TRIGGER remove_entity_support BEFORE DELETE ON documents BEGIN
    DELETE FROM entity_link_support WHERE document_id=OLD.id;
END;
CREATE TRIGGER count_entity_documents_insert AFTER INSERT ON document_entities BEGIN
    UPDATE entities SET mention_count=(SELECT COUNT(*) FROM document_entities WHERE entity_id=NEW.entity_id) WHERE id=NEW.entity_id;
END;
CREATE TRIGGER count_entity_documents_delete AFTER DELETE ON document_entities BEGIN
    UPDATE entities SET mention_count=(SELECT COUNT(*) FROM document_entities WHERE entity_id=OLD.entity_id) WHERE id=OLD.entity_id;
END;
UPDATE entities SET mention_count=(SELECT COUNT(*) FROM document_entities WHERE entity_id=entities.id);

-- Resolve indexed block IDs and document IDs to the revision read at creation.
CREATE TRIGGER bind_knowledge_evidence AFTER INSERT ON knowledge_evidence BEGIN
    UPDATE knowledge_evidence SET
      document_id=COALESCE((SELECT document_id FROM chunks WHERE id=NEW.source_ref),(SELECT id FROM documents WHERE id=NEW.source_ref)),
      document_revision=(SELECT index_revision FROM documents WHERE id=COALESCE((SELECT document_id FROM chunks WHERE id=NEW.source_ref),(SELECT id FROM documents WHERE id=NEW.source_ref))),
      locator_json=COALESCE((SELECT metadata_json FROM chunks WHERE id=NEW.source_ref),'{}')
    WHERE id=NEW.id AND NEW.document_id IS NULL;
END;
-- Unversioned legacy assertions remain reviewable; never certify them retroactively.
UPDATE knowledge_claims SET review_state='needs_review' WHERE id IN (SELECT claim_id FROM knowledge_evidence WHERE document_revision IS NULL);
UPDATE knowledge_events SET review_state='needs_review' WHERE id IN (SELECT event_id FROM knowledge_evidence WHERE document_revision IS NULL);
