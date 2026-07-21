use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use a3s_code_core::tools::{Tool, ToolContext, ToolErrorKind, ToolExecutor, ToolOutput};

struct SearchFixture {
    queries: Arc<Mutex<Vec<String>>>,
    results: serde_json::Value,
}

struct QuerySearchFixture {
    queries: Arc<Mutex<Vec<String>>>,
    results_by_query: BTreeMap<String, serde_json::Value>,
}

struct TextFetchFixture {
    urls: Arc<Mutex<Vec<String>>>,
    bodies: BTreeMap<String, String>,
}

struct SemanticSelectorFixture {
    preferred_fragments: Vec<String>,
    fail: bool,
    invalid_selection: bool,
}

struct RetryOnceSemanticSelectorFixture {
    calls: Arc<AtomicUsize>,
    selector: SemanticSelectorFixture,
}

struct FailMatchingSemanticSelectorFixture {
    schema_name: &'static str,
    fragment: String,
    selector: SemanticSelectorFixture,
}

struct PaginatedPdfFixture {
    offsets: Arc<Mutex<Vec<u64>>>,
}

struct TransientFetchFixture {
    calls: Arc<AtomicUsize>,
}

struct UntypedFetchFailureFixture {
    calls: Arc<AtomicUsize>,
}

struct LocalEvidenceFixture;

#[async_trait::async_trait]
impl Tool for SearchFixture {
    fn name(&self) -> &str {
        "fixture_web_search"
    }

    fn description(&self) -> &str {
        "Returns deterministic search candidates."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        assert_eq!(
            args.get("engines"),
            Some(&serde_json::json!(["anysearch", "tavily", "ddg"])),
            "DeepResearch must use its fixed non-Wikipedia search ensemble"
        );
        self.queries.lock().unwrap().push(
            args.get("query")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        );
        Ok(ToolOutput::success(self.results.to_string()))
    }
}

#[async_trait::async_trait]
impl Tool for QuerySearchFixture {
    fn name(&self) -> &str {
        "fixture_query_web_search"
    }

    fn description(&self) -> &str {
        "Returns deterministic candidates for each exact provider query."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        assert_eq!(
            args.get("engines"),
            Some(&serde_json::json!(["anysearch", "tavily", "ddg"])),
            "DeepResearch must use its fixed non-Wikipedia search ensemble"
        );
        let query = args
            .get("query")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.queries.lock().unwrap().push(query.clone());
        let results = self
            .results_by_query
            .get(&query)
            .ok_or_else(|| anyhow::anyhow!("unexpected fixture query: {query}"))?;
        Ok(ToolOutput::success(results.to_string()))
    }
}

#[async_trait::async_trait]
impl Tool for TextFetchFixture {
    fn name(&self) -> &str {
        "fixture_web_fetch"
    }

    fn description(&self) -> &str {
        "Returns deterministic fetched source text."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let url = args
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.urls.lock().unwrap().push(url.clone());
        let body = self
            .bodies
            .get(&url)
            .ok_or_else(|| anyhow::anyhow!("unexpected fixture URL: {url}"))?;
        Ok(ToolOutput::success(body.clone()))
    }
}

#[async_trait::async_trait]
impl Tool for SemanticSelectorFixture {
    fn name(&self) -> &str {
        "generate_object"
    }

