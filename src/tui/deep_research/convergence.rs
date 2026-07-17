//! Deterministic convergence policy for DeepResearch.
//!
//! The model and workflow collect evidence; this policy alone decides whether
//! another collection round is justified. Keeping the decision typed and pure
//! makes every stop/continue result replayable and testable.

use serde::{Deserialize, Serialize};

use a3s::research::{InquiryPhase, InquiryState, QuestionStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InquiryTerminalOutcome {
    Completed,
    Qualified,
    Exhausted,
}

/// Derive the only publishable terminal meaning from the replayed inquiry
/// projection. Retrieval workflows and their legacy checkers are evidence
/// producers; they do not own cross-wave convergence for a host-managed
/// inquiry.
pub(crate) fn inquiry_terminal_outcome(state: &InquiryState) -> Option<InquiryTerminalOutcome> {
    if state.phase == InquiryPhase::Exhausted {
        return Some(InquiryTerminalOutcome::Exhausted);
    }
    if !matches!(
        state.phase,
        InquiryPhase::Outlining
            | InquiryPhase::Drafting
            | InquiryPhase::Auditing
            | InquiryPhase::Completed
    ) {
        return None;
    }
    let material = state
        .questions
        .iter()
        .filter(|question| question.material)
        .collect::<Vec<_>>();
    if material.is_empty()
        || material
            .iter()
            .any(|question| question.status != QuestionStatus::Answered)
        || state
            .questions
            .iter()
            .any(|question| question.status == QuestionStatus::Queued)
    {
        return None;
    }
    if state
        .questions
        .iter()
        .any(|question| question.status == QuestionStatus::Bounded)
    {
        Some(InquiryTerminalOutcome::Qualified)
    } else {
        Some(InquiryTerminalOutcome::Completed)
    }
}

/// Validate an embedded inquiry projection before it can override legacy
/// workflow status fields. This prevents a forged or stale serialized state
/// from becoming a publication decision.
pub(crate) fn validated_inquiry_terminal_outcome(
    workflow: &serde_json::Value,
) -> Result<Option<InquiryTerminalOutcome>, String> {
    let Some(inquiry) = workflow.get("inquiry") else {
        return Ok(None);
    };
    let events: Vec<a3s::research::InquiryEvent> = serde_json::from_value(
        inquiry
            .get("events")
            .cloned()
            .ok_or_else(|| "DeepResearch inquiry projection omitted events".to_string())?,
    )
    .map_err(|error| format!("decode DeepResearch inquiry events: {error}"))?;
    let state: InquiryState = serde_json::from_value(
        inquiry
            .get("state")
            .cloned()
            .ok_or_else(|| "DeepResearch inquiry projection omitted state".to_string())?,
    )
    .map_err(|error| format!("decode DeepResearch inquiry state: {error}"))?;
    let replayed = a3s::research::replay(&events, &a3s::research::InquiryLimits::default())
        .map_err(|error| format!("replay DeepResearch inquiry projection: {error}"))?;
    if replayed != state {
        return Err("DeepResearch inquiry state differs from its event replay".to_string());
    }
    Ok(inquiry_terminal_outcome(&state))
}

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

/// Convert the replayed inquiry projection into the terminal workflow
/// decision. Cross-wave continuation is executed inside the inquiry runtime;
/// once that runtime returns, a `Continue` decision would be contradictory.
pub(crate) fn evaluate_terminal_inquiry_convergence(
    state: &InquiryState,
    input: ConvergenceInput,
) -> ConvergenceDecision {
    let (action, reason) = match inquiry_terminal_outcome(state) {
        Some(InquiryTerminalOutcome::Completed) => (
            ConvergenceAction::Finalize,
            "all inquiry questions are evidence-answered and ready for outlining".to_string(),
        ),
        Some(InquiryTerminalOutcome::Qualified) => (
            ConvergenceAction::Finalize,
            "all material inquiry questions are evidence-answered; bounded supporting gaps will qualify the report"
                .to_string(),
        ),
        Some(InquiryTerminalOutcome::Exhausted) => (
            ConvergenceAction::Degrade,
            state
                .budget_exhausted_reason
                .clone()
                .unwrap_or_else(|| "the replayed inquiry exhausted its bounded budget".to_string()),
        ),
        None => (
            ConvergenceAction::Degrade,
            format!(
                "the replayed inquiry returned from non-publishable phase {:?}; no hidden continuation was scheduled",
                state.phase
            ),
        ),
    };
    ConvergenceDecision {
        action,
        reason,
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

    #[test]
    fn terminal_inquiry_never_returns_continue() {
        let limits = a3s::research::InquiryLimits::default();
        let mut state = InquiryState::default();
        state
            .apply(
                &a3s::research::InquiryEvent::StrategySelected {
                    method: a3s::research::ResearchMethod::Focused,
                },
                &limits,
            )
            .expect("strategy");
        state
            .apply(
                &a3s::research::InquiryEvent::QuestionsQueued {
                    questions: vec![a3s::research::Question::queued(
                        "question:material",
                        None,
                        "What does the evidence establish?",
                    )],
                },
                &limits,
            )
            .expect("question");
        state
            .apply(
                &a3s::research::InquiryEvent::QuestionBounded {
                    question_id: "question:material".to_string(),
                    reason: "No accepted evidence remained.".to_string(),
                },
                &limits,
            )
            .expect("bounded");
        state
            .apply(
                &a3s::research::InquiryEvent::BudgetExhausted {
                    reason: "material question remained bounded".to_string(),
                },
                &limits,
            )
            .expect("exhausted");

        let decision = evaluate_terminal_inquiry_convergence(&state, input());
        assert_eq!(decision.action, ConvergenceAction::Degrade);
        assert_eq!(decision.reason, "material question remained bounded");
    }
}
