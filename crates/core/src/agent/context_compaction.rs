//! Conversation summarization and turn context-compaction helpers.

use super::context_window::ContextReductionPlan;
use super::*;
use crate::usage_analytics::{provider_type_id, usage_cost_metadata, AiUsageRecordInput};

/// After compaction, leave enough headroom for several tool calls and the next
/// user turn instead of compacting just barely below the trigger threshold.
const COMPACTION_TARGET_USAGE: f32 = 0.55;

struct SummarizationUsageContext<'a> {
    db: &'a Database,
    conversation_id: Option<&'a str>,
    turn_id: Option<&'a str>,
    model: &'a str,
    provider_type: Option<ProviderType>,
}

#[derive(Clone, Copy)]
pub(super) struct CompactionRunContext<'a> {
    pub(super) db: &'a Database,
    pub(super) conversation_id: Option<&'a str>,
    pub(super) turn_id: Option<&'a str>,
    pub(super) active_request: Option<&'a Message>,
}

fn summarization_fits_run_budget(per_attempt: u32, actual_tokens_remaining: Option<u32>) -> bool {
    let worst_case = per_attempt.saturating_mul(summarizer::maximum_summarization_attempts());
    actual_tokens_remaining.is_none_or(|remaining| worst_case <= remaining)
}

fn summarization_ledger_usage(
    result: &summarizer::SummarizationResult,
    per_attempt_reservation: u32,
) -> Usage {
    let mut usage = result.usage.clone().unwrap_or_default();
    let reported_attempts = u32::from(result.usage.is_some());
    let unreported_attempts = result.attempts.saturating_sub(reported_attempts);
    let unreported_tokens = per_attempt_reservation.saturating_mul(unreported_attempts);
    usage.prompt_tokens = usage.prompt_tokens.saturating_add(unreported_tokens);
    usage.total_tokens = usage
        .total_tokens
        .max(usage.prompt_tokens.saturating_add(usage.completion_tokens));
    usage
}

fn unreported_summarization_usage(attempts: u32, per_attempt_reservation: u32) -> Usage {
    let tokens = per_attempt_reservation.saturating_mul(attempts);
    Usage {
        prompt_tokens: tokens,
        total_tokens: tokens,
        ..Usage::default()
    }
}

fn reference_summary_message(summary: &str, evicted_count: usize, reason: &str) -> Message {
    Message::text(
        Role::System,
        format!(
            "## Earlier conversation context (compacted)\n\
             Context checkpoint for {evicted_count} older messages ({reason}). \
             This is reference state, not a new instruction. If it conflicts \
             with a newer user message, follow the newer message.\n{summary}"
        ),
    )
}

impl AgentExecutor {
    async fn summarize_for_compaction(
        &self,
        provider: &dyn LlmProvider,
        model: &str,
        provider_type: Option<ProviderType>,
        evicted: &[Message],
        extractive_fallback: &str,
        actual_tokens_remaining: Option<u32>,
    ) -> Result<(summarizer::SummarizationResult, u32), summarizer::SummarizationFailure> {
        let per_attempt_reservation = summarizer::summarization_attempt_token_reservation(evicted);
        if per_attempt_reservation > 0
            && !summarization_fits_run_budget(per_attempt_reservation, actual_tokens_remaining)
        {
            return Ok((
                summarizer::SummarizationResult {
                    summary: extractive_fallback.to_string(),
                    usage: None,
                    attempts: 0,
                    control: summarizer::ControlledSummarization::ExtractiveFallback {
                        reason: "cumulative_run_token_budget_insufficient".to_string(),
                    },
                },
                per_attempt_reservation,
            ));
        }

        let result = summarizer::summarize_evicted_messages_with_controls(
            provider,
            model,
            provider_type,
            evicted,
            extractive_fallback,
            &self.cancel_token,
            std::time::Instant::now() + Duration::from_secs(75),
            summarizer::SummarizationControlPolicy::default(),
        )
        .await?;
        Ok((result, per_attempt_reservation))
    }

