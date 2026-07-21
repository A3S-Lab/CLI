//! Inquiry-journal recovery and durable report-stage transaction boundaries.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReportResumeMode {
    DraftSections,
    RecoverFailedAudit,
    Audit,
    VerifyCompleted,
}

pub(super) async fn load_projection(
    session: &AgentSession,
    workflow_output: &str,
    workflow_metadata: Option<&Value>,
    run_id: &str,
) -> Result<(Vec<InquiryEvent>, InquiryState), String> {
    let (workflow_events, workflow_state) =
        inquiry_projection_from_workflow(workflow_output, workflow_metadata)?.ok_or_else(|| {
            "DeepResearch inquiry projection is unavailable for outlining".to_string()
        })?;
    let journal =
        super::super::deep_research_state_journal::load_inquiry_state(session.workspace(), run_id)
            .await
            .map_err(|error| format!("load durable report Inquiry state: {error}"))?;
    select_projection(workflow_events, workflow_state, journal)
}

pub(super) fn select_projection(
    workflow_events: Vec<InquiryEvent>,
    workflow_state: InquiryState,
    journal: Option<(Vec<InquiryEvent>, InquiryState)>,
) -> Result<(Vec<InquiryEvent>, InquiryState), String> {
    let Some((journal_events, journal_state)) = journal else {
        return Ok((workflow_events, workflow_state));
    };
    if journal_events.len() < workflow_events.len() || !journal_events.starts_with(&workflow_events)
    {
        return Err(
            "durable report Inquiry journal does not extend the collected workflow projection"
                .to_string(),
        );
    }
    if journal_events.len() == workflow_events.len() && journal_state != workflow_state {
        return Err(
            "durable report Inquiry state disagrees with the collected workflow projection"
                .to_string(),
        );
    }
    Ok((journal_events, journal_state))
}

pub(super) fn resume_mode(state: &InquiryState) -> Result<ReportResumeMode, String> {
    match state.phase {
        InquiryPhase::Outlining | InquiryPhase::Drafting if state.audit.is_none() => {
            Ok(ReportResumeMode::DraftSections)
        }
        InquiryPhase::Drafting if state.audit.as_ref().is_some_and(|audit| !audit.passed) => {
            Ok(ReportResumeMode::RecoverFailedAudit)
        }
        InquiryPhase::Auditing => Ok(ReportResumeMode::Audit),
        InquiryPhase::Completed if state.audit.as_ref().is_some_and(|audit| audit.passed) => {
            Ok(ReportResumeMode::VerifyCompleted)
        }
        phase => Err(format!(
            "DeepResearch report pipeline cannot resume from phase {phase:?}"
        )),
    }
}

pub(super) fn restored_revision_rounds(state: &InquiryState) -> usize {
    state.section_revisions.len()
}

pub(super) async fn persist_projection(
    session: &AgentSession,
    run_id: &str,
    events: &[InquiryEvent],
    state: &InquiryState,
) -> Result<(), String> {
    super::super::deep_research_state_journal::record_inquiry_state(
        session.workspace(),
        run_id,
        events,
        state,
    )
    .await
    .map_err(|error| format!("persist durable report Inquiry state: {error}"))
}

pub(super) fn sections_from_drafts(
    outline: &ResearchOutline,
    state: &InquiryState,
) -> Result<BTreeMap<String, SectionGeneration>, String> {
    let mut sections = BTreeMap::new();
    for (section_id, draft) in &state.drafts {
        let planned = outline
            .sections
            .iter()
            .find(|section| section.id == *section_id)
            .ok_or_else(|| {
                format!("durable Inquiry contains unknown drafted section `{section_id}`")
            })?;
        if draft.section_id != *section_id {
            return Err(format!(
                "durable Inquiry draft key `{section_id}` disagrees with draft id `{}`",
                draft.section_id
            ));
        }
        let planned_source_ids = planned.source_ids.iter().collect::<BTreeSet<_>>();
        let cited_source_ids = draft
            .citation_ids
            .iter()
            .filter(|citation_id| planned_source_ids.contains(citation_id))
            .cloned()
            .collect::<Vec<_>>();
        sections.insert(
            section_id.clone(),
            SectionGeneration {
                section_id: section_id.clone(),
                markdown: draft.content.clone(),
                claim_ids: planned.claim_ids.clone(),
                source_ids: cited_source_ids,
            },
        );
    }
    Ok(sections)
}

pub(super) fn missing_section_ids(
    outline: &ResearchOutline,
    sections: &BTreeMap<String, SectionGeneration>,
) -> Vec<String> {
    outline
        .sections
        .iter()
        .filter(|section| !sections.contains_key(&section.id))
        .map(|section| section.id.clone())
        .collect()
}

pub(super) async fn commit_sections(
    session: &AgentSession,
    run_id: &str,
    events: &mut Vec<InquiryEvent>,
    state: &mut InquiryState,
    sections: &BTreeMap<String, SectionGeneration>,
    section_ids: &[String],
) -> Result<(), String> {
    for section_id in section_ids {
        let section = sections
            .get(section_id)
            .ok_or_else(|| format!("cannot commit missing section `{section_id}`"))?;
        apply_event(
            state,
            events,
            InquiryEvent::SectionDrafted {
                section_id: section.section_id.clone(),
                content: section.markdown.clone(),
                citation_ids: section.citation_ids(),
            },
        )?;
    }
    if !section_ids.is_empty() {
        persist_projection(session, run_id, events, state).await?;
    }
    Ok(())
}
