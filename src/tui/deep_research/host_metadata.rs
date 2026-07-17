//! Sanitization and bounded metadata projection for DeepResearch workflows.

use super::*;

/// Return the final workflow projection committed by the event-sourced
/// runtime. The tool's display output is only a transport projection and can
/// be truncated or replaced by diagnostic text; the completed snapshot is the
/// durable source of truth for report classification and synthesis.
pub(super) fn deep_research_canonical_workflow_output(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> String {
    let Some(dynamic_workflow) = workflow_metadata
        .and_then(|metadata| metadata.get("dynamic_workflow"))
        .and_then(serde_json::Value::as_object)
    else {
        return workflow_output.to_string();
    };
    let completed = dynamic_workflow
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("completed"));
    if !completed {
        return workflow_output.to_string();
    }
    let Some(output) = dynamic_workflow
        .get("snapshot")
        .and_then(|snapshot| snapshot.get("output"))
        .filter(|output| !output.is_null())
    else {
        return workflow_output.to_string();
    };

    serde_json::to_string(output).unwrap_or_else(|_| workflow_output.to_string())
}

pub(super) fn deep_research_prompt_workflow_output(workflow_output: &str) -> String {
    let value = match serde_json::from_str::<serde_json::Value>(workflow_output) {
        Ok(value) => value,
        Err(_) => {
            if deep_research_output_has_internal_leak(workflow_output) {
                return "Research evidence was non-JSON and contained internal tool logs; raw text withheld from synthesis.".to_string();
            }
            return deep_research_truncate_chars(
                &workflow_output
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                DEEP_RESEARCH_PROMPT_TEXT_LIMIT,
            );
        }
    };
    let digest = deep_research_workflow_output_digest(&value);
    serde_json::to_string_pretty(&digest).unwrap_or_else(|_| {
        deep_research_truncate_chars(workflow_output, DEEP_RESEARCH_PROMPT_TEXT_LIMIT)
    })
}

pub(super) fn deep_research_tool_card_output(workflow_output: &str) -> String {
    workflow_evidence_summary(workflow_output)
        .unwrap_or_else(|| {
            if deep_research_output_has_internal_leak(workflow_output) {
                "Evidence collection returned internal diagnostic logs; raw output withheld from the tool card.".to_string()
            } else {
                deep_research_truncate_chars(workflow_output, 1200)
            }
        })
}

pub(super) fn deep_research_prompt_metadata(
    workflow_metadata: Option<&serde_json::Value>,
) -> String {
    workflow_metadata
        .map(deep_research_workflow_metadata_diagnostics_digest)
        .and_then(|metadata| serde_json::to_string_pretty(&metadata).ok())
        .unwrap_or_else(|| "{}".to_string())
}

pub(super) fn deep_research_workflow_metadata_diagnostics_digest(
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let mut digest = deep_research_workflow_metadata_digest(metadata);
    remove_json_key_recursive(&mut digest, "evidence_items");
    digest
}

pub(super) fn remove_json_key_recursive(value: &mut serde_json::Value, key: &str) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove(key);
            for child in map.values_mut() {
                remove_json_key_recursive(child, key);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                remove_json_key_recursive(item, key);
            }
        }
        _ => {}
    }
}

pub(super) fn deep_research_sanitize_workflow_metadata(
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let mut sanitized = metadata.clone();
    deep_research_sanitize_parallel_task_values(&mut sanitized);
    sanitized
}

