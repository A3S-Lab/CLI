use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use a3s_code_core::llm::structured::{
    self, NativeStructuredSupport, StructuredDirective, StructuredMode, StructuredRequest,
};
use a3s_code_core::llm::{
    ContentBlock, ModelGenerationConcurrency, StreamEvent, TokenUsage, ToolDefinition,
};
use a3s_code_core::{CodeConfig, LlmClient, LlmResponse, Message, SessionOptions};
use a3s_deep_research::engine::{
    DEFAULT_DURABLE_GENERATION_GRACE_MS, DEFAULT_MAX_CONCURRENT_GENERATIONS,
    DEFAULT_PLANNER_ATTEMPT_TIMEOUT_MS, DEFAULT_REPORT_ATTEMPT_TIMEOUT_MS,
};
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{
    deep_research_candidate_timeout, prioritize_candidate_diversity,
    resolve_deep_research_llm_client, structured_support_satisfies, DeepResearchFailoverClient,
    ModelFailureDomain, ModelRoute, ModelSource, ResearchCandidateSpec,
};

#[derive(Clone)]
struct ControlledClient {
    label: &'static str,
    failure: Option<&'static str>,
    support: NativeStructuredSupport,
    distinct_non_streaming_transport: bool,
    concurrency: NonZeroUsize,
    forkable: bool,
    response_delay: Duration,
    stream_completion_delay: Duration,
    stream_progress_interval: Option<Duration>,
    scripted_stream: Option<Vec<StreamEvent>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl ControlledClient {
    fn succeeding(label: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            label,
            failure: None,
            support: NativeStructuredSupport::ForcedTool,
            distinct_non_streaming_transport: true,
            concurrency: NonZeroUsize::MIN,
            forkable: false,
            response_delay: Duration::ZERO,
            stream_completion_delay: Duration::ZERO,
            stream_progress_interval: None,
            scripted_stream: None,
            calls,
        }
    }

