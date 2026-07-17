//! Bounded workflow execution and closed-evidence inquiry resolution.

use std::collections::BTreeSet;

use a3s::research::{
    perspective_discovery_events, perspective_discovery_generation_params,
    question_resolution_events, question_resolution_generation_params,
    research_contract_assessment_event, research_contract_assessment_generation_params,
    research_contract_outcome, EvidenceDiagnostic, EvidenceDiagnosticKind, EvidenceRef,
    InquiryEvent, InquiryLimits, InquiryState, PerspectiveDiscoveryOutput, Question,
    QuestionResolutionOutput, QuestionStatus, ResearchContractAssessment, ResearchContractOutcome,
};
use a3s_code_core::{AgentEvent, AgentSession, ToolCallResult};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::sync::mpsc;

use super::super::{
    accepted_evidence_ledger, accepted_evidence_synthesis_payload,
    deep_research_canonical_workflow_output, AcceptedEvidence,
};
use super::plan::{
    bound_question_batch, bound_questions, bound_workflow_timeout, defer_or_bound_question_batch,
    follow_up_research_plan, perspective_research_plan, plan_max_iterations,
    questions_scheduled_for_retrieval, queued_questions_for_next_wave, scout_plan,
    workflow_args_with_plan, PlannedInquiry,
};
use super::{
    apply_event, apply_event_and_checkpoint, budget_terminal_result, checkpoint_inquiry,
    configured_workflow_timeout_ms, terminalize_budget_exhaustion, InquiryCheckpointWriter,
    FOLLOW_UP_WORKFLOW_TIMEOUT_MS, MAX_FOLLOW_UP_QUESTIONS_PER_WAVE, MAX_QUESTION_EVIDENCE_ITEMS,
    PERSPECTIVE_DISCOVERY_TIMEOUT_MS, QUESTION_RESOLUTION_TIMEOUT_MS,
    RESEARCH_CONTRACT_ASSESSMENT_TIMEOUT_MS, SCOUT_WORKFLOW_TIMEOUT_MS,
};

