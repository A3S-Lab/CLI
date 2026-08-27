use std::collections::HashSet;
use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use a3s_code_core::llm::structured::{
    is_complete_streamed_value, NativeStructuredSupport, ResponseFormat, StructuredDirective,
};
use a3s_code_core::llm::{
    create_client_with_config, LlmConfig, ModelGenerationConcurrency, StreamEvent, ToolDefinition,
};
use a3s_code_core::{CodeConfig, LlmClient, LlmResponse, Message, SessionOptions};
use a3s_deep_research::engine::{
    DEFAULT_DURABLE_GENERATION_GRACE_MS, DEFAULT_MAX_CONCURRENT_GENERATIONS,
    DEFAULT_PLANNER_ATTEMPT_TIMEOUT_MS, DEFAULT_REPORT_ATTEMPT_TIMEOUT_MS,
};
use async_trait::async_trait;
use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::account_providers::AccountProvider;
use crate::model::route::{ModelRoute, ModelSource};

pub(crate) fn resolve_config_llm_client(
    code_config: &CodeConfig,
    options: &SessionOptions,
    session_id: &str,
) -> Result<Arc<dyn LlmClient>, String> {
    let model_ref = options
        .model
        .as_deref()
        .or(code_config.default_model.as_deref())
        .ok_or_else(|| "default_model must be set in 'provider/model' format".to_string())?;
    let route = model_ref
        .parse::<ModelRoute>()
        .map_err(|error| format!("invalid model route `{model_ref}`: {error}"))?;
    match route.source {
        ModelSource::Config => {
            if route.model == model_ref {
                prepare_config_llm_config(code_config, options, session_id)
                    .map(create_client_with_config)
            } else {
                let mut routed = options.clone();
                routed.model = Some(route.model);
                prepare_config_llm_config(code_config, &routed, session_id)
                    .map(create_client_with_config)
            }
        }
        ModelSource::Claude | ModelSource::Codex | ModelSource::Kimi | ModelSource::CodeBuddy => {
            let provider = route
                .source
                .account_provider()
                .ok_or_else(|| format!("{} is not an account provider", route.source.label()))?;
            provider
                .client(&route.model, session_id)
                .map_err(|error| format!("{} account unavailable: {error}", provider.label()))
        }
        ModelSource::OsGateway => {
            Err("A3S OS gateway routes require a signed-in interactive session".to_string())
        }
    }
}

pub(crate) fn resolve_session_llm_client(
    code_config: &CodeConfig,
    options: &SessionOptions,
    session_id: &str,
) -> Result<Arc<dyn LlmClient>, String> {
    match options.llm_client.as_ref() {
        Some(client) => Ok(Arc::clone(client)),
        None => resolve_config_llm_client(code_config, options, session_id),
    }
}

