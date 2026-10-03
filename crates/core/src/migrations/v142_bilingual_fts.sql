DROP TRIGGER IF EXISTS chunks_ai;
DROP TRIGGER IF EXISTS chunks_ad;
DROP TRIGGER IF EXISTS chunks_au;
DROP TABLE fts_chunks;

CREATE VIEW fts_chunk_content AS
SELECT rowid, nexa_lexical_v1(content) AS content FROM chunks;
CREATE VIRTUAL TABLE fts_chunks USING fts5(
    content, content='fts_chunk_content', content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
    INSERT INTO fts_chunks(rowid, content) VALUES (new.rowid, nexa_lexical_v1(new.content));
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
    INSERT INTO fts_chunks(fts_chunks, rowid, content) VALUES ('delete', old.rowid, nexa_lexical_v1(old.content));
END;
CREATE TRIGGER chunks_au AFTER UPDATE OF content ON chunks BEGIN
    INSERT INTO fts_chunks(fts_chunks, rowid, content) VALUES ('delete', old.rowid, nexa_lexical_v1(old.content));
    INSERT INTO fts_chunks(rowid, content) VALUES (new.rowid, nexa_lexical_v1(new.content));
END;
INSERT INTO fts_chunks(fts_chunks) VALUES('rebuild');
