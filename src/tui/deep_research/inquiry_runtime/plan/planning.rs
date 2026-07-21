const DEEP_RESEARCH_LOOP_STAGES: [&str; 8] = [
    "semantic_plan",
    "initial_retrieval",
    "semantic_chunk_selection",
    "typed_coverage_evaluation",
    "optional_supplemental_retrieval",
    "final_closed_question_review",
    "host_contract_reduction",
    "sectioned_report_transaction",
];

const DEEP_RESEARCH_LOOP_CARDINALITY: [&str; 7] = [
    "semantic_iterations",
    "retrieval_passes",
    "semantic_selections",
    "question_reviews",
    "contract_assessments",
    "report_transactions",
    "section_revision_rounds",
];

const SEMANTIC_PLAN_FIELDS: [&str; 5] = [
    "report_title",
    "freshness_required",
    "workspace_evidence_required",
    "tracks",
    "stop_conditions",
];
const RETRIEVAL_PLAN_FIELDS: [&str; 3] = ["search_queries", "seed_urls", "budget"];

pub(super) async fn generate_plan(
    session: &AgentSession,
    workflow_args: &Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    checkpoint: &InquiryCheckpointWriter,
) -> Result<PlannedInquiry, String> {
    let planner = validated_loop_planner(workflow_args)?;
    let full_schema = planner
        .get("output_schema")
        .cloned()
        .ok_or_else(|| "DeepResearch planner contract has no output schema".to_string())?;
    let semantic_schema = planner_fragment_schema(&full_schema, &SEMANTIC_PLAN_FIELDS)?;
    let retrieval_schema = planner_fragment_schema(&full_schema, &RETRIEVAL_PLAN_FIELDS)?;
    let semantic_prompt = required_planner_text(planner, "semantic_prompt")?;
    let retrieval_prompt = required_planner_text(planner, "retrieval_prompt")?;
    let semantic_timeout_ms = required_planner_timeout(planner, "semantic_timeout_ms")?;
    let retrieval_timeout_ms = required_planner_timeout(planner, "retrieval_timeout_ms")?;

    let semantic_args = serde_json::json!({
        "schema": semantic_schema,
        "schema_name": "deep_research_semantic_plan",
        "schema_description": "Semantic research contract for one bounded DeepResearch inquiry",
        "prompt": semantic_prompt,
        "system": "You are a concise research-contract planner. Return only the requested semantic object and no reasoning.",
        "mode": "auto",
        "max_repair_attempts": 1,
        "include_raw_text": false,
        "timeout_ms": semantic_timeout_ms,
    });
    let semantic_workflow_timeout_ms = semantic_timeout_ms
        .saturating_mul(u64::from(PLANNER_GENERATION_MAX_ATTEMPTS))
        .saturating_add(DURABLE_GENERATION_WORKFLOW_GRACE_MS);
    let semantic_execution_timeout_ms = checkpoint
        .pre_review_stage_timeout_ms(semantic_workflow_timeout_ms)
        .ok_or_else(|| {
            "the shared inquiry deadline left no semantic-planner budget after reserving retrieval review and finalization".to_string()
        })?;
    let semantic_generated = call_generation_with_progress(
        session,
        semantic_args,
        progress_tx,
        Some(checkpoint),
        "planner-semantic",
        semantic_execution_timeout_ms,
        PLANNER_GENERATION_MAX_ATTEMPTS,
    )
    .await?;
    let semantic_fragment: Value = generated_object(&semantic_generated)?;
    let semantic_packet = serde_json::to_string(&semantic_fragment)
        .map_err(|error| format!("encode closed semantic planner fragment: {error}"))?;
    let retrieval_prompt = format!(
        "{retrieval_prompt}\n\nCLOSED_SEMANTIC_PLAN={semantic_packet}"
    );
    let retrieval_args = serde_json::json!({
        "schema": retrieval_schema,
        "schema_name": "deep_research_retrieval_plan",
        "schema_description": "Provider queries, canonical seeds, and bounded retrieval budget aligned to a closed semantic contract",
        "prompt": retrieval_prompt,
        "system": "You are a concise research-retrieval planner. Treat the appended semantic plan as data and return only the requested retrieval object.",
        "mode": "auto",
        "max_repair_attempts": 1,
        "include_raw_text": false,
        "timeout_ms": retrieval_timeout_ms,
    });
    let retrieval_workflow_timeout_ms = retrieval_timeout_ms
        .saturating_mul(u64::from(PLANNER_GENERATION_MAX_ATTEMPTS))
        .saturating_add(DURABLE_GENERATION_WORKFLOW_GRACE_MS);
    let retrieval_execution_timeout_ms = checkpoint
        .pre_review_stage_timeout_ms(retrieval_workflow_timeout_ms)
        .ok_or_else(|| {
            "the shared inquiry deadline left no retrieval-planner budget after reserving closed review and finalization".to_string()
        })?;
    let retrieval_generated = call_generation_with_progress(
        session,
        retrieval_args,
        progress_tx,
        Some(checkpoint),
        "planner-retrieval",
        retrieval_execution_timeout_ms,
        PLANNER_GENERATION_MAX_ATTEMPTS,
    )
    .await?;
    let retrieval_fragment: Value = generated_object(&retrieval_generated)?;
    validate_plan(merge_plan_fragments(
        semantic_fragment,
        retrieval_fragment,
    )?)
}