pub(super) fn deep_research_workflow_output_digest(value: &serde_json::Value) -> serde_json::Value {
    if value
        .get("evidence_items")
        .and_then(serde_json::Value::as_array)
        .is_some()
    {
        return deep_research_accepted_evidence_payload_digest(value);
    }
    let mut digest = serde_json::Map::new();
    copy_json_field(&mut digest, value, "query");
    digest.insert(
        "collection_status".to_string(),
        serde_json::Value::String(deep_research_collection_status(value).to_string()),
    );
    if let Some(runtime_error) = value.get("runtime_error") {
        digest.insert(
            "collection_error".to_string(),
            serde_json::Value::String(deep_research_error_or_digest_text(runtime_error, 1000)),
        );
    }
    if let Some(verification) = value
        .get("verification")
        .and_then(serde_json::Value::as_object)
    {
        let mut compact = serde_json::Map::new();
        for key in ["status", "checker_completed"] {
            copy_json_field(
                &mut compact,
                &serde_json::Value::Object(verification.clone()),
                key,
            );
        }
        if !compact.is_empty() {
            digest.insert(
                "verification".to_string(),
                serde_json::Value::Object(compact),
            );
        }
    }

    if let Some(research) = value.get("research") {
        if let Some(research) = research.as_object() {
            let mut compact = serde_json::Map::new();
            for key in [
                "algorithm",
                "status",
                "max_rounds",
                "completed_rounds",
                "stop_reason",
            ] {
                copy_json_field(
                    &mut compact,
                    &serde_json::Value::Object(research.clone()),
                    key,
                );
            }
            if let Some(complexity) = research.get("complexity") {
                compact.insert("complexity".to_string(), complexity.clone());
            }
            if let Some(metadata) = research.get("metadata") {
                compact.insert(
                    "counts".to_string(),
                    deep_research_compact_count_metadata(metadata),
                );
            }
            compact.insert(
                "rounds".to_string(),
                deep_research_compact_rounds(research.get("rounds")),
            );
            // Collect from the complete workflow value so hybrid direct-web
            // seed evidence is retained alongside delegated round results.
            let (evidence_items, evidence_items_omitted) =
                deep_research_collect_structured_evidence_bounded(value);
            compact.insert(
                "evidence_items".to_string(),
                serde_json::Value::Array(evidence_items),
            );
            if evidence_items_omitted > 0 {
                compact.insert(
                    "evidence_items_omitted".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(
                        evidence_items_omitted as u64,
                    )),
                );
            }
            if let Some(warnings) = research.get("warnings") {
                compact.insert(
                    "warnings".to_string(),
                    deep_research_compact_warnings(warnings),
                );
            }
            digest.insert("research".to_string(), serde_json::Value::Object(compact));
        } else {
            digest.insert(
                "research_summary".to_string(),
                serde_json::Value::String(deep_research_compact_json_text(
                    research,
                    DEEP_RESEARCH_PROMPT_SUCCESS_OUTPUT_LIMIT,
                )),
            );
        }
    }

    if let Some(seed_research) = value
        .get("seed_research")
        .and_then(serde_json::Value::as_object)
    {
        let mut compact = serde_json::Map::new();
        for key in ["algorithm", "status"] {
            copy_json_field(
                &mut compact,
                &serde_json::Value::Object(seed_research.clone()),
                key,
            );
        }
        if let Some(metadata) = seed_research.get("metadata") {
            compact.insert(
                "counts".to_string(),
                deep_research_compact_count_metadata(metadata),
            );
        }
        if let Some(warnings) = seed_research.get("warnings") {
            compact.insert(
                "warnings".to_string(),
                deep_research_compact_warnings(warnings),
            );
        }
        digest.insert(
            "seed_research".to_string(),
            serde_json::Value::Object(compact),
        );
    }

    serde_json::Value::Object(digest)
}

