CREATE TABLE project_hooks (
  id TEXT PRIMARY KEY NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  config_json TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_project_hooks_project ON project_hooks(project_id);
CREATE TABLE project_hook_runs (
  id TEXT PRIMARY KEY NOT NULL,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  hook_id TEXT NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  turn_id TEXT NOT NULL REFERENCES conversation_turns(id) ON DELETE CASCADE,
  event TEXT NOT NULL,
  revision TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('running','passed','failed','cancelled')),
  detail TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(hook_id, conversation_id, turn_id, event, revision)
);
CREATE INDEX idx_project_hook_runs_project ON project_hook_runs(project_id, created_at DESC);
