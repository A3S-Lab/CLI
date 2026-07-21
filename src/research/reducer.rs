//! Transition and hard-limit validation for the inquiry projection.

use std::collections::HashSet;

use super::model::{MAX_PERSPECTIVE_RETRIEVAL_WAVES, MIN_PERSPECTIVE_RETRIEVAL_WAVES};
use super::validation::{
    ensure_limit, ensure_nonempty, ensure_string, ensure_strings, ensure_unique_ids,
    validate_outline_limits, validate_queued_questions, validate_research_obligations,
    validate_section_citations,
};
use super::{
    material_evidence_floor, research_contract_outcome, validate_research_contract_assessment,
    validate_research_outline, InquiryAudit, InquiryError, InquiryEvent, InquiryLimits,
    InquiryPhase, InquiryState, QuestionStatus, ResearchContractOutcome, ResearchMethod,
    SectionDraft, SectionRevision, SourceEvidenceRole,
};

pub fn reduce(
    state: &InquiryState,
    event: &InquiryEvent,
    limits: &InquiryLimits,
) -> Result<InquiryState, InquiryError> {
    ensure_limit(
        "events",
        state.events_applied.saturating_add(1),
        limits.max_events,
    )?;
    if state.phase.is_terminal() {
        return Err(invalid_transition(state, event));
    }

    let mut next = state.clone();
    match event {
        InquiryEvent::StrategySelected { method } => {
            require_phase(state, event, &[InquiryPhase::StrategySelection])?;
            next.method = Some(*method);
            next.phase = match method {
                ResearchMethod::Focused => InquiryPhase::Questioning,
                ResearchMethod::PerspectiveGuided => InquiryPhase::Scouting,
            };
        }
        InquiryEvent::ResearchObligationsCommitted {
            obligations,
            stop_conditions,
        } => {
            require_phase(
                state,
                event,
                &[InquiryPhase::Scouting, InquiryPhase::Questioning],
            )?;
            if !state.obligations.is_empty()
                || !state.stop_conditions.is_empty()
                || state.scout_completed
                || state.perspective_retrieval_wave_budget.is_some()
                || !state.perspectives.is_empty()
                || !state.questions.is_empty()
                || !state.evidence_catalog.is_empty()
            {
                return Err(invalid_transition(state, event));
            }
            validate_research_obligations(obligations, stop_conditions, limits)?;
            next.obligations.clone_from(obligations);
            next.stop_conditions.clone_from(stop_conditions);
        }
        // The following three arms are retained exclusively to replay journals
        // written by the removed scout/perspective runtime.
        InquiryEvent::ScoutCompleted { source_ids } => {
            require_phase(state, event, &[InquiryPhase::Scouting])?;
            ensure_nonempty(source_ids, "scout sources")?;
            ensure_limit("scout sources", source_ids.len(), limits.max_scout_sources)?;
            ensure_unique_ids(source_ids.iter().map(String::as_str), "scout source")?;
            ensure_strings(source_ids, "scout source", limits.max_text_chars)?;
            next.scout_completed = true;
            next.scout_source_ids.clone_from(source_ids);
            next.phase = InquiryPhase::PerspectiveDiscovery;
        }
        InquiryEvent::PerspectiveBudgetSelected {
            total_retrieval_waves,
        } => {
            require_phase(state, event, &[InquiryPhase::PerspectiveDiscovery])?;
            if state.perspective_retrieval_wave_budget.is_some()
                || !(MIN_PERSPECTIVE_RETRIEVAL_WAVES..=MAX_PERSPECTIVE_RETRIEVAL_WAVES)
                    .contains(total_retrieval_waves)
            {
                return Err(InquiryError::InvalidResearchPlan {
                    reason: format!(
                        "perspective retrieval wave budget must be selected exactly once between {MIN_PERSPECTIVE_RETRIEVAL_WAVES} and {MAX_PERSPECTIVE_RETRIEVAL_WAVES}; got {total_retrieval_waves}"
                    ),
                });
            }
            next.perspective_retrieval_wave_budget = Some(*total_retrieval_waves);
        }
        InquiryEvent::PerspectivesCommitted { perspectives } => {
            require_phase(state, event, &[InquiryPhase::PerspectiveDiscovery])?;
            ensure_nonempty(perspectives, "perspectives")?;
            ensure_limit("perspectives", perspectives.len(), limits.max_perspectives)?;
            ensure_unique_ids(
                perspectives
                    .iter()
                    .map(|perspective| perspective.id.as_str()),
                "perspective",
            )?;
            let scout_sources = state
                .scout_source_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            for perspective in perspectives {
                ensure_string(
                    &perspective.id,
                    "perspective id",
                    limits.max_identifier_chars,
                )?;
                ensure_string(
                    &perspective.title,
                    "perspective title",
                    limits.max_text_chars,
                )?;
                ensure_string(
                    &perspective.focus,
                    "perspective focus",
                    limits.max_text_chars,
                )?;
                ensure_nonempty(&perspective.source_ids, "perspective source ids")?;
                ensure_limit(
                    "perspective source ids",
                    perspective.source_ids.len(),
                    limits.max_scout_sources,
                )?;
                ensure_unique_ids(
                    perspective.source_ids.iter().map(String::as_str),
                    "perspective source",
                )?;
                for source_id in &perspective.source_ids {
                    if !scout_sources.contains(source_id.as_str()) {
                        return Err(InquiryError::UnknownId {
                            resource: "scout source",
                            id: source_id.clone(),
                        });
                    }
                }
            }
            next.perspectives.clone_from(perspectives);
            next.phase = InquiryPhase::Questioning;
        }
        InquiryEvent::QuestionsQueued { questions } => {
            require_phase(
                state,
                event,
                &[InquiryPhase::Questioning, InquiryPhase::Outlining],
            )?;
            if state.outline.is_some() {
                return Err(invalid_transition(state, event));
            }
            validate_queued_questions(state, questions, limits)?;
            next.questions.extend(questions.iter().cloned());
            next.phase = InquiryPhase::Questioning;
        }
        InquiryEvent::EvidenceAccepted { evidence } => {
            require_phase(
                state,
                event,
                &[
                    InquiryPhase::Scouting,
                    InquiryPhase::PerspectiveDiscovery,
                    InquiryPhase::Questioning,
                ],
            )?;
            ensure_string(
                &evidence.evidence_id,
                "evidence id",
                limits.max_identifier_chars,
            )?;
            ensure_nonempty(&evidence.claim_ids, "evidence claim ids")?;
            ensure_nonempty(&evidence.source_ids, "evidence source ids")?;
            ensure_limit(
                "evidence claim ids",
                evidence.claim_ids.len(),
                limits.max_citation_ids_per_section,
            )?;
            ensure_limit(
                "evidence source ids",
                evidence.source_ids.len(),
                limits.max_citation_ids_per_section,
            )?;
            ensure_unique_ids(
                evidence.claim_ids.iter().map(String::as_str),
                "evidence claim",
            )?;
            ensure_unique_ids(
                evidence.source_ids.iter().map(String::as_str),
                "evidence source",
            )?;
            ensure_strings(
                &evidence.claim_ids,
                "evidence claim id",
                limits.max_identifier_chars,
            )?;
            ensure_strings(
                &evidence.source_ids,
                "evidence source id",
                limits.max_identifier_chars,
            )?;
            ensure_limit(
                "evidence source coverage",
                evidence.source_coverage.len(),
                limits.max_citation_ids_per_section,
            )?;
            let evidence_source_ids = evidence
                .source_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();
            let mut coverage_edges = HashSet::new();
            for binding in &evidence.source_coverage {
                ensure_string(
                    &binding.source_id,
                    "source coverage source id",
                    limits.max_identifier_chars,
                )?;
                ensure_string(
                    &binding.obligation_id,
                    "source coverage obligation id",
                    limits.max_identifier_chars,
                )?;
                if !evidence_source_ids.contains(binding.source_id.as_str()) {
                    return Err(InquiryError::UnknownId {
                        resource: "evidence source",
                        id: binding.source_id.clone(),
                    });
                }
                let obligation = state
                    .obligations
                    .iter()
                    .find(|obligation| obligation.id == binding.obligation_id)
                    .ok_or_else(|| InquiryError::UnknownId {
                        resource: "research obligation",
                        id: binding.obligation_id.clone(),
                    })?;
                if !coverage_edges
                    .insert((binding.source_id.as_str(), binding.obligation_id.as_str()))
                {
                    return Err(InquiryError::InvalidResearchPlan {
                        reason: format!(
                            "evidence `{}` repeats source coverage edge `{}` -> `{}`",
                            evidence.evidence_id, binding.source_id, binding.obligation_id
                        ),
                    });
                }
                ensure_nonempty(
                    &binding.completion_criterion_indexes,
                    "source coverage completion criterion indexes",
                )?;
                ensure_limit(
                    "source coverage completion criterion indexes",
                    binding.completion_criterion_indexes.len(),
                    limits.max_completion_criteria_per_obligation,
                )?;
                let mut criterion_indexes = HashSet::new();
                for criterion_index in &binding.completion_criterion_indexes {
                    if *criterion_index >= obligation.completion_criteria.len()
                        || !criterion_indexes.insert(*criterion_index)
                    {
                        return Err(InquiryError::InvalidResearchPlan {
                            reason: format!(
                                "source coverage edge `{}` -> `{}` has invalid or duplicate completion criterion index `{criterion_index}`",
                                binding.source_id, binding.obligation_id
                            ),
                        });
                    }
                }
                ensure_nonempty(&binding.roles, "source coverage roles")?;
                ensure_limit("source coverage roles", binding.roles.len(), 3)?;
                let roles = binding.roles.iter().copied().collect::<HashSet<_>>();
                if roles.len() != binding.roles.len()
                    || !roles.contains(&SourceEvidenceRole::Supporting)
                {
                    return Err(InquiryError::InvalidResearchPlan {
                        reason: format!(
                            "source coverage edge `{}` -> `{}` must contain unique roles including `supporting`",
                            binding.source_id, binding.obligation_id
                        ),
                    });
                }
                if roles.contains(&SourceEvidenceRole::Primary)
                    && !obligation.evidence_requirements.primary_source_required
                {
                    return Err(InquiryError::InvalidResearchPlan {
                        reason: format!(
                            "source coverage edge `{}` -> `{}` declares an unrequested primary role",
                            binding.source_id, binding.obligation_id
                        ),
                    });
                }
                if roles.contains(&SourceEvidenceRole::Independent)
                    && !obligation
                        .evidence_requirements
                        .independent_corroboration_required
                {
                    return Err(InquiryError::InvalidResearchPlan {
                        reason: format!(
                            "source coverage edge `{}` -> `{}` declares an unrequested independent role",
                            binding.source_id, binding.obligation_id
                        ),
                    });
                }
            }
            ensure_limit(
                "evidence diagnostics",
                evidence.diagnostics.len(),
                limits.max_citation_ids_per_section,
            )?;
            ensure_unique_ids(
                evidence
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.id.as_str()),
                "evidence diagnostic",
            )?;
            let accepted_diagnostic_ids = state
                .evidence_catalog
                .values()
                .flat_map(|accepted| accepted.diagnostics.iter())
                .map(|diagnostic| diagnostic.id.as_str())
                .collect::<HashSet<_>>();
            for diagnostic in &evidence.diagnostics {
                ensure_string(
                    &diagnostic.id,
                    "evidence diagnostic id",
                    limits.max_identifier_chars,
                )?;
                ensure_string(
                    &diagnostic.detail,
                    "evidence diagnostic detail",
                    limits.max_text_chars,
                )?;
                if accepted_diagnostic_ids.contains(diagnostic.id.as_str()) {
                    return Err(InquiryError::DuplicateId {
                        resource: "evidence diagnostic",
                        id: diagnostic.id.clone(),
                    });
                }
            }
            if let Some(accepted) = state.evidence_catalog.get(&evidence.evidence_id) {
                return if accepted == evidence {
                    Err(InquiryError::DuplicateId {
                        resource: "evidence",
                        id: evidence.evidence_id.clone(),
                    })
                } else {
                    Err(InquiryError::ConflictingEvidence {
                        id: evidence.evidence_id.clone(),
                    })
                };
            }
            next.claim_catalog
                .extend(evidence.claim_ids.iter().cloned());
            next.source_catalog
                .extend(evidence.source_ids.iter().cloned());
            next.evidence_catalog
                .insert(evidence.evidence_id.clone(), evidence.clone());
        }
        InquiryEvent::QuestionAnswered {
            question_id,
            answer,
            evidence_ids,
        } => {
            require_phase(state, event, &[InquiryPhase::Questioning])?;
            apply_answer_resolution(
                state,
                &mut next,
                question_id,
                answer,
                None,
                evidence_ids,
                limits,
            )?;
            advance_after_question_resolution(&mut next);
        }
        InquiryEvent::QuestionPartiallyAnswered {
            question_id,
            answer,
            limitation,
            evidence_ids,
        } => {
            require_phase(state, event, &[InquiryPhase::Questioning])?;
            ensure_string(
                limitation,
                "question answer limitation",
                limits.max_text_chars,
            )?;
            apply_answer_resolution(
                state,
                &mut next,
                question_id,
                answer,
                Some(limitation),
                evidence_ids,
                limits,
            )?;
            advance_after_question_resolution(&mut next);
        }
        InquiryEvent::QuestionDeferred {
            question_id,
            reason,
        } => {
            // Historical follow-up waves left questions queued with a reason.
            // The active closed-evidence review never emits this event.
            require_phase(state, event, &[InquiryPhase::Questioning])?;
            ensure_string(question_id, "question id", limits.max_identifier_chars)?;
            ensure_string(reason, "question defer reason", limits.max_text_chars)?;
            let question = queued_question_mut(&mut next, question_id)?;
            question.bound_reason = Some(reason.clone());
        }
        InquiryEvent::QuestionBounded {
            question_id,
            reason,
        } => {
            require_phase(state, event, &[InquiryPhase::Questioning])?;
            ensure_string(question_id, "question id", limits.max_identifier_chars)?;
            ensure_string(reason, "question bound reason", limits.max_text_chars)?;
            let question = queued_question_mut(&mut next, question_id)?;
            question.status = QuestionStatus::Bounded;
            question.bound_reason = Some(reason.clone());
            advance_after_question_resolution(&mut next);
        }
        InquiryEvent::ResearchContractAssessed { assessment } => {
            require_phase(state, event, &[InquiryPhase::Outlining])?;
            if state.contract_assessment.is_some() {
                return Err(invalid_transition(state, event));
            }
            validate_obligation_coverage(state)?;
            validate_research_contract_assessment(state, assessment).map_err(|error| {
                InquiryError::InvalidResearchPlan {
                    reason: error.to_string(),
                }
            })?;
            next.contract_assessment = Some(assessment.clone());
        }
        InquiryEvent::OutlineCommitted { outline } => {
            require_phase(state, event, &[InquiryPhase::Outlining])?;
            let unresolved = unresolved_questions(state);
            if unresolved > 0 {
                return Err(InquiryError::UnresolvedQuestions { count: unresolved });
            }
            let material_questions = state
                .questions
                .iter()
                .filter(|question| question.material)
                .count();
            if material_questions == 0 {
                return Err(InquiryError::InvalidOutline {
                    reason:
                        "at least one material research question must be answered before outlining"
                            .to_string(),
                });
            }
            let bounded_material = state
                .questions
                .iter()
                .filter(|question| question.material && question.status != QuestionStatus::Answered)
                .count();
            if state.obligations.is_empty() && bounded_material > 0 {
                return Err(InquiryError::InvalidOutline {
                    reason: format!(
                        "every material research question must be answered before outlining; {bounded_material} remain bounded"
                    ),
                });
            }
            if !material_evidence_floor(state) {
                return Err(InquiryError::InvalidOutline {
                    reason: "every material research obligation requires a traceable answered material question before outlining"
                        .to_string(),
                });
            }
            validate_obligation_coverage(state)?;
            if !state.obligations.is_empty()
                && !matches!(
                    research_contract_outcome(state),
                    Some(ResearchContractOutcome::Satisfied | ResearchContractOutcome::Qualified)
                )
            {
                return Err(InquiryError::InvalidOutline {
                    reason:
                        "research completion criteria and stop conditions have not been satisfied"
                            .to_string(),
                });
            }
            validate_outline_limits(outline, limits)?;
            validate_research_outline(outline, &state.outline_validation_context()).map_err(
                |error| InquiryError::InvalidOutline {
                    reason: error.to_string(),
                },
            )?;
            next.outline = Some(outline.clone());
            next.phase = InquiryPhase::Drafting;
        }
        InquiryEvent::SectionDrafted {
            section_id,
            content,
            citation_ids,
        } => {
            require_phase(
                state,
                event,
                &[InquiryPhase::Drafting, InquiryPhase::Auditing],
            )?;
            ensure_string(
                section_id,
                "outline section id",
                limits.max_identifier_chars,
            )?;
            ensure_string(content, "section content", limits.max_section_chars)?;
            let outline = state
                .outline
                .as_ref()
                .ok_or_else(|| invalid_transition(state, event))?;
            let section = outline
                .sections
                .iter()
                .find(|section| section.id == *section_id)
                .ok_or_else(|| InquiryError::UnknownId {
                    resource: "outline section",
                    id: section_id.clone(),
                })?;
            if let Some(revision) = state.active_section_revision() {
                if !revision.section_ids.contains(section_id) {
                    return Err(InquiryError::InvalidSectionRevision {
                        reason: format!(
                            "round {} cannot draft untargeted section `{section_id}`",
                            revision.round
                        ),
                    });
                }
            }
            validate_section_citations(section, citation_ids, limits)?;
            next.drafts.insert(
                section_id.clone(),
                SectionDraft {
                    section_id: section_id.clone(),
                    content: content.clone(),
                    citation_ids: citation_ids.clone(),
                },
            );
            let draft_chars = next
                .drafts
                .values()
                .map(|draft| char_count(&draft.content))
                .sum();
            ensure_limit(
                "total draft chars",
                draft_chars,
                limits.max_total_draft_chars,
            )?;
            next.audit = None;
            if let Some(revision) = next
                .section_revisions
                .last_mut()
                .filter(|revision| !revision.committed)
            {
                if !revision.drafted_section_ids.contains(section_id) {
                    revision.drafted_section_ids.push(section_id.clone());
                }
            }
            next.phase = if next.drafts.len() == outline.sections.len() {
                InquiryPhase::Auditing
            } else {
                InquiryPhase::Drafting
            };
        }
        InquiryEvent::SectionRevisionStarted {
            round,
            section_ids,
            input_digest,
        } => {
            require_phase(
                state,
                event,
                &[InquiryPhase::Drafting, InquiryPhase::Auditing],
            )?;
            if let Some(active) = state.active_section_revision() {
                return Err(InquiryError::InvalidSectionRevision {
                    reason: format!(
                        "round {} is still active for input `{}`",
                        active.round, active.input_digest
                    ),
                });
            }
            let expected_round = state.section_revisions.len().saturating_add(1);
            if *round != expected_round {
                return Err(InquiryError::InvalidSectionRevision {
                    reason: format!("expected round {expected_round}, received {round}"),
                });
            }
            ensure_limit(
                "section revision rounds",
                *round,
                limits.max_section_revision_rounds,
            )?;
            ensure_nonempty(section_ids, "section revision targets")?;
            ensure_limit(
                "section revision targets",
                section_ids.len(),
                limits.max_outline_sections,
            )?;
            ensure_unique_ids(
                section_ids.iter().map(String::as_str),
                "section revision target",
            )?;
            ensure_strings(
                section_ids,
                "section revision target",
                limits.max_identifier_chars,
            )?;
            ensure_string(
                input_digest,
                "section revision input digest",
                limits.max_identifier_chars,
            )?;
            let outline = state
                .outline
                .as_ref()
                .ok_or_else(|| invalid_transition(state, event))?;
            for section_id in section_ids {
                if !outline
                    .sections
                    .iter()
                    .any(|section| section.id == *section_id)
                {
                    return Err(InquiryError::UnknownId {
                        resource: "outline section",
                        id: section_id.clone(),
                    });
                }
            }
            next.section_revisions.push(SectionRevision {
                round: *round,
                section_ids: section_ids.clone(),
                input_digest: input_digest.clone(),
                drafted_section_ids: Vec::new(),
                committed: false,
            });
        }
        InquiryEvent::SectionRevisionCommitted {
            round,
            input_digest,
        } => {
            require_phase(
                state,
                event,
                &[InquiryPhase::Drafting, InquiryPhase::Auditing],
            )?;
            let active = state.active_section_revision().ok_or_else(|| {
                InquiryError::InvalidSectionRevision {
                    reason: "no section revision is active".to_string(),
                }
            })?;
            if active.round != *round || active.input_digest != *input_digest {
                return Err(InquiryError::InvalidSectionRevision {
                    reason: format!(
                        "active round {} input `{}` does not match round {round} input `{input_digest}`",
                        active.round, active.input_digest
                    ),
                });
            }
            let missing = active
                .section_ids
                .iter()
                .filter(|section_id| !active.drafted_section_ids.contains(section_id))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(InquiryError::InvalidSectionRevision {
                    reason: format!(
                        "round {round} has no replacement draft for {}",
                        missing.join(", ")
                    ),
                });
            }
            if let Some(revision) = next.section_revisions.last_mut() {
                revision.committed = true;
            }
        }
        InquiryEvent::AuditCompleted { passed, issues } => {
            require_phase(state, event, &[InquiryPhase::Auditing])?;
            if let Some(active) = state.active_section_revision() {
                return Err(InquiryError::InvalidSectionRevision {
                    reason: format!(
                        "round {} must be committed before report audit",
                        active.round
                    ),
                });
            }
            let missing = state.outline.as_ref().map_or(0, |outline| {
                outline.sections.len().saturating_sub(state.drafts.len())
            });
            if missing > 0 {
                return Err(InquiryError::IncompleteSections { count: missing });
            }
            ensure_limit("audit issues", issues.len(), limits.max_audit_issues)?;
            ensure_strings(issues, "audit issue", limits.max_text_chars)?;
            ensure_limit(
                "audit attempts",
                state.audit_attempts.saturating_add(1),
                limits.max_audit_attempts,
            )?;
            next.audit = Some(InquiryAudit {
                passed: *passed,
                issues: issues.clone(),
            });
            next.audit_attempts += 1;
            next.phase = if *passed {
                InquiryPhase::Completed
            } else {
                InquiryPhase::Drafting
            };
        }
        InquiryEvent::BudgetExhausted { reason } => {
            ensure_string(reason, "budget exhaustion reason", limits.max_text_chars)?;
            next.budget_exhausted_reason = Some(reason.clone());
            next.phase = InquiryPhase::Exhausted;
        }
    }
    next.events_applied += 1;
    Ok(next)
}

