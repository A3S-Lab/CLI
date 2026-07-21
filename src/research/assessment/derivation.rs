/// Derive one conservative, typed contract assessment from the already
/// completed semantic question review.
///
/// The semantic model decides only whether each structurally criterion-linked
/// question is answered or bounded. This reducer then maps those decisions
/// onto criteria, source-quality requirements, stop conditions, and evidence
/// diagnostics without another model call. Source authority and independence
/// are satisfied only through typed, host-validated source-role edges.
pub fn derive_research_contract_assessment(
    state: &InquiryState,
) -> Result<ResearchContractAssessment, ResearchContractAssessmentError> {
    validate_assessment_input_state(state)?;

    let obligations = state
        .obligations
        .iter()
        .map(|obligation| derive_obligation_assessment(state, obligation))
        .collect::<Vec<_>>();
    let diagnostics = evidence_diagnostic_catalog(state)
        .into_iter()
        .map(|(diagnostic, parent_evidence_id)| {
            derive_diagnostic_assessment(state, diagnostic, parent_evidence_id)
        })
        .collect::<Vec<_>>();
    let all_answered_evidence_ids = state
        .questions
        .iter()
        .filter(|question| question.status == QuestionStatus::Answered)
        .flat_map(|question| question.evidence_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let partial = state.questions.iter().any(|question| {
        question.status == QuestionStatus::Bounded
            || (question.status == QuestionStatus::Answered && question.bound_reason.is_some())
    }) || obligations.iter().any(obligation_assessment_is_partial)
        || diagnostics
            .iter()
            .any(|diagnostic| diagnostic.disposition == DiagnosticDisposition::Bounded);
    let stop_status = if partial {
        ContractAssessmentStatus::Bounded
    } else {
        ContractAssessmentStatus::Satisfied
    };
    let stop_rationale = if partial {
        "The material evidence floor is met and every question is terminal, but one or more criterion, source-quality requirement, supporting obligation, or evidence diagnostic remains explicitly bounded or uncovered."
    } else {
        "Every criterion-linked question was answered with traceable accepted evidence and no bounded contract edge remains."
    };
    let stop_conditions = state
        .stop_conditions
        .iter()
        .enumerate()
        .map(|(condition_index, _)| StopConditionAssessment {
            condition_index,
            status: stop_status,
            rationale: stop_rationale.to_string(),
            evidence_ids: all_answered_evidence_ids.clone(),
        })
        .collect::<Vec<_>>();
    let assessment = ResearchContractAssessment {
        obligations,
        stop_conditions,
        diagnostics,
    };
    validate_research_contract_assessment(state, &assessment)?;
    Ok(assessment)
}

fn derive_obligation_assessment(
    state: &InquiryState,
    obligation: &ResearchObligation,
) -> ResearchObligationAssessment {
    let obligation_evidence_ids = obligation_evidence_ids(state, &obligation.id)
        .into_iter()
        .collect::<Vec<_>>();
    let criteria = obligation
        .completion_criteria
        .iter()
        .enumerate()
        .map(|(criterion_index, _)| {
            derive_completion_criterion(
                state,
                obligation,
                criterion_index,
                &obligation_evidence_ids,
            )
        })
        .collect::<Vec<_>>();
    ResearchObligationAssessment {
        obligation_id: obligation.id.clone(),
        criteria,
        primary_source: derive_source_quality_requirement(
            state,
            &obligation.id,
            obligation.evidence_requirements.primary_source_required,
            "primary-source",
            SourceEvidenceRole::Primary,
            1,
            &obligation_evidence_ids,
        ),
        independent_corroboration: derive_source_quality_requirement(
            state,
            &obligation.id,
            obligation
                .evidence_requirements
                .independent_corroboration_required,
            "independent-corroboration",
            SourceEvidenceRole::Independent,
            2,
            &obligation_evidence_ids,
        ),
    }
}

fn derive_completion_criterion(
    state: &InquiryState,
    obligation: &ResearchObligation,
    criterion_index: usize,
    obligation_evidence_ids: &[String],
) -> CompletionCriterionAssessment {
    let questions = state
        .questions
        .iter()
        .filter(|question| question.obligation_ids.contains(&obligation.id))
        .filter(|question| {
            question.completion_criterion_indexes.is_empty()
                || question
                    .completion_criterion_indexes
                    .contains(&criterion_index)
        })
        .collect::<Vec<_>>();
    let criterion_evidence_ids = questions
        .iter()
        .filter(|question| question.status == QuestionStatus::Answered)
        .flat_map(|question| question.evidence_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let answered = questions
        .iter()
        .filter(|question| {
            question.status == QuestionStatus::Answered && question.bound_reason.is_none()
        })
        .count();
    let partially_answered = questions
        .iter()
        .filter(|question| {
            question.status == QuestionStatus::Answered && question.bound_reason.is_some()
        })
        .count();
    let bounded = questions
        .iter()
        .filter(|question| question.status == QuestionStatus::Bounded)
        .count();

    if !questions.is_empty()
        && partially_answered == 0
        && bounded == 0
        && !criterion_evidence_ids.is_empty()
    {
        return CompletionCriterionAssessment {
            criterion_index,
            status: ContractAssessmentStatus::Satisfied,
            rationale: format!(
                "All {answered} structurally linked question(s) were answered with traceable accepted evidence."
            ),
            evidence_ids: criterion_evidence_ids,
        };
    }
    if !obligation_evidence_ids.is_empty() {
        return CompletionCriterionAssessment {
            criterion_index,
            status: ContractAssessmentStatus::Bounded,
            rationale: format!(
                "{answered} structurally linked question(s) were fully answered, {partially_answered} retained a traceable partial answer, and {bounded} were bounded; the accepted evidence provides only qualified support."
            ),
            evidence_ids: obligation_evidence_ids.to_vec(),
        };
    }
    CompletionCriterionAssessment {
        criterion_index,
        status: ContractAssessmentStatus::Uncovered,
        rationale:
            "No structurally linked question produced traceable accepted evidence for this criterion."
                .to_string(),
        evidence_ids: Vec::new(),
    }
}

fn derive_source_quality_requirement(
    state: &InquiryState,
    obligation_id: &str,
    required: bool,
    requirement: &str,
    role: SourceEvidenceRole,
    satisfied_source_minimum: usize,
    evidence_ids: &[String],
) -> Option<EvidenceRequirementAssessment> {
    if !required {
        return None;
    }
    let source_ids = source_ids_for_evidence(state, evidence_ids)
        .into_iter()
        .collect::<Vec<_>>();
    if evidence_ids.is_empty() || source_ids.is_empty() {
        return Some(EvidenceRequirementAssessment {
            status: ContractAssessmentStatus::Uncovered,
            rationale: format!(
                "The accepted evidence graph contains no traceable source path for the declared {requirement} requirement."
            ),
            evidence_ids: Vec::new(),
            source_ids: Vec::new(),
        });
    }
    let (role_evidence_ids, role_source_ids) =
        source_coverage_for_role(state, obligation_id, evidence_ids, role);
    let role_evidence_ids = role_evidence_ids.into_iter().collect::<Vec<_>>();
    let role_source_ids = role_source_ids.into_iter().collect::<Vec<_>>();
    if role_source_ids.len() >= satisfied_source_minimum {
        return Some(EvidenceRequirementAssessment {
            status: ContractAssessmentStatus::Satisfied,
            rationale: format!(
                "The accepted graph contains host-validated {requirement} role edges for {} distinct answer-path source(s), meeting the required minimum of {satisfied_source_minimum}.",
                role_source_ids.len()
            ),
            evidence_ids: role_evidence_ids,
            source_ids: role_source_ids,
        });
    }
    if !role_source_ids.is_empty() {
        return Some(EvidenceRequirementAssessment {
            status: ContractAssessmentStatus::Bounded,
            rationale: format!(
                "The accepted graph contains host-validated {requirement} role edges for {} distinct answer-path source(s), below the required minimum of {satisfied_source_minimum}.",
                role_source_ids.len()
            ),
            evidence_ids: role_evidence_ids,
            source_ids: role_source_ids,
        });
    }
    Some(EvidenceRequirementAssessment {
        status: ContractAssessmentStatus::Bounded,
        rationale: format!(
            "Traceable evidence and source identities exist, but the accepted graph does not encode a host-verifiable {requirement} role; the host will not infer it from names, URLs, or keywords."
        ),
        evidence_ids: evidence_ids.to_vec(),
        source_ids,
    })
}

fn derive_diagnostic_assessment(
    state: &InquiryState,
    diagnostic: &EvidenceDiagnostic,
    parent_evidence_id: &str,
) -> EvidenceDiagnosticAssessment {
    let obligation_ids = evidence_obligation_ids(state, parent_evidence_id)
        .into_iter()
        .collect::<Vec<_>>();
    if obligation_ids.is_empty() {
        return EvidenceDiagnosticAssessment {
            diagnostic_id: diagnostic.id.clone(),
            disposition: DiagnosticDisposition::Irrelevant,
            obligation_ids: Vec::new(),
            rationale:
                "The diagnostic is not on any answered obligation-to-evidence path in the closed graph."
                    .to_string(),
            evidence_ids: Vec::new(),
        };
    }
    EvidenceDiagnosticAssessment {
        diagnostic_id: diagnostic.id.clone(),
        disposition: DiagnosticDisposition::Bounded,
        obligation_ids,
        rationale: "The diagnostic remains on an answered evidence path and no separately typed resolution edge exists, so the host preserves it as a report limitation."
            .to_string(),
        evidence_ids: vec![parent_evidence_id.to_string()],
    }
}

fn obligation_assessment_is_partial(assessment: &ResearchObligationAssessment) -> bool {
    assessment
        .criteria
        .iter()
        .any(|criterion| criterion.status != ContractAssessmentStatus::Satisfied)
        || assessment
            .primary_source
            .as_ref()
            .is_some_and(|requirement| requirement.status != ContractAssessmentStatus::Satisfied)
        || assessment
            .independent_corroboration
            .as_ref()
            .is_some_and(|requirement| requirement.status != ContractAssessmentStatus::Satisfied)
}
