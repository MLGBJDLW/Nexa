-- Bundled content remains immutable; only the user's enablement choice is stored.
CREATE TABLE builtin_skill_state (
    skill_id TEXT PRIMARY KEY NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