    fn description(&self) -> &str {
        "Selects semantic chunk IDs from the closed evidence packet."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let schema_name = args.get("schema_name").and_then(serde_json::Value::as_str);
        if matches!(
            schema_name,
            Some(
                "deep_research_web_source_selection"
                    | "deep_research_supplemental_web_source_selection"
            )
        ) {
            let prompt = args
                .get("prompt")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let marker = if schema_name == Some("deep_research_supplemental_web_source_selection") {
                "CLOSED_SUPPLEMENTAL_DISCOVERY_PACKET="
            } else {
                "CLOSED_WEB_DISCOVERY_PACKET="
            };
            let packet = prompt
                .split_once(marker)
                .map(|(_, packet)| packet)
                .ok_or_else(|| {
                    anyhow::anyhow!("web source selector omitted its closed discovery packet")
                })?;
            let packet: serde_json::Value = serde_json::from_str(packet)?;
            let candidates = packet["candidates"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("web source selector packet omitted candidates"))?;
            let maximum = args["schema"]["properties"]["candidate_ids"]["maxItems"]
                .as_u64()
                .unwrap_or(candidates.len() as u64) as usize;
            let mut candidate_ids = Vec::new();
            for preferred in &self.preferred_fragments {
                if let Some(candidate) = candidates.iter().find(|candidate| {
                    ["title", "url", "content"].iter().any(|field| {
                        candidate[*field]
                            .as_str()
                            .is_some_and(|text| text.contains(preferred))
                    })
                }) {
                    let candidate_id = candidate["candidate_id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("candidate omitted ID"))?;
                    if !candidate_ids.iter().any(|seen| seen == candidate_id) {
                        candidate_ids.push(candidate_id.to_string());
                    }
                }
            }
            if candidate_ids.is_empty() {
                candidate_ids.extend(
                    candidates
                        .iter()
                        .take(maximum)
                        .filter_map(|candidate| candidate["candidate_id"].as_str())
                        .map(str::to_string),
                );
            }
            candidate_ids.truncate(maximum);
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "object": { "candidate_ids": candidate_ids },
                    "repair_rounds": 0,
                    "mode_used": "fixture"
                })
                .to_string(),
            ));
        }
        if self.fail {
            return Ok(ToolOutput::error("simulated semantic selector failure"));
        }
        if self.invalid_selection {
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "object": {
                        "chunk_ids": ["source-1:chunk:not-in-catalog"],
                        "source_coverage": [],
                        "source_relevance": []
                    },
                    "repair_rounds": 0,
                    "mode_used": "fixture"
                })
                .to_string(),
            ));
        }
        let prompt = args
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let packet = prompt
            .split_once("CLOSED_EVIDENCE_PACKET=")
            .map(|(_, packet)| packet)
            .ok_or_else(|| anyhow::anyhow!("selector omitted its closed evidence packet"))?;
        let packet: serde_json::Value = serde_json::from_str(packet)?;
        let focuses = packet["focuses"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("selector packet omitted focuses"))?;
        for focus in focuses {
            assert!(
                focus["obligation_id"].is_string(),
                "selector focus omitted its stable obligation identity"
            );
            assert!(
                focus["material"].is_boolean(),
                "selector focus omitted its materiality"
            );
            assert!(
                focus["completion_criteria"].is_array(),
                "selector focus omitted its completion criteria"
            );
            assert!(
                focus["evidence_requirements"].is_object(),
                "selector focus omitted its source-quality requirements"
            );
        }
        let sources = packet["sources"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("selector packet omitted sources"))?;
        let mut chunk_ids = Vec::new();
        for source in sources {
            let chunks = source["chunks"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("selector source omitted chunks"))?;
            let mut retained_for_source = Vec::new();
            for preferred in &self.preferred_fragments {
                if let Some(chunk) = chunks.iter().find(|chunk| {
                    chunk["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(preferred))
                }) {
                    let chunk_id = chunk["chunk_id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("selector chunk omitted ID"))?;
                    if !retained_for_source.iter().any(|seen| seen == chunk_id) {
                        retained_for_source.push(chunk_id.to_string());
                    }
                }
            }
            if retained_for_source.is_empty() {
                let chunk_id = chunks
                    .first()
                    .and_then(|chunk| chunk["chunk_id"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("selector source omitted chunks"))?;
                retained_for_source.push(chunk_id.to_string());
            }
            chunk_ids.extend(retained_for_source);
        }
        let maximum = args["schema"]["properties"]["chunk_ids"]["maxItems"]
            .as_u64()
            .unwrap_or(chunk_ids.len() as u64) as usize;
        chunk_ids.truncate(maximum);
        let selected_chunk_ids = chunk_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        let mut source_coverage = Vec::new();
        let mut source_relevance = Vec::new();
        for source in sources {
            let selected = source["chunks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|chunk| chunk["chunk_id"].as_str())
                .any(|chunk_id| selected_chunk_ids.contains(chunk_id));
            if !selected {
                continue;
            }
            let source_id = source["source_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("selector source omitted ID"))?;
            for focus in focuses {
                source_relevance.push(serde_json::json!({
                    "source_id": source_id,
                    "obligation_id": focus["obligation_id"],
                }));
                let completion_criterion_count = focus["completion_criteria"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or_default();
                let roles = serde_json::json!({
                    "supporting": true,
                    "primary":
                        focus["evidence_requirements"]["primary_source_required"] == true,
                    "independent":
                        focus["evidence_requirements"]["independent_corroboration_required"]
                            == true,
                });
                source_coverage.push(serde_json::json!({
                    "source_id": source_id,
                    "obligation_id": focus["obligation_id"],
                    "completion_criterion_indexes":
                        (0..completion_criterion_count).collect::<Vec<_>>(),
                    "roles": roles,
                }));
            }
        }
        assert!(
            !focuses.is_empty(),
            "selector packet omitted semantic focuses"
        );
        Ok(ToolOutput::success(
            serde_json::json!({
                "object": {
                    "chunk_ids": chunk_ids,
                    "source_coverage": source_coverage,
                    "source_relevance": source_relevance
                },
                "repair_rounds": 0,
                "mode_used": "fixture"
            })
            .to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl Tool for RetryOnceSemanticSelectorFixture {
    fn name(&self) -> &str {
        "generate_object"
    }

    fn description(&self) -> &str {
        "Fails once before selecting semantic chunk IDs from the closed evidence packet."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(ToolOutput::error(
                "simulated transient semantic selector failure",
            ));
        }
        self.selector.execute(args, ctx).await
    }
}

#[async_trait::async_trait]
impl Tool for FailMatchingSemanticSelectorFixture {
    fn name(&self) -> &str {
        "generate_object"
    }

    fn description(&self) -> &str {
        "Fails only semantic selector packets containing one exact fixture fragment."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let prompt = args
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if args["schema_name"] == self.schema_name && prompt.contains(&self.fragment) {
            return Ok(ToolOutput::error("simulated source-local shard timeout"));
        }
        self.selector.execute(args, ctx).await
    }
}

#[async_trait::async_trait]
impl Tool for PaginatedPdfFixture {
    fn name(&self) -> &str {
        "fixture_pdf_fetch"
    }

    fn description(&self) -> &str {
        "Returns three deterministic extracted PDF ranges from one admitted source."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let offset = args
            .get("offset")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        self.offsets.lock().unwrap().push(offset);
        let (body, next_offset) = match offset {
            0 => (
                "第一段记录了双阶段检索方法以及证据保留边界，内容足够构成一个结构化文本块。",
                Some(100),
            ),
            100 => (
                "第二段报告消融实验使引用完整率下降十七个百分点，并给出明确测量条件。",
                Some(200),
            ),
            200 => (
                "第三段说明评测只覆盖英语技术主题，因此其他语言和领域仍然属于证据缺口。",
                None,
            ),
            _ => return Ok(ToolOutput::error("unexpected PDF range offset")),
        };
        Ok(ToolOutput::success(body).with_metadata(serde_json::json!({
            "document_kind": "pdf",
            "content_type": "application/pdf",
            "range": {
                "offset": offset,
                "next_offset": next_offset,
                "eof": next_offset.is_none()
            }
        })))
    }
}

#[async_trait::async_trait]
impl Tool for TransientFetchFixture {
    fn name(&self) -> &str {
        "fixture_transient_fetch"
    }

    fn description(&self) -> &str {
        "Fails each initial fetch and succeeds on the bounded retry."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < 2 {
            return Ok(
                ToolOutput::error("typed transport timeout").with_error_kind(
                    ToolErrorKind::Timeout {
                        op: "fixture fetch".to_string(),
                        duration_ms: 20_000,
                    },
                ),
            );
        }
        let url = args
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Ok(ToolOutput::success(format!(
            "The bounded retry fetched substantive authoritative evidence from {url}."
        )))
    }
}

#[async_trait::async_trait]
impl Tool for UntypedFetchFailureFixture {
    fn name(&self) -> &str {
        "fixture_untyped_fetch_failure"
    }

    fn description(&self) -> &str {
        "Returns transport-like prose without a typed error classification."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutput::error("TLS handshake timed out during lookup"))
    }
}

#[async_trait::async_trait]
impl Tool for LocalEvidenceFixture {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Returns observed and fabricated local source paths."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        Ok(
            ToolOutput::success("local evidence").with_metadata(serde_json::json!({
                "success": true,
                "source_anchors": [{
                    "tool": "read",
                    "url_or_path": "src/research.rs"
                }, {
                    "tool": "ls",
                    "url_or_path": "src/listed-only.rs"
                }],
                "structured": {
                    "sources": [{
                        "url_or_path": "src/research.rs",
                        "ranges": [{"offset": 0, "limit": 20}]
                    }, {
                        "url_or_path": "src/fabricated.rs",
                        "ranges": [{"offset": 0, "limit": 20}]
                    }, {
                        "url_or_path": "src/listed-only.rs",
                        "ranges": [{"offset": 0, "limit": 20}]
                    }]
                }
            })),
        )
    }
}

fn minimal_plan(
    tracks: serde_json::Value,
    search_queries: serde_json::Value,
    seed_urls: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "report_title": "Retrieval fixture",
        "freshness_required": false,
        "workspace_evidence_required": false,
        "tracks": tracks,
        "search_queries": search_queries,
        "seed_urls": seed_urls,
        "budget": {
            "retrieval_timeout_ms": 30_000,
            "direct_searches": 4,
            "direct_fetches": 8
        },
        "stop_conditions": ["Retain traceable evidence or a bounded gap."]
    })
}

fn track(id: &str, title: &str, focus: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": title,
        "focus": focus,
        "material": true,
        "questions": [focus],
        "completion_criteria": ["The focus has traceable evidence or a bounded gap."],
        "evidence_requirements": {
            "primary_source_required": false,
            "independent_corroboration_required": false
        }
    })
}

fn replace_web_tools(source: &str, search: &str, fetch: &str) -> String {
    source
        .replace("ctx.tool(\"web_search\"", &format!("ctx.tool(\"{search}\""))
        .replace("ctx.tool(\"web_fetch\"", &format!("ctx.tool(\"{fetch}\""))
        .replace("tool: \"web_search\"", &format!("tool: \"{search}\""))
        .replace("tool: \"web_fetch\"", &format!("tool: \"{fetch}\""))
}

fn workflow_args(
    query: &str,
    scope: super::DeepResearchEvidenceScope,
    plan: serde_json::Value,
    search: &str,
    fetch: &str,
) -> serde_json::Value {
    let mut args = super::deep_research_workflow_args_with_scope(query, scope);
    args["input"]["research_plan"] = plan;
    args["input"]["execution_mode"] = serde_json::json!("collect_only");
    args["limits"]["timeoutMs"] = serde_json::json!(45_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(24);
    if scope == super::DeepResearchEvidenceScope::WebAndWorkspace {
        args["source"] = serde_json::Value::String(replace_web_tools(
            args["source"].as_str().expect("workflow source"),
            search,
            fetch,
        ));
    }
    args
}

async fn execute(executor: &ToolExecutor, args: &serde_json::Value) -> serde_json::Value {
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());
    let result = executor
        .execute("dynamic_workflow", args)
        .await
        .expect("retrieval workflow execution");
    assert_eq!(result.exit_code, 0, "{}", result.output);
    serde_json::from_str(&result.output).expect("retrieval output")
}

#[tokio::test]
async fn bootstrap_acquisition_persists_raw_sources_without_model_admission() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let queries = Arc::new(Mutex::new(Vec::new()));
    let urls = Arc::new(Mutex::new(Vec::new()));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::clone(&queries),
        results: serde_json::json!([{
            "title": "First ranked source",
            "url": "https://bootstrap.example/first",
            "engines": ["fixture"]
        }, {
            "title": "Second ranked source",
            "url": "https://bootstrap.example/second",
            "engines": ["fixture"]
        }, {
            "title": "Unspent candidate",
            "url": "https://bootstrap.example/third",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([
            (
                "https://bootstrap.example/first".to_string(),
                "The first fetched source contains substantive traceable bootstrap evidence."
                    .to_string(),
            ),
            (
                "https://bootstrap.example/second".to_string(),
                "The second fetched source contains separate substantive bootstrap evidence."
                    .to_string(),
            ),
        ]),
    }));
    let query = "Acquire evidence before semantic planning";
    let mut plan = minimal_plan(
        serde_json::json!([track("request.primary", "Original request", query)]),
        serde_json::json!([query]),
        serde_json::json!([]),
    );
    plan["budget"]["direct_searches"] = serde_json::json!(1);
    plan["budget"]["direct_fetches"] = serde_json::json!(2);
    let mut args = workflow_args(
        query,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );
    args["input"]["execution_mode"] = serde_json::json!("bootstrap_acquisition");
    args["run_id"] = serde_json::json!("deepresearch-bootstrap-acquisition-test");

    // No generate_object fixture is registered. A successful run therefore
    // proves that bootstrap acquisition never waits for model admission.
    let output = execute(&executor, &args).await;

    assert_eq!(*queries.lock().unwrap(), [query]);
    assert_eq!(
        *urls.lock().unwrap(),
        [
            "https://bootstrap.example/first",
            "https://bootstrap.example/second"
        ]
    );
    assert_eq!(output["mode"], "bootstrap_acquisition");
    assert_eq!(
        output["acquisition"]["packet"]["sources"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        output["acquisition"]["metadata"]["source_selection_mode"],
        "provider_round_robin"
    );
    let history = std::fs::read_to_string(
        workspace
            .path()
            .join(".a3s/workflow/deepresearch-bootstrap-acquisition-test.jsonl"),
    )
    .expect("durable bootstrap history");
    assert!(history.lines().any(|line| {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        event["event"]["type"] == "step_completed"
            && event["event"]["step_id"] == "checkpoint_bootstrap_acquisition"
    }));
}