fn apply_answer_resolution(
    state: &InquiryState,
    next: &mut InquiryState,
    question_id: &str,
    answer: &str,
    limitation: Option<&str>,
    evidence_ids: &[String],
    limits: &InquiryLimits,
) -> Result<(), InquiryError> {
    ensure_string(question_id, "question id", limits.max_identifier_chars)?;
    ensure_string(answer, "question answer", limits.max_answer_chars)?;
    ensure_nonempty(evidence_ids, "answer evidence ids")?;
    ensure_limit(
        "answer evidence ids",
        evidence_ids.len(),
        limits.max_evidence_ids_per_answer,
    )?;
    ensure_unique_ids(evidence_ids.iter().map(String::as_str), "evidence")?;
    ensure_strings(evidence_ids, "evidence id", limits.max_identifier_chars)?;
    for evidence_id in evidence_ids {
        if !state.evidence_catalog.contains_key(evidence_id) {
            return Err(InquiryError::UnknownId {
                resource: "accepted evidence",
                id: evidence_id.clone(),
            });
        }
    }
    let question = queued_question_mut(next, question_id)?;
    question.status = QuestionStatus::Answered;
    question.answer = Some(answer.to_string());
    question.bound_reason = limitation.map(str::to_string);
    question.evidence_ids = evidence_ids.to_vec();
    let answer_chars = next
        .questions
        .iter()
        .filter_map(|question| question.answer.as_deref())
        .map(char_count)
        .sum();
    ensure_limit(
        "total answer chars",
        answer_chars,
        limits.max_total_answer_chars,
    )
}

