use a3s_code_core::llm::{HttpClient, HttpResponse, StreamingHttpResponse};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest_codex::cookie::{CookieStore, Jar};
use reqwest_codex::header::HeaderValue;
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// HTTP generation used only by the ChatGPT-account Codex adapter.
///
/// The generic core client currently uses reqwest 0.11. Cloudflare evaluates
/// transport characteristics in addition to headers, so Codex needs the same
/// reqwest generation and cookie handshake used by the official client.
pub(super) struct CodexHttpClient {
    client: reqwest_codex::Client,
}

impl CodexHttpClient {
    pub(super) fn new() -> Result<Self> {
        let cloudflare_cookies = Arc::new(ChatGptCloudflareCookieStore::default());
        let client = reqwest_codex::Client::builder()
            .cookie_provider(cloudflare_cookies)
            .build()
            .context("build Codex HTTP client")?;
        Ok(Self { client })
    }

    fn request(
        &self,
        url: &str,
        headers: Vec<(&str, &str)>,
        body: &Value,
    ) -> reqwest_codex::RequestBuilder {
        headers
            .into_iter()
            .fold(self.client.post(url), |request, (name, value)| {
                request.header(name, value)
            })
            .json(body)
    }
}

#[async_trait]
impl HttpClient for CodexHttpClient {
    async fn post(
        &self,
        url: &str,
        headers: Vec<(&str, &str)>,
        body: &Value,
        cancel_token: CancellationToken,
    ) -> Result<HttpResponse> {
        let request = self.request(url, headers, body);
        let response = tokio::select! {
            _ = cancel_token.cancelled() => anyhow::bail!("Codex HTTP request cancelled"),
            result = request.send() => result.with_context(|| format!("send Codex request to {url}"))?,
        };
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .with_context(|| format!("read Codex response from {url}"))?;
        Ok(HttpResponse { status, body })
    }

    async fn post_streaming(
        &self,
        url: &str,
        headers: Vec<(&str, &str)>,
        body: &Value,
        cancel_token: CancellationToken,
    ) -> Result<StreamingHttpResponse> {
        let request = self.request(url, headers, body);
        let response = tokio::select! {
            _ = cancel_token.cancelled() => anyhow::bail!("Codex HTTP streaming request cancelled"),
            result = request.send() => result.with_context(|| format!("send Codex streaming request to {url}"))?,
        };
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest_codex::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        if (200..300).contains(&status) {
            let byte_stream = response.bytes_stream().map(|result| {
                result.map_err(|error| anyhow::anyhow!("Codex stream error: {error}"))
            });
            Ok(StreamingHttpResponse {
                status,
                retry_after,
                byte_stream: Box::pin(byte_stream),
                error_body: String::new(),
            })
        } else {
            let error_body = response.text().await.unwrap_or_default();
            Ok(StreamingHttpResponse {
                status,
                retry_after,
                byte_stream: Box::pin(futures::stream::empty()),
                error_body,
            })
        }
    }
}

/// A session-scoped jar that can never retain ChatGPT identity or auth state.
///
/// Keep this allowlist narrow: the Codex bearer token remains the only account
/// credential used by this transport.
#[derive(Debug, Default)]
struct ChatGptCloudflareCookieStore {
    jar: Jar,
}

impl CookieStore for ChatGptCloudflareCookieStore {
    fn set_cookies(
        &self,
        cookie_headers: &mut dyn Iterator<Item = &HeaderValue>,
        url: &reqwest_codex::Url,
    ) {
        if !is_chatgpt_cookie_url(url) {
            return;
        }

        let mut cloudflare_cookies = cookie_headers.filter(|header| {
            header
                .to_str()
                .ok()
                .and_then(set_cookie_name)
                .is_some_and(is_allowed_cloudflare_cookie_name)
        });
        self.jar.set_cookies(&mut cloudflare_cookies, url);
    }

    fn cookies(&self, url: &reqwest_codex::Url) -> Option<HeaderValue> {
        if !is_chatgpt_cookie_url(url) {
            return None;
        }

        let cookies = self.jar.cookies(url)?;
        let filtered = only_cloudflare_cookies(cookies.to_str().ok()?)?;
        HeaderValue::from_str(&filtered).ok()
    }
}

fn is_chatgpt_cookie_url(url: &reqwest_codex::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }

    url.host_str().is_some_and(is_allowed_chatgpt_host)
}

