use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use a3s::research::{InquiryEvent, InquiryLimits, InquiryState, QuestionStatus, ResearchMethod};
use a3s_code_core::llm::{
    LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
};
use a3s_code_core::tools::{Tool, ToolContext, ToolOutput};
use a3s_code_core::{AgentEvent, ToolCallResult};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::execution::{resolve_questions_with_bounded_follow_up_waves, InquiryExecution};
use super::integration_tests::{
    evidence_output, inquiry_plan, successful_tool_result, test_session, workflow_args,
};
use super::plan::{commit_plan_research_contract, queue_plan_questions, workflow_args_with_plan};

struct SequenceResolutionClient {
    calls: AtomicUsize,
    responses: Mutex<VecDeque<String>>,
}

impl SequenceResolutionClient {
    fn new(responses: impl IntoIterator<Item = serde_json::Value>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            responses: Mutex::new(
                responses
                    .into_iter()
                    .map(|response| response.to_string())
                    .collect(),
            ),
        }
    }

    fn next_response(&self) -> anyhow::Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = self
            .responses
            .lock()
            .expect("response queue")
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("unexpected structured generation call"))?;
        Ok(LlmResponse {
            message: Message::assistant(&text),
            usage: TokenUsage::default(),
            stop_reason: Some("stop".to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for SequenceResolutionClient {
    async fn complete(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.next_response()
    }

    async fn complete_streaming(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = self.next_response()?;
        let text = response.message.text();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(StreamEvent::TextDelta(text)).await.ok();
            tx.send(StreamEvent::Done(response)).await.ok();
        });
        Ok(rx)
    }
}

struct RetryWorkflowTool {
    calls: Arc<AtomicUsize>,
    failures_remaining: AtomicUsize,
}

impl RetryWorkflowTool {
    fn new(calls: Arc<AtomicUsize>, failures: usize) -> Self {
        Self {
            calls,
            failures_remaining: AtomicUsize::new(failures),
        }
    }
}

#[async_trait::async_trait]
impl Tool for RetryWorkflowTool {
    fn name(&self) -> &str {
        "dynamic_workflow"
    }

    fn description(&self) -> &str {
        "Returns deterministic retry evidence for inquiry lifecycle tests."
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
        if self
            .failures_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            anyhow::bail!("transient workflow failure {call}");
        }
        Ok(ToolOutput::success(evidence_output(&format!(
            "retry-{call}"
        ))))
    }
}

fn inquiry_state(plan: &serde_json::Value) -> (InquiryState, Vec<InquiryEvent>, InquiryLimits) {
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
    commit_plan_research_contract(plan, &mut state, &mut events, &limits)
        .expect("stable research contract");
    queue_plan_questions(plan, None, &mut state, &mut events, &limits).expect("focused question");
    (state, events, limits)
}

fn empty_workflow_result() -> ToolCallResult {
    successful_tool_result(
        "dynamic_workflow",
        serde_json::json!({
            "query": "fixture inquiry",
            "research": {"status": "failed", "results": []}
        })
        .to_string(),
    )
}

#[tokio::test]
async fn empty_evidence_bounds_the_same_frontier_without_an_identical_retry() {
    let client = Arc::new(SequenceResolutionClient::new(std::iter::empty::<
        serde_json::Value,
    >()));
    let (_agent, session, _temp) = test_session(
        "empty-evidence-retry",
        Arc::clone(&client) as Arc<dyn LlmClient>,
    )
    .await;
    let workflow_calls = Arc::new(AtomicUsize::new(0));
    session
        .register_dynamic_tool(Arc::new(RetryWorkflowTool::new(
            Arc::clone(&workflow_calls),
            0,
        )))
        .expect("retry workflow");
    let plan = inquiry_plan();
    let (mut state, mut events, limits) = inquiry_state(&plan);
    let planned_args = workflow_args_with_plan(workflow_args(), plan.clone(), None)
        .expect("planned workflow args");
    let mut execution = InquiryExecution {
        result: empty_workflow_result(),
        retrieval_plan: plan,
        workflow_args: planned_args,
        follow_up_waves_remaining: 1,
    };
    let (progress_tx, _progress_rx) = mpsc::channel::<AgentEvent>(16);

    resolve_questions_with_bounded_follow_up_waves(
        &session,
        &progress_tx,
        &mut execution,
        &mut state,
        &mut events,
        &limits,
        None,
    )
    .await
    .expect("converge after empty evidence");

    assert_eq!(workflow_calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.calls.load(Ordering::SeqCst), 0);
    assert_eq!(state.questions[0].status, QuestionStatus::Bounded);
    assert!(events.iter().any(|event| matches!(
        event,
        InquiryEvent::QuestionDeferred { reason, .. }
            if reason.contains("no accepted evidence")
    )));
    assert!(events.iter().any(
        |event| matches!(event, InquiryEvent::QuestionBounded { reason, .. }
            if reason.contains("equivalent normalized query"))
    ));
    session.close().await;
}

#[tokio::test]
async fn failed_follow_up_frontier_is_not_retried_under_the_same_question() {
    let initial_output = evidence_output("initial");
    let client = Arc::new(SequenceResolutionClient::new([serde_json::json!({
        "resolutions": [{
            "status": "bounded",
            "question_id": "question:plan-1-1",
            "reason": "The first packet exposes a consequential evidence gap."
        }],
        "follow_up_questions": [{
            "id": "question:retry-follow-up",
            "parent_question_id": "question:plan-1-1",
            "prompt": "Which retry evidence closes the consequential gap?",
            "retrieval_query": "retry evidence consequential gap",
            "material": true,
            "round": 1
        }]
    })]));
    let (_agent, session, _temp) = test_session(
        "workflow-failure-retry",
        Arc::clone(&client) as Arc<dyn LlmClient>,
    )
    .await;
    let workflow_calls = Arc::new(AtomicUsize::new(0));
    session
        .register_dynamic_tool(Arc::new(RetryWorkflowTool::new(
            Arc::clone(&workflow_calls),
            1,
        )))
        .expect("retry workflow");
    let plan = inquiry_plan();
    let (mut state, mut events, limits) = inquiry_state(&plan);
    let planned_args = workflow_args_with_plan(workflow_args(), plan.clone(), None)
        .expect("planned workflow args");
    let mut execution = InquiryExecution {
        result: successful_tool_result("dynamic_workflow", initial_output),
        retrieval_plan: plan,
        workflow_args: planned_args,
        follow_up_waves_remaining: 2,
    };
    let (progress_tx, _progress_rx) = mpsc::channel::<AgentEvent>(16);

    resolve_questions_with_bounded_follow_up_waves(
        &session,
        &progress_tx,
        &mut execution,
        &mut state,
        &mut events,
        &limits,
        None,
    )
    .await
    .expect("converge after follow-up workflow failure");

    assert_eq!(workflow_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
    assert!(state
        .questions
        .iter()
        .all(|question| question.status == QuestionStatus::Bounded));
    assert!(events.iter().any(|event| matches!(
        event,
        InquiryEvent::QuestionDeferred { reason, .. }
            if reason.contains("follow-up retrieval wave 1")
    )));
    assert!(events.iter().any(
        |event| matches!(event, InquiryEvent::QuestionBounded { reason, .. }
            if reason.contains("equivalent normalized query"))
    ));
    session.close().await;
}
