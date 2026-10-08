-- Preserve mutation provenance without copying file bodies. Legacy events stay
-- unknown; only newly recorded content transitions can renew repair patience.
ALTER TABLE turn_file_change_events ADD COLUMN absolute_path TEXT;
ALTER TABLE turn_file_change_events ADD COLUMN before_hash TEXT;
ALTER TABLE turn_file_change_events ADD COLUMN after_hash TEXT;
