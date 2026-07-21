fn evidence_requirement_assessment_schema(
    evidence_ids: &[String],
    source_ids: &[String],
    satisfied_source_minimum: usize,
) -> serde_json::Value {
    let mut variants = Vec::new();
    if !evidence_ids.is_empty() && source_ids.len() >= satisfied_source_minimum {
        variants.push(evidence_requirement_status_schema(
            "satisfied",
            closed_id_array_schema_with_minimum(evidence_ids, 1),
            closed_id_array_schema_with_minimum(source_ids, satisfied_source_minimum),
        ));
    }
    if !evidence_ids.is_empty() && !source_ids.is_empty() {
        variants.push(evidence_requirement_status_schema(
            "bounded",
            closed_id_array_schema_with_minimum(evidence_ids, 1),
            closed_id_array_schema_with_minimum(source_ids, 1),
        ));
    }
    variants.push(evidence_requirement_status_schema(
        "uncovered",
        empty_id_array_schema(),
        empty_id_array_schema(),
    ));
    serde_json::json!({ "oneOf": variants })
}

fn evidence_requirement_status_schema(
    status: &str,
    evidence_ids: serde_json::Value,
    source_ids: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "status": { "type": "string", "enum": [status] },
            "rationale": { "type": "string", "minLength": 1, "maxLength": MAX_RATIONALE_CHARS },
            "evidence_ids": evidence_ids,
            "source_ids": source_ids,
        },
        "required": ["status", "rationale", "evidence_ids", "source_ids"]
    })
}

fn criterion_assessment_schema(
    criterion_index: usize,
    evidence_ids: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "criterion_index": { "type": "integer", "enum": [criterion_index] },
            "status": { "type": "string", "enum": ["satisfied", "bounded", "uncovered"] },
            "rationale": { "type": "string", "minLength": 1, "maxLength": MAX_RATIONALE_CHARS },
            "evidence_ids": closed_id_array_schema(evidence_ids),
        },
        "required": ["criterion_index", "status", "rationale", "evidence_ids"]
    })
}

fn stop_condition_schema(condition_index: usize, evidence_ids: &[String]) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "condition_index": { "type": "integer", "enum": [condition_index] },
            "status": { "type": "string", "enum": ["satisfied", "bounded", "uncovered"] },
            "rationale": { "type": "string", "minLength": 1, "maxLength": MAX_RATIONALE_CHARS },
            "evidence_ids": closed_id_array_schema(evidence_ids),
        },
        "required": ["condition_index", "status", "rationale", "evidence_ids"]
    })
}

fn diagnostic_assessment_schema(
    state: &InquiryState,
    diagnostic: &EvidenceDiagnostic,
    parent_evidence_id: &str,
) -> serde_json::Value {
    let parent_obligation_ids = evidence_obligation_ids(state, parent_evidence_id)
        .into_iter()
        .collect::<Vec<_>>();
    let resolving_evidence_ids = parent_obligation_ids
        .iter()
        .flat_map(|obligation_id| obligation_evidence_ids(state, obligation_id))
        .filter(|evidence_id| evidence_id != parent_evidence_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let exact_parent_obligations = exact_closed_id_array_schema(&parent_obligation_ids);
    let mut variants = Vec::new();
    if !parent_obligation_ids.is_empty() && !resolving_evidence_ids.is_empty() {
        variants.push(diagnostic_disposition_schema(
            diagnostic,
            "resolved",
            exact_parent_obligations.clone(),
            nonempty_closed_id_array_schema(&resolving_evidence_ids),
        ));
    }
    if !parent_obligation_ids.is_empty() {
        variants.push(diagnostic_disposition_schema(
            diagnostic,
            "bounded",
            exact_parent_obligations,
            exact_closed_id_array_schema(&[parent_evidence_id.to_string()]),
        ));
    }
    variants.push(diagnostic_disposition_schema(
        diagnostic,
        "irrelevant",
        empty_id_array_schema(),
        empty_id_array_schema(),
    ));
    serde_json::json!({ "oneOf": variants })
}

fn diagnostic_disposition_schema(
    diagnostic: &EvidenceDiagnostic,
    disposition: &str,
    obligation_ids: serde_json::Value,
    evidence_ids: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "diagnostic_id": { "type": "string", "enum": [diagnostic.id] },
            "disposition": { "type": "string", "enum": [disposition] },
            "obligation_ids": obligation_ids,
            "rationale": { "type": "string", "minLength": 1, "maxLength": MAX_RATIONALE_CHARS },
            "evidence_ids": evidence_ids,
        },
        "required": ["diagnostic_id", "disposition", "obligation_ids", "rationale", "evidence_ids"]
    })
}

