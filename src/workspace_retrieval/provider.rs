use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use a3s_code_core::embedding::{
    EmbeddingBatchRequest, EmbeddingBatchResponse, EmbeddingExecutorConfig, EmbeddingProvider,
    EmbeddingProviderDescriptor, EmbeddingProviderError, EmbeddingVector,
};
use a3s_code_core::{CodeConfig, WorkspaceRetrievalOptions, WorkspaceSemanticIndexLimits};
use anyhow::{bail, Context};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE,
    HOST, RETRY_AFTER, TRANSFER_ENCODING,
};
use reqwest::{redirect, Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use url::Host;

use super::WorkspaceRetrievalConfig;

const MAX_PROVIDER_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
pub(super) struct OpenAiCompatibleEmbeddingProvider {
    client: Client,
    endpoint: Url,
    headers: HeaderMap,
    descriptor: EmbeddingProviderDescriptor,
    response_limit: usize,
}

impl fmt::Debug for OpenAiCompatibleEmbeddingProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiCompatibleEmbeddingProvider")
            .field("endpoint", &"<configured>")
            .field("descriptor", &self.descriptor)
            .field("header_count", &self.headers.len())
            .field("response_limit", &self.response_limit)
            .finish()
    }
}

#[cfg(test)]
impl OpenAiCompatibleEmbeddingProvider {
    pub(super) fn for_test(endpoint: Url, response_limit: usize) -> Self {
        Self {
            client: Client::builder()
                .redirect(redirect::Policy::none())
                .build()
                .expect("test HTTP client must build"),
            endpoint,
            headers: HeaderMap::new(),
            descriptor: EmbeddingProviderDescriptor::new("test", "embed-v1", 3)
                .with_normalization(a3s_code_core::embedding::EmbeddingNormalization::Unit),
            response_limit,
        }
    }
}

#[derive(Serialize)]
struct OpenAiEmbeddingRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

#[derive(Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
}

#[derive(Deserialize)]
struct OpenAiEmbeddingData {
    index: usize,
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for OpenAiCompatibleEmbeddingProvider {
    fn descriptor(&self) -> EmbeddingProviderDescriptor {
        self.descriptor.clone()
    }

    async fn embed(
        &self,
        request: EmbeddingBatchRequest,
        cancellation: CancellationToken,
    ) -> Result<EmbeddingBatchResponse, EmbeddingProviderError> {
        let payload = OpenAiEmbeddingRequest {
            model: &self.descriptor.model,
            input: request.inputs().iter().map(|input| input.text()).collect(),
        };
        let send = self
            .client
            .post(self.endpoint.clone())
            .headers(self.headers.clone())
            .json(&payload)
            .send();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(EmbeddingProviderError::Cancelled),
            response = send => response.map_err(map_reqwest_error)?,
        };
        if !response.status().is_success() {
            return Err(map_status(
                response.status(),
                parse_retry_after(response.headers()),
            ));
        }
        let body = read_body_bounded(response, self.response_limit, &cancellation).await?;
        let response: OpenAiEmbeddingResponse =
            serde_json::from_slice(&body).map_err(|_| EmbeddingProviderError::Other)?;
        let mut outputs = Vec::with_capacity(response.data.len());
        let mut seen = vec![false; request.inputs().len()];
        for data in response.data {
            let Some(input) = request.inputs().get(data.index) else {
                return Err(EmbeddingProviderError::Other);
            };
            if seen[data.index] {
                return Err(EmbeddingProviderError::Other);
            }
            seen[data.index] = true;
            outputs.push(EmbeddingVector::new(input.id(), data.embedding));
        }
        Ok(EmbeddingBatchResponse::new(self.descriptor(), outputs))
    }
}