fn deep_research_accepted_evidence_payload_digest(value: &serde_json::Value) -> serde_json::Value {
    let mut evidence_items = Vec::new();
    let mut evidence_items_omitted = 0usize;
    let mut seen = std::collections::HashSet::new();
    for item in value
        .get("evidence_items")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(compact) = deep_research_compact_evidence_object(item, None, &mut seen) else {
            continue;
        };
        if evidence_items.len() < DEEP_RESEARCH_MAX_DIGEST_EVIDENCE {
            evidence_items.push(compact);
        } else {
            evidence_items_omitted = evidence_items_omitted.saturating_add(1);
        }
    }
    let mut digest = serde_json::Map::new();
    let requested_status = value
        .get("collection_status")
        .and_then(serde_json::Value::as_str);
    let collection_status = if evidence_items.is_empty() {
        "degraded"
    } else if requested_status == Some("completed") {
        "completed"
    } else {
        "degraded"
    };
    digest.insert(
        "collection_status".to_string(),
        serde_json::Value::String(collection_status.to_string()),
    );
    digest.insert(
        "evidence_items".to_string(),
        serde_json::Value::Array(evidence_items),
    );
    if evidence_items_omitted > 0 {
        digest.insert(
            "evidence_items_omitted".to_string(),
            serde_json::Value::Number(serde_json::Number::from(evidence_items_omitted as u64)),
        );
    }
    if let Some(context) = value
        .get("report_context")
        .and_then(deep_research_report_context_digest)
    {
        digest.insert("report_context".to_string(), context);
    }
    serde_json::Value::Object(digest)
}

fn deep_research_report_context_digest(value: &serde_json::Value) -> Option<serde_json::Value> {
    let context = value.as_object()?;
    let mut compact_context = serde_json::Map::new();

    if let Some(plan) = context.get("plan").and_then(serde_json::Value::as_object) {
        let mut compact_plan = serde_json::Map::new();
        for (key, limit) in [
            ("report_title", 300usize),
            ("answer_shape", 100),
            ("execution_route", 100),
        ] {
            if let Some(value) = plan.get(key).and_then(serde_json::Value::as_str) {
                compact_plan.insert(
                    key.to_string(),
                    serde_json::Value::String(deep_research_digest_text(value, limit)),
                );
            }
        }
        for key in ["phases", "tracks", "stop_conditions"] {
            let values = deep_research_compact_string_array(plan.get(key), 6, 500);
            if !values.is_empty() {
                compact_plan.insert(key.to_string(), serde_json::Value::Array(values));
            }
        }
        if !compact_plan.is_empty() {
            compact_context.insert("plan".to_string(), serde_json::Value::Object(compact_plan));
        }
    }

    if let Some(checker) = context
        .get("checker")
        .and_then(serde_json::Value::as_object)
    {
        let mut compact_checker = serde_json::Map::new();
        for (key, limit) in [
            ("decision", 100usize),
            ("report_summary", 1_200),
            ("coverage_summary", 1_200),
        ] {
            if let Some(value) = checker.get(key).and_then(serde_json::Value::as_str) {
                compact_checker.insert(
                    key.to_string(),
                    serde_json::Value::String(deep_research_digest_text(value, limit)),
                );
            }
        }
        for key in [
            "verified_findings",
            "unresolved_gaps",
            "limitations",
            "contradictions",
        ] {
            let values = deep_research_compact_string_array(checker.get(key), 10, 1_000);
            if !values.is_empty() {
                compact_checker.insert(key.to_string(), serde_json::Value::Array(values));
            }
        }
        for (key, label_key) in [
            ("track_assessments", "track"),
            ("stop_condition_assessments", "stop_condition"),
        ] {
            let assessments =
                deep_research_compact_checker_assessments(checker.get(key), label_key);
            if !assessments.is_empty() {
                compact_checker.insert(key.to_string(), serde_json::Value::Array(assessments));
            }
        }
        if !compact_checker.is_empty() {
            compact_context.insert(
                "checker".to_string(),
                serde_json::Value::Object(compact_checker),
            );
        }
    }

    if let Some(verification) = context
        .get("verification")
        .and_then(serde_json::Value::as_object)
    {
        let mut compact_verification = serde_json::Map::new();
        for key in ["status", "checker_completed", "prior_checker_retained"] {
            if let Some(value) = verification.get(key) {
                let safe_value = match value {
                    serde_json::Value::String(value) => {
                        serde_json::Value::String(deep_research_digest_text(value, 100))
                    }
                    serde_json::Value::Bool(value) => serde_json::Value::Bool(*value),
                    _ => continue,
                };
                compact_verification.insert(key.to_string(), safe_value);
            }
        }
        if !compact_verification.is_empty() {
            compact_context.insert(
                "verification".to_string(),
                serde_json::Value::Object(compact_verification),
            );
        }
    }

    (!compact_context.is_empty()).then_some(serde_json::Value::Object(compact_context))
}

