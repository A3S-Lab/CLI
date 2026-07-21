use super::*;

use std::fs::OpenOptions;
use std::future::pending;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use a3s_code_core::llm::{
    LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
};
use a3s_code_core::tools::{Tool, ToolContext, ToolOutput};
use a3s_code_core::{Agent, SessionOptions};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::integration_tests::{evidence_output, inquiry_plan, workflow_args};

const PROCESS_ROLE_ENV: &str = "A3S_INQUIRY_PROCESS_ROLE";
const PROCESS_WORKSPACE_ENV: &str = "A3S_INQUIRY_PROCESS_WORKSPACE";
const PROCESS_RUN_ID_ENV: &str = "A3S_INQUIRY_PROCESS_RUN_ID";
const PAUSE_AFTER_STAGE_ENV: &str = "A3S_INQUIRY_PROCESS_PAUSE_AFTER_STAGE";
const EFFECT_COMPLETED_MARKER: &str = ".a3s/inquiry-process-effect-completed";
const FAIL_FIRST_PLANNER_MARKER: &str = "fail-first-planner";
static PROCESS_INVOCATION_LOG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scenario {
    Planner,
    Retrieval,
    Resolution,
}

impl Scenario {
    fn run_id(self) -> &'static str {
        match self {
            Self::Planner => "process-inquiry-planner",
            Self::Retrieval => "process-inquiry-retrieval",
            Self::Resolution => "process-inquiry-resolution",
        }
    }

    fn pause_stage(self) -> Option<&'static str> {
        match self {
            Self::Planner => Some("planner-retrieval"),
            Self::Retrieval => None,
            Self::Resolution => Some("question-review"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetrievalMode {
    Complete,
    BlockBeta,
}

struct ProcessRetrievalTool {
    workspace: PathBuf,
    mode: RetrievalMode,
}

impl ProcessRetrievalTool {
    fn record(&self, label: &str) -> anyhow::Result<()> {
        append_invocation(&self.workspace, &format!("retrieval:{label}"))
    }

    fn structured(label: &str) -> Value {
        serde_json::from_str::<Value>(&evidence_output(label))
            .expect("decode process evidence fixture")["structured"]
            .clone()
    }
}

#[async_trait::async_trait]
impl Tool for ProcessRetrievalTool {
    fn name(&self) -> &str {
        "inquiry_process_retrieval"
    }

    fn description(&self) -> &str {
        "Returns deterministic process-level collection evidence."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "label": { "type": "string", "enum": ["alpha", "beta"] }
            },
            "required": ["label"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> anyhow::Result<ToolOutput> {
        let label = args
            .get("label")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("process retrieval omitted label"))?;
        self.record(label)?;
        if self.mode == RetrievalMode::BlockBeta && label == "beta" {
            return pending::<anyhow::Result<ToolOutput>>().await;
        }
        Ok(ToolOutput::success(Self::structured(label).to_string()))
    }
}

struct ProcessInquiryClient {
    workspace: PathBuf,
}

impl ProcessInquiryClient {
    fn prompt(messages: &[Message]) -> String {
        messages
            .iter()
            .map(Message::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn classify(&self, messages: &[Message]) -> anyhow::Result<Value> {
        let prompt = Self::prompt(messages);
        if prompt.contains("Return the deterministic semantic inquiry plan.") {
            let prior_invocations = invocation_count(&self.workspace, "planner-semantic");
            append_invocation(&self.workspace, "planner-semantic")?;
            if self.workspace.join(FAIL_FIRST_PLANNER_MARKER).is_file() && prior_invocations == 0 {
                anyhow::bail!("synthetic transient planner failure");
            }
            return Ok(plan_fragment(
                inquiry_plan(),
                &[
                    "report_title",
                    "freshness_required",
                    "workspace_evidence_required",
                    "tracks",
                    "stop_conditions",
                ],
            ));
        }
        if prompt.contains("Return the deterministic retrieval portfolio.") {
            append_invocation(&self.workspace, "planner-retrieval")?;
            return Ok(retrieval_plan_fragment(inquiry_plan()));
        }
        if prompt.contains("CLOSED_QUESTION_EVIDENCE_PACKET=") {
            append_invocation(&self.workspace, "resolution")?;
            return Ok(json!({
                "resolutions": {
                    "question:plan-1-1": {
                        "status": "answered",
                        "content": "The retained process evidence establishes the material fixture finding.",
                        "limitation": "",
                        "evidence_refs": ["E1"],
                    }
                },
            }));
        }
        append_invocation(&self.workspace, "unexpected:model")?;
        anyhow::bail!("unexpected process inquiry generation prompt")
    }

    fn response(value: Value) -> LlmResponse {
        LlmResponse {
            message: Message::assistant(&value.to_string()),
            usage: TokenUsage::default(),
            stop_reason: Some("stop".to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }
}

fn plan_fragment(plan: Value, fields: &[&str]) -> Value {
    let plan = plan.as_object().expect("fixture plan object");
    Value::Object(
        fields
            .iter()
            .map(|field| {
                (
                    (*field).to_string(),
                    plan.get(*field)
                        .cloned()
                        .unwrap_or_else(|| panic!("fixture plan omitted {field}")),
                )
            })
            .collect(),
    )
}

fn retrieval_plan_fragment(plan: Value) -> Value {
    let mut fragment = plan_fragment(plan, &["search_queries", "seed_urls", "budget"]);
    let budget = fragment["budget"]
        .as_object_mut()
        .expect("fixture retrieval budget");
    let timeout_ms = budget
        .remove("retrieval_timeout_ms")
        .and_then(|value| value.as_u64())
        .expect("fixture retrieval timeout milliseconds");
    budget.insert(
        "retrieval_timeout_secs".to_string(),
        Value::from(timeout_ms / 1_000),
    );
    budget.insert("direct_fetches".to_string(), Value::from(8));
    fragment
}

#[async_trait::async_trait]
impl LlmClient for ProcessInquiryClient {
    async fn complete(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.classify(messages).map(Self::response)
    }

    async fn complete_streaming(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = Self::response(self.classify(messages)?);
        let text = response.message.text();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(StreamEvent::TextDelta(text)).await.ok();
            tx.send(StreamEvent::Done(response)).await.ok();
        });
        Ok(rx)
    }
}

struct IsolatedReviewInquiryClient {
    workspace: PathBuf,
}

impl IsolatedReviewInquiryClient {
    fn classify(&self, messages: &[Message]) -> anyhow::Result<Value> {
        let prompt = ProcessInquiryClient::prompt(messages);
        if prompt.contains("Return the isolated-review semantic plan.") {
            append_invocation(&self.workspace, "isolated:planner-semantic")?;
            return Ok(plan_fragment(
                isolated_review_plan(),
                &[
                    "report_title",
                    "freshness_required",
                    "workspace_evidence_required",
                    "tracks",
                    "stop_conditions",
                ],
            ));
        }
        if prompt.contains("Return the isolated-review retrieval portfolio.") {
            append_invocation(&self.workspace, "isolated:planner-retrieval")?;
            return Ok(retrieval_plan_fragment(isolated_review_plan()));
        }
        if prompt.contains("CLOSED_QUESTION_EVIDENCE_PACKET=") {
            let question_ids = [
                "question:plan-1-1",
                "question:plan-2-1",
                "question:plan-3-1",
            ];
            let matched = question_ids
                .iter()
                .filter(|question_id| prompt.contains(**question_id))
                .copied()
                .collect::<Vec<_>>();
            if matched.len() != 1 {
                append_invocation(&self.workspace, "isolated:unexpected-review-packet")?;
                anyhow::bail!(
                    "isolated review packet contained {} question identities",
                    matched.len()
                );
            }
            let question_id = matched[0];
            append_invocation(&self.workspace, &format!("isolated:review:{question_id}"))?;
            if question_id == "question:plan-2-1" {
                anyhow::bail!("synthetic provider failure for one isolated review unit");
            }
            let mut resolutions = serde_json::Map::new();
            let resolution = if question_id == "question:plan-3-1" {
                json!({
                    "status": "partial",
                    "content": "The retained evidence establishes the material beta finding.",
                    "limitation": "The closed packet does not establish one supporting beta detail.",
                    "evidence_refs": ["E1"],
                })
            } else {
                json!({
                    "status": "answered",
                    "content": format!(
                        "The retained evidence establishes the finding for {question_id}."
                    ),
                    "limitation": "",
                    "evidence_refs": ["E1"],
                })
            };
            resolutions.insert(question_id.to_string(), resolution);
            return Ok(json!({ "resolutions": resolutions }));
        }
        append_invocation(&self.workspace, "isolated:unexpected-model")?;
        anyhow::bail!("unexpected isolated-review generation prompt")
    }
}

#[async_trait::async_trait]
impl LlmClient for IsolatedReviewInquiryClient {
    async fn complete(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.classify(messages).map(ProcessInquiryClient::response)
    }

    async fn complete_streaming(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = ProcessInquiryClient::response(self.classify(messages)?);
        let text = response.message.text();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(StreamEvent::TextDelta(text)).await.ok();
            tx.send(StreamEvent::Done(response)).await.ok();
        });
        Ok(rx)
    }
}

fn append_invocation(workspace: &Path, label: &str) -> anyhow::Result<()> {
    let _guard = PROCESS_INVOCATION_LOG_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = workspace.join("process-invocations.log");
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(format!("{label}\n").as_bytes())?;
    file.flush()?;
    Ok(())
}

fn isolated_review_plan() -> Value {
    json!({
        "report_title": "Isolated question review fixture",
        "freshness_required": false,
        "workspace_evidence_required": false,
        "tracks": [{
            "id": "track:material-alpha",
            "title": "Material alpha",
            "focus": "Resolve the first material finding",
            "material": true,
            "questions": ["What does the retained alpha evidence establish?"],
            "completion_criteria": ["A traceable alpha answer"],
            "evidence_requirements": {
                "primary_source_required": false,
                "independent_corroboration_required": false
            }
        }, {
            "id": "track:supporting-gap",
            "title": "Supporting gap",
            "focus": "Resolve a non-material supporting finding",
            "material": false,
            "questions": ["What does the retained supporting evidence establish?"],
            "completion_criteria": ["A traceable supporting answer or an explicit bound"],
            "evidence_requirements": {
                "primary_source_required": false,
                "independent_corroboration_required": false
            }
        }, {
            "id": "track:material-beta",
            "title": "Material beta",
            "focus": "Resolve the second material finding",
            "material": true,
            "questions": ["What does the retained beta evidence establish?"],
            "completion_criteria": ["A traceable beta answer"],
            "evidence_requirements": {
                "primary_source_required": false,
                "independent_corroboration_required": false
            }
        }],
        "search_queries": ["fixture evidence"],
        "seed_urls": [],
        "budget": {
            "retrieval_timeout_ms": 30_000,
            "direct_searches": 1,
            "direct_fetches": 1
        },
        "stop_conditions": ["Both material findings have traceable evidence"]
    })
}

fn process_retrieval_source() -> String {
    r#"
async function run(ctx, inputs) {
  const labels = ["alpha", "beta"];
  if (inputs.kind === "workflow") {
    const outputs = inputs.step_outputs || {};
    const failures = inputs.step_failures || {};
    const failed = labels.find((label) => failures[label]);
    if (failed) {
      return { type: "fail", error: failures[failed].error || `retrieval ${failed} failed` };
    }
    const pending = labels.filter((label) => !outputs[label]);
    if (pending.length > 0) {
      return {
        type: "schedule_steps",
        steps: pending.map((label) => ({
          step_id: label,
          step_name: "retrieve",
          input: { label },
          retry: { max_attempts: 1, delay_ms: 0 },
        })),
      };
    }
    return {
      type: "complete",
      output: {
        query: "fixture inquiry",
        research: {
          status: "success",
          results: labels.map((label) => ({
            success: true,
            structured: outputs[label],
          })),
        },
      },
    };
  }
  if (inputs.kind === "step" && inputs.step_name === "retrieve") {
    const result = await ctx.tool("inquiry_process_retrieval", inputs.input);
    if (!result || Number(result.exitCode) !== 0) {
      throw new Error(result && result.output ? result.output : "process retrieval failed");
    }
    return JSON.parse(result.output);
  }
  return { error: "unknown process retrieval invocation" };
}
"#
    .to_string()
}

async fn process_session(workspace: &Path, retrieval_mode: RetrievalMode) -> (Agent, AgentSession) {
    let config = workspace.join("config.acl");
    if !config.exists() {
        std::fs::write(
            &config,
            "default_model = \"openai/x\"\n\
             providers \"openai\" {\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
             models \"x\" { name = \"x\" }\n}\n",
        )
        .expect("write process inquiry config");
    }
    let agent = Agent::new(config.to_string_lossy().to_string())
        .await
        .expect("process inquiry agent");
    let client = Arc::new(ProcessInquiryClient {
        workspace: workspace.to_path_buf(),
    });
    let options = SessionOptions::new()
        .with_session_id(format!("inquiry-process-worker-{}", std::process::id()))
        .with_llm_client(client)
        .with_auto_save(false)
        .with_tool_timeout(60_000);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(options))
        .await
        .expect("process inquiry session");
    session
        .register_dynamic_workflow_runtime()
        .expect("register process inquiry dynamic workflow");
    session
        .register_dynamic_tool(Arc::new(ProcessRetrievalTool {
            workspace: workspace.to_path_buf(),
            mode: retrieval_mode,
        }))
        .expect("register process retrieval tool");
    (agent, session)
}

async fn isolated_review_session(workspace: &Path) -> (Agent, AgentSession) {
    let config = workspace.join("config.acl");
    std::fs::write(
        &config,
        "default_model = \"openai/x\"\n\
         providers \"openai\" {\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
         models \"x\" { name = \"x\" }\n}\n",
    )
    .expect("write isolated review config");
    let agent = Agent::new(config.to_string_lossy().to_string())
        .await
        .expect("isolated review agent");
    let client = Arc::new(IsolatedReviewInquiryClient {
        workspace: workspace.to_path_buf(),
    });
    let options = SessionOptions::new()
        .with_session_id(format!("isolated-review-worker-{}", std::process::id()))
        .with_llm_client(client)
        .with_auto_save(false)
        .with_tool_timeout(60_000);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(options))
        .await
        .expect("isolated review session");
    session
        .register_dynamic_workflow_runtime()
        .expect("register isolated review dynamic workflow");
    session
        .register_dynamic_tool(Arc::new(ProcessRetrievalTool {
            workspace: workspace.to_path_buf(),
            mode: RetrievalMode::Complete,
        }))
        .expect("register isolated review retrieval tool");
    (agent, session)
}