#[tokio::test]
async fn semantic_retrieval_reuses_bootstrap_packet_without_repeating_transport() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec!["preserved raw evidence".to_string()],
        fail: false,
        invalid_selection: false,
    }));
    let query = "Reuse already fetched evidence";
    let plan = minimal_plan(
        serde_json::json!([track("request.primary", "Original request", query)]),
        serde_json::json!([query]),
        serde_json::json!([]),
    );
    let mut args = super::deep_research_workflow_args_with_scope(
        query,
        super::DeepResearchEvidenceScope::WebAndWorkspace,
    );
    args["input"]["research_plan"] = plan;
    args["input"]["execution_mode"] = serde_json::json!("collect_only");
    args["input"]["bootstrap_acquisition"] = serde_json::json!({
        "status": "success",
        "packet": {
            "version": 1,
            "focuses": [],
            "sources": [{
                "source_id": "bootstrap-web-source-1",
                "title": "Preserved source",
                "url_or_path": "https://bootstrap.example/preserved",
                "reliability": "Fetched and durably preserved before planning.",
                "chunks": [{
                    "chunk_id": "bootstrap-web-source-1:chunk:1",
                    "text": "This preserved raw evidence remains available after planning settles."
                }]
            }]
        },
        "errors": [],
        "metadata": {
            "source_selection_mode": "provider_round_robin",
            "fetched_count": 1
        }
    });
    args["run_id"] = serde_json::json!("deepresearch-bootstrap-reuse-test");
    args["limits"]["timeoutMs"] = serde_json::json!(45_000);
    args["limits"]["maxToolCalls"] = serde_json::json!(24);

    // No search or fetch fixture is registered. The final retrieval can only
    // succeed by consuming the immutable bootstrap packet.
    let output = execute(&executor, &args).await;

    assert_eq!(output["mode"], "inquiry_collection");
    assert_eq!(output["research"]["metadata"]["bootstrap_source_count"], 1);
    assert_eq!(
        output["research"]["results"][0]["structured"]["sources"][0]["url_or_path"],
        "https://bootstrap.example/preserved"
    );
}

#[tokio::test]
async fn provider_query_and_cross_language_semantic_selection_are_preserved() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let queries = Arc::new(Mutex::new(Vec::new()));
    let urls = Arc::new(Mutex::new(Vec::new()));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::clone(&queries),
        results: serde_json::json!([{
            "title": "運用監査記録",
            "url": "https://primary.example/record",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls,
        bodies: BTreeMap::from([(
            "https://primary.example/record".to_string(),
            "監査ログはサービスが正常に稼働していること、観測時刻、監査範囲、証拠境界を記録している。".to_string(),
        )]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec!["正常に稼働".to_string()],
        fail: false,
        invalid_selection: false,
    }));
    let provider_query = "MiXeD Case?!  日本語／中文 — café №42";
    let plan = minimal_plan(
        serde_json::json!([track("operating.state", "运行状态", "核实服务是否正常运行")]),
        serde_json::json!([provider_query]),
        serde_json::json!([]),
    );
    let args = workflow_args(
        "Assess the operating condition",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(*queries.lock().unwrap(), [provider_query]);
    assert_eq!(
        output["research"]["metadata"]["evidence_selection_mode"],
        "semantic_chunk_ids_with_typed_coverage"
    );
    assert!(
        output["research"]["results"][0]["structured"]["sources"][0]["quote_or_fact"]
            .as_str()
            .is_some_and(|text| text.contains("正常に稼働"))
    );
}

#[tokio::test]
async fn provider_publication_date_remains_discovery_metadata() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Provider-dated record",
            "url": "https://dates.example/record",
            "published_date": "2099-12-31",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::new(Mutex::new(Vec::new())),
        bodies: BTreeMap::from([(
            "https://dates.example/record".to_string(),
            "The fetched record establishes the requested operational fact without publishing a date."
                .to_string(),
        )]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec!["requested operational fact".to_string()],
        fail: false,
        invalid_selection: false,
    }));
    let plan = minimal_plan(
        serde_json::json!([track(
            "date.provenance",
            "Date provenance",
            "Retain only dates established by fetched evidence"
        )]),
        serde_json::json!(["provider date provenance"]),
        serde_json::json!([]),
    );
    let args = workflow_args(
        "Do not treat provider metadata as publication evidence",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    let source = &output["research"]["results"][0]["structured"]["sources"][0];
    assert_eq!(source["url_or_path"], "https://dates.example/record");
    assert!(
        source.get("date").is_none() || source["date"].is_null(),
        "provider-supplied dates must not cross the fetched-evidence boundary: {source}"
    );
    assert!(
        !output.to_string().contains("2099-12-31"),
        "unverified discovery dates must not survive evidence materialization"
    );
}

#[tokio::test]
async fn github_release_catalog_fetches_the_official_atom_feed() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let urls = Arc::new(Mutex::new(Vec::new()));
    let atom_feed = format!(
        "<?xml version=\"1.0\"?><feed><title>Release notes</title><subtitle>{}</subtitle><entry><id>tag:github.com,2008:Repository/1/v1.13.2</id><updated>2025-08-15T01:43:57Z</updated><link href=\"https://github.com/example/runtime/releases/tag/v1.13.2\"/><title>v1.13.2</title><content>Latest bounded official release notes.</content></entry><entry><id>tag:github.com,2008:Repository/1/v1.13.1</id><updated>2025-03-15T22:05:29Z</updated><title>v1.13.1</title></entry></feed>",
        "feed metadata ".repeat(80)
    );
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([(
            "https://github.com/example/runtime/releases.atom".to_string(),
            atom_feed,
        )]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec!["v1.13.2".to_string()],
        fail: false,
        invalid_selection: false,
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "release.feed",
            "Release feed",
            "Verify the latest official release record"
        )]),
        serde_json::json!([]),
        serde_json::json!(["https://github.com/example/runtime/releases"]),
    );
    plan["budget"]["direct_searches"] = serde_json::json!(0);
    plan["budget"]["direct_fetches"] = serde_json::json!(1);
    let args = workflow_args(
        "Read a bounded official GitHub release catalog",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "unused_fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(
        *urls.lock().unwrap(),
        ["https://github.com/example/runtime/releases.atom"]
    );
    assert_eq!(
        output["research"]["results"][0]["structured"]["sources"][0]["url_or_path"],
        "https://github.com/example/runtime/releases"
    );
    let fact = output["research"]["results"][0]["structured"]["sources"][0]["quote_or_fact"]
        .as_str()
        .expect("selected release fact");
    assert!(fact.contains("v1.13.2"), "{fact}");
    assert!(fact.contains("2025-08-15T01:43:57Z"), "{fact}");
}

#[tokio::test]
async fn truncated_batch_child_is_refetched_without_losing_later_sources() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let urls = Arc::new(Mutex::new(Vec::new()));
    let source_urls = [
        "https://batch.example/one",
        "https://batch.example/two",
        "https://batch.example/three",
    ];
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: source_urls
            .iter()
            .enumerate()
            .map(|(index, url)| {
                (
                    (*url).to_string(),
                    format!(
                        "Authoritative evidence source {}. {}",
                        index + 1,
                        "bounded evidence text ".repeat(1_550)
                    ),
                )
            })
            .collect(),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: Vec::new(),
        fail: false,
        invalid_selection: false,
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "batch.recovery",
            "Batch recovery",
            "Retain every fetched source after aggregate output truncation"
        )]),
        serde_json::json!([]),
        serde_json::json!(source_urls),
    );
    plan["budget"]["direct_searches"] = serde_json::json!(0);
    plan["budget"]["direct_fetches"] = serde_json::json!(3);
    let args = workflow_args(
        "Recover a truncated child from a large fetch batch",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "unused_fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(output["research"]["metadata"]["source_count"], 3);
    assert!(
        output["research"]["metadata"]["web"]["batch_output_recovery_count"]
            .as_u64()
            .is_some_and(|count| count >= 1),
        "large batch output should exercise the per-child recovery path: {}",
        output["research"]["metadata"]
    );
    let observed = urls.lock().unwrap();
    assert!(
        source_urls
            .iter()
            .all(|url| observed.iter().any(|seen| seen == url)),
        "every selected source must be fetched: {observed:?}"
    );
    assert!(
        observed.len() > source_urls.len(),
        "at least one truncated batch child must be refetched independently"
    );
}

