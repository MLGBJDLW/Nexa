//! Shared facts for observation-bound desktop and browser actions.
//!
//! Platform adapters retain target identity, permissions and input ownership.
//! This module records consumption and possible effects; it never authorizes
//! input, retries an action, or treats a visual change as a completed goal.

use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Debug, Clone, Default)]
pub struct InteractionCommit {
    consumed: Arc<AtomicBool>,
    effect_possible: Arc<AtomicBool>,
}

impl InteractionCommit {
    pub fn consume_observation(&self) {
        self.consumed.store(true, Ordering::Release);
    }
    pub fn before_effect(&self) {
        self.effect_possible.store(true, Ordering::Release);
    }
    pub fn observation_consumed(&self) -> bool {
        self.consumed.load(Ordering::Acquire)
    }
    pub fn effect_may_have_occurred(&self) -> bool {
        self.effect_possible.load(Ordering::Acquire)
    }

    pub fn failure(&self) -> InteractionReceipt {
        InteractionReceipt::failed(self.observation_consumed(), self.effect_may_have_occurred())
    }

    pub fn completed(
        &self,
        evidence: ObservedEvidence,
        condition: RequestedCondition,
    ) -> InteractionReceipt {
        InteractionReceipt {
            delivery: if self.effect_may_have_occurred() {
                InputDelivery::InputSent
            } else {
                InputDelivery::NotCommitted
            },
            evidence,
            condition,
            observation_consumed: self.observation_consumed(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InputDelivery {
    NotCommitted,
    InputSent,
    MayHaveOccurred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservedEvidence {
    Changed,
    Unchanged,
    TargetClosed,
    Unavailable,
}

impl ObservedEvidence {
    pub fn from_changed(changed: bool) -> Self {
        if changed {
            Self::Changed
        } else {
            Self::Unchanged
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestedCondition {
    NotRequested,
    Matched,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionReceipt {
    pub delivery: InputDelivery,
    pub evidence: ObservedEvidence,
    pub condition: RequestedCondition,
    pub observation_consumed: bool,
}

impl InteractionReceipt {
    pub fn failed(observation_consumed: bool, effect_possible: bool) -> Self {
        Self {
            delivery: if effect_possible {
                InputDelivery::MayHaveOccurred
            } else {
                InputDelivery::NotCommitted
            },
            evidence: ObservedEvidence::Unavailable,
            condition: RequestedCondition::NotRequested,
            observation_consumed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InteractionPhase {
    Moving,
    Committing,
    Waiting,
    ObservedChanged,
    ObservedUnchanged,
    ConditionMatched,
    ObservedPending,
    Failed,
    Interrupted,
}

impl InteractionPhase {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Moving | Self::Committing | Self::Waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_preserves_consumption_and_possible_effects_across_worker_clones() {
        let caller = InteractionCommit::default();
        let worker = caller.clone();
        assert_eq!(caller.failure().delivery, InputDelivery::NotCommitted);
        worker.consume_observation();
        assert!(caller.failure().observation_consumed);
        worker.before_effect();
        assert_eq!(caller.failure().delivery, InputDelivery::MayHaveOccurred);
    }

    #[test]
    fn changed_pixels_never_prove_a_requested_condition() {
        let commit = InteractionCommit::default();
        commit.consume_observation();
        commit.before_effect();
        let receipt = commit.completed(ObservedEvidence::Changed, RequestedCondition::NotRequested);
        assert_eq!(receipt.delivery, InputDelivery::InputSent);
        assert_eq!(receipt.condition, RequestedCondition::NotRequested);
        let missing = commit.completed(ObservedEvidence::Unavailable, RequestedCondition::Pending);
        assert_eq!(missing.delivery, InputDelivery::InputSent);
        assert_eq!(missing.condition, RequestedCondition::Pending);
    }
}