fn validate_obligation_coverage(state: &InquiryState) -> Result<(), InquiryError> {
    if state.obligations.is_empty() {
        return Ok(());
    }
    for obligation in &state.obligations {
        let linked = state
            .questions
            .iter()
            .filter(|question| question.obligation_ids.contains(&obligation.id))
            .collect::<Vec<_>>();
        if linked.is_empty() {
            return Err(InquiryError::InvalidOutline {
                reason: format!(
                    "research obligation `{}` has no traceable question path",
                    obligation.id
                ),
            });
        }
        for criterion_index in 0..obligation.completion_criteria.len() {
            if !linked.iter().any(|question| {
                question.completion_criterion_indexes.is_empty()
                    || question
                        .completion_criterion_indexes
                        .contains(&criterion_index)
            }) {
                return Err(InquiryError::InvalidOutline {
                    reason: format!(
                        "research obligation `{}` completion criterion {criterion_index} has no traceable question path",
                        obligation.id
                    ),
                });
            }
        }
        if obligation.material {
            let material = linked
                .iter()
                .filter(|question| question.material)
                .collect::<Vec<_>>();
            if material.is_empty() {
                return Err(InquiryError::InvalidOutline {
                    reason: format!(
                        "material research obligation `{}` has no material question",
                        obligation.id
                    ),
                });
            }
            let answered = material
                .iter()
                .filter(|question| {
                    question.status == QuestionStatus::Answered && !question.evidence_ids.is_empty()
                })
                .count();
            if answered == 0 {
                return Err(InquiryError::InvalidOutline {
                    reason: format!(
                        "material research obligation `{}` has no traceable answered material question",
                        obligation.id
                    ),
                });
            }
        }
    }
    Ok(())
}

