use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, fmt::Write as _};

use a3s_code_core::embedding::{
    EmbeddingError, EmbeddingExecutor, EmbeddingExecutorConfig, EmbeddingFailureKind,
    EmbeddingInput, EmbeddingProvider,
};
use axum::extract::State;
use axum::http::{HeaderMap as AxumHeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect};
use axum::routing::post;
use axum::{Json, Router};
use reqwest::Url;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio_util::sync::CancellationToken;

use super::provider::{
    build_headers, build_workspace_retrieval_options, derive_embedding_endpoint, validate_endpoint,
    OpenAiCompatibleEmbeddingProvider,
};
use super::WorkspaceRetrievalConfig;

async fn server(app: Router) -> (Url, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let worker = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (
        Url::parse(&format!("http://{address}/embeddings")).unwrap(),
        worker,
    )
}

fn provider(endpoint: Url, response_limit: usize) -> Arc<dyn EmbeddingProvider> {
    Arc::new(OpenAiCompatibleEmbeddingProvider::for_test(
        endpoint,
        response_limit,
    ))
}

fn executor(provider: Arc<dyn EmbeddingProvider>) -> EmbeddingExecutor {
    EmbeddingExecutor::new(
        provider,
        EmbeddingExecutorConfig {
            max_retries: 0,
            request_timeout: Duration::from_secs(5),
            ..EmbeddingExecutorConfig::default()
        },
    )
    .unwrap()
}

#[tokio::test]
async fn sends_openai_compatible_input_and_preserves_input_identity() {
    let captured = Arc::new(Mutex::new(None));
    let app = Router::new().route(
        "/embeddings",
        post({
            let captured = Arc::clone(&captured);
            move |headers: AxumHeaderMap, Json(body): Json<Value>| {
                let captured = Arc::clone(&captured);
                async move {
                    *captured.lock().await = Some((headers, body));
                    Json(json!({
                        "data": [
                            {"index": 0, "embedding": [1.0, 0.0, 0.0]},
                            {"index": 1, "embedding": [0.0, 1.0, 0.0]}
                        ]
                    }))
                }
            }
        }),
    );
    let (endpoint, worker) = server(app).await;

    let result = executor(provider(endpoint, 1024 * 1024))
        .embed(
            vec![
                EmbeddingInput::new("first", "alpha"),
                EmbeddingInput::new("second", "beta"),
            ],
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.vectors[0].id.as_ref(), "first");
    assert_eq!(result.vectors[1].id.as_ref(), "second");
    let (_, body) = captured.lock().await.take().unwrap();
    assert_eq!(body["model"], "embed-v1");
    assert_eq!(body["input"], json!(["alpha", "beta"]));
    worker.abort();
}

#[tokio::test]
async fn maps_rate_limit_without_exposing_the_remote_body() {
    let marker = "REMOTE_BODY_MUST_NOT_LEAK";
    let app = Router::new().route(
        "/embeddings",
        post(move || async move {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", "999999")],
                marker,
            )
        }),
    );
    let (endpoint, worker) = server(app).await;

    let error = executor(provider(endpoint, 1024))
        .embed(
            vec![EmbeddingInput::new("one", "source text")],
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        EmbeddingError::RetriesExhausted {
            kind: EmbeddingFailureKind::RateLimited,
            attempts: 1
        }
    ));
    assert!(!error.to_string().contains(marker));
    worker.abort();
}

#[tokio::test]
async fn rejects_oversized_success_body_before_json_parsing() {
    let app = Router::new().route(
        "/embeddings",
        post(|| async { Json(json!({"data": [{"index": 0, "embedding": [1, 0, 0]}]})) }),
    );
    let (endpoint, worker) = server(app).await;

    let error = executor(provider(endpoint, 8))
        .embed(
            vec![EmbeddingInput::new("one", "source text")],
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        EmbeddingError::ProviderFailure {
            kind: EmbeddingFailureKind::Other,
            attempts: 1
        }
    ));
    worker.abort();
}

#[tokio::test]
async fn rejects_duplicate_provider_indexes_without_accepting_partial_vectors() {
    let app = Router::new().route(
        "/embeddings",
        post(|| async {
            Json(json!({
                "data": [
                    {"index": 0, "embedding": [1, 0, 0]},
                    {"index": 0, "embedding": [0, 1, 0]}
                ]
            }))
        }),
    );
    let (endpoint, worker) = server(app).await;

    let error = executor(provider(endpoint, 1024))
        .embed(
            vec![
                EmbeddingInput::new("one", "source one"),
                EmbeddingInput::new("two", "source two"),
            ],
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        EmbeddingError::ProviderFailure {
            kind: EmbeddingFailureKind::Other,
            attempts: 1
        }
    ));
    worker.abort();
}