#[tokio::test]
async fn oversubscribed_provider_catalog_is_semantically_admitted_before_fetch() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let urls = Arc::new(Mutex::new(Vec::new()));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Unrelated first provider result",
            "url": "https://selection.example/unrelated-first",
            "content": "A valid page that does not address the planned research focus.",
            "engines": ["fixture"]
        }, {
            "title": "另一个无关结果",
            "url": "https://selection.example/unrelated-second",
            "content": "Este resultado tampoco responde a la pregunta.",
            "engines": ["fixture"]
        }, {
            "title": "真正相关的跨语言记录",
            "url": "https://selection.example/authoritative",
            "content": "This authoritative record directly addresses the planned focus.",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([
            (
                "https://selection.example/unrelated-first".to_string(),
                "Unrelated but substantive provider text.".to_string(),
            ),
            (
                "https://selection.example/unrelated-second".to_string(),
                "Otro texto sustantivo pero irrelevante.".to_string(),
            ),
            (
                "https://selection.example/authoritative".to_string(),
                "真正相关的跨语言记录提供了可追溯的一手证据，并明确回答了计划中的研究问题。"
                    .to_string(),
            ),
        ]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec!["真正相关的跨语言记录".to_string()],
        fail: false,
        invalid_selection: false,
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "semantic.admission",
            "语义来源准入",
            "从完整供应商候选目录中选择真正相关的跨语言记录"
        )]),
        serde_json::json!(["MiXeD provider catalog"]),
        serde_json::json!([]),
    );
    plan["budget"]["direct_fetches"] = serde_json::json!(1);
    let args = workflow_args(
        "验证跨语言语义来源准入",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(
        *urls.lock().unwrap(),
        ["https://selection.example/authoritative"]
    );
    assert_eq!(output["research"]["metadata"]["source_count"], 1);
    assert!(
        output["research"]["results"][0]["structured"]["sources"][0]["quote_or_fact"]
            .as_str()
            .is_some_and(|text| text.contains("可追溯的一手证据"))
    );
}

#[tokio::test]
async fn plan_seed_does_not_displace_semantically_selected_source_portfolio() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let urls = Arc::new(Mutex::new(Vec::new()));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Canonical first-party record",
            "url": "https://official.example/canonical",
            "content": "The canonical record directly establishes the planned focus.",
            "engines": ["fixture"]
        }, {
            "title": "Provider-selected independent record",
            "url": "https://selection.example/independent",
            "content": "The independent record directly corroborates the planned focus.",
            "engines": ["fixture"]
        }, {
            "title": "Unrelated provider record",
            "url": "https://selection.example/unrelated",
            "content": "This page does not address the planned focus.",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([
            (
                "https://seed.example/generic".to_string(),
                "A generic seed page does not establish the planned fact.".to_string(),
            ),
            (
                "https://official.example/canonical".to_string(),
                "The canonical first-party record establishes the planned fact.".to_string(),
            ),
            (
                "https://selection.example/independent".to_string(),
                "The independent record directly corroborates the planned fact.".to_string(),
            ),
            (
                "https://selection.example/unrelated".to_string(),
                "Unrelated but substantive provider text.".to_string(),
            ),
        ]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec![
            "Canonical first-party record".to_string(),
            "Provider-selected independent record".to_string(),
        ],
        fail: false,
        invalid_selection: false,
    }));
    let mut source_track = track(
        "source.portfolio",
        "Source portfolio",
        "Retain the canonical record and independent corroboration",
    );
    source_track["evidence_requirements"]["primary_source_required"] = serde_json::json!(true);
    source_track["evidence_requirements"]["independent_corroboration_required"] =
        serde_json::json!(true);
    let mut plan = minimal_plan(
        serde_json::json!([source_track]),
        serde_json::json!(["independent corroborating record"]),
        serde_json::json!(["https://seed.example/generic"]),
    );
    plan["budget"]["direct_fetches"] = serde_json::json!(2);
    let args = workflow_args(
        "Retain a typed source portfolio",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(
        *urls.lock().unwrap(),
        [
            "https://official.example/canonical",
            "https://selection.example/independent"
        ]
    );
    assert_eq!(output["research"]["metadata"]["source_count"], 2);
    let structured = output["research"]["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| &result["structured"])
        .collect::<Vec<_>>();
    assert_eq!(structured.len(), 2);
    assert!(structured.iter().all(|item| {
        item["sources"]
            .as_array()
            .is_some_and(|sources| sources.len() == 1)
            && item["source_coverage"]
                .as_array()
                .is_some_and(|bindings| bindings.len() == 1)
            && item["relevant_obligation_ids"]
                .as_array()
                .is_some_and(|obligations| obligations.len() == 1)
    }));
    assert!(structured
        .iter()
        .flat_map(|item| item["source_coverage"].as_array().into_iter().flatten())
        .all(|binding| binding["roles"]
            .as_array()
            .is_some_and(|roles| roles.contains(&serde_json::json!("supporting")))));

    let ledger =
        super::deep_research_evidence_ledger::accepted_evidence_ledger(&output.to_string(), None);
    assert_eq!(ledger.len(), 2);
    assert!(ledger.iter().all(|item| item.sources.len() == 1
        && item.source_coverage.len() == 1
        && item.relevant_obligation_ids == ["source.portfolio"]
        && item.source_coverage[0].source_id == item.sources[0].id));
}

#[tokio::test]
async fn typed_source_gap_drives_one_supplemental_retrieval_pass() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let urls = Arc::new(Mutex::new(Vec::new()));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Independent corroborating record",
            "url": "https://loop.example/independent",
            "content": "A separately attributable record corroborates the direct finding.",
            "engines": ["fixture"]
        }, {
            "title": "Unrelated remaining record",
            "url": "https://loop.example/unrelated",
            "content": "This candidate does not address the research obligation.",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([
            (
                "https://loop.example/primary".to_string(),
                "The direct first-party record establishes the finding.".to_string(),
            ),
            (
                "https://loop.example/independent".to_string(),
                "A separately attributable record corroborates the direct finding.".to_string(),
            ),
            (
                "https://loop.example/unrelated".to_string(),
                "Unrelated but substantive source text.".to_string(),
            ),
        ]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec![
            "https://loop.example/primary".to_string(),
            "Independent corroborating record".to_string(),
        ],
        fail: false,
        invalid_selection: false,
    }));
    let mut source_track = track(
        "source.loop",
        "Source loop",
        "Close direct support and independent corroboration",
    );
    source_track["evidence_requirements"]["primary_source_required"] = serde_json::json!(true);
    source_track["evidence_requirements"]["independent_corroboration_required"] =
        serde_json::json!(true);
    let mut plan = minimal_plan(
        serde_json::json!([source_track]),
        serde_json::json!(["independent corroborating record"]),
        serde_json::json!(["https://loop.example/primary"]),
    );
    plan["budget"]["direct_fetches"] = serde_json::json!(1);
    let mut args = workflow_args(
        "Close a typed source portfolio with one supplemental pass",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );
    args["run_id"] = serde_json::json!("deepresearch-initial-checkpoint-test");

    let output = execute(&executor, &args).await;

    assert_eq!(
        *urls.lock().unwrap(),
        [
            "https://loop.example/primary",
            "https://loop.example/independent"
        ]
    );
    assert_eq!(output["research"]["metadata"]["retrieval_pass_count"], 2);
    assert_eq!(
        output["research"]["metadata"]["supplemental_retrieval_attempted"],
        true
    );
    assert_eq!(output["research"]["metadata"]["source_count"], 2);
    let ledger =
        super::deep_research_evidence_ledger::accepted_evidence_ledger(&output.to_string(), None);
    assert_eq!(ledger.len(), 2);
    assert_eq!(
        ledger
            .iter()
            .flat_map(|item| item.source_coverage.iter())
            .filter(|binding| binding
                .roles
                .contains(&a3s::research::SourceEvidenceRole::Independent))
            .map(|binding| binding.source_id.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        2
    );
    let history = std::fs::read_to_string(
        workspace
            .path()
            .join(".a3s/workflow/deepresearch-initial-checkpoint-test.jsonl"),
    )
    .expect("durable retrieval history");
    let events = history
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let checkpoint_sequence = events
        .iter()
        .find(|event| {
            event["event"]["type"] == "step_completed"
                && event["event"]["step_id"] == "checkpoint_initial_retrieval"
        })
        .and_then(|event| event["sequence"].as_u64())
        .expect("completed initial checkpoint");
    let supplemental_sequence = events
        .iter()
        .find(|event| {
            event["event"]["type"] == "step_created"
                && event["event"]["step_id"] == "select_supplemental_web_sources"
        })
        .and_then(|event| event["sequence"].as_u64())
        .expect("supplemental source selection");
    assert!(checkpoint_sequence < supplemental_sequence);
}