#[derive(Debug)]
pub(super) struct InquiryExecution {
    pub(super) result: ToolCallResult,
    pub(super) retrieval_plan: Value,
    pub(super) workflow_args: Value,
    pub(super) follow_up_waves_remaining: usize,
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

pub(super) async fn run_perspective_guided(
    session: &AgentSession,
    args: Value,
    plan: PlannedInquiry,
    progress_tx: &mpsc::Sender<AgentEvent>,
    state: &mut InquiryState,
    inquiry_events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    checkpoint: &InquiryCheckpointWriter,
) -> Result<InquiryExecution, String> {
    let scout_plan = scout_plan(&plan.value, &plan.scout_queries)?;
    let scout_run_id = args
        .get("run_id")
        .and_then(Value::as_str)
        .map(|run_id| format!("{run_id}-scout"));
    let mut scout_args =
        workflow_args_with_plan(args.clone(), scout_plan, scout_run_id.as_deref())?;
    let Some(scout_timeout_ms) = checkpoint.stage_timeout_ms(SCOUT_WORKFLOW_TIMEOUT_MS) else {
        let reason =
            "the shared inquiry deadline left no scout budget after reserving finalization time";
        terminalize_budget_exhaustion(Some(checkpoint), state, inquiry_events, limits, reason)
            .await?;
        return Ok(InquiryExecution {
            result: budget_terminal_result(&args, reason),
            retrieval_plan: plan.value,
            workflow_args: args,
            follow_up_waves_remaining: 0,
        });
    };
    bound_workflow_timeout(&mut scout_args, scout_timeout_ms)?;
    let scout_result = run_dynamic_workflow(session, scout_args, progress_tx).await?;
    let scout_output = deep_research_canonical_workflow_output(
        &scout_result.output,
        scout_result.metadata.as_ref(),
    );
    let scout_evidence = accepted_evidence_ledger(&scout_output, scout_result.metadata.as_ref());
    let allowed_source_ids = accepted_source_ids(&scout_evidence);
    if allowed_source_ids.is_empty() {
        return Err(
            "perspective-guided research could not retain any scout source for perspective discovery"
                .to_string(),
        );
    }
    apply_event(
        state,
        inquiry_events,
        InquiryEvent::ScoutCompleted {
            source_ids: allowed_source_ids.iter().cloned().collect(),
        },
        limits,
    )?;
    checkpoint_inquiry(Some(checkpoint), inquiry_events, state).await?;

    let scout_packet = accepted_evidence_synthesis_payload(&scout_evidence, &scout_output);
    let Some(discovery_timeout_ms) = checkpoint.stage_timeout_ms(PERSPECTIVE_DISCOVERY_TIMEOUT_MS)
    else {
        let reason = "the shared inquiry deadline left no perspective-discovery budget after reserving finalization time";
        terminalize_budget_exhaustion(Some(checkpoint), state, inquiry_events, limits, reason)
            .await?;
        return Ok(InquiryExecution {
            result: scout_result,
            retrieval_plan: plan.value,
            workflow_args: args,
            follow_up_waves_remaining: 0,
        });
    };
    let discovery_args = perspective_discovery_generation_params(
        args.pointer("/input/query")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        &state.obligations,
        &scout_packet,
        &allowed_source_ids,
        discovery_timeout_ms,
    )
    .map_err(|error| error.to_string())?;
    let generated = call_tool_with_progress(
        session,
        "generate_object",
        serde_json::to_value(discovery_args)
            .map_err(|error| format!("encode perspective discovery request: {error}"))?,
        progress_tx,
        false,
    )
    .await?;
    let discovery: PerspectiveDiscoveryOutput = generated_object(&generated)?;
    let discovered_events =
        perspective_discovery_events(&discovery, &allowed_source_ids, &state.obligations)
            .map_err(|error| error.to_string())?;
    for event in discovered_events {
        apply_event(state, inquiry_events, event, limits)?;
    }
    checkpoint_inquiry(Some(checkpoint), inquiry_events, state).await?;

    let follow_up_waves_remaining = plan_max_iterations(&plan.value)
        .saturating_sub(1)
        .min(limits.max_question_round as u64) as usize;
    let research_plan = perspective_research_plan(&plan.value, &discovery)?;
    let mut research_args = workflow_args_with_plan(args.clone(), research_plan.clone(), None)?;
    let requested_timeout =
        configured_workflow_timeout_ms(&research_args, FOLLOW_UP_WORKFLOW_TIMEOUT_MS);
    let Some(research_timeout_ms) = checkpoint.stage_timeout_ms(requested_timeout) else {
        let reason = "the shared inquiry deadline left no perspective retrieval budget after reserving finalization time";
        terminalize_budget_exhaustion(Some(checkpoint), state, inquiry_events, limits, reason)
            .await?;
        return Ok(InquiryExecution {
            result: scout_result,
            retrieval_plan: research_plan,
            workflow_args: args,
            follow_up_waves_remaining: 0,
        });
    };
    bound_workflow_timeout(&mut research_args, research_timeout_ms)?;
    let research_result = run_dynamic_workflow(session, research_args.clone(), progress_tx).await?;
    let result = combine_workflow_results(
        research_result,
        &scout_result,
        &scout_output,
        inquiry_events,
        state,
    )?;
    Ok(InquiryExecution {
        result,
        retrieval_plan: research_plan,
        workflow_args: research_args,
        follow_up_waves_remaining,
    })
}

pub(super) async fn resolve_questions_with_bounded_follow_up_waves(
    session: &AgentSession,
    progress_tx: &mpsc::Sender<AgentEvent>,
    execution: &mut InquiryExecution,
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    checkpoint: Option<&InquiryCheckpointWriter>,
) -> Result<(), String> {
    let planned_follow_up_waves = execution.follow_up_waves_remaining;
    let mut resolution_plan = execution.retrieval_plan.clone();
    let mut resolve_current_wave = true;
    loop {
        if resolve_current_wave {
            let resolution_timeout_ms = match checkpoint {
                Some(checkpoint) => {
                    let Some(timeout_ms) =
                        checkpoint.stage_timeout_ms(QUESTION_RESOLUTION_TIMEOUT_MS)
                    else {
                        terminalize_budget_exhaustion(
                            Some(checkpoint),
                            state,
                            events,
                            limits,
                            "the shared inquiry deadline left no question-resolution budget after reserving finalization time",
                        )
                        .await?;
                        return Ok(());
                    };
                    timeout_ms
                }
                None => QUESTION_RESOLUTION_TIMEOUT_MS,
            };
            let scheduled = questions_scheduled_for_retrieval(state, &resolution_plan)?;
            resolve_queued_questions(
                session,
                progress_tx,
                &execution.result,
                &scheduled,
                state,
                events,
                limits,
                execution.follow_up_waves_remaining > 0,
                checkpoint,
                resolution_timeout_ms,
            )
            .await?;
        }
        let next_wave = queued_questions_for_next_wave(state, events, limits)?;
        if !next_wave.already_attempted.is_empty() {
            bound_question_batch(
                state,
                events,
                limits,
                &next_wave.already_attempted,
                "an equivalent normalized query, obligation frontier, and evidence head was already attempted without a material frontier change",
            )?;
            checkpoint_inquiry(checkpoint, events, state).await?;
        }
        let follow_ups = next_wave.novel;
        if follow_ups.is_empty() {
            exhaust_if_material_inquiry_unresolved(state, events, limits)?;
            checkpoint_inquiry(checkpoint, events, state).await?;
            return Ok(());
        }

        if execution.follow_up_waves_remaining == 0 {
            bound_questions(
                state,
                events,
                limits,
                "the LLM-selected retrieval-wave budget was exhausted",
            )?;
            exhaust_if_material_inquiry_unresolved(state, events, limits)?;
            checkpoint_inquiry(checkpoint, events, state).await?;
            return Ok(());
        }

        let wave_number = planned_follow_up_waves
            .saturating_sub(execution.follow_up_waves_remaining)
            .saturating_add(1);
        let follow_up_plan =
            follow_up_research_plan(&execution.retrieval_plan, &follow_ups, &state.obligations)?;
        let scheduled = questions_scheduled_for_retrieval(state, &follow_up_plan)?;
        let follow_up_run_id = execution
            .workflow_args
            .get("run_id")
            .and_then(Value::as_str)
            .map(|run_id| format!("{run_id}-followup-{wave_number}"));
        let mut follow_up_args = workflow_args_with_plan(
            execution.workflow_args.clone(),
            follow_up_plan.clone(),
            follow_up_run_id.as_deref(),
        )?;
        let follow_up_timeout_ms = match checkpoint {
            Some(checkpoint) => {
                let Some(timeout_ms) = checkpoint.stage_timeout_ms(FOLLOW_UP_WORKFLOW_TIMEOUT_MS)
                else {
                    terminalize_budget_exhaustion(
                        Some(checkpoint),
                        state,
                        events,
                        limits,
                        "the shared inquiry deadline left no follow-up retrieval budget after reserving finalization time",
                    )
                    .await?;
                    return Ok(());
                };
                timeout_ms
            }
            None => FOLLOW_UP_WORKFLOW_TIMEOUT_MS,
        };
        bound_workflow_timeout(&mut follow_up_args, follow_up_timeout_ms)?;
        execution.follow_up_waves_remaining -= 1;
        let follow_up_result = match run_dynamic_workflow(session, follow_up_args, progress_tx)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                defer_or_bound_question_batch(
                    state,
                    events,
                    limits,
                    &scheduled,
                    execution.follow_up_waves_remaining > 0,
                    &format!(
                        "follow-up retrieval wave {wave_number} ended before evidence was retained: {error}"
                    ),
                )?;
                checkpoint_inquiry(checkpoint, events, state).await?;
                resolve_current_wave = false;
                continue;
            }
        };
        merge_additional_workflow_result(
            &mut execution.result,
            &follow_up_result,
            &format!("follow_up_{wave_number}"),
        )?;
        resolution_plan = follow_up_plan;
        resolve_current_wave = true;
    }
}