pub(crate) fn build_workspace_retrieval_options(
    retrieval: &WorkspaceRetrievalConfig,
    code: &CodeConfig,
) -> anyhow::Result<Option<WorkspaceRetrievalOptions>> {
    retrieval.validate()?;
    if !retrieval.enabled {
        return Ok(None);
    }
    let route = retrieval
        .model
        .as_deref()
        .context("workspace_retrieval model is missing")?;
    let (provider_name, model_id) = route
        .split_once('/')
        .context("workspace_retrieval model must use the provider/model format")?;
    let provider = code.find_provider(provider_name).with_context(|| {
        format!("workspace_retrieval provider `{provider_name}` is not configured")
    })?;
    let model = provider.find_model(model_id);
    let configured_base_url = model
        .and_then(|model| provider.get_base_url(model))
        .or(provider.base_url.as_deref());
    let endpoint = match retrieval.endpoint.as_deref() {
        Some(endpoint) => validate_endpoint(endpoint)?,
        None => derive_embedding_endpoint(
            configured_base_url
                .context("workspace_retrieval requires endpoint or a provider/model baseUrl")?,
        )?,
    };
    let static_headers = model
        .map(|model| provider.get_headers(model))
        .unwrap_or_else(|| provider.headers.clone());
    let api_key = model
        .and_then(|model| provider.get_api_key(model))
        .or(provider.api_key.as_deref());
    let headers = build_headers(static_headers, api_key)?;
    let timeout = Duration::from_millis(retrieval.provider_timeout_ms);
    let client = Client::builder()
        .redirect(redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(timeout)
        .build()
        .context("could not construct the workspace retrieval HTTP client")?;
    let mut descriptor = EmbeddingProviderDescriptor::new(
        provider_name,
        model_id,
        retrieval
            .dimension
            .context("workspace_retrieval dimension is missing")?,
    )
    .with_normalization(retrieval.normalization);
    if let Some(revision) = &retrieval.revision {
        descriptor = descriptor.with_revision(revision.clone());
    }
    let provider: Arc<dyn EmbeddingProvider> = Arc::new(OpenAiCompatibleEmbeddingProvider {
        client,
        endpoint,
        headers,
        descriptor,
        response_limit: MAX_PROVIDER_RESPONSE_BYTES,
    });
    let embedding = EmbeddingExecutorConfig {
        request_timeout: timeout,
        ..EmbeddingExecutorConfig::default()
    };
    let limits = WorkspaceSemanticIndexLimits {
        max_records: retrieval.max_records,
        max_bytes: retrieval.max_bytes,
        shutdown_timeout: Duration::from_millis(retrieval.shutdown_timeout_ms),
    };
    Ok(Some(
        WorkspaceRetrievalOptions::new(provider)
            .with_embedding_config(embedding)
            .with_index_limits(limits),
    ))
}

pub(super) fn validate_endpoint(value: &str) -> anyhow::Result<Url> {
    let url = Url::parse(value).context("workspace_retrieval endpoint must be an absolute URL")?;
    validate_endpoint_url(url)
}

pub(super) fn derive_embedding_endpoint(value: &str) -> anyhow::Result<Url> {
    let mut url = validate_endpoint(value)?;
    let path = url.path().trim_end_matches('/');
    if !path.ends_with("/embeddings") {
        url.set_path(&format!("{path}/embeddings"));
    }
    Ok(url)
}

fn validate_endpoint_url(url: Url) -> anyhow::Result<Url> {
    if !url.username().is_empty() || url.password().is_some() {
        bail!("workspace_retrieval endpoint must not contain user information");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("workspace_retrieval endpoint must not contain a query or fragment");
    }
    let loopback = match url
        .host()
        .context("workspace_retrieval endpoint must contain a host")?
    {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(address) => address.is_loopback(),
        Host::Ipv6(address) => address.is_loopback(),
    };
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => bail!("workspace_retrieval requires HTTPS for non-loopback endpoints"),
        _ => bail!("workspace_retrieval endpoint must use HTTPS or loopback HTTP"),
    }
    Ok(url)
}

pub(super) fn build_headers(
    configured: HashMap<String, String>,
    api_key: Option<&str>,
) -> anyhow::Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in configured {
        let name = HeaderName::from_bytes(name.as_bytes())
            .context("workspace_retrieval provider contains an invalid header name")?;
        if matches!(
            name,
            HOST | CONTENT_LENGTH | CONTENT_TYPE | CONNECTION | TRANSFER_ENCODING
        ) {
            bail!("workspace_retrieval provider contains a reserved HTTP header");
        }
        let mut value = HeaderValue::from_str(&value)
            .context("workspace_retrieval provider contains an invalid header value")?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    if !headers.contains_key(AUTHORIZATION) {
        if let Some(api_key) = api_key {
            let mut value = HeaderValue::from_str(&format!("Bearer {api_key}"))
                .context("workspace_retrieval provider API key is not a valid header value")?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
    }
    Ok(headers)
}

async fn read_body_bounded(
    response: reqwest::Response,
    limit: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, EmbeddingProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(EmbeddingProviderError::Other);
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancellation.cancelled() => return Err(EmbeddingProviderError::Cancelled),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(map_reqwest_error)?;
        let size = body
            .len()
            .checked_add(chunk.len())
            .ok_or(EmbeddingProviderError::Other)?;
        if size > limit {
            return Err(EmbeddingProviderError::Other);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn map_reqwest_error(error: reqwest::Error) -> EmbeddingProviderError {
    if error.is_timeout() {
        EmbeddingProviderError::Timeout
    } else if error.is_connect() || error.is_request() || error.is_body() {
        EmbeddingProviderError::Unavailable { retry_after: None }
    } else {
        EmbeddingProviderError::Other
    }
}

fn map_status(status: StatusCode, retry_after: Option<Duration>) -> EmbeddingProviderError {
    match status.as_u16() {
        401 | 403 => EmbeddingProviderError::Authentication,
        408 | 504 => EmbeddingProviderError::Timeout,
        429 => EmbeddingProviderError::RateLimited { retry_after },
        500..=599 => EmbeddingProviderError::Unavailable { retry_after },
        300..=499 => EmbeddingProviderError::InvalidRequest,
        _ => EmbeddingProviderError::Other,
    }
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
    let delay = value
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            let target = httpdate::parse_http_date(value).ok()?;
            target.duration_since(SystemTime::now()).ok()
        })?;
    Some(delay.min(MAX_RETRY_AFTER))
}
