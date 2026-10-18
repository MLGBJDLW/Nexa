//! Project runtime-reported token snapshots into the canonical AI usage ledger.
//! Context occupancy and subscription quotas never enter this accounting path.
use nexa_core::{db::Database, error::CoreError, llm::Usage, usage_analytics::AiUsageRecordInput};
use std::{collections::BTreeMap, sync::Arc, time::Instant};

pub(super) struct RuntimeUsageAccounting {
    db: Arc<Database>,
    conversation: String,
    turn: String,
    run: Option<String>,
    subtask: Option<String>,
    provider: String,
    scope: String,
    started: Instant,
    previous: Usage,
    models: BTreeMap<String, Usage>,
    cancellation: nexa_core::agent::CancellationToken,
    finished: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::{projection::Projection, tests::fixture, AgentRuntimeKind};
    use nexa_core::usage_analytics::UsageAnalyticsFilter;

    #[tokio::test]
    async fn native_child_usage_survives_before_terminal_worker_settlement() {
        let (parent, _parent_rx, _, _) = fixture(AgentRuntimeKind::Copilot, "native");
        let db = parent.db.clone();
        let parent_turn = db.get_conversation_turn(&parent.turn_id).unwrap();
        let run = db
            .create_agent_task_run(
                &parent.conversation_id,
                &parent.turn_id,
                &parent_turn.user_message_id,
                "Parent",
                Some("github_copilot"),
                Some("native"),
            )
            .unwrap();
        let subtask = db
            .create_agent_subtask_run(
                &run.id,
                "child",
                "researcher",
                Some(&serde_json::json!({"kind":"subagent_run"})),
                None,
            )
            .unwrap();
        db.mark_agent_subtask_run_started(&subtask.id, "running")
            .unwrap();
        let (mut child, _child_rx, _, _) = super::super::tests::fixture_with_database(
            AgentRuntimeKind::Copilot,
            "native",
            db.clone(),
        );
        child.usage_owner = Some((run.id.clone(), subtask.id.clone()));
        let turn = child.prepare(false).unwrap();
        let mut output = Projection::for_turn(&turn);
        output.usage_reported = true;
        output.usage = Usage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            ..Default::default()
        };
        output.publish_usage(&turn.events).await.unwrap();
        drop(output);
        drop(turn);
        assert!(db
            .get_agent_subtask_run(&subtask.id)
            .unwrap()
            .output
            .is_none());
        assert_eq!(
            db.delegated_usage_for_run(&run.id).unwrap()[&subtask.id].0,
            120
        );
        let scheduler = crate::delegation_scheduler::DelegationScheduler::new(
            &nexa_core::agent::AgentConfig::default(),
        );
        scheduler
            .bind_resume_accounting(db.as_ref().clone(), &run.id, &parent.conversation_id)
            .unwrap();
        assert_eq!(scheduler.snapshot().await.tokens_spent, 120);
        assert_eq!(
            db.get_usage_analytics(&UsageAnalyticsFilter::default())
                .unwrap()
                .totals
                .total_tokens,
            120
        );
    }

    #[tokio::test]
    async fn privacy_revocation_settles_usage_when_native_future_is_dropped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usage.db");
        let database = Arc::new(Database::new(&path).unwrap());
        let reader = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        for kind in [
            AgentRuntimeKind::Codex,
            AgentRuntimeKind::Copilot,
            AgentRuntimeKind::Acp("hermes".into()),
        ] {
            for settled in [false, true] {
                let (request, _rx, _, _) = super::super::tests::fixture_with_database(
                    kind.clone(),
                    "native",
                    database.clone(),
                );
                let turn_id = request.turn_id.clone();
                let lease = database.privacy_lease(&request.cancellation).unwrap();
                let db = database.clone();
                // The same biased revocation boundary as agent_runtime::run:
                // the suspended future is dropped without async cleanup.
                tokio::select! {
                    biased;
                    _ = lease.cancelled() => {},
                    _ = async move {
                        let turn = request.prepare(false).unwrap();
                        let mut output = Projection::for_turn(&turn);
                        output.usage_reported = true;
                        output.usage = Usage { prompt_tokens: 10, completion_tokens: 2, total_tokens: 12, ..Default::default() };
                        output.publish_usage(&turn.events).await.unwrap();
                        if settled {
                            output.persist_partial(&turn, &CoreError::StreamIncomplete("lost connection".into())).await.unwrap();
                        }
                        let mut policy = db.load_privacy_config().unwrap();
                        policy.redact_patterns.push(nexa_core::privacy::RedactRule {
                            name: uuid::Uuid::new_v4().to_string(), pattern: "fixture-private-value".into(), replacement: "[PRIVATE]".into(),
                        });
                        db.save_privacy_config(&policy).unwrap();
                        std::future::pending::<()>().await;
                        drop((turn, output));
                    } => unreachable!(),
                }
                let (status, tokens): (String, u64) = reader.query_row(
                    "SELECT request_status, total_tokens FROM ai_usage_records WHERE turn_id=?1",
                    [&turn_id], |row| Ok((row.get(0)?, row.get(1)?)),
                ).unwrap();
                assert_eq!(status, if settled { "error" } else { "cancelled" });
                assert_eq!(tokens, 12);
            }
        }
    }

    #[tokio::test]
    async fn cleanup_cancellation_does_not_change_the_recorded_turn_outcome() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("usage.db");
        let database = Arc::new(Database::new(&path).unwrap());
        let reader = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        for kind in [
            AgentRuntimeKind::Codex,
            AgentRuntimeKind::Copilot,
            AgentRuntimeKind::Acp("hermes".into()),
        ] {
            for user_cancelled in [false, true] {
                let (request, _rx, _, _) = super::super::tests::fixture_with_database(
                    kind.clone(),
                    "native",
                    database.clone(),
                );
                let turn_id = request.turn_id.clone();
                let turn = request.prepare(false).unwrap();
                let mut output = Projection::for_turn(&turn);
                output.usage_reported = true;
                output.usage = Usage {
                    prompt_tokens: 10,
                    completion_tokens: 2,
                    total_tokens: 12,
                    ..Default::default()
                };
                let cause = if user_cancelled {
                    CoreError::Cancelled("Stopped by user".into())
                } else {
                    CoreError::StreamIncomplete("Native connection closed".into())
                };
                // All runtimes may stop child work before saving partial output.
                turn.cancellation.cancel();
                output.persist_partial(&turn, &cause).await.unwrap();
                let (status, tokens): (String, u64) = reader
                    .query_row(
                        "SELECT request_status, total_tokens FROM ai_usage_records WHERE turn_id=?1",
                        [&turn_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(status, if user_cancelled { "cancelled" } else { "error" });
                assert_eq!(tokens, 12);
            }
        }
    }

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
        output
            .persist_partial(
                &turn,
                &nexa_core::error::CoreError::StreamIncomplete("fixture interruption".into()),
            )
            .await
            .unwrap();
        output
            .persist_partial(
                &turn,
                &nexa_core::error::CoreError::StreamIncomplete("fixture interruption".into()),
            )
            .await
            .unwrap();
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
        let (request, _rx, _, _) = fixture(AgentRuntimeKind::Acp("gemini_cli".into()), "native");
        let db = request.db.clone();
        let turn = request.prepare(false).unwrap();
        let mut output = Projection::for_turn(&turn);
        output
            .context_snapshot(&turn.events, 12000, Some(128000), vec![])
            .await
            .unwrap();
        output
            .persist_partial(
                &turn,
                &nexa_core::error::CoreError::StreamIncomplete("fixture interruption".into()),
            )
            .await
            .unwrap();
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
        cancellation: nexa_core::agent::CancellationToken,
        usage_owner: Option<(String, String)>,
    ) -> Result<Self, CoreError> {
        let (run, subtask) = match usage_owner {
            Some((run, subtask)) => {
                if db.get_agent_subtask_run(&subtask)?.parent_run_id != run {
                    return Err(CoreError::InvalidInput(
                        "Native worker usage owner mismatch".into(),
                    ));
                }
                (Some(run), Some(subtask))
            }
            None => (
                db.get_agent_task_run_by_turn(&turn)?.map(|run| run.id),
                None,
            ),
        };
        Ok(Self {
            db,
            conversation,
            turn,
            run,
            subtask,
            provider: provider.into(),
            scope: uuid::Uuid::new_v4().to_string(),
            started: Instant::now(),
            previous: Usage::default(),
            models: BTreeMap::new(),
            cancellation,
            finished: false,
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

    pub(super) fn finish(&mut self, model: &str, status: &str) -> Result<(), CoreError> {
        if self.models.is_empty() {
            self.persist(model, Usage::default(), status, false)?;
        } else {
            for (model, usage) in &self.models {
                self.persist(model, usage.clone(), status, true)?;
            }
        }
        self.finished = true;
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
            operation_kind: if self.subtask.is_some() {
                "subagent"
            } else {
                "agent_main"
            },
            conversation_id: Some(&self.conversation),
            turn_id: Some(&self.turn),
            run_id: self.run.as_deref(),
            subtask_run_id: self.subtask.as_deref(),
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

impl Drop for RuntimeUsageAccounting {
    fn drop(&mut self) {
        // Privacy revocation or task abortion may drop the native future before
        // its async cleanup runs. Settle only observed usage; normal completion
        // keeps its actual outcome even if child-process cleanup cancels a token.
        if !self.finished && !self.models.is_empty() {
            let status = if self.cancellation.is_cancelled() {
                "cancelled"
            } else {
                "error"
            };
            if let Err(error) = self.finish("", status) {
                log::warn!("Failed to settle dropped external runtime usage: {error}");
            }
        }
    }
}