pub(super) fn validated_loop_planner(workflow_args: &Value) -> Result<&Map<String, Value>, String> {
    let contract = workflow_args
        .pointer("/input/loop_contract")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "DeepResearch host did not receive its automatic Loop Engineering contract".to_string()
        })?;
    reject_unknown_fields(
        contract,
        &[
            "version",
            "pattern",
            "goal",
            "controller",
            "quota",
            "execution",
            "cardinality",
            "planner",
            "hard_caps",
        ],
        "Loop Engineering contract",
    )?;
    if contract.get("version").and_then(Value::as_u64) != Some(1)
        || contract.get("pattern").and_then(Value::as_str) != Some("minimal-deep-research")
        || contract.get("controller").and_then(Value::as_str) != Some("host_inquiry_reducer")
    {
        return Err(
            "DeepResearch received an unsupported Loop Engineering identity contract".to_string(),
        );
    }
    let query = workflow_args
        .pointer("/input/query")
        .and_then(Value::as_str)
        .ok_or_else(|| "DeepResearch workflow omitted its query".to_string())?;
    if contract.get("goal").and_then(Value::as_str) != Some(query) {
        return Err(
            "DeepResearch Loop Engineering goal differs from the workflow query".to_string(),
        );
    }

    let quota = contract
        .get("quota")
        .and_then(Value::as_object)
        .ok_or_else(|| "DeepResearch Loop Engineering contract omitted quota".to_string())?;
    reject_unknown_fields(quota, &["mode"], "Loop Engineering quota")?;
    if quota.get("mode").and_then(Value::as_str) != Some("unlimited") {
        return Err("DeepResearch Loop Engineering quota must be `unlimited`".to_string());
    }

    let execution = contract
        .get("execution")
        .and_then(Value::as_object)
        .ok_or_else(|| "DeepResearch Loop Engineering contract omitted execution".to_string())?;
    reject_unknown_fields(execution, &["mode", "stages"], "Loop Engineering execution")?;
    if execution.get("mode").and_then(Value::as_str) != Some("coverage_driven") {
        return Err(
            "DeepResearch Loop Engineering execution must be `coverage_driven`".to_string(),
        );
    }
    let stages = execution
        .get("stages")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "DeepResearch Loop Engineering execution omitted its stage graph".to_string()
        })?;
    let stages = stages
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            "DeepResearch Loop Engineering stage graph contains a non-string stage".to_string()
        })?;
    if stages.as_slice() != DEEP_RESEARCH_LOOP_STAGES {
        return Err(
            "DeepResearch Loop Engineering stage graph differs from the minimal pipeline"
                .to_string(),
        );
    }

    let cardinality = contract
        .get("cardinality")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "DeepResearch Loop Engineering contract omitted stage cardinality".to_string()
        })?;
    reject_unknown_fields(
        cardinality,
        &DEEP_RESEARCH_LOOP_CARDINALITY,
        "Loop Engineering cardinality",
    )?;
    for (field, expected) in [
        ("semantic_iterations", 2),
        ("retrieval_passes", 2),
        ("semantic_selections", 2),
        ("question_reviews", 1),
        ("contract_assessments", 1),
        ("report_transactions", 1),
        ("section_revision_rounds", 2),
    ] {
        if cardinality.get(field).and_then(Value::as_u64) != Some(expected) {
            return Err(format!(
                "DeepResearch Loop Engineering cardinality `{field}` must be exactly {expected}"
            ));
        }
    }

    let planner = contract
        .get("planner")
        .and_then(Value::as_object)
        .ok_or_else(|| "DeepResearch Loop Engineering contract omitted its planner".to_string())?;
    reject_unknown_fields(
        planner,
        &[
            "agent",
            "description",
            "max_steps",
            "semantic_timeout_ms",
            "retrieval_timeout_ms",
            "semantic_prompt",
            "retrieval_prompt",
            "output_schema",
        ],
        "Loop Engineering planner",
    )?;
    if planner.get("agent").and_then(Value::as_str) != Some("research-planner")
        || planner.get("max_steps").and_then(Value::as_u64) != Some(2)
    {
        return Err(
            "DeepResearch Loop Engineering planner must contain the two bounded planning effects"
                .to_string(),
        );
    }
    for field in ["description", "semantic_prompt", "retrieval_prompt"] {
        if !planner
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(format!(
                "DeepResearch Loop Engineering planner omitted non-empty `{field}`"
            ));
        }
    }
    for field in ["semantic_timeout_ms", "retrieval_timeout_ms"] {
        required_integer_in_range(
            planner,
            field,
            1_000,
            600_000,
            "Loop Engineering planner",
        )?;
    }
    if !planner.get("output_schema").is_some_and(Value::is_object) {
        return Err(
            "DeepResearch Loop Engineering planner omitted its object output schema".to_string(),
        );
    }

    let hard_caps = contract
        .get("hard_caps")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "DeepResearch Loop Engineering contract omitted its safety fuses".to_string()
        })?;
    reject_unknown_fields(
        hard_caps,
        &[
            "max_tracks",
            "max_searches",
            "max_fetches",
            "max_supplemental_fetches",
            "retrieval_timeout_ms",
        ],
        "Loop Engineering safety fuses",
    )?;
    required_integer_in_range(
        hard_caps,
        "max_tracks",
        1,
        4,
        "Loop Engineering safety fuses",
    )?;
    for (field, expected) in [
        ("max_searches", 4),
        ("max_fetches", 8),
        ("max_supplemental_fetches", 2),
        ("retrieval_timeout_ms", 150_000),
    ] {
        if hard_caps.get(field).and_then(Value::as_u64) != Some(expected) {
            return Err(format!(
                "DeepResearch Loop Engineering safety fuse `{field}` must be {expected}"
            ));
        }
    }

    Ok(planner)
}

