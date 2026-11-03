//! Conversation delivery is owned by the existing durable scheduler, not the UI.
use super::*;
use crate::conversation::AgentConfig;

impl Database {
    pub fn workflow_conversation_is_busy(&self, conversation_id: &str) -> Result<bool, CoreError> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_task_runs WHERE conversation_id=?1 AND status IN ('queued','running','waiting_approval','awaiting_user_input','cancelling','paused','resuming'))",
            [conversation_id], |row| row.get(0),
        )?)
    }

    pub fn validate_workflow_conversation(
        &self,
        automation: &WorkflowAutomation,
        conversation_id: &str,
    ) -> Result<(), CoreError> {
        let conversation = self.get_conversation(conversation_id)?;
        if conversation.project_id != automation.schedule_config.execution_policy.project_id {
            return Err(CoreError::InvalidInput(
                "Scheduled conversation changed project; review the task before running it".into(),
            ));
        }
        let mut actual = self.get_effective_conversation_source_scope(conversation_id)?;
        let mut expected = automation.source_scope.clone();
        actual.sort();
        actual.dedup();
        expected.sort();
        expected.dedup();
        if expected.iter().any(|source| !actual.contains(source)) {
            return Err(CoreError::InvalidInput(
                "Scheduled conversation sources changed; review the task before running it".into(),
            ));
        }
        Ok(())
    }

    /// Reserve a standalone schedule's conversation before starting its first
    /// agent turn. Concurrent starts and failed launches reuse the same target.
    pub fn workflow_delivery_conversation(
        &self,
        automation: &WorkflowAutomation,
        config: &AgentConfig,
    ) -> Result<String, CoreError> {
        if let Some(id) = automation
            .schedule_config
            .execution_policy
            .conversation_id
            .as_deref()
            .or(automation.conversation_id.as_deref())
        {
            self.validate_workflow_conversation(automation, id)?;
            return Ok(id.to_string());
        }
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved: Option<String> = tx.query_row(
            "SELECT conversation_id FROM workflow_automation_delivery_targets WHERE automation_id=?1",
            [&automation.id], |row| row.get(0),
        ).optional()?;
        let id = if let Some(id) = saved {
            id
        } else {
            let id = new_id();
            tx.execute(
                "INSERT INTO conversations (id, title, provider, model, system_prompt, system_prompt_origin, project_id) VALUES (?1, ?2, ?3, ?4, '', 'none', ?5)",
                rusqlite::params![&id, &automation.name, &config.provider, &config.model, &automation.schedule_config.execution_policy.project_id],
            )?;
            for source in &automation.source_scope {
                tx.execute(
                    "INSERT INTO conversation_sources (conversation_id,source_id) VALUES (?1,?2)",
                    rusqlite::params![&id, source],
                )?;
            }
            tx.execute("INSERT INTO workflow_automation_delivery_targets (automation_id,conversation_id,owns_conversation) VALUES (?1,?2,1)", rusqlite::params![&automation.id,&id])?;
            id
        };
        tx.commit()?;
        drop(conn);
        self.validate_workflow_conversation(automation, &id)?;
        Ok(id)
    }
}

/// An explicit definition edit may update a task-created chat, but never an
/// originating user chat or a target whose source/project boundary has drifted.
pub(super) fn reconcile_owned_delivery_scope(
    tx: &rusqlite::Transaction<'_>,
    automation_id: &str,
    source_scope: &[String],
    config: &WorkflowAutomationScheduleConfig,
) -> Result<(), CoreError> {
    if config.execution_policy.conversation_id.is_some() {
        return Ok(());
    }
    let target: Option<String> = tx.query_row(
        "SELECT conversation_id FROM workflow_automation_delivery_targets WHERE automation_id=?1 AND owns_conversation=1",
        [automation_id], |row| row.get(0),
    ).optional()?;
    let Some(target) = target else { return Ok(()) };
    let (previous_scope, previous_config): (String, Option<String>) = tx.query_row(
        "SELECT a.source_scope_json,c.config_json FROM workflow_automations a LEFT JOIN workflow_automation_schedule_configs c ON c.automation_id=a.id WHERE a.id=?1",
        [automation_id], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let previous_config: WorkflowAutomationScheduleConfig = previous_config
        .map(|raw| serde_json::from_str(&raw))
        .transpose()?
        .unwrap_or_default();
    if previous_config.execution_policy.conversation_id.is_some() {
        return Ok(());
    }
    let previous_scope: BTreeSet<String> =
        normalize_string_list(&serde_json::from_str::<Vec<String>>(&previous_scope)?)
            .into_iter()
            .collect();
    let next_scope: BTreeSet<String> = source_scope.iter().cloned().collect();
    if previous_scope == next_scope
        && previous_config.execution_policy.project_id == config.execution_policy.project_id
    {
        return Ok(());
    }
    let project: Option<Option<String>> = tx
        .query_row(
            "SELECT project_id FROM conversations WHERE id=?1",
            [&target],
            |row| row.get(0),
        )
        .optional()?;
    let mut stmt =
        tx.prepare("SELECT source_id FROM conversation_sources WHERE conversation_id=?1")?;
    let linked = stmt
        .query_map([&target], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if project != Some(previous_config.execution_policy.project_id)
        || normalize_string_list(&linked)
            .into_iter()
            .collect::<BTreeSet<_>>()
            != previous_scope
    {
        return Err(CoreError::Conflict(
            "Scheduled conversation sources or project changed; restore its previous task scope before editing this task".into(),
        ));
    }
    tx.execute(
        "UPDATE conversations SET project_id=?2 WHERE id=?1",
        rusqlite::params![&target, &config.execution_policy.project_id],
    )?;
    tx.execute(
        "DELETE FROM conversation_sources WHERE conversation_id=?1",
        [&target],
    )?;
    for source in source_scope {
        tx.execute(
            "INSERT INTO conversation_sources(conversation_id,source_id) VALUES(?1,?2)",
            rusqlite::params![&target, source],
        )?;
    }
    Ok(())
}