    fn failing(label: &'static str, error: &'static str, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            failure: Some(error),
            ..Self::succeeding(label, calls)
        }
    }

    fn with_capabilities(mut self, support: NativeStructuredSupport, concurrency: usize) -> Self {
        self.support = support;
        self.concurrency = NonZeroUsize::new(concurrency).unwrap_or(NonZeroUsize::MIN);
        self
    }

    fn with_streaming_only_transport(mut self) -> Self {
        self.distinct_non_streaming_transport = false;
        self
    }

    fn with_forking(mut self) -> Self {
        self.forkable = true;
        self
    }

    fn with_response_delay(mut self, response_delay: Duration) -> Self {
        self.response_delay = response_delay;
        self
    }

    fn with_stream_completion_delay(mut self, stream_completion_delay: Duration) -> Self {
        self.stream_completion_delay = stream_completion_delay;
        self
    }

    fn with_stream_progress_interval(mut self, stream_progress_interval: Duration) -> Self {
        self.stream_progress_interval = Some(stream_progress_interval);
        self
    }

    fn with_scripted_stream(mut self, events: Vec<StreamEvent>) -> Self {
        self.scripted_stream = Some(events);
        self
    }

    fn record(&self, method: &str) {
        self.calls
            .lock()
            .expect("controlled client call log")
            .push(format!("{}:{method}", self.label));
    }

    async fn outcome(&self) -> anyhow::Result<LlmResponse> {
        if !self.response_delay.is_zero() {
            tokio::time::sleep(self.response_delay).await;
        }
        match self.failure {
            Some(error) => Err(anyhow::anyhow!(error)),
            None => Ok(test_response(self.label)),
        }
    }

    async fn streaming_outcome(&self) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = self.outcome().await?;
        let (tx, rx) = mpsc::channel(1);
        if let Some(events) = self.scripted_stream.clone() {
            tokio::spawn(async move {
                for event in events {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
            });
            return Ok(rx);
        }
        let completion_delay = self.stream_completion_delay;
        let progress_interval = self.stream_progress_interval;
        tokio::spawn(async move {
            let started_at = tokio::time::Instant::now();
            if let Some(progress_interval) = progress_interval {
                loop {
                    let elapsed = started_at.elapsed();
                    let remaining = completion_delay.saturating_sub(elapsed);
                    if remaining <= progress_interval {
                        break;
                    }
                    tokio::time::sleep(progress_interval).await;
                    if tx
                        .send(StreamEvent::TextDelta("progress".to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            let remaining = completion_delay.saturating_sub(started_at.elapsed());
            if !remaining.is_zero() {
                tokio::time::sleep(remaining).await;
            }
            tx.send(StreamEvent::Done(response)).await.ok();
        });
        Ok(rx)
    }
}

#[async_trait]
impl LlmClient for ControlledClient {
    fn model_generation_concurrency(&self) -> ModelGenerationConcurrency {
        ModelGenerationConcurrency::bounded(self.concurrency)
    }

    fn fork_for_session(&self, _session_id: &str) -> Option<Arc<dyn LlmClient>> {
        self.forkable
            .then(|| Arc::new(self.clone()) as Arc<dyn LlmClient>)
    }

    async fn complete(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        self.record("complete");
        self.outcome().await
    }

    async fn complete_streaming(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        self.record("streaming");
        self.streaming_outcome().await
    }

    fn native_structured_support(&self) -> NativeStructuredSupport {
        self.support
    }

    fn has_distinct_non_streaming_transport(&self) -> bool {
        self.distinct_non_streaming_transport
    }

    async fn complete_structured(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _directive: &StructuredDirective,
    ) -> anyhow::Result<LlmResponse> {
        self.record("structured");
        self.outcome().await
    }

    async fn complete_streaming_structured(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _directive: &StructuredDirective,
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        self.record("streaming_structured");
        self.streaming_outcome().await
    }
}

fn test_response(text: &str) -> LlmResponse {
    LlmResponse {
        message: Message {
            role: "assistant".to_string(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            reasoning_content: None,
        },
        usage: TokenUsage::default(),
        stop_reason: Some("stop".to_string()),
        token_logprobs: Vec::new(),
        meta: None,
    }
}

fn failover_client(clients: Vec<Arc<dyn LlmClient>>) -> DeepResearchFailoverClient {
    DeepResearchFailoverClient::new(clients, 0, Duration::from_secs(1))
        .expect("non-empty test candidates")
}

fn capability_config() -> CodeConfig {
    CodeConfig::from_acl(
        r#"
            default_model = "provider-a/enabled"

            providers "provider-a" {
              apiKey = "test-key"
              baseUrl = "https://endpoint-a.invalid/v1"

              models "enabled" {}
              models "disabled" { toolCall = false }
            }
        "#,
    )
    .expect("capability config")
}

#[test]
fn deep_research_rejects_a_configured_primary_without_tool_calling() {
    let options = SessionOptions::new().with_model("provider-a/disabled");

    let error =
        resolve_deep_research_llm_client(&capability_config(), &options, "capability-check")
            .err()
            .expect("DeepResearch requires tool calling");

    assert!(error.contains("must support tool calling"));
}

#[test]
fn structured_support_filter_never_weakens_the_primary_capability() {
    assert!(structured_support_satisfies(
        NativeStructuredSupport::None,
        NativeStructuredSupport::None,
    ));
    assert!(structured_support_satisfies(
        NativeStructuredSupport::ForcedTool,
        NativeStructuredSupport::None,
    ));
    assert!(structured_support_satisfies(
        NativeStructuredSupport::JsonSchema,
        NativeStructuredSupport::ForcedTool,
    ));
    assert!(!structured_support_satisfies(
        NativeStructuredSupport::None,
        NativeStructuredSupport::ForcedTool,
    ));
    assert!(!structured_support_satisfies(
        NativeStructuredSupport::ForcedTool,
        NativeStructuredSupport::JsonSchema,
    ));
}

#[test]
fn candidate_deadline_is_derived_from_shared_engine_budgets() {
    let usable_budget = DEFAULT_PLANNER_ATTEMPT_TIMEOUT_MS
        .min(DEFAULT_REPORT_ATTEMPT_TIMEOUT_MS)
        .saturating_sub(DEFAULT_DURABLE_GENERATION_GRACE_MS);

    for candidate_count in 1..=usize::from(DEFAULT_MAX_CONCURRENT_GENERATIONS) {
        assert_eq!(
            deep_research_candidate_timeout(
                Duration::from_millis(
                    DEFAULT_PLANNER_ATTEMPT_TIMEOUT_MS.min(DEFAULT_REPORT_ATTEMPT_TIMEOUT_MS),
                ),
                candidate_count,
            )
            .as_millis(),
            u128::from(usable_budget / candidate_count as u64),
        );
    }
}

#[test]
fn caller_timeout_rebudgets_every_candidate_without_changing_capabilities() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let first: Arc<dyn LlmClient> =
        Arc::new(ControlledClient::succeeding("first", Arc::clone(&calls)));
    let second: Arc<dyn LlmClient> = Arc::new(ControlledClient::succeeding("second", calls));
    let client = failover_client(vec![first, second]);
    let active_timeout = Duration::from_millis(DEFAULT_REPORT_ATTEMPT_TIMEOUT_MS / 2);

    let rebudgeted = client
        .with_active_generation_timeout(active_timeout)
        .expect("composite client supports active timeout rebudgeting");

    assert_eq!(
        rebudgeted.native_structured_support(),
        client.native_structured_support()
    );
    assert_eq!(
        deep_research_candidate_timeout(active_timeout, 2),
        Duration::from_millis(
            (u64::try_from(active_timeout.as_millis()).expect("test timeout fits u64")
                - DEFAULT_DURABLE_GENERATION_GRACE_MS)
                / 2,
        ),
    );
}

#[test]
fn fallback_order_covers_distinct_sources_then_domains_before_repeats() {
    let labels = ["candidate-0", "candidate-1", "candidate-2", "candidate-3"];
    let mut routes: Vec<ModelRoute> = labels
        .into_iter()
        .map(|label| ModelRoute::new(ModelSource::Config, format!("provider/{label}")))
        .collect::<anyhow::Result<_>>()
        .expect("candidate routes");
    routes[3] =
        ModelRoute::new(ModelSource::Codex, "candidate-3").expect("account candidate route");
    let primary_domain = ModelFailureDomain::Provider("provider-a".to_string());
    let alternate_domain = ModelFailureDomain::Endpoint {
        scheme: "https".to_string(),
        host: "endpoint-b.invalid".to_string(),
        port: Some(443),
    };
    let candidates = vec![
        ResearchCandidateSpec {
            route: routes[0].clone(),
            failure_domain: primary_domain.clone(),
        },
        ResearchCandidateSpec {
            route: routes[1].clone(),
            failure_domain: alternate_domain.clone(),
        },
        ResearchCandidateSpec {
            route: routes[2].clone(),
            failure_domain: alternate_domain,
        },
        ResearchCandidateSpec {
            route: routes[3].clone(),
            failure_domain: ModelFailureDomain::Account(ModelSource::Codex),
        },
    ];

    let ordered = prioritize_candidate_diversity(
        Some(ModelSource::Config),
        Some(&primary_domain),
        candidates,
    );

    for (candidate, expected) in ordered.iter().zip([3, 1, 0, 2]) {
        assert_eq!(candidate.route, routes[expected]);
    }
}

#[tokio::test]
async fn completion_failover_remembers_the_last_successful_candidate() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let primary: Arc<dyn LlmClient> = Arc::new(ControlledClient::failing(
        "primary",
        "transport unavailable",
        Arc::clone(&calls),
    ));
    let fallback: Arc<dyn LlmClient> =
        Arc::new(ControlledClient::succeeding("fallback", Arc::clone(&calls)));
    let client = failover_client(vec![primary, fallback]);

    client.complete(&[], None, &[]).await.expect("fallback");
    client
        .complete(&[], None, &[])
        .await
        .expect("remembered fallback");

    assert_eq!(
        calls.lock().expect("call log").as_slice(),
        ["primary:complete", "fallback:complete", "fallback:complete"]
    );
}

#[tokio::test]
async fn candidate_deadline_allows_the_next_failure_domain_to_take_over() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let slow: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("slow", Arc::clone(&calls))
            .with_response_delay(Duration::from_millis(50)),
    );
    let fallback: Arc<dyn LlmClient> =
        Arc::new(ControlledClient::succeeding("fallback", Arc::clone(&calls)));
    let client =
        DeepResearchFailoverClient::new(vec![slow, fallback], 0, Duration::from_millis(40))
            .expect("bounded candidates");

    client
        .complete(&[], None, &[])
        .await
        .expect("fallback after candidate deadline");

    assert_eq!(
        calls.lock().expect("call log").as_slice(),
        ["slow:complete", "fallback:complete"]
    );
}