fn closed_id_array_schema(ids: &[String]) -> serde_json::Value {
    if ids.is_empty() {
        serde_json::json!({
            "type": "array",
            "maxItems": 0,
            "items": { "type": "string" }
        })
    } else {
        serde_json::json!({
            "type": "array",
            "maxItems": ids.len(),
            "uniqueItems": true,
            "items": { "type": "string", "enum": ids }
        })
    }
}

fn nonempty_closed_id_array_schema(ids: &[String]) -> serde_json::Value {
    closed_id_array_schema_with_minimum(ids, 1)
}

fn exact_closed_id_array_schema(ids: &[String]) -> serde_json::Value {
    let mut schema = closed_id_array_schema(ids);
    schema["minItems"] = serde_json::Value::from(ids.len());
    schema["maxItems"] = serde_json::Value::from(ids.len());
    schema
}

fn closed_id_array_schema_with_minimum(ids: &[String], minimum: usize) -> serde_json::Value {
    let mut schema = closed_id_array_schema(ids);
    schema["minItems"] = serde_json::Value::from(minimum);
    schema
}

fn empty_id_array_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "maxItems": 0,
        "items": { "type": "string" }
    })
}

fn closed_keyed_object_schema(
    entries: Vec<(String, serde_json::Value)>,
) -> Result<serde_json::Value, ResearchContractAssessmentError> {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::with_capacity(entries.len());
    for (key, schema) in entries {
        if properties.insert(key.clone(), schema).is_some() {
            return Err(ResearchContractAssessmentError::new(format!(
                "assessment schema contains duplicate keyed entry `{key}`"
            )));
        }
        required.push(serde_json::Value::String(key));
    }
    Ok(serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    }))
}

pub fn validate_research_contract_assessment(
    state: &InquiryState,
    assessment: &ResearchContractAssessment,
) -> Result<(), ResearchContractAssessmentError> {
    validate_assessment_input_state(state)?;
    if assessment.obligations.len() != state.obligations.len() {
        return Err(ResearchContractAssessmentError::new(format!(
            "expected {} obligation assessments; got {}",
            state.obligations.len(),
            assessment.obligations.len()
        )));
    }
    let by_obligation = unique_by_key(
        &assessment.obligations,
        |item| item.obligation_id.as_str(),
        "research obligation assessment",
    )?;
    for obligation in &state.obligations {
        let item = by_obligation.get(obligation.id.as_str()).ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "missing research obligation assessment `{}`",
                obligation.id
            ))
        })?;
        validate_obligation_assessment(state, obligation, item)?;
    }

    if assessment.stop_conditions.len() != state.stop_conditions.len() {
        return Err(ResearchContractAssessmentError::new(format!(
            "expected {} stop-condition assessments; got {}",
            state.stop_conditions.len(),
            assessment.stop_conditions.len()
        )));
    }
    let mut condition_indexes = BTreeSet::new();
    for item in &assessment.stop_conditions {
        if item.condition_index >= state.stop_conditions.len()
            || !condition_indexes.insert(item.condition_index)
        {
            return Err(ResearchContractAssessmentError::new(format!(
                "invalid or duplicate stop-condition index `{}`",
                item.condition_index
            )));
        }
        validate_status_evidence(
            "stop condition",
            item.status,
            &item.rationale,
            &item.evidence_ids,
            &state.evidence_catalog.keys().cloned().collect(),
        )?;
    }

    validate_diagnostic_assessments(state, &assessment.diagnostics)
}

