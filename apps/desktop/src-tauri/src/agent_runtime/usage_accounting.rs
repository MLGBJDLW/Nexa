//! Project runtime-reported token snapshots into the canonical AI usage ledger.
//! Context occupancy and subscription quotas never enter this accounting path.
use nexa_core::{db::Database, error::CoreError, llm::Usage, usage_analytics::AiUsageRecordInput};
use std::{collections::BTreeMap, sync::Arc, time::Instant};

pub(super) struct RuntimeUsageAccounting {
    db: Arc<Database>,
    conversation: String,
    turn: String,
    run: Option<String>,
    provider: String,
    scope: String,
    started: Instant,
    previous: Usage,
    models: BTreeMap<String, Usage>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::{projection::Projection, tests::fixture, AgentRuntimeKind};
    use nexa_core::usage_analytics::UsageAnalyticsFilter;

    #[tokio::test]
    async fn runtime_usage_reaches_analytics_once_and_survives_interruption() {
        let (request, _rx, _, _) = fixture(AgentRuntimeKind::Codex, "model-one");
        let db = request.db.clone();
        let turn = request.prepare(false).unwrap();
        let mut output = Projection::for_turn(&turn);
        output.usage_reported = true;
        output.usage = Usage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            cache_read_tokens: Some(80),
            thinking_tokens: Some(10),
            ..Default::default()
        };
        output.publish_usage(&turn.events).await.unwrap();
        output.publish_usage(&turn.events).await.unwrap();
        // Compaction occupancy and identity-only messages must not bill tokens.
        output
            .context_snapshot(&turn.events, 15, Some(1000), vec![])
            .await
            .unwrap();
        output.observe_runtime_model("model-two");
        output.publish_usage(&turn.events).await.unwrap();
        output.usage.prompt_tokens += 30;
        output.usage.completion_tokens += 5;
        output.usage.total_tokens += 35;
        output.publish_usage(&turn.events).await.unwrap();
        output.persist_partial(&turn).await.unwrap();
        output.persist_partial(&turn).await.unwrap();
        let analytics = db
            .get_usage_analytics(&UsageAnalyticsFilter::default())
            .unwrap();
        assert_eq!(analytics.totals.total_tokens, 155);
        assert_eq!(analytics.totals.cache_read_tokens, 80);
        assert_eq!(analytics.totals.thinking_tokens, 10);
        assert_eq!(analytics.by_model.len(), 2);
        assert!(analytics
            .by_model
            .iter()
            .all(|row| row.provider_id.as_deref() == Some("openai_codex")));
        assert_eq!(
            analytics
                .by_model
                .iter()
                .find(|row| row.model_id.as_deref() == Some("model-two"))
                .unwrap()
                .total_tokens,
            35
        );
        assert!(analytics.by_model.iter().all(|row| row.success_count == 0));
    }

    #[tokio::test]
    async fn context_only_external_runtime_usage_stays_unknown() {
        let (request, _rx, _, _) = fixture(AgentRuntimeKind::Acp("gemini_cli"), "native");
        let db = request.db.clone();
        let turn = request.prepare(false).unwrap();
        let mut output = Projection::for_turn(&turn);
        output
            .context_snapshot(&turn.events, 12000, Some(128000), vec![])
            .await
            .unwrap();
        output.persist_partial(&turn).await.unwrap();
        let totals = db
            .get_usage_analytics(&UsageAnalyticsFilter::default())
            .unwrap()
            .totals;
        assert_eq!(totals.total_tokens, 0);
        assert_eq!(totals.unknown_percent, 100.0);
        assert_eq!(totals.estimated_cost_micros, None);
    }
}

impl RuntimeUsageAccounting {
    pub(super) fn new(
        db: Arc<Database>,
        conversation: String,
        turn: String,
        provider: &str,
    ) -> Result<Self, CoreError> {
        let run = db.get_agent_task_run_by_turn(&turn)?.map(|run| run.id);
        Ok(Self {
            db,
            conversation,
            turn,
            run,
            provider: provider.into(),
            scope: uuid::Uuid::new_v4().to_string(),
            started: Instant::now(),
            previous: Usage::default(),
            models: BTreeMap::new(),
        })
    }

