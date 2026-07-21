use super::*;

use std::fs::OpenOptions;
use std::future::pending;
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s::research::{
    replay, CompletionCriterionAssessment, ContractAssessmentStatus, EvidenceRef, InquiryLimits,
    Question, ResearchContractAssessment, ResearchMethod, ResearchObligation,
    ResearchObligationAssessment, ResearchOutline, StopConditionAssessment,
};
use a3s_code_core::llm::{
    LlmClient, LlmResponse, Message, ModelGenerationConcurrency, StreamEvent, TokenUsage,
    ToolDefinition,
};
use a3s_code_core::{Agent, AgentSession, SessionOptions};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const PROCESS_ROLE_ENV: &str = "A3S_SECTIONED_REPORT_PROCESS_ROLE";
const PROCESS_WORKSPACE_ENV: &str = "A3S_SECTIONED_REPORT_PROCESS_WORKSPACE";
const SECTION_RUN_ID: &str = "process-section-resume";
const FRAME_RUN_ID: &str = "process-frame-resume";
const SEMANTIC_AUDIT_RUN_ID: &str = "process-semantic-audit-resume";
const SECOND_REPAIR_RUN_ID: &str = "bounded-second-semantic-repair";

#[derive(Clone)]
struct ProcessFixture {
    workflow_output: String,
    evidence: Vec<AcceptedEvidence>,
    outline: ResearchOutline,
    drafted_state: InquiryState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClientMode {
    BlockSecondSection,
    BlockLastSemanticAudit,
    CompleteReport,
    CompleteFrame,
    DenyModelCalls,
    ExerciseSecondRepair,
}

struct ProcessResumeClient {
    workspace: PathBuf,
    mode: ClientMode,
    outline: ResearchOutline,
    evidence: Vec<AcceptedEvidence>,
}

impl ProcessResumeClient {
    fn prompt(messages: &[Message]) -> String {
        messages
            .iter()
            .map(Message::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn record_call(&self, label: &str) -> anyhow::Result<()> {
        let path = self.workspace.join("model-invocations.log");
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{label}")?;
        file.flush()?;
        Ok(())
    }

    fn outline_response(&self) -> Value {
        serde_json::to_value(&self.outline).expect("serialize process fixture outline")
    }

    fn section_response(&self, section_id: &str) -> Value {
        let planned = self
            .outline
            .sections
            .iter()
            .find(|section| section.id == section_id)
            .expect("known process fixture section");
        let claim_id = planned.claim_ids.first().expect("section claim");
        let source_id = planned.source_ids.first().expect("section source");
        let claim = self
            .evidence
            .iter()
            .flat_map(|item| &item.claims)
            .find(|claim| claim.id == *claim_id)
            .expect("fixture claim");
        let source = self
            .evidence
            .iter()
            .flat_map(|item| &item.sources)
            .find(|source| source.id == *source_id)
            .expect("fixture source");
        json!({
            "section_id": section_id,
            "markdown": format!(
                "{} [{}]({})",
                claim.text,
                source.title.as_deref().unwrap_or("Accepted source"),
                source.anchor
            ),
        })
    }

    fn frame_response(&self) -> Value {
        json!({
            "report_title": "Process-resumed DeepResearch report",
            "reader_labels": {
                "qualification_heading": "Evidence boundaries",
                "qualification_intro": "The following consequential points remain bounded.",
                "sources_heading": "Sources",
                "decision_heading": "Decision guidance",
                "evidence_limitation": "Evidence limitation",
                "primary_source_support": "Primary-source support",
                "independent_corroboration": "Independent corroboration",
                "established_boundary": "The evidence establishes this point.",
                "qualified_boundary": "The evidence supports a qualified conclusion.",
                "unresolved_boundary": "The evidence does not establish this point."
            },
            "decision_guidance": [],
            "editorial": {
                "thesis": "The durable report recovers both accepted findings without repeating completed effects.",
                "track_coverage": [
                    {
                        "obligation_id": "obligation:alpha-report",
                        "status": "answered",
                        "finding": "The accepted alpha finding remains present after process restart.",
                        "interpretation": "Stable Flow identities preserve completed alpha work.",
                        "implication": "A restarted report can reuse its completed alpha section.",
                        "uncertainty": "The beta effect may still be ambiguous at interruption."
                    },
                    {
                        "obligation_id": "obligation:beta-report",
                        "status": "answered",
                        "finding": "The accepted beta finding remains present after process restart.",
                        "interpretation": "Stable Flow identities redeliver ambiguous beta work.",
                        "implication": "A restarted report can finish the remaining beta section.",
                        "uncertainty": "A running effect is redelivered because its completion is ambiguous."
                    }
                ]
            },
            "presentation": {
                "narrative_mode": "briefing",
                "archetype": "analytical",
                "palette": "ocean",
                "density": "balanced",
                "hero": "split",
                "visual_stance": "shifted",
                "rationale": "Two evidence findings form a compact recovery comparison for an engineering reader.",
                "section_plan": self.outline.sections.iter().enumerate().map(|(index, section)| {
                    json!({
                        "heading": section.heading,
                        "rhythm": if index == 0 { "anchor" } else { "breathing" },
                        "composition": "evidence",
                    })
                }).collect::<Vec<_>>()
            }
        })
    }

    fn semantic_audit_response(&self, target_id: &str) -> Value {
        let checks = || {
            json!({
                "claim_granularity": "clear",
                "derived_quantities": "clear",
                "temporal_labels": "clear",
                "compatibility_scope": "clear",
                "maintenance_scope": "clear",
                "replacement_properties": "clear",
                "promotional_attribution": "clear",
                "sample_scope": "clear",
                "unknown_item_quantifiers": "clear",
                "evidence_gap_scope": "clear",
                "recommendation_support": "clear",
                "reader_language_and_internal_jargon": "clear"
            })
        };
        json!({
            "reviews": [{
                "target_id": target_id,
                "checks": checks(),
                "issues": []
            }]
        })
    }

    fn semantic_issue_response(&self, target_id: &str, excerpt: &str) -> Value {
        let mut response = self.semantic_audit_response(target_id);
        response["reviews"][0]["checks"]["claim_granularity"] = Value::String("issue".to_string());
        response["reviews"][0]["issues"] = json!([{
            "category": "claim_granularity",
            "excerpt": excerpt,
            "detail": "The fixture requires one exact targeted repair before this target can pass."
        }]);
        response
    }

    fn classify(&self, messages: &[Message]) -> anyhow::Result<(String, Option<Value>)> {
        if self.mode == ClientMode::DenyModelCalls {
            self.record_call("unexpected:model")?;
            anyhow::bail!("completed Flow effect was unexpectedly executed again");
        }
        let prompt = Self::prompt(messages);
        if prompt.contains("CLOSED_OUTLINE_PACKET=") {
            self.record_call("outline")?;
            return Ok(("outline".to_string(), Some(self.outline_response())));
        }
        if prompt.contains("CLOSED_SECTION_PACKET=") {
            let section_id = self
                .outline
                .sections
                .iter()
                .find(|section| prompt.contains(&section.id))
                .map(|section| section.id.as_str())
                .ok_or_else(|| anyhow::anyhow!("section prompt omitted a fixture section id"))?;
            let label = format!("section:{section_id}");
            self.record_call(&label)?;
            if self.mode == ClientMode::BlockSecondSection && section_id == "section:2" {
                return Ok((label, None));
            }
            return Ok((label, Some(self.section_response(section_id))));
        }
        if prompt.contains("CLOSED_SECTION_REVISION_PACKET=") {
            let section_id = self
                .outline
                .sections
                .iter()
                .find(|section| prompt.contains(&section.id))
                .map(|section| section.id.as_str())
                .ok_or_else(|| anyhow::anyhow!("revision prompt omitted a fixture section id"))?;
            let label = format!("section-revision:{section_id}");
            self.record_call(&label)?;
            return Ok((label, Some(self.section_response(section_id))));
        }
        if prompt.contains("CLOSED_SEMANTIC_AUDIT_PACKET=") {
            let packet = prompt
                .split_once("CLOSED_SEMANTIC_AUDIT_PACKET=")
                .and_then(|(_, packet)| {
                    serde_json::Deserializer::from_str(packet)
                        .into_iter::<Value>()
                        .next()
                        .and_then(Result::ok)
                })
                .ok_or_else(|| anyhow::anyhow!("semantic audit prompt omitted its packet"))?;
            let target_id = packet
                .pointer("/target/target_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("semantic audit packet omitted its target id"))?;
            let label = format!("semantic-audit:{target_id}");
            let prior_invocations = invocation_count(&self.workspace, &label);
            self.record_call(&label)?;
            if self.mode == ClientMode::BlockLastSemanticAudit && target_id == "section:2" {
                return Ok((label, None));
            }
            if self.mode == ClientMode::ExerciseSecondRepair {
                let issue_excerpt = match (target_id, prior_invocations) {
                    ("section:1", 0) => Some("accepted alpha finding"),
                    ("section:2", 1) => Some("accepted beta finding"),
                    _ => None,
                };
                if let Some(excerpt) = issue_excerpt {
                    return Ok((
                        label,
                        Some(self.semantic_issue_response(target_id, excerpt)),
                    ));
                }
            }
            return Ok((label, Some(self.semantic_audit_response(target_id))));
        }
        if prompt.contains("CLOSED_REPORT_EDITORIAL_PACKET=") {
            let frame = self.frame_response();
            self.record_call("frame-editorial")?;
            return Ok((
                "frame-editorial".to_string(),
                Some(json!({
                    "report_title": frame["report_title"],
                    "reader_labels": frame["reader_labels"],
                    "editorial": frame["editorial"],
                })),
            ));
        }
        if prompt.contains("CLOSED_REPORT_GUIDANCE_PACKET=") {
            let frame = self.frame_response();
            self.record_call("frame-guidance")?;
            return Ok((
                "frame-guidance".to_string(),
                Some(json!({"decision_guidance": frame["decision_guidance"]})),
            ));
        }
        if prompt.contains("CLOSED_REPORT_PRESENTATION_PACKET=") {
            let frame = self.frame_response();
            self.record_call("frame-presentation")?;
            return Ok((
                "frame-presentation".to_string(),
                Some(json!({"presentation": frame["presentation"]})),
            ));
        }
        self.record_call("unexpected:unknown")?;
        anyhow::bail!("unexpected structured-generation prompt")
    }

    fn response(value: Value) -> LlmResponse {
        let text = value.to_string();
        LlmResponse {
            message: Message::assistant(&text),
            usage: TokenUsage::default(),
            stop_reason: Some("stop".to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }
}

#[async_trait::async_trait]
impl LlmClient for ProcessResumeClient {
    fn model_generation_concurrency(&self) -> ModelGenerationConcurrency {
        ModelGenerationConcurrency::bounded(
            NonZeroUsize::new(2).expect("fixture concurrency is non-zero"),
        )
    }

    async fn complete(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let (_, value) = self.classify(messages)?;
        match value {
            Some(value) => Ok(Self::response(value)),
            None => pending::<anyhow::Result<LlmResponse>>().await,
        }
    }

    async fn complete_streaming(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let (_, value) = self.classify(messages)?;
        let (tx, rx) = mpsc::channel(4);
        match value {
            Some(value) => {
                let response = Self::response(value);
                let text = response.message.text();
                tokio::spawn(async move {
                    tx.send(StreamEvent::TextDelta(text)).await.ok();
                    tx.send(StreamEvent::Done(response)).await.ok();
                });
            }
            None => {
                tokio::spawn(async move {
                    cancel_token.cancelled().await;
                    drop(tx);
                });
            }
        }
        Ok(rx)
    }
}

fn build_fixture() -> ProcessFixture {
    let mut workflow = json!({
        "query": "Verify durable report-stage recovery.",
        "research": {
            "status": "completed",
            "results": [{
                "summary": "Alpha evidence supports the accepted alpha finding.",
                "sources": [{
                    "title": "Alpha source",
                    "url_or_path": "https://example.test/alpha",
                    "quote_or_fact": "The alpha source establishes the accepted alpha finding.",
                    "reliability": "authoritative fixture"
                }],
                "key_evidence": ["The accepted alpha finding is established."],
                "contradictions": [],
                "gaps": [],
                "confidence": "high"
            }, {
                "summary": "Beta evidence supports the accepted beta finding.",
                "sources": [{
                    "title": "Beta source",
                    "url_or_path": "https://example.test/beta",
                    "quote_or_fact": "The beta source establishes the accepted beta finding.",
                    "reliability": "authoritative fixture"
                }],
                "key_evidence": ["The accepted beta finding is established."],
                "contradictions": [],
                "gaps": [],
                "confidence": "high"
            }]
        }
    });
    let evidence = accepted_evidence_ledger(&workflow.to_string(), None);
    assert_eq!(evidence.len(), 2, "fixture must retain two evidence items");
    let alpha = evidence
        .iter()
        .find(|item| item.sources[0].anchor.ends_with("/alpha"))
        .expect("alpha evidence");
    let beta = evidence
        .iter()
        .find(|item| item.sources[0].anchor.ends_with("/beta"))
        .expect("beta evidence");
    let alpha_obligation_id = "obligation:alpha-report";
    let beta_obligation_id = "obligation:beta-report";
    let mut alpha_question = Question::queued(
        "question:alpha",
        None,
        "What does the accepted alpha evidence establish?",
    );
    alpha_question.obligation_ids = vec![alpha_obligation_id.to_string()];
    let mut beta_question = Question::queued(
        "question:beta",
        None,
        "What does the accepted beta evidence establish?",
    );
    beta_question.obligation_ids = vec![beta_obligation_id.to_string()];
    let collected_events = vec![
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        InquiryEvent::ResearchObligationsCommitted {
            obligations: vec![
                ResearchObligation::new(
                    alpha_obligation_id,
                    "Alpha finding",
                    "Preserve the accepted alpha finding across report-stage interruption",
                    true,
                    vec!["The alpha finding remains traceable after recovery".to_string()],
                ),
                ResearchObligation::new(
                    beta_obligation_id,
                    "Beta finding",
                    "Preserve the accepted beta finding across report-stage interruption",
                    true,
                    vec!["The beta finding remains traceable after recovery".to_string()],
                ),
            ],
            stop_conditions: vec![
                "Both accepted findings are present in the audited report".to_string()
            ],
        },
        InquiryEvent::QuestionsQueued {
            questions: vec![alpha_question, beta_question],
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                alpha.id.clone(),
                alpha.claims.iter().map(|claim| claim.id.clone()).collect(),
                alpha
                    .sources
                    .iter()
                    .map(|source| source.id.clone())
                    .collect(),
            ),
        },
        InquiryEvent::EvidenceAccepted {
            evidence: EvidenceRef::new(
                beta.id.clone(),
                beta.claims.iter().map(|claim| claim.id.clone()).collect(),
                beta.sources
                    .iter()
                    .map(|source| source.id.clone())
                    .collect(),
            ),
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:alpha".to_string(),
            answer: "The accepted alpha evidence establishes the alpha finding.".to_string(),
            evidence_ids: vec![alpha.id.clone()],
        },
        InquiryEvent::QuestionAnswered {
            question_id: "question:beta".to_string(),
            answer: "The accepted beta evidence establishes the beta finding.".to_string(),
            evidence_ids: vec![beta.id.clone()],
        },
        InquiryEvent::ResearchContractAssessed {
            assessment: ResearchContractAssessment {
                obligations: vec![
                    ResearchObligationAssessment {
                        obligation_id: alpha_obligation_id.to_string(),
                        criteria: vec![CompletionCriterionAssessment {
                            criterion_index: 0,
                            status: ContractAssessmentStatus::Satisfied,
                            rationale: "The alpha finding has traceable evidence.".to_string(),
                            evidence_ids: vec![alpha.id.clone()],
                        }],
                        primary_source: None,
                        independent_corroboration: None,
                    },
                    ResearchObligationAssessment {
                        obligation_id: beta_obligation_id.to_string(),
                        criteria: vec![CompletionCriterionAssessment {
                            criterion_index: 0,
                            status: ContractAssessmentStatus::Satisfied,
                            rationale: "The beta finding has traceable evidence.".to_string(),
                            evidence_ids: vec![beta.id.clone()],
                        }],
                        primary_source: None,
                        independent_corroboration: None,
                    },
                ],
                stop_conditions: vec![StopConditionAssessment {
                    condition_index: 0,
                    status: ContractAssessmentStatus::Satisfied,
                    rationale: "The accepted evidence can support both report sections."
                        .to_string(),
                    evidence_ids: vec![alpha.id.clone(), beta.id.clone()],
                }],
                diagnostics: Vec::new(),
            },
        },
    ];
    let collected_state =
        replay(&collected_events, &InquiryLimits::default()).expect("collected fixture state");
    assert_eq!(collected_state.phase, InquiryPhase::Outlining);
    let outline = derive_outline(
        "Verify durable report-stage recovery.",
        &collected_state,
        &collected_state.outline_validation_context(),
    )
    .expect("derive process fixture outline");
    assert_eq!(outline.sections.len(), 2);
    let mut drafted_events = collected_events.clone();
    drafted_events.push(InquiryEvent::OutlineCommitted {
        outline: outline.clone(),
    });
    for section in &outline.sections {
        let claim = evidence
            .iter()
            .flat_map(|item| &item.claims)
            .find(|claim| section.claim_ids.contains(&claim.id))
            .expect("draft claim");
        let source = evidence
            .iter()
            .flat_map(|item| &item.sources)
            .find(|source| section.source_ids.contains(&source.id))
            .expect("draft source");
        drafted_events.push(InquiryEvent::SectionDrafted {
            section_id: section.id.clone(),
            content: format!(
                "{} [{}]({})",
                claim.text,
                source.title.as_deref().unwrap_or("Accepted source"),
                source.anchor
            ),
            citation_ids: section
                .claim_ids
                .iter()
                .chain(&section.source_ids)
                .cloned()
                .collect(),
        });
    }
    let drafted_state =
        replay(&drafted_events, &InquiryLimits::default()).expect("drafted fixture state");
    assert_eq!(drafted_state.phase, InquiryPhase::Auditing);
    workflow["inquiry"] = json!({
        "events": collected_events,
        "state": collected_state,
    });
    ProcessFixture {
        workflow_output: workflow.to_string(),
        evidence,
        outline,
        drafted_state,
    }
}

async fn worker_session(
    workspace: &Path,
    fixture: &ProcessFixture,
    mode: ClientMode,
) -> (Agent, AgentSession) {
    let config = workspace.join("config.acl");
    if !config.exists() {
        std::fs::write(
            &config,
            "default_model = \"openai/x\"\n\
             providers \"openai\" {\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
             models \"x\" { name = \"x\" }\n}\n",
        )
        .expect("write process fixture config");
    }
    let agent = Agent::new(config.to_string_lossy().to_string())
        .await
        .expect("process fixture agent");
    let client = Arc::new(ProcessResumeClient {
        workspace: workspace.to_path_buf(),
        mode,
        outline: fixture.outline.clone(),
        evidence: fixture.evidence.clone(),
    });
    let options = SessionOptions::new()
        .with_session_id("sectioned-report-process-worker")
        .with_llm_client(client)
        .with_auto_save(false)
        .with_tool_timeout(60_000);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(options))
        .await
        .expect("process fixture session");
    session
        .register_dynamic_workflow_runtime()
        .expect("register process fixture dynamic workflow");
    (agent, session)
}

async fn initialize_process_journal(workspace: &Path, run_id: &str) {
    super::super::deep_research_state_journal::record_workflow_started(
        workspace,
        run_id,
        super::super::deep_research_state_journal::ResearchSpec {
            query: "Verify durable report-stage recovery.".to_string(),
            current_date: "2026-07-18".to_string(),
            evidence_scope: "web_and_workspace".to_string(),
            required_claims: vec![
                "The accepted alpha finding is established.".to_string(),
                "The accepted beta finding is established.".to_string(),
            ],
            total_budget_ms: 60_000,
            retrieval_stage_budget_ms: 30_000,
            question_review_stage_budget_ms: 20_000,
            finalization_reserve_ms: 5_000,
            host_pid: std::process::id(),
        },
    )
    .await
    .expect("initialize process report journal");
}

async fn run_section_worker(workspace: &Path, role: &str) {
    let fixture = build_fixture();
    initialize_process_journal(workspace, SECTION_RUN_ID).await;
    let mode = if role == "section-interrupt" {
        ClientMode::BlockSecondSection
    } else {
        ClientMode::CompleteReport
    };
    let (_agent, session) = worker_session(workspace, &fixture, mode).await;
    let mut workflow_output = fixture.workflow_output.clone();
    let mut workflow_metadata = None;
    let result = super::super::complete_deep_research_cli_sectioned_report(
        &session,
        "Verify durable report-stage recovery.",
        &mut workflow_output,
        &mut workflow_metadata,
        SECTION_RUN_ID,
        60_000,
    )
    .await;
    if role == "section-interrupt" {
        panic!("interrupted section worker returned before the parent terminated it: {result:?}");
    }
    let result = result.expect("resumed sectioned report");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    std::fs::write(workspace.join("section-result.json"), result.output)
        .expect("write resumed section result");
    session.close().await;
}

async fn run_frame_worker(workspace: &Path, role: &str) {
    let fixture = build_fixture();
    let mode = if role == "frame-interrupt" {
        ClientMode::CompleteFrame
    } else {
        ClientMode::DenyModelCalls
    };
    let (_agent, session) = worker_session(workspace, &fixture, mode).await;
    let deadline = ReportDeadline::new(Instant::now() + Duration::from_secs(60));
    let frame = generate_frame(
        &session,
        "Verify durable report-stage recovery.",
        FRAME_RUN_ID,
        &fixture.workflow_output,
        &fixture.outline,
        &fixture.drafted_state,
        &fixture.evidence,
        None,
        &deadline,
    )
    .await
    .expect("frame generation or recovery");
    assert_eq!(frame.report_title, "Process-resumed DeepResearch report");
    if role == "frame-interrupt" {
        std::fs::write(workspace.join("frame-effect-returned"), b"completed")
            .expect("write frame completion marker");
        pending::<()>().await;
    }
    std::fs::write(
        workspace.join("frame-result.json"),
        serde_json::to_vec(&json!({"report_title": frame.report_title}))
            .expect("encode frame result"),
    )
    .expect("write resumed frame result");
    session.close().await;
}

async fn run_semantic_audit_worker(workspace: &Path, role: &str) {
    let fixture = build_fixture();
    let mode = if role == "semantic-audit-interrupt" {
        ClientMode::BlockLastSemanticAudit
    } else {
        ClientMode::CompleteReport
    };
    let (_agent, session) = worker_session(workspace, &fixture, mode).await;
    let deadline = ReportDeadline::new(Instant::now() + Duration::from_secs(60));
    let frame = generate_frame(
        &session,
        "Verify durable report-stage recovery.",
        SEMANTIC_AUDIT_RUN_ID,
        &fixture.workflow_output,
        &fixture.outline,
        &fixture.drafted_state,
        &fixture.evidence,
        None,
        &deadline,
    )
    .await
    .expect("semantic-audit fixture frame generation or recovery");
    let sections = recovery::sections_from_drafts(&fixture.outline, &fixture.drafted_state)
        .expect("restore semantic-audit fixture sections");
    let result = semantic_audit::audit_report_semantics(
        &session,
        "Verify durable report-stage recovery.",
        SEMANTIC_AUDIT_RUN_ID,
        "semantic_audit_1",
        &fixture.outline,
        &fixture.drafted_state,
        &sections,
        &frame,
        &fixture.evidence,
        &deadline,
    )
    .await;
    if role == "semantic-audit-interrupt" {
        panic!("interrupted semantic-audit worker returned before termination: {result:?}");
    }
    let review = result.expect("resumed target-level semantic audit");
    assert!(review.passed());
    std::fs::write(
        workspace.join("semantic-audit-result.json"),
        serde_json::to_vec(&review).expect("encode semantic-audit result"),
    )
    .expect("write resumed semantic-audit result");
    session.close().await;
}

fn exact_test_name(function: &str) -> String {
    let module = module_path!();
    let module = module.strip_prefix("a3s::").unwrap_or(module);
    format!("{module}::{function}")
}

fn spawn_worker(test_name: &str, workspace: &Path, role: &str) -> Child {
    Command::new(std::env::current_exe().expect("current test executable"))
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(PROCESS_ROLE_ENV, role)
        .env(PROCESS_WORKSPACE_ENV, workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn process-resume worker")
}

async fn wait_for_condition(
    description: &str,
    timeout: Duration,
    mut condition: impl FnMut() -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_success(child: &mut Child, description: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait().expect("poll worker") {
            assert!(status.success(), "{description} exited with {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{description} did not finish within 60 seconds");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn event_count(path: &Path, event_type: &str, step_id: Option<&str>) -> usize {
    std::fs::read_to_string(path)
        .ok()
        .into_iter()
        .flat_map(|text| {
            text.lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .collect::<Vec<_>>()
        })
        .filter(|event| {
            event.pointer("/event/type").and_then(Value::as_str) == Some(event_type)
                && step_id.is_none_or(|step_id| {
                    event.pointer("/event/step_id").and_then(Value::as_str) == Some(step_id)
                })
        })
        .count()
}

fn invocation_count(workspace: &Path, label: &str) -> usize {
    std::fs::read_to_string(workspace.join("model-invocations.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == label)
        .count()
}

fn flow_journals_with_prefix(workspace: &Path, prefix: &str) -> Vec<PathBuf> {
    let root = workspace.join(".a3s/workflow");
    let mut paths = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".jsonl"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_semantic_failure_triggers_the_final_targeted_repair() {
    let workspace = tempfile::tempdir().expect("bounded second-repair workspace");
    let fixture = build_fixture();
    initialize_process_journal(workspace.path(), SECOND_REPAIR_RUN_ID).await;
    let (_agent, session) =
        worker_session(workspace.path(), &fixture, ClientMode::ExerciseSecondRepair).await;
    let mut workflow_output = fixture.workflow_output.clone();
    let mut workflow_metadata = None;

    let result = super::super::complete_deep_research_cli_sectioned_report(
        &session,
        "Verify durable report-stage recovery.",
        &mut workflow_output,
        &mut workflow_metadata,
        SECOND_REPAIR_RUN_ID,
        60_000,
    )
    .await
    .expect("two bounded semantic repairs should produce a publishable report");

    assert_eq!(result.exit_code, 0, "{}", result.output);
    assert_eq!(invocation_count(workspace.path(), "frame-editorial"), 1);
    assert_eq!(invocation_count(workspace.path(), "frame-guidance"), 1);
    assert_eq!(invocation_count(workspace.path(), "frame-presentation"), 1);
    assert_eq!(
        invocation_count(workspace.path(), "section-revision:section:1"),
        1
    );
    assert_eq!(
        invocation_count(workspace.path(), "section-revision:section:2"),
        1
    );
    assert_eq!(
        invocation_count(workspace.path(), "semantic-audit:frame"),
        2,
        "the unchanged frame is covered by the two full audits and skipped by the final targeted re-audit"
    );
    assert_eq!(
        invocation_count(workspace.path(), "semantic-audit:section:1"),
        2
    );
    assert_eq!(
        invocation_count(workspace.path(), "semantic-audit:section:2"),
        3,
        "only the second full audit failure is redelivered in the final semantic pass"
    );

    let (events, state) = super::super::deep_research_state_journal::load_inquiry_state(
        workspace.path(),
        SECOND_REPAIR_RUN_ID,
    )
    .await
    .expect("load second-repair Inquiry")
    .expect("second-repair Inquiry exists");
    assert_eq!(state.phase, InquiryPhase::Completed);
    assert_eq!(state.section_revisions.len(), 2);
    assert_eq!(state.audit_attempts, 3);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, InquiryEvent::AuditCompleted { passed: false, .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, InquiryEvent::AuditCompleted { passed: true, .. }))
            .count(),
        1
    );
    session.close().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_interruption_resumes_completed_section_effects() {
    if let Ok(role) = std::env::var(PROCESS_ROLE_ENV) {
        let workspace = PathBuf::from(
            std::env::var_os(PROCESS_WORKSPACE_ENV).expect("process worker workspace"),
        );
        run_section_worker(&workspace, &role).await;
        return;
    }

    let workspace = tempfile::tempdir().expect("process section workspace");
    let test_name = exact_test_name("process_interruption_resumes_completed_section_effects");
    let first_section_prefix = format!("{SECTION_RUN_ID}-sections-section_1-");
    let second_section_prefix = format!("{SECTION_RUN_ID}-sections-section_2-");
    let mut interrupted = spawn_worker(&test_name, workspace.path(), "section-interrupt");
    wait_for_condition(
        "one completed section and one running section",
        Duration::from_secs(60),
        || {
            let first = flow_journals_with_prefix(workspace.path(), first_section_prefix.as_str());
            let second =
                flow_journals_with_prefix(workspace.path(), second_section_prefix.as_str());
            if first.len() != 1 || second.len() != 1 {
                return false;
            }
            event_count(&first[0], "step_completed", Some("section_1")) == 1
                && event_count(&second[0], "step_started", Some("section_2")) == 1
                && invocation_count(workspace.path(), "section:section:2") == 1
        },
    )
    .await;
    let first_section_journals =
        flow_journals_with_prefix(workspace.path(), first_section_prefix.as_str());
    let second_section_journals =
        flow_journals_with_prefix(workspace.path(), second_section_prefix.as_str());
    assert_eq!(first_section_journals.len(), 1);
    assert_eq!(second_section_journals.len(), 1);
    assert!(
        !workspace
            .path()
            .join(".a3s/workflow")
            .join(format!("{SECTION_RUN_ID}-sections.jsonl"))
            .exists(),
        "section orchestration must not collapse independent effects into one serial Flow"
    );
    interrupted
        .kill()
        .expect("forcefully interrupt section worker");
    let interrupted_status = interrupted.wait().expect("reap interrupted section worker");
    assert!(!interrupted_status.success());

    let mut resumed = spawn_worker(&test_name, workspace.path(), "section-resume");
    wait_for_success(&mut resumed, "resumed section worker").await;
    assert!(workspace.path().join("section-result.json").is_file());
    assert_eq!(invocation_count(workspace.path(), "outline"), 0);
    assert_eq!(
        invocation_count(workspace.path(), "section:section:1"),
        1,
        "the completed section effect must not execute again"
    );
    assert_eq!(
        invocation_count(workspace.path(), "section:section:2"),
        2,
        "the ambiguous running effect must be redelivered"
    );
    assert_eq!(invocation_count(workspace.path(), "frame-editorial"), 1);
    assert_eq!(invocation_count(workspace.path(), "frame-guidance"), 1);
    assert_eq!(invocation_count(workspace.path(), "frame-presentation"), 1);
    assert_eq!(
        invocation_count(workspace.path(), "semantic-audit:frame"),
        1
    );
    assert_eq!(
        invocation_count(workspace.path(), "semantic-audit:section:1"),
        1
    );
    assert_eq!(
        invocation_count(workspace.path(), "semantic-audit:section:2"),
        1
    );
    assert_eq!(
        event_count(
            &first_section_journals[0],
            "step_started",
            Some("section_1")
        ),
        1
    );
    assert_eq!(
        event_count(
            &first_section_journals[0],
            "step_completed",
            Some("section_1")
        ),
        1
    );
    assert_eq!(
        event_count(
            &second_section_journals[0],
            "step_started",
            Some("section_2")
        ),
        1,
        "Flow redelivery reuses the interrupted attempt"
    );
    assert_eq!(
        event_count(
            &second_section_journals[0],
            "step_completed",
            Some("section_2")
        ),
        1
    );
    assert_eq!(
        event_count(&first_section_journals[0], "run_completed", None),
        1
    );
    assert_eq!(
        event_count(&second_section_journals[0], "run_completed", None),
        1
    );
    assert_eq!(
        flow_journals_with_prefix(workspace.path(), first_section_prefix.as_str()),
        first_section_journals
    );
    assert_eq!(
        flow_journals_with_prefix(workspace.path(), second_section_prefix.as_str()),
        second_section_journals
    );

    let (events, state) = super::super::deep_research_state_journal::load_inquiry_state(
        workspace.path(),
        SECTION_RUN_ID,
    )
    .await
    .expect("load resumed report Inquiry")
    .expect("resumed report Inquiry exists");
    assert_eq!(state.phase, InquiryPhase::Completed);
    assert_eq!(state.drafts.len(), 2);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, InquiryEvent::OutlineCommitted { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, InquiryEvent::AuditCompleted { passed: true, .. }))
            .count(),
        1
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_interruption_reuses_completed_semantic_target_audits() {
    if let Ok(role) = std::env::var(PROCESS_ROLE_ENV) {
        let workspace = PathBuf::from(
            std::env::var_os(PROCESS_WORKSPACE_ENV).expect("process worker workspace"),
        );
        run_semantic_audit_worker(&workspace, &role).await;
        return;
    }

    let workspace = tempfile::tempdir().expect("process semantic-audit workspace");
    let test_name = exact_test_name("process_interruption_reuses_completed_semantic_target_audits");
    let target_prefix =
        |ordinal: usize| format!("{SEMANTIC_AUDIT_RUN_ID}-semantic_audit_1_target_{ordinal}-");
    let mut interrupted = spawn_worker(&test_name, workspace.path(), "semantic-audit-interrupt");
    wait_for_condition(
        "two completed semantic targets and one running target",
        Duration::from_secs(60),
        || {
            let first = flow_journals_with_prefix(workspace.path(), &target_prefix(1));
            let second = flow_journals_with_prefix(workspace.path(), &target_prefix(2));
            let third = flow_journals_with_prefix(workspace.path(), &target_prefix(3));
            first.len() == 1
                && second.len() == 1
                && third.len() == 1
                && event_count(&first[0], "run_completed", None) == 1
                && event_count(&second[0], "run_completed", None) == 1
                && event_count(&third[0], "step_started", Some("semantic_audit_1_target_3")) == 1
                && invocation_count(workspace.path(), "semantic-audit:section:2") == 1
        },
    )
    .await;
    let journals = (1..=3)
        .map(|ordinal| {
            let paths = flow_journals_with_prefix(workspace.path(), &target_prefix(ordinal));
            assert_eq!(paths.len(), 1);
            paths[0].clone()
        })
        .collect::<Vec<_>>();
    interrupted
        .kill()
        .expect("forcefully interrupt semantic-audit worker");
    let interrupted_status = interrupted
        .wait()
        .expect("reap interrupted semantic-audit worker");
    assert!(!interrupted_status.success());

    let mut resumed = spawn_worker(&test_name, workspace.path(), "semantic-audit-resume");
    wait_for_success(&mut resumed, "resumed semantic-audit worker").await;
    assert!(workspace
        .path()
        .join("semantic-audit-result.json")
        .is_file());
    assert_eq!(invocation_count(workspace.path(), "frame-editorial"), 1);
    assert_eq!(invocation_count(workspace.path(), "frame-guidance"), 1);
    assert_eq!(invocation_count(workspace.path(), "frame-presentation"), 1);
    assert_eq!(
        invocation_count(workspace.path(), "semantic-audit:frame"),
        1
    );
    assert_eq!(
        invocation_count(workspace.path(), "semantic-audit:section:1"),
        1
    );
    assert_eq!(
        invocation_count(workspace.path(), "semantic-audit:section:2"),
        2,
        "only the ambiguous running target should be redelivered"
    );
    for (index, path) in journals.iter().enumerate() {
        let step_id = format!("semantic_audit_1_target_{}", index + 1);
        assert_eq!(event_count(path, "step_started", Some(&step_id)), 1);
        assert_eq!(event_count(path, "step_completed", Some(&step_id)), 1);
        assert_eq!(event_count(path, "run_completed", None), 1);
        assert_eq!(
            flow_journals_with_prefix(workspace.path(), &target_prefix(index + 1)),
            std::slice::from_ref(path)
        );
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_interruption_reuses_completed_frame_effect() {
    if let Ok(role) = std::env::var(PROCESS_ROLE_ENV) {
        let workspace = PathBuf::from(
            std::env::var_os(PROCESS_WORKSPACE_ENV).expect("process worker workspace"),
        );
        run_frame_worker(&workspace, &role).await;
        return;
    }

    let workspace = tempfile::tempdir().expect("process frame workspace");
    let test_name = exact_test_name("process_interruption_reuses_completed_frame_effect");
    let mut interrupted = spawn_worker(&test_name, workspace.path(), "frame-interrupt");
    wait_for_condition(
        "completed frame effect before caller acknowledgement",
        Duration::from_secs(60),
        || workspace.path().join("frame-effect-returned").is_file(),
    )
    .await;
    let frame_effects = [
        ("frame_editorial", "frame-editorial"),
        ("frame_guidance", "frame-guidance"),
        ("frame_presentation", "frame-presentation"),
    ];
    let frame_journals = frame_effects
        .iter()
        .map(|(step_id, _)| {
            let paths =
                flow_journals_with_prefix(workspace.path(), &format!("{FRAME_RUN_ID}-{step_id}-"));
            assert_eq!(paths.len(), 1);
            assert_eq!(
                event_count(&paths[0], "run_completed", None),
                1,
                "every frame sub-effect must be durable before interruption"
            );
            paths[0].clone()
        })
        .collect::<Vec<_>>();
    interrupted
        .kill()
        .expect("forcefully interrupt frame worker");
    let interrupted_status = interrupted.wait().expect("reap interrupted frame worker");
    assert!(!interrupted_status.success());

    let mut resumed = spawn_worker(&test_name, workspace.path(), "frame-resume");
    wait_for_success(&mut resumed, "resumed frame worker").await;
    assert!(workspace.path().join("frame-result.json").is_file());
    for (_, invocation) in frame_effects {
        assert_eq!(
            invocation_count(workspace.path(), invocation),
            1,
            "a completed frame sub-effect must be reused without another model call"
        );
    }
    assert_eq!(invocation_count(workspace.path(), "unexpected:model"), 0);
    for ((step_id, _), original) in frame_effects.iter().zip(&frame_journals) {
        let resumed =
            flow_journals_with_prefix(workspace.path(), &format!("{FRAME_RUN_ID}-{step_id}-"));
        assert_eq!(resumed, std::slice::from_ref(original));
        assert_eq!(event_count(&resumed[0], "step_completed", Some(step_id)), 1);
        assert_eq!(event_count(&resumed[0], "run_completed", None), 1);
    }
}