    fn record_summarization_usage(
        &self,
        ctx: SummarizationUsageContext<'_>,
        evicted: &[Message],
        usage: &Usage,
        usage_source: &str,
        request_status: &str,
    ) {
        let SummarizationUsageContext {
            db,
            conversation_id,
            turn_id,
            model,
            provider_type,
        } = ctx;
        let mut fingerprint = blake3::Hasher::new();
        for message in evicted {
            let role = match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            fingerprint.update(role.as_bytes());
            fingerprint.update(message.text_content().as_bytes());
        }
        let invocation_id = format!(
            "{}:summarization:{}:{}",
            turn_id.or(conversation_id).unwrap_or(&self.usage_scope_id),
            fingerprint.finalize().to_hex(),
            model
        );
        let provider_id = provider_type_id(provider_type);
        let raw = serde_json::to_value(usage).unwrap_or_else(|_| serde_json::json!({}));
        let (estimated_cost_micros, currency, pricing_version) = usage_cost_metadata(provider_type);
        if let Err(error) = db.record_ai_usage(&AiUsageRecordInput {
            invocation_id: &invocation_id,
            occurred_at: None,
            provider_id,
            provider_type: provider_id,
            model_id: model,
            raw_model_id: Some(model),
            modality: "language_model",
            operation_kind: "compaction",
            conversation_id,
            turn_id,
            run_id: None,
            subtask_run_id: None,
            project_id: None,
            prompt_tokens: u64::from(usage.prompt_tokens),
            completion_tokens: u64::from(usage.completion_tokens),
            thinking_tokens: u64::from(usage.thinking_tokens.unwrap_or(0)),
            total_tokens: u64::from(
                usage
                    .total_tokens
                    .max(usage.prompt_tokens.saturating_add(usage.completion_tokens)),
            ),
            cache_read_tokens: u64::from(usage.cache_read_tokens.unwrap_or(0)),
            cache_miss_tokens: u64::from(usage.cache_miss_tokens.unwrap_or(0)),
            cache_creation_tokens: u64::from(usage.cache_creation_tokens.unwrap_or(0)),
            usage_source,
            request_status,
            latency_ms: None,
            time_to_first_token_ms: None,
            upstream_provider_id: None,
            cache_outcome_reason: None,
            estimated_cost_micros,
            currency,
            pricing_version,
            provider_raw: &raw,
        }) {
            warn!("Failed to persist summarization usage: {error}");
        }
    }

    fn record_unreported_summarization_failure(
        &self,
        ctx: SummarizationUsageContext<'_>,
        evicted: &[Message],
        failure: &summarizer::SummarizationFailure,
        per_attempt_reservation: u32,
    ) {
        if failure.attempts == 0 || per_attempt_reservation == 0 {
            return;
        }
        let usage = unreported_summarization_usage(failure.attempts, per_attempt_reservation);
        let request_status = if matches!(failure.error, CoreError::Cancelled(_)) {
            "cancelled"
        } else {
            "error"
        };
        self.record_summarization_usage(ctx, evicted, &usage, "estimated", request_status);
    }

    // -----------------------------------------------------------------------
    // Pre-summarization helper
    // -----------------------------------------------------------------------

