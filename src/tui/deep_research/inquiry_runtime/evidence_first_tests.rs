use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use a3s_code_core::llm::{
    LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
};
use a3s_code_core::tools::{Tool, ToolContext, ToolOutput};
use a3s_code_core::{Agent, SessionOptions};
use tokio_util::sync::CancellationToken;

use super::*;

const FETCHED_SENTENCE: &str =
    "The official Nimbus record states that version 2 receives fixes through September 2027.";

struct EvidenceFirstSearch {
    return_source: bool,
}

#[async_trait::async_trait]
impl Tool for EvidenceFirstSearch {
    fn name(&self) -> &str {
        "evidence_first_fixture_search"
    }

    fn description(&self) -> &str {
        "Returns the evidence-first runtime fixture search catalog."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> anyhow::Result<ToolOutput> {
        let results = if self.return_source {
            serde_json::json!([{
                "title": "Official Nimbus support record",
                "url": "https://docs.rs/nimbus/latest/nimbus/support",
                "engines": ["fixture"]
            }])
        } else {
            serde_json::json!([])
        };
        Ok(ToolOutput::success(results.to_string()))
    }
}

struct EvidenceFirstFetch;

#[async_trait::async_trait]
impl Tool for EvidenceFirstFetch {
    fn name(&self) -> &str {
        "evidence_first_fixture_fetch"
    }

