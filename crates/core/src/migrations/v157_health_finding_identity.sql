-- Repeated scans used random IDs and accumulated the same finding indefinitely.
-- Keep an existing resolved decision in preference to an unresolved duplicate.
DELETE FROM health_checks WHERE rowid IN (
    SELECT finding_row FROM (
        SELECT rowid AS finding_row,
               ROW_NUMBER() OVER (
                   PARTITION BY check_type, COALESCE(target_doc_id, ''),
                                COALESCE(target_entity_id, ''), description
                   ORDER BY resolved DESC, checked_at DESC, id
               ) AS occurrence
        FROM health_checks
    ) WHERE occurrence > 1
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_health_finding_identity
ON health_checks(check_type, COALESCE(target_doc_id, ''), COALESCE(target_entity_id, ''), description);