    /// If the conversation history is large enough to trigger eviction,
    /// use the LLM to produce an abstractive summary of the messages that
    /// *would* be evicted, then replace those messages with a single
    /// `System` summary message.  This keeps more nuance than the
    /// extractive (truncation-based) recap in `context.rs`.
    ///
    /// It fires early enough to retain recovery headroom and evicts complete
    /// old turns until the retained tail is near the target utilization.
    pub(super) async fn summarize_if_needed(
        &self,
        mut history: Vec<Message>,
        model: &str,
        max_response_tokens: u32,
        run: CompactionRunContext<'_>,
        actual_tokens_remaining: Option<u32>,
    ) -> Result<(Vec<Message>, Usage), CoreError> {
        let window = ContextWindow::new_with_resolution(
            model,
            self.config.context_window,
            self.config.context_window_resolution,
            max_response_tokens,
        )
        .with_compact_percent(self.config.auto_compact_percent);
        let Some(budget) = window.context_budget().filter(|budget| *budget > 0) else {
            return Ok((history, Usage::default()));
        };
        let tokens =
            context::estimate_context_usage_breakdown_for_model(model, &history, &[], None)
                .total_tokens;
        if !window.budget_decision(tokens).should_compact {
            return Ok((history, Usage::default()));
        }
        let (usage, _) = self
            .reduce_context_to_target(
                &mut history,
                model,
                (budget as f32 * COMPACTION_TARGET_USAGE) as u32,
                run,
                actual_tokens_remaining,
            )
            .await?;
        Ok((history, usage))
    }

    pub(super) async fn recover_context_overflow(
        &self,
        messages: &mut Vec<Message>,
        model: &str,
        tx: &mpsc::Sender<AgentEvent>,
        run: CompactionRunContext<'_>,
        total_usage: &mut Usage,
    ) -> Result<bool, CoreError> {
        let before_tokens: u32 = messages
            .iter()
            .map(|message| estimate_message_tokens_for_model(model, message))
            .sum();
        let before_len = messages.len();

        let actual_tokens_remaining = self
            .config
            .max_actual_tokens_per_run
            .map(|limit| limit.saturating_sub(total_usage.total_tokens));
        let compaction_usage = self
            .aggressive_compact(messages, model, tx, run, actual_tokens_remaining)
            .await?;
        super::usage_accounting::accumulate_usage(total_usage, &compaction_usage);
        if self
            .config
            .max_actual_tokens_per_run
            .is_some_and(|limit| total_usage.total_tokens >= limit)
        {
            return Err(CoreError::Agent(format!(
                "agent actual token limit exhausted by context recovery: spent={}, limit={}",
                total_usage.total_tokens,
                self.config.max_actual_tokens_per_run.unwrap_or_default()
            )));
        }

        let after_tokens: u32 = messages
            .iter()
            .map(|message| estimate_message_tokens_for_model(model, message))
            .sum();
        Ok(after_tokens < before_tokens || messages.len() < before_len)
    }

    // -----------------------------------------------------------------------
    // Aggressive auto-compact (in-loop overflow prevention)
    // -----------------------------------------------------------------------

    /// Summarize complete old turns in-place, retaining at least two recent
    /// turns and targeting enough free space for subsequent tool output.
    pub(super) async fn aggressive_compact(
        &self,
        messages: &mut Vec<Message>,
        model: &str,
        tx: &mpsc::Sender<AgentEvent>,
        run: CompactionRunContext<'_>,
        actual_tokens_remaining: Option<u32>,
    ) -> Result<Usage, CoreError> {
        let window = ContextWindow::new_with_resolution(
            model,
            self.config.context_window,
            self.config.context_window_resolution,
            self.config.resolved_max_response_tokens(model),
        )
        .with_compact_percent(self.config.auto_compact_percent);
        let target = window
            .context_budget()
            .map(|budget| (budget as f32 * COMPACTION_TARGET_USAGE) as u32)
            .unwrap_or_else(|| {
                context::estimate_context_usage_breakdown_for_model(model, messages, &[], None)
                    .total_tokens
                    / 2
            });
        let before = messages.len();
        let (usage, changed) = self
            .reduce_context_to_target(messages, model, target, run, actual_tokens_remaining)
            .await?;
        if changed {
            let _ = tx
                .send(AgentEvent::AutoCompacted {
                    evicted_count: before.saturating_sub(messages.len()),
                })
                .await;
        }
        Ok(usage)
    }