fn required_planner_text<'a>(
    planner: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, String> {
    planner
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("DeepResearch planner contract has no non-empty `{field}`"))
}

fn required_planner_timeout(
    planner: &Map<String, Value>,
    field: &str,
) -> Result<u64, String> {
    required_integer_in_range(
        planner,
        field,
        1_000,
        600_000,
        "Loop Engineering planner",
    )
}

pub(super) fn planner_fragment_schema(
    full_schema: &Value,
    fields: &[&str],
) -> Result<Value, String> {
    if fields.is_empty() {
        return Err("DeepResearch planner fragment requires at least one field".to_string());
    }
    let mut schema = full_schema.clone();
    let object = schema
        .as_object_mut()
        .ok_or_else(|| "DeepResearch full planner schema is not an object".to_string())?;
    let properties = object
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "DeepResearch full planner schema omitted object properties".to_string())?;
    let missing = fields
        .iter()
        .filter(|field| !properties.contains_key(**field))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "DeepResearch planner fragment requested unknown schema fields: {}",
            missing.join(", ")
        ));
    }
    properties.retain(|field, _| fields.contains(&field.as_str()));
    object.insert(
        "required".to_string(),
        Value::Array(
            fields
                .iter()
                .map(|field| Value::String((*field).to_string()))
                .collect(),
        ),
    );
    Ok(schema)
}