fn validate_obligation_assessment(
    state: &InquiryState,
    obligation: &ResearchObligation,
    assessment: &ResearchObligationAssessment,
) -> Result<(), ResearchContractAssessmentError> {
    if assessment.criteria.len() != obligation.completion_criteria.len() {
        return Err(ResearchContractAssessmentError::new(format!(
            "research obligation `{}` expected {} criterion assessments; got {}",
            obligation.id,
            obligation.completion_criteria.len(),
            assessment.criteria.len()
        )));
    }
    let allowed_evidence = obligation_evidence_ids(state, &obligation.id);
    let mut indexes = BTreeSet::new();
    for criterion in &assessment.criteria {
        if criterion.criterion_index >= obligation.completion_criteria.len()
            || !indexes.insert(criterion.criterion_index)
        {
            return Err(ResearchContractAssessmentError::new(format!(
                "research obligation `{}` has invalid or duplicate criterion index `{}`",
                obligation.id, criterion.criterion_index
            )));
        }
        validate_status_evidence(
            "completion criterion",
            criterion.status,
            &criterion.rationale,
            &criterion.evidence_ids,
            &allowed_evidence,
        )?;
    }
    validate_declared_evidence_requirement(
        state,
        obligation,
        "primary source",
        obligation.evidence_requirements.primary_source_required,
        assessment.primary_source.as_ref(),
        SourceEvidenceRole::Primary,
        1,
        &allowed_evidence,
    )?;
    validate_declared_evidence_requirement(
        state,
        obligation,
        "independent corroboration",
        obligation
            .evidence_requirements
            .independent_corroboration_required,
        assessment.independent_corroboration.as_ref(),
        SourceEvidenceRole::Independent,
        2,
        &allowed_evidence,
    )?;
    Ok(())
}

fn validate_declared_evidence_requirement(
    state: &InquiryState,
    obligation: &ResearchObligation,
    resource: &str,
    required: bool,
    assessment: Option<&EvidenceRequirementAssessment>,
    required_role: SourceEvidenceRole,
    satisfied_source_minimum: usize,
    allowed_evidence_ids: &BTreeSet<String>,
) -> Result<(), ResearchContractAssessmentError> {
    let Some(assessment) = assessment else {
        if required {
            return Err(ResearchContractAssessmentError::new(format!(
                "research obligation `{}` omitted its declared {resource} assessment",
                obligation.id
            )));
        }
        return Ok(());
    };
    if !required {
        return Err(ResearchContractAssessmentError::new(format!(
            "research obligation `{}` assessed undeclared {resource}",
            obligation.id
        )));
    }

    validate_status_evidence(
        resource,
        assessment.status,
        &assessment.rationale,
        &assessment.evidence_ids,
        allowed_evidence_ids,
    )?;
    let allowed_source_ids = source_ids_for_evidence(
        state,
        &allowed_evidence_ids.iter().cloned().collect::<Vec<_>>(),
    );
    let cited_source_ids = source_ids_for_evidence(state, &assessment.evidence_ids);
    let mut unique_source_ids = BTreeSet::new();
    for source_id in &assessment.source_ids {
        if !allowed_source_ids.contains(source_id) {
            return Err(ResearchContractAssessmentError::new(format!(
                "{resource} assessment references source `{source_id}` outside obligation `{}`",
                obligation.id
            )));
        }
        if !cited_source_ids.contains(source_id) {
            return Err(ResearchContractAssessmentError::new(format!(
                "{resource} assessment source `{source_id}` does not belong to its cited evidence"
            )));
        }
        if !unique_source_ids.insert(source_id) {
            return Err(ResearchContractAssessmentError::new(format!(
                "{resource} assessment repeats source `{source_id}`"
            )));
        }
    }

    match assessment.status {
        ContractAssessmentStatus::Satisfied => {
            let (role_evidence_ids, role_source_ids) = source_coverage_for_role(
                state,
                &obligation.id,
                &assessment.evidence_ids,
                required_role,
            );
            if unique_source_ids.len() < satisfied_source_minimum {
                return Err(ResearchContractAssessmentError::new(format!(
                "satisfied {resource} assessment requires at least {satisfied_source_minimum} distinct traceable source(s)"
                )));
            }
            if assessment
                .evidence_ids
                .iter()
                .any(|evidence_id| !role_evidence_ids.contains(evidence_id))
                || assessment
                    .source_ids
                    .iter()
                    .any(|source_id| !role_source_ids.contains(source_id))
            {
                return Err(ResearchContractAssessmentError::new(format!(
                    "satisfied {resource} assessment requires a typed source-role edge for every cited evidence and source"
                )));
            }
            Ok(())
        }
        ContractAssessmentStatus::Bounded
            if assessment.evidence_ids.is_empty() || unique_source_ids.is_empty() =>
        {
            Err(ResearchContractAssessmentError::new(format!(
                "bounded {resource} assessment requires partial traceable evidence and a source"
            )))
        }
        ContractAssessmentStatus::Uncovered if !unique_source_ids.is_empty() => {
            Err(ResearchContractAssessmentError::new(format!(
                "uncovered {resource} assessment cannot claim a source"
            )))
        }
        _ => Ok(()),
    }
}