/// Resolve the preferred model plus capability-compatible fallbacks for one
/// DeepResearch run.
///
/// Candidate selection is deliberately content-blind. It uses only typed
/// tool-call capability, provider-native structured-output support, stable
/// registry order, and provider/endpoint failure domains. Non-streaming calls
/// receive an equal slice of the usable stage budget. Ordinary streaming calls
/// use that slice only as a no-progress timeout. Structured streaming races the
/// capability-compatible transports under one shared stage deadline so a
/// buffered but healthy response is never aborted merely to start another
/// candidate after half of the same deadline has already elapsed.
pub(crate) fn resolve_deep_research_llm_client(
    code_config: &CodeConfig,
    options: &SessionOptions,
    session_id: &str,
) -> Result<Arc<dyn LlmClient>, String> {
    let primary = resolve_session_llm_client(code_config, options, session_id)?;
    let selected_route = (options.llm_client.is_none())
        .then(|| selected_model_route(code_config, options))
        .flatten();
    let primary_model = selected_route
        .as_ref()
        .and_then(|route| selected_configured_model(code_config, route));
    if primary_model.as_ref().is_some_and(|model| !model.tool_call) {
        return Err("the selected DeepResearch model must support tool calling".to_string());
    }

    let required_structured_support = primary.native_structured_support();
    let primary_domain = primary_model
        .as_ref()
        .map(|model| model.failure_domain.clone())
        .or_else(|| {
            selected_route.as_ref().and_then(|route| {
                route
                    .source
                    .account_provider()
                    .map(|_| ModelFailureDomain::Account(route.source))
            })
        });
    let mut candidate_specs = Vec::new();

    for provider in &code_config.providers {
        for model in &provider.models {
            if !model.tool_call {
                continue;
            }
            let Ok(route) = ModelRoute::new(
                ModelSource::Config,
                format!("{}/{}", provider.name, model.id),
            ) else {
                continue;
            };
            if selected_route.as_ref() == Some(&route) {
                continue;
            }
            candidate_specs.push(ResearchCandidateSpec {
                route,
                failure_domain: configured_failure_domain(provider, model),
            });
        }
    }

    for account_provider in AccountProvider::ALL {
        if !account_provider.is_available() {
            continue;
        }
        let source = ModelSource::from_account_provider(account_provider);
        for model in account_provider.local_models() {
            let model = account_provider.canonical_model(&model);
            let Ok(route) = ModelRoute::new(source, model) else {
                continue;
            };
            if selected_route.as_ref() == Some(&route) {
                continue;
            }
            candidate_specs.push(ResearchCandidateSpec {
                route,
                failure_domain: ModelFailureDomain::Account(source),
            });
        }
    }

    let primary_source = selected_route.as_ref().map(|route| route.source);
    let candidate_specs =
        prioritize_candidate_diversity(primary_source, primary_domain.as_ref(), candidate_specs);
    let max_candidates = usize::from(DEFAULT_MAX_CONCURRENT_GENERATIONS).max(1);
    let mut clients = Vec::with_capacity(max_candidates);
    clients.push(primary);
    for candidate in candidate_specs {
        if clients.len() >= max_candidates {
            break;
        }
        let Some(client) =
            resolve_research_candidate(code_config, options, session_id, &candidate.route)
        else {
            continue;
        };
        if structured_support_satisfies(
            client.native_structured_support(),
            required_structured_support,
        ) {
            clients.push(client);
        }
    }
    let default_active_timeout = Duration::from_millis(
        DEFAULT_PLANNER_ATTEMPT_TIMEOUT_MS.min(DEFAULT_REPORT_ATTEMPT_TIMEOUT_MS),
    );
    let failover = DeepResearchFailoverClient::new(clients, 0, default_active_timeout)
        .ok_or_else(|| "DeepResearch model candidate set is empty".to_string())?;
    Ok(Arc::new(failover))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ModelFailureDomain {
    Endpoint {
        scheme: String,
        host: String,
        port: Option<u16>,
    },
    Provider(String),
    Account(ModelSource),
}

struct SelectedConfiguredModel {
    failure_domain: ModelFailureDomain,
    tool_call: bool,
}

struct ResearchCandidateSpec {
    route: ModelRoute,
    failure_domain: ModelFailureDomain,
}

fn selected_model_route(code_config: &CodeConfig, options: &SessionOptions) -> Option<ModelRoute> {
    options
        .model
        .as_deref()
        .or(code_config.default_model.as_deref())?
        .parse()
        .ok()
}

fn selected_configured_model(
    code_config: &CodeConfig,
    route: &ModelRoute,
) -> Option<SelectedConfiguredModel> {
    if route.source != ModelSource::Config {
        return None;
    }
    let (provider_name, model_id) = route.model.split_once('/')?;
    let provider = code_config
        .providers
        .iter()
        .find(|provider| provider.name == provider_name)?;
    let model = provider.find_model(model_id)?;
    Some(SelectedConfiguredModel {
        failure_domain: configured_failure_domain(provider, model),
        tool_call: model.tool_call,
    })
}

fn resolve_research_candidate(
    code_config: &CodeConfig,
    options: &SessionOptions,
    session_id: &str,
    route: &ModelRoute,
) -> Option<Arc<dyn LlmClient>> {
    match route.source {
        ModelSource::Config => {
            let mut candidate_options = options.clone();
            candidate_options.llm_client = None;
            candidate_options.model = Some(route.model.clone());
            prepare_config_llm_config(code_config, &candidate_options, session_id)
                .ok()
                .map(create_client_with_config)
        }
        ModelSource::Claude | ModelSource::Codex | ModelSource::Kimi | ModelSource::CodeBuddy => {
            route
                .source
                .account_provider()?
                .client(&route.model, session_id)
                .ok()
        }
        ModelSource::OsGateway => None,
    }
}

fn configured_failure_domain(
    provider: &a3s_code_core::config::ProviderConfig,
    model: &a3s_code_core::config::ModelConfig,
) -> ModelFailureDomain {
    let provider_name = provider.name.trim().to_ascii_lowercase();
    let Some(base_url) = provider.get_base_url(model) else {
        return ModelFailureDomain::Provider(provider_name);
    };
    let Ok(endpoint) = Url::parse(base_url) else {
        return ModelFailureDomain::Provider(provider_name);
    };
    let Some(host) = endpoint.host_str() else {
        return ModelFailureDomain::Provider(provider_name);
    };
    ModelFailureDomain::Endpoint {
        scheme: endpoint.scheme().to_ascii_lowercase(),
        host: host.to_ascii_lowercase(),
        port: endpoint.port_or_known_default(),
    }
}

fn structured_support_satisfies(
    candidate: NativeStructuredSupport,
    required: NativeStructuredSupport,
) -> bool {
    // Keep only candidates that can preserve the primary client's native
    // enforcement contract.  `ForcedTool` and `JsonObject` are incomparable:
    // the former can force a tool call while the latter can send a JSON-object
    // response format but explicitly cannot send `tool_choice`.  Treating one
    // as satisfying the other would make the failover aggregate advertise a
    // mode that one of its transports rejects.
    matches!(
        (candidate, required),
        (_, NativeStructuredSupport::None)
            | (NativeStructuredSupport::JsonSchema, _)
            | (
                NativeStructuredSupport::ForcedTool,
                NativeStructuredSupport::ForcedTool
            )
            | (
                NativeStructuredSupport::JsonObject,
                NativeStructuredSupport::JsonObject
            )
    )
}

fn weaker_structured_support(
    left: NativeStructuredSupport,
    right: NativeStructuredSupport,
) -> NativeStructuredSupport {
    match (left, right) {
        (NativeStructuredSupport::None, _) | (_, NativeStructuredSupport::None) => {
            NativeStructuredSupport::None
        }
        (NativeStructuredSupport::JsonSchema, support)
        | (support, NativeStructuredSupport::JsonSchema) => support,
        (NativeStructuredSupport::ForcedTool, NativeStructuredSupport::ForcedTool) => {
            NativeStructuredSupport::ForcedTool
        }
        (NativeStructuredSupport::JsonObject, NativeStructuredSupport::JsonObject) => {
            NativeStructuredSupport::JsonObject
        }
        // There is no shared native request shape: `tool_choice` is rejected
        // by the JSON-object-only provider and `response_format` is not
        // guaranteed by a tool-only provider.  The structured engine will
        // therefore use its prompt+schema path for this mixed set.
        (NativeStructuredSupport::ForcedTool, NativeStructuredSupport::JsonObject)
        | (NativeStructuredSupport::JsonObject, NativeStructuredSupport::ForcedTool) => {
            NativeStructuredSupport::None
        }
    }
}

fn prioritize_candidate_diversity(
    primary_source: Option<ModelSource>,
    primary_domain: Option<&ModelFailureDomain>,
    candidates: Vec<ResearchCandidateSpec>,
) -> Vec<ResearchCandidateSpec> {
    let mut seen_sources = HashSet::new();
    if let Some(primary_source) = primary_source {
        seen_sources.insert(primary_source);
    }
    let mut seen_domains = HashSet::new();
    if let Some(primary_domain) = primary_domain {
        seen_domains.insert(primary_domain.clone());
    }
    let mut source_distinct = Vec::new();
    let mut domain_distinct = Vec::new();
    let mut repeated = Vec::new();
    for candidate in candidates {
        if seen_sources.insert(candidate.route.source) {
            seen_domains.insert(candidate.failure_domain.clone());
            source_distinct.push(candidate);
        } else if seen_domains.insert(candidate.failure_domain.clone()) {
            domain_distinct.push(candidate);
        } else {
            repeated.push(candidate);
        }
    }
    source_distinct.extend(domain_distinct);
    source_distinct.extend(repeated);
    source_distinct
}

fn deep_research_candidate_timeout(
    active_generation_timeout: Duration,
    candidate_count: usize,
) -> Duration {
    let usable_budget_ms = u64::try_from(
        deep_research_usable_generation_timeout(active_generation_timeout).as_millis(),
    )
    .unwrap_or(u64::MAX);
    let candidate_count = u64::try_from(candidate_count).unwrap_or(u64::MAX).max(1);
    Duration::from_millis((usable_budget_ms / candidate_count).max(1))
}

fn deep_research_usable_generation_timeout(active_generation_timeout: Duration) -> Duration {
    let active_timeout_ms =
        u64::try_from(active_generation_timeout.as_millis()).unwrap_or(u64::MAX);
    let usable_budget_ms = if active_timeout_ms > DEFAULT_DURABLE_GENERATION_GRACE_MS {
        active_timeout_ms - DEFAULT_DURABLE_GENERATION_GRACE_MS
    } else {
        active_timeout_ms
    };
    Duration::from_millis(usable_budget_ms.max(1))
}

struct DeepResearchFailoverClient {
    clients: Vec<Arc<dyn LlmClient>>,
    preferred: AtomicUsize,
    structured_support: NativeStructuredSupport,
    generation_concurrency: ModelGenerationConcurrency,
    active_generation_timeout: Duration,
    candidate_idle_timeout: Duration,
    usable_generation_timeout: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CandidateFailureKind {
    ProviderError,
    RequestStall,
    StreamStartStall,
    StreamIdle,
    PrematureClose,
    GlobalDeadline,
    MissingTerminalEvent,
}

impl CandidateFailureKind {
    const ALL: [Self; 7] = [
        Self::ProviderError,
        Self::RequestStall,
        Self::StreamStartStall,
        Self::StreamIdle,
        Self::PrematureClose,
        Self::GlobalDeadline,
        Self::MissingTerminalEvent,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::ProviderError => "provider_error",
            Self::RequestStall => "request_stall",
            Self::StreamStartStall => "stream_start_stall",
            Self::StreamIdle => "stream_idle",
            Self::PrematureClose => "premature_close",
            Self::GlobalDeadline => "global_deadline",
            Self::MissingTerminalEvent => "missing_terminal_event",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamAttemptError {
    Cancelled,
    Failed(CandidateFailureKind),
}

enum StructuredCandidateCompletion {
    Streaming(Result<Vec<StreamEvent>, StreamAttemptError>),
    NonStreaming(anyhow::Result<Box<LlmResponse>>),
}

type StructuredCandidateFuture =
    Pin<Box<dyn Future<Output = (usize, usize, StructuredCandidateCompletion)> + Send + 'static>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StructuredStreamChannel {
    Text,
    ToolInput,
}

#[derive(Clone, Debug)]
struct StructuredStreamCompletion {
    channel: StructuredStreamChannel,
    response_schema: serde_json::Value,
    buffer: String,
}

impl StructuredStreamCompletion {
    fn from_request(tools: &[ToolDefinition], directive: &StructuredDirective) -> Option<Self> {
        let response_schema = directive.validation_schema.clone().or_else(|| {
            let ResponseFormat::JsonSchema { schema, .. } = directive.response_format.as_ref()?
            else {
                return None;
            };
            Some(schema.clone())
        })?;
        if let Some(forced_tool) = directive.force_tool.as_deref() {
            tools.iter().find(|tool| tool.name == forced_tool)?;
            return Some(Self {
                channel: StructuredStreamChannel::ToolInput,
                response_schema,
                buffer: String::new(),
            });
        }
        Some(Self {
            channel: StructuredStreamChannel::Text,
            response_schema,
            buffer: String::new(),
        })
    }

    fn observe(&mut self, event: &StreamEvent) -> bool {
        let delta = match (self.channel, event) {
            (StructuredStreamChannel::Text, StreamEvent::TextDelta(delta))
            | (StructuredStreamChannel::ToolInput, StreamEvent::ToolUseInputDelta { delta, .. }) => {
                delta
            }
            _ => return false,
        };
        self.buffer.push_str(delta);
        (delta.contains('}') || delta.contains(']'))
            && is_complete_streamed_value(&self.buffer, &self.response_schema)
    }

    fn response_is_complete(&self, response: &LlmResponse) -> bool {
        match self.channel {
            StructuredStreamChannel::ToolInput => response.tool_calls().iter().any(|call| {
                serde_json::to_string(&call.args)
                    .ok()
                    .is_some_and(|raw| is_complete_streamed_value(&raw, &self.response_schema))
            }),
            StructuredStreamChannel::Text => {
                is_complete_streamed_value(&response.text(), &self.response_schema)
                    || response
                        .message
                        .reasoning_content
                        .as_deref()
                        .is_some_and(|raw| is_complete_streamed_value(raw, &self.response_schema))
            }
        }
    }
}

impl DeepResearchFailoverClient {
    fn new(
        clients: Vec<Arc<dyn LlmClient>>,
        preferred: usize,
        active_generation_timeout: Duration,
    ) -> Option<Self> {
        let structured_support = clients
            .iter()
            .map(|client| client.native_structured_support())
            .reduce(weaker_structured_support)?;
        let max_concurrency = clients
            .iter()
            .map(|client| {
                client
                    .model_generation_concurrency()
                    .max_concurrency()
                    .get()
            })
            .min()
            .and_then(NonZeroUsize::new)
            .unwrap_or(NonZeroUsize::MIN);
        Some(Self {
            preferred: AtomicUsize::new(preferred.min(clients.len() - 1)),
            candidate_idle_timeout: deep_research_candidate_timeout(
                active_generation_timeout,
                clients.len(),
            ),
            usable_generation_timeout: deep_research_usable_generation_timeout(
                active_generation_timeout,
            ),
            active_generation_timeout,
            clients,
            structured_support,
            generation_concurrency: ModelGenerationConcurrency::bounded(max_concurrency),
        })
    }

    fn ordered_indices(&self) -> Vec<usize> {
        let len = self.clients.len();
        let start = self.preferred.load(Ordering::Acquire).min(len - 1);
        let tail_len = len - start;
        (0..len)
            .map(|offset| {
                if offset < tail_len {
                    start + offset
                } else {
                    offset - tail_len
                }
            })
            .collect()
    }

    fn mark_success(&self, index: usize) {
        self.preferred.store(index, Ordering::Release);
    }

    fn mark_failure(&self, index: usize) {
        self.preferred
            .store((index + 1) % self.clients.len(), Ordering::Release);
    }

    fn exhausted_error(attempts: usize, failures: &[CandidateFailureKind]) -> anyhow::Error {
        let summary = CandidateFailureKind::ALL
            .into_iter()
            .filter_map(|kind| {
                let count = failures.iter().filter(|failure| **failure == kind).count();
                (count > 0).then(|| format!("{}={count}", kind.label()))
            })
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::anyhow!(
            "DeepResearch model request failed across {attempts} capability-compatible candidate(s); failure classes: {summary}"
        )
    }

    fn record_failure(attempt: usize, failure: CandidateFailureKind) {
        tracing::warn!(
            candidate_ordinal = attempt,
            failure_class = failure.label(),
            "DeepResearch capability-compatible model candidate failed"
        );
    }

    fn cancelled_error() -> anyhow::Error {
        anyhow::anyhow!("DeepResearch model request was cancelled")
    }
}

async fn collect_complete_stream(
    mut receiver: mpsc::Receiver<StreamEvent>,
    cancellation: CancellationToken,
    idle_timeout: Duration,
    deadline: tokio::time::Instant,
    mut structured_completion: Option<StructuredStreamCompletion>,
) -> Result<Vec<StreamEvent>, StreamAttemptError> {
    let mut events = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            let failure = if events.is_empty() {
                CandidateFailureKind::GlobalDeadline
            } else {
                CandidateFailureKind::MissingTerminalEvent
            };
            return Err(StreamAttemptError::Failed(failure));
        }
        let deadline_limited = remaining <= idle_timeout;
        let event = tokio::time::timeout(idle_timeout.min(remaining), async {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    Err(())
                }
                event = receiver.recv() => Ok(event),
            }
        })
        .await
        .map_err(|_| {
            let failure = if deadline_limited {
                if events.is_empty() {
                    CandidateFailureKind::GlobalDeadline
                } else {
                    CandidateFailureKind::MissingTerminalEvent
                }
            } else {
                CandidateFailureKind::StreamIdle
            };
            StreamAttemptError::Failed(failure)
        })?
        .map_err(|()| StreamAttemptError::Cancelled)?;
        let Some(event) = event else {
            return Err(StreamAttemptError::Failed(
                CandidateFailureKind::PrematureClose,
            ));
        };
        let terminal = matches!(event, StreamEvent::Done(_));
        let terminal_complete = match (&structured_completion, &event) {
            (Some(completion), StreamEvent::Done(response)) => {
                completion.response_is_complete(response)
            }
            (None, StreamEvent::Done(_)) => true,
            _ => false,
        };
        let structured_complete = structured_completion
            .as_mut()
            .is_some_and(|completion| completion.observe(&event));
        events.push(event);
        if terminal_complete || structured_complete {
            return Ok(events);
        }
        if terminal {
            return Err(StreamAttemptError::Failed(
                CandidateFailureKind::ProviderError,
            ));
        }
    }
}