pub(super) async fn assess_completed_research_contract(
    session: &AgentSession,
    progress_tx: &mpsc::Sender<AgentEvent>,
    execution: &InquiryExecution,
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    checkpoint: Option<&InquiryCheckpointWriter>,
) -> Result<ResearchContractOutcome, String> {
    if state.phase != a3s::research::InquiryPhase::Outlining {
        return Err(format!(
            "research contract assessment requires Outlining; current phase is {:?}",
            state.phase
        ));
    }
    let assessment_timeout_ms = match checkpoint {
        Some(checkpoint) => {
            let Some(timeout_ms) =
                checkpoint.stage_timeout_ms(RESEARCH_CONTRACT_ASSESSMENT_TIMEOUT_MS)
            else {
                terminalize_budget_exhaustion(
                    Some(checkpoint),
                    state,
                    events,
                    limits,
                    "the shared inquiry deadline left no contract-assessment budget after reserving finalization time",
                )
                .await?;
                return Ok(ResearchContractOutcome::Unsatisfied);
            };
            timeout_ms
        }
        None => RESEARCH_CONTRACT_ASSESSMENT_TIMEOUT_MS,
    };
    let canonical = deep_research_canonical_workflow_output(
        &execution.result.output,
        execution.result.metadata.as_ref(),
    );
    let evidence = accepted_evidence_ledger(&canonical, execution.result.metadata.as_ref());
    let packet = accepted_evidence_synthesis_payload(&evidence, &canonical);
    let generation_args = research_contract_assessment_generation_params(
        canonical_query(&canonical)
            .as_deref()
            .unwrap_or("DeepResearch inquiry"),
        state,
        &bounded_chars(&packet, 60_000),
        assessment_timeout_ms,
    )
    .map_err(|error| error.to_string())?;
    let generated = match call_tool_with_progress(
        session,
        "generate_object",
        serde_json::to_value(generation_args)
            .map_err(|error| format!("encode research contract assessment request: {error}"))?,
        progress_tx,
        false,
    )
    .await
    {
        Ok(generated) => generated,
        Err(error) => {
            let reason = format!(
                "closed-evidence contract assessment ended before completion: {}",
                bounded_chars(&error, 1_000)
            );
            terminalize_budget_exhaustion(checkpoint, state, events, limits, &reason).await?;
            return Ok(ResearchContractOutcome::Unsatisfied);
        }
    };
    let assessment: ResearchContractAssessment = match generated_object(&generated) {
        Ok(assessment) => assessment,
        Err(error) => {
            let reason = format!(
                "closed-evidence contract assessment was invalid: {}",
                bounded_chars(&error, 1_000)
            );
            terminalize_budget_exhaustion(checkpoint, state, events, limits, &reason).await?;
            return Ok(ResearchContractOutcome::Unsatisfied);
        }
    };
    let event = match research_contract_assessment_event(state, assessment) {
        Ok(event) => event,
        Err(error) => {
            let reason = format!(
                "closed-evidence contract assessment violated its host contract: {}",
                bounded_chars(&error.to_string(), 1_000)
            );
            terminalize_budget_exhaustion(checkpoint, state, events, limits, &reason).await?;
            return Ok(ResearchContractOutcome::Unsatisfied);
        }
    };
    apply_event_and_checkpoint(checkpoint, state, events, event, limits).await?;
    research_contract_outcome(state)
        .ok_or_else(|| "research contract assessment produced no terminal outcome".to_string())
}