pub(super) fn merge_plan_fragments(
    semantic: Value,
    retrieval: Value,
) -> Result<Value, String> {
    let mut merged = semantic
        .as_object()
        .cloned()
        .ok_or_else(|| "DeepResearch semantic planner returned a non-object fragment".to_string())?;
    let retrieval = retrieval
        .as_object()
        .ok_or_else(|| "DeepResearch retrieval planner returned a non-object fragment".to_string())?;
    for (field, value) in retrieval {
        if merged.insert(field.clone(), value.clone()).is_some() {
            return Err(format!(
                "DeepResearch planner fragments overlap on field `{field}`"
            ));
        }
    }
    Ok(Value::Object(merged))
}

pub(super) fn validate_plan(value: Value) -> Result<PlannedInquiry, String> {
    let value = normalize_planner_budget(value)?;
    let object = value
        .as_object()
        .ok_or_else(|| "DeepResearch planner returned a non-object plan".to_string())?;
    reject_unknown_fields(
        object,
        &[
            "report_title",
            "freshness_required",
            "workspace_evidence_required",
            "tracks",
            "search_queries",
            "seed_urls",
            "budget",
            "stop_conditions",
        ],
        "plan",
    )?;
    required_text(object, "report_title")?;
    required_bool(object, "freshness_required", "plan")?;
    required_bool(object, "workspace_evidence_required", "plan")?;
    let _search_queries = exact_string_array(object.get("search_queries"), "search_queries", 4)?;
    let _seed_urls = string_array(object.get("seed_urls"), "seed_urls", 3)?;
    let budget = object
        .get("budget")
        .and_then(Value::as_object)
        .ok_or_else(|| "DeepResearch plan omitted its retrieval budget".to_string())?;
    reject_unknown_fields(
        budget,
        &["retrieval_timeout_ms", "direct_searches", "direct_fetches"],
        "retrieval budget",
    )?;
    required_integer_in_range(
        budget,
        "retrieval_timeout_ms",
        30_000,
        150_000,
        "retrieval budget",
    )?;
    required_integer_in_range(budget, "direct_searches", 0, 4, "retrieval budget")?;
    required_integer_in_range(budget, "direct_fetches", 0, 8, "retrieval budget")?;
    let (obligations, _) = research_contract_from_plan(&value)?;
    let tracks = object
        .get("tracks")
        .and_then(Value::as_array)
        .ok_or_else(|| "DeepResearch plan did not contain stable research tracks".to_string())?;
    for track in tracks {
        let track = track
            .as_object()
            .ok_or_else(|| "DeepResearch planner returned a non-object track".to_string())?;
        reject_unknown_fields(
            track,
            &[
                "id",
                "title",
                "focus",
                "material",
                "questions",
                "completion_criteria",
                "evidence_requirements",
            ],
            "track",
        )?;
        required_bool(track, "material", "track")?;
        let questions = string_array(track.get("questions"), "track questions", 2)?;
        if questions.is_empty() {
            return Err("DeepResearch track has no research question".to_string());
        }
        let completion_criteria = string_array(
            track.get("completion_criteria"),
            "track completion_criteria",
            2,
        )?;
        if completion_criteria.is_empty() {
            return Err("DeepResearch track has no completion criterion".to_string());
        }
        let evidence_requirements = track
            .get("evidence_requirements")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                "DeepResearch track omitted object `evidence_requirements`".to_string()
            })?;
        reject_unknown_fields(
            evidence_requirements,
            &[
                "primary_source_required",
                "independent_corroboration_required",
            ],
            "track evidence requirements",
        )?;
    }
    debug_assert!(obligations.iter().any(|obligation| obligation.material));
    Ok(PlannedInquiry { value })
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    resource: &str,
) -> Result<(), String> {
    let unexpected = object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if unexpected.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "DeepResearch {resource} contains unsupported field(s): {}",
            unexpected.join(", ")
        ))
    }
}

