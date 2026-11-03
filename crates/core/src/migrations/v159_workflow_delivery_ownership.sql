-- Only newly allocated standalone chats have proven scheduler ownership.
-- Historical bindings may point at user-authored chats and must remain read-only.
ALTER TABLE workflow_automation_delivery_targets
    ADD COLUMN owns_conversation INTEGER NOT NULL DEFAULT 0 CHECK (owns_conversation IN (0, 1));
