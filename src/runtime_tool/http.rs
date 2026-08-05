use std::future::Future;

use anyhow::{Context, Result};
use futures::StreamExt;
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{RuntimeTool, MAX_RUNTIME_ID_CHARS};

pub(super) fn endpoint_url(origin: &str, segments: &[&str]) -> Result<Url> {
    let mut url = Url::parse(origin).context("invalid A3S Runtime origin")?;
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("A3S Runtime origin cannot be used as an API base"))?;
        path.clear();
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

pub(super) async fn request_envelope(
    request: reqwest::RequestBuilder,
    max_bytes: usize,
    cancellation: &CancellationToken,
) -> Result<Value> {
    let response = run_cancel_aware(
        async { request.send().await.context("runtime request failed") },
        cancellation,
    )
    .await?;
    let status = response.status().as_u16();
    let body = read_bounded_response(response, max_bytes, cancellation).await?;
    RuntimeTool::unwrap_envelope(&body, status)
}

async fn read_bounded_response(
    response: reqwest::Response,
    max_bytes: usize,
    cancellation: &CancellationToken,
) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        anyhow::bail!("runtime response exceeds the {max_bytes}-byte limit");
    }

    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(max_bytes);
    let mut body = Vec::with_capacity(initial_capacity);
    let mut stream = response.bytes_stream();
    loop {
        let next = tokio::select! {
            _ = cancellation.cancelled() => anyhow::bail!("runtime request cancelled"),
            next = stream.next() => next,
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.context("could not read runtime response")?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            anyhow::bail!("runtime response exceeds the {max_bytes}-byte limit");
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).context("runtime response was not valid UTF-8")
}

async fn run_cancel_aware<T>(
    future: impl Future<Output = Result<T>>,
    cancellation: &CancellationToken,
) -> Result<T> {
    tokio::select! {
        _ = cancellation.cancelled() => anyhow::bail!("runtime request cancelled"),
        result = future => result,
    }
}

pub(super) async fn run_until_deadline<T>(
    future: impl Future<Output = Result<T>>,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<Option<T>> {
    tokio::select! {
        _ = cancellation.cancelled() => anyhow::bail!("runtime request cancelled"),
        _ = tokio::time::sleep_until(deadline) => Ok(None),
        result = future => result.map(Some),
    }
}

pub(super) fn validate_runtime_id(kind: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_RUNTIME_ID_CHARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        anyhow::bail!(
            "runtime returned an invalid {kind}; IDs must contain at most {MAX_RUNTIME_ID_CHARS} ASCII letters, digits, dots, underscores, or hyphens"
        );
    }
    Ok(value.to_string())
}