fn deep_research_compact_checker_assessments(
    value: Option<&serde_json::Value>,
    label_key: &str,
) -> Vec<serde_json::Value> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_object)
        .take(6)
        .filter_map(|assessment| {
            let plan_index = assessment.get("plan_index")?.as_u64()?;
            let status = assessment.get("status")?.as_str()?;
            if !matches!(status, "supported" | "bounded" | "uncovered") {
                return None;
            }
            let finding = assessment.get("finding")?.as_str()?;
            let mut compact = serde_json::Map::new();
            compact.insert("plan_index".to_string(), serde_json::json!(plan_index));
            compact.insert("status".to_string(), serde_json::json!(status));
            compact.insert(
                "finding".to_string(),
                serde_json::json!(deep_research_digest_text(finding, 1_000)),
            );
            if let Some(label) = assessment
                .get(label_key)
                .and_then(serde_json::Value::as_str)
            {
                compact.insert(
                    label_key.to_string(),
                    serde_json::json!(deep_research_digest_text(label, 300)),
                );
            }
            let source_urls =
                deep_research_compact_string_array(assessment.get("source_urls"), 4, 1_000);
            if !source_urls.is_empty() {
                compact.insert(
                    "source_urls".to_string(),
                    serde_json::Value::Array(source_urls),
                );
            }
            Some(serde_json::Value::Object(compact))
        })
        .collect()
}

pub(super) fn deep_research_collection_status(value: &serde_json::Value) -> &'static str {
    match validated_inquiry_projection(value) {
        Ok(ValidatedInquiryProjection::Inquiry { ref state, .. })
            if matches!(
                inquiry_terminal_outcome(state),
                Some(InquiryTerminalOutcome::Completed | InquiryTerminalOutcome::Qualified)
            ) =>
        {
            return "completed";
        }
        Ok(ValidatedInquiryProjection::Inquiry { .. }) | Err(_) => return "degraded",
        Ok(ValidatedInquiryProjection::LegacyCheckedLoop) => {}
    }
    let mode = value
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let research_status = value
        .pointer("/research/status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let has_completed_evidence = value
        .pointer("/research/results")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|results| {
            !results.is_empty()
                && results
                    .iter()
                    .all(deep_research_result_has_completed_evidence)
        });
    let has_reportable_evidence = ["research", "seed_research"].into_iter().any(|field| {
        value
            .get(field)
            .and_then(|research| research.get("results"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|results| {
                results
                    .iter()
                    .any(deep_research_result_has_completed_evidence)
            })
    });
    let checker_finalized = value
        .pointer("/checker/decision")
        .and_then(serde_json::Value::as_str)
        == Some("finalize");
    let verification_degraded = value
        .pointer("/verification/status")
        .and_then(serde_json::Value::as_str)
        == Some("degraded");
    if mode.contains("failed")
        || research_status.eq_ignore_ascii_case("failed")
        || value.get("error").is_some()
    {
        "failed"
    } else if checker_finalized && value.get("runtime_error").is_none() && has_completed_evidence {
        // Search and fetch backends may return partial transport coverage even
        // when every retained evidence item is schema-valid. Once the
        // independent checker explicitly finalizes that cumulative package,
        // preserve the missing searches as report limitations instead of
        // replacing a useful source-backed report with a recovery artifact.
        "completed"
    } else if verification_degraded
        && value.get("runtime_error").is_none()
        && has_reportable_evidence
    {
        // A checker timeout does not erase already validated, traceable
        // evidence. Complete the collection and make the missing independent
        // verification explicit in the report instead of emitting Recovery.
        "completed"
    } else if value.get("runtime_error").is_some()
        || mode.contains("fallback")
        || !research_status.eq_ignore_ascii_case("success")
        || !has_completed_evidence
    {
        "degraded"
    } else {
        "completed"
    }
}