fn required_bool(object: &Map<String, Value>, key: &str, resource: &str) -> Result<bool, String> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("DeepResearch {resource} omitted boolean `{key}`"))
}

fn required_integer_in_range(
    object: &Map<String, Value>,
    key: &str,
    minimum: u64,
    maximum: u64,
    resource: &str,
) -> Result<u64, String> {
    let value = object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("DeepResearch {resource} omitted integer `{key}`"))?;
    if (minimum..=maximum).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "DeepResearch {resource} `{key}` must be between {minimum} and {maximum}"
        ))
    }
}

/// Convert the LLM-authored stable tracks into the typed coverage contract
/// consumed by the replayable Inquiry reducer. This is the only planner-to-
/// state boundary for research obligations and stopping conditions.
pub(super) fn research_contract_from_plan(
    plan: &Value,
) -> Result<(Vec<ResearchObligation>, Vec<String>), String> {
    let object = plan
        .as_object()
        .ok_or_else(|| "DeepResearch planner returned a non-object plan".to_string())?;
    let tracks = object
        .get("tracks")
        .and_then(Value::as_array)
        .filter(|tracks| !tracks.is_empty())
        .ok_or_else(|| "DeepResearch plan did not contain stable research tracks".to_string())?;
    let limits = InquiryLimits::default();
    if tracks.len() > limits.max_obligations {
        return Err(format!(
            "DeepResearch plan has {} stable research tracks; maximum is {}",
            tracks.len(),
            limits.max_obligations
        ));
    }

    let mut track_ids = BTreeSet::new();
    let mut obligations = Vec::with_capacity(tracks.len());
    for track in tracks {
        let track = track
            .as_object()
            .ok_or_else(|| "DeepResearch planner returned a non-object track".to_string())?;
        let id = required_text(track, "id")?;
        if !is_stable_plan_id(id) {
            return Err(format!(
                "DeepResearch track id `{id}` is not a stable ASCII identifier"
            ));
        }
        if !track_ids.insert(id) {
            return Err(format!("duplicate DeepResearch track id `{id}`"));
        }
        let title = required_text(track, "title")?;
        let focus = required_text(track, "focus")?;
        let material = track
            .get("material")
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("DeepResearch track `{id}` omitted boolean `material`"))?;
        let completion_criteria = string_array(
            track.get("completion_criteria"),
            "track completion_criteria",
            2,
        )?;
        if completion_criteria.is_empty() {
            return Err(format!(
                "DeepResearch track `{id}` has no completion criterion"
            ));
        }
        let evidence_requirements = track
            .get("evidence_requirements")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                format!("DeepResearch track `{id}` omitted object `evidence_requirements`")
            })?;
        let primary_source_required = evidence_requirements
            .get("primary_source_required")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                format!("DeepResearch track `{id}` omitted boolean `primary_source_required`")
            })?;
        let independent_corroboration_required = evidence_requirements
            .get("independent_corroboration_required")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                format!(
                    "DeepResearch track `{id}` omitted boolean `independent_corroboration_required`"
                )
            })?;
        obligations.push(
            ResearchObligation::new(id, title, focus, material, completion_criteria)
                .with_evidence_requirements(EvidenceQualityRequirements {
                    primary_source_required,
                    independent_corroboration_required,
                }),
        );
    }
    if !obligations.iter().any(|obligation| obligation.material) {
        return Err("DeepResearch plan must contain at least one material track".to_string());
    }

    let stop_conditions = string_array(
        object.get("stop_conditions"),
        "stop_conditions",
        limits.max_stop_conditions,
    )?;
    if stop_conditions.is_empty() {
        return Err("DeepResearch plan has no stopping condition".to_string());
    }
    Ok((obligations, stop_conditions))
}

pub(super) fn commit_plan_research_contract(
    plan: &Value,
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
) -> Result<(), String> {
    let (obligations, stop_conditions) = research_contract_from_plan(plan)?;
    apply_event(
        state,
        events,
        InquiryEvent::ResearchObligationsCommitted {
            obligations,
            stop_conditions,
        },
        limits,
    )
}

fn is_stable_plan_id(value: &str) -> bool {
    value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value.chars().skip(1).all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
}