    fn description(&self) -> &str {
        "Returns one deterministic fetched source."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> anyhow::Result<ToolOutput> {
        anyhow::ensure!(
            args.get("url").and_then(Value::as_str)
                == Some("https://docs.rs/nimbus/latest/nimbus/support"),
            "unexpected evidence-first fixture URL"
        );
        Ok(ToolOutput::success(FETCHED_SENTENCE))
    }
}

#[derive(Clone, Copy)]
enum ProposalBehavior {
    Slow,
    Invalid,
    FailOnceThenValid,
}

struct EvidenceFirstProposal {
    behavior: ProposalBehavior,
    report_path: PathBuf,
    calls: Arc<AtomicUsize>,
    saw_staged_report: Arc<AtomicBool>,
}

impl EvidenceFirstProposal {
    async fn proposal(&self, messages: &[Message]) -> anyhow::Result<Value> {
        let prompt = messages
            .iter()
            .map(Message::text)
            .collect::<Vec<_>>()
            .join("\n");
        if prompt.contains("Create one bounded semantic retrieval plan") {
            return Ok(serde_json::json!({
                "report_title": "Nimbus support research",
                "research_scope": "focused",
                "freshness_required": false,
                "workspace_evidence_required": false,
                "tracks": [{
                    "id": "support.boundary",
                    "title": "Support boundary",
                    "focus": "Establish the supported Nimbus release and maintenance boundary.",
                    "material": true,
                    "completion_criteria": [
                        "A traceable source identifies the release and support boundary."
                    ],
                    "evidence_requirements": {
                        "primary_source_required": true,
                        "independent_corroboration_required": false
                    }
                }],
                "supplemental_queries": []
            }));
        }
        if prompt.contains("CLOSED_WEB_DISCOVERY_PACKET=") {
            return Ok(serde_json::json!({
                "candidate_ids": ["web-candidate-1"]
            }));
        }
        if let Some((_, packet)) = prompt.split_once("CLOSED_EVIDENCE_PACKET=") {
            let packet: Value = serde_json::from_str(packet)?;
            let source = packet["sources"]
                .as_array()
                .and_then(|sources| sources.first())
                .ok_or_else(|| anyhow::anyhow!("fixture evidence packet omitted source"))?;
            let source_id = source["source_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("fixture evidence source omitted ID"))?;
            let chunk_id = source["chunks"]
                .as_array()
                .and_then(|chunks| chunks.first())
                .and_then(|chunk| chunk["chunk_id"].as_str())
                .ok_or_else(|| anyhow::anyhow!("fixture evidence source omitted chunk"))?;
            let focus = packet["focuses"]
                .as_array()
                .and_then(|focuses| focuses.first())
                .ok_or_else(|| anyhow::anyhow!("fixture evidence packet omitted focus"))?;
            let obligation_id = focus["obligation_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("fixture focus omitted obligation ID"))?;
            let criterion_count = focus["completion_criteria"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(1);
            return Ok(serde_json::json!({
                "chunk_ids": [chunk_id],
                "source_coverage": [{
                    "source_id": source_id,
                    "obligation_id": obligation_id,
                    "completion_criterion_indexes": (0..criterion_count).collect::<Vec<_>>(),
                    "roles": {
                        "supporting": true,
                        "primary": focus["evidence_requirements"]["primary_source_required"] == true,
                        "independent": focus["evidence_requirements"]["independent_corroboration_required"] == true
                    }
                }],
                "source_relevance": [{
                    "source_id": source_id,
                    "obligation_id": obligation_id
                }]
            }));
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(
            prompt.contains("CLOSED_REPORT_PACKET="),
            "unexpected evidence-first proposal prompt"
        );
        let staged = std::fs::read_to_string(&self.report_path)?;
        anyhow::ensure!(
            staged.contains(FETCHED_SENTENCE),
            "the model proposal started before the source-backed report was staged"
        );
        self.saw_staged_report.store(true, Ordering::SeqCst);

        match self.behavior {
            ProposalBehavior::Slow => std::future::pending::<anyhow::Result<Value>>().await,
            ProposalBehavior::Invalid => Ok(serde_json::json!({
                "summary": [{
                    "text": "A fabricated source claims support through 2099.",
                    "source_aliases": ["source-99"],
                    "track_ids": ["support.boundary"]
                }],
                "findings": [],
                "recommendations": [],
                "limitations": []
            })),
            ProposalBehavior::FailOnceThenValid if self.calls.load(Ordering::SeqCst) == 1 => {
                anyhow::bail!("simulated transient streaming failure")
            }
            ProposalBehavior::FailOnceThenValid => Ok(serde_json::json!({
                "summary": [{
                    "text": "Nimbus version 2 receives fixes through September 2027.",
                    "source_aliases": ["source-1"],
                    "track_ids": ["support.boundary"]
                }],
                "findings": [{
                    "text": "The official Nimbus record identifies version 2 and September 2027 as the support boundary.",
                    "source_aliases": ["source-1"],
                    "track_ids": ["support.boundary"]
                }],
                "recommendations": [],
                "limitations": []
            })),
        }
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

#[async_trait::async_trait]
impl LlmClient for EvidenceFirstProposal {
    async fn complete(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.proposal(messages).await.map(Self::response)
    }

    async fn complete_streaming(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = Self::response(self.proposal(messages).await?);
        let text = response.message.text();
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            tx.send(StreamEvent::TextDelta(text)).await.ok();
            tx.send(StreamEvent::Done(response)).await.ok();
        });
        Ok(rx)
    }
}

struct UnexpectedProposal {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl LlmClient for UnexpectedProposal {
    async fn complete(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let prompt = messages
            .iter()
            .map(Message::text)
            .collect::<Vec<_>>()
            .join("\n");
        if prompt.contains("Create one bounded semantic retrieval plan") {
            return Ok(EvidenceFirstProposal::response(serde_json::json!({
                "report_title": "Nimbus evidence check",
                "research_scope": "focused",
                "freshness_required": false,
                "workspace_evidence_required": false,
                "tracks": [{
                    "id": "request.primary",
                    "title": "Requested evidence",
                    "focus": "Establish the requested answer.",
                    "material": true,
                    "completion_criteria": ["The answer is supported or explicitly bounded."],
                    "evidence_requirements": {
                        "primary_source_required": false,
                        "independent_corroboration_required": false
                    }
                }],
                "supplemental_queries": []
            })));
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("no-evidence publication must not invoke report generation")
    }

    async fn complete_streaming(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let prompt = messages
            .iter()
            .map(Message::text)
            .collect::<Vec<_>>()
            .join("\n");
        if prompt.contains("Create one bounded semantic retrieval plan") {
            let response = EvidenceFirstProposal::response(serde_json::json!({
                "report_title": "Nimbus evidence check",
                "research_scope": "focused",
                "freshness_required": false,
                "workspace_evidence_required": false,
                "tracks": [{
                    "id": "request.primary",
                    "title": "Requested evidence",
                    "focus": "Establish the requested answer.",
                    "material": true,
                    "completion_criteria": ["The answer is supported or explicitly bounded."],
                    "evidence_requirements": {
                        "primary_source_required": false,
                        "independent_corroboration_required": false
                    }
                }],
                "supplemental_queries": []
            }));
            let text = response.message.text();
            let (tx, rx) = mpsc::channel(4);
            tokio::spawn(async move {
                tx.send(StreamEvent::TextDelta(text)).await.ok();
                tx.send(StreamEvent::Done(response)).await.ok();
            });
            return Ok(rx);
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("no-evidence publication must not invoke report generation")
    }
}

#[tokio::test]
async fn proposal_timeout_preserves_the_already_staged_source_report() {
    let workspace = tempfile::tempdir().expect("create timeout workspace");
    let query = "Which Nimbus release is supported?";
    let calls = Arc::new(AtomicUsize::new(0));
    let saw_staged_report = Arc::new(AtomicBool::new(false));
    let report_path = report_markdown_path(workspace.path(), query);
    let (_agent, session) = fixture_session(
        workspace.path(),
        true,
        Arc::new(EvidenceFirstProposal {
            behavior: ProposalBehavior::Slow,
            report_path,
            calls: Arc::clone(&calls),
            saw_staged_report: Arc::clone(&saw_staged_report),
        }),
    )
    .await;
    let args = evidence_first_args(query, "evidence-first-proposal-timeout");
    record_workflow_started(
        workspace.path(),
        "evidence-first-proposal-timeout",
        deep_research_evidence_first_research_spec(&args),
    )
    .await
    .expect("pre-create the TUI-owned journal with the shared spec");

    let result = tokio::time::timeout(
        Duration::from_secs(6),
        execute_fixture_runtime(session, args, 1_200),
    )
    .await
    .expect("two bounded proposal attempts must cancel an indefinitely pending model call")
    .expect("timeout must fall back instead of failing the run");
    assert!(
        (1..=2).contains(&calls.load(Ordering::SeqCst)),
        "{}",
        result.output
    );
    assert!(saw_staged_report.load(Ordering::SeqCst));
    assert_source_backed_result(workspace.path(), query, &result);
}

#[tokio::test]
async fn transient_report_stream_failure_retries_and_publishes_a_real_report() {
    let workspace = tempfile::tempdir().expect("create retry workspace");
    let query = "Which Nimbus release is supported?";
    let calls = Arc::new(AtomicUsize::new(0));
    let saw_staged_report = Arc::new(AtomicBool::new(false));
    let report_path = report_markdown_path(workspace.path(), query);
    let (_agent, session) = fixture_session(
        workspace.path(),
        true,
        Arc::new(EvidenceFirstProposal {
            behavior: ProposalBehavior::FailOnceThenValid,
            report_path,
            calls: Arc::clone(&calls),
            saw_staged_report: Arc::clone(&saw_staged_report),
        }),
    )
    .await;
    let args = evidence_first_args(query, "evidence-first-proposal-retry");

    let result = execute_fixture_runtime(session, args, 5_000)
        .await
        .expect("transient report generation failure must be retried");

    assert_eq!(calls.load(Ordering::SeqCst), 2, "{}", result.output);
    assert!(saw_staged_report.load(Ordering::SeqCst));
    let output: Value = serde_json::from_str(&result.output).expect("decode retry output");
    assert_eq!(output["publication"]["status"], "synthesized");
    assert_eq!(output["research"]["status"], "success");
    assert_eq!(output["publication"]["quality"]["direct_answer_count"], 1);
    assert_eq!(output["publication"]["quality"]["finding_count"], 1);
    assert_eq!(output["publication"]["quality"]["accepted_claim_count"], 2);
    let markdown = std::fs::read_to_string(report_markdown_path(workspace.path(), query))
        .expect("read synthesized retry report");
    assert!(markdown.contains("## Direct Answer"), "{markdown}");
    assert!(markdown.contains("## Findings"), "{markdown}");
    assert!(
        !markdown.contains("Preserved Source Evidence"),
        "{markdown}"
    );
}

#[tokio::test]
async fn invalid_proposal_preserves_valid_fetched_evidence() {
    let workspace = tempfile::tempdir().expect("create invalid-proposal workspace");
    let query = "Which Nimbus release is supported?";
    let calls = Arc::new(AtomicUsize::new(0));
    let saw_staged_report = Arc::new(AtomicBool::new(false));
    let report_path = report_markdown_path(workspace.path(), query);
    let (_agent, session) = fixture_session(
        workspace.path(),
        true,
        Arc::new(EvidenceFirstProposal {
            behavior: ProposalBehavior::Invalid,
            report_path,
            calls: Arc::clone(&calls),
            saw_staged_report: Arc::clone(&saw_staged_report),
        }),
    )
    .await;
    let args = evidence_first_args(query, "evidence-first-invalid-proposal");

    let result = execute_fixture_runtime(session, args, 2_000)
        .await
        .expect("invalid proposal must fall back instead of failing the run");
    assert_eq!(calls.load(Ordering::SeqCst), 1, "{}", result.output);
    assert!(saw_staged_report.load(Ordering::SeqCst));
    assert_source_backed_result(workspace.path(), query, &result);
    let markdown = std::fs::read_to_string(report_markdown_path(workspace.path(), query))
        .expect("read retained source-backed Markdown");
    assert!(!markdown.contains("2099"));
    assert!(!markdown.contains("source-99"));
}

#[tokio::test]
async fn empty_acquisition_publishes_honest_artifacts_without_a_model_call() {
    let workspace = tempfile::tempdir().expect("create no-evidence runtime workspace");
    let query = "核查 Nimbus 当前支持策略";
    let calls = Arc::new(AtomicUsize::new(0));
    let (_agent, session) = fixture_session(
        workspace.path(),
        false,
        Arc::new(UnexpectedProposal {
            calls: Arc::clone(&calls),
        }),
    )
    .await;
    let args = evidence_first_args(query, "evidence-first-no-evidence");

    let result = execute_fixture_runtime(session, args, 1_000)
        .await
        .expect("empty acquisition must publish an honest terminal artifact");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        result.metadata.is_none(),
        "the Host result must not expose child workflow metadata"
    );
    let output: Value = serde_json::from_str(&result.output).expect("decode runtime output");
    assert_eq!(output["publication"]["status"], "no_evidence");
    assert_eq!(output["research"]["status"], "failed");
    let published =
        super::super::deep_research_artifacts::deep_research_evidence_first_published_report(
            workspace.path(),
            query,
            &result.output,
        )
        .expect("validate no-evidence publication")
        .expect("rediscover no-evidence artifacts");
    assert_eq!(
        published.publication,
        super::super::deep_research_artifacts::DeepResearchEvidenceFirstPublication::NoEvidence
    );
}

async fn fixture_session(
    workspace: &Path,
    return_source: bool,
    proposal: Arc<dyn LlmClient>,
) -> (Agent, AgentSession) {
    let config = workspace.join("config.acl");
    std::fs::write(
        &config,
        "default_model = \"openai/x\"\n\
         providers \"openai\" {\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
         models \"x\" { name = \"x\" }\n}\n",
    )
    .expect("write evidence-first fixture config");
    let agent = Agent::new(config.to_string_lossy().to_string())
        .await
        .expect("create evidence-first fixture agent");
    let options = SessionOptions::new()
        .with_session_id(format!(
            "evidence-first-fixture-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
        .with_llm_client(proposal)
        .with_auto_save(false)
        .with_tool_timeout(5_000);
    let session = agent
        .session_async(workspace.to_string_lossy().to_string(), Some(options))
        .await
        .expect("create evidence-first fixture session");
    session
        .register_dynamic_workflow_runtime()
        .expect("register dynamic workflow runtime");
    session
        .register_dynamic_tool(Arc::new(EvidenceFirstSearch { return_source }))
        .expect("register fixture search");
    session
        .register_dynamic_tool(Arc::new(EvidenceFirstFetch))
        .expect("register fixture fetch");
    (agent, session)
}

fn evidence_first_args(query: &str, run_id: &str) -> Value {
    let mut args = super::super::deep_research_workflow_args_with_scope(
        query,
        super::super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    let source = args["source"]
        .as_str()
        .expect("DeepResearch workflow source")
        .replace(
            "ctx.tool(\"web_search\"",
            "ctx.tool(\"evidence_first_fixture_search\"",
        )
        .replace(
            "tool: \"web_search\"",
            "tool: \"evidence_first_fixture_search\"",
        )
        .replace(
            "ctx.tool(\"web_fetch\"",
            "ctx.tool(\"evidence_first_fixture_fetch\"",
        )
        .replace(
            "tool: \"web_fetch\"",
            "tool: \"evidence_first_fixture_fetch\"",
        );
    args["source"] = Value::String(source);
    args["run_id"] = Value::String(run_id.to_string());
    args
}

async fn execute_fixture_runtime(
    session: AgentSession,
    args: Value,
    proposal_stage_timeout_ms: u64,
) -> Result<ToolCallResult, String> {
    let (progress_tx, mut progress_rx) = mpsc::channel(PROGRESS_CHANNEL_CAPACITY);
    let progress_drain = tokio::spawn(async move { while progress_rx.recv().await.is_some() {} });
    let result = run_evidence_first_research_with_limits(
        Arc::new(session),
        args,
        progress_tx,
        EvidenceFirstRuntimeLimits {
            bootstrap_stage_timeout_ms: 5_000,
            planned_retrieval_stage_timeout_ms: 5_000,
            report_proposal_attempt_timeout_ms: proposal_stage_timeout_ms
                .saturating_sub(200)
                .max(1_000),
            report_proposal_stage_timeout_ms: proposal_stage_timeout_ms,
        },
    )
    .await;
    progress_drain.await.expect("drain progress events");
    result
}

fn assert_source_backed_result(workspace: &Path, query: &str, result: &ToolCallResult) {
    assert!(
        result.metadata.is_none(),
        "the Host result must not expose child workflow metadata"
    );
    let output: Value = serde_json::from_str(&result.output).expect("decode runtime output");
    assert_eq!(output["publication"]["status"], "source_backed");
    assert_eq!(output["research"]["status"], "degraded");
    assert!(output["research"]["warnings"]["report_error"].is_string());
    let published =
        super::super::deep_research_artifacts::deep_research_evidence_first_published_report(
            workspace,
            query,
            &result.output,
        )
        .expect("validate source-backed publication")
        .expect("rediscover source-backed artifacts");
    assert_eq!(
        published.publication,
        super::super::deep_research_artifacts::DeepResearchEvidenceFirstPublication::SourceBacked
    );
    let markdown =
        std::fs::read_to_string(published.artifacts.markdown).expect("read source-backed Markdown");
    assert!(markdown.contains(FETCHED_SENTENCE));
}

fn report_markdown_path(workspace: &Path, query: &str) -> PathBuf {
    workspace
        .join(".a3s/research")
        .join(super::super::deep_research_artifacts::deep_research_report_slug(query))
        .join("report.md")
}