async fn complete_stream_attempt<F>(
    start: F,
    cancellation: CancellationToken,
    idle_timeout: Duration,
    deadline: tokio::time::Instant,
    structured_completion: Option<StructuredStreamCompletion>,
) -> Result<Vec<StreamEvent>, StreamAttemptError>
where
    F: Future<Output = anyhow::Result<mpsc::Receiver<StreamEvent>>>,
{
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
        return Err(StreamAttemptError::Failed(
            CandidateFailureKind::GlobalDeadline,
        ));
    }
    let deadline_limited = remaining <= idle_timeout;
    let receiver = tokio::time::timeout(idle_timeout.min(remaining), async {
        tokio::select! {
            _ = cancellation.cancelled() => {
                None
            }
            result = start => Some(result),
        }
    })
    .await
    .map_err(|_| {
        StreamAttemptError::Failed(if deadline_limited {
            CandidateFailureKind::GlobalDeadline
        } else {
            CandidateFailureKind::StreamStartStall
        })
    })?
    .ok_or(StreamAttemptError::Cancelled)?
    .map_err(|_| StreamAttemptError::Failed(CandidateFailureKind::ProviderError))?;
    collect_complete_stream(
        receiver,
        cancellation,
        idle_timeout,
        deadline,
        structured_completion,
    )
    .await
}

