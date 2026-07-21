/// Decode the ID-keyed generation wire format into the stable vector-backed
/// domain/event representation. The keyed wire closes every host-owned set in
/// JSON Schema without changing persisted Inquiry events.
pub fn decode_research_contract_assessment(
    mut value: serde_json::Value,
) -> Result<ResearchContractAssessment, ResearchContractAssessmentError> {
    let root = value.as_object_mut().ok_or_else(|| {
        ResearchContractAssessmentError::new("research contract assessment must be an object")
    })?;

    let obligations = take_keyed_object(root, "obligations")?;
    let mut normalized_obligations = Vec::with_capacity(obligations.len());
    for (obligation_id, mut obligation) in sorted_entries(obligations) {
        require_string_discriminator(&obligation, "obligation_id", &obligation_id)?;
        let obligation_object = obligation.as_object_mut().ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "research obligation `{obligation_id}` must be an object"
            ))
        })?;
        let criteria = take_keyed_object(obligation_object, "criteria")?;
        obligation_object.insert(
            "criteria".to_string(),
            serde_json::Value::Array(normalize_indexed_entries(
                criteria,
                "criterion_index",
                "completion criterion",
            )?),
        );
        normalized_obligations.push(obligation);
    }
    root.insert(
        "obligations".to_string(),
        serde_json::Value::Array(normalized_obligations),
    );

    let stop_conditions = take_keyed_object(root, "stop_conditions")?;
    root.insert(
        "stop_conditions".to_string(),
        serde_json::Value::Array(normalize_indexed_entries(
            stop_conditions,
            "condition_index",
            "stop condition",
        )?),
    );

    let diagnostics = take_keyed_object(root, "diagnostics")?;
    let mut normalized_diagnostics = Vec::with_capacity(diagnostics.len());
    for (diagnostic_id, diagnostic) in sorted_entries(diagnostics) {
        require_string_discriminator(&diagnostic, "diagnostic_id", &diagnostic_id)?;
        normalized_diagnostics.push(diagnostic);
    }
    root.insert(
        "diagnostics".to_string(),
        serde_json::Value::Array(normalized_diagnostics),
    );

    serde_json::from_value(value).map_err(|error| {
        ResearchContractAssessmentError::new(format!(
            "research contract assessment violated its typed contract: {error}"
        ))
    })
}

/// Decode one generated assessment chunk and restore any chunk-local integer
/// references to their durable inquiry identifiers.
pub fn decode_research_contract_assessment_chunk(
    state: &InquiryState,
    chunk: &ResearchContractAssessmentGenerationChunk,
    value: serde_json::Value,
) -> Result<ResearchContractAssessment, ResearchContractAssessmentError> {
    validate_assessment_input_state(state)?;
    let assessment = match chunk.reference_encoding {
        AssessmentReferenceEncoding::ExactIds => decode_research_contract_assessment(value)?,
        AssessmentReferenceEncoding::Indexed => {
            decode_indexed_research_contract_assessment(value, &chunk.reference_catalog)?
        }
    };
    validate_assessment_chunk_identity(state, &chunk.scope, &assessment)?;
    Ok(assessment)
}

