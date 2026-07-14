//! Report discovery, convergence, and bounded recovery for DeepResearch runs.

use super::*;

pub(super) fn research_report_view_spec(
    output: &str,
    workspace: &Path,
) -> Option<remote_ui::ViewSpec> {
    let artifacts = research_report_artifacts_from_output(output, workspace)?;
    remote_ui::local_file_view(&artifacts.html).ok()
}

pub(super) fn deep_research_report_view_spec_for_current_run(
    output: &str,
    workspace: &Path,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    baseline: &DeepResearchReportArtifactBaseline,
) -> Option<remote_ui::ViewSpec> {
    let artifacts = deep_research_report_artifacts_from_output_for_current_run(
        output,
        workspace,
        query,
        workflow_output,
        workflow_metadata,
        baseline,
    )?;
    remote_ui::local_file_view(&artifacts.html).ok()
}

#[cfg(test)]
pub(super) fn deep_research_report_is_missing(
    deep_research_active: bool,
    report_already_ready: bool,
    query: Option<&str>,
    review_text: &str,
    workspace: &Path,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    deep_research_report_is_missing_since(
        deep_research_active,
        report_already_ready,
        query,
        review_text,
        workspace,
        workflow_output,
        workflow_metadata,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn deep_research_report_is_missing_since(
    deep_research_active: bool,
    report_already_ready: bool,
    query: Option<&str>,
    review_text: &str,
    workspace: &Path,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    baseline: Option<&DeepResearchReportArtifactBaseline>,
) -> bool {
    if !deep_research_active {
        return false;
    }
    match query {
        Some(query) => {
            let validate = |output: &str| match baseline {
                Some(baseline) => deep_research_report_artifacts_from_output_for_current_run(
                    output,
                    workspace,
                    query,
                    workflow_output,
                    workflow_metadata,
                    baseline,
                ),
                None => deep_research_report_artifacts_from_output_for_query(
                    output,
                    workspace,
                    query,
                    workflow_output,
                    workflow_metadata,
                ),
            };
            if validate(review_text).is_some() {
                return false;
            }

            // `report_already_ready` is only a hint that an earlier layer
            // captured the view. Rebuild its deterministic marker and validate
            // the files again so a later repair/verification tool cannot leave
            // a broken artifact pair behind while the bool latch stays true.
            if report_already_ready {
                let marker = format!(
                    "{RESEARCH_VIEW_MARKER} .a3s/research/{}/index.html",
                    deep_research_report_slug(query)
                );
                return validate(&marker).is_none();
            }
            true
        }
        None => research_report_artifacts_from_output(review_text, workspace).is_none(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResearchReportViewAction {
    OpenNow,
    DeferUntilDeepResearchComplete,
}

pub(super) fn research_report_view_action(deep_research_active: bool) -> ResearchReportViewAction {
    if deep_research_active {
        ResearchReportViewAction::DeferUntilDeepResearchComplete
    } else {
        ResearchReportViewAction::OpenNow
    }
}

pub(super) fn arm_deep_research_report_repair(
    loop_remaining: &mut usize,
    repair_used: &mut bool,
) -> bool {
    if *repair_used {
        return false;
    }
    *repair_used = true;
    *loop_remaining = (*loop_remaining).max(1);
    true
}

#[derive(Debug)]
pub(super) enum DeepResearchReportRecovery {
    CompletedMaterialized { artifacts: ResearchReportArtifacts },
    RecoveryMaterialized { artifacts: ResearchReportArtifacts },
    RepairPassArmed,
    Missing(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum DeepResearchRunOutcome {
    #[default]
    Active,
    Completed,
    Qualified,
    Degraded,
}

impl DeepResearchRunOutcome {
    pub(super) fn report_ready(self) -> bool {
        matches!(self, Self::Completed | Self::Qualified)
    }

    pub(super) fn ensure_smoke_success(
        self,
        artifacts: &ResearchReportArtifacts,
    ) -> anyhow::Result<()> {
        match self {
            Self::Completed | Self::Qualified => Ok(()),
            Self::Degraded => anyhow::bail!(
                "DeepResearch smoke produced only a degraded recovery report at {}",
                artifacts.html.display()
            ),
            Self::Active => anyhow::bail!("DeepResearch smoke ended without a terminal outcome"),
        }
    }
}

/// Ephemeral host data retained between evidence collection and report
/// synthesis. Durable run truth belongs to the event journal; this snapshot
/// only carries values that cannot be reconstructed from the run projection.
#[derive(Debug, Default)]
pub(super) struct DeepResearchWorkflowSnapshot {
    pub(super) output: Option<String>,
    pub(super) metadata: Option<serde_json::Value>,
    pub(super) args: Option<serde_json::Value>,
    pub(super) last_synthesis_text: Option<String>,
    pub(super) report_baseline: Option<DeepResearchReportArtifactBaseline>,
}

impl DeepResearchWorkflowSnapshot {
    pub(super) fn reset_for_run(&mut self, report_baseline: DeepResearchReportArtifactBaseline) {
        *self = Self {
            report_baseline: Some(report_baseline),
            ..Self::default()
        };
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod deep_research_workflow_snapshot_tests {
    use super::*;

    #[test]
    pub(super) fn reset_for_run_discards_prior_transient_values_and_keeps_baseline() {
        let mut snapshot = DeepResearchWorkflowSnapshot {
            output: Some("stale output".to_string()),
            metadata: Some(serde_json::json!({"stale": true})),
            args: Some(serde_json::json!({"run_id": "old"})),
            last_synthesis_text: Some("stale synthesis".to_string()),
            report_baseline: None,
        };

        snapshot.reset_for_run(DeepResearchReportArtifactBaseline::default());

        assert!(snapshot.output.is_none());
        assert!(snapshot.metadata.is_none());
        assert!(snapshot.args.is_none());
        assert!(snapshot.last_synthesis_text.is_none());
        assert!(snapshot.report_baseline.is_some());
    }

    #[test]
    pub(super) fn clear_removes_all_transient_run_data() {
        let mut snapshot = DeepResearchWorkflowSnapshot {
            output: Some("output".to_string()),
            metadata: Some(serde_json::json!({"source": "workflow"})),
            args: Some(serde_json::json!({"run_id": "run"})),
            last_synthesis_text: Some("synthesis".to_string()),
            report_baseline: Some(DeepResearchReportArtifactBaseline::default()),
        };

        snapshot.clear();

        assert!(snapshot.output.is_none());
        assert!(snapshot.metadata.is_none());
        assert!(snapshot.args.is_none());
        assert!(snapshot.last_synthesis_text.is_none());
        assert!(snapshot.report_baseline.is_none());
    }
}

pub(super) fn deep_research_evidence_package_is_complete_for_query(
    query: &str,
    evidence_scope: DeepResearchEvidenceScope,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    if deep_research_workflow_needs_recovery_report(workflow_output) {
        return false;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(workflow_output) {
        if value.get("plan").is_some() {
            if value
                .pointer("/checker/decision")
                .and_then(serde_json::Value::as_str)
                != Some("finalize")
            {
                return false;
            }
            let evidence = deep_research_collect_structured_evidence(&value);
            let source_families = evidence
                .iter()
                .flat_map(|item| item.get("sources").and_then(serde_json::Value::as_array))
                .flatten()
                .filter_map(deep_research_traceable_source_anchor)
                .filter_map(|anchor| {
                    reqwest::Url::parse(&anchor)
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
                        .or_else(|| {
                            Path::new(&anchor).components().next().map(|component| {
                                component.as_os_str().to_string_lossy().to_string()
                            })
                        })
                })
                .collect::<HashSet<_>>()
                .len();
            let required_families = value
                .pointer("/plan/budget/min_source_families")
                .and_then(serde_json::Value::as_u64)
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or(1)
                .clamp(1, 5);
            return !evidence.is_empty() && source_families >= required_families;
        }
    }
    let evidence = serde_json::from_str::<serde_json::Value>(workflow_output)
        .ok()
        .map(|value| deep_research_collect_structured_evidence(&value))
        .unwrap_or_default();
    let _ = (query, evidence_scope, workflow_metadata);
    !evidence.is_empty()
}

pub(super) struct DeepResearchConvergenceContext<'a> {
    pub(super) query: &'a str,
    pub(super) evidence_scope: DeepResearchEvidenceScope,
    pub(super) workflow_output: &'a str,
    pub(super) workflow_metadata: Option<&'a serde_json::Value>,
    pub(super) args: &'a serde_json::Value,
    pub(super) elapsed: Duration,
    pub(super) total_budget_ms: u64,
    pub(super) finalization_reserve_ms: u64,
}

pub(super) fn deep_research_convergence_input(
    context: DeepResearchConvergenceContext<'_>,
) -> ConvergenceInput {
    let DeepResearchConvergenceContext {
        query,
        evidence_scope,
        workflow_output,
        workflow_metadata,
        args,
        elapsed,
        total_budget_ms,
        finalization_reserve_ms,
    } = context;
    let mut evidence = serde_json::from_str::<serde_json::Value>(workflow_output)
        .ok()
        .map(|value| deep_research_collect_structured_evidence(&value))
        .unwrap_or_default();
    if let Some(metadata) = workflow_metadata {
        evidence.extend(deep_research_collect_structured_evidence(metadata));
    }
    let mut sources = HashSet::new();
    let mut authoritative_sources = HashSet::new();
    let mut contradictions = 0usize;
    let mut gaps = 0usize;
    for item in &evidence {
        contradictions = contradictions.saturating_add(
            item.get("contradictions")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
        );
        gaps = gaps.saturating_add(
            item.get("gaps")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
        );
        for source in item
            .get("sources")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(anchor) = deep_research_traceable_source_anchor(source) else {
                continue;
            };
            let reliability = source
                .get("reliability")
                .or_else(|| source.get("publisher"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let authoritative = reliability.contains("official")
                || reliability.contains("authoritative")
                || reliability.contains("primary")
                || anchor.contains(".gov.")
                || anchor.contains("://gov.")
                || anchor.contains(".gov/");
            sources.insert(anchor.clone());
            if authoritative {
                authoritative_sources.insert(anchor);
            }
        }
    }
    let output_value = serde_json::from_str::<serde_json::Value>(workflow_output).ok();
    let completed_rounds = output_value
        .as_ref()
        .and_then(|value| value.pointer("/research/completed_rounds"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_else(|| usize::from(!evidence.is_empty()));
    let max_rounds = output_value
        .as_ref()
        .and_then(|value| value.pointer("/research/max_rounds"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            args.pointer("/input/config/local_research_rounds")?
                .as_u64()
        })
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(1)
        .max(1);
    let elapsed_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
    ConvergenceInput {
        accepted_evidence: evidence.len(),
        traceable_sources: sources.len(),
        authoritative_sources: authoritative_sources.len(),
        unresolved_contradictions: contradictions,
        unresolved_gaps: gaps,
        completed_rounds,
        max_rounds,
        rounds_without_material_gain: if evidence.is_empty() {
            completed_rounds
        } else {
            0
        },
        remaining_ms: total_budget_ms.saturating_sub(elapsed_ms),
        finalization_reserve_ms,
        evidence_package_complete: deep_research_evidence_package_is_complete_for_query(
            query,
            evidence_scope,
            workflow_output,
            workflow_metadata,
        ),
    }
}

pub(super) fn recover_missing_deep_research_report(
    workspace: &Path,
    query: Option<&str>,
    review_text: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    loop_remaining: &mut usize,
    repair_used: &mut bool,
) -> DeepResearchReportRecovery {
    let Some(query) = query else {
        return DeepResearchReportRecovery::Missing(
            "DeepResearch ended without a valid local HTML report marker".to_string(),
        );
    };

    if deep_research_workflow_needs_recovery_report(workflow_output) {
        return match materialize_deep_research_recovery_report(
            workspace,
            query,
            review_text,
            workflow_output,
            workflow_metadata,
        ) {
            Ok(artifacts) => {
                *loop_remaining = 0;
                DeepResearchReportRecovery::RecoveryMaterialized { artifacts }
            }
            Err(error) => DeepResearchReportRecovery::Missing(format!(
                "DeepResearch recovery report failed: {error}"
            )),
        };
    }

    if let Some(artifacts) = materialize_deep_research_completed_report_from_answer_text(
        workspace,
        query,
        review_text,
        workflow_output,
        workflow_metadata,
    ) {
        *loop_remaining = 0;
        return DeepResearchReportRecovery::CompletedMaterialized { artifacts };
    }

    // A deterministic query slug may already contain a report from an older
    // run. Prefer this run's answer and evidence before considering that file.
    if let Some(artifacts) = materialize_deep_research_completed_report_from_markdown(
        workspace,
        query,
        workflow_output,
        workflow_metadata,
    ) {
        *loop_remaining = 0;
        return DeepResearchReportRecovery::CompletedMaterialized { artifacts };
    }

    if let Some(artifacts) = materialize_deep_research_completed_report_from_workflow_evidence(
        workspace,
        query,
        workflow_output,
        workflow_metadata,
    ) {
        *loop_remaining = 0;
        return DeepResearchReportRecovery::CompletedMaterialized { artifacts };
    }

    if arm_deep_research_report_repair(loop_remaining, repair_used) {
        return DeepResearchReportRecovery::RepairPassArmed;
    }

    match materialize_deep_research_recovery_report(
        workspace,
        query,
        review_text,
        workflow_output,
        workflow_metadata,
    ) {
        Ok(artifacts) => {
            *loop_remaining = 0;
            DeepResearchReportRecovery::RecoveryMaterialized { artifacts }
        }
        Err(error) => DeepResearchReportRecovery::Missing(format!(
            "DeepResearch ended without a valid local HTML report marker and recovery report failed ({error})"
        )),
    }
}

pub(super) fn materialize_deep_research_timeout_completed_report(
    workspace: &Path,
    query: &str,
    streamed_text: &str,
    prior_synthesis_text: Option<&str>,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Option<ResearchReportArtifacts> {
    if deep_research_workflow_needs_recovery_report(workflow_output) {
        return None;
    }
    [Some(streamed_text), prior_synthesis_text]
        .into_iter()
        .flatten()
        .filter(|text| !text.trim().is_empty() && !deep_research_output_has_internal_leak(text))
        .find_map(|text| {
            materialize_deep_research_completed_report_from_answer_text(
                workspace,
                query,
                text,
                workflow_output,
                workflow_metadata,
            )
        })
        .or_else(|| {
            materialize_deep_research_completed_report_from_workflow_evidence(
                workspace,
                query,
                workflow_output,
                workflow_metadata,
            )
        })
        .or_else(|| {
            materialize_deep_research_completed_report_from_markdown(
                workspace,
                query,
                workflow_output,
                workflow_metadata,
            )
        })
}

pub(super) fn recover_deep_research_workflow_state_for_report_timeout(
    workspace: &Path,
    _query: &str,
    workflow_args: Option<&serde_json::Value>,
    workflow_output: String,
    workflow_metadata: Option<serde_json::Value>,
) -> (String, Option<serde_json::Value>) {
    if deep_research_workflow_state_has_structured_evidence(
        &workflow_output,
        workflow_metadata.as_ref(),
    ) {
        return (workflow_output, workflow_metadata);
    }

    let mut recovered_without_evidence = None;
    let Some(args) = workflow_args else {
        return (workflow_output, workflow_metadata);
    };
    if let Some(recovered) = recover_deep_research_workflow_run_from_store(workspace, args) {
        let recovered_output = recovered.output.unwrap_or_default();
        let recovered_metadata = Some(recovered.metadata);
        if deep_research_workflow_state_has_structured_evidence(
            &recovered_output,
            recovered_metadata.as_ref(),
        ) {
            return (recovered_output, recovered_metadata);
        }
        recovered_without_evidence = Some((recovered_output, recovered_metadata));
    }

    if workflow_output.trim().is_empty() && workflow_metadata.is_none() {
        recovered_without_evidence.unwrap_or((workflow_output, workflow_metadata))
    } else {
        (workflow_output, workflow_metadata)
    }
}

pub(super) fn deep_research_workflow_state_has_structured_evidence(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(workflow_output) {
        if !deep_research_collect_structured_evidence(&value).is_empty() {
            return true;
        }
        let digest = deep_research_workflow_output_digest(&value);
        if !deep_research_collect_structured_evidence(&digest).is_empty() {
            return true;
        }
    }
    workflow_metadata.is_some_and(|metadata| {
        !deep_research_collect_structured_evidence(metadata).is_empty()
            || !deep_research_collect_structured_evidence(&deep_research_workflow_metadata_digest(
                metadata,
            ))
            .is_empty()
    })
}

pub(super) fn nonempty_report_section(text: &str, fallback: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}