fn replay_complete_stream(events: Vec<StreamEvent>) -> mpsc::Receiver<StreamEvent> {
    let (sender, receiver) = mpsc::channel(64);
    tokio::spawn(async move {
        for event in events {
            if sender.send(event).await.is_err() {
                break;
            }
        }
    });
    receiver
}

#[async_trait]
impl LlmClient for DeepResearchFailoverClient {
    fn model_generation_concurrency(&self) -> ModelGenerationConcurrency {
        self.generation_concurrency
    }

    fn fork_for_session(&self, session_id: &str) -> Option<Arc<dyn LlmClient>> {
        let mut forked = Vec::with_capacity(self.clients.len());
        for client in &self.clients {
            forked.push(client.fork_for_session(session_id)?);
        }
        let preferred = self
            .preferred
            .load(Ordering::Acquire)
            .min(forked.len().saturating_sub(1));
        DeepResearchFailoverClient::new(forked, preferred, self.active_generation_timeout)
            .map(|client| Arc::new(client) as Arc<dyn LlmClient>)
    }

    fn with_active_generation_timeout(&self, timeout: Duration) -> Option<Arc<dyn LlmClient>> {
        let preferred = self
            .preferred
            .load(Ordering::Acquire)
            .min(self.clients.len().saturating_sub(1));
        DeepResearchFailoverClient::new(self.clients.clone(), preferred, timeout)
            .map(|client| Arc::new(client) as Arc<dyn LlmClient>)
    }