fn exhaust_if_material_inquiry_unresolved(
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
) -> Result<(), String> {
    if state.phase == a3s::research::InquiryPhase::Exhausted {
        return Ok(());
    }
    let unresolved_material = state
        .questions
        .iter()
        .filter(|question| question.material && question.status != QuestionStatus::Answered)
        .count();
    if unresolved_material == 0 {
        return Ok(());
    }
    apply_event(
        state,
        events,
        InquiryEvent::BudgetExhausted {
            reason: format!(
                "{unresolved_material} material research question(s) remained bounded after the available retrieval waves"
            ),
        },
        limits,
    )
}

async fn resolve_queued_questions(
    session: &AgentSession,
    progress_tx: &mpsc::Sender<AgentEvent>,
    result: &ToolCallResult,
    queued: &[Question],
    state: &mut InquiryState,
    events: &mut Vec<InquiryEvent>,
    limits: &InquiryLimits,
    allow_follow_ups: bool,
    checkpoint: Option<&InquiryCheckpointWriter>,
    resolution_timeout_ms: u64,
) -> Result<(), String> {
    if queued.is_empty() {
        return Err("DeepResearch retrieval wave has no scheduled questions".to_string());
    }
    let canonical =
        deep_research_canonical_workflow_output(&result.output, result.metadata.as_ref());
    let evidence = accepted_evidence_ledger(&canonical, result.metadata.as_ref());
    let evidence_catalog = prepare_evidence_catalog(state, &evidence)?;
    let packet_evidence =
        balanced_evidence_packet(&evidence_catalog.addressable, MAX_QUESTION_EVIDENCE_ITEMS);
    if packet_evidence.is_empty() {
        defer_or_bound_question_batch(
            state,
            events,
            limits,
            queued,
            allow_follow_ups,
            "no accepted evidence was retained for this question",
        )?;
        checkpoint_inquiry(checkpoint, events, state).await?;
        return Ok(());
    }
    let allowed_evidence_ids = accepted_evidence_ids(&packet_evidence);
    let packet = accepted_evidence_synthesis_payload(&packet_evidence, &canonical);
    let query = canonical_query(&canonical);
    let generation_args = question_resolution_generation_params(
        query.as_deref().unwrap_or("DeepResearch inquiry"),
        queued,
        &state.obligations,
        &state.stop_conditions,
        &allowed_evidence_ids,
        &bounded_chars(&packet, 60_000),
        resolution_timeout_ms,
    )
    .map_err(|error| error.to_string())?;
    let generated = match call_tool_with_progress(
        session,
        "generate_object",
        serde_json::to_value(generation_args)
            .map_err(|error| format!("encode question resolution request: {error}"))?,
        progress_tx,
        false,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            apply_pending_evidence(state, events, limits, evidence_catalog.pending)?;
            defer_or_bound_question_batch(
                state,
                events,
                limits,
                queued,
                allow_follow_ups,
                &format!("closed-evidence question assessment failed: {error}"),
            )?;
            checkpoint_inquiry(checkpoint, events, state).await?;
            return Ok(());
        }
    };
    let mut resolution: QuestionResolutionOutput = match generated_object(&generated) {
        Ok(output) => output,
        Err(error) => {
            apply_pending_evidence(state, events, limits, evidence_catalog.pending)?;
            defer_or_bound_question_batch(
                state,
                events,
                limits,
                queued,
                allow_follow_ups,
                &format!("closed-evidence question assessment was invalid: {error}"),
            )?;
            checkpoint_inquiry(checkpoint, events, state).await?;
            return Ok(());
        }
    };
    if allow_follow_ups {
        resolution
            .follow_up_questions
            .truncate(MAX_FOLLOW_UP_QUESTIONS_PER_WAVE);
    } else {
        resolution.follow_up_questions.clear();
    }
    let resolution_events =
        match question_resolution_events(&resolution, queued, &allowed_evidence_ids) {
            Ok(events) => events,
            Err(error) => {
                apply_pending_evidence(state, events, limits, evidence_catalog.pending)?;
                defer_or_bound_question_batch(
                    state,
                    events,
                    limits,
                    queued,
                    allow_follow_ups,
                    &format!("closed-evidence question assessment was invalid: {error}"),
                )?;
                checkpoint_inquiry(checkpoint, events, state).await?;
                return Ok(());
            }
        };
    apply_pending_evidence(state, events, limits, evidence_catalog.pending)?;
    for event in resolution_events {
        apply_event(state, events, event, limits)?;
    }
    checkpoint_inquiry(checkpoint, events, state).await?;
    Ok(())
}