    pub(super) fn observe(&mut self, model: &str, usage: &Usage) -> Result<(), CoreError> {
        let mut delta = Usage::default();
        macro_rules! counter {
            ($field:ident) => {{
                delta.$field = usage.$field.saturating_sub(self.previous.$field);
                self.previous.$field = self.previous.$field.max(usage.$field);
            }};
        }
        counter!(prompt_tokens);
        counter!(completion_tokens);
        counter!(total_tokens);
        macro_rules! optional_counter {
            ($field:ident) => {{
                if let Some(value) = usage.$field {
                    delta.$field = Some(value.saturating_sub(self.previous.$field.unwrap_or(0)));
                    self.previous.$field = Some(self.previous.$field.unwrap_or(0).max(value));
                }
            }};
        }
        optional_counter!(thinking_tokens);
        optional_counter!(cache_read_tokens);
        optional_counter!(cache_miss_tokens);
        optional_counter!(cache_creation_tokens);
        // Identity-only/context notifications must not create an extra request.
        if !self.models.is_empty()
            && delta.total_tokens == 0
            && delta.prompt_tokens == 0
            && delta.completion_tokens == 0
            && delta.thinking_tokens.unwrap_or(0) == 0
            && delta.cache_read_tokens.unwrap_or(0) == 0
            && delta.cache_creation_tokens.unwrap_or(0) == 0
            && delta.cache_miss_tokens.unwrap_or(0) == 0
        {
            return Ok(());
        }
        let accumulated = self.models.entry(model.into()).or_default();
        if usage.provider_raw.is_some() {
            accumulated.provider_raw = usage.provider_raw.clone();
        }
        accumulated.prompt_tokens = accumulated
            .prompt_tokens
            .saturating_add(delta.prompt_tokens);
        accumulated.completion_tokens = accumulated
            .completion_tokens
            .saturating_add(delta.completion_tokens);
        accumulated.total_tokens = accumulated.total_tokens.saturating_add(delta.total_tokens);
        macro_rules! add_optional {
            ($field:ident) => {
                if let Some(value) = delta.$field {
                    accumulated.$field =
                        Some(accumulated.$field.unwrap_or(0).saturating_add(value));
                }
            };
        }
        add_optional!(thinking_tokens);
        add_optional!(cache_read_tokens);
        add_optional!(cache_miss_tokens);
        add_optional!(cache_creation_tokens);
        let snapshot = accumulated.clone();
        self.persist(model, snapshot, "in_progress", true)
    }

    pub(super) fn finish(&self, model: &str, status: &str) -> Result<(), CoreError> {
        if self.models.is_empty() {
            return self.persist(model, Usage::default(), status, false);
        }
        for (model, usage) in &self.models {
            self.persist(model, usage.clone(), status, true)?;
        }
        Ok(())
    }

    fn persist(
        &self,
        model: &str,
        usage: Usage,
        status: &str,
        reported: bool,
    ) -> Result<(), CoreError> {
        let invocation = format!("external:{}:{}:{}", self.turn, self.scope, model);
        let raw = serde_json::json!({"runtime":self.provider,"aggregation":"external_turn","usageReported":reported,"nativeUsageScope":usage.provider_raw});
        self.db.record_ai_usage_snapshot(&AiUsageRecordInput {
            invocation_id: &invocation,
            occurred_at: None,
            provider_id: &self.provider,
            provider_type: &self.provider,
            model_id: model,
            raw_model_id: Some(model),
            modality: "language_model",
            operation_kind: "agent_main",
            conversation_id: Some(&self.conversation),
            turn_id: Some(&self.turn),
            run_id: self.run.as_deref(),
            subtask_run_id: None,
            project_id: None,
            prompt_tokens: usage.prompt_tokens.into(),
            completion_tokens: usage.completion_tokens.into(),
            thinking_tokens: usage.thinking_tokens.unwrap_or(0).into(),
            total_tokens: usage.total_tokens.into(),
            cache_read_tokens: usage.cache_read_tokens.unwrap_or(0).into(),
            cache_miss_tokens: usage.cache_miss_tokens.unwrap_or(0).into(),
            cache_creation_tokens: usage.cache_creation_tokens.unwrap_or(0).into(),
            usage_source: if reported { "provider" } else { "unknown" },
            request_status: status,
            latency_ms: Some(self.started.elapsed().as_millis().min(u64::MAX.into()) as u64),
            time_to_first_token_ms: None,
            upstream_provider_id: None,
            cache_outcome_reason: None,
            estimated_cost_micros: None,
            currency: None,
            pricing_version: None,
            provider_raw: &raw,
        })
    }
}