#[tokio::test]
async fn cancellation_aborts_a_provider_request_without_sleeping() {
    #[derive(Clone)]
    struct Gate {
        entered: mpsc::Sender<()>,
        release: Arc<Notify>,
    }

    async fn blocked(State(gate): State<Gate>) -> impl IntoResponse {
        gate.entered.send(()).await.unwrap();
        gate.release.notified().await;
        Json(json!({"data": [{"index": 0, "embedding": [1, 0, 0]}]}))
    }

    let (entered_tx, mut entered_rx) = mpsc::channel(1);
    let gate = Gate {
        entered: entered_tx,
        release: Arc::new(Notify::new()),
    };
    let app = Router::new()
        .route("/embeddings", post(blocked))
        .with_state(gate.clone());
    let (endpoint, worker) = server(app).await;
    let cancellation = CancellationToken::new();
    let request = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            executor(provider(endpoint, 1024))
                .embed(
                    vec![EmbeddingInput::new("one", "source text")],
                    cancellation,
                )
                .await
        }
    });
    entered_rx.recv().await.unwrap();

    cancellation.cancel();
    let error = request.await.unwrap().unwrap_err();

    assert_eq!(error, EmbeddingError::Cancelled);
    gate.release.notify_waiters();
    worker.abort();
}

#[tokio::test]
async fn redirect_is_not_followed() {
    let target_hit = Arc::new(AtomicBool::new(false));
    let app = Router::new()
        .route(
            "/embeddings",
            post(|| async { Redirect::temporary("/redirect-target") }),
        )
        .route(
            "/redirect-target",
            post({
                let target_hit = Arc::clone(&target_hit);
                move || {
                    let target_hit = Arc::clone(&target_hit);
                    async move {
                        target_hit.store(true, Ordering::SeqCst);
                        Json(json!({"data": []}))
                    }
                }
            }),
        );
    let (endpoint, worker) = server(app).await;

    let error = executor(provider(endpoint, 1024))
        .embed(
            vec![EmbeddingInput::new("one", "source text")],
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        EmbeddingError::ProviderFailure {
            kind: EmbeddingFailureKind::InvalidRequest,
            attempts: 1
        }
    ));
    assert!(!target_hit.load(Ordering::SeqCst));
    worker.abort();
}

#[test]
fn endpoint_policy_allows_tls_or_exact_loopback_only() {
    for allowed in [
        "https://embedding.example/v1/embeddings",
        "http://localhost:8080/embeddings",
        "http://127.0.0.1:8080/embeddings",
        "http://[::1]:8080/embeddings",
    ] {
        validate_endpoint(allowed).unwrap();
    }
    for rejected in [
        "http://embedding.example/v1/embeddings",
        "http://localhost.example/embeddings",
        "ftp://127.0.0.1/embeddings",
        "https://user:pass@embedding.example/embeddings",
        "https://embedding.example/embeddings?secret=value",
        "https://embedding.example/embeddings#fragment",
    ] {
        assert!(validate_endpoint(rejected).is_err(), "accepted {rejected}");
    }
}

#[test]
fn provider_base_url_derives_the_embeddings_path_without_replacing_v1() {
    assert_eq!(
        derive_embedding_endpoint("https://embedding.example/v1")
            .unwrap()
            .as_str(),
        "https://embedding.example/v1/embeddings"
    );
    assert_eq!(
        derive_embedding_endpoint("https://embedding.example/v1/embeddings")
            .unwrap()
            .as_str(),
        "https://embedding.example/v1/embeddings"
    );
}

#[test]
fn configured_headers_are_sensitive_and_errors_never_echo_values() {
    let marker = "HEADER_SECRET_MUST_NOT_LEAK";
    let headers = build_headers(
        HashMap::from([("x-provider-key".to_string(), marker.to_string())]),
        None,
    )
    .unwrap();
    assert!(headers["x-provider-key"].is_sensitive());

    let error = build_headers(
        HashMap::from([("x-provider-key".to_string(), format!("{marker}\n"))]),
        None,
    )
    .unwrap_err();
    assert!(!error.to_string().contains(marker));
    assert!(build_headers(
        HashMap::from([("host".to_string(), "attacker.example".to_string())]),
        None,
    )
    .is_err());
}

#[test]
fn built_options_redact_credentials_and_endpoint_from_debug_output() {
    let api_key_marker = "API_KEY_MUST_NOT_LEAK";
    let endpoint_marker = "http://127.0.0.1:8080/private-token/v1";
    let mut acl = String::new();
    write!(
        acl,
        r#"
providers "local" {{
  apiKey = "{api_key_marker}"
  baseUrl = "{endpoint_marker}"
}}
"#
    )
    .unwrap();
    let code = a3s_code_core::CodeConfig::from_acl(&acl).unwrap();
    let retrieval = WorkspaceRetrievalConfig {
        enabled: true,
        allow_source_egress: true,
        model: Some("local/embed-v1".to_string()),
        endpoint: Some(format!("{endpoint_marker}/embeddings")),
        dimension: Some(3),
        ..WorkspaceRetrievalConfig::default()
    };

    let options = build_workspace_retrieval_options(&retrieval, &code)
        .unwrap()
        .unwrap();
    let debug = format!("{options:?} {retrieval:?}");

    assert!(!debug.contains(api_key_marker));
    assert!(!debug.contains(endpoint_marker));
    assert!(debug.contains("<host-injected>"));
    assert!(debug.contains("<configured>"));
}