/// Keep both the initial evidence base and the newest retrieval wave in a
/// bounded resolver packet. A first-N truncation can otherwise make a
/// successful follow-up invisible once earlier waves fill the packet.
pub(super) fn balanced_evidence_packet(
    evidence: &[AcceptedEvidence],
    maximum: usize,
) -> Vec<AcceptedEvidence> {
    if evidence.len() <= maximum {
        return evidence.to_vec();
    }
    if maximum == 0 {
        return Vec::new();
    }
    let leading = maximum / 2;
    let trailing = maximum.saturating_sub(leading);
    evidence
        .iter()
        .take(leading)
        .chain(
            evidence
                .iter()
                .skip(evidence.len().saturating_sub(trailing)),
        )
        .cloned()
        .collect()
}

pub(super) async fn run_dynamic_workflow(
    session: &AgentSession,
    args: Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
) -> Result<ToolCallResult, String> {
    let result =
        call_tool_with_progress(session, "dynamic_workflow", args, progress_tx, true).await?;
    if result.exit_code != 0 {
        return Err(result
            .output
            .lines()
            .next()
            .unwrap_or("dynamic_workflow failed without an error message")
            .to_string());
    }
    Ok(result)
}

pub(super) async fn call_tool_with_progress(
    session: &AgentSession,
    name: &str,
    args: Value,
    progress_tx: &mpsc::Sender<AgentEvent>,
    filter_dynamic_workflow_envelope: bool,
) -> Result<ToolCallResult, String> {
    let (progress_rx, join) = session.tool_with_events(name, args);
    forward_tool_call_with_progress(
        name,
        progress_rx,
        join,
        progress_tx,
        filter_dynamic_workflow_envelope,
    )
    .await
}