#[tokio::test]
async fn streaming_failover_waits_for_done_before_remembering_a_candidate() {
    for method in ["streaming", "streaming_structured"] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let slow: Arc<dyn LlmClient> = Arc::new(
            ControlledClient::succeeding("slow", Arc::clone(&calls))
                .with_streaming_only_transport()
                .with_stream_completion_delay(Duration::from_millis(50)),
        );
        let fallback: Arc<dyn LlmClient> = Arc::new(
            ControlledClient::succeeding("fallback", Arc::clone(&calls))
                .with_streaming_only_transport(),
        );
        let client =
            DeepResearchFailoverClient::new(vec![slow, fallback], 0, Duration::from_millis(40))
                .expect("bounded candidates");

        let mut stream = match method {
            "streaming" => client
                .complete_streaming(&[], None, &[], CancellationToken::new())
                .await
                .expect("fallback after an incomplete streaming attempt"),
            "streaming_structured" => client
                .complete_streaming_structured(
                    &[],
                    None,
                    &[],
                    &StructuredDirective::default(),
                    CancellationToken::new(),
                )
                .await
                .expect("structured fallback after an incomplete streaming attempt"),
            _ => unreachable!("test method is exhaustive"),
        };
        let response = match stream.recv().await {
            Some(StreamEvent::Done(response)) => response,
            event => panic!("expected buffered terminal response, got {event:?}"),
        };

        assert_eq!(response.text(), "fallback");
        assert_eq!(
            calls.lock().expect("call log").as_slice(),
            [format!("slow:{method}"), format!("fallback:{method}")]
        );
    }
}

