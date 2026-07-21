//! Report discovery, terminal authority, and transient state for DeepResearch runs.

use super::*;

pub(super) fn research_report_view_spec(
    output: &str,
    workspace: &Path,
) -> Option<remote_ui::ViewSpec> {
    let artifacts = research_report_artifacts_from_output(output, workspace)?;
    remote_ui::local_file_view(&artifacts.html).ok()
}

pub(super) fn arm_deep_research_report_resume(
    loop_remaining: &mut usize,
    resume_used: &mut bool,
) -> bool {
    if *resume_used {
        return false;
    }
    *resume_used = true;
    *loop_remaining = (*loop_remaining).max(1);
    true
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
}

impl DeepResearchWorkflowSnapshot {
    pub(super) fn reset_for_run(&mut self) {
        *self = Self::default();
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

pub(super) fn deep_research_evidence_package_is_complete_for_query(
    query: &str,
    evidence_scope: DeepResearchEvidenceScope,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    let canonical_output =
        deep_research_canonical_workflow_output(workflow_output, workflow_metadata);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&canonical_output) else {
        return false;
    };
    match validated_inquiry_projection(&value) {
        Ok(ValidatedInquiryProjection::Inquiry { ref state, .. }) => {
            inquiry_terminal_outcome(state) == Some(InquiryTerminalOutcome::Completed)
        }
        Err(_) => false,
        Ok(ValidatedInquiryProjection::LegacyCheckedLoop) => {
            let _ = (query, evidence_scope);
            legacy_checked_loop_evidence_package_is_complete(
                &value,
                &canonical_output,
                workflow_metadata,
            )
        }
    }
}

/// Historical checked-loop output compatibility only. Current runs never use
/// checker fields or source-family budgets as terminal authority.
fn legacy_checked_loop_evidence_package_is_complete(
    value: &serde_json::Value,
    canonical_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    if deep_research_workflow_needs_recovery_report_with_metadata(
        canonical_output,
        workflow_metadata,
    ) {
        return false;
    }
    if value.get("plan").is_some() {
        if value
            .pointer("/checker/decision")
            .and_then(serde_json::Value::as_str)
            != Some("finalize")
        {
            return false;
        }
        let evidence = deep_research_collect_structured_evidence(value);
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
                        Path::new(&anchor)
                            .components()
                            .next()
                            .map(|component| component.as_os_str().to_string_lossy().to_string())
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
    let evidence = deep_research_collect_structured_evidence(value);
    !evidence.is_empty()
}

pub(super) fn deep_research_report_outcome_for_workflow(
    query: &str,
    evidence_scope: DeepResearchEvidenceScope,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> DeepResearchRunOutcome {
    let canonical_output =
        deep_research_canonical_workflow_output(workflow_output, workflow_metadata);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&canonical_output) else {
        return DeepResearchRunOutcome::Degraded;
    };
    match validated_inquiry_projection(&value) {
        Ok(ValidatedInquiryProjection::Inquiry { ref state, .. }) => {
            let Some(outcome) = inquiry_terminal_outcome(state) else {
                return DeepResearchRunOutcome::Degraded;
            };
            match outcome {
                InquiryTerminalOutcome::Completed => DeepResearchRunOutcome::Completed,
                InquiryTerminalOutcome::Qualified => DeepResearchRunOutcome::Qualified,
                InquiryTerminalOutcome::Exhausted => DeepResearchRunOutcome::Degraded,
            }
        }
        Err(_) => DeepResearchRunOutcome::Degraded,
        Ok(ValidatedInquiryProjection::LegacyCheckedLoop) => legacy_checked_loop_report_outcome(
            query,
            evidence_scope,
            &value,
            &canonical_output,
            workflow_metadata,
        ),
    }
}

/// Historical checked-loop output compatibility only. New runs classify their
/// outcome exclusively from the replayed Inquiry contract.
fn legacy_checked_loop_report_outcome(
    query: &str,
    evidence_scope: DeepResearchEvidenceScope,
    workflow: &serde_json::Value,
    canonical_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> DeepResearchRunOutcome {
    if deep_research_workflow_needs_recovery_report_with_metadata(
        canonical_output,
        workflow_metadata,
    ) {
        return DeepResearchRunOutcome::Degraded;
    }
    let explicitly_qualified = workflow
        .pointer("/verification/status")
        .and_then(serde_json::Value::as_str)
        == Some("degraded")
        || workflow
            .pointer("/checker/decision")
            .and_then(serde_json::Value::as_str)
            == Some("degrade")
        || workflow
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|mode| mode.contains("degraded"));
    if explicitly_qualified {
        return DeepResearchRunOutcome::Qualified;
    }
    if deep_research_evidence_package_is_complete_for_query(
        query,
        evidence_scope,
        canonical_output,
        workflow_metadata,
    ) {
        DeepResearchRunOutcome::Completed
    } else {
        // Traceable evidence can remain useful when the final checker is
        // unavailable or explicitly leaves bounded gaps. Publish that report
        // with a qualified lifecycle instead of discarding the evidence into
        // a generic recovery artifact or claiming full completion.
        DeepResearchRunOutcome::Qualified
    }
}

/// Validate the successful-report authority after synthesis.
///
/// Inquiry-backed runs must have committed outline, section, and passing audit
/// events. `Ok(None)` means the output is a legacy checked-loop run whose
/// publication remains governed by the legacy report validators.
pub(super) fn deep_research_inquiry_publication_outcome(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Result<Option<DeepResearchRunOutcome>, String> {
    let canonical = deep_research_canonical_workflow_output(workflow_output, workflow_metadata);
    let value = serde_json::from_str::<serde_json::Value>(&canonical)
        .map_err(|error| format!("decode DeepResearch workflow for publication: {error}"))?;
    validated_inquiry_publication_outcome(&value).map(|outcome| {
        outcome.map(|outcome| match outcome {
            InquiryTerminalOutcome::Completed => DeepResearchRunOutcome::Completed,
            InquiryTerminalOutcome::Qualified => DeepResearchRunOutcome::Qualified,
            InquiryTerminalOutcome::Exhausted => DeepResearchRunOutcome::Degraded,
        })
    })
}

#[cfg(test)]
mod deep_research_workflow_snapshot_tests {
    use super::*;

    #[test]
    pub(super) fn reset_for_run_discards_prior_transient_values() {
        let mut snapshot = DeepResearchWorkflowSnapshot {
            output: Some("stale output".to_string()),
            metadata: Some(serde_json::json!({"stale": true})),
            args: Some(serde_json::json!({"run_id": "old"})),
            last_synthesis_text: Some("stale synthesis".to_string()),
        };

        snapshot.reset_for_run();

        assert!(snapshot.output.is_none());
        assert!(snapshot.metadata.is_none());
        assert!(snapshot.args.is_none());
        assert!(snapshot.last_synthesis_text.is_none());
    }

    #[test]
    pub(super) fn clear_removes_all_transient_run_data() {
        let mut snapshot = DeepResearchWorkflowSnapshot {
            output: Some("output".to_string()),
            metadata: Some(serde_json::json!({"source": "workflow"})),
            args: Some(serde_json::json!({"run_id": "run"})),
            last_synthesis_text: Some("synthesis".to_string()),
        };

        snapshot.clear();

        assert!(snapshot.output.is_none());
        assert!(snapshot.metadata.is_none());
        assert!(snapshot.args.is_none());
        assert!(snapshot.last_synthesis_text.is_none());
    }
}