pub(super) async fn forward_tool_call_with_progress(
    name: &str,
    mut progress_rx: mpsc::Receiver<AgentEvent>,
    mut join: tokio::task::JoinHandle<a3s_code_core::Result<ToolCallResult>>,
    progress_tx: &mpsc::Sender<AgentEvent>,
    filter_dynamic_workflow_envelope: bool,
) -> Result<ToolCallResult, String> {
    let abort = join.abort_handle();
    let mut abort_on_drop = AbortInnerToolOnDrop(Some(abort.clone()));
    let mut progress_open = true;
    let result = loop {
        if !progress_open {
            let result = join
                .await
                .map_err(|error| format!("{name} task failed: {error}"))?
                .map_err(|error| format!("{name} failed: {error}"));
            abort_on_drop.disarm();
            break result;
        }
        tokio::select! {
            biased;
            event = progress_rx.recv() => {
                let Some(event) = event else {
                    progress_open = false;
                    continue;
                };
                if filter_dynamic_workflow_envelope && is_dynamic_workflow_envelope(&event) {
                    continue;
                }
                if progress_tx.send(event).await.is_err() {
                    abort.abort();
                    return Err("DeepResearch progress consumer closed".to_string());
                }
            }
            result = &mut join => {
                let result = result
                    .map_err(|error| format!("{name} task failed: {error}"))?
                    .map_err(|error| format!("{name} failed: {error}"));
                abort_on_drop.disarm();
                break result;
            }
        }
    };
    while let Ok(event) = progress_rx.try_recv() {
        if filter_dynamic_workflow_envelope && is_dynamic_workflow_envelope(&event) {
            continue;
        }
        if progress_tx.send(event).await.is_err() {
            break;
        }
    }
    result
}

fn is_dynamic_workflow_envelope(event: &AgentEvent) -> bool {
    match event {
        AgentEvent::ToolStart { name, .. }
        | AgentEvent::ToolExecutionStart { name, .. }
        | AgentEvent::ToolOutputDelta { name, .. }
        | AgentEvent::ToolEnd { name, .. } => name == "dynamic_workflow",
        _ => false,
    }
}

pub(super) fn generated_object<T: DeserializeOwned>(result: &ToolCallResult) -> Result<T, String> {
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
    let object = envelope
        .get("object")
        .cloned()
        .ok_or_else(|| "structured generation response omitted object".to_string())?;
    serde_json::from_value(object)
        .map_err(|error| format!("structured generation object violated its contract: {error}"))
}

fn accepted_source_ids(evidence: &[AcceptedEvidence]) -> BTreeSet<String> {
    evidence
        .iter()
        .flat_map(|item| item.sources.iter())
        .map(|source| source.id.clone())
        .collect()
}