#[tokio::test]
async fn failed_initial_fetch_uses_bounded_supplemental_replacement() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let urls = Arc::new(Mutex::new(Vec::new()));
    let unavailable = "https://replacement.example/unavailable";
    let replacement = "https://replacement.example/authoritative";
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Initially selected unavailable source",
            "url": unavailable,
            "content": "A promising initial source that fails during fetch.",
            "engines": ["fixture"]
        }, {
            "title": "Authoritative replacement source",
            "url": replacement,
            "content": "A replacement that directly establishes the planned finding.",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([(
            replacement.to_string(),
            "The authoritative replacement directly establishes the planned finding.".to_string(),
        )]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec![
            "Initially selected unavailable source".to_string(),
            "Authoritative replacement source".to_string(),
        ],
        fail: false,
        invalid_selection: false,
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "replacement.fetch",
            "Replacement fetch",
            "Retain traceable evidence after the first admitted fetch fails"
        )]),
        serde_json::json!(["replacement evidence"]),
        serde_json::json!([]),
    );
    plan["budget"]["direct_fetches"] = serde_json::json!(1);
    let args = workflow_args(
        "Recover one failed admitted fetch",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(*urls.lock().unwrap(), [unavailable, replacement]);
    assert_eq!(output["research"]["metadata"]["retrieval_pass_count"], 2);
    assert_eq!(
        output["research"]["metadata"]["supplemental"]["operational_gap_count"],
        1
    );
    assert_eq!(output["research"]["metadata"]["source_count"], 1);
    assert!(output.to_string().contains("authoritative replacement"));
    assert!(!output.to_string().contains("promising initial source"));
}

#[tokio::test]
async fn supplemental_replacement_avoids_a_failed_transport_surface() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let urls = Arc::new(Mutex::new(Vec::new()));
    let unavailable = "https://registry.example/items/unavailable";
    let same_surface = "https://registry.example/items/alternate";
    let diversified = "https://official.example/releases/current";
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Initially selected unavailable registry page",
            "url": unavailable,
            "content": "A promising registry record that retains no fetched text.",
            "engines": ["fixture"]
        }, {
            "title": "Another page on the failed registry surface",
            "url": same_surface,
            "content": "A near-identical registry transport opportunity.",
            "engines": ["fixture"]
        }, {
            "title": "Diversified official release record",
            "url": diversified,
            "content": "A first-party release record on a distinct transport surface.",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([(
            diversified.to_string(),
            "The first-party release record directly establishes the planned finding.".to_string(),
        )]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec![
            "Initially selected unavailable registry page".to_string(),
            "Another page on the failed registry surface".to_string(),
            "Diversified official release record".to_string(),
        ],
        fail: false,
        invalid_selection: false,
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "replacement.surface",
            "Replacement surface",
            "Recover traceable evidence through a distinct transport surface"
        )]),
        serde_json::json!(["release evidence"]),
        serde_json::json!([]),
    );
    plan["budget"]["direct_fetches"] = serde_json::json!(1);
    let args = workflow_args(
        "Recover a failed fetch without repeating its transport surface",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(*urls.lock().unwrap(), [unavailable, diversified]);
    assert!(!urls.lock().unwrap().iter().any(|url| url == same_surface));
    assert_eq!(
        output["research"]["metadata"]["supplemental"]["web"]["failed_transport_surface_count"],
        1
    );
    assert_eq!(output["research"]["metadata"]["source_count"], 1);
}

#[tokio::test]
async fn transient_web_source_selector_failure_replays_only_source_admission() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let queries = Arc::new(Mutex::new(Vec::new()));
    let urls = Arc::new(Mutex::new(Vec::new()));
    let selector_calls = Arc::new(AtomicUsize::new(0));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::clone(&queries),
        results: serde_json::json!([{
            "title": "Unrelated discovery candidate",
            "url": "https://source-retry.example/unrelated",
            "content": "Unrelated discovery metadata.",
            "engines": ["fixture"]
        }, {
            "title": "Authoritative source retry record",
            "url": "https://source-retry.example/authoritative",
            "content": "The authoritative source retry record addresses the focus.",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([
            (
                "https://source-retry.example/unrelated".to_string(),
                "Unrelated but substantive source text.".to_string(),
            ),
            (
                "https://source-retry.example/authoritative".to_string(),
                "The authoritative source retry record remains traceable after recovery."
                    .to_string(),
            ),
        ]),
    }));
    executor.register_dynamic_tool(Arc::new(RetryOnceSemanticSelectorFixture {
        calls: Arc::clone(&selector_calls),
        selector: SemanticSelectorFixture {
            preferred_fragments: vec!["authoritative source retry record".to_string()],
            fail: false,
            invalid_selection: false,
        },
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "source.retry",
            "Source retry",
            "Retain the authoritative source retry record"
        )]),
        serde_json::json!(["source selector recovery"]),
        serde_json::json!([]),
    );
    plan["budget"]["direct_fetches"] = serde_json::json!(1);
    let args = workflow_args(
        "Verify source selector recovery",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("retrieval workflow execution");

    assert_eq!(result.exit_code, 0, "{}", result.output);
    assert_eq!(selector_calls.load(Ordering::SeqCst), 3);
    assert_eq!(queries.lock().unwrap().len(), 1);
    assert_eq!(
        *urls.lock().unwrap(),
        ["https://source-retry.example/authoritative"]
    );
    let steps = result
        .metadata
        .as_ref()
        .and_then(|metadata| metadata["dynamic_workflow"]["snapshot"]["steps"].as_object())
        .expect("durable workflow steps");
    assert_eq!(steps.len(), 5);
    assert_eq!(steps["discover_web_sources"]["attempt"], 1);
    assert_eq!(steps["select_web_sources"]["attempt"], 2);
    assert_eq!(steps["retrieve_web"]["attempt"], 1);
    assert_eq!(steps["select_evidence_chunks"]["attempt"], 1);
    assert_eq!(steps["checkpoint_initial_retrieval"]["attempt"], 1);
}

#[tokio::test]
async fn fetch_candidates_preserve_query_and_provider_result_order() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let queries = Arc::new(Mutex::new(Vec::new()));
    let urls = Arc::new(Mutex::new(Vec::new()));
    let first_query = "First exact provider query";
    let second_query = "第二个精确查询";
    let first_urls = [
        "https://order-a.example/one",
        "https://order-a.example/two",
        "https://order-c.example/three",
    ];
    let second_urls = ["https://order-b.example/one", "https://order-d.example/two"];
    executor.register_dynamic_tool(Arc::new(QuerySearchFixture {
        queries: Arc::clone(&queries),
        results_by_query: BTreeMap::from([
            (
                first_query.to_string(),
                serde_json::json!(first_urls
                    .iter()
                    .map(|url| serde_json::json!({
                        "title": url,
                        "url": url,
                        "engines": ["fixture"]
                    }))
                    .collect::<Vec<_>>()),
            ),
            (
                second_query.to_string(),
                serde_json::json!(second_urls
                    .iter()
                    .map(|url| serde_json::json!({
                        "title": url,
                        "url": url,
                        "engines": ["fixture"]
                    }))
                    .collect::<Vec<_>>()),
            ),
        ]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: first_urls
            .iter()
            .chain(second_urls.iter())
            .map(|url| {
                (
                    (*url).to_string(),
                    format!("Substantive provider-ordered evidence from {url}."),
                )
            })
            .collect(),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: Vec::new(),
        fail: false,
        invalid_selection: false,
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "provider.order",
            "Provider order",
            "Preserve provider discovery order"
        )]),
        serde_json::json!([first_query, second_query]),
        serde_json::json!([]),
    );
    plan["budget"]["direct_fetches"] = serde_json::json!(4);
    let args = workflow_args(
        "Preserve provider discovery order",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_query_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(
        *queries.lock().unwrap(),
        [first_query.to_string(), second_query.to_string()]
    );
    assert_eq!(
        *urls.lock().unwrap(),
        [
            first_urls[0].to_string(),
            first_urls[1].to_string(),
            first_urls[2].to_string(),
            second_urls[0].to_string(),
        ]
    );
    assert_eq!(output["research"]["metadata"]["source_count"], 4);
}