    async fn complete(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let mut attempts = 0;
        let mut failures = Vec::new();
        for index in self.ordered_indices() {
            attempts += 1;
            let result = tokio::time::timeout(
                self.candidate_idle_timeout,
                self.clients[index].complete(messages, system, tools),
            )
            .await;
            let failure = match result {
                Ok(Ok(response)) => {
                    self.mark_success(index);
                    return Ok(response);
                }
                Ok(Err(_)) => CandidateFailureKind::ProviderError,
                Err(_) => CandidateFailureKind::RequestStall,
            };
            failures.push(failure);
            Self::record_failure(attempts, failure);
            self.mark_failure(index);
        }
        Err(Self::exhausted_error(attempts, &failures))
    }

    async fn complete_streaming(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
        cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        if cancel_token.is_cancelled() {
            return Err(Self::cancelled_error());
        }
        let mut attempts = 0;
        let mut failures = Vec::new();
        let deadline = tokio::time::Instant::now() + self.usable_generation_timeout;
        for index in self.ordered_indices() {
            if cancel_token.is_cancelled() {
                return Err(Self::cancelled_error());
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            attempts += 1;
            let attempt_cancel = cancel_token.child_token();
            let attempt = self.clients[index].complete_streaming(
                messages,
                system,
                tools,
                attempt_cancel.clone(),
            );
            match complete_stream_attempt(
                attempt,
                attempt_cancel.clone(),
                self.candidate_idle_timeout,
                deadline,
                None,
            )
            .await
            {
                Ok(events) => {
                    self.mark_success(index);
                    return Ok(replay_complete_stream(events));
                }
                Err(StreamAttemptError::Cancelled) => {
                    attempt_cancel.cancel();
                    return Err(Self::cancelled_error());
                }
                Err(StreamAttemptError::Failed(failure)) => {
                    attempt_cancel.cancel();
                    if cancel_token.is_cancelled() {
                        return Err(Self::cancelled_error());
                    }
                    failures.push(failure);
                    Self::record_failure(attempts, failure);
                    self.mark_failure(index);
                }
            }
        }
        Err(Self::exhausted_error(attempts, &failures))
    }

    fn native_structured_support(&self) -> NativeStructuredSupport {
        self.structured_support
    }

    fn has_distinct_non_streaming_transport(&self) -> bool {
        self.clients
            .iter()
            .any(|client| client.has_distinct_non_streaming_transport())
    }

    async fn complete_structured(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
        directive: &StructuredDirective,
    ) -> anyhow::Result<LlmResponse> {
        let mut attempts = 0;
        let mut failures = Vec::new();
        for index in self.ordered_indices() {
            attempts += 1;
            let result = tokio::time::timeout(
                self.candidate_idle_timeout,
                self.clients[index].complete_structured(messages, system, tools, directive),
            )
            .await;
            let failure = match result {
                Ok(Ok(response)) => {
                    self.mark_success(index);
                    return Ok(response);
                }
                Ok(Err(_)) => CandidateFailureKind::ProviderError,
                Err(_) => CandidateFailureKind::RequestStall,
            };
            failures.push(failure);
            Self::record_failure(attempts, failure);
            self.mark_failure(index);
        }
        Err(Self::exhausted_error(attempts, &failures))
    }

    async fn complete_streaming_structured(
        &self,
        messages: &[Message],
        system: Option<&str>,
        tools: &[ToolDefinition],
        directive: &StructuredDirective,
        cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        if cancel_token.is_cancelled() {
            return Err(Self::cancelled_error());
        }
        let deadline = tokio::time::Instant::now() + self.usable_generation_timeout;
        let ordered_indices = self.ordered_indices();
        let hedge_index = ordered_indices
            .iter()
            .copied()
            .find(|index| self.clients[*index].has_distinct_non_streaming_transport());
        let attempts = ordered_indices.len();
        let mut failures = Vec::new();
        let mut pending = HashSet::with_capacity(attempts);
        let mut attempt_ordinals = Vec::with_capacity(attempts);
        let mut attempt_cancellations = Vec::with_capacity(attempts);
        let mut candidate_tasks = FuturesUnordered::<StructuredCandidateFuture>::new();

        for (offset, index) in ordered_indices.into_iter().enumerate() {
            let ordinal = offset + 1;
            pending.insert(index);
            attempt_ordinals.push((index, ordinal));
            let client = Arc::clone(&self.clients[index]);
            let messages = messages.to_vec();
            let system = system.map(str::to_string);
            let tools = tools.to_vec();
            let directive = directive.clone();
            if Some(index) == hedge_index {
                candidate_tasks.push(Box::pin(async move {
                    let result = client
                        .complete_structured(&messages, system.as_deref(), &tools, &directive)
                        .await
                        .map(Box::new);
                    (
                        index,
                        ordinal,
                        StructuredCandidateCompletion::NonStreaming(result),
                    )
                }));
            } else {
                let attempt_cancel = cancel_token.child_token();
                attempt_cancellations.push(attempt_cancel.clone());
                let idle_timeout = self.usable_generation_timeout;
                candidate_tasks.push(Box::pin(async move {
                    let structured_completion =
                        StructuredStreamCompletion::from_request(&tools, &directive);
                    let result = complete_stream_attempt(
                        client.complete_streaming_structured(
                            &messages,
                            system.as_deref(),
                            &tools,
                            &directive,
                            attempt_cancel.clone(),
                        ),
                        attempt_cancel,
                        idle_timeout,
                        deadline,
                        structured_completion,
                    )
                    .await;
                    (
                        index,
                        ordinal,
                        StructuredCandidateCompletion::Streaming(result),
                    )
                }));
            }
        }

        let mut deadline_sleep = Box::pin(tokio::time::sleep_until(deadline));
        while !candidate_tasks.is_empty() {
            let next = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => {
                    attempt_cancellations.iter().for_each(CancellationToken::cancel);
                    return Err(Self::cancelled_error());
                }
                _ = &mut deadline_sleep => None,
                result = candidate_tasks.next() => result,
            };
            let Some((index, ordinal, completion)) = next else {
                for (pending_index, pending_ordinal) in &attempt_ordinals {
                    if pending.remove(pending_index) {
                        let failure = CandidateFailureKind::GlobalDeadline;
                        failures.push(failure);
                        Self::record_failure(*pending_ordinal, failure);
                        self.mark_failure(*pending_index);
                    }
                }
                attempt_cancellations
                    .iter()
                    .for_each(CancellationToken::cancel);
                break;
            };
            pending.remove(&index);
            let failure = match completion {
                StructuredCandidateCompletion::Streaming(Ok(events)) => {
                    attempt_cancellations
                        .iter()
                        .for_each(CancellationToken::cancel);
                    self.mark_success(index);
                    return Ok(replay_complete_stream(events));
                }
                StructuredCandidateCompletion::NonStreaming(Ok(response)) => {
                    attempt_cancellations
                        .iter()
                        .for_each(CancellationToken::cancel);
                    self.mark_success(index);
                    return Ok(replay_complete_stream(vec![StreamEvent::Done(*response)]));
                }
                StructuredCandidateCompletion::Streaming(Err(StreamAttemptError::Cancelled)) => {
                    attempt_cancellations
                        .iter()
                        .for_each(CancellationToken::cancel);
                    return Err(Self::cancelled_error());
                }
                StructuredCandidateCompletion::Streaming(Err(StreamAttemptError::Failed(
                    failure,
                ))) => failure,
                StructuredCandidateCompletion::NonStreaming(Err(_)) => {
                    CandidateFailureKind::ProviderError
                }
            };
            failures.push(failure);
            Self::record_failure(ordinal, failure);
            self.mark_failure(index);
        }

        if cancel_token.is_cancelled() {
            return Err(Self::cancelled_error());
        }
        Err(Self::exhausted_error(attempts, &failures))
    }
}

