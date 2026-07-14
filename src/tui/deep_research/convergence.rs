//! Deterministic convergence policy for DeepResearch.
//!
//! The model and workflow collect evidence; this policy alone decides whether
//! another collection round is justified. Keeping the decision typed and pure
//! makes every stop/continue result replayable and testable.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConvergenceInput {
    pub(crate) accepted_evidence: usize,
    pub(crate) traceable_sources: usize,
    pub(crate) authoritative_sources: usize,
    pub(crate) unresolved_contradictions: usize,
    pub(crate) unresolved_gaps: usize,
    pub(crate) completed_rounds: usize,
    pub(crate) max_rounds: usize,
    pub(crate) rounds_without_material_gain: usize,
    pub(crate) remaining_ms: u64,
    pub(crate) finalization_reserve_ms: u64,
    pub(crate) evidence_package_complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConvergenceAction {
    Continue,
    Finalize,
    Degrade,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConvergenceDecision {
    pub(crate) action: ConvergenceAction,
    pub(crate) reason: String,
    pub(crate) input: ConvergenceInput,
}

pub(crate) fn evaluate_convergence(input: ConvergenceInput) -> ConvergenceDecision {
    let (action, reason) =
        if input.evidence_package_complete && input.unresolved_contradictions == 0 {
            (
                ConvergenceAction::Finalize,
                "validated evidence package satisfies the completion gate",
            )
        } else if input.remaining_ms <= input.finalization_reserve_ms {
            (
                ConvergenceAction::Degrade,
                "finalization reserve reached; retrieval must stop",
            )
        } else if input.completed_rounds >= input.max_rounds.max(1) {
            (
                ConvergenceAction::Degrade,
                "bounded research round limit reached",
            )
        } else if input.rounds_without_material_gain >= 2 {
            (
                ConvergenceAction::Degrade,
                "two consecutive rounds produced no material evidence gain",
            )
        } else if input.accepted_evidence == 0 && input.completed_rounds > 0 {
            (
                ConvergenceAction::Degrade,
                "completed retrieval produced no accepted evidence",
            )
        } else {
            (
                ConvergenceAction::Continue,
                "material evidence gaps remain within the retrieval budget",
            )
        };
    ConvergenceDecision {
        action,
        reason: reason.to_string(),
        input,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ConvergenceInput {
        ConvergenceInput {
            accepted_evidence: 3,
            traceable_sources: 2,
            authoritative_sources: 1,
            unresolved_contradictions: 0,
            unresolved_gaps: 1,
            completed_rounds: 1,
            max_rounds: 3,
            rounds_without_material_gain: 0,
            remaining_ms: 60_000,
            finalization_reserve_ms: 10_000,
            evidence_package_complete: false,
        }
    }

    #[test]
    fn complete_package_finalizes_without_an_extra_round() {
        let mut state = input();
        state.evidence_package_complete = true;
        assert_eq!(
            evaluate_convergence(state).action,
            ConvergenceAction::Finalize
        );
    }

    #[test]
    fn finalization_reserve_preempts_more_retrieval() {
        let mut state = input();
        state.remaining_ms = state.finalization_reserve_ms;
        assert_eq!(
            evaluate_convergence(state).action,
            ConvergenceAction::Degrade
        );
    }

    #[test]
    fn repeated_no_gain_stops_deterministically() {
        let mut state = input();
        state.rounds_without_material_gain = 2;
        assert_eq!(
            evaluate_convergence(state).action,
            ConvergenceAction::Degrade
        );
    }

    #[test]
    fn unresolved_material_gap_can_continue_within_budget() {
        assert_eq!(
            evaluate_convergence(input()).action,
            ConvergenceAction::Continue
        );
    }
}