fn validate_status_evidence(
    resource: &str,
    status: ContractAssessmentStatus,
    rationale: &str,
    evidence_ids: &[String],
    allowed_evidence_ids: &BTreeSet<String>,
) -> Result<(), ResearchContractAssessmentError> {
    validate_text("assessment rationale", rationale, MAX_RATIONALE_CHARS)?;
    let mut local = BTreeSet::new();
    for evidence_id in evidence_ids {
        if !allowed_evidence_ids.contains(evidence_id) {
            return Err(ResearchContractAssessmentError::new(format!(
                "{resource} assessment references evidence `{evidence_id}` outside its traceable question path"
            )));
        }
        if !local.insert(evidence_id) {
            return Err(ResearchContractAssessmentError::new(format!(
                "{resource} assessment repeats evidence `{evidence_id}`"
            )));
        }
    }
    if matches!(
        status,
        ContractAssessmentStatus::Satisfied | ContractAssessmentStatus::Bounded
    ) && evidence_ids.is_empty()
    {
        return Err(ResearchContractAssessmentError::new(format!(
            "{status:?} {resource} assessment requires traceable evidence"
        )));
    }
    if status == ContractAssessmentStatus::Uncovered && !evidence_ids.is_empty() {
        return Err(ResearchContractAssessmentError::new(format!(
            "uncovered {resource} assessment cannot claim supporting evidence"
        )));
    }
    Ok(())
}