pub(super) fn deep_research_result_has_completed_evidence(result: &serde_json::Value) -> bool {
    if result.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        return false;
    }
    let Some(structured) = result
        .get("structured")
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    let has_summary = structured
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|summary| !summary.trim().is_empty());
    let has_confidence = structured
        .get("confidence")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|confidence| !confidence.trim().is_empty());
    let has_traceable_source = structured
        .get("sources")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|sources| {
            sources
                .iter()
                .any(|source| deep_research_traceable_source_anchor(source).is_some())
        });
    has_summary && has_confidence && has_traceable_source
}

pub(super) fn deep_research_workflow_metadata_digest(
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let sanitized = deep_research_sanitize_workflow_metadata(metadata);
    let Some(workflow) = sanitized.get("dynamic_workflow") else {
        let (evidence_items, evidence_items_omitted) =
            deep_research_collect_structured_evidence_bounded(&sanitized);
        return if evidence_items.is_empty() {
            serde_json::json!({})
        } else {
            let mut research_run = serde_json::Map::new();
            research_run.insert(
                "evidence_items".to_string(),
                serde_json::Value::Array(evidence_items),
            );
            if evidence_items_omitted > 0 {
                research_run.insert(
                    "evidence_items_omitted".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(
                        evidence_items_omitted as u64,
                    )),
                );
            }
            serde_json::json!({ "research_run": research_run })
        };
    };
    let mut dynamic = serde_json::Map::new();
    copy_json_field(&mut dynamic, workflow, "status");
    copy_json_field(&mut dynamic, workflow, "last_sequence");

    if let Some(steps) = workflow
        .pointer("/snapshot/steps")
        .and_then(serde_json::Value::as_object)
    {
        let mut compact_steps = Vec::new();
        for (index, step) in steps.values().enumerate() {
            let mut compact = serde_json::Map::new();
            compact.insert(
                "step".to_string(),
                serde_json::Value::Number(serde_json::Number::from(index + 1)),
            );
            copy_json_field(&mut compact, step, "status");
            copy_json_field(&mut compact, step, "attempt");
            if let Some(output) = step.get("output") {
                if let Some(metadata) = output.get("metadata") {
                    compact.insert(
                        "counts".to_string(),
                        deep_research_compact_count_metadata(metadata),
                    );
                }
                if let Some(warnings) = output.get("warnings") {
                    compact.insert(
                        "warnings".to_string(),
                        deep_research_compact_warnings(warnings),
                    );
                }
            }
            compact_steps.push(serde_json::Value::Object(compact));
        }
        dynamic.insert("steps".to_string(), serde_json::Value::Array(compact_steps));
    }
    let (evidence_items, evidence_items_omitted) =
        deep_research_collect_structured_evidence_bounded(&sanitized);
    dynamic.insert(
        "evidence_items".to_string(),
        serde_json::Value::Array(evidence_items),
    );
    if evidence_items_omitted > 0 {
        dynamic.insert(
            "evidence_items_omitted".to_string(),
            serde_json::Value::Number(serde_json::Number::from(evidence_items_omitted as u64)),
        );
    }

    serde_json::json!({ "research_run": dynamic })
}

pub(super) fn deep_research_sanitize_parallel_task_values(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let is_parallel_task = map
                .get("tool")
                .or_else(|| map.get("name"))
                .or_else(|| map.get("tool_name"))
                .and_then(serde_json::Value::as_str)
                == Some("parallel_task");
            if is_parallel_task {
                deep_research_sanitize_parallel_task_object(map);
            }
            for value in map.values_mut() {
                deep_research_sanitize_parallel_task_values(value);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                deep_research_sanitize_parallel_task_values(item);
            }
        }
        _ => {}
    }
}