fn title_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["title"],
        "additionalProperties": false,
        "properties": {
            "title": { "type": "string", "minLength": 1 }
        }
    })
}

fn scripted_tool_stream(title_json: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolUseStart {
            id: "tool-1".to_string(),
            name: "emit_report".to_string(),
        },
        StreamEvent::ToolUseInputDelta {
            id: Some("tool-1".to_string()),
            delta: title_json[..title_json.len() / 2].to_string(),
        },
        StreamEvent::ToolUseInputDelta {
            id: Some("tool-1".to_string()),
            delta: title_json[title_json.len() / 2..].to_string(),
        },
    ]
}

fn scripted_text_stream(title_json: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::TextDelta(title_json[..title_json.len() / 2].to_string()),
        StreamEvent::TextDelta(title_json[title_json.len() / 2..].to_string()),
    ]
}

async fn generate_scripted_title(client: &dyn LlmClient) -> serde_json::Value {
    structured::generate_streaming(
        client,
        &StructuredRequest {
            prompt: "Create a title".to_string(),
            system: None,
            schema: title_schema(),
            schema_name: "report".to_string(),
            schema_description: None,
            mode: StructuredMode::Auto,
            max_repair_attempts: 0,
        },
        Box::new(|_| {}),
    )
    .await
    .expect("schema-complete buffered stream")
    .object
}

#[tokio::test]
async fn structured_stream_accepts_a_schema_complete_object_without_done() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let primary: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("primary", Arc::clone(&calls))
            .with_streaming_only_transport()
            .with_scripted_stream(scripted_tool_stream(r#"{"title":"complete"}"#)),
    );
    let fallback: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("fallback", Arc::clone(&calls))
            .with_streaming_only_transport(),
    );
    let client = failover_client(vec![primary, fallback]);

    let object = generate_scripted_title(&client).await;

    assert_eq!(object, serde_json::json!({"title": "complete"}));
    let mut observed = calls.lock().expect("call log").clone();
    observed.sort();
    assert_eq!(
        observed,
        [
            "fallback:streaming_structured".to_string(),
            "primary:streaming_structured".to_string(),
        ]
    );
}

#[tokio::test]
async fn prompt_fallback_stream_accepts_a_schema_complete_object_without_done() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let primary: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("primary", Arc::clone(&calls))
            .with_capabilities(NativeStructuredSupport::None, 1)
            .with_streaming_only_transport()
            .with_scripted_stream(scripted_text_stream(r#"{"title":"complete"}"#)),
    );
    let fallback: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("fallback", Arc::clone(&calls))
            .with_capabilities(NativeStructuredSupport::None, 1)
            .with_streaming_only_transport(),
    );
    let client = failover_client(vec![primary, fallback]);

    let object = generate_scripted_title(&client).await;

    assert_eq!(object, serde_json::json!({"title": "complete"}));
    let mut observed = calls.lock().expect("call log").clone();
    observed.sort();
    assert_eq!(
        observed,
        [
            "fallback:streaming_structured".to_string(),
            "primary:streaming_structured".to_string(),
        ]
    );
}

