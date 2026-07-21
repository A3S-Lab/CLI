//! Evidence-bound outline and section-oriented report synthesis.
//!
//! The report model never receives the open tool surface. Rust commits a
//! source-addressed outline, Flow runs independent bounded section calls, and
//! the host assembles and audits the final document before publication.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

#[cfg(test)]
use a3s::research::research_outline_json_schema;
use a3s::research::{
    research_contract_outcome, validate_research_outline, ContractAssessmentStatus,
    DiagnosticDisposition, InquiryEvent, InquiryLimits, InquiryPhase, InquiryState, OutlineSection,
    OutlineValidationContext, ResearchContractOutcome, ResearchOutline,
};
use a3s_code_core::{AgentSession, ToolCallResult};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

use super::deep_research_convergence::{validated_inquiry_projection, ValidatedInquiryProjection};
use super::deep_research_report_audit::{CitationRequirement, ReportAudit, ReportAuditIssue};
use super::{
    accepted_evidence_ledger, deep_research_canonical_workflow_output,
    deep_research_report_frame_schema, inquiry_projection_from_workflow,
    validate_report_obligation_coverage, AcceptedEvidence, GeneratedDeepResearchReport,
    ReportEditorialPlan, ReportPresentation, DEEP_RESEARCH_ABORT_GRACE_MS,
    GRACEFUL_QUIT_ABORT_SETTLE_MS,
};

#[path = "sectioned_report/acceptance.rs"]
mod acceptance;
#[path = "sectioned_report/audit.rs"]
mod audit;
#[path = "sectioned_report/composition.rs"]
mod composition;
#[path = "sectioned_report/generation.rs"]
mod generation;
#[path = "sectioned_report/recovery.rs"]
mod recovery;
#[path = "sectioned_report/revision.rs"]
mod revision;
#[path = "sectioned_report/semantic_audit.rs"]
mod semantic_audit;

use audit::{
    audit_section_generation, materialize_section_candidate, normalize_exact_bracketed_citations,
    normalize_section_markdown, resolve_evidence_ids, unique_sources_for_ids,
    validate_section_obligation_coverage, ResolvedEvidence, UsedEvidenceCatalog,
};
#[cfg(test)]
use composition::assemble_markdown;
use composition::{assemble_and_audit, generate_frame};
#[cfg(test)]
use generation::section_generation_args;
use generation::{
    generate_sections, run_section_workflow, run_single_generation_workflow,
    section_generation_envelope, section_generation_packet,
};

const SECTION_TIMEOUT_MS: u64 = 300_000;
const FRAME_TIMEOUT_MS: u64 = 360_000;
const FRAME_PRESENTATION_TIMEOUT_MS: u64 = 240_000;
const SEMANTIC_AUDIT_TIMEOUT_MS: u64 = 360_000;
const SECTION_WORKFLOW_GRACE_MS: u64 = 15_000;
const MAX_CONCURRENT_SECTION_GENERATIONS: usize = 3;
const SECTION_UNIT_WORKFLOW_TIMEOUT_MS: u64 = (2 * SECTION_TIMEOUT_MS) + SECTION_WORKFLOW_GRACE_MS;
const SECTION_STAGE_BUDGET_MS: u64 = 2 * SECTION_UNIT_WORKFLOW_TIMEOUT_MS;
const FRAME_CONTENT_WORKFLOW_TIMEOUT_MS: u64 = (2 * FRAME_TIMEOUT_MS) + SECTION_WORKFLOW_GRACE_MS;
const FRAME_PRESENTATION_WORKFLOW_TIMEOUT_MS: u64 =
    (2 * FRAME_PRESENTATION_TIMEOUT_MS) + SECTION_WORKFLOW_GRACE_MS;
const FRAME_STAGE_BUDGET_MS: u64 =
    FRAME_CONTENT_WORKFLOW_TIMEOUT_MS + FRAME_PRESENTATION_WORKFLOW_TIMEOUT_MS;
const SEMANTIC_AUDIT_WORKFLOW_TIMEOUT_MS: u64 =
    (2 * SEMANTIC_AUDIT_TIMEOUT_MS) + SECTION_WORKFLOW_GRACE_MS;