fn is_allowed_chatgpt_host(host: &str) -> bool {
    const EXACT_HOSTS: &[&str] = &["chatgpt.com", "chat.openai.com", "chatgpt-staging.com"];
    const SUBDOMAIN_SUFFIXES: &[&str] = &[".chatgpt.com", ".chatgpt-staging.com"];

    EXACT_HOSTS.contains(&host)
        || SUBDOMAIN_SUFFIXES
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

fn set_cookie_name(header: &str) -> Option<&str> {
    let (name, _) = header.split_once('=')?;
    let name = name.trim();
    (!name.is_empty()).then_some(name)
}

fn only_cloudflare_cookies(header: &str) -> Option<String> {
    let cookies = header
        .split(';')
        .filter_map(|cookie| {
            let cookie = cookie.trim();
            let name = cookie.split_once('=')?.0.trim();
            is_allowed_cloudflare_cookie_name(name).then_some(cookie)
        })
        .collect::<Vec<_>>()
        .join("; ");

    (!cookies.is_empty()).then_some(cookies)
}

fn is_allowed_cloudflare_cookie_name(name: &str) -> bool {
    matches!(
        name,
        "__cf_bm"
            | "__cflb"
            | "__cfruid"
            | "__cfseq"
            | "__cfwaitingroom"
            | "_cfuvid"
            | "cf_clearance"
            | "cf_ob_info"
            | "cf_use_ob"
    ) || name.starts_with("cf_chl_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_code_core::llm::HttpClient;
    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
        Json, Router,
    };
    use futures::StreamExt;
    use reqwest_codex::cookie::CookieStore;
    use reqwest_codex::header::HeaderValue;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    #[derive(Debug, Default)]
    struct CapturedRequest {
        authorization: String,
        body: Option<Value>,
    }

    async fn capture_json(
        State(captured): State<Arc<Mutex<CapturedRequest>>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        let mut captured = captured
            .lock()
            .expect("capture lock should not be poisoned");
        captured.authorization = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        captured.body = Some(body);
        (StatusCode::CREATED, Json(json!({"accepted": true})))
    }

    async fn stream_sse() -> impl IntoResponse {
        (
            StatusCode::OK,
            [("content-type", "text/event-stream")],
            "data: {\"type\":\"response.completed\"}\n\n",
        )
    }

    async fn rate_limited() -> impl IntoResponse {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "17")],
            "synthetic failure",
        )
    }

    async fn slow_response() -> impl IntoResponse {
        tokio::time::sleep(Duration::from_secs(10)).await;
        (StatusCode::OK, "too late")
    }

    async fn spawn_server() -> (
        String,
        Arc<Mutex<CapturedRequest>>,
        tokio::task::JoinHandle<()>,
    ) {
        let captured = Arc::new(Mutex::new(CapturedRequest::default()));
        let app = Router::new()
            .route("/json", post(capture_json))
            .route("/stream", post(stream_sse))
            .route("/error", post(rate_limited))
            .route("/slow", post(slow_response))
            .with_state(Arc::clone(&captured));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test server should run");
        });
        (format!("http://{address}"), captured, server)
    }

    #[test]
    fn allows_only_https_chatgpt_cookie_urls() {
        for url in [
            "https://chatgpt.com/backend-api/codex/responses",
            "https://chat.openai.com/backend-api/codex/responses",
            "https://chatgpt-staging.com/backend-api/codex/responses",
            "https://api.chatgpt.com/backend-api/codex/responses",
            "https://api.chatgpt-staging.com/backend-api/codex/responses",
        ] {
            let url = reqwest_codex::Url::parse(url).expect("test URL should parse");
            assert!(is_chatgpt_cookie_url(&url), "should allow {url}");
        }

        for url in [
            "http://chatgpt.com/backend-api/codex/responses",
            "https://example.com/backend-api/codex/responses",
            "https://notchatgpt.com/backend-api/codex/responses",
            "https://chatgpt.com.example.com/backend-api/codex/responses",
        ] {
            let url = reqwest_codex::Url::parse(url).expect("test URL should parse");
            assert!(!is_chatgpt_cookie_url(&url), "should reject {url}");
        }
    }

    #[test]
    fn allows_only_documented_cloudflare_cookie_names() {
        for name in [
            "__cf_bm",
            "__cflb",
            "__cfruid",
            "__cfseq",
            "__cfwaitingroom",
            "_cfuvid",
            "cf_clearance",
            "cf_ob_info",
            "cf_use_ob",
            "cf_chl_rc_i",
        ] {
            assert!(
                is_allowed_cloudflare_cookie_name(name),
                "should allow {name}"
            );
        }

        for name in [
            "__Secure-next-auth.session-token",
            "chatgpt_session",
            "oai-auth-token",
            "not_cf_clearance",
            "x_cf_chl_rc_i",
        ] {
            assert!(
                !is_allowed_cloudflare_cookie_name(name),
                "should reject {name}"
            );
        }
    }

    #[test]
    fn outgoing_cookie_filter_drops_account_and_session_state() {
        let filtered = only_cloudflare_cookies(
            "__cf_bm=bot; chatgpt_session=secret; cf_clearance=clear; \
             __Secure-next-auth.session-token=also-secret; _cfuvid=visitor",
        );

        assert_eq!(
            filtered.as_deref(),
            Some("__cf_bm=bot; cf_clearance=clear; _cfuvid=visitor")
        );
        assert!(!filtered.unwrap().contains("secret"));
    }

    #[test]
    fn set_cookie_name_requires_a_nonempty_name() {
        assert_eq!(set_cookie_name("__cf_bm=value; Secure"), Some("__cf_bm"));
        assert_eq!(set_cookie_name(" =value"), None);
        assert_eq!(set_cookie_name("missing-separator"), None);
    }

    #[test]
    fn store_keeps_only_cloudflare_cookies_for_chatgpt_https() {
        let store = ChatGptCloudflareCookieStore::default();
        let url = reqwest_codex::Url::parse("https://chatgpt.com/backend-api/codex/responses")
            .expect("test URL should parse");
        let bot = HeaderValue::from_static("__cf_bm=bot; Path=/; Secure; HttpOnly");
        let clearance = HeaderValue::from_static("cf_clearance=clear; Path=/; Secure; HttpOnly");
        let session = HeaderValue::from_static("chatgpt_session=secret; Path=/; Secure; HttpOnly");

        store.set_cookies(&mut [&bot, &session, &clearance].into_iter(), &url);

        let mut cookies = store
            .cookies(&url)
            .and_then(|value| value.to_str().ok().map(str::to_string))
            .expect("safe cookies should be returned")
            .split("; ")
            .map(str::to_string)
            .collect::<Vec<_>>();
        cookies.sort();
        assert_eq!(cookies, ["__cf_bm=bot", "cf_clearance=clear"]);
    }

    #[test]
    fn store_rejects_cloudflare_cookies_from_untrusted_urls() {
        let store = ChatGptCloudflareCookieStore::default();
        let cookie = HeaderValue::from_static("__cf_bm=bot; Path=/; Secure; HttpOnly");
        for url in [
            "http://chatgpt.com/backend-api/codex/responses",
            "https://example.com/backend-api/codex/responses",
        ] {
            let url = reqwest_codex::Url::parse(url).expect("test URL should parse");
            store.set_cookies(&mut std::iter::once(&cookie), &url);
            assert_eq!(store.cookies(&url), None);
        }
    }

    #[tokio::test]
    async fn post_forwards_headers_and_json_without_leaking_auth() {
        let (base, captured, server) = spawn_server().await;
        let client = CodexHttpClient::new().expect("client should build");
        let token = "Bearer synthetic-secret";

        let response = client
            .post(
                &format!("{base}/json"),
                vec![("Authorization", token)],
                &json!({"model": "test-model"}),
                CancellationToken::new(),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status, 201);
        assert_eq!(
            serde_json::from_str::<Value>(&response.body).expect("response should be JSON"),
            json!({"accepted": true})
        );
        let captured = captured
            .lock()
            .expect("capture lock should not be poisoned");
        assert_eq!(captured.authorization, token);
        assert_eq!(captured.body, Some(json!({"model": "test-model"})));
        assert!(!response.body.contains("synthetic-secret"));
        server.abort();
    }

    #[tokio::test]
    async fn post_streaming_returns_bytes_and_buffers_errors() {
        let (base, _, server) = spawn_server().await;
        let client = CodexHttpClient::new().expect("client should build");

        let mut success = client
            .post_streaming(
                &format!("{base}/stream"),
                Vec::new(),
                &json!({}),
                CancellationToken::new(),
            )
            .await
            .expect("stream request should succeed");
        let mut body = Vec::new();
        while let Some(chunk) = success.byte_stream.next().await {
            body.extend_from_slice(&chunk.expect("stream chunk should succeed"));
        }
        assert_eq!(success.status, 200);
        assert_eq!(
            String::from_utf8(body).expect("stream should be UTF-8"),
            "data: {\"type\":\"response.completed\"}\n\n"
        );

        let mut error = client
            .post_streaming(
                &format!("{base}/error"),
                Vec::new(),
                &json!({}),
                CancellationToken::new(),
            )
            .await
            .expect("HTTP errors should remain structured responses");
        assert_eq!(error.status, 429);
        assert_eq!(error.retry_after.as_deref(), Some("17"));
        assert_eq!(error.error_body, "synthetic failure");
        assert!(error.byte_stream.next().await.is_none());
        server.abort();
    }

    #[tokio::test]
    async fn cancelled_request_fails_without_exposing_headers() {
        let (base, _, server) = spawn_server().await;
        let client = CodexHttpClient::new().expect("client should build");
        let cancel = CancellationToken::new();
        cancel.cancel();

        let result = client
            .post(
                &format!("{base}/slow"),
                vec![("Authorization", "Bearer do-not-print")],
                &json!({}),
                cancel,
            )
            .await;
        let error = match result {
            Ok(_) => panic!("cancelled request should fail"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("cancelled"));
        assert!(!error.to_string().contains("do-not-print"));
        server.abort();
    }
}
