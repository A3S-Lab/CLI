use std::collections::BTreeSet;
use std::future::pending;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use a3s::research::{
    perspective_discovery_events, DiscoveredPerspective, DiscoveredQuestion, InquiryEvent,
    InquiryLimits, InquiryPhase, InquiryState, PerspectiveDiscoveryOutput, QuestionStatus,
    ResearchMethod, ResearchObligation,
};
use a3s_code_core::llm::{
    LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
};
use a3s_code_core::tools::{Tool, ToolContext, ToolOutput};
use a3s_code_core::{Agent, AgentEvent, AgentSession, SessionOptions, ToolCallResult};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::execution::{
    assess_completed_research_contract, call_tool_with_progress, forward_tool_call_with_progress,
    resolve_questions_with_bounded_follow_up_waves, InquiryExecution,
};
use super::plan::{
    commit_plan_research_contract, perspective_research_plan, queue_plan_questions,
    queued_questions, workflow_args_with_plan,
};

fn inquiry_obligations() -> Vec<ResearchObligation> {
    vec![ResearchObligation::new(
        "track:material.v2",
        "Material obligation",
        "Resolve the material evidence obligation",
        true,
        vec!["A traceable answer or a bounded gap".to_string()],
    )]
}

pub(super) fn inquiry_plan() -> serde_json::Value {
    serde_json::json!({
        "answer_shape": "investigation",
        "report_title": "Bounded inquiry fixture",
        "freshness_required": false,
        "workspace_evidence_required": false,
        "research_method": "focused",
        "execution_route": "direct_then_review",
        "phases": ["collect", "check"],
        "tracks": [{
            "id": "track:material.v2",
            "title": "Material obligation",
            "focus": "Resolve the material evidence obligation",
            "perspective": "",
            "material": true,
            "questions": ["What does the retained evidence establish?"],
            "completion_criteria": ["A traceable answer or a bounded gap"]
        }],
        "scout_queries": [],
        "search_queries": ["fixture evidence"],
        "seed_urls": [],
        "budget": {
            "retrieval_timeout_ms": 30000,
            "synthesis_timeout_ms": 30000,
            "max_iterations": 2,
            "max_parallel_tasks": 2,
            "max_steps_per_task": 2,
            "direct_searches": 1,
            "direct_fetches": 1
        },
        "stop_conditions": ["The material obligation is resolved"]
    })
}

pub(super) fn workflow_args() -> serde_json::Value {
    serde_json::json!({
        "run_id": "inquiry-integration",
        "input": {
            "query": "fixture inquiry",
            "workflow_timeout_ms": 30000
        },
        "limits": {
            "timeoutMs": 30000
        }
    })
}

pub(super) fn evidence_output(label: &str) -> String {
    serde_json::json!({
        "query": "fixture inquiry",
        "structured": {
            "summary": format!("The {label} evidence establishes the bounded fixture fact."),
            "sources": [{
                "title": format!("{label} source"),
                "url_or_path": format!("https://example.test/{label}"),
                "quote_or_fact": format!("The {label} source contains the fixture fact."),
                "reliability": "authoritative fixture"
            }],
            "key_evidence": [format!("The {label} fixture fact is retained.")],
            "contradictions": [],
            "gaps": [],
            "confidence": "high"
        }
    })
    .to_string()
}

pub(super) fn successful_tool_result(name: &str, output: String) -> ToolCallResult {
    ToolCallResult {
        name: name.to_string(),
        output,
        exit_code: 0,
        metadata: None,
        error_kind: None,
    }
}

struct ScriptedResolutionClient {
    calls: AtomicUsize,
    evidence_id: String,
}

struct ContractAssessmentClient {
    calls: AtomicUsize,
    evidence_id: String,
}