fn validate_diagnostic_assessments(
    state: &InquiryState,
    assessments: &[EvidenceDiagnosticAssessment],
) -> Result<(), ResearchContractAssessmentError> {
    let diagnostics = evidence_diagnostic_catalog(state);
    if assessments.len() != diagnostics.len() {
        return Err(ResearchContractAssessmentError::new(format!(
            "expected {} evidence-diagnostic assessments; got {}",
            diagnostics.len(),
            assessments.len()
        )));
    }
    let allowed_obligations = state
        .obligations
        .iter()
        .map(|obligation| obligation.id.as_str())
        .collect::<BTreeSet<_>>();
    let allowed_evidence = state
        .evidence_catalog
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let diagnostic_evidence = diagnostics
        .iter()
        .map(|(diagnostic, evidence_id)| (diagnostic.id.as_str(), evidence_id.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for assessment in assessments {
        let parent_evidence_id = diagnostic_evidence
            .get(assessment.diagnostic_id.as_str())
            .ok_or_else(|| {
                ResearchContractAssessmentError::new(format!(
                    "unknown evidence diagnostic `{}`",
                    assessment.diagnostic_id
                ))
            })?;
        if !seen.insert(assessment.diagnostic_id.as_str()) {
            return Err(ResearchContractAssessmentError::new(format!(
                "duplicate evidence diagnostic assessment `{}`",
                assessment.diagnostic_id
            )));
        }
        validate_text(
            "diagnostic assessment rationale",
            &assessment.rationale,
            MAX_RATIONALE_CHARS,
        )?;
        let mut obligation_ids = BTreeSet::new();
        for obligation_id in &assessment.obligation_ids {
            if !allowed_obligations.contains(obligation_id.as_str()) {
                return Err(ResearchContractAssessmentError::new(format!(
                    "diagnostic `{}` references unknown research obligation `{obligation_id}`",
                    assessment.diagnostic_id
                )));
            }
            if !obligation_ids.insert(obligation_id.as_str()) {
                return Err(ResearchContractAssessmentError::new(format!(
                    "diagnostic `{}` repeats research obligation `{obligation_id}`",
                    assessment.diagnostic_id
                )));
            }
        }
        let mut evidence_ids = BTreeSet::new();
        for evidence_id in &assessment.evidence_ids {
            if !allowed_evidence.contains(evidence_id) || !evidence_ids.insert(evidence_id) {
                return Err(ResearchContractAssessmentError::new(format!(
                    "diagnostic `{}` has unknown or duplicate evidence `{evidence_id}`",
                    assessment.diagnostic_id
                )));
            }
        }
        let parent_obligation_ids = evidence_obligation_ids(state, parent_evidence_id);
        match assessment.disposition {
            DiagnosticDisposition::Irrelevant => {
                if !assessment.obligation_ids.is_empty() || !assessment.evidence_ids.is_empty() {
                    return Err(ResearchContractAssessmentError::new(format!(
                        "irrelevant diagnostic `{}` requires empty obligation_ids and evidence_ids",
                        assessment.diagnostic_id
                    )));
                }
            }
            DiagnosticDisposition::Resolved => {
                if assessment.obligation_ids.is_empty() {
                    return Err(ResearchContractAssessmentError::new(format!(
                        "resolved diagnostic `{}` requires a linked obligation",
                        assessment.diagnostic_id
                    )));
                }
                if assessment.obligation_ids.len() != parent_obligation_ids.len()
                    || assessment
                        .obligation_ids
                        .iter()
                        .any(|id| !parent_obligation_ids.contains(id))
                {
                    return Err(ResearchContractAssessmentError::new(format!(
                        "resolved diagnostic `{}` must reference exactly its parent evidence obligation path",
                        assessment.diagnostic_id
                    )));
                }
                let linked_evidence_ids = assessment
                    .obligation_ids
                    .iter()
                    .flat_map(|obligation_id| obligation_evidence_ids(state, obligation_id))
                    .collect::<BTreeSet<_>>();
                if assessment.evidence_ids.is_empty()
                    || assessment
                        .evidence_ids
                        .iter()
                        .any(|evidence_id| evidence_id.as_str() == *parent_evidence_id)
                {
                    return Err(ResearchContractAssessmentError::new(format!(
                        "resolved diagnostic `{}` requires different traceable evidence; its parent evidence cannot resolve itself",
                        assessment.diagnostic_id
                    )));
                }
                if assessment
                    .evidence_ids
                    .iter()
                    .any(|evidence_id| !linked_evidence_ids.contains(evidence_id.as_str()))
                {
                    return Err(ResearchContractAssessmentError::new(format!(
                        "resolved diagnostic `{}` cites resolving evidence outside its linked obligation path",
                        assessment.diagnostic_id
                    )));
                }
            }
            DiagnosticDisposition::Bounded => {
                if assessment.obligation_ids.is_empty() {
                    return Err(ResearchContractAssessmentError::new(format!(
                        "bounded diagnostic `{}` requires a linked obligation",
                        assessment.diagnostic_id
                    )));
                }
                if assessment.obligation_ids.len() != parent_obligation_ids.len()
                    || assessment
                        .obligation_ids
                        .iter()
                        .any(|id| !parent_obligation_ids.contains(id))
                {
                    return Err(ResearchContractAssessmentError::new(format!(
                        "bounded diagnostic `{}` must reference exactly its parent evidence obligation path",
                        assessment.diagnostic_id
                    )));
                }
                if assessment.evidence_ids.len() != 1
                    || assessment.evidence_ids[0] != *parent_evidence_id
                {
                    return Err(ResearchContractAssessmentError::new(format!(
                        "bounded diagnostic `{}` must cite only its traceable parent evidence",
                        assessment.diagnostic_id
                    )));
                }
            }
        }
    }
    Ok(())
}

pub fn research_contract_assessment_event(
    state: &InquiryState,
    mut assessment: ResearchContractAssessment,
) -> Result<InquiryEvent, ResearchContractAssessmentError> {
    normalize_irrelevant_diagnostic_links(state, &mut assessment);
    validate_research_contract_assessment(state, &assessment)?;
    Ok(InquiryEvent::ResearchContractAssessed { assessment })
}

fn normalize_irrelevant_diagnostic_links(
    state: &InquiryState,
    assessment: &mut ResearchContractAssessment,
) {
    let diagnostic_evidence = evidence_diagnostic_catalog(state)
        .into_iter()
        .map(|(diagnostic, evidence_id)| (diagnostic.id.as_str(), evidence_id.as_str()))
        .collect::<BTreeMap<_, _>>();
    for item in &mut assessment.diagnostics {
        if item.disposition != DiagnosticDisposition::Irrelevant {
            continue;
        }
        let Some(parent_evidence_id) = diagnostic_evidence.get(item.diagnostic_id.as_str()) else {
            continue;
        };
        if item.obligation_ids.is_empty() {
            item.evidence_ids.clear();
            continue;
        }
        let parent_obligation_ids = evidence_obligation_ids(state, parent_evidence_id);
        if parent_obligation_ids.is_empty() {
            item.obligation_ids.clear();
            item.evidence_ids.clear();
            continue;
        }
        item.disposition = DiagnosticDisposition::Bounded;
        item.obligation_ids = parent_obligation_ids.into_iter().collect();
        item.evidence_ids = vec![(*parent_evidence_id).to_string()];
        item.rationale = "The host conservatively bounded this diagnostic because the model classified it as irrelevant while linking it to an obligation, without traceable resolution evidence."
            .to_string();
    }
}

fn validate_assessment_input_state(
    state: &InquiryState,
) -> Result<(), ResearchContractAssessmentError> {
    if state.obligations.is_empty() || state.stop_conditions.is_empty() {
        return Err(ResearchContractAssessmentError::new(
            "research contract cannot be empty",
        ));
    }
    if state
        .questions
        .iter()
        .any(|question| question.status == super::QuestionStatus::Queued)
    {
        return Err(ResearchContractAssessmentError::new(
            "research questions must be answered or explicitly bounded before contract assessment",
        ));
    }
    if !material_evidence_floor(state) {
        return Err(ResearchContractAssessmentError::new(
            "every material research obligation requires a traceable answered material question before contract assessment",
        ));
    }
    let mut diagnostic_ids = BTreeSet::new();
    for diagnostic in state
        .evidence_catalog
        .values()
        .flat_map(|evidence| evidence.diagnostics.iter())
    {
        if !diagnostic_ids.insert(diagnostic.id.as_str()) {
            return Err(ResearchContractAssessmentError::new(format!(
                "evidence diagnostic id `{}` is not globally unique",
                diagnostic.id
            )));
        }
    }
    Ok(())
}

fn obligation_evidence_ids(state: &InquiryState, obligation_id: &str) -> BTreeSet<String> {
    state
        .questions
        .iter()
        .filter(|question| {
            question
                .obligation_ids
                .iter()
                .any(|candidate| candidate == obligation_id)
        })
        .flat_map(|question| question.evidence_ids.iter().cloned())
        .collect()
}

fn source_ids_for_evidence(state: &InquiryState, evidence_ids: &[String]) -> BTreeSet<String> {
    evidence_ids
        .iter()
        .filter_map(|evidence_id| state.evidence_catalog.get(evidence_id))
        .flat_map(|evidence| evidence.source_ids.iter().cloned())
        .collect()
}

fn source_coverage_for_role(
    state: &InquiryState,
    obligation_id: &str,
    evidence_ids: &[String],
    role: SourceEvidenceRole,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut covered_evidence_ids = BTreeSet::new();
    let mut covered_source_ids = BTreeSet::new();
    for evidence_id in evidence_ids {
        let Some(evidence) = state.evidence_catalog.get(evidence_id) else {
            continue;
        };
        for binding in &evidence.source_coverage {
            if binding.obligation_id == obligation_id
                && binding.roles.contains(&role)
                && evidence.source_ids.contains(&binding.source_id)
            {
                covered_evidence_ids.insert(evidence_id.clone());
                covered_source_ids.insert(binding.source_id.clone());
            }
        }
    }
    (covered_evidence_ids, covered_source_ids)
}

fn evidence_obligation_ids(state: &InquiryState, evidence_id: &str) -> BTreeSet<String> {
    state
        .questions
        .iter()
        .filter(|question| {
            question
                .evidence_ids
                .iter()
                .any(|candidate| candidate == evidence_id)
        })
        .flat_map(|question| question.obligation_ids.iter().cloned())
        .collect()
}

fn evidence_diagnostic_catalog(state: &InquiryState) -> Vec<(&EvidenceDiagnostic, &String)> {
    state
        .evidence_catalog
        .iter()
        .flat_map(|(evidence_id, evidence)| {
            evidence
                .diagnostics
                .iter()
                .map(move |diagnostic| (diagnostic, evidence_id))
        })
        .collect()
}

fn unique_by_key<'a, T, F>(
    values: &'a [T],
    mut key: F,
    resource: &str,
) -> Result<BTreeMap<&'a str, &'a T>, ResearchContractAssessmentError>
where
    F: FnMut(&'a T) -> &'a str,
{
    let mut result = BTreeMap::new();
    for value in values {
        let id = key(value);
        if result.insert(id, value).is_some() {
            return Err(ResearchContractAssessmentError::new(format!(
                "duplicate {resource} `{id}`"
            )));
        }
    }
    Ok(result)
}

fn validate_text(
    resource: &str,
    value: &str,
    maximum: usize,
) -> Result<(), ResearchContractAssessmentError> {
    if value.trim().is_empty() {
        Err(ResearchContractAssessmentError::new(format!(
            "{resource} cannot be blank"
        )))
    } else if value.chars().count() > maximum {
        Err(ResearchContractAssessmentError::new(format!(
            "{resource} exceeds {maximum} characters"
        )))
    } else {
        Ok(())
    }
}