#[tokio::test]
async fn structured_stream_never_admits_a_schema_invalid_object_without_done() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let invalid: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("invalid", Arc::clone(&calls))
            .with_streaming_only_transport()
            .with_scripted_stream(scripted_tool_stream(r#"{"title":7}"#)),
    );
    let valid: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("valid", Arc::clone(&calls))
            .with_streaming_only_transport()
            .with_scripted_stream(scripted_tool_stream(r#"{"title":"fallback"}"#)),
    );
    let client = failover_client(vec![invalid, valid]);

    let object = generate_scripted_title(&client).await;

    assert_eq!(object, serde_json::json!({"title": "fallback"}));
    assert_eq!(
        calls.lock().expect("call log").as_slice(),
        ["invalid:streaming_structured", "valid:streaming_structured"]
    );
}

#[tokio::test]
async fn streaming_progress_uses_the_global_budget_without_disabling_stall_failover() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let progressing: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("progressing", Arc::clone(&calls))
            .with_stream_completion_delay(Duration::from_millis(160))
            .with_stream_progress_interval(Duration::from_millis(50)),
    );
    let fallback: Arc<dyn LlmClient> =
        Arc::new(ControlledClient::succeeding("fallback", Arc::clone(&calls)));
    let client =
        DeepResearchFailoverClient::new(vec![progressing, fallback], 0, Duration::from_millis(200))
            .expect("bounded candidates");
    assert_eq!(client.candidate_idle_timeout, Duration::from_millis(100));

    let mut stream = client
        .complete_streaming(&[], None, &[], CancellationToken::new())
        .await
        .expect("progressing candidate completes within the global deadline");
    let mut response = None;
    while let Some(event) = stream.recv().await {
        if let StreamEvent::Done(done) = event {
            response = Some(done);
            break;
        }
    }

    assert_eq!(response.expect("terminal response").text(), "progressing");
    assert_eq!(
        calls.lock().expect("call log").as_slice(),
        ["progressing:streaming"]
    );
}

#[tokio::test]
async fn a_stream_that_exhausts_the_global_deadline_rotates_the_next_retry() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let unfinished: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("unfinished", Arc::clone(&calls))
            .with_stream_completion_delay(Duration::from_millis(500))
            .with_stream_progress_interval(Duration::from_millis(50)),
    );
    let fallback: Arc<dyn LlmClient> =
        Arc::new(ControlledClient::succeeding("fallback", Arc::clone(&calls)));
    let client =
        DeepResearchFailoverClient::new(vec![unfinished, fallback], 0, Duration::from_millis(200))
            .expect("bounded candidates");

    client
        .complete_streaming(&[], None, &[], CancellationToken::new())
        .await
        .expect_err("unfinished stream exhausts the first global deadline");
    let mut stream = client
        .complete_streaming(&[], None, &[], CancellationToken::new())
        .await
        .expect("the next invocation starts from the rotated fallback");
    let response = match stream.recv().await {
        Some(StreamEvent::Done(response)) => response,
        event => panic!("expected fallback terminal response, got {event:?}"),
    };

    assert_eq!(response.text(), "fallback");
    assert_eq!(
        calls.lock().expect("call log").as_slice(),
        ["unfinished:streaming", "fallback:streaming"]
    );
}

