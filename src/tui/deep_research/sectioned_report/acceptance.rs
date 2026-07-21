//! Bounded structural and semantic acceptance with durable targeted repairs.

use super::*;

pub(super) struct ReportAcceptanceContext<'a> {
    pub(super) session: &'a AgentSession,
    pub(super) query: &'a str,
    pub(super) run_id: &'a str,
    pub(super) canonical_workflow_output: &'a str,
    pub(super) outline: &'a ResearchOutline,
    pub(super) events: &'a mut Vec<InquiryEvent>,
    pub(super) state: &'a mut InquiryState,
    pub(super) evidence: &'a [AcceptedEvidence],
    pub(super) sections: &'a mut BTreeMap<String, SectionGeneration>,
    pub(super) resume_mode: recovery::ReportResumeMode,
    pub(super) deadline: &'a ReportDeadline,
}

pub(super) struct AcceptedReport {
    pub(super) assembled: AssembledReportText,
    pub(super) audit: ReportAudit,
    pub(super) frame: ReportFrame,
}

pub(super) async fn accept_report(
    mut context: ReportAcceptanceContext<'_>,
) -> Result<AcceptedReport, String> {
    let mut frame = generate_frame(
        context.session,
        context.query,
        context.run_id,
        context.canonical_workflow_output,
        context.outline,
        context.state,
        context.evidence,
        None,
        context.deadline,
    )
    .await?;
    let (used_evidence, resolved_used_evidence) =
        resolve_report_evidence(context.sections, context.outline, context.evidence)?;
    let (initial_assembled, initial_structural_audit) = assemble_and_audit(
        &frame,
        context.outline,
        context.state,
        &used_evidence,
        &resolved_used_evidence,
        context.evidence,
    )?;
    let initial_semantic_audit = semantic_audit::audit_report_semantics(
        context.session,
        context.query,
        context.run_id,
        "semantic_audit_1",
        context.outline,
        context.state,
        context.sections,
        &frame,
        context.evidence,
        context.deadline,
    )
    .await?;
    let initial_audit =
        semantic_audit::merge_semantic_audit(initial_structural_audit, &initial_semantic_audit);

    if context.resume_mode == recovery::ReportResumeMode::VerifyCompleted {
        if !initial_audit.passed {
            return Err(format!(
                "durable completed report failed structural or semantic re-audit: {}",
                initial_audit.reason
            ));
        }
        return Ok(AcceptedReport {
            assembled: initial_assembled,
            audit: initial_audit,
            frame,
        });
    }

    let recovering_failed_audit =
        context.resume_mode == recovery::ReportResumeMode::RecoverFailedAudit;
    let revision_required = match context.state.phase {
        InquiryPhase::Drafting => {
            if !recovering_failed_audit {
                return Err(
                    "report Inquiry returned to Drafting without a failed audit".to_string()
                );
            }
            if initial_audit.passed {
                redraft_first_section(&mut context).await?;
                record_report_audit(&mut context, &initial_audit).await?;
                false
            } else {
                true
            }
        }
        InquiryPhase::Auditing => {
            record_report_audit(&mut context, &initial_audit).await?;
            !initial_audit.passed
        }
        phase => {
            return Err(format!("report Inquiry cannot audit from phase {phase:?}"));
        }
    };
    if !revision_required {
        return Ok(AcceptedReport {
            assembled: initial_assembled,
            audit: initial_audit,
            frame,
        });
    }

    apply_targeted_report_repair(
        &mut context,
        &mut frame,
        &initial_audit,
        &initial_semantic_audit,
        &resolved_used_evidence,
    )
    .await?;
    let (revised_used_evidence, revised_resolved_evidence) =
        resolve_report_evidence(context.sections, context.outline, context.evidence)?;
    let (revised_assembled, revised_structural_audit) = assemble_and_audit(
        &frame,
        context.outline,
        context.state,
        &revised_used_evidence,
        &revised_resolved_evidence,
        context.evidence,
    )?;
    // The first repair is followed by a fresh audit of every target. Besides
    // checking changed prose, this independent pass can catch a false clear
    // from the initial model audit, as observed in the real-model v26 run.
    let revised_semantic_audit = semantic_audit::audit_report_semantics(
        context.session,
        context.query,
        context.run_id,
        "semantic_audit_2",
        context.outline,
        context.state,
        context.sections,
        &frame,
        context.evidence,
        context.deadline,
    )
    .await?;
    let revised_audit =
        semantic_audit::merge_semantic_audit(revised_structural_audit, &revised_semantic_audit);
    record_report_audit(&mut context, &revised_audit).await?;
    if revised_audit.passed {
        return Ok(AcceptedReport {
            assembled: revised_assembled,
            audit: revised_audit,
            frame,
        });
    }

    let final_changed_targets = apply_targeted_report_repair(
        &mut context,
        &mut frame,
        &revised_audit,
        &revised_semantic_audit,
        &revised_resolved_evidence,
    )
    .await?;
    let (final_used_evidence, final_resolved_evidence) =
        resolve_report_evidence(context.sections, context.outline, context.evidence)?;
    let (final_assembled, final_structural_audit) = assemble_and_audit(
        &frame,
        context.outline,
        context.state,
        &final_used_evidence,
        &final_resolved_evidence,
        context.evidence,
    )?;
    // Unchanged targets retain their exact-content result from audit 2. Only
    // targets replaced by the final repair are redelivered, with their stable
    // report ordinal preserved in each durable Flow ID.
    let final_target_reviews = semantic_audit::audit_report_semantics_for_targets(
        context.session,
        context.query,
        context.run_id,
        "semantic_audit_3",
        context.outline,
        context.state,
        context.sections,
        &frame,
        context.evidence,
        &final_changed_targets,
        context.deadline,
    )
    .await?;
    let final_semantic_audit = semantic_audit::merge_reaudited_targets(
        revised_semantic_audit,
        final_target_reviews,
        context.outline,
        context.sections,
        &frame,
        context.state,
    )?;
    let final_audit =
        semantic_audit::merge_semantic_audit(final_structural_audit, &final_semantic_audit);
    record_report_audit(&mut context, &final_audit).await?;
    if !final_audit.passed {
        return Err(format!(
            "sectioned report remained invalid after its bounded targeted repairs: {}",
            final_audit.reason
        ));
    }
    Ok(AcceptedReport {
        assembled: final_assembled,
        audit: final_audit,
        frame,
    })
}