const FINAL_TARGETED_REPAIR_RESERVE_MS: u64 = 30 * 60 * 1000;
const REPORT_FINALIZATION_RESERVE_MS: u64 = 15_000;
pub(super) const SECTIONED_REPORT_BUDGET_MS: u64 = SECTION_STAGE_BUDGET_MS
    + FRAME_STAGE_BUDGET_MS
    + (2 * SEMANTIC_AUDIT_WORKFLOW_TIMEOUT_MS)
    + FINAL_TARGETED_REPAIR_RESERVE_MS
    + REPORT_FINALIZATION_RESERVE_MS;
const MAX_REPORT_SECTIONS: usize = 6;
#[cfg(test)]
const MAX_OUTLINE_PACKET_CHARS: usize = 32_000;
const MAX_FRAME_PROMPT_CHARS: usize = 64_000;
const MAX_SECTION_PROMPT_CHARS: usize = 32_000;
const SECTION_WORKFLOW_SOURCE: &str = include_str!("workflow/section_synthesis.js");
const CLOSED_EVIDENCE_REASONING_GUARDRAILS: &str = "Keep every factual inference at the granularity supported by the cited evidence. Do not calculate or estimate intervals, rates, totals, trends, or before/after chronology from raw dates or counts; list the exact supported observations instead. Do not describe a release as later than the same release as its own announcement. A dependency requirement does not establish incompatibility or inability to coexist. The absence of a compatibility statement establishes only that the reviewed source does not document it, not that compatibility is impossible or unsupported elsewhere. Project discontinuation does not establish that no future fixes or releases can occur. A recommendation to migrate to a named replacement supports only that recommendation; it establishes no maintenance, security, compatibility, performance, resource, maturity, or adoption property of the replacement unless a bound claim states it. Source-authored praise such as great or excellent is attributed promotional wording, not evidence of an objective technical property; quote it as attributed wording or omit it instead of translating it into one. Do not generalize one or a few named examples into ecosystem-wide dominance, defaults, exclusivity, or completeness. Do not make a collective all, only, every, or none claim across reviewed items when any included item is partial, indirect, undocumented, or unknown; preserve those item statuses separately unless a bound claim explicitly supplies that quantifier. A question- or section-scoped evidence absence does not establish that the whole report has no evidence; name the unresolved claim or dimension without referring to the packet or asserting report-wide absence. An `updated` timestamp is not a release or publication date unless the source labels it that way; an author or poster name does not establish governance or sole decision authority. A short or incomplete excerpt does not establish that omitted events, changes, or support do not exist. Recommendations may combine supported premises, but distinguish each normative recommendation from a sourced fact and keep its scope to the reviewed evidence.";

#[derive(Clone, Copy, Debug)]
struct ReportDeadline {
    expires_at: Instant,
}

impl ReportDeadline {
    fn new(expires_at: Instant) -> Self {
        Self { expires_at }
    }

    fn from_durable_start(
        caller_expires_at: Instant,
        monotonic_now: Instant,
        wall_now_ms: u64,
        started_at_ms: u64,
    ) -> Result<Self, String> {
        // A regressed wall clock must not grant a fresh report budget. Host
        // monotonic time cannot survive a process restart, so the durable
        // journal timestamp is the cross-process authority.
        let elapsed_ms = wall_now_ms.checked_sub(started_at_ms).unwrap_or(u64::MAX);
        let remaining_ms = SECTIONED_REPORT_BUDGET_MS.saturating_sub(elapsed_ms);
        let durable_expires_at = monotonic_now
            .checked_add(Duration::from_millis(remaining_ms))
            .ok_or_else(|| "DeepResearch durable report deadline overflowed".to_string())?;
        Ok(Self::new(caller_expires_at.min(durable_expires_at)))
    }