/// Combine independently decoded chunks into the stable event representation.
/// The full host validator is authoritative: omitted, repeated, or unknown
/// obligation/criterion/stop/diagnostic identities fail before an event can be
/// persisted.
pub fn aggregate_research_contract_assessments(
    state: &InquiryState,
    parts: Vec<ResearchContractAssessment>,
) -> Result<ResearchContractAssessment, ResearchContractAssessmentError> {
    let mut assessment = ResearchContractAssessment {
        obligations: Vec::new(),
        stop_conditions: Vec::new(),
        diagnostics: Vec::new(),
    };
    for part in parts {
        assessment.obligations.extend(part.obligations);
        assessment.stop_conditions.extend(part.stop_conditions);
        assessment.diagnostics.extend(part.diagnostics);
    }

    let obligation_order = state
        .obligations
        .iter()
        .enumerate()
        .map(|(index, obligation)| (obligation.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    assessment.obligations.sort_by_key(|item| {
        obligation_order
            .get(item.obligation_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    assessment
        .stop_conditions
        .sort_by_key(|item| item.condition_index);
    let diagnostic_order = evidence_diagnostic_catalog(state)
        .into_iter()
        .enumerate()
        .map(|(index, (diagnostic, _))| (diagnostic.id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    assessment.diagnostics.sort_by_key(|item| {
        diagnostic_order
            .get(item.diagnostic_id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });

    validate_research_contract_assessment(state, &assessment)?;
    Ok(assessment)
}

fn decode_indexed_research_contract_assessment(
    mut value: serde_json::Value,
    references: &AssessmentReferenceCatalog,
) -> Result<ResearchContractAssessment, ResearchContractAssessmentError> {
    let root = value.as_object_mut().ok_or_else(|| {
        ResearchContractAssessmentError::new("research contract assessment must be an object")
    })?;

    let obligations = take_keyed_object(root, "obligations")?;
    let mut normalized_obligations = Vec::with_capacity(obligations.len());
    for (obligation_id, mut obligation) in sorted_entries(obligations) {
        let local_references = references.obligations.get(&obligation_id).ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "indexed assessment references obligation `{obligation_id}` outside its chunk"
            ))
        })?;
        let obligation_object = obligation.as_object_mut().ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "research obligation `{obligation_id}` must be an object"
            ))
        })?;
        inject_string_discriminator(obligation_object, "obligation_id", &obligation_id)?;
        let criteria = take_keyed_object(obligation_object, "criteria")?;
        let mut normalized_criteria = Vec::with_capacity(criteria.len());
        for (key, mut criterion) in sorted_entries(criteria) {
            let criterion_index = parse_index_key(&key, "completion criterion")?;
            let criterion_object = criterion.as_object_mut().ok_or_else(|| {
                ResearchContractAssessmentError::new(format!(
                    "completion criterion `{key}` must be an object"
                ))
            })?;
            inject_integer_discriminator(
                criterion_object,
                "criterion_index",
                criterion_index,
                "completion criterion",
            )?;
            normalize_indexed_reference_field(
                criterion_object,
                "evidence_ids",
                &local_references.evidence_ids,
            )?;
            normalized_criteria.push((criterion_index, criterion));
        }
        normalized_criteria.sort_by_key(|(index, _)| *index);
        obligation_object.insert(
            "criteria".to_string(),
            serde_json::Value::Array(
                normalized_criteria
                    .into_iter()
                    .map(|(_, criterion)| criterion)
                    .collect(),
            ),
        );
        for requirement in ["primary_source", "independent_corroboration"] {
            let Some(requirement_object) = obligation_object
                .get_mut(requirement)
                .and_then(serde_json::Value::as_object_mut)
            else {
                continue;
            };
            normalize_indexed_reference_field(
                requirement_object,
                "evidence_ids",
                &local_references.evidence_ids,
            )?;
            normalize_indexed_reference_field(
                requirement_object,
                "source_ids",
                &local_references.source_ids,
            )?;
        }
        normalized_obligations.push(obligation);
    }
    root.insert(
        "obligations".to_string(),
        serde_json::Value::Array(normalized_obligations),
    );

    let stop_conditions = take_keyed_object(root, "stop_conditions")?;
    let mut normalized_stops = Vec::with_capacity(stop_conditions.len());
    for (key, mut stop) in sorted_entries(stop_conditions) {
        let condition_index = parse_index_key(&key, "stop condition")?;
        let stop_object = stop.as_object_mut().ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "stop condition `{key}` must be an object"
            ))
        })?;
        inject_integer_discriminator(
            stop_object,
            "condition_index",
            condition_index,
            "stop condition",
        )?;
        normalize_indexed_reference_field(
            stop_object,
            "evidence_ids",
            &references.stop_condition_evidence_ids,
        )?;
        normalized_stops.push((condition_index, stop));
    }
    normalized_stops.sort_by_key(|(index, _)| *index);
    root.insert(
        "stop_conditions".to_string(),
        serde_json::Value::Array(normalized_stops.into_iter().map(|(_, stop)| stop).collect()),
    );

    let diagnostics = take_keyed_object(root, "diagnostics")?;
    let mut normalized_diagnostics = Vec::with_capacity(diagnostics.len());
    for (diagnostic_id, mut diagnostic) in sorted_entries(diagnostics) {
        let local_references = references.diagnostics.get(&diagnostic_id).ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "indexed assessment references diagnostic `{diagnostic_id}` outside its chunk"
            ))
        })?;
        let diagnostic_object = diagnostic.as_object_mut().ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "evidence diagnostic `{diagnostic_id}` must be an object"
            ))
        })?;
        inject_string_discriminator(diagnostic_object, "diagnostic_id", &diagnostic_id)?;
        normalize_indexed_reference_field(
            diagnostic_object,
            "obligation_ids",
            &local_references.obligation_ids,
        )?;
        let disposition = diagnostic_object
            .get("disposition")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ResearchContractAssessmentError::new(format!(
                    "evidence diagnostic `{diagnostic_id}` omitted string `disposition`"
                ))
            })?;
        let evidence_references = match disposition {
            "resolved" => &local_references.resolved_evidence_ids,
            "bounded" => &local_references.bounded_evidence_ids,
            "irrelevant" => &[][..],
            other => {
                return Err(ResearchContractAssessmentError::new(format!(
                    "evidence diagnostic `{diagnostic_id}` has unknown disposition `{other}`"
                )))
            }
        };
        normalize_indexed_reference_field(diagnostic_object, "evidence_ids", evidence_references)?;
        normalized_diagnostics.push(diagnostic);
    }
    root.insert(
        "diagnostics".to_string(),
        serde_json::Value::Array(normalized_diagnostics),
    );

    serde_json::from_value(value).map_err(|error| {
        ResearchContractAssessmentError::new(format!(
            "research contract assessment violated its typed contract: {error}"
        ))
    })
}