async fn run_worker(workspace: &Path, run_id: &str, scenario: Scenario, role: &str) {
    let retrieval_mode = if role == "interrupt" && scenario == Scenario::Retrieval {
        RetrievalMode::BlockBeta
    } else {
        RetrievalMode::Complete
    };
    let (_agent, session) = process_session(workspace, retrieval_mode).await;
    let args = process_workflow_args(run_id);
    let (progress_tx, mut progress_rx) = mpsc::channel(64);
    tokio::spawn(async move { while progress_rx.recv().await.is_some() {} });
    let result = super::run_inquiry(Arc::new(session), args, progress_tx).await;
    if role == "interrupt" {
        panic!("interrupted inquiry worker returned before termination: {result:?}");
    }
    let result = result.expect("resumed process inquiry");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    let (events, state) =
        super::inquiry_projection_from_workflow(&result.output, result.metadata.as_ref())
            .expect("decode process inquiry projection")
            .expect("host process inquiry projection");
    assert_eq!(state.phase, InquiryPhase::Outlining);
    assert!(state.contract_assessment.is_some());
    std::fs::write(
        workspace.join("process-result.json"),
        serde_json::to_vec(&json!({ "events": events, "state": state }))
            .expect("encode process inquiry result"),
    )
    .expect("write process inquiry result");
}