    fn tool_timeout_ms(
        self,
        now: Instant,
        local_cap_ms: u64,
        operation: &str,
    ) -> Result<u64, String> {
        let active_remaining = self
            .expires_at
            .saturating_duration_since(now)
            .saturating_sub(Duration::from_millis(REPORT_FINALIZATION_RESERVE_MS));
        let active_remaining_ms = active_remaining.as_millis().min(u128::from(u64::MAX)) as u64;
        if active_remaining_ms == 0 {
            return Err(format!(
                "DeepResearch report budget exhausted before {operation}; no active synthesis time remains after reserving {REPORT_FINALIZATION_RESERVE_MS} ms for finalization"
            ));
        }
        Ok(local_cap_ms.min(active_remaining_ms))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SectionGeneration {
    section_id: String,
    markdown: String,
    claim_ids: Vec<String>,
    source_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SectionGenerationDraft {
    section_id: String,
    markdown: String,
}

impl SectionGeneration {
    fn citation_ids(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        self.claim_ids
            .iter()
            .chain(&self.source_ids)
            .filter(|id| seen.insert((*id).clone()))
            .cloned()
            .collect()
    }
}

struct AssembledReportText {
    body: String,
    markdown: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SectionWorkflowOutput {
    sections: Vec<SectionWorkflowItem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SectionWorkflowItem {
    section_id: String,
    result: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReportFrame {
    report_title: String,
    reader_labels: ReportReaderLabels,
    decision_guidance: Vec<ReportDecisionGuidance>,
    editorial: ReportEditorialPlan,
    presentation: ReportPresentation,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReportReaderLabels {
    qualification_heading: String,
    qualification_intro: String,
    sources_heading: String,
    decision_heading: String,
    evidence_limitation: String,
    primary_source_support: String,
    independent_corroboration: String,
    established_boundary: String,
    qualified_boundary: String,
    unresolved_boundary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReportDecisionGuidance {
    scenario: String,
    recommendation: String,
    basis_obligation_ids: Vec<String>,
    boundary: String,
}

struct AbortInnerToolOnDrop(Option<tokio::task::AbortHandle>);

impl AbortInnerToolOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for AbortInnerToolOnDrop {
    fn drop(&mut self) {
        if let Some(abort) = self.0.take() {
            abort.abort();
        }
    }
}

pub(super) fn sectioned_report_available(
    workflow_output: &str,
    workflow_metadata: Option<&Value>,
) -> bool {
    inquiry_projection_from_workflow(workflow_output, workflow_metadata)
        .ok()
        .flatten()
        .is_some_and(|(_, state)| state.phase == InquiryPhase::Outlining)
}

pub(super) fn merge_sectioned_inquiry_projection(
    workflow_output: &mut String,
    workflow_metadata: Option<&mut Value>,
    generation_metadata: Option<&Value>,
) -> Result<bool, String> {
    let canonical =
        deep_research_canonical_workflow_output(workflow_output, workflow_metadata.as_deref());
    let mut value = serde_json::from_str::<Value>(&canonical)
        .map_err(|error| format!("decode workflow for sectioned inquiry merge: {error}"))?;
    let original_projection = validated_inquiry_projection(&value)?;
    let Some(inquiry) = generation_metadata.and_then(|metadata| metadata.get("inquiry")) else {
        return match original_projection {
            ValidatedInquiryProjection::LegacyCheckedLoop => Ok(false),
            ValidatedInquiryProjection::Inquiry { .. } => {
                Err("sectioned report omitted the required terminal Inquiry projection".to_string())
            }
        };
    };
    let events = serde_json::from_value::<Vec<InquiryEvent>>(
        inquiry
            .get("events")
            .cloned()
            .ok_or_else(|| "sectioned report metadata omitted inquiry events".to_string())?,
    )
    .map_err(|error| format!("decode sectioned report inquiry events: {error}"))?;
    let state = serde_json::from_value::<InquiryState>(
        inquiry
            .get("state")
            .cloned()
            .ok_or_else(|| "sectioned report metadata omitted inquiry state".to_string())?,
    )
    .map_err(|error| format!("decode sectioned report inquiry state: {error}"))?;
    let replayed = a3s::research::replay(&events, &InquiryLimits::default())
        .map_err(|error| format!("replay sectioned report inquiry events: {error}"))?;
    if replayed != state {
        return Err("sectioned report inquiry state differs from its replay".to_string());
    }
    if state.phase != InquiryPhase::Completed {
        return Err(format!(
            "sectioned report Inquiry must reach Completed before merge; current phase is {:?}",
            state.phase
        ));
    }
    if let ValidatedInquiryProjection::Inquiry {
        events: original_events,
        ..
    } = &original_projection
    {
        if !events.starts_with(original_events) {
            return Err(
                "sectioned report Inquiry events do not extend the collected Inquiry journal"
                    .to_string(),
            );
        }
    }

    let object = value
        .as_object_mut()
        .ok_or_else(|| "workflow output is not an object".to_string())?;
    let merged_inquiry = object
        .entry("inquiry")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| "workflow inquiry field is not an object".to_string())?;
    merged_inquiry.insert(
        "events".to_string(),
        serde_json::to_value(events)
            .map_err(|error| format!("encode sectioned report inquiry events: {error}"))?,
    );
    merged_inquiry.insert(
        "state".to_string(),
        serde_json::to_value(state)
            .map_err(|error| format!("encode sectioned report inquiry state: {error}"))?,
    );
    *workflow_output = serde_json::to_string(&value)
        .map_err(|error| format!("encode workflow after sectioned inquiry merge: {error}"))?;
    if let Some(snapshot) = workflow_metadata
        .and_then(|metadata| metadata.pointer_mut("/dynamic_workflow/snapshot"))
        .and_then(Value::as_object_mut)
    {
        snapshot.insert("output".to_string(), value);
    }
    Ok(true)
}

pub(super) async fn generate_sectioned_report(
    session: &AgentSession,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&Value>,
    run_id: &str,
    report_deadline: Instant,
) -> Result<ToolCallResult, String> {
    let report_started_at_ms = super::deep_research_state_journal::record_report_started_at_ms(
        session.workspace(),
        run_id,
    )
    .await
    .map_err(|error| format!("record durable report transaction start: {error:#}"))?;
    let monotonic_now = Instant::now();
    let wall_now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("read wall clock for durable report deadline: {error}"))?
        .as_millis()
        .try_into()
        .map_err(|_| "wall-clock milliseconds exceed u64".to_string())?;
    let deadline = ReportDeadline::from_durable_start(
        report_deadline,
        monotonic_now,
        wall_now_ms,
        report_started_at_ms,
    )?;
    let canonical = deep_research_canonical_workflow_output(workflow_output, workflow_metadata);
    let (mut events, mut state) =
        recovery::load_projection(session, &canonical, workflow_metadata, run_id).await?;
    let evidence = accepted_evidence_ledger(&canonical, workflow_metadata);
    let context = outline_context(&state, &evidence)?;
    let outline = match state.phase {
        InquiryPhase::Outlining => {
            let outline = derive_outline(query, &state, &context)?;
            validate_research_outline(&outline, &context).map_err(|error| error.to_string())?;
            apply_event(
                &mut state,
                &mut events,
                InquiryEvent::OutlineCommitted {
                    outline: outline.clone(),
                },
            )?;
            recovery::persist_projection(session, run_id, &events, &state).await?;
            outline
        }
        InquiryPhase::Drafting | InquiryPhase::Auditing | InquiryPhase::Completed => state
            .outline
            .clone()
            .ok_or_else(|| "durable report Inquiry omitted its committed outline".to_string())?,
        phase => {
            return Err(format!(
                "DeepResearch report pipeline cannot resume from phase {phase:?}"
            ));
        }
    };
    validate_research_outline(&outline, &context).map_err(|error| error.to_string())?;

    let mut sections_by_id = recovery::sections_from_drafts(&outline, &state)?;
    let missing_section_ids = recovery::missing_section_ids(&outline, &sections_by_id);
    if !missing_section_ids.is_empty() {
        let mut generated = generate_sections(
            session, query, run_id, &outline, &state, &evidence, &deadline,
        )
        .await?;
        for section_id in &missing_section_ids {
            let section = generated
                .remove(section_id)
                .ok_or_else(|| format!("missing generated section `{section_id}`"))?;
            sections_by_id.insert(section_id.clone(), section);
        }
    }
    let resume_mode = recovery::resume_mode(&state)?;
    if resume_mode != recovery::ReportResumeMode::VerifyCompleted {
        revision::revise_invalid_sections_once(
            session,
            query,
            run_id,
            &outline,
            &mut events,
            &mut state,
            &evidence,
            &mut sections_by_id,
            &deadline,
        )
        .await?;
    }
    if resume_mode == recovery::ReportResumeMode::DraftSections {
        let uncommitted_section_ids = missing_section_ids
            .iter()
            .filter(|section_id| !state.drafts.contains_key(*section_id))
            .cloned()
            .collect::<Vec<_>>();
        recovery::commit_sections(
            session,
            run_id,
            &mut events,
            &mut state,
            &sections_by_id,
            &uncommitted_section_ids,
        )
        .await?;
    }
    if !matches!(
        state.phase,
        InquiryPhase::Drafting | InquiryPhase::Auditing | InquiryPhase::Completed
    ) {
        return Err(
            "DeepResearch section synthesis did not reach a resumable report phase".to_string(),
        );
    }

    let accepted = acceptance::accept_report(acceptance::ReportAcceptanceContext {
        session,
        query,
        run_id,
        canonical_workflow_output: &canonical,
        outline: &outline,
        events: &mut events,
        state: &mut state,
        evidence: &evidence,
        sections: &mut sections_by_id,
        resume_mode,
        deadline: &deadline,
    })
    .await?;

    let generated = GeneratedDeepResearchReport {
        markdown: accepted.assembled.markdown,
        editorial: accepted.frame.editorial,
        presentation: accepted.frame.presentation,
    };
    let output = serde_json::to_string(&serde_json::json!({"object": generated}))
        .map_err(|error| format!("encode sectioned report result: {error}"))?;
    Ok(ToolCallResult {
        name: "generate_object".to_string(),
        output,
        exit_code: 0,
        metadata: Some(serde_json::json!({
            "sectioned_report": {
                "outline_sections": outline.sections.len(),
                "audit": accepted.audit,
                "revision_rounds": recovery::restored_revision_rounds(&state),
            },
            "inquiry": {
                "events": events,
                "state": state,
            }
        })),
        error_kind: None,
    })
}

fn derive_outline(
    query: &str,
    state: &InquiryState,
    context: &OutlineValidationContext,
) -> Result<ResearchOutline, String> {
    if context.allowed_claim_ids.is_empty() || context.allowed_source_ids.is_empty() {
        return Err(
            "DeepResearch Host outline requires at least one accepted claim and source".to_string(),
        );
    }
    if state.obligations.len() > MAX_REPORT_SECTIONS {
        return Err(format!(
            "DeepResearch contract has {} obligations; the Host outline limit is {MAX_REPORT_SECTIONS}",
            state.obligations.len(),
        ));
    }
    let mut sections = state
        .obligations
        .iter()
        .enumerate()
        .filter_map(|(index, obligation)| {
            let question_ids = state
                .questions
                .iter()
                .filter(|question| {
                    context.required_question_ids.contains(&question.id)
                        && question.obligation_ids.contains(&obligation.id)
                })
                .map(|question| question.id.clone())
                .collect::<Vec<_>>();
            let (mut claim_ids, mut source_ids) =
                outline_references_for_questions(&question_ids, context);
            if claim_ids.is_empty() || source_ids.is_empty() {
                if !obligation.material {
                    // A supporting obligation whose closed question path has no
                    // accepted evidence remains visible through Host-derived
                    // qualification disclosures and frame coverage. Giving it
                    // every unrelated report claim would create a misleading,
                    // oversized catch-all section.
                    return None;
                }
                claim_ids = context.allowed_claim_ids.clone();
                source_ids = context.allowed_source_ids.clone();
            }
            let criteria = obligation.completion_criteria.join(" ");
            let composition_hint = if criteria.trim().is_empty() {
                obligation.focus.clone()
            } else {
                format!("{} {}", obligation.focus, criteria)
            };
            Some(OutlineSection {
                id: format!("section:{}", index + 1),
                heading: bounded_chars(&obligation.title, 240),
                purpose: bounded_chars(&obligation.focus, 4_000),
                perspective_ids: Vec::new(),
                question_ids,
                claim_ids: claim_ids.into_iter().collect(),
                source_ids: source_ids.into_iter().collect(),
                composition_hint: bounded_chars(&composition_hint, 4_000),
            })
        })
        .collect::<Vec<_>>();
    if sections.is_empty() {
        sections.push(OutlineSection {
            id: "section:1".to_string(),
            heading: bounded_chars(query, 240),
            purpose: bounded_chars(query, 4_000),
            perspective_ids: Vec::new(),
            question_ids: state
                .questions
                .iter()
                .filter(|question| context.required_question_ids.contains(&question.id))
                .map(|question| question.id.clone())
                .collect(),
            claim_ids: context.allowed_claim_ids.iter().cloned().collect(),
            source_ids: context.allowed_source_ids.iter().cloned().collect(),
            composition_hint: bounded_chars(query, 4_000),
        });
    }
    let covered_questions = sections
        .iter()
        .flat_map(|section| section.question_ids.iter().cloned())
        .collect::<BTreeSet<_>>();
    let unassigned_questions = state
        .questions
        .iter()
        .filter(|question| {
            context.required_question_ids.contains(&question.id)
                && !covered_questions.contains(&question.id)
        })
        .map(|question| question.id.clone())
        .collect::<Vec<_>>();
    if !unassigned_questions.is_empty() {
        let (claim_ids, source_ids) =
            outline_references_for_questions(&unassigned_questions, context);
        let first = sections
            .first_mut()
            .ok_or_else(|| "DeepResearch Host outline produced no section".to_string())?;
        first.question_ids.extend(unassigned_questions);
        first.claim_ids.extend(claim_ids);
        first.source_ids.extend(source_ids);
        first.question_ids.sort();
        first.question_ids.dedup();
        first.claim_ids.sort();
        first.claim_ids.dedup();
        first.source_ids.sort();
        first.source_ids.dedup();
    }
    Ok(ResearchOutline { sections })
}

fn outline_references_for_questions(
    question_ids: &[String],
    context: &OutlineValidationContext,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut claim_ids = BTreeSet::new();
    let mut source_ids = BTreeSet::new();
    for question_id in question_ids {
        let Some(evidence_ids) = context.question_evidence_ids.get(question_id) else {
            continue;
        };
        for evidence_id in evidence_ids {
            let Some(evidence) = context.evidence_catalog.get(evidence_id) else {
                continue;
            };
            claim_ids.extend(evidence.claim_ids.iter().cloned());
            source_ids.extend(evidence.source_ids.iter().cloned());
        }
    }
    (claim_ids, source_ids)
}

#[cfg(test)]
fn closed_outline_packet(
    query: &str,
    state: &InquiryState,
    evidence: &[AcceptedEvidence],
    context: &OutlineValidationContext,
) -> Result<Value, String> {
    let evidence_items = outline_evidence_items(context, evidence)?;
    let question_evidence_bindings = state
        .questions
        .iter()
        .filter(|question| !question.evidence_ids.is_empty())
        .map(|question| {
            serde_json::json!({
                "question_id": question.id,
                "obligation_ids": question.obligation_ids,
                "completion_criterion_indexes": question.completion_criterion_indexes,
                "evidence_ids": question.evidence_ids,
            })
        })
        .collect::<Vec<_>>();
    let questions = state
        .questions
        .iter()
        .map(|question| {
            serde_json::json!({
                "id": question.id,
                "obligation_ids": question.obligation_ids,
                "completion_criterion_indexes": question.completion_criterion_indexes,
                "material": question.material,
                "prompt": question.prompt,
                "status": question.status,
                "answer": question.answer.as_deref().map(|answer| bounded_chars(answer, 1_500)),
                "bound_reason": question.bound_reason.as_deref().map(|reason| bounded_chars(reason, 800)),
                "evidence_ids": question.evidence_ids,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "query": query,
        "stable_research_obligations": state.obligations,
        "stop_conditions": state.stop_conditions,
        "contract_assessment": outline_contract_assessment(state),
        "questions": questions,
        "evidence_graph": {
            "question_evidence_bindings": question_evidence_bindings,
            "evidence_items": evidence_items,
        },
        "allowed_question_ids": context.allowed_question_ids,
        "material_question_ids": context.material_question_ids,
        "required_question_ids": context.required_question_ids,
    }))
}

#[cfg(test)]
fn outline_evidence_items(
    context: &OutlineValidationContext,
    evidence: &[AcceptedEvidence],
) -> Result<Vec<Value>, String> {
    let mut found = BTreeSet::new();
    let items = evidence
        .iter()
        .filter(|item| {
            let retained = context.evidence_catalog.contains_key(&item.id);
            if retained {
                found.insert(item.id.clone());
            }
            retained
        })
        .collect::<Vec<_>>();
    let expected = context
        .evidence_catalog
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if found != expected {
        let missing = expected.difference(&found).cloned().collect::<Vec<_>>();
        return Err(format!(
            "retained evidence ledger omitted inquiry evidence items: {}",
            missing.join(", ")
        ));
    }
    Ok(items
        .into_iter()
        .map(|item| {
            serde_json::json!({
                "id": item.id,
                "summary": bounded_chars(&item.summary, 600),
                "confidence": item.confidence,
                "claims": item.claims.iter().map(|claim| serde_json::json!({
                    "id": claim.id,
                    "text": bounded_chars(&claim.text, 240),
                })).collect::<Vec<_>>(),
                "sources": item.sources.iter().map(|source| serde_json::json!({
                    "id": source.id,
                    "anchor": source.anchor,
                    "title": source.title.as_deref().map(|title| bounded_chars(title, 300)),
                    "date": source.date,
                    "reliability": source.reliability.as_deref().map(|value| bounded_chars(value, 400)),
                })).collect::<Vec<_>>(),
                "contradictions": item.contradictions.iter()
                    .take(8)
                    .map(|value| bounded_chars(value, 500))
                    .collect::<Vec<_>>(),
                "gaps": item.gaps.iter()
                    .take(8)
                    .map(|value| bounded_chars(value, 500))
                    .collect::<Vec<_>>(),
            })
        })
        .collect())
}

fn outline_contract_assessment(state: &InquiryState) -> Value {
    let Some(assessment) = state.contract_assessment.as_ref() else {
        return Value::Null;
    };
    let diagnostic_details = state
        .evidence_catalog
        .values()
        .flat_map(|evidence| evidence.diagnostics.iter())
        .map(|diagnostic| (diagnostic.id.as_str(), diagnostic.detail.as_str()))
        .collect::<BTreeMap<_, _>>();
    serde_json::json!({
        "outcome": research_contract_outcome(state),
        "obligations": assessment.obligations.iter().map(|obligation| serde_json::json!({
            "obligation_id": obligation.obligation_id,
            "criteria": obligation.criteria.iter().map(|criterion| serde_json::json!({
                "criterion_index": criterion.criterion_index,
                "status": criterion.status,
            })).collect::<Vec<_>>(),
            "primary_source": obligation.primary_source.as_ref().map(|requirement| serde_json::json!({
                "status": requirement.status,
                "evidence_ids": requirement.evidence_ids,
                "source_ids": requirement.source_ids,
            })),
            "independent_corroboration": obligation.independent_corroboration.as_ref().map(|requirement| serde_json::json!({
                "status": requirement.status,
                "evidence_ids": requirement.evidence_ids,
                "source_ids": requirement.source_ids,
            })),
        })).collect::<Vec<_>>(),
        "stop_conditions": assessment.stop_conditions.iter().map(|condition| serde_json::json!({
            "condition_index": condition.condition_index,
            "status": condition.status,
        })).collect::<Vec<_>>(),
        "diagnostics": assessment.diagnostics.iter().map(|diagnostic| serde_json::json!({
            "diagnostic_id": diagnostic.diagnostic_id,
            "disposition": diagnostic.disposition,
            "obligation_ids": diagnostic.obligation_ids,
            "evidence_ids": diagnostic.evidence_ids,
            "detail": diagnostic_details
                .get(diagnostic.diagnostic_id.as_str())
                .map(|detail| bounded_chars(detail, 600)),
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
fn closed_outline_schema(
    context: &OutlineValidationContext,
    section_limit: usize,
) -> Result<Value, String> {
    let mut schema = research_outline_json_schema();
    let sections = schema
        .pointer_mut("/properties/sections")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "research outline schema omitted sections".to_string())?;
    sections.insert(
        "maxItems".to_string(),
        Value::from(section_limit.min(MAX_REPORT_SECTIONS)),
    );
    let properties = sections
        .get_mut("items")
        .and_then(Value::as_object_mut)
        .and_then(|items| items.get_mut("properties"))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "research outline schema omitted section properties".to_string())?;

    close_outline_reference_array(properties, "question_ids", &context.allowed_question_ids)?;
    close_outline_reference_array(properties, "claim_ids", &context.allowed_claim_ids)?;
    close_outline_reference_array(properties, "source_ids", &context.allowed_source_ids)?;
    Ok(schema)
}

#[cfg(test)]
fn close_outline_reference_array(
    properties: &mut serde_json::Map<String, Value>,
    field: &str,
    allowed: &BTreeSet<String>,
) -> Result<(), String> {
    let property = properties
        .get_mut(field)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("research outline schema omitted `{field}`"))?;
    let minimum = property
        .get("minItems")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default();
    if allowed.len() < minimum {
        return Err(format!(
            "research outline schema requires {minimum} `{field}` values but only {} are allowed",
            allowed.len()
        ));
    }
    let maximum = property
        .get("maxItems")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(allowed.len())
        .min(allowed.len());
    property.insert("maxItems".to_string(), Value::from(maximum));

    // Empty optional catalogs are closed by maxItems = 0. Keeping the base
    // item schema avoids an empty enum, which structured-output providers do
    // not consistently accept even though no array item can be produced.
    if !allowed.is_empty() {
        property
            .get_mut("items")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("research outline schema `{field}` omitted item schema"))?
            .insert(
                "enum".to_string(),
                Value::Array(allowed.iter().cloned().map(Value::String).collect()),
            );
    }
    Ok(())
}

fn outline_context(
    state: &InquiryState,
    evidence: &[AcceptedEvidence],
) -> Result<OutlineValidationContext, String> {
    if state.claim_catalog.is_empty() || state.source_catalog.is_empty() {
        return Err(
            "inquiry state does not contain both accepted claim and source catalogs for outlining"
                .to_string(),
        );
    }
    for (evidence_id, accepted) in &state.evidence_catalog {
        let retained = evidence
            .iter()
            .find(|item| item.id == *evidence_id)
            .ok_or_else(|| {
                format!(
                    "inquiry evidence `{evidence_id}` is absent from the retained evidence ledger"
                )
            })?;
        let retained_claim_ids = retained
            .claims
            .iter()
            .map(|claim| claim.id.clone())
            .collect::<BTreeSet<_>>();
        let retained_source_ids = retained
            .sources
            .iter()
            .map(|source| source.id.clone())
            .collect::<BTreeSet<_>>();
        if accepted.claim_ids.iter().cloned().collect::<BTreeSet<_>>() != retained_claim_ids
            || accepted.source_ids.iter().cloned().collect::<BTreeSet<_>>() != retained_source_ids
        {
            return Err(format!(
                "inquiry evidence `{evidence_id}` claim/source bindings differ from the retained evidence ledger"
            ));
        }
    }
    Ok(state.outline_validation_context())
}

fn apply_event(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    event: InquiryEvent,
) -> Result<(), String> {
    state
        .apply(&event, &InquiryLimits::default())
        .map_err(|error| format!("apply report inquiry event `{}`: {error}", event.name()))?;
    events.push(event);
    Ok(())
}

async fn timed_tool(
    session: &AgentSession,
    name: &str,
    args: Value,
    timeout_ms: u64,
) -> Result<ToolCallResult, String> {
    let (mut events, mut join) = session.tool_with_events(name, args);
    let abort = join.abort_handle();
    let mut abort_on_drop = AbortInnerToolOnDrop(Some(abort.clone()));
    let timeout = tokio::time::sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout);
    let mut events_open = true;

    loop {
        tokio::select! {
            result = &mut join => {
                abort_on_drop.disarm();
                while events.try_recv().is_ok() {}
                return result
                    .map_err(|error| format!("{name} task failed: {error}"))?
                    .map_err(|error| format!("{name} failed: {error}"));
            }
            event = events.recv(), if events_open => {
                if event.is_none() {
                    events_open = false;
                }
            }
            _ = &mut timeout => {
                abort.abort();
                let _ = tokio::time::timeout(
                    Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                    &mut join,
                )
                .await;
                let settled = session
                    .cancel_and_settle(
                        Duration::from_millis(DEEP_RESEARCH_ABORT_GRACE_MS),
                        Duration::from_millis(GRACEFUL_QUIT_ABORT_SETTLE_MS),
                    )
                    .await;
                abort_on_drop.disarm();
                while events.try_recv().is_ok() {}
                return if settled {
                    Err(format!("{name} timed out after {timeout_ms} ms"))
                } else {
                    Err(format!(
                        "{name} timed out after {timeout_ms} ms and the session did not settle"
                    ))
                };
            }
        }
    }
}

fn generated_object<T: DeserializeOwned>(result: &ToolCallResult) -> Result<T, String> {
    if result.exit_code != 0 {
        return Err(result
            .output
            .lines()
            .next()
            .unwrap_or("structured generation failed")
            .to_string());
    }
    let envelope = serde_json::from_str::<Value>(&result.output)
        .map_err(|error| format!("structured generation returned invalid JSON: {error}"))?;
    serde_json::from_value(
        envelope
            .get("object")
            .cloned()
            .ok_or_else(|| "structured generation response omitted object".to_string())?,
    )
    .map_err(|error| format!("structured generation object violated its contract: {error}"))
}

fn tool_result_from_step(value: &Value) -> Result<ToolCallResult, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "section step returned a non-object result".to_string())?;
    Ok(ToolCallResult {
        name: object
            .get("name")
            .or_else(|| object.get("tool"))
            .and_then(Value::as_str)
            .unwrap_or("generate_object")
            .to_string(),
        output: object
            .get("output")
            .and_then(Value::as_str)
            .ok_or_else(|| "section step result omitted output".to_string())?
            .to_string(),
        exit_code: object
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or_default(),
        metadata: object.get("metadata").cloned(),
        error_kind: None,
    })
}

fn bounded_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(test)]
#[path = "sectioned_report/process_resume_tests.rs"]
mod process_resume_tests;
#[cfg(test)]
#[path = "sectioned_report/tests.rs"]
mod tests;