#[tokio::test]
async fn oversized_chunk_catalog_fails_closed_without_positional_sampling() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([
            {
                "title": "Oversized source one",
                "url": "https://overflow.example/evidence-one",
                "engines": ["fixture"]
            },
            {
                "title": "Oversized source two",
                "url": "https://overflow.example/evidence-two",
                "engines": ["fixture"]
            }
        ]),
    }));
    let oversized = (0..210)
        .map(|index| {
            format!(
                "OVERFLOW_SECRET_EVIDENCE_{index:03} {}",
                "bounded-source-content ".repeat(18)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::new(Mutex::new(Vec::new())),
        bodies: BTreeMap::from([
            (
                "https://overflow.example/evidence-one".to_string(),
                oversized.clone(),
            ),
            (
                "https://overflow.example/evidence-two".to_string(),
                oversized,
            ),
        ]),
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "closed.catalog",
            "Closed catalog",
            "Retain a complete bounded chunk catalog or fail closed"
        )]),
        serde_json::json!(["oversized evidence catalog"]),
        serde_json::json!([]),
    );
    plan["budget"]["direct_fetches"] = serde_json::json!(2);
    let args = workflow_args(
        "Retain a complete bounded catalog",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(output["research"]["status"], "failed");
    assert_eq!(output["research"]["metadata"]["source_count"], 0);
    let catalog_chunk_count = output["research"]["metadata"]["web"]["catalog_chunk_count"]
        .as_u64()
        .unwrap_or_default();
    assert!(
        catalog_chunk_count > 384,
        "fixture produced only {catalog_chunk_count} chunks"
    );
    assert!(output.to_string().contains("closed catalog limit"));
    assert!(!output.to_string().contains("OVERFLOW_SECRET_EVIDENCE"));
}

#[tokio::test]
async fn eight_source_catalog_above_the_old_limit_uses_one_selector_per_source() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let queries = Arc::new(Mutex::new(Vec::new()));
    let urls = Arc::new(Mutex::new(Vec::new()));
    let source_urls = (1..=8)
        .map(|index| format!("https://source-local-selection.example/source-{index}"))
        .collect::<Vec<_>>();
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::clone(&queries),
        results: serde_json::json!(source_urls
            .iter()
            .map(|url| serde_json::json!({
                "title": url,
                "url": url,
                "engines": ["fixture"]
            }))
            .collect::<Vec<_>>()),
    }));
    let bodies = source_urls
        .iter()
        .enumerate()
        .map(|(source_index, url)| {
            let lines = (1..=30)
                .map(|chunk_index| {
                    format!(
                        "SHARDED_TARGET_{}_{} {}",
                        source_index + 1,
                        chunk_index,
                        "complete semantic shard evidence ".repeat(19)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            (url.clone(), lines)
        })
        .collect::<BTreeMap<_, _>>();
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies,
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: (1..=8)
            .flat_map(|source| {
                [1, 10, 20, 30]
                    .into_iter()
                    .map(move |chunk| format!("SHARDED_TARGET_{source}_{chunk}"))
            })
            .collect(),
        fail: false,
        invalid_selection: false,
    }));
    let plan = minimal_plan(
        serde_json::json!([track(
            "selector.sources",
            "Semantic source selectors",
            "Retain the late source-local target after every chunk is semantically considered"
        )]),
        serde_json::json!(["complete source-local semantic catalog"]),
        serde_json::json!([]),
    );
    let args = workflow_args(
        "Verify complete source-local semantic selection",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("source-local retrieval workflow execution");

    assert_eq!(result.exit_code, 0, "{}", result.output);
    assert_eq!(queries.lock().unwrap().len(), 1);
    assert_eq!(
        urls.lock().unwrap().iter().collect::<BTreeSet<_>>().len(),
        8,
        "isolated batch-output recovery may refetch a URL but must preserve all eight identities"
    );
    let output: serde_json::Value =
        serde_json::from_str(&result.output).expect("sharded retrieval output");
    assert_eq!(output["research"]["status"], "success");
    assert_eq!(
        output["research"]["metadata"]["semantic_selection_shard_count"],
        8
    );
    assert!(
        output["research"]["metadata"]["catalog_chunk_count"]
            .as_u64()
            .is_some_and(|count| count > 192 && count <= 384),
        "the complete eight-source catalog must cross the retired 192-chunk ceiling"
    );
    assert!(
        output["research"]["metadata"]["semantic_selection_candidate_count"]
            .as_u64()
            .is_some_and(|count| count > 0 && count <= 32)
    );
    assert!(output["research"]["results"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|result| {
            result["structured"]["sources"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .flat_map(|source| source["evidence_excerpts"].as_array().into_iter().flatten())
        .any(|excerpt| excerpt["quote_or_fact"]
            .as_str()
            .is_some_and(|text| text.contains("SHARDED_TARGET_8_30"))));
    assert_eq!(
        output["research"]["results"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|result| {
                result["structured"]["key_evidence"]
                    .as_array()
                    .into_iter()
                    .flatten()
            })
            .count(),
        32,
        "all bounded source-local selections must reach closed question review"
    );

    let steps = result
        .metadata
        .as_ref()
        .and_then(|metadata| metadata["dynamic_workflow"]["snapshot"]["steps"].as_object())
        .expect("source-local durable workflow steps");
    assert_eq!(steps["discover_web_sources"]["attempt"], 1);
    assert_eq!(steps["retrieve_web"]["attempt"], 1);
    assert!(!steps.contains_key("select_evidence_chunks"));
    let mut shard_attempts = (1..=8)
        .map(|index| {
            steps[&format!("select_evidence_chunks_shard_{index}")]["attempt"]
                .as_u64()
                .expect("shard attempt")
        })
        .collect::<Vec<_>>();
    shard_attempts.sort_unstable();
    assert_eq!(shard_attempts, [1, 1, 1, 1, 1, 1, 1, 1]);
}

#[tokio::test]
async fn failed_shards_promote_no_own_text_but_preserve_valid_siblings() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let failed_url = "https://partial-shards.example/failed";
    let retained_url = "https://partial-shards.example/retained";
    let body = |prefix: &str| {
        (1..=12)
            .map(|index| {
                format!(
                    "{prefix}_{index:02} {}",
                    "bounded source-local semantic evidence ".repeat(16)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::new(Mutex::new(Vec::new())),
        bodies: BTreeMap::from([
            (failed_url.to_string(), body("FAILED_SHARD_TEXT")),
            (retained_url.to_string(), body("RETAINED_SHARD_TEXT")),
        ]),
    }));
    executor.register_dynamic_tool(Arc::new(FailMatchingSemanticSelectorFixture {
        schema_name: "deep_research_evidence_shard_selection",
        fragment: "FAILED_SHARD_TEXT".to_string(),
        selector: SemanticSelectorFixture {
            preferred_fragments: vec!["RETAINED_SHARD_TEXT_12".to_string()],
            fail: false,
            invalid_selection: false,
        },
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "partial.shards",
            "Partial shards",
            "Preserve only independently validated source-local shard text"
        )]),
        serde_json::json!([]),
        serde_json::json!([failed_url, retained_url]),
    );
    plan["budget"]["direct_searches"] = serde_json::json!(0);
    plan["budget"]["direct_fetches"] = serde_json::json!(2);
    let args = workflow_args(
        "Retain validated siblings after one source-local selector fails",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "unused_fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(output["research"]["status"], "partial_success");
    assert_eq!(output["research"]["metadata"]["source_count"], 1);
    assert_eq!(
        output["research"]["metadata"]["semantic_selection_failed_shard_count"],
        1
    );
    let encoded = output.to_string();
    assert!(encoded.contains("RETAINED_SHARD_TEXT"));
    assert!(!encoded.contains("FAILED_SHARD_TEXT"));
    assert!(encoded.contains("simulated source-local shard timeout"));
}

#[tokio::test]
async fn failed_source_local_selection_drops_only_that_source() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let failed_url = "https://partial-reduction.example/failed";
    let retained_url = "https://partial-reduction.example/retained";
    let failed_body = (1..=40)
        .map(|index| {
            format!(
                "FAILED_SOURCE_REDUCTION_{index:02} {}",
                "bounded source reduction evidence ".repeat(18)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::new(Mutex::new(Vec::new())),
        bodies: BTreeMap::from([
            (failed_url.to_string(), failed_body),
            (
                retained_url.to_string(),
                "RETAINED_WITHOUT_REDUCTION direct independently validated evidence.".to_string(),
            ),
        ]),
    }));
    executor.register_dynamic_tool(Arc::new(FailMatchingSemanticSelectorFixture {
        schema_name: "deep_research_evidence_shard_selection",
        fragment: "FAILED_SOURCE_REDUCTION".to_string(),
        selector: SemanticSelectorFixture {
            preferred_fragments: vec![
                "FAILED_SOURCE_REDUCTION_05".to_string(),
                "FAILED_SOURCE_REDUCTION_08".to_string(),
                "FAILED_SOURCE_REDUCTION_15".to_string(),
                "FAILED_SOURCE_REDUCTION_18".to_string(),
                "FAILED_SOURCE_REDUCTION_25".to_string(),
                "FAILED_SOURCE_REDUCTION_28".to_string(),
                "FAILED_SOURCE_REDUCTION_35".to_string(),
                "FAILED_SOURCE_REDUCTION_38".to_string(),
                "RETAINED_WITHOUT_REDUCTION".to_string(),
            ],
            fail: false,
            invalid_selection: false,
        },
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "partial.source-reduction",
            "Partial source reduction",
            "Drop only the source whose final bounded reduction fails"
        )]),
        serde_json::json!([]),
        serde_json::json!([failed_url, retained_url]),
    );
    plan["budget"]["direct_searches"] = serde_json::json!(0);
    plan["budget"]["direct_fetches"] = serde_json::json!(2);
    let args = workflow_args(
        "Retain independent sources after one source reducer fails",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "unused_fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(output["research"]["status"], "partial_success");
    assert_eq!(output["research"]["metadata"]["source_count"], 1);
    assert_eq!(
        output["research"]["metadata"]["semantic_selection_failed_shard_count"],
        1
    );
    assert_eq!(
        output["research"]["metadata"]["semantic_selection_source_reduction_count"],
        0
    );
    let encoded = output.to_string();
    assert!(encoded.contains("RETAINED_WITHOUT_REDUCTION"));
    assert!(!encoded.contains("FAILED_SOURCE_REDUCTION"));
    assert!(encoded.contains("simulated source-local shard timeout"));
}

#[tokio::test]
async fn large_source_is_semantically_reduced_in_one_source_local_unit() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let queries = Arc::new(Mutex::new(Vec::new()));
    let urls = Arc::new(Mutex::new(Vec::new()));
    let source_url = "https://source-reduction.example/complete";
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::clone(&queries),
        results: serde_json::json!([{
            "title": "Complete source reduction fixture",
            "url": source_url,
            "engines": ["fixture"]
        }]),
    }));
    let body = (1..=40)
        .map(|chunk_index| {
            format!(
                "SOURCE_REDUCER_TARGET_{chunk_index} {}",
                "closed semantic source evidence ".repeat(20)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([(source_url.to_string(), body)]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: [40, 35, 25, 15, 10, 5]
            .into_iter()
            .map(|index| format!("SOURCE_REDUCER_TARGET_{index}"))
            .collect(),
        fail: false,
        invalid_selection: false,
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "selector.source-reduction",
            "Semantic source reduction",
            "Retain the strongest late candidate while enforcing the per-source evidence limit"
        )]),
        serde_json::json!(["complete source semantic reduction"]),
        serde_json::json!([]),
    );
    plan["budget"]["direct_fetches"] = serde_json::json!(1);
    let args = workflow_args(
        "Verify semantic per-source reduction",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("semantic source-reduction workflow");

    assert_eq!(result.exit_code, 0, "{}", result.output);
    assert_eq!(queries.lock().unwrap().len(), 1);
    assert_eq!(*urls.lock().unwrap(), [source_url]);
    let output: serde_json::Value =
        serde_json::from_str(&result.output).expect("source-reduction output");
    assert_eq!(output["research"]["status"], "success");
    assert_eq!(
        output["research"]["metadata"]["semantic_selection_shard_count"],
        1
    );
    assert_eq!(
        output["research"]["metadata"]["semantic_selection_source_reduction_count"],
        0
    );
    assert_eq!(
        output["research"]["metadata"]["semantic_selection_materialized_count"],
        4
    );
    let excerpts = output["research"]["results"][0]["structured"]["sources"][0]
        ["evidence_excerpts"]
        .as_array()
        .expect("bounded source excerpts");
    assert_eq!(excerpts.len(), 4);
    assert!(excerpts.iter().any(|excerpt| excerpt["quote_or_fact"]
        .as_str()
        .is_some_and(|text| text.contains("SOURCE_REDUCER_TARGET_40"))));

    let steps = result
        .metadata
        .as_ref()
        .and_then(|metadata| metadata["dynamic_workflow"]["snapshot"]["steps"].as_object())
        .expect("source-reduction durable steps");
    assert_eq!(steps["select_evidence_chunks_shard_1"]["attempt"], 1);
    assert!(!steps.contains_key("select_evidence_chunks_shard_2"));
    assert!(!steps.contains_key("select_evidence_chunks_source_1"));
    assert!(!steps.contains_key("select_evidence_chunks"));
}