fn prepare_config_llm_config(
    code_config: &CodeConfig,
    options: &SessionOptions,
    session_id: &str,
) -> Result<LlmConfig, String> {
    let model_ref = options
        .model
        .as_deref()
        .or(code_config.default_model.as_deref())
        .ok_or_else(|| "default_model must be set in 'provider/model' format".to_string())?;
    let (provider_name, model_id) = model_ref
        .split_once('/')
        .ok_or_else(|| "model format must be 'provider/model'".to_string())?;
    let mut config = code_config
        .llm_config(provider_name, model_id)
        .ok_or_else(|| {
            format!("provider '{provider_name}' or model '{model_id}' not found in config")
        })?;

    if options.model.is_some() {
        if let Some(temperature) = options.temperature {
            config = config.with_temperature(temperature);
        }
        if let Some(thinking_budget) = options.thinking_budget {
            config = config.with_thinking_budget(thinking_budget);
        }
    }
    if let Some(timeout_ms) = options.llm_api_timeout_ms {
        config = config.with_api_timeout(timeout_ms);
    }
    if let Some(enabled) = options
        .llm_logprobs
        .or_else(|| env_bool("A3S_CODE_LLM_LOGPROBS"))
        .or_else(|| env_bool("A3S_CODE_OPENAI_LOGPROBS"))
    {
        config = config.with_logprobs(enabled);
    }
    if let Some(top_logprobs) = options
        .llm_top_logprobs
        .or_else(|| env_usize("A3S_CODE_LLM_TOP_LOGPROBS"))
        .or_else(|| env_usize("A3S_CODE_OPENAI_TOP_LOGPROBS"))
    {
        config = config.with_top_logprobs(top_logprobs);
    }

    Ok(config.with_session_id(session_id))
}

