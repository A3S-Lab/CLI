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

pub(super) async fn generate_plan(
    session: &AgentSession,
    workflow_args: &Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    checkpoint: &InquiryCheckpointWriter,
) -> Result<PlannedInquiry, String> {
    let planner = validated_loop_planner(workflow_args)?;
    let schema = planner
        .get("output_schema")
        .cloned()
        .ok_or_else(|| "DeepResearch planner contract has no output schema".to_string())?;
    let prompt = planner
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|prompt| !prompt.trim().is_empty())
        .ok_or_else(|| "DeepResearch planner contract has no prompt".to_string())?;
    let timeout_ms = planner
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(120_000)
        .clamp(1_000, 600_000);
    let generation_args = serde_json::json!({
        "schema": schema,
        "schema_name": "deep_research_plan",
        "schema_description": "Semantic plan for one bounded coverage-driven DeepResearch inquiry",
        "prompt": prompt,
        "mode": "auto",
        "max_repair_attempts": 1,
        "include_raw_text": false,
        "timeout_ms": timeout_ms,
    });
    let workflow_timeout_ms = timeout_ms
        .saturating_mul(u64::from(PLANNER_GENERATION_MAX_ATTEMPTS))
        .saturating_add(DURABLE_GENERATION_WORKFLOW_GRACE_MS);
    let execution_timeout_ms = checkpoint
        .pre_review_stage_timeout_ms(workflow_timeout_ms)
        .ok_or_else(|| {
            "the shared inquiry deadline left no planner budget after reserving retrieval review and finalization".to_string()
        })?;
    let generated = call_generation_with_progress(
        session,
        generation_args,
        progress_tx,
        Some(checkpoint),
        "planner",
        execution_timeout_ms,
        PLANNER_GENERATION_MAX_ATTEMPTS,
    )
    .await?;
    let value: Value = generated_object(&generated)?;
    validate_plan(value)
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
            "timeout_ms",
            "prompt",
            "output_schema",
        ],
        "Loop Engineering planner",
    )?;
    if planner.get("agent").and_then(Value::as_str) != Some("research-planner")
        || planner.get("max_steps").and_then(Value::as_u64) != Some(1)
    {
        return Err(
            "DeepResearch Loop Engineering planner must be the one-step semantic planner"
                .to_string(),
        );
    }
    for field in ["description", "prompt"] {
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
    required_integer_in_range(
        planner,
        "timeout_ms",
        1_000,
        600_000,
        "Loop Engineering planner",
    )?;
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