fn queued_question_mut<'a>(
    state: &'a mut InquiryState,
    id: &str,
) -> Result<&'a mut super::Question, InquiryError> {
    let question = state
        .questions
        .iter_mut()
        .find(|question| question.id == id)
        .ok_or_else(|| InquiryError::UnknownId {
            resource: "question",
            id: id.to_string(),
        })?;
    if question.status != QuestionStatus::Queued {
        return Err(InquiryError::InvalidQuestionState {
            id: id.to_string(),
            status: question.status,
        });
    }
    Ok(question)
}

fn advance_after_question_resolution(state: &mut InquiryState) {
    if !state.questions.is_empty() && unresolved_questions(state) == 0 {
        state.phase = InquiryPhase::Outlining;
    }
}

fn unresolved_questions(state: &InquiryState) -> usize {
    state
        .questions
        .iter()
        .filter(|question| question.status == QuestionStatus::Queued)
        .count()
}

fn require_phase(
    state: &InquiryState,
    event: &InquiryEvent,
    allowed: &[InquiryPhase],
) -> Result<(), InquiryError> {
    if allowed.contains(&state.phase) {
        Ok(())
    } else {
        Err(invalid_transition(state, event))
    }
}

fn invalid_transition(state: &InquiryState, event: &InquiryEvent) -> InquiryError {
    InquiryError::InvalidTransition {
        phase: state.phase,
        event: event.name(),
    }
}

fn char_count(value: &str) -> usize {
    value.chars().count()
}
