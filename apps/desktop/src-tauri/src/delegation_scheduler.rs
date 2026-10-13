use std::sync::{Arc, Mutex};
use std::time::Duration;

use nexa_core::agent::AgentConfig;
use nexa_core::agent::CancellationToken;
use nexa_core::error::CoreError;
use nexa_core::llm::Usage;
use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const DEFAULT_CONNECT_DEADLINE_MS: u64 = 15_000;
const DEFAULT_FIRST_TOKEN_DEADLINE_MS: u64 = 45_000;
const LONG_REASONER_CONNECT_DEADLINE_MS: u64 = 90_000;
const LONG_REASONER_FIRST_TOKEN_DEADLINE_MS: u64 = 150_000;

/// Task deadlines are optional; absence never becomes an arbitrary duration.
pub(crate) async fn with_optional_timeout<F: std::future::Future>(
    deadline_ms: Option<u64>,
    future: F,
) -> Result<F::Output, tokio::time::error::Elapsed> {
    match deadline_ms {
        Some(ms) => tokio::time::timeout(Duration::from_millis(ms), future).await,
        None => Ok(future.await),
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DelegationLimitPolicy {
    Auto,
    Explicit(u64),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DelegationLimitsV2 {
    pub input_context_policy: DelegationLimitPolicy,
    pub handoff_context_tokens_per_worker: Option<u64>,
    pub max_output_tokens_per_worker: DelegationLimitPolicy,
    pub max_actual_tokens_per_worker: Option<u64>,
    pub total_actual_tokens_soft_limit: Option<u64>,
    pub total_cost_soft_limit_micros: Option<u64>,
    pub cost_accounting_available: bool,
    pub max_parallel: u32,
    pub max_calls_per_turn: Option<u32>,
    pub exploration_lane_slots: u32,
    pub verification_lane_slots: u32,
    pub judge_lane_slots: u32,
    pub queue_deadline_ms: Option<u64>,
    pub connect_deadline_ms: u64,
    pub first_token_deadline_ms: u64,
    pub run_deadline_ms: Option<u64>,
}

impl DelegationLimitsV2 {
    pub(crate) fn resolve(config: &AgentConfig) -> Self {
        let configured = config.delegation_limits_v2.as_ref();
        let catalog_limits = config.provider_type.and_then(|provider| {
            config.model.as_deref().and_then(|model| {
                nexa_core::provider_catalog::model_limits_from_catalog(provider, model)
            })
        });
        let catalog_reasoning = config.provider_type.and_then(|provider| {
            config.model.as_deref().and_then(|model| {
                nexa_core::provider_catalog::model_capabilities_from_catalog(provider, model)
                    .and_then(|capabilities| capabilities.reasoning)
            })
        });
        let inferred_context_tokens = config
            .context_window_resolution
            .and_then(|resolved| resolved.capacity_tokens)
            .or(config.context_window)
            .map(u64::from);
        let capability_needs_long_prefill = catalog_limits.as_ref().is_some_and(|limits| {
            limits
                .context_tokens
                .is_some_and(|tokens| tokens >= 500_000)
                || limits
                    .max_output_tokens
                    .is_some_and(|tokens| tokens >= 65_536)
        }) || inferred_context_tokens
            .is_some_and(|tokens| tokens >= 500_000)
            || config.max_tokens.is_some_and(|tokens| tokens >= 65_536)
            || catalog_reasoning.as_ref().is_some_and(|reasoning| {
                matches!(reasoning.default_effort.as_deref(), Some("xhigh" | "max"))
                    || reasoning
                        .thinking_budget
                        .as_ref()
                        .and_then(|budget| budget.max_tokens)
                        .is_some_and(|tokens| tokens >= 65_536)
            });
        let long_prefill_profile = capability_needs_long_prefill
            || matches!(
                config.provider_type,
                Some(nexa_core::llm::ProviderType::Ollama | nexa_core::llm::ProviderType::LmStudio)
            );
        let default_connect_deadline_ms = if long_prefill_profile {
            LONG_REASONER_CONNECT_DEADLINE_MS
        } else {
            DEFAULT_CONNECT_DEADLINE_MS
        };
        let default_first_token_deadline_ms = if long_prefill_profile {
            LONG_REASONER_FIRST_TOKEN_DEADLINE_MS
        } else {
            DEFAULT_FIRST_TOKEN_DEADLINE_MS
        };
        let max_parallel = match configured {
            Some(limits) => limits.max_parallel,
            None => config.subagent_max_parallel,
        }
        .unwrap_or(4)
        .clamp(1, 12);
        let max_calls_per_turn = match configured {
            Some(limits) => limits.max_calls_per_turn,
            None => config.subagent_max_calls_per_turn,
        };
        let dedicated_lanes = config
            .subagent_verification_reserve_percent
            .unwrap_or_default()
            > 0
            && max_parallel >= 3
            && max_calls_per_turn.is_none_or(|limit| limit >= 3);
        let (exploration_lane_slots, verification_lane_slots, judge_lane_slots) = if dedicated_lanes
        {
            (max_parallel - 2, 1, 1)
        } else {
            (max_parallel, 0, 0)
        };
        let run_deadline_ms = configured.and_then(|limits| limits.run_deadline_ms);
        Self {
            input_context_policy: configured
                .and_then(|limits| limits.input_context_limit)
                .or_else(|| {
                    configured
                        .is_none()
                        .then_some(config.context_window.map(u64::from))
                        .flatten()
                })
                .map(DelegationLimitPolicy::Explicit)
                .unwrap_or(DelegationLimitPolicy::Auto),
            handoff_context_tokens_per_worker: configured
                .and_then(|limits| limits.handoff_context_tokens_per_worker),
            max_output_tokens_per_worker: configured
                .and_then(|limits| {
                    limits
                        .max_output_tokens_per_step
                        .or(limits.max_output_tokens_per_worker)
                })
                .or_else(|| {
                    configured
                        .is_none()
                        .then_some(config.max_tokens.map(u64::from))
                        .flatten()
                })
                .map(DelegationLimitPolicy::Explicit)
                .unwrap_or(DelegationLimitPolicy::Auto),
            max_actual_tokens_per_worker: configured
                .and_then(|limits| limits.max_actual_tokens_per_worker),
            total_actual_tokens_soft_limit: match configured {
                Some(limits) => limits.total_actual_tokens_soft_limit,
                None => config.subagent_token_budget.map(u64::from),
            },
            total_cost_soft_limit_micros: configured
                .and_then(|limits| limits.total_cost_soft_limit_micros),
            cost_accounting_available: nexa_core::usage_analytics::usage_cost_metadata(
                config.provider_type,
            )
            .0
            .is_some(),
            max_parallel,
            max_calls_per_turn,
            exploration_lane_slots,
            verification_lane_slots,
            judge_lane_slots,
            queue_deadline_ms: configured
                .and_then(|limits| limits.queue_deadline_ms)
                .map(|limit| limit.min(run_deadline_ms.unwrap_or(u64::MAX))),
            connect_deadline_ms: configured
                .and_then(|limits| limits.connect_deadline_ms)
                .unwrap_or(default_connect_deadline_ms)
                .clamp(100, run_deadline_ms.unwrap_or(u64::MAX).max(100)),
            first_token_deadline_ms: configured
                .and_then(|limits| limits.first_token_deadline_ms)
                .unwrap_or(default_first_token_deadline_ms)
                .clamp(100, run_deadline_ms.unwrap_or(u64::MAX).max(100)),
            run_deadline_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DelegationLane {
    Exploration,
    Verification,
    Judge,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BudgetSnapshot {
    pub max_parallel: u32,
    pub max_calls_per_turn: Option<u32>,
    pub calls_started: u32,
    pub remaining_calls: Option<u32>,
    pub token_budget: u32,
    pub tokens_spent: u32,
    pub tokens_reserved: u32,
    pub remaining_tokens: u32,
    pub verification_reserve_tokens: u32,
    pub cost_soft_limit_micros: Option<u64>,
    pub cost_spent_micros: u64,
    pub cost_accounting_available: bool,
    pub exploration_lane_slots: u32,
    pub verification_lane_slots: u32,
    pub judge_lane_slots: u32,
}

#[derive(Debug)]
struct DelegationBudgetState {
    restore_error: Option<String>,
    limits: DelegationLimitsV2,
    calls_started: u32,
    verification_calls_started: u32,
    judge_calls_started: u32,
    tokens_spent: u32,
    tokens_reserved: u32,
    cost_spent_micros: u64,
}

#[derive(Clone)]
pub(crate) struct DelegationScheduler {
    resume: Arc<Mutex<Option<ResumeAccounting>>>,
    shared_lane: Option<Arc<Semaphore>>,
    exploration_lane: Arc<Semaphore>,
    verification_lane: Option<Arc<Semaphore>>,
    judge_lane: Option<Arc<Semaphore>>,
    state: Arc<Mutex<DelegationBudgetState>>,
}

struct ResumeAccounting {
    db: nexa_core::db::Database,
    run_id: String,
    ids: std::collections::HashSet<String>,
    calls: u32,
    verification_calls: u32,
    judge_calls: u32,
    tokens: u32,
    cost: u64,
    reservations: u32,
}

/// A started provider call keeps its call credit; preflight/setup failures
/// refund it. All paths release the estimate, including unwind/early return.
#[derive(Debug)]
pub(crate) struct WorkerBudgetReservation {
    state: Arc<Mutex<DelegationBudgetState>>,
    reserved_tokens: u32,
    is_verification: bool,
    started: bool,
    settled: bool,
}

impl WorkerBudgetReservation {
    pub(crate) fn mark_started(&mut self) {
        self.started = true;
    }

    pub(crate) fn settle(&mut self, usage: &Usage, cost_micros: Option<u64>) {
        if self.settled {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.tokens_reserved = state.tokens_reserved.saturating_sub(self.reserved_tokens);
        state.tokens_spent = state.tokens_spent.saturating_add(usage.total_tokens);
        state.cost_spent_micros = state
            .cost_spent_micros
            .saturating_add(cost_micros.unwrap_or(0));
        self.settled = true;
    }
}

impl Drop for WorkerBudgetReservation {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.tokens_reserved = state.tokens_reserved.saturating_sub(self.reserved_tokens);
        if !self.started {
            state.calls_started = state.calls_started.saturating_sub(1);
            if self.is_verification {
                state.verification_calls_started =
                    state.verification_calls_started.saturating_sub(1);
            }
        }
    }
}

impl DelegationScheduler {
    pub(crate) fn bind_resume_accounting(
        &self,
        db: nexa_core::db::Database,
        run_id: &str,
        conversation: &str,
    ) -> Result<(), CoreError> {
        let run = db.get_agent_task_run(run_id)?;
        if run.conversation_id != conversation {
            return Err(CoreError::InvalidInput(
                "Delegation accounting owner mismatch".into(),
            ));
        }
        let subtasks = db.list_agent_subtask_runs(run_id)?;
        let ids = subtasks
            .iter()
            .filter(|row| {
                row.input
                    .as_ref()
                    .and_then(|value| value.get("kind"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|kind| matches!(kind, "subagent_run" | "subagent_judgement"))
            })
            .map(|row| row.id.clone())
            .collect();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            for row in &subtasks {
                if let Some(limits) = row
                    .input
                    .as_ref()
                    .and_then(|value| value.get("delegationLimitsV2"))
                {
                    if let Some(limit) = limits
                        .get("maxCallsPerTurn")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                    {
                        state.limits.max_calls_per_turn = Some(
                            state
                                .limits
                                .max_calls_per_turn
                                .map_or(limit, |current| current.min(limit)),
                        );
                    }
                    if let Some(limit) = limits
                        .get("totalActualTokensSoftLimit")
                        .and_then(serde_json::Value::as_u64)
                    {
                        state.limits.total_actual_tokens_soft_limit = Some(
                            state
                                .limits
                                .total_actual_tokens_soft_limit
                                .map_or(limit, |current| current.min(limit)),
                        );
                    }
                    if let Some(limit) = limits
                        .get("totalCostSoftLimitMicros")
                        .and_then(serde_json::Value::as_u64)
                    {
                        state.limits.total_cost_soft_limit_micros = Some(
                            state
                                .limits
                                .total_cost_soft_limit_micros
                                .map_or(limit, |current| current.min(limit)),
                        );
                    }
                }
            }
        }
        *self
            .resume
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(ResumeAccounting {
            db,
            run_id: run_id.into(),
            ids,
            calls: 0,
            verification_calls: 0,
            judge_calls: 0,
            tokens: 0,
            cost: 0,
            reservations: 0,
        });
        self.refresh_resume_accounting()
    }

    pub(crate) fn block_accounting(&self, detail: String) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .restore_error = Some(detail);
    }

    fn refresh_resume_accounting(&self) -> Result<(), CoreError> {
        let mut binding = self
            .resume
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(prior) = binding.as_mut() else {
            return Ok(());
        };
        let result = (|| {
            let rows = prior.db.list_agent_subtask_runs(&prior.run_id)?;
            let usage = prior.db.delegated_usage_for_run(&prior.run_id)?;
            let (mut calls, mut verifiers, mut judges, mut tokens, mut cost, mut reservations) =
                (0u32, 0u32, 0u32, 0u32, 0u64, 0u32);
            for row in rows.iter().filter(|row| prior.ids.contains(&row.id)) {
                let input = row.input.as_ref();
                if row.started_at.is_some() {
                    calls = calls.saturating_add(1);
                    if input
                        .and_then(|value| value.get("kind"))
                        .and_then(serde_json::Value::as_str)
                        == Some("subagent_judgement")
                    {
                        judges += 1;
                    } else if input
                        .and_then(|value| value.get("roleId"))
                        .and_then(serde_json::Value::as_str)
                        == Some("verifier")
                    {
                        verifiers += 1;
                    }
                    if matches!(row.status.as_str(), "queued" | "running") {
                        reservations = reservations.saturating_add(row.token_budget.unwrap_or(0));
                    }
                }
                let (ledger_tokens, ledger_cost) = usage.get(&row.id).copied().unwrap_or_default();
                let artifact_tokens = row
                    .output
                    .as_ref()
                    .and_then(|output| {
                        output
                            .pointer("/run/usageTotal/totalTokens")
                            .or_else(|| output.pointer("/usageTotal/totalTokens"))
                            .or_else(|| output.pointer("/judgement/usageTotal/totalTokens"))
                    })
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                tokens = tokens.saturating_add(
                    ledger_tokens.max(artifact_tokens).min(u64::from(u32::MAX)) as u32,
                );
                cost = cost.saturating_add(ledger_cost);
            }
            Ok::<_, CoreError>((calls, verifiers, judges, tokens, cost, reservations))
        })();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        match result {
            Ok((calls, verifiers, judges, tokens, cost, reservations)) => {
                state.restore_error = None;
                state.calls_started = state
                    .calls_started
                    .saturating_add(calls.saturating_sub(prior.calls));
                state.verification_calls_started = state
                    .verification_calls_started
                    .saturating_add(verifiers.saturating_sub(prior.verification_calls));
                state.judge_calls_started = state
                    .judge_calls_started
                    .saturating_add(judges.saturating_sub(prior.judge_calls));
                state.tokens_spent = state
                    .tokens_spent
                    .saturating_add(tokens.saturating_sub(prior.tokens));
                state.cost_spent_micros = state
                    .cost_spent_micros
                    .saturating_add(cost.saturating_sub(prior.cost));
                state.tokens_reserved = state
                    .tokens_reserved
                    .saturating_sub(prior.reservations)
                    .saturating_add(reservations);
                prior.calls = prior.calls.max(calls);
                prior.verification_calls = prior.verification_calls.max(verifiers);
                prior.judge_calls = prior.judge_calls.max(judges);
                prior.tokens = prior.tokens.max(tokens);
                prior.cost = prior.cost.max(cost);
                prior.reservations = reservations;
                Ok(())
            }
            Err(error) => {
                state.restore_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    pub(crate) fn new(config: &AgentConfig) -> Self {
        Self::with_limits(DelegationLimitsV2::resolve(config))
    }

    fn with_limits(limits: DelegationLimitsV2) -> Self {
        let dedicated_lanes = limits.verification_lane_slots > 0 || limits.judge_lane_slots > 0;
        let shared_lane =
            (!dedicated_lanes).then(|| Arc::new(Semaphore::new(limits.max_parallel as usize)));
        Self {
            resume: Arc::new(Mutex::new(None)),
            shared_lane,
            exploration_lane: Arc::new(Semaphore::new(limits.exploration_lane_slots as usize)),
            verification_lane: (limits.verification_lane_slots > 0)
                .then(|| Arc::new(Semaphore::new(limits.verification_lane_slots as usize))),
            judge_lane: (limits.judge_lane_slots > 0)
                .then(|| Arc::new(Semaphore::new(limits.judge_lane_slots as usize))),
            state: Arc::new(Mutex::new(DelegationBudgetState {
                restore_error: None,
                limits,
                calls_started: 0,
                verification_calls_started: 0,
                judge_calls_started: 0,
                tokens_spent: 0,
                tokens_reserved: 0,
                cost_spent_micros: 0,
            })),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_queue_deadline(config: &AgentConfig, queue_deadline: Duration) -> Self {
        let mut limits = DelegationLimitsV2::resolve(config);
        limits.queue_deadline_ms =
            Some(u64::try_from(queue_deadline.as_millis()).unwrap_or(u64::MAX));
        Self::with_limits(limits)
    }

    /// Queue before allocating an isolated worker thread/provider. Admission
    /// credit is committed only after that worker's preflight succeeds.
    pub(crate) async fn acquire_worker_slot(
        &self,
        label: &str,
        is_verification: bool,
        cancel_token: &CancellationToken,
    ) -> Result<OwnedSemaphorePermit, CoreError> {
        let lane = if is_verification {
            DelegationLane::Verification
        } else {
            DelegationLane::Exploration
        };
        self.acquire_slot(label, lane, cancel_token).await
    }

    pub(crate) async fn commit_worker_call(
        &self,
        label: &str,
        reserved_tokens: u32,
        is_verification: bool,
        cancel_token: &CancellationToken,
    ) -> Result<WorkerBudgetReservation, CoreError> {
        let lane = if is_verification {
            DelegationLane::Verification
        } else {
            DelegationLane::Exploration
        };
        self.commit_call(label, reserved_tokens, lane, cancel_token)
            .await?;
        Ok(WorkerBudgetReservation {
            state: Arc::clone(&self.state),
            reserved_tokens,
            is_verification,
            started: false,
            settled: false,
        })
    }

    pub(crate) async fn begin_judge_call(
        &self,
        label: &str,
        reserved_tokens: u32,
        cancel_token: &CancellationToken,
    ) -> Result<OwnedSemaphorePermit, CoreError> {
        self.admit(label, reserved_tokens, DelegationLane::Judge, cancel_token)
            .await
    }

    async fn admit(
        &self,
        label: &str,
        reserved_tokens: u32,
        lane: DelegationLane,
        cancel_token: &CancellationToken,
    ) -> Result<OwnedSemaphorePermit, CoreError> {
        let permit = self.acquire_slot(label, lane, cancel_token).await?;
        self.commit_call(label, reserved_tokens, lane, cancel_token)
            .await?;
        Ok(permit)
    }

    async fn acquire_slot(
        &self,
        label: &str,
        lane: DelegationLane,
        cancel_token: &CancellationToken,
    ) -> Result<OwnedSemaphorePermit, CoreError> {
        self.refresh_resume_accounting()?;
        // Phase one: enqueue and wait for a concrete worker lane. No call or
        // token credit is consumed while this future is merely queued.
        let queue_deadline_ms = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            Self::validate_admission(&state, label, lane)?;
            state.limits.queue_deadline_ms
        };
        let semaphore = self.semaphore_for(lane);
        let permit = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => Err(CoreError::Agent(format!(
                "Delegated execution '{label}' was cancelled while waiting for a worker slot."
            ))),
            result = with_optional_timeout(
                queue_deadline_ms,
                semaphore.acquire_owned(),
            ) => match result {
                Ok(Ok(permit)) => Ok(permit),
                Ok(Err(_)) => Err(CoreError::Internal(
                    "delegated execution semaphore closed".into()
                )),
                Err(_) => Err(CoreError::Agent(format!(
                    "Delegated execution '{label}' exceeded its {}ms queue deadline.",
                    queue_deadline_ms.unwrap_or_default()
                ))),
            },
        }?;

        if cancel_token.is_cancelled() {
            return Err(CoreError::Agent(format!(
                "Delegated execution '{label}' was cancelled before worker admission."
            )));
        }
        Self::validate_admission(
            &self.state.lock().unwrap_or_else(|error| error.into_inner()),
            label,
            lane,
        )?;
        Ok(permit)
    }

    async fn commit_call(
        &self,
        label: &str,
        reserved_tokens: u32,
        lane: DelegationLane,
        cancel_token: &CancellationToken,
    ) -> Result<(), CoreError> {
        self.refresh_resume_accounting()?;
        // Phase two: once a slot exists, consume call admission and the small
        // role credit. Actual usage replaces this reservation at completion.
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if cancel_token.is_cancelled() {
            return Err(CoreError::Agent(format!(
                "Delegated execution '{label}' was cancelled before worker admission."
            )));
        }
        Self::validate_admission(&state, label, lane)?;
        state.calls_started += 1;
        if lane == DelegationLane::Verification {
            state.verification_calls_started += 1;
        } else if lane == DelegationLane::Judge {
            state.judge_calls_started += 1;
        }
        state.tokens_reserved = state.tokens_reserved.saturating_add(reserved_tokens);
        drop(state);
        Ok(())
    }

    fn validate_admission(
        state: &DelegationBudgetState,
        label: &str,
        lane: DelegationLane,
    ) -> Result<(), CoreError> {
        if let Some(error) = &state.restore_error {
            return Err(CoreError::InvalidInput(format!(
                "Delegation accounting could not be restored: {error}"
            )));
        }
        let reserved_for_other_control_lanes = match lane {
            DelegationLane::Exploration => {
                u32::from(
                    state.limits.verification_lane_slots > 0
                        && state.verification_calls_started == 0,
                ) + u32::from(state.limits.judge_lane_slots > 0 && state.judge_calls_started == 0)
            }
            DelegationLane::Verification => {
                u32::from(state.limits.judge_lane_slots > 0 && state.judge_calls_started == 0)
            }
            DelegationLane::Judge => u32::from(
                state.limits.verification_lane_slots > 0 && state.verification_calls_started == 0,
            ),
        };
        let lane_call_limit = state
            .limits
            .max_calls_per_turn
            .map(|limit| limit.saturating_sub(reserved_for_other_control_lanes));
        if lane_call_limit.is_some_and(|limit| state.calls_started >= limit) {
            return Err(CoreError::InvalidInput(format!(
                "Delegated execution budget exhausted before starting {label}: {} call(s) started and {reserved_for_other_control_lanes} call credit(s) remain reserved for unused control lanes (maximum {} per turn).",
                state.calls_started,
                state.limits.max_calls_per_turn.unwrap_or_default(),
            )));
        }
        let token_budget = state
            .limits
            .total_actual_tokens_soft_limit
            .unwrap_or(u64::MAX);
        if lane == DelegationLane::Exploration && u64::from(state.tokens_spent) >= token_budget {
            return Err(CoreError::InvalidInput(format!(
                "Delegated execution token soft limit exhausted before starting {label}. Spent: {} of {token_budget} tokens.",
                state.tokens_spent,
            )));
        }
        if let Some(cost_limit) = state.limits.total_cost_soft_limit_micros {
            // A soft cost limit is enforceable only when the selected provider
            // has versioned pricing metadata. Unknown pricing remains visible
            // in BudgetSnapshot, but must not disable all remote delegation.
            if state.limits.cost_accounting_available && state.cost_spent_micros >= cost_limit {
                return Err(CoreError::InvalidInput(format!(
                    "Delegated execution cost soft limit exhausted before starting {label}. Spent: {} of {cost_limit} micros.",
                    state.cost_spent_micros,
                )));
            }
        }
        Ok(())
    }

    fn semaphore_for(&self, lane: DelegationLane) -> Arc<Semaphore> {
        if let Some(shared) = &self.shared_lane {
            return Arc::clone(shared);
        }
        match lane {
            DelegationLane::Exploration => Arc::clone(&self.exploration_lane),
            DelegationLane::Verification => self
                .verification_lane
                .as_ref()
                .map(Arc::clone)
                .unwrap_or_else(|| Arc::clone(&self.exploration_lane)),
            DelegationLane::Judge => self
                .judge_lane
                .as_ref()
                .map(Arc::clone)
                .or_else(|| self.verification_lane.as_ref().map(Arc::clone))
                .unwrap_or_else(|| Arc::clone(&self.exploration_lane)),
        }
    }

    pub(crate) async fn finish_call(
        &self,
        reserved_tokens: u32,
        usage: &Usage,
        estimated_cost_micros: Option<u64>,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.tokens_reserved = state.tokens_reserved.saturating_sub(reserved_tokens);
        state.tokens_spent = state.tokens_spent.saturating_add(usage.total_tokens);
        state.cost_spent_micros = state
            .cost_spent_micros
            .saturating_add(estimated_cost_micros.unwrap_or(0));
    }

    pub(crate) async fn rollback_unstarted_judge(&self, reserved_tokens: u32) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.calls_started = state.calls_started.saturating_sub(1);
        state.judge_calls_started = state.judge_calls_started.saturating_sub(1);
        state.tokens_reserved = state.tokens_reserved.saturating_sub(reserved_tokens);
    }

    pub(crate) async fn snapshot(&self) -> BudgetSnapshot {
        let _ = self.refresh_resume_accounting();
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let token_budget = state
            .limits
            .total_actual_tokens_soft_limit
            .unwrap_or(u64::from(u32::MAX))
            .min(u64::from(u32::MAX)) as u32;
        BudgetSnapshot {
            max_parallel: state.limits.max_parallel,
            max_calls_per_turn: state.limits.max_calls_per_turn,
            calls_started: state.calls_started,
            remaining_calls: state
                .limits
                .max_calls_per_turn
                .map(|limit| limit.saturating_sub(state.calls_started)),
            token_budget,
            tokens_spent: state.tokens_spent,
            tokens_reserved: state.tokens_reserved,
            remaining_tokens: token_budget
                .saturating_sub(state.tokens_spent.saturating_add(state.tokens_reserved)),
            // V2 reserves worker lanes and role credits, not a frozen fraction
            // of the entire delegated token budget.
            verification_reserve_tokens: 0,
            cost_soft_limit_micros: state.limits.total_cost_soft_limit_micros,
            cost_spent_micros: state.cost_spent_micros,
            cost_accounting_available: state.limits.cost_accounting_available,
            exploration_lane_slots: state.limits.exploration_lane_slots,
            verification_lane_slots: state.limits.verification_lane_slots,
            judge_lane_slots: state.limits.judge_lane_slots,
        }
    }

    pub(crate) async fn limits(&self) -> DelegationLimitsV2 {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .limits
            .clone()
    }
}