#[tokio::test]
async fn every_llm_call_interface_uses_the_same_error_only_failover_policy() {
    for method in ["streaming", "structured", "streaming_structured"] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let primary: Arc<dyn LlmClient> = Arc::new(ControlledClient::failing(
            "primary",
            "request failed",
            Arc::clone(&calls),
        ));
        let fallback: Arc<dyn LlmClient> =
            Arc::new(ControlledClient::succeeding("fallback", Arc::clone(&calls)));
        let client = failover_client(vec![primary, fallback]);
        match method {
            "streaming" => {
                let mut stream = client
                    .complete_streaming(&[], None, &[], CancellationToken::new())
                    .await
                    .expect("streaming fallback");
                assert!(matches!(stream.recv().await, Some(StreamEvent::Done(_))));
            }
            "structured" => {
                client
                    .complete_structured(&[], None, &[], &StructuredDirective::default())
                    .await
                    .expect("structured fallback");
            }
            "streaming_structured" => {
                let mut stream = client
                    .complete_streaming_structured(
                        &[],
                        None,
                        &[],
                        &StructuredDirective::default(),
                        CancellationToken::new(),
                    )
                    .await
                    .expect("structured streaming fallback");
                assert!(matches!(stream.recv().await, Some(StreamEvent::Done(_))));
            }
            _ => unreachable!("test method is exhaustive"),
        }
        let mut observed = calls.lock().expect("call log").clone();
        observed.sort();
        let mut expected = match method {
            "streaming_structured" => vec![
                "primary:structured".to_string(),
                "fallback:streaming_structured".to_string(),
            ],
            _ => vec![format!("primary:{method}"), format!("fallback:{method}")],
        };
        expected.sort();
        assert_eq!(observed, expected);
    }
}

#[tokio::test]
async fn one_distinct_non_streaming_candidate_uses_the_full_deadline_once() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let stream_broken: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("stream-broken", Arc::clone(&calls))
            .with_stream_completion_delay(Duration::from_millis(500))
            .with_stream_progress_interval(Duration::from_millis(10)),
    );
    let client =
        DeepResearchFailoverClient::new(vec![stream_broken], 0, Duration::from_millis(100))
            .expect("one bounded candidate");

    let mut stream = client
        .complete_streaming_structured(
            &[],
            None,
            &[],
            &StructuredDirective::default(),
            CancellationToken::new(),
        )
        .await
        .expect("non-streaming structured transport recovers the request");
    let response = match stream.recv().await {
        Some(StreamEvent::Done(response)) => response,
        event => panic!("expected replayed terminal response, got {event:?}"),
    };

    assert_eq!(response.text(), "stream-broken");
    assert_eq!(
        calls.lock().expect("call log").as_slice(),
        ["stream-broken:structured"]
    );
}

#[tokio::test]
async fn streaming_only_client_uses_the_complete_streaming_deadline() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let streaming_only: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("streaming-only", Arc::clone(&calls))
            .with_streaming_only_transport()
            .with_stream_completion_delay(Duration::from_millis(75))
            .with_stream_progress_interval(Duration::from_millis(20)),
    );
    let client =
        DeepResearchFailoverClient::new(vec![streaming_only], 0, Duration::from_millis(100))
            .expect("one bounded candidate");

    let mut stream = client
        .complete_streaming_structured(
            &[],
            None,
            &[],
            &StructuredDirective::default(),
            CancellationToken::new(),
        )
        .await
        .expect("streaming-only client keeps the full transport budget");
    let mut response = None;
    while let Some(event) = stream.recv().await {
        if let StreamEvent::Done(done) = event {
            response = Some(done);
            break;
        }
    }

    assert_eq!(
        response.expect("terminal streaming response").text(),
        "streaming-only"
    );
    assert_eq!(
        calls.lock().expect("call log").as_slice(),
        ["streaming-only:streaming_structured"]
    );
}

#[tokio::test]
async fn distinct_non_streaming_transport_is_hedged_against_an_independent_stream() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let distinct_non_streaming: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("distinct-non-streaming", Arc::clone(&calls))
            .with_response_delay(Duration::from_millis(75)),
    );
    let streaming_only: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("streaming-only", Arc::clone(&calls))
            .with_streaming_only_transport()
            .with_stream_completion_delay(Duration::from_millis(500))
            .with_stream_progress_interval(Duration::from_millis(10)),
    );
    let client = DeepResearchFailoverClient::new(
        vec![distinct_non_streaming, streaming_only],
        0,
        Duration::from_millis(100),
    )
    .expect("two bounded candidates");

    let mut stream = client
        .complete_streaming_structured(
            &[],
            None,
            &[],
            &StructuredDirective::default(),
            CancellationToken::new(),
        )
        .await
        .expect("the independent non-streaming hedge completes first");
    let response = match stream.recv().await {
        Some(StreamEvent::Done(response)) => response,
        event => panic!("expected replayed terminal response, got {event:?}"),
    };

    assert_eq!(response.text(), "distinct-non-streaming");
    let mut observed = calls.lock().expect("call log").clone();
    observed.sort();
    assert_eq!(
        observed,
        [
            "distinct-non-streaming:structured".to_string(),
            "streaming-only:streaming_structured".to_string()
        ]
    );
}

