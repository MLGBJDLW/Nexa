//! Logical tool-round budgeting for one user turn.
//!
//! Provider samples are transport/runtime implementation details. They must
//! not consume the user-facing tool-round budget when Nexa is continuing an
//! output-limited answer, repairing an incomplete provider envelope, or
//! restarting a discarded sample. A finite tool budget also owns one
//! answer-only synthesis step after the last verified tool round.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnStepPurpose {
    Normal,
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnStepMode {
    ToolsAllowed,
    FinalAnswerOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TurnStepPermit {
    pub(super) sample_index: u32,
    pub(super) mode: TurnStepMode,
    pub(super) tool_rounds_used: u32,
    pub(super) remaining_tool_rounds: Option<u32>,
}

impl TurnStepPermit {
    pub(super) fn allows_tools(self) -> bool {
        self.mode == TurnStepMode::ToolsAllowed
    }
}

/// Owns the distinction between logical tool rounds and physical model
/// samples. Provider invocations have no implicit per-turn ceiling. Only an
/// explicitly configured tool-round policy can exhaust this budget.
/// `u32::MAX` remains the legacy boundary encoding for no configured
/// tool-round cap; it is normalized to `None` at this seam.
#[derive(Debug)]
pub(super) struct TurnBudget {
    tool_round_limit: Option<u32>,
    tool_rounds_used: u32,
    next_sample_index: u32,
    final_answer_sample_started: bool,
}

impl TurnBudget {
    pub(super) fn restore(&mut self, used: u32, prior_limit: Option<u32>) {
        self.tool_rounds_used = self.tool_rounds_used.max(used);
        if let Some(prior) = prior_limit.filter(|limit| *limit != u32::MAX) {
            self.tool_round_limit = Some(
                self.tool_round_limit
                    .map_or(prior, |limit| limit.min(prior)),
            );
        }
    }

    pub(super) fn restore_receipts(
        &mut self,
        db: &crate::db::Database,
        conversation: Option<&str>,
        turn: Option<&str>,
    ) -> Result<(), crate::error::CoreError> {
        let (Some(conversation), Some(turn)) = (conversation, turn) else {
            return Ok(());
        };
        let Some(run) = db.get_agent_task_run_by_turn(turn)? else {
            return Ok(());
        };
        if run.conversation_id != conversation {
            return Err(crate::error::CoreError::InvalidInput(
                "Turn budget owner mismatch".into(),
            ));
        }
        for event in db
            .get_agent_task_run_events(&run.id)?
            .into_iter()
            .filter(|event| event.event_type == "tool_round_admitted")
        {
            let value = event.payload.ok_or_else(|| {
                crate::error::CoreError::InvalidInput("Missing durable tool-round admission".into())
            })?;
            let count = value
                .get("roundsCharged")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    crate::error::CoreError::InvalidInput(
                        "Invalid durable tool-round admission".into(),
                    )
                })?;
            let limit = value
                .get("maxIterations")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u32::try_from(value).ok());
            self.restore(count, limit);
        }
        // Older hosts recorded logical rounds in the host-owned turn trace.
        if let Some(trace) = db.get_conversation_turn(turn)?.trace {
            self.restore_trace(&trace);
        }
        Ok(())
    }

    fn restore_trace(&mut self, trace: &serde_json::Value) {
        for event in trace
            .get("items")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.get("kind").and_then(serde_json::Value::as_str) == Some("loop"))
            .filter_map(|item| item.get("event"))
        {
            match event.get("kind").and_then(serde_json::Value::as_str) {
                Some("toolFinished") => {
                    if let Some(index) = event
                        .get("iteration")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                    {
                        self.restore(index.saturating_add(1), None);
                    }
                }
                Some("turnStarted") => self.restore(
                    0,
                    event
                        .get("max_iterations")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok()),
                ),
                _ => {}
            }
        }
    }

    pub(super) fn persist_admission(
        &self,
        db: &crate::db::Database,
        turn: Option<&str>,
    ) -> Result<(), crate::error::CoreError> {
        let Some(run) = turn
            .map(|turn| db.get_agent_task_run_by_turn(turn))
            .transpose()?
            .flatten()
        else {
            return Ok(());
        };
        db.record_agent_task_run_event(&run.id, "tool_round_admitted", "Tool round admitted", Some("running"), Some(&serde_json::json!({"roundsCharged":self.tool_rounds_used.saturating_add(1), "maxIterations":self.tool_round_limit})))?;
        Ok(())
    }
    pub(super) fn new(legacy_max_iterations: u32) -> Self {
        Self {
            tool_round_limit: (legacy_max_iterations != u32::MAX).then_some(legacy_max_iterations),
            tool_rounds_used: 0,
            next_sample_index: 0,
            final_answer_sample_started: false,
        }
    }

    pub(super) fn permit(&mut self, purpose: TurnStepPurpose) -> Option<TurnStepPermit> {
        let final_answer_only = self
            .tool_round_limit
            .is_some_and(|limit| self.tool_rounds_used >= limit);

        if final_answer_only
            && purpose == TurnStepPurpose::Normal
            && self.final_answer_sample_started
        {
            return None;
        }
        if final_answer_only {
            self.final_answer_sample_started = true;
        }

        let permit = TurnStepPermit {
            sample_index: self.next_sample_index,
            mode: if final_answer_only {
                TurnStepMode::FinalAnswerOnly
            } else {
                TurnStepMode::ToolsAllowed
            },
            tool_rounds_used: self.tool_rounds_used,
            remaining_tool_rounds: self.remaining_tool_rounds(),
        };
        self.next_sample_index = self.next_sample_index.saturating_add(1);
        Some(permit)
    }

    pub(super) fn record_verified_tool_round(&mut self) {
        debug_assert!(
            self.can_dispatch_tool_round(),
            "a verified tool round must acquire budget authority before dispatch"
        );
        self.tool_rounds_used = self.tool_rounds_used.saturating_add(1);
    }

    /// Whether a controller- or model-owned tool batch may enter dispatch.
    /// Unlike [`Self::can_start_normal_step`], this never treats the reserved
    /// answer-only sample as tool authority.
    pub(super) fn can_dispatch_tool_round(&self) -> bool {
        self.tool_round_limit
            .is_none_or(|limit| self.tool_rounds_used < limit)
    }

    pub(super) fn can_start_normal_step(&self) -> bool {
        match self.tool_round_limit {
            None => true,
            Some(limit) if self.tool_rounds_used < limit => true,
            Some(_) => !self.final_answer_sample_started,
        }
    }

    pub(super) fn tool_rounds_used(&self) -> u32 {
        self.tool_rounds_used
    }

    pub(super) fn remaining_tool_rounds(&self) -> Option<u32> {
        self.tool_round_limit
            .map(|limit| limit.saturating_sub(self.tool_rounds_used))
    }

    pub(super) fn configured_tool_round_limit(&self) -> Option<u32> {
        self.tool_round_limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_samples_do_not_consume_a_tool_round() {
        let mut budget = TurnBudget::new(1);

        let first = budget.permit(TurnStepPurpose::Normal).unwrap();
        let recovery = budget.permit(TurnStepPurpose::Recovery).unwrap();

        assert!(first.allows_tools());
        assert!(recovery.allows_tools());
        assert_eq!(first.tool_rounds_used, 0);
        assert_eq!(recovery.tool_rounds_used, 0);
        assert_eq!(recovery.sample_index, 1);
    }

    #[test]
    fn finite_tool_budget_reserves_one_answer_only_sample() {
        let mut budget = TurnBudget::new(1);
        assert!(budget
            .permit(TurnStepPurpose::Normal)
            .unwrap()
            .allows_tools());
        budget.record_verified_tool_round();

        let final_answer = budget.permit(TurnStepPurpose::Normal).unwrap();
        assert_eq!(final_answer.mode, TurnStepMode::FinalAnswerOnly);
        assert_eq!(final_answer.remaining_tool_rounds, Some(0));
        assert!(budget.permit(TurnStepPurpose::Normal).is_none());
    }

    #[test]
    fn final_answer_output_recovery_remains_permitted_without_tools() {
        let mut budget = TurnBudget::new(0);
        assert!(!budget.can_dispatch_tool_round());
        let final_answer = budget.permit(TurnStepPurpose::Normal).unwrap();
        let continuation = budget.permit(TurnStepPurpose::Recovery).unwrap();

        assert_eq!(final_answer.mode, TurnStepMode::FinalAnswerOnly);
        assert_eq!(continuation.mode, TurnStepMode::FinalAnswerOnly);
        assert_eq!(continuation.sample_index, 1);
    }

    #[test]
    fn legacy_unlimited_budget_never_enters_answer_only_mode() {
        let mut budget = TurnBudget::new(u32::MAX);
        for _ in 0..1_024 {
            assert_eq!(
                budget.permit(TurnStepPurpose::Normal).unwrap().mode,
                TurnStepMode::ToolsAllowed
            );
            budget.record_verified_tool_round();
        }
        assert_eq!(budget.configured_tool_round_limit(), None);
    }

    #[test]
    fn controller_tool_batches_share_the_verified_round_budget() {
        let mut budget = TurnBudget::new(1);
        assert!(budget.can_dispatch_tool_round());
        budget.record_verified_tool_round();
        assert!(!budget.can_dispatch_tool_round());
        assert_eq!(
            budget.permit(TurnStepPurpose::Normal).unwrap().mode,
            TurnStepMode::FinalAnswerOnly
        );
    }

    #[test]
    fn resumed_budget_preserves_spend_and_explicit_ceiling() {
        let mut budget = TurnBudget::new(20);
        budget.restore(1, Some(2));
        assert_eq!(budget.remaining_tool_rounds(), Some(1));
        budget.restore(0, Some(99));
        assert_eq!(budget.remaining_tool_rounds(), Some(1));
        budget.record_verified_tool_round();
        assert!(!budget.can_dispatch_tool_round());
        assert!(!budget
            .permit(TurnStepPurpose::Normal)
            .unwrap()
            .allows_tools());
    }

    #[test]
    fn untrusted_tool_artifacts_cannot_rewrite_resumed_budget() {
        let mut budget = TurnBudget::new(20);
        budget.restore_trace(&serde_json::json!({"items":[{"kind":"tool","toolCall":{"artifacts":{"kind":"turnStarted","max_iterations":0}}},{"kind":"loop","event":{"kind":"toolFinished","iteration":2}}]}));
        assert_eq!(budget.remaining_tool_rounds(), Some(17));
    }
}