impl ContractAssessmentClient {
    fn response(&self) -> LlmResponse {
        assert_eq!(self.calls.fetch_add(1, Ordering::SeqCst), 0);
        let value = serde_json::json!({
            "obligations": [{
                "obligation_id": "track:material.v2",
                "criteria": [{
                    "criterion_index": 0,
                    "status": "satisfied",
                    "rationale": "The accepted evidence directly supports the material criterion.",
                    "evidence_ids": [self.evidence_id]
                }]
            }],
            "stop_conditions": [{
                "condition_index": 0,
                "status": "satisfied",
                "rationale": "The material obligation is traceably resolved.",
                "evidence_ids": [self.evidence_id]
            }],
            "diagnostics": []
        });
        LlmResponse {
            message: Message::assistant(&value.to_string()),
            usage: TokenUsage::default(),
            stop_reason: Some("stop".to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }
}

#[async_trait::async_trait]
impl LlmClient for ContractAssessmentClient {
    async fn complete(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        Ok(self.response())
    }

    async fn complete_streaming(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = self.response();
        let text = response.message.text();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(StreamEvent::TextDelta(text)).await.ok();
            tx.send(StreamEvent::Done(response)).await.ok();
        });
        Ok(rx)
    }
}

impl ScriptedResolutionClient {
    fn new(evidence_id: String) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            evidence_id,
        }
    }

    fn next_response(&self) -> LlmResponse {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let (question_id, follow_up_questions) = match call {
            0 => (
                "question:plan-1-1",
                serde_json::json!([{
                    "id": "question:follow-up-1",
                    "parent_question_id": "question:plan-1-1",
                    "prompt": "Can the retained evidence change the material conclusion?",
                    "retrieval_query": "retained evidence material conclusion",
                    "material": true,
                    "round": 1
                }]),
            ),
            1 => (
                "question:follow-up-1",
                serde_json::json!([{
                    "id": "question:follow-up-2",
                    "parent_question_id": "question:follow-up-1",
                    "prompt": "Would another retrieval wave change the same conclusion?",
                    "retrieval_query": "additional evidence conclusion change",
                    "material": true,
                    "round": 2
                }]),
            ),
            2 => ("question:follow-up-2", serde_json::json!([])),
            other => panic!("unexpected structured generation call {other}"),
        };
        let text = serde_json::json!({
            "resolutions": [{
                "status": "answered",
                "question_id": question_id,
                "answer": "The retained fixture evidence answers the bounded question.",
                "evidence_ids": [self.evidence_id]
            }],
            "follow_up_questions": follow_up_questions
        })
        .to_string();
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
impl LlmClient for ScriptedResolutionClient {
    async fn complete(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        Ok(self.next_response())
    }

    async fn complete_streaming(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = self.next_response();
        let text = response.message.text();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(StreamEvent::TextDelta(text)).await.ok();
            tx.send(StreamEvent::Done(response)).await.ok();
        });
        Ok(rx)
    }
}

struct RecoveringResolutionClient {
    calls: AtomicUsize,
    evidence_id: String,
}

struct CoverageResolutionClient {
    calls: AtomicUsize,
    evidence_id: String,
}

impl CoverageResolutionClient {
    fn new(evidence_id: String) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            evidence_id,
        }
    }

    fn next_response(&self) -> LlmResponse {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let value = match call {
            0 => {
                let question_ids = [
                    "question:p1-q1",
                    "question:p2-q1",
                    "question:p3-q1",
                    "question:p4-q1",
                ];
                serde_json::json!({
                    "resolutions": question_ids.iter().map(|question_id| serde_json::json!({
                        "status": "bounded",
                        "question_id": question_id,
                        "reason": "The first retrieval wave exposes a consequential follow-up."
                    })).collect::<Vec<_>>(),
                    "follow_up_questions": question_ids.iter().enumerate().map(|(index, question_id)| serde_json::json!({
                        "id": format!("question:follow-up-{}", index + 1),
                        "parent_question_id": question_id,
                        "prompt": format!("Which additional evidence resolves follow-up {}?", index + 1),
                        "retrieval_query": format!("additional evidence follow-up {}", index + 1),
                        "material": true,
                        "round": 1
                    })).collect::<Vec<_>>()
                })
            }
            1 => answered_batch(
                &[
                    "question:p1-q2",
                    "question:p1-q3",
                    "question:p2-q2",
                    "question:p2-q3",
                ],
                &self.evidence_id,
            ),
            2 => answered_batch(
                &[
                    "question:p3-q2",
                    "question:p3-q3",
                    "question:p4-q2",
                    "question:p4-q3",
                ],
                &self.evidence_id,
            ),
            other => panic!("unexpected coverage generation call {other}"),
        };
        LlmResponse {
            message: Message::assistant(&value.to_string()),
            usage: TokenUsage::default(),
            stop_reason: Some("stop".to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }
}

fn answered_batch(question_ids: &[&str], evidence_id: &str) -> serde_json::Value {
    serde_json::json!({
        "resolutions": question_ids.iter().map(|question_id| serde_json::json!({
            "status": "answered",
            "question_id": question_id,
            "answer": "The scheduled retrieval wave answers this material question.",
            "evidence_ids": [evidence_id]
        })).collect::<Vec<_>>(),
        "follow_up_questions": []
    })
}

#[async_trait::async_trait]
impl LlmClient for CoverageResolutionClient {
    async fn complete(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        Ok(self.next_response())
    }

    async fn complete_streaming(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = self.next_response();
        let text = response.message.text();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(StreamEvent::TextDelta(text)).await.ok();
            tx.send(StreamEvent::Done(response)).await.ok();
        });
        Ok(rx)
    }
}