fn process_workflow_args(run_id: &str) -> Value {
    let mut args = workflow_args();
    args["run_id"] = json!(run_id);
    args["source"] = json!(process_retrieval_source());
    let mut loop_contract = crate::tui::loop_engineering::deep_research_loop_contract(
        "fixture inquiry",
        "2026-07-19",
        "deterministic process evidence",
        1,
    );
    loop_contract["planner"]["semantic_prompt"] =
        json!("Return the deterministic semantic inquiry plan.");
    loop_contract["planner"]["retrieval_prompt"] =
        json!("Return the deterministic retrieval portfolio.");
    loop_contract["planner"]["semantic_timeout_ms"] = json!(30_000);
    loop_contract["planner"]["retrieval_timeout_ms"] = json!(30_000);
    args["input"]["loop_contract"] = loop_contract;
    args
}

fn exact_test_name(function: &str) -> String {
    let module = module_path!();
    let module = module.strip_prefix("a3s::").unwrap_or(module);
    format!("{module}::{function}")
}

fn spawn_worker(
    test_name: &str,
    workspace: &Path,
    role: &str,
    run_id: &str,
    pause_stage: Option<&str>,
) -> Child {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(PROCESS_ROLE_ENV, role)
        .env(PROCESS_WORKSPACE_ENV, workspace)
        .env(PROCESS_RUN_ID_ENV, run_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    if let Some(stage) = pause_stage {
        command.env(PAUSE_AFTER_STAGE_ENV, stage);
    }
    command.spawn().expect("spawn process inquiry worker")
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
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().expect("poll process inquiry worker") {
            assert!(status.success(), "{description} exited with {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{description} did not finish within 30 seconds");
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

fn flow_journals(workspace: &Path) -> Vec<PathBuf> {
    let mut paths = std::fs::read_dir(workspace.join(".a3s/workflow"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("jsonl"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn flow_journal_with_prefix(workspace: &Path, prefix: &str) -> Option<PathBuf> {
    flow_journals(workspace).into_iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix))
    })
}

fn invocation_count(workspace: &Path, label: &str) -> usize {
    std::fs::read_to_string(workspace.join("process-invocations.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == label)
        .count()
}

fn result_value(workspace: &Path) -> Value {
    serde_json::from_slice(
        &std::fs::read(workspace.join("process-result.json")).expect("read process inquiry result"),
    )
    .expect("decode process inquiry result")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_planner_failure_retries_the_same_durable_effect() {
    let workspace = tempfile::tempdir().expect("planner retry workspace");
    std::fs::write(workspace.path().join(FAIL_FIRST_PLANNER_MARKER), b"retry")
        .expect("write planner retry marker");
    let (_agent, session) = process_session(workspace.path(), RetrievalMode::Complete).await;
    let session = Arc::new(session);
    let run_id = "process-inquiry-planner-retry";
    let args = process_workflow_args(run_id);
    let (progress_tx, mut progress_rx) = mpsc::channel(64);
    tokio::spawn(async move { while progress_rx.recv().await.is_some() {} });

    let result = super::run_inquiry(Arc::clone(&session), args, progress_tx)
        .await
        .expect("the second planner attempt should complete the inquiry");

    assert_eq!(result.exit_code, 0, "{}", result.output);
    assert_eq!(invocation_count(workspace.path(), "planner-semantic"), 2);
    assert_eq!(invocation_count(workspace.path(), "planner-retrieval"), 1);
    let journal =
        flow_journal_with_prefix(workspace.path(), &format!("{run_id}-planner-semantic-"))
            .expect("one stable semantic-planner Flow journal");
    assert_eq!(event_count(&journal, "run_created", None), 1);
    assert_eq!(event_count(&journal, "step_started", Some("generation")), 2);
    assert_eq!(
        event_count(&journal, "step_retrying", Some("generation")),
        1
    );
    assert_eq!(
        event_count(&journal, "step_completed", Some("generation")),
        1
    );
    assert_eq!(event_count(&journal, "run_completed", None), 1);
    let retrieval =
        flow_journal_with_prefix(workspace.path(), &format!("{run_id}-planner-retrieval-"))
            .expect("one stable retrieval-planner Flow journal");
    assert_eq!(event_count(&retrieval, "run_created", None), 1);
    assert_eq!(
        event_count(&retrieval, "step_started", Some("generation")),
        1
    );
    assert_eq!(event_count(&retrieval, "run_completed", None), 1);
    session.close().await;
}

async fn assert_interrupted_prefix(workspace: &Path, run_id: &str, scenario: Scenario) {
    let restored = super::super::deep_research_state_journal::load_inquiry_state(workspace, run_id)
        .await
        .expect("load interrupted process inquiry");
    match scenario {
        Scenario::Planner => assert!(
            restored.is_none(),
            "planner effect completed before any Inquiry event was committed"
        ),
        Scenario::Retrieval | Scenario::Resolution => {
            let (events, state) = restored.expect("queued Inquiry prefix");
            assert_eq!(events.len(), 3);
            assert_eq!(state.phase, InquiryPhase::Questioning);
            assert_eq!(state.questions.len(), 1);
        }
    }
}

async fn run_process_resume_scenario(scenario: Scenario, function: &str) {
    if let Ok(role) = std::env::var(PROCESS_ROLE_ENV) {
        let workspace = PathBuf::from(
            std::env::var_os(PROCESS_WORKSPACE_ENV).expect("process inquiry workspace"),
        );
        let run_id = std::env::var(PROCESS_RUN_ID_ENV).expect("process inquiry run id");
        run_worker(&workspace, &run_id, scenario, &role).await;
        return;
    }

    let workspace = tempfile::tempdir().expect("process inquiry workspace");
    let baseline = tempfile::tempdir().expect("baseline process inquiry workspace");
    let test_name = exact_test_name(function);
    let run_id = scenario.run_id();
    let mut interrupted = spawn_worker(
        &test_name,
        workspace.path(),
        "interrupt",
        run_id,
        scenario.pause_stage(),
    );
    match scenario {
        Scenario::Planner => {
            let marker = workspace.path().join(EFFECT_COMPLETED_MARKER);
            wait_for_condition(
                "completed durable generation before Inquiry acknowledgement",
                Duration::from_secs(20),
                || marker.is_file(),
            )
            .await;
            for label in ["planner-semantic", "planner-retrieval"] {
                let journal =
                    flow_journal_with_prefix(workspace.path(), &format!("{run_id}-{label}-"))
                        .unwrap_or_else(|| panic!("durable {label} Flow journal"));
                assert_eq!(event_count(&journal, "run_completed", None), 1);
            }
        }
        Scenario::Resolution => {
            let marker = workspace.path().join(EFFECT_COMPLETED_MARKER);
            wait_for_condition(
                "completed durable generation before Inquiry acknowledgement",
                Duration::from_secs(20),
                || marker.is_file(),
            )
            .await;
            let prefix = format!(
                "{run_id}-{}-",
                scenario.pause_stage().expect("generation pause stage")
            );
            let journal = flow_journal_with_prefix(workspace.path(), &prefix)
                .expect("durable generation Flow journal");
            assert_eq!(event_count(&journal, "run_completed", None), 1);
        }
        Scenario::Retrieval => {
            let journal = workspace
                .path()
                .join(".a3s/workflow")
                .join(format!("{run_id}.jsonl"));
            wait_for_condition(
                "one completed and one running retrieval effect",
                Duration::from_secs(20),
                || {
                    event_count(&journal, "step_completed", Some("alpha")) == 1
                        && event_count(&journal, "step_started", Some("beta")) == 1
                        && invocation_count(workspace.path(), "retrieval:beta") == 1
                },
            )
            .await;
        }
    }
    interrupted
        .kill()
        .expect("forcefully interrupt process inquiry worker");
    let interrupted_status = interrupted
        .wait()
        .expect("reap interrupted process inquiry worker");
    assert!(!interrupted_status.success());
    assert_interrupted_prefix(workspace.path(), run_id, scenario).await;

    let mut resumed = spawn_worker(&test_name, workspace.path(), "resume", run_id, None);
    wait_for_success(&mut resumed, "resumed process inquiry worker").await;
    let baseline_run_id = format!("{run_id}-baseline");
    let mut uninterrupted = spawn_worker(
        &test_name,
        baseline.path(),
        "baseline",
        &baseline_run_id,
        None,
    );
    wait_for_success(&mut uninterrupted, "baseline process inquiry worker").await;

    assert_eq!(
        result_value(workspace.path()),
        result_value(baseline.path()),
        "resumed Inquiry projection must match uninterrupted execution"
    );
    assert_eq!(invocation_count(workspace.path(), "planner-semantic"), 1);
    assert_eq!(invocation_count(workspace.path(), "planner-retrieval"), 1);
    assert_eq!(invocation_count(workspace.path(), "resolution"), 1);
    assert_eq!(invocation_count(workspace.path(), "contract"), 0);
    assert_eq!(invocation_count(workspace.path(), "retrieval:alpha"), 1);
    assert_eq!(
        invocation_count(workspace.path(), "retrieval:beta"),
        if scenario == Scenario::Retrieval {
            2
        } else {
            1
        }
    );
    assert_eq!(invocation_count(workspace.path(), "unexpected:model"), 0);

    for journal in flow_journals(workspace.path()) {
        assert_eq!(
            event_count(&journal, "run_created", None),
            1,
            "stable Flow identity must not fork or conflict: {}",
            journal.display()
        );
    }
    if scenario == Scenario::Retrieval {
        let journal = workspace
            .path()
            .join(".a3s/workflow")
            .join(format!("{run_id}.jsonl"));
        assert_eq!(event_count(&journal, "step_started", Some("alpha")), 1);
        assert_eq!(event_count(&journal, "step_completed", Some("alpha")), 1);
        assert_eq!(
            event_count(&journal, "step_started", Some("beta")),
            1,
            "ambiguous retrieval redelivery reuses the interrupted attempt"
        );
        assert_eq!(event_count(&journal, "step_completed", Some("beta")), 1);
        assert_eq!(event_count(&journal, "run_completed", None), 1);
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_interruption_reuses_completed_planner_effect() {
    run_process_resume_scenario(
        Scenario::Planner,
        "process_interruption_reuses_completed_planner_effect",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_interruption_resumes_partial_retrieval_flow() {
    run_process_resume_scenario(
        Scenario::Retrieval,
        "process_interruption_resumes_partial_retrieval_flow",
    )
    .await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_interruption_reuses_resolution_before_checkpoint() {
    run_process_resume_scenario(
        Scenario::Resolution,
        "process_interruption_reuses_resolution_before_checkpoint",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_question_review_failure_is_isolated_and_durable_on_replay() {
    let workspace = tempfile::tempdir().expect("isolated review workspace");
    let (_agent, session) = isolated_review_session(workspace.path()).await;
    let session = Arc::new(session);
    let run_id = "isolated-question-review";
    let mut args = workflow_args();
    args["run_id"] = json!(run_id);
    args["source"] = json!(process_retrieval_source());
    let mut loop_contract = crate::tui::loop_engineering::deep_research_loop_contract(
        "fixture inquiry",
        "2026-07-19",
        "deterministic isolated review evidence",
        3,
    );
    loop_contract["planner"]["semantic_prompt"] =
        json!("Return the isolated-review semantic plan.");
    loop_contract["planner"]["retrieval_prompt"] =
        json!("Return the isolated-review retrieval portfolio.");
    loop_contract["planner"]["semantic_timeout_ms"] = json!(30_000);
    loop_contract["planner"]["retrieval_timeout_ms"] = json!(30_000);
    args["input"]["loop_contract"] = loop_contract;

    let mut projections = Vec::new();
    for _ in 0..2 {
        let (progress_tx, mut progress_rx) = mpsc::channel(128);
        tokio::spawn(async move { while progress_rx.recv().await.is_some() {} });
        let result = super::run_inquiry(Arc::clone(&session), args.clone(), progress_tx)
            .await
            .expect("isolated review inquiry");
        assert_eq!(result.exit_code, 0, "{}", result.output);
        projections.push(
            super::inquiry_projection_from_workflow(&result.output, result.metadata.as_ref())
                .expect("decode isolated review projection")
                .expect("host isolated review projection"),
        );
    }

    assert_eq!(
        projections[0], projections[1],
        "durable replay must reproduce the same Inquiry projection"
    );
    let (events, state) = &projections[0];
    assert_eq!(state.phase, InquiryPhase::Outlining);
    assert_eq!(
        a3s::research::research_contract_outcome(state),
        Some(a3s::research::ResearchContractOutcome::Qualified)
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, InquiryEvent::ResearchContractAssessed { .. }))
            .count(),
        1
    );
    assert_eq!(state.questions.len(), 3);
    assert_eq!(state.questions[0].status, QuestionStatus::Answered);
    assert!(!state.questions[0].evidence_ids.is_empty());
    assert_eq!(state.questions[1].status, QuestionStatus::Bounded);
    assert_eq!(
        state.questions[1].bound_reason.as_deref(),
        Some(
            "closed-evidence assessment did not establish a valid, traceable answer for this question"
        )
    );
    assert_eq!(state.questions[2].status, QuestionStatus::Answered);
    assert!(!state.questions[2].evidence_ids.is_empty());
    assert_eq!(
        state.questions[2].bound_reason.as_deref(),
        Some("The closed packet does not establish one supporting beta detail.")
    );
    assert!(events.iter().any(|event| matches!(
        event,
        InquiryEvent::QuestionPartiallyAnswered { question_id, .. }
            if question_id == "question:plan-3-1"
    )));

    assert_eq!(
        invocation_count(workspace.path(), "isolated:planner-semantic"),
        1
    );
    assert_eq!(
        invocation_count(workspace.path(), "isolated:planner-retrieval"),
        1
    );
    assert_eq!(
        invocation_count(workspace.path(), "isolated:review:question:plan-1-1"),
        1
    );
    assert_eq!(
        invocation_count(workspace.path(), "isolated:review:question:plan-2-1"),
        2,
        "a provider-level failure gets one bounded retry inside the same durable review unit"
    );
    assert_eq!(
        invocation_count(workspace.path(), "isolated:review:question:plan-3-1"),
        1
    );
    assert_eq!(invocation_count(workspace.path(), "retrieval:alpha"), 1);
    assert_eq!(invocation_count(workspace.path(), "retrieval:beta"), 1);
    assert_eq!(
        invocation_count(workspace.path(), "isolated:unexpected-review-packet"),
        0
    );
    assert_eq!(
        invocation_count(workspace.path(), "isolated:unexpected-model"),
        0
    );

    for ordinal in 1..=3 {
        let prefix = format!("{run_id}-question-review-{ordinal}-");
        let journal = flow_journal_with_prefix(workspace.path(), &prefix)
            .unwrap_or_else(|| panic!("missing isolated review journal `{prefix}`"));
        assert_eq!(event_count(&journal, "run_created", None), 1);
        if ordinal == 2 {
            assert_eq!(event_count(&journal, "step_started", Some("generation")), 2);
            assert_eq!(event_count(&journal, "run_completed", None), 0);
            assert_eq!(event_count(&journal, "run_failed", None), 1);
            assert!(event_count(&journal, "step_failed", Some("generation")) >= 1);
        } else {
            assert_eq!(event_count(&journal, "step_started", Some("generation")), 1);
            assert_eq!(event_count(&journal, "run_completed", None), 1);
            assert_eq!(event_count(&journal, "step_failed", Some("generation")), 0);
        }
    }
}