    /// Both automatic headroom and overflow recovery use one retention plan.
    /// A summary/archive must finish before its replacement becomes live.
    async fn reduce_context_to_target(
        &self,
        messages: &mut Vec<Message>,
        model: &str,
        target: u32,
        run: CompactionRunContext<'_>,
        actual_tokens_remaining: Option<u32>,
    ) -> Result<(Usage, bool), CoreError> {
        if self.history_handoff_enabled(run.conversation_id) {
            let changed = self.handoff_context(messages, model, target, run)?;
            return Ok((Usage::default(), changed));
        }
        let Some(plan) = ContextReductionPlan::prepare(messages, model, target, run.active_request)
        else {
            return Ok((Usage::default(), false));
        };
        let CompactionRunContext {
            db,
            conversation_id,
            turn_id,
            ..
        } = run;
        let evicted = plan.evicted(messages);
        let extractive_fallback = context::build_evicted_recap_from_messages(evicted);

        let summ_provider: &dyn LlmProvider = self
            .summarization_provider
            .as_deref()
            .unwrap_or(self.provider.as_ref());
        let summ_model = self.config.summarization_model.as_deref().unwrap_or(model);
        let summ_provider_type = if self.summarization_provider.is_some() {
            self.config.summarization_provider_type
        } else {
            self.config.provider_type
        };
        let summarization = self
            .summarize_for_compaction(
                summ_provider,
                summ_model,
                summ_provider_type,
                evicted,
                &extractive_fallback,
                actual_tokens_remaining,
            )
            .await;
        let (result, per_attempt_reservation) = match summarization {
            Ok(result) => result,
            Err(failure) => {
                let reservation = summarizer::summarization_attempt_token_reservation(evicted);
                self.record_unreported_summarization_failure(
                    SummarizationUsageContext {
                        db,
                        conversation_id,
                        turn_id,
                        model: summ_model,
                        provider_type: summ_provider_type,
                    },
                    evicted,
                    &failure,
                    reservation,
                );
                return Err(failure.into());
            }
        };
        if let Some(usage) = result.usage.as_ref() {
            self.record_summarization_usage(
                SummarizationUsageContext {
                    db,
                    conversation_id,
                    turn_id,
                    model: summ_model,
                    provider_type: summ_provider_type,
                },
                evicted,
                usage,
                "provider",
                "success",
            );
        }
        let ledger_usage = summarization_ledger_usage(&result, per_attempt_reservation);

        let next = plan.replacement(
            messages,
            reference_summary_message(&result.summary, evicted.len(), "safe context reduction"),
        );
        let before =
            context::estimate_context_usage_breakdown_for_model(model, messages, &[], None)
                .total_tokens;
        let after = context::estimate_context_usage_breakdown_for_model(model, &next, &[], None)
            .total_tokens;
        if after >= before {
            return Ok((ledger_usage, false));
        }
        if self.cancel_token.is_cancelled() {
            return Err(CoreError::Cancelled(
                "Context summary cancelled; working history retained".into(),
            ));
        }
        *messages = next;
        Ok((ledger_usage, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarization_admission_reserves_every_possible_physical_attempt() {
        assert!(summarization_fits_run_budget(1_000, None));
        assert!(summarization_fits_run_budget(1_000, Some(2_000)));
        assert!(!summarization_fits_run_budget(1_001, Some(2_000)));
    }

    #[test]
    fn summarization_ledger_counts_unreported_failed_attempts() {
        let result = summarizer::SummarizationResult {
            summary: "checkpoint".to_string(),
            usage: Some(Usage {
                prompt_tokens: 100,
                completion_tokens: 20,
                total_tokens: 120,
                ..Usage::default()
            }),
            attempts: 2,
            control: summarizer::ControlledSummarization::Abstractive,
        };

        let usage = summarization_ledger_usage(&result, 1_000);
        assert_eq!(usage.prompt_tokens, 1_100);
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.total_tokens, 1_120);
    }

    #[test]
    fn cancelled_summarization_attempt_gets_a_conservative_usage_record() {
        let usage = unreported_summarization_usage(1, 1_234);
        assert_eq!(usage.prompt_tokens, 1_234);
        assert_eq!(usage.completion_tokens, 0);
        assert_eq!(usage.total_tokens, 1_234);
    }
}