impl RecoveringResolutionClient {
    fn new(evidence_id: String) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            evidence_id,
        }
    }

    fn next_response(&self) -> LlmResponse {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let value = match call {
            0 => serde_json::json!({
                "resolutions": [{
                    "status": "bounded",
                    "question_id": "question:plan-1-1",
                    "reason": "The initial packet does not resolve the material claim."
                }],
                "follow_up_questions": [{
                    "id": "question:follow-up-1",
                    "parent_question_id": "question:plan-1-1",
                    "prompt": "Which retained primary evidence resolves the material claim?",
                    "retrieval_query": "primary evidence material claim",
                    "material": true,
                    "round": 1
                }]
            }),
            1 => serde_json::json!({
                "resolutions": [
                    {
                        "status": "answered",
                        "question_id": "question:plan-1-1",
                        "answer": "The follow-up evidence resolves the original material question.",
                        "evidence_ids": [self.evidence_id]
                    },
                    {
                        "status": "answered",
                        "question_id": "question:follow-up-1",
                        "answer": "The retained primary evidence closes the follow-up.",
                        "evidence_ids": [self.evidence_id]
                    }
                ],
                "follow_up_questions": []
            }),
            other => panic!("unexpected recovery generation call {other}"),
        };
        LlmResponse {
            message: Message::assistant(&value.to_string()),
            usage: TokenUsage::default(),
            stop_reason: Some("stop".to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }
}

#[async_trait::async_trait]
impl LlmClient for RecoveringResolutionClient {
    async fn complete(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        Ok(self.next_response())
    }

    async fn complete_streaming(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = self.next_response();
        let text = response.message.text();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(StreamEvent::TextDelta(text)).await.ok();
            tx.send(StreamEvent::Done(response)).await.ok();
        });
        Ok(rx)
    }
}

struct NoModelCalls;

#[async_trait::async_trait]
impl LlmClient for NoModelCalls {
    async fn complete(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        anyhow::bail!("this fixture must not call the model")
    }

    async fn complete_streaming(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        anyhow::bail!("this fixture must not call the model")
    }
}

struct EvidenceWorkflowTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for EvidenceWorkflowTool {
    fn name(&self) -> &str {
        "dynamic_workflow"
    }

    fn description(&self) -> &str {
        "Returns deterministic evidence for inquiry host integration tests."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(ToolOutput::success(evidence_output(&format!(
            "follow-up-{call}"
        ))))
    }
}

struct BlockingProbeTool {
    active: Arc<AtomicUsize>,
}

struct ActiveCallGuard(Arc<AtomicUsize>);

impl Drop for ActiveCallGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl Tool for BlockingProbeTool {
    fn name(&self) -> &str {
        "inquiry_blocking_probe"
    }

    fn description(&self) -> &str {
        "Blocks one direct call so abort cleanup can be verified."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        if args.get("block").and_then(serde_json::Value::as_bool) == Some(true) {
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = ActiveCallGuard(Arc::clone(&self.active));
            return pending::<anyhow::Result<ToolOutput>>().await;
        }
        Ok(ToolOutput::success("session reusable"))
    }
}