fn parse_index_key(key: &str, resource: &str) -> Result<usize, ResearchContractAssessmentError> {
    key.parse::<usize>().map_err(|_| {
        ResearchContractAssessmentError::new(format!(
            "{resource} key `{key}` is not a non-negative index"
        ))
    })
}

fn inject_string_discriminator(
    object: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    expected: &str,
) -> Result<(), ResearchContractAssessmentError> {
    if let Some(value) = object.get(field) {
        let actual = value.as_str().ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "research contract assessment entry `{expected}` has a non-string `{field}`"
            ))
        })?;
        if actual != expected {
            return Err(ResearchContractAssessmentError::new(format!(
                "research contract assessment key `{expected}` disagrees with `{field}` value `{actual}`"
            )));
        }
    } else {
        object.insert(
            field.to_string(),
            serde_json::Value::String(expected.to_string()),
        );
    }
    Ok(())
}

fn inject_integer_discriminator(
    object: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    expected: usize,
    resource: &str,
) -> Result<(), ResearchContractAssessmentError> {
    if let Some(value) = object.get(field) {
        let actual = value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                ResearchContractAssessmentError::new(format!(
                    "{resource} `{expected}` has a non-integer `{field}`"
                ))
            })?;
        if actual != expected {
            return Err(ResearchContractAssessmentError::new(format!(
                "{resource} key `{expected}` disagrees with `{field}` value `{actual}`"
            )));
        }
    } else {
        object.insert(field.to_string(), serde_json::Value::from(expected));
    }
    Ok(())
}

fn normalize_indexed_reference_field(
    object: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    references: &[String],
) -> Result<(), ResearchContractAssessmentError> {
    let values = object
        .get_mut(field)
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "indexed research contract assessment omitted array `{field}`"
            ))
        })?;
    let mut seen = BTreeSet::new();
    for value in values.iter_mut() {
        let index = value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                ResearchContractAssessmentError::new(format!(
                    "indexed research contract assessment `{field}` contains a non-integer reference"
                ))
            })?;
        let reference = references.get(index).ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "indexed research contract assessment `{field}` reference `{index}` is out of range"
            ))
        })?;
        if !seen.insert(index) {
            return Err(ResearchContractAssessmentError::new(format!(
                "indexed research contract assessment `{field}` repeats reference `{index}`"
            )));
        }
        *value = serde_json::Value::String(reference.clone());
    }
    Ok(())
}

