fn accepted_evidence_ids(evidence: &[AcceptedEvidence]) -> BTreeSet<String> {
    evidence.iter().map(|item| item.id.clone()).collect()
}

struct PreparedEvidenceCatalog {
    addressable: Vec<AcceptedEvidence>,
    pending: Vec<EvidenceRef>,
}

/// Stage the selected evidence until the one closed review has a validated
/// outcome. Evidence and question-resolution events then become one durable
/// logical batch, avoiding an fsync per item and a half-applied review.
fn prepare_evidence_catalog(
    state: &InquiryState,
    evidence: &[AcceptedEvidence],
) -> Result<PreparedEvidenceCatalog, String> {
    let mut addressable = Vec::new();
    let mut pending = Vec::new();
    for item in evidence {
        let claim_ids = item
            .claims
            .iter()
            .map(|claim| claim.id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let source_ids = item
            .sources
            .iter()
            .map(|source| source.id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if claim_ids.is_empty() || source_ids.is_empty() {
            continue;
        }
        let diagnostics = item
            .contradictions
            .iter()
            .enumerate()
            .map(|(index, detail)| {
                EvidenceDiagnostic::new(
                    format!("diagnostic:{}:contradiction:{}", item.id, index + 1),
                    EvidenceDiagnosticKind::Contradiction,
                    detail.clone(),
                )
            })
            .chain(item.gaps.iter().enumerate().map(|(index, detail)| {
                EvidenceDiagnostic::new(
                    format!("diagnostic:{}:gap:{}", item.id, index + 1),
                    EvidenceDiagnosticKind::Gap,
                    detail.clone(),
                )
            }))
            .collect();
        let accepted = EvidenceRef::new(item.id.clone(), claim_ids, source_ids)
            .with_source_coverage(item.source_coverage.clone())
            .with_diagnostics(diagnostics);
        match state.evidence(&item.id) {
            Some(existing) if existing != &accepted => {
                return Err(format!(
                    "accepted evidence `{}` disagrees with its recovered claim/source relationships",
                    item.id
                ));
            }
            Some(_) => {}
            None => pending.push(accepted),
        }
        addressable.push(item.clone());
    }
    Ok(PreparedEvidenceCatalog {
        addressable,
        pending,
    })
}

fn apply_pending_evidence(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    pending: Vec<EvidenceRef>,
) -> Result<(), String> {
    for evidence in pending {
        apply_event(
            state,
            events,
            InquiryEvent::EvidenceAccepted { evidence },
            limits,
        )?;
    }
    Ok(())
}

fn canonical_query(workflow_output: &str) -> Option<String> {
    serde_json::from_str::<Value>(workflow_output)
        .ok()?
        .get("query")?
        .as_str()
        .map(str::to_string)
}

pub(super) fn attach_inquiry_projection(
    mut result: ToolCallResult,
    inquiry_events: &[InquiryEvent],
    state: &InquiryState,
) -> Result<ToolCallResult, String> {
    let canonical =
        deep_research_canonical_workflow_output(&result.output, result.metadata.as_ref());
    let mut value = serde_json::from_str::<Value>(&canonical)
        .map_err(|error| format!("decode focused workflow output: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "focused workflow returned a non-object output".to_string())?;
    let execution = object
        .entry("execution")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| "focused workflow execution field is not an object".to_string())?;
    execution.insert(
        "terminal_authority".to_string(),
        Value::String("host_inquiry_reducer".to_string()),
    );
    let inquiry = object
        .entry("inquiry")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| "workflow inquiry field is not an object".to_string())?;
    inquiry.insert(
        "events".to_string(),
        serde_json::to_value(inquiry_events)
            .map_err(|error| format!("encode inquiry events: {error}"))?,
    );
    inquiry.insert(
        "state".to_string(),
        serde_json::to_value(state).map_err(|error| format!("encode inquiry state: {error}"))?,
    );
    result.output = serde_json::to_string(&value)
        .map_err(|error| format!("encode focused inquiry output: {error}"))?;
    if let Some(snapshot) = result
        .metadata
        .as_mut()
        .and_then(|metadata| metadata.pointer_mut("/dynamic_workflow/snapshot"))
        .and_then(Value::as_object_mut)
    {
        snapshot.insert("output".to_string(), value);
    }
    Ok(result)
}