#[tokio::test]
async fn transient_selector_failure_retries_only_the_durable_selection_step() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let queries = Arc::new(Mutex::new(Vec::new()));
    let urls = Arc::new(Mutex::new(Vec::new()));
    let selector_calls = Arc::new(AtomicUsize::new(0));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::clone(&queries),
        results: serde_json::json!([{
            "title": "Durable selector record",
            "url": "https://selector-retry.example/record",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([(
            "https://selector-retry.example/record".to_string(),
            "The durable semantic selection retry retains this exact authoritative evidence."
                .to_string(),
        )]),
    }));
    executor.register_dynamic_tool(Arc::new(RetryOnceSemanticSelectorFixture {
        calls: Arc::clone(&selector_calls),
        selector: SemanticSelectorFixture {
            preferred_fragments: vec!["durable semantic selection retry".to_string()],
            fail: false,
            invalid_selection: false,
        },
    }));
    let plan = minimal_plan(
        serde_json::json!([track(
            "selector.retry",
            "Selector retry",
            "Verify durable semantic selection recovery"
        )]),
        serde_json::json!(["durable selector evidence"]),
        serde_json::json!([]),
    );
    let args = workflow_args(
        "Verify durable semantic selection recovery",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );
    a3s_code_core::tools::register_dynamic_workflow(executor.registry());

    let result = executor
        .execute("dynamic_workflow", &args)
        .await
        .expect("retrieval workflow execution");

    assert_eq!(result.exit_code, 0, "{}", result.output);
    assert_eq!(selector_calls.load(Ordering::SeqCst), 2);
    assert_eq!(queries.lock().unwrap().len(), 1);
    assert_eq!(urls.lock().unwrap().len(), 1);
    let output: serde_json::Value = serde_json::from_str(&result.output).expect("retrieval output");
    assert_eq!(output["research"]["status"], "success");
    assert!(
        output["research"]["results"][0]["structured"]["sources"][0]["quote_or_fact"]
            .as_str()
            .is_some_and(|text| text.contains("exact authoritative evidence"))
    );
    let steps = result
        .metadata
        .as_ref()
        .and_then(|metadata| metadata["dynamic_workflow"]["snapshot"]["steps"].as_object())
        .expect("durable workflow steps");
    assert_eq!(steps.len(), 4);
    assert_eq!(steps["discover_web_sources"]["attempt"], 1);
    assert_eq!(steps["retrieve_web"]["attempt"], 1);
    assert_eq!(steps["select_evidence_chunks"]["attempt"], 2);
    assert_eq!(steps["checkpoint_initial_retrieval"]["attempt"], 1);
}

#[tokio::test]
async fn selector_failure_promotes_no_fetched_text() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Fetched record",
            "url": "https://failure.example/record",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::new(Mutex::new(Vec::new())),
        bodies: BTreeMap::from([(
            "https://failure.example/record".to_string(),
            "RAW_SECRET_EVIDENCE must never be promoted when semantic selection fails.".to_string(),
        )]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: Vec::new(),
        fail: true,
        invalid_selection: false,
    }));
    let plan = minimal_plan(
        serde_json::json!([track("failure.boundary", "Boundary", "Verify the boundary")]),
        serde_json::json!(["boundary record"]),
        serde_json::json!([]),
    );
    let args = workflow_args(
        "Verify the boundary",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(output["research"]["status"], "failed");
    assert_eq!(output["research"]["metadata"]["source_count"], 0);
    assert_eq!(
        output["research"]["results"].as_array().map(Vec::len),
        Some(0)
    );
    assert!(!output.to_string().contains("RAW_SECRET_EVIDENCE"));
}