pub(super) async fn test_session(
    label: &str,
    client: Arc<dyn LlmClient>,
) -> (Agent, AgentSession, tempfile::TempDir) {
    let temp = tempfile::tempdir().expect("temp workspace");
    let config = temp.path().join("config.acl");
    std::fs::write(
        &config,
        "default_model = \"openai/x\"\n\
         providers \"openai\" {\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
         models \"x\" { name = \"x\" }\n}\n",
    )
    .expect("fixture config");
    let agent = Agent::new(config.to_string_lossy().to_string())
        .await
        .expect("fixture agent");
    let options = SessionOptions::new()
        .with_session_id(format!(
            "inquiry-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
        .with_llm_client(client)
        .with_auto_save(false);
    let session = agent
        .session_async(temp.path().to_string_lossy().to_string(), Some(options))
        .await
        .expect("fixture session");
    (agent, session, temp)
}

fn maximum_perspective_discovery() -> PerspectiveDiscoveryOutput {
    PerspectiveDiscoveryOutput {
        perspectives: (1..=4)
            .map(|perspective_index| DiscoveredPerspective {
                id: format!("perspective:p{perspective_index}"),
                title: format!("Perspective {perspective_index}"),
                focus: format!("Resolve perspective {perspective_index}"),
                source_ids: vec!["source:scout".to_string()],
                questions: (1..=3)
                    .map(|question_index| DiscoveredQuestion {
                        id: format!("question:p{perspective_index}-q{question_index}"),
                        prompt: format!(
                            "What evidence resolves perspective {perspective_index} question {question_index}?"
                        ),
                        retrieval_query: format!(
                            "perspective {perspective_index} evidence {question_index}"
                        ),
                        obligation_ids: vec!["track:material.v2".to_string()],
                        material: true,
                        round: 0,
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[tokio::test]
async fn closed_progress_channel_still_awaits_the_tool_join() {
    let (inner_tx, inner_rx) = mpsc::channel(1);
    drop(inner_tx);
    let (outer_tx, _outer_rx) = mpsc::channel(1);
    let completed = Arc::new(AtomicUsize::new(0));
    let completed_by_join = Arc::clone(&completed);
    let join = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(25)).await;
        completed_by_join.store(1, Ordering::SeqCst);
        Ok::<ToolCallResult, a3s_code_core::CodeError>(successful_tool_result(
            "fixture",
            "joined".to_string(),
        ))
    });

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        forward_tool_call_with_progress("fixture", inner_rx, join, &outer_tx, false),
    )
    .await
    .expect("closed progress channel must not spin")
    .expect("fixture join result");

    assert_eq!(result.output, "joined");
    assert_eq!(completed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn completed_questions_require_a_traceable_contract_assessment_before_reporting() {
    let output = evidence_output("contract");
    let evidence = super::super::accepted_evidence_ledger(&output, None);
    let accepted = evidence.first().expect("accepted evidence");
    let evidence_id = accepted.id.clone();
    let client = Arc::new(ContractAssessmentClient {
        calls: AtomicUsize::new(0),
        evidence_id: evidence_id.clone(),
    });
    let (_agent, session, _temp) = test_session(
        "contract-assessment",
        Arc::clone(&client) as Arc<dyn LlmClient>,
    )
    .await;

    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    let plan = inquiry_plan();
    commit_plan_research_contract(&plan, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&plan, None, &mut state, &mut events, &limits)
        .expect("material question");
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::EvidenceAccepted {
            evidence: a3s::research::EvidenceRef::new(
                evidence_id.clone(),
                accepted
                    .claims
                    .iter()
                    .map(|claim| claim.id.clone())
                    .collect(),
                accepted
                    .sources
                    .iter()
                    .map(|source| source.id.clone())
                    .collect(),
            ),
        },
        &limits,
    )
    .expect("accepted evidence");
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::QuestionAnswered {
            question_id: "question:plan-1-1".to_string(),
            answer: "The accepted evidence resolves the material criterion.".to_string(),
            evidence_ids: vec![evidence_id],
        },
        &limits,
    )
    .expect("answer");
    assert_eq!(state.phase, InquiryPhase::Outlining);
    assert!(state.contract_assessment.is_none());

    let execution = InquiryExecution {
        result: successful_tool_result("dynamic_workflow", output),
        retrieval_plan: plan.clone(),
        workflow_args: workflow_args_with_plan(workflow_args(), plan, None)
            .expect("workflow args"),
        follow_up_waves_remaining: 0,
    };
    let (progress_tx, _progress_rx) = mpsc::channel(16);
    let outcome = assess_completed_research_contract(
        &session,
        &progress_tx,
        &execution,
        &mut state,
        &mut events,
        &limits,
    )
    .await
    .expect("contract assessment");
    assert_eq!(
        outcome,
        a3s::research::ResearchContractOutcome::Satisfied
    );
    assert!(state.contract_assessment.is_some());
    assert!(matches!(
        events.last(),
        Some(InquiryEvent::ResearchContractAssessed { .. })
    ));
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
    session.close().await;
}

#[tokio::test]
async fn inquiry_executes_the_llm_selected_follow_up_wave_budget() {
    let initial_output = evidence_output("initial");
    let evidence = super::super::accepted_evidence_ledger(&initial_output, None);
    let evidence_id = evidence
        .first()
        .expect("accepted fixture evidence")
        .id
        .clone();
    let client = Arc::new(ScriptedResolutionClient::new(evidence_id));
    let (_agent, session, _temp) =
        test_session("follow-up", Arc::clone(&client) as Arc<dyn LlmClient>).await;
    let workflow_calls = Arc::new(AtomicUsize::new(0));
    session
        .register_dynamic_tool(Arc::new(EvidenceWorkflowTool {
            calls: Arc::clone(&workflow_calls),
        }))
        .expect("register workflow fixture");

    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    let plan = inquiry_plan();
    commit_plan_research_contract(&plan, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&plan, None, &mut state, &mut events, &limits).expect("initial question");
    let planned_args = workflow_args_with_plan(workflow_args(), plan.clone(), None)
        .expect("planned workflow args");
    let mut execution = InquiryExecution {
        result: successful_tool_result("dynamic_workflow", initial_output),
        retrieval_plan: plan,
        workflow_args: planned_args,
        follow_up_waves_remaining: 2,
    };
    let (progress_tx, _progress_rx) = mpsc::channel(32);

    resolve_questions_with_bounded_follow_up_waves(
        &session,
        &progress_tx,
        &mut execution,
        &mut state,
        &mut events,
        &limits,
    )
    .await
    .expect("bounded follow-up resolution");

    assert_eq!(workflow_calls.load(Ordering::SeqCst), 2);
    assert_eq!(client.calls.load(Ordering::SeqCst), 3);
    assert!(queued_questions(&state).is_empty());
    assert_eq!(state.questions.len(), 3);
    assert!(state
        .questions
        .iter()
        .all(|question| question.status == QuestionStatus::Answered));
    let output: serde_json::Value =
        serde_json::from_str(&execution.result.output).expect("merged workflow output");
    assert_eq!(
        output
            .pointer("/inquiry/retrieval_waves")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    session.close().await;
}

#[tokio::test]
async fn initial_material_questions_receive_retrieval_waves_before_deferred_questions_repeat() {
    let initial_output = evidence_output("initial-coverage");
    let evidence = super::super::accepted_evidence_ledger(&initial_output, None);
    let evidence_id = evidence
        .first()
        .expect("accepted fixture evidence")
        .id
        .clone();
    let client = Arc::new(CoverageResolutionClient::new(evidence_id));
    let (_agent, session, _temp) = test_session(
        "initial-coverage",
        Arc::clone(&client) as Arc<dyn LlmClient>,
    )
    .await;
    let workflow_calls = Arc::new(AtomicUsize::new(0));
    session
        .register_dynamic_tool(Arc::new(EvidenceWorkflowTool {
            calls: Arc::clone(&workflow_calls),
        }))
        .expect("register workflow fixture");

    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: ResearchMethod::PerspectiveGuided,
        },
        &limits,
    )
    .expect("strategy");
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::ResearchObligationsCommitted {
            obligations: inquiry_obligations(),
            stop_conditions: vec!["The material obligation is resolved".to_string()],
        },
        &limits,
    )
    .expect("research obligations");
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::ScoutCompleted {
            source_ids: vec!["source:scout".to_string()],
        },
        &limits,
    )
    .expect("scout");
    let discovery = maximum_perspective_discovery();
    for event in perspective_discovery_events(
        &discovery,
        &BTreeSet::from(["source:scout".to_string()]),
        &inquiry_obligations(),
    )
    .expect("perspective discovery")
    {
        super::apply_event(&mut state, &mut events, event, &limits).expect("discovery event");
    }

    let mut base = inquiry_plan();
    base["research_method"] = serde_json::Value::String("perspective_guided".to_string());
    base["scout_queries"] = serde_json::json!(["source landscape"]);
    base["budget"]["max_iterations"] = serde_json::Value::from(3);
    let retrieval_plan =
        perspective_research_plan(&base, &discovery).expect("initial perspective wave");
    assert_eq!(
        retrieval_plan["search_queries"].as_array().map(Vec::len),
        Some(4)
    );
    assert_eq!(state.questions.len(), 12);
    let planned_args = workflow_args_with_plan(workflow_args(), retrieval_plan.clone(), None)
        .expect("planned workflow args");
    let mut execution = InquiryExecution {
        result: successful_tool_result("dynamic_workflow", initial_output),
        retrieval_plan,
        workflow_args: planned_args,
        follow_up_waves_remaining: 2,
    };
    let (progress_tx, _progress_rx) = mpsc::channel(32);

    resolve_questions_with_bounded_follow_up_waves(
        &session,
        &progress_tx,
        &mut execution,
        &mut state,
        &mut events,
        &limits,
    )
    .await
    .expect("coverage-aware wave resolution");

    assert_eq!(workflow_calls.load(Ordering::SeqCst), 2);
    assert_eq!(client.calls.load(Ordering::SeqCst), 3);
    for perspective_index in 1..=4 {
        for question_index in 2..=3 {
            let id = format!("question:p{perspective_index}-q{question_index}");
            let question = state
                .questions
                .iter()
                .find(|question| question.id == id)
                .expect("initial material question");
            assert_eq!(question.status, QuestionStatus::Answered, "{id}");
            assert!(!events.iter().any(|event| matches!(
                event,
                InquiryEvent::QuestionBounded { question_id, .. } if question_id == &id
            )));
        }
    }
    session.close().await;
}

#[tokio::test]
async fn follow_up_wave_can_recover_a_previously_bounded_material_question() {
    let initial_output = evidence_output("initial");
    let evidence = super::super::accepted_evidence_ledger(&initial_output, None);
    let evidence_id = evidence
        .first()
        .expect("accepted fixture evidence")
        .id
        .clone();
    let client = Arc::new(RecoveringResolutionClient::new(evidence_id));
    let (_agent, session, _temp) =
        test_session("recover-bounded", Arc::clone(&client) as Arc<dyn LlmClient>).await;
    let workflow_calls = Arc::new(AtomicUsize::new(0));
    session
        .register_dynamic_tool(Arc::new(EvidenceWorkflowTool {
            calls: Arc::clone(&workflow_calls),
        }))
        .expect("register workflow fixture");

    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    let plan = inquiry_plan();
    commit_plan_research_contract(&plan, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&plan, None, &mut state, &mut events, &limits).expect("initial question");
    let planned_args = workflow_args_with_plan(workflow_args(), plan.clone(), None)
        .expect("planned workflow args");
    let mut execution = InquiryExecution {
        result: successful_tool_result("dynamic_workflow", initial_output),
        retrieval_plan: plan,
        workflow_args: planned_args,
        follow_up_waves_remaining: 1,
    };
    let (progress_tx, _progress_rx) = mpsc::channel(32);

    resolve_questions_with_bounded_follow_up_waves(
        &session,
        &progress_tx,
        &mut execution,
        &mut state,
        &mut events,
        &limits,
    )
    .await
    .expect("follow-up recovery");

    assert_eq!(workflow_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.calls.load(Ordering::SeqCst), 2);
    assert!(events.iter().any(|event| matches!(
        event,
        InquiryEvent::QuestionDeferred { question_id, .. }
            if question_id == "question:plan-1-1"
    )));
    assert_eq!(state.phase, InquiryPhase::Outlining);
    assert!(state
        .questions
        .iter()
        .all(|question| question.status == QuestionStatus::Answered));
    assert!(state
        .questions
        .iter()
        .all(|question| question.bound_reason.is_none()));
    session.close().await;
}

#[tokio::test]
async fn all_bounded_material_questions_exhaust_the_inquiry() {
    let (_agent, session, _temp) =
        test_session("bounded", Arc::new(NoModelCalls) as Arc<dyn LlmClient>).await;
    let limits = InquiryLimits::default();
    let mut state = InquiryState::default();
    let mut events = Vec::new();
    super::apply_event(
        &mut state,
        &mut events,
        InquiryEvent::StrategySelected {
            method: ResearchMethod::Focused,
        },
        &limits,
    )
    .expect("strategy");
    let plan = inquiry_plan();
    commit_plan_research_contract(&plan, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(&plan, None, &mut state, &mut events, &limits).expect("material question");
    let planned_args = workflow_args_with_plan(workflow_args(), plan.clone(), None)
        .expect("planned workflow args");
    let mut execution = InquiryExecution {
        result: successful_tool_result(
            "dynamic_workflow",
            serde_json::json!({
                "query": "fixture inquiry",
                "research": {"results": []}
            })
            .to_string(),
        ),
        retrieval_plan: plan,
        workflow_args: planned_args,
        follow_up_waves_remaining: 1,
    };
    let (progress_tx, _progress_rx) = mpsc::channel(8);

    resolve_questions_with_bounded_follow_up_waves(
        &session,
        &progress_tx,
        &mut execution,
        &mut state,
        &mut events,
        &limits,
    )
    .await
    .expect("bounded terminal inquiry");

    assert_eq!(state.phase, InquiryPhase::Exhausted);
    assert!(state.budget_exhausted_reason.is_some());
    assert_eq!(state.questions[0].status, QuestionStatus::Bounded);
    assert!(matches!(
        events.last(),
        Some(InquiryEvent::BudgetExhausted { .. })
    ));
    session.close().await;
}

#[test]
fn perspective_discovery_rejects_a_source_outside_the_scout_catalog() {
    let allowed = BTreeSet::from([
        "source:accepted-a".to_string(),
        "source:accepted-b".to_string(),
    ]);
    let mut output = PerspectiveDiscoveryOutput {
        perspectives: vec![
            DiscoveredPerspective {
                id: "perspective:first".to_string(),
                title: "First evidence perspective".to_string(),
                focus: "Test the first material implication".to_string(),
                source_ids: vec!["source:accepted-a".to_string()],
                questions: vec![DiscoveredQuestion {
                    id: "question:first".to_string(),
                    prompt: "What does the first accepted source establish?".to_string(),
                    retrieval_query: "first accepted source finding".to_string(),
                    obligation_ids: vec!["track:material.v2".to_string()],
                    material: true,
                    round: 0,
                }],
            },
            DiscoveredPerspective {
                id: "perspective:second".to_string(),
                title: "Independent evidence perspective".to_string(),
                focus: "Test the conclusion independently".to_string(),
                source_ids: vec!["source:accepted-b".to_string()],
                questions: vec![DiscoveredQuestion {
                    id: "question:second".to_string(),
                    prompt: "Does independent evidence support the conclusion?".to_string(),
                    retrieval_query: "independent evidence conclusion".to_string(),
                    obligation_ids: vec!["track:material.v2".to_string()],
                    material: true,
                    round: 0,
                }],
            },
        ],
    };
    output.perspectives[1].source_ids = vec!["source:fabricated".to_string()];

    let error = perspective_discovery_events(&output, &allowed, &inquiry_obligations())
        .expect_err("a model-authored source ID must fail closed");
    assert!(error.to_string().contains("unknown scout source id"));
}

#[tokio::test]
async fn dropping_a_timed_out_tool_forwarder_aborts_the_inner_tool() {
    let (_agent, session, _temp) =
        test_session("abort", Arc::new(NoModelCalls) as Arc<dyn LlmClient>).await;
    let active = Arc::new(AtomicUsize::new(0));
    session
        .register_dynamic_tool(Arc::new(BlockingProbeTool {
            active: Arc::clone(&active),
        }))
        .expect("register blocking probe");
    let (progress_tx, _progress_rx) = mpsc::channel::<AgentEvent>(16);

    let timed_out = tokio::time::timeout(
        Duration::from_millis(100),
        call_tool_with_progress(
            &session,
            "inquiry_blocking_probe",
            serde_json::json!({"block": true}),
            &progress_tx,
            false,
        ),
    )
    .await;
    assert!(timed_out.is_err(), "the blocking fixture should time out");
    tokio::time::timeout(Duration::from_secs(1), async {
        while active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("aborted inner tool must release its execution future");

    let reused = tokio::time::timeout(
        Duration::from_secs(1),
        call_tool_with_progress(
            &session,
            "inquiry_blocking_probe",
            serde_json::json!({"block": false}),
            &progress_tx,
            false,
        ),
    )
    .await
    .expect("reused tool deadline")
    .expect("session should remain reusable");
    assert_eq!(reused.output, "session reusable");
    session.close().await;
}