async fn apply_targeted_report_repair(
    context: &mut ReportAcceptanceContext<'_>,
    frame: &mut ReportFrame,
    audit: &ReportAudit,
    semantic: &semantic_audit::SemanticReportReview,
    resolved_evidence: &ResolvedEvidence,
) -> Result<BTreeSet<String>, String> {
    let targets = revision::target_sections_for_audit(audit, resolved_evidence, context.outline)?;
    let mut changed_target_ids = targets.keys().cloned().collect::<BTreeSet<_>>();
    let frame_implicated = semantic.issue_target_ids().contains("frame");
    if targets.is_empty() {
        if !frame_implicated {
            return Err(format!(
                "report audit failed without a repairable section or frame target: {}",
                audit.reason
            ));
        }
        redraft_first_section(context).await?;
    } else {
        revision::revise_targets(
            context.session,
            context.query,
            context.run_id,
            context.outline,
            context.events,
            context.state,
            context.evidence,
            context.sections,
            targets,
            &audit.reason,
            context.deadline,
        )
        .await?;
    }
    revision::ensure_sections_valid_after_revision(
        context.sections,
        context.outline,
        context.evidence,
    )?;
    if context.state.phase != InquiryPhase::Auditing {
        return Err(format!(
            "report Inquiry did not return to Auditing after targeted report repair; current phase is {:?}",
            context.state.phase
        ));
    }
    if frame_implicated {
        let revision_context = semantic.revision_context_for_target("frame");
        *frame = generate_frame(
            context.session,
            context.query,
            context.run_id,
            context.canonical_workflow_output,
            context.outline,
            context.state,
            context.evidence,
            Some(&revision_context),
            context.deadline,
        )
        .await?;
        changed_target_ids.insert("frame".to_string());
    }
    if changed_target_ids.is_empty() {
        return Err("targeted report repair changed no auditable target".to_string());
    }
    Ok(changed_target_ids)
}

async fn redraft_first_section(context: &mut ReportAcceptanceContext<'_>) -> Result<(), String> {
    let section_id = context
        .outline
        .sections
        .first()
        .map(|section| section.id.clone())
        .ok_or_else(|| "cannot resume an empty report outline".to_string())?;
    recovery::commit_sections(
        context.session,
        context.run_id,
        context.events,
        context.state,
        context.sections,
        &[section_id],
    )
    .await
}

async fn record_report_audit(
    context: &mut ReportAcceptanceContext<'_>,
    audit: &ReportAudit,
) -> Result<(), String> {
    apply_event(
        context.state,
        context.events,
        InquiryEvent::AuditCompleted {
            passed: audit.passed,
            issues: if audit.passed {
                Vec::new()
            } else {
                vec![audit.reason.clone()]
            },
        },
    )?;
    recovery::persist_projection(
        context.session,
        context.run_id,
        context.events,
        context.state,
    )
    .await
}

fn resolve_report_evidence(
    sections: &mut BTreeMap<String, SectionGeneration>,
    outline: &ResearchOutline,
    evidence: &[AcceptedEvidence],
) -> Result<(UsedEvidenceCatalog, ResolvedEvidence), String> {
    let mut used = UsedEvidenceCatalog::default();
    for planned in &outline.sections {
        let section = sections
            .get_mut(&planned.id)
            .ok_or_else(|| format!("missing generated section `{}`", planned.id))?;
        let resolved = revision::validate_section_candidate(section, planned, evidence)?;
        used.record(&resolved);
    }
    let resolved = resolve_evidence_ids(&used.claim_ids, &used.source_ids, evidence)?;
    Ok((used, resolved))
}