fn env_bool(name: &str) -> Option<bool> {
    let value = std::env::var(name).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use a3s_code_core::llm::ToolDefinition;
    use a3s_code_core::{CodeConfig, LlmClient, LlmResponse, Message, SessionOptions};
    use async_trait::async_trait;

    use super::{prepare_config_llm_config, resolve_session_llm_client};

    struct OverrideClient;

    #[async_trait]
    impl LlmClient for OverrideClient {
        async fn complete(
            &self,
            _messages: &[Message],
            _system: Option<&str>,
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            unreachable!("client identity test does not send requests")
        }

        async fn complete_streaming(
            &self,
            _messages: &[Message],
            _system: Option<&str>,
            _tools: &[ToolDefinition],
            _cancel_token: tokio_util::sync::CancellationToken,
        ) -> anyhow::Result<tokio::sync::mpsc::Receiver<a3s_code_core::llm::StreamEvent>> {
            unreachable!("client identity test does not send requests")
        }
    }

    fn test_config() -> CodeConfig {
        CodeConfig::from_acl(
            r#"
                default_model = "openai/default-model"
                llm_api_timeout_ms = 1200

                providers "openai" {
                  apiKey = "sk-test"
                  baseUrl = "https://example.com/v1"
                  sessionIdHeader = "x-session-id"

                  models "default-model" {}
                  models "selected-model" {}
                  models "text-only" { toolCall = false }
                }
            "#,
        )
        .expect("test config")
    }

    #[test]
    fn prepares_selected_model_with_session_overrides() {
        let mut options = SessionOptions::new().with_model("openai/selected-model");
        options.temperature = Some(0.25);
        options.thinking_budget = Some(4096);
        options.llm_api_timeout_ms = Some(2400);
        options.llm_logprobs = Some(true);
        options.llm_top_logprobs = Some(3);

        let resolved = prepare_config_llm_config(&test_config(), &options, "session-42")
            .expect("resolve selected model");

        assert_eq!(resolved.provider, "openai");
        assert_eq!(resolved.model, "selected-model");
        assert_eq!(resolved.session_id.as_deref(), Some("session-42"));
        assert_eq!(resolved.temperature, Some(0.25));
        assert_eq!(resolved.thinking_budget, Some(4096));
        assert_eq!(resolved.api_timeout_ms, Some(2400));
        assert_eq!(resolved.logprobs, Some(true));
        assert_eq!(resolved.top_logprobs, Some(3));
        assert_eq!(resolved.native_structured_support, None);
    }

    #[test]
    fn prepares_default_model_when_session_has_no_override() {
        let resolved =
            prepare_config_llm_config(&test_config(), &SessionOptions::new(), "session-default")
                .expect("resolve default model");

        assert_eq!(resolved.provider, "openai");
        assert_eq!(resolved.model, "default-model");
        assert_eq!(resolved.session_id.as_deref(), Some("session-default"));
        assert_eq!(resolved.api_timeout_ms, Some(1200));
    }

    #[test]
    fn rejects_unknown_model_reference() {
        let options = SessionOptions::new().with_model("openai/missing");

        let error = prepare_config_llm_config(&test_config(), &options, "session-unknown")
            .expect_err("unknown model should fail");

        assert!(error.contains("openai"));
        assert!(error.contains("missing"));
    }

    #[test]
    fn custom_openai_text_only_model_keeps_prompt_structured_fallback() {
        let options = SessionOptions::new().with_model("openai/text-only");
        let resolved = prepare_config_llm_config(&test_config(), &options, "session-text")
            .expect("resolve text-only model");

        assert_eq!(resolved.native_structured_support, None);
    }

    #[test]
    fn session_override_is_retained_by_identity() {
        let override_client: Arc<dyn LlmClient> = Arc::new(OverrideClient);
        let options = SessionOptions::new().with_llm_client(Arc::clone(&override_client));

        let resolved = resolve_session_llm_client(&test_config(), &options, "session-override")
            .expect("resolve override client");

        assert!(Arc::ptr_eq(&override_client, &resolved));
    }
}

#[cfg(test)]
#[path = "session_llm/deep_research_tests.rs"]
mod deep_research_tests;