fn validate_assessment_chunk_identity(
    state: &InquiryState,
    scope: &AssessmentChunkScope,
    assessment: &ResearchContractAssessment,
) -> Result<(), ResearchContractAssessmentError> {
    let diagnostics = evidence_diagnostic_catalog(state);
    let expected_obligations = scope
        .units
        .iter()
        .filter_map(|unit| match *unit {
            AssessmentUnit::Obligation(index) => Some(state.obligations[index].id.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let actual_obligations = unique_by_key(
        &assessment.obligations,
        |item| item.obligation_id.as_str(),
        "research obligation assessment",
    )?;
    if actual_obligations.keys().copied().collect::<BTreeSet<_>>() != expected_obligations {
        return Err(ResearchContractAssessmentError::new(
            "research contract assessment chunk has missing or unknown obligation identities",
        ));
    }
    for obligation_id in expected_obligations {
        let obligation = state
            .obligations
            .iter()
            .find(|obligation| obligation.id == obligation_id)
            .ok_or_else(|| {
                ResearchContractAssessmentError::new(format!(
                    "research contract assessment chunk references unknown obligation `{obligation_id}`"
                ))
            })?;
        let item = actual_obligations[obligation_id];
        let actual_criteria = item
            .criteria
            .iter()
            .map(|criterion| criterion.criterion_index)
            .collect::<BTreeSet<_>>();
        let expected_criteria = (0..obligation.completion_criteria.len()).collect::<BTreeSet<_>>();
        if item.criteria.len() != expected_criteria.len() || actual_criteria != expected_criteria {
            return Err(ResearchContractAssessmentError::new(format!(
                "research obligation `{obligation_id}` chunk has missing or duplicate criterion identities"
            )));
        }
    }

    let expected_stops = scope
        .units
        .iter()
        .filter_map(|unit| match *unit {
            AssessmentUnit::StopCondition(index) => Some(index),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let actual_stops = assessment
        .stop_conditions
        .iter()
        .map(|condition| condition.condition_index)
        .collect::<BTreeSet<_>>();
    if assessment.stop_conditions.len() != expected_stops.len() || actual_stops != expected_stops {
        return Err(ResearchContractAssessmentError::new(
            "research contract assessment chunk has missing, duplicate, or unknown stop-condition identities",
        ));
    }

    let expected_diagnostics = scope
        .units
        .iter()
        .filter_map(|unit| match *unit {
            AssessmentUnit::Diagnostic(index) => Some(diagnostics[index].0.id.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let actual_diagnostics = unique_by_key(
        &assessment.diagnostics,
        |item| item.diagnostic_id.as_str(),
        "evidence diagnostic assessment",
    )?;
    if actual_diagnostics.keys().copied().collect::<BTreeSet<_>>() != expected_diagnostics {
        return Err(ResearchContractAssessmentError::new(
            "research contract assessment chunk has missing or unknown diagnostic identities",
        ));
    }
    Ok(())
}

fn take_keyed_object(
    container: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, ResearchContractAssessmentError> {
    container
        .remove(field)
        .ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "research contract assessment omitted `{field}`"
            ))
        })?
        .as_object()
        .cloned()
        .ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "research contract assessment `{field}` must be a keyed object"
            ))
        })
}

fn sorted_entries(
    entries: serde_json::Map<String, serde_json::Value>,
) -> Vec<(String, serde_json::Value)> {
    entries
        .into_iter()
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect()
}

fn require_string_discriminator(
    value: &serde_json::Value,
    field: &str,
    expected: &str,
) -> Result<(), ResearchContractAssessmentError> {
    let actual = value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ResearchContractAssessmentError::new(format!(
                "research contract assessment entry `{expected}` omitted string `{field}`"
            ))
        })?;
    if actual == expected {
        Ok(())
    } else {
        Err(ResearchContractAssessmentError::new(format!(
            "research contract assessment key `{expected}` disagrees with `{field}` value `{actual}`"
        )))
    }
}

fn normalize_indexed_entries(
    entries: serde_json::Map<String, serde_json::Value>,
    field: &str,
    resource: &str,
) -> Result<Vec<serde_json::Value>, ResearchContractAssessmentError> {
    let mut indexed = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let index = key.parse::<usize>().map_err(|_| {
            ResearchContractAssessmentError::new(format!(
                "{resource} key `{key}` is not a non-negative index"
            ))
        })?;
        let actual = value
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                ResearchContractAssessmentError::new(format!(
                    "{resource} `{key}` omitted integer `{field}`"
                ))
            })?;
        if actual != index {
            return Err(ResearchContractAssessmentError::new(format!(
                "{resource} key `{key}` disagrees with `{field}` value `{actual}`"
            )));
        }
        indexed.push((index, value));
    }
    indexed.sort_by_key(|(index, _)| *index);
    Ok(indexed.into_iter().map(|(_, value)| value).collect())
}

fn add_required_property(
    schema: &mut serde_json::Value,
    name: &str,
    property: serde_json::Value,
) -> Result<(), ResearchContractAssessmentError> {
    let properties = schema
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| {
            ResearchContractAssessmentError::new(
                "research obligation assessment schema omitted properties",
            )
        })?;
    properties.insert(name.to_string(), property);
    let required = schema
        .get_mut("required")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| {
            ResearchContractAssessmentError::new(
                "research obligation assessment schema omitted required fields",
            )
        })?;
    required.push(serde_json::Value::String(name.to_string()));
    Ok(())
}
