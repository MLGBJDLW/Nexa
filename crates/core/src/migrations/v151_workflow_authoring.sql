ALTER TABLE workflow_automations ADD COLUMN recipe_json TEXT;
ALTER TABLE workflow_automation_definition_revisions ADD COLUMN recipe_json TEXT;

CREATE TABLE workflow_automation_occurrence_origins_v151 (
    occurrence_id TEXT PRIMARY KEY NOT NULL REFERENCES workflow_automation_occurrences(id) ON DELETE CASCADE,
    origin TEXT NOT NULL DEFAULT 'schedule' CHECK (origin IN ('schedule', 'manual_run_now', 'folder_event')),
    resume_next_run_at TEXT,
    folder_cutoff_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK ((origin = 'folder_event' AND folder_cutoff_at IS NOT NULL) OR (origin != 'folder_event' AND folder_cutoff_at IS NULL))
);
INSERT INTO workflow_automation_occurrence_origins_v151 (occurrence_id, origin, resume_next_run_at, created_at)
SELECT occurrence_id, origin, resume_next_run_at, created_at FROM workflow_automation_occurrence_origins;
DROP TABLE workflow_automation_occurrence_origins;
ALTER TABLE workflow_automation_occurrence_origins_v151 RENAME TO workflow_automation_occurrence_origins;

-- Do not label a legacy revision-1 run with today's previously unsnapshotted definition.
INSERT INTO workflow_automation_schedule_configs (automation_id, config_json, revision)
SELECT a.id,
    '{"version":2,"timezone":"UTC","misfirePolicy":"run_latest","misfireGraceSeconds":300,"overlapPolicy":"skip","executionPolicy":{"workspacePolicy":"deny_writes","powerMode":"standard","orchestrationProfile":"balanced","collaborationMode":"direct"},"legacyNeedsReview":false}',
    1 + MAX(COALESCE((SELECT MAX(d.revision) FROM workflow_automation_definition_revisions d WHERE d.automation_id = a.id), 0),
            COALESCE((SELECT MAX(r.definition_revision) FROM workflow_automation_runs r WHERE r.automation_id = a.id), 0))
FROM workflow_automations a
WHERE a.trigger_kind IN ('manual', 'folder') AND NOT EXISTS (
    SELECT 1 FROM workflow_automation_schedule_configs c WHERE c.automation_id = a.id
);
INSERT INTO workflow_automation_definition_revisions
    (automation_id, revision, name, description, workflow_template_id, prompt, trigger_json, trigger_kind, source_scope_json, approval_policy_json, schedule_config_json)
SELECT a.id, c.revision, a.name, a.description, a.workflow_template_id, a.prompt, a.trigger_json, a.trigger_kind, a.source_scope_json, a.approval_policy_json, c.config_json
FROM workflow_automations a JOIN workflow_automation_schedule_configs c ON c.automation_id = a.id
WHERE NOT EXISTS (SELECT 1 FROM workflow_automation_definition_revisions d WHERE d.automation_id = a.id AND d.revision = c.revision);

CREATE TABLE workflow_automation_folder_cursors (
    automation_id TEXT NOT NULL REFERENCES workflow_automations(id) ON DELETE CASCADE,
    definition_revision INTEGER NOT NULL,
    observed_through TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (automation_id, definition_revision),
    FOREIGN KEY (automation_id, definition_revision) REFERENCES workflow_automation_definition_revisions(automation_id, revision) ON DELETE CASCADE
);
INSERT INTO workflow_automation_folder_cursors (automation_id, definition_revision, observed_through)
SELECT a.id, c.revision, a.last_run_at FROM workflow_automations a JOIN workflow_automation_schedule_configs c ON c.automation_id = a.id WHERE a.trigger_kind = 'folder';

CREATE TABLE workflow_automation_run_snapshots (
    run_id TEXT PRIMARY KEY NOT NULL REFERENCES workflow_automation_runs(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE workflow_automation_launch_requests (
    request_id TEXT PRIMARY KEY NOT NULL,
    automation_id TEXT NOT NULL REFERENCES workflow_automations(id) ON DELETE CASCADE,
    occurrence_id TEXT NOT NULL REFERENCES workflow_automation_occurrences(id) ON DELETE CASCADE,
    payload_digest TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