#[tokio::test]
async fn structured_stream_candidates_race_with_the_complete_global_deadline() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let stalled: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("stalled", Arc::clone(&calls))
            .with_streaming_only_transport()
            .with_stream_completion_delay(Duration::from_millis(500)),
    );
    let completable: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("completable", Arc::clone(&calls))
            .with_streaming_only_transport()
            .with_stream_completion_delay(Duration::from_millis(160)),
    );
    let client =
        DeepResearchFailoverClient::new(vec![stalled, completable], 0, Duration::from_millis(200))
            .expect("two bounded candidates");

    let mut stream = client
        .complete_streaming_structured(
            &[],
            None,
            &[],
            &StructuredDirective::default(),
            CancellationToken::new(),
        )
        .await
        .expect("one concurrent candidate completes within the shared deadline");
    let response = match stream.recv().await {
        Some(StreamEvent::Done(response)) => response,
        event => panic!("expected replayed terminal response, got {event:?}"),
    };

    assert_eq!(response.text(), "completable");
    let mut observed = calls.lock().expect("call log").clone();
    observed.sort();
    assert_eq!(
        observed,
        [
            "completable:streaming_structured".to_string(),
            "stalled:streaming_structured".to_string(),
        ]
    );
}

#[tokio::test]
async fn exhausted_failover_bounds_errors_without_echoing_provider_details() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let clients: Vec<Arc<dyn LlmClient>> = vec![
        Arc::new(ControlledClient::failing(
            "first",
            "credential sk-sensitive-first",
            Arc::clone(&calls),
        )),
        Arc::new(ControlledClient::failing(
            "second",
            "authorization bearer-sensitive-second",
            Arc::clone(&calls),
        )),
    ];
    let client = failover_client(clients);

    let error = client
        .complete(&[], None, &[])
        .await
        .expect_err("all candidates fail")
        .to_string();

    assert!(error.contains("2 capability-compatible candidate"));
    assert!(error.contains("provider_error=2"));
    for secret_fragment in ["sensitive", "credential", "authorization"] {
        assert!(!error.contains(secret_fragment));
    }
}

#[tokio::test]
async fn exhausted_stream_reports_a_safe_missing_terminal_class() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let unfinished: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("unfinished", calls)
            .with_stream_completion_delay(Duration::from_millis(500))
            .with_stream_progress_interval(Duration::from_millis(20)),
    );
    let client = DeepResearchFailoverClient::new(vec![unfinished], 0, Duration::from_millis(100))
        .expect("one bounded candidate");

    let error = client
        .complete_streaming(&[], None, &[], CancellationToken::new())
        .await
        .expect_err("progress without a terminal event must remain bounded")
        .to_string();

    assert!(error.contains("missing_terminal_event=1"));
    assert!(!error.contains("unfinished"));
}

#[test]
fn failover_reports_only_capabilities_shared_by_every_candidate() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let stronger: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("stronger", Arc::clone(&calls))
            .with_capabilities(NativeStructuredSupport::JsonSchema, 4),
    );
    let conservative: Arc<dyn LlmClient> = Arc::new(
        ControlledClient::succeeding("conservative", calls)
            .with_capabilities(NativeStructuredSupport::ForcedTool, 2),
    );
    let client = failover_client(vec![stronger, conservative]);

    assert_eq!(
        client.native_structured_support(),
        NativeStructuredSupport::ForcedTool
    );
    assert_eq!(
        client
            .model_generation_concurrency()
            .max_concurrency()
            .get(),
        2
    );
}

#[test]
fn failover_forks_only_when_every_candidate_can_fork() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let forkable: Arc<dyn LlmClient> =
        Arc::new(ControlledClient::succeeding("forkable", Arc::clone(&calls)).with_forking());
    let shared_only: Arc<dyn LlmClient> =
        Arc::new(ControlledClient::succeeding("shared", Arc::clone(&calls)));
    let mixed = failover_client(vec![Arc::clone(&forkable), shared_only]);
    assert!(mixed.fork_for_session("child").is_none());

    let all_forkable = failover_client(vec![Arc::clone(&forkable), forkable]);
    assert!(all_forkable.fork_for_session("child").is_some());
}