#[tokio::test]
async fn selector_id_outside_closed_catalog_promotes_no_fetched_text() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Fetched record",
            "url": "https://invalid-selection.example/record",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::new(Mutex::new(Vec::new())),
        bodies: BTreeMap::from([(
            "https://invalid-selection.example/record".to_string(),
            "OUT_OF_CATALOG_SECRET must never be promoted from fetched text.".to_string(),
        )]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: Vec::new(),
        fail: false,
        invalid_selection: true,
    }));
    let plan = minimal_plan(
        serde_json::json!([track(
            "closed.catalog",
            "Closed catalog",
            "Verify selector IDs against the fetched catalog",
        )]),
        serde_json::json!(["closed catalog record"]),
        serde_json::json!([]),
    );
    let args = workflow_args(
        "Verify the closed evidence catalog",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(output["research"]["status"], "failed");
    assert_eq!(output["research"]["metadata"]["source_count"], 0);
    assert_eq!(
        output["research"]["results"].as_array().map(Vec::len),
        Some(0)
    );
    assert!(!output.to_string().contains("OUT_OF_CATALOG_SECRET"));
    assert!(output.to_string().contains("outside the closed catalog"));
}

#[tokio::test]
async fn pdf_additional_ranges_remain_one_source_in_one_retrieval_pass() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let offsets = Arc::new(Mutex::new(Vec::new()));
    executor.register_dynamic_tool(Arc::new(PaginatedPdfFixture {
        offsets: Arc::clone(&offsets),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: vec![
            "双阶段检索方法".to_string(),
            "引用完整率下降".to_string(),
            "只覆盖英语技术主题".to_string(),
        ],
        fail: false,
        invalid_selection: false,
    }));
    let plan = minimal_plan(
        serde_json::json!([
            track("method", "方法", "双阶段检索方法"),
            track("evaluation", "评测", "引用完整率下降"),
            track("limitations", "限制", "只覆盖英语技术主题")
        ]),
        serde_json::json!([]),
        serde_json::json!(["https://papers.example/report.pdf"]),
    );
    let args = workflow_args(
        "分析报告的方法、评测与限制",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_pdf_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(*offsets.lock().unwrap(), [0, 100, 200]);
    assert_eq!(
        output["research"]["metadata"]["web"]["document_range_count"],
        3
    );
    let sources = output["research"]["results"][0]["structured"]["sources"]
        .as_array()
        .expect("PDF sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0]["evidence_excerpts"].as_array().map(Vec::len),
        Some(3)
    );
}

#[tokio::test]
async fn local_only_retrieval_accepts_only_read_or_grep_anchors() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(workspace.path().join("src")).unwrap();
    std::fs::write(
        workspace.path().join("src/research.rs"),
        "pub const RESEARCH_BOUNDARY: &str = \"The observed file defines the research boundary from exact workspace text.\";\n",
    )
    .unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    executor.register_dynamic_tool(Arc::new(LocalEvidenceFixture));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: Vec::new(),
        fail: false,
        invalid_selection: false,
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "workspace",
            "Workspace",
            "Inspect observed workspace evidence"
        )]),
        serde_json::json!([]),
        serde_json::json!([]),
    );
    plan["workspace_evidence_required"] = serde_json::json!(true);
    let args = workflow_args(
        "Inspect the local workspace",
        super::DeepResearchEvidenceScope::LocalOnly,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(output["research"]["status"], "partial_success");
    assert_eq!(
        output["research"]["metadata"]["evidence_selection_mode"],
        "semantic_chunk_ids_with_typed_coverage"
    );
    let sources = output["research"]["results"][0]["structured"]["sources"]
        .as_array()
        .expect("local sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["url_or_path"], "src/research.rs");
    assert!(sources[0]["quote_or_fact"]
        .as_str()
        .is_some_and(|text| text.contains("exact workspace text")));
    assert_eq!(
        output["research"]["results"][0]["structured"]["gaps"],
        serde_json::json!([]),
        "collection diagnostics must not become semantic research gaps"
    );
    assert!(output["research"]["warnings"]["collection_errors"]
        .as_array()
        .is_some_and(|errors| !errors.is_empty()));
    assert!(!output.to_string().contains("fabricated.rs"));
    assert!(!output.to_string().contains("listed-only.rs"));
}

#[tokio::test]
async fn local_text_is_not_promoted_when_closed_chunk_selection_fails() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(workspace.path().join("src")).unwrap();
    let exact_text =
        "This exact local sentence must remain behind the failed semantic selector boundary.";
    std::fs::write(
        workspace.path().join("src/research.rs"),
        format!("pub const LOCAL_EVIDENCE: &str = \"{exact_text}\";\n"),
    )
    .unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    executor.register_dynamic_tool(Arc::new(LocalEvidenceFixture));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: Vec::new(),
        fail: true,
        invalid_selection: false,
    }));
    let mut plan = minimal_plan(
        serde_json::json!([track(
            "workspace",
            "Workspace",
            "Inspect observed workspace evidence"
        )]),
        serde_json::json!([]),
        serde_json::json!([]),
    );
    plan["workspace_evidence_required"] = serde_json::json!(true);
    let args = workflow_args(
        "Inspect the local workspace",
        super::DeepResearchEvidenceScope::LocalOnly,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(output["research"]["status"], "failed");
    assert_eq!(output["research"]["metadata"]["source_count"], 0);
    assert!(!output.to_string().contains(exact_text));
}

#[tokio::test]
async fn non_document_urls_are_filtered_before_fetch() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let urls = Arc::new(Mutex::new(Vec::new()));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Avatar",
            "url": "https://cdn.example/avatar.png",
            "engines": ["fixture"]
        }, {
            "title": "Archive",
            "url": "https://downloads.example/research.zip",
            "engines": ["fixture"]
        }, {
            "title": "Evidence",
            "url": "https://valid.example/evidence",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TextFetchFixture {
        urls: Arc::clone(&urls),
        bodies: BTreeMap::from([(
            "https://valid.example/evidence".to_string(),
            "The valid document contains substantive traceable evidence for the requested focus."
                .to_string(),
        )]),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: Vec::new(),
        fail: false,
        invalid_selection: false,
    }));
    let plan = minimal_plan(
        serde_json::json!([track("filter", "Filter", "Retain document evidence")]),
        serde_json::json!(["document evidence"]),
        serde_json::json!([]),
    );
    let args = workflow_args(
        "Retain document evidence",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_web_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(*urls.lock().unwrap(), ["https://valid.example/evidence"]);
    assert_eq!(output["research"]["metadata"]["source_count"], 1);
}

#[tokio::test]
async fn transient_fetches_receive_exactly_one_retry() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let calls = Arc::new(AtomicUsize::new(0));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "First",
            "url": "https://first.example/evidence",
            "engines": ["fixture"]
        }, {
            "title": "Second",
            "url": "https://second.example/evidence",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(TransientFetchFixture {
        calls: Arc::clone(&calls),
    }));
    executor.register_dynamic_tool(Arc::new(SemanticSelectorFixture {
        preferred_fragments: Vec::new(),
        fail: false,
        invalid_selection: false,
    }));
    let plan = minimal_plan(
        serde_json::json!([track("retry", "Retry", "Retain retried evidence")]),
        serde_json::json!(["retried evidence"]),
        serde_json::json!([]),
    );
    let args = workflow_args(
        "Retain retried evidence",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_transient_fetch",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert_eq!(output["research"]["metadata"]["source_count"], 2);
}

#[tokio::test]
async fn transport_like_error_text_does_not_trigger_an_untyped_retry() {
    let workspace = tempfile::tempdir().unwrap();
    let executor = ToolExecutor::new(workspace.path().to_string_lossy().to_string());
    let calls = Arc::new(AtomicUsize::new(0));
    executor.register_dynamic_tool(Arc::new(SearchFixture {
        queries: Arc::new(Mutex::new(Vec::new())),
        results: serde_json::json!([{
            "title": "Untyped failure",
            "url": "https://failure.example/evidence",
            "engines": ["fixture"]
        }]),
    }));
    executor.register_dynamic_tool(Arc::new(UntypedFetchFailureFixture {
        calls: Arc::clone(&calls),
    }));
    let plan = minimal_plan(
        serde_json::json!([track(
            "typed-retry",
            "Typed retry",
            "Retry only structured transport failures"
        )]),
        serde_json::json!(["structured retry"]),
        serde_json::json!([]),
    );
    let args = workflow_args(
        "Retry only structured transport failures",
        super::DeepResearchEvidenceScope::WebAndWorkspace,
        plan,
        "fixture_web_search",
        "fixture_untyped_fetch_failure",
    );

    let output = execute(&executor, &args).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        output["research"]["metadata"]["web"]["transport_retry_count"],
        0
    );
    assert_eq!(output["research"]["status"], "failed");
}