pub(super) fn deep_research_sanitize_parallel_task_object(
    map: &mut serde_json::Map<String, serde_json::Value>,
) {
    let sanitized_results = map
        .get("metadata")
        .and_then(|metadata| metadata.get("results"))
        .and_then(serde_json::Value::as_array)
        .map(|results| {
            let mut successes = Vec::new();
            let mut failed_tasks = Vec::new();
            for result in results {
                let success = result
                    .get("success")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if success {
                    successes.push(deep_research_sanitize_parallel_result(result, true));
                } else {
                    failed_tasks.push(deep_research_sanitize_parallel_result(result, false));
                }
            }
            (successes, failed_tasks)
        });

    if let Some((successes, failed_tasks)) = sanitized_results {
        if let Some(metadata) = map
            .get_mut("metadata")
            .and_then(serde_json::Value::as_object_mut)
        {
            metadata.insert(
                "results".to_string(),
                serde_json::Value::Array(successes.clone()),
            );
        }
        if !failed_tasks.is_empty() {
            let warnings = map
                .entry("warnings".to_string())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if let Some(warnings) = warnings.as_object_mut() {
                warnings.insert(
                    "failed_tasks".to_string(),
                    serde_json::Value::Array(failed_tasks),
                );
            }
        }
        map.remove("output");
    } else if let Some(output) = map.remove("output") {
        map.insert(
            "output_summary".to_string(),
            serde_json::Value::String(deep_research_compact_json_text(
                &output,
                DEEP_RESEARCH_PROMPT_SUCCESS_OUTPUT_LIMIT,
            )),
        );
    }
}

pub(super) fn deep_research_sanitize_parallel_result(
    result: &serde_json::Value,
    success: bool,
) -> serde_json::Value {
    let mut next = serde_json::Map::new();
    for key in [
        "task_id",
        "session_id",
        "agent",
        "success",
        "artifact_id",
        "artifact_uri",
        "output_bytes",
        "truncated_for_context",
        "retry_attempts",
        "structured_error",
    ] {
        if let Some(value) = result.get(key) {
            next.insert(key.to_string(), value.clone());
        }
    }

    if success {
        if let Some(structured) = result.get("structured") {
            if let Some(structured) = deep_research_verified_structured_evidence(result, structured)
            {
                next.insert("structured".to_string(), structured);
            } else {
                next.insert(
                    "structured_error".to_string(),
                    serde_json::Value::String(
                        "Delegated evidence had no source observed by a successful research tool."
                            .to_string(),
                    ),
                );
            }
        } else if let Some(output) = result
            .get("output_excerpt")
            .or_else(|| result.get("output"))
        {
            let parsed = output
                .as_str()
                .and_then(parse_embedded_structured_evidence_json)
                .or_else(|| output.is_object().then(|| output.clone()));
            if let Some(structured) = parsed.and_then(|structured| {
                deep_research_verified_structured_evidence(result, &structured)
            }) {
                next.insert("structured".to_string(), structured);
            } else {
                next.insert(
                    "structured_error".to_string(),
                    serde_json::Value::String(
                        "Delegated task returned no verified schema-shaped evidence.".to_string(),
                    ),
                );
            }
        }
    } else {
        let summary = result
            .get("error_message")
            .or_else(|| result.get("output_excerpt"))
            .or_else(|| result.get("output"))
            .or_else(|| result.get("error"))
            .map(deep_research_failure_summary)
            .unwrap_or_else(|| {
                "Delegated task failed before returning usable evidence.".to_string()
            });
        next.insert(
            "error_summary".to_string(),
            serde_json::Value::String(summary),
        );
    }

    serde_json::Value::Object(next)
}