fn accepted_evidence_ids(evidence: &[AcceptedEvidence]) -> BTreeSet<String> {
    evidence.iter().map(|item| item.id.clone()).collect()
}

struct PreparedEvidenceCatalog {
    addressable: Vec<AcceptedEvidence>,
    pending: Vec<EvidenceRef>,
}

/// Stage accepted evidence until the wave has a validated resolution outcome.
/// Evidence and answer/defer events then become one durable logical batch,
/// avoiding an fsync per evidence item and preventing half-applied waves.
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
        let accepted =
            EvidenceRef::new(item.id.clone(), claim_ids, source_ids).with_diagnostics(diagnostics);
        match state.evidence(&item.id) {
            Some(existing) if existing != &accepted => {
                return Err(format!(
                    "accepted evidence `{}` changed its claim/source relationships between retrieval waves",
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

fn bounded_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn merge_additional_workflow_result(
    base: &mut ToolCallResult,
    additional: &ToolCallResult,
    wave_id: &str,
) -> Result<(), String> {
    let base_output = deep_research_canonical_workflow_output(&base.output, base.metadata.as_ref());
    let additional_output =
        deep_research_canonical_workflow_output(&additional.output, additional.metadata.as_ref());
    let mut base_value = serde_json::from_str::<Value>(&base_output)
        .map_err(|error| format!("decode base inquiry output: {error}"))?;
    let additional_value = serde_json::from_str::<Value>(&additional_output)
        .unwrap_or_else(|_| Value::String(additional_output));
    let object = base_value
        .as_object_mut()
        .ok_or_else(|| "base inquiry output is not an object".to_string())?;
    let inquiry = object
        .entry("inquiry")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| "base inquiry field is not an object".to_string())?;
    let waves = inquiry
        .entry("retrieval_waves")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| "inquiry retrieval_waves field is not an array".to_string())?;
    waves.push(serde_json::json!({
        "id": wave_id,
        "output": additional_value,
    }));
    base.output = serde_json::to_string(&base_value)
        .map_err(|error| format!("encode merged inquiry output: {error}"))?;
    if let Some(snapshot) = base
        .metadata
        .as_mut()
        .and_then(|metadata| metadata.pointer_mut("/dynamic_workflow/snapshot"))
        .and_then(Value::as_object_mut)
    {
        snapshot.insert("output".to_string(), base_value);
    }
    Ok(())
}

fn combine_workflow_results(
    mut research: ToolCallResult,
    scout: &ToolCallResult,
    scout_output: &str,
    inquiry_events: &[InquiryEvent],
    state: &InquiryState,
) -> Result<ToolCallResult, String> {
    let research_output =
        deep_research_canonical_workflow_output(&research.output, research.metadata.as_ref());
    let mut combined = serde_json::from_str::<Value>(&research_output)
        .map_err(|error| format!("decode research workflow output: {error}"))?;
    let object = combined
        .as_object_mut()
        .ok_or_else(|| "research workflow returned a non-object output".to_string())?;
    object.insert(
        "inquiry".to_string(),
        serde_json::json!({
            "events": inquiry_events,
            "state": state,
            "scout": serde_json::from_str::<Value>(scout_output).unwrap_or_else(|_| Value::String(scout_output.to_string())),
        }),
    );
    research.output = serde_json::to_string(&combined)
        .map_err(|error| format!("encode combined inquiry output: {error}"))?;
    let metadata = research
        .metadata
        .get_or_insert_with(|| Value::Object(Map::new()));
    if let Some(snapshot) = metadata
        .pointer_mut("/dynamic_workflow/snapshot")
        .and_then(Value::as_object_mut)
    {
        snapshot.insert("output".to_string(), combined.clone());
    }
    if let Some(metadata) = metadata.as_object_mut() {
        metadata.insert(
            "inquiry".to_string(),
            serde_json::json!({
                "events": inquiry_events,
                "state": state,
                "scout_metadata": scout.metadata,
            }),
        );
    }
    Ok(research)
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
