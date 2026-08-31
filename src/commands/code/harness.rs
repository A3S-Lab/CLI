use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use a3s_code_core::release::{
    agent_harness_compatibility_v1, AgentReleaseError, AgentReleaseManifest,
    AgentReleaseSecretTarget,
};
use a3s_code_core::{
    Agent, AgentProtocolChangeSetRequestV1, AgentProtocolCommandV1,
    AgentProtocolEventPageRequestV1, AgentProtocolHarness, AgentProtocolHarnessError,
    AgentProtocolHostError, SessionOptions, AGENT_PROTOCOL_CHANGE_SET_HTTP_PATH_V1,
    AGENT_PROTOCOL_COMMAND_HTTP_PATH_V1, AGENT_PROTOCOL_EVENT_PAGE_HTTP_PATH_V1,
};
use anyhow::{bail, Context};
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::cli::args::CodeHarnessArgs;
use crate::cli::context::InvocationContext;
use crate::cli::output::{coded_error, ExitClass};

const HEALTH_SCHEMA_V1: &str = "a3s.code.agent-health.v1";
const ERROR_SCHEMA_V1: &str = "a3s.code.agent-error.v1";
const MAX_REQUEST_BODY_BYTES: usize = 128 * 1024;

#[derive(Clone)]
struct HarnessServiceState {
    harness: Arc<AgentProtocolHarness>,
    ready: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct HealthResponse {
    schema: &'static str,
    status: &'static str,
}

#[derive(Serialize)]
struct ErrorResponse {
    schema: &'static str,
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

pub(super) async fn run(args: CodeHarnessArgs, context: &InvocationContext) -> anyhow::Result<()> {
    let manifest_path = context.resolve_path(args.manifest);
    let manifest =
        AgentReleaseManifest::from_file(&manifest_path).map_err(release_admission_error)?;
    manifest
        .verify_compatibility(&agent_harness_compatibility_v1())
        .map_err(release_admission_error)?;
    verify_required_secrets(&manifest, context).await?;

    let (_, code_config) = crate::commands::config::load_active_config(context)?;
    let agent = Arc::new(
        Agent::from_config(code_config)
            .await
            .context("could not initialize the Agent Harness runtime")?,
    );
    let shutdown_grace = Duration::from_secs(u64::from(manifest.health().shutdown_grace_seconds()));
    let harness = Arc::new(
        AgentProtocolHarness::new(
            manifest,
            agent,
            context.directory.to_string_lossy().to_string(),
        )
        .map_err(harness_admission_error)?
        .with_session_options(SessionOptions::new()),
    );
    let address = (args.listen, harness.manifest().health().port());
    let listener = TcpListener::bind(address).await.with_context(|| {
        format!(
            "could not bind Agent Harness to {}:{}",
            args.listen,
            harness.manifest().health().port()
        )
    })?;

    serve_listener(
        listener,
        harness,
        context.cancellation.child_token(),
        shutdown_grace,
    )
    .await
}

fn release_admission_error(error: AgentReleaseError) -> anyhow::Error {
    coded_error(error.code(), error.to_string(), ExitClass::Failure)
}

fn harness_admission_error(error: AgentProtocolHarnessError) -> anyhow::Error {
    coded_error(error.code(), error.to_string(), ExitClass::Failure)
}

async fn verify_required_secrets(
    manifest: &AgentReleaseManifest,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    for requirement in manifest.required_secrets() {
        let injected = match requirement.target() {
            AgentReleaseSecretTarget::Environment => context
                .environment
                .nonempty_var_os(requirement.destination())
                .is_some(),
            AgentReleaseSecretTarget::File => tokio::fs::metadata(requirement.destination())
                .await
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0),
            _ => false,
        };
        if !injected {
            bail!(
                "required Agent release secret slot `{}` was not injected",
                requirement.name()
            );
        }
    }
    Ok(())
}

async fn serve_listener(
    listener: TcpListener,
    harness: Arc<AgentProtocolHarness>,
    cancellation: CancellationToken,
    shutdown_grace: Duration,
) -> anyhow::Result<()> {
    let ready = Arc::new(AtomicBool::new(false));
    let state = HarnessServiceState {
        harness: Arc::clone(&harness),
        ready: Arc::clone(&ready),
    };
    let readiness_path = harness.manifest().health().readiness_path().to_string();
    let liveness_path = harness.manifest().health().liveness_path().to_string();
    let router = Router::new()
        .route(&readiness_path, get(readiness))
        .route(&liveness_path, get(liveness))
        .route(AGENT_PROTOCOL_COMMAND_HTTP_PATH_V1, post(command))
        .route(AGENT_PROTOCOL_EVENT_PAGE_HTTP_PATH_V1, post(event_page))
        .route(AGENT_PROTOCOL_CHANGE_SET_HTTP_PATH_V1, post(change_set))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .with_state(state);

    let drain = CancellationToken::new();
    let server_drain = drain.clone();
    ready.store(true, Ordering::Release);
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(server_drain.cancelled_owned())
            .await
    });

    tokio::select! {
        joined = &mut server => {
            ready.store(false, Ordering::Release);
            close_harness(&harness, shutdown_grace).await?;
            joined.context("Agent Harness server task failed")?
                .context("Agent Harness listener failed")?;
            if cancellation.is_cancelled() {
                Ok(())
            } else {
                bail!("Agent Harness listener stopped unexpectedly")
            }
        }
        _ = cancellation.cancelled() => {
            ready.store(false, Ordering::Release);
            drain.cancel();
            let settle = async {
                let (server_result, ()) = tokio::join!(&mut server, harness.close());
                server_result
                    .context("Agent Harness server task failed during shutdown")?
                    .context("Agent Harness listener failed during shutdown")?;
                Ok::<(), anyhow::Error>(())
            };
            match tokio::time::timeout(shutdown_grace, settle).await {
                Ok(result) => result,
                Err(_) => {
                    server.abort();
                    let _ = server.await;
                    bail!(
                        "Agent Harness exceeded its declared {} second shutdown grace",
                        shutdown_grace.as_secs()
                    )
                }
            }
        }
    }
}

async fn close_harness(
    harness: &AgentProtocolHarness,
    shutdown_grace: Duration,
) -> anyhow::Result<()> {
    tokio::time::timeout(shutdown_grace, harness.close())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Agent Harness exceeded its declared {} second shutdown grace",
                shutdown_grace.as_secs()
            )
        })
}

async fn readiness(State(state): State<HarnessServiceState>) -> Response {
    if state.ready.load(Ordering::Acquire) && !state.harness.is_closed() {
        (
            StatusCode::OK,
            Json(HealthResponse {
                schema: HEALTH_SCHEMA_V1,
                status: "ready",
            }),
        )
            .into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                schema: HEALTH_SCHEMA_V1,
                status: "not_ready",
            }),
        )
            .into_response()
    }
}

async fn liveness() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(HealthResponse {
            schema: HEALTH_SCHEMA_V1,
            status: "live",
        }),
    )
}

async fn command(
    State(state): State<HarnessServiceState>,
    payload: Result<Json<AgentProtocolCommandV1>, JsonRejection>,
) -> Response {
    if !state.ready.load(Ordering::Acquire) {
        return service_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "a3s.code.agent_protocol.harness_closed",
            "Agent Harness is draining",
        );
    }
    let Json(command) = match payload {
        Ok(payload) => payload,
        Err(_) => return invalid_json(),
    };
    match state.harness.execute(&command).await {
        Ok(receipt) => (StatusCode::OK, Json(receipt)).into_response(),
        Err(error) => harness_error(error),
    }
}

async fn event_page(
    State(state): State<HarnessServiceState>,
    payload: Result<Json<AgentProtocolEventPageRequestV1>, JsonRejection>,
) -> Response {
    if !state.ready.load(Ordering::Acquire) {
        return service_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "a3s.code.agent_protocol.harness_closed",
            "Agent Harness is draining",
        );
    }
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(_) => return invalid_json(),
    };
    match state.harness.event_page(&request).await {
        Ok(page) => (StatusCode::OK, Json(page)).into_response(),
        Err(error) => harness_error(error),
    }
}

async fn change_set(
    State(state): State<HarnessServiceState>,
    payload: Result<Json<AgentProtocolChangeSetRequestV1>, JsonRejection>,
) -> Response {
    if !state.ready.load(Ordering::Acquire) {
        return service_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "a3s.code.agent_protocol.harness_closed",
            "Agent Harness is draining",
        );
    }
    let Json(request) = match payload {
        Ok(payload) => payload,
        Err(_) => return invalid_json(),
    };
    match state.harness.change_set(&request).await {
        Ok(change_set) => (StatusCode::OK, Json(change_set)).into_response(),
        Err(error) => harness_error(error),
    }
}

fn invalid_json() -> Response {
    service_error(
        StatusCode::BAD_REQUEST,
        "a3s.code.agent_protocol.invalid_json",
        "request body is not a valid bounded Agent protocol document",
    )
}

fn harness_error(error: AgentProtocolHarnessError) -> Response {
    let status = match &error {
        AgentProtocolHarnessError::Protocol(_) | AgentProtocolHarnessError::Release(_) => {
            StatusCode::BAD_REQUEST
        }
        AgentProtocolHarnessError::Host(host) => match host {
            AgentProtocolHostError::Protocol(_) | AgentProtocolHostError::SequenceOverflow => {
                StatusCode::BAD_REQUEST
            }
            AgentProtocolHostError::RunNotFound => StatusCode::NOT_FOUND,
            AgentProtocolHostError::ReleaseMismatch
            | AgentProtocolHostError::ReleaseProtocolMismatch
            | AgentProtocolHostError::SessionMismatch
            | AgentProtocolHostError::RunUnavailable => StatusCode::CONFLICT,
            AgentProtocolHostError::ChangeSetPending => StatusCode::TOO_EARLY,
            AgentProtocolHostError::ChangeSetUnavailable => StatusCode::UNPROCESSABLE_ENTITY,
            AgentProtocolHostError::Code(_) => StatusCode::INTERNAL_SERVER_ERROR,
        },
        AgentProtocolHarnessError::SessionNotFound => StatusCode::NOT_FOUND,
        AgentProtocolHarnessError::SessionCapacity | AgentProtocolHarnessError::Closed => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        AgentProtocolHarnessError::Code(_) | AgentProtocolHarnessError::Workspace(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    let message = match status {
        StatusCode::BAD_REQUEST => "Agent protocol request is invalid",
        StatusCode::NOT_FOUND => "Agent protocol resource was not found",
        StatusCode::CONFLICT => "Agent protocol identity or state conflicts with the request",
        StatusCode::TOO_EARLY => "Agent run changes are still being captured",
        StatusCode::UNPROCESSABLE_ENTITY => "Agent run has no Git-compatible change set",
        StatusCode::SERVICE_UNAVAILABLE => "Agent Harness is unavailable",
        _ => "Agent Harness could not complete the request",
    };
    service_error(status, error.code(), message)
}

fn service_error(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        Json(ErrorResponse {
            schema: ERROR_SCHEMA_V1,
            error: ErrorBody { code, message },
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_code_core::llm::{
        ContentBlock, LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
    };
    use a3s_code_core::{
        AgentProtocolChangeSetV1, AgentProtocolEventPageV1, AgentProtocolRunIdentityV1,
        AgentProtocolRunStartV1, AgentProtocolRunStateV1, CodeConfig, PlanningMode,
    };
    use async_trait::async_trait;
    use base64::Engine as _;
    use serde_json::json;
    use std::process::Command;
    use tokio::sync::mpsc;

    struct StaticLlmClient;

    #[async_trait]
    impl LlmClient for StaticLlmClient {
        async fn complete(
            &self,
            _messages: &[Message],
            _system: Option<&str>,
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(done_response())
        }

        async fn complete_streaming(
            &self,
            _messages: &[Message],
            _system: Option<&str>,
            _tools: &[ToolDefinition],
            _cancel_token: CancellationToken,
        ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
            let (sender, receiver) = mpsc::channel(1);
            tokio::spawn(async move {
                let _ = sender.send(StreamEvent::Done(done_response())).await;
            });
            Ok(receiver)
        }
    }

    fn done_response() -> LlmResponse {
        LlmResponse {
            message: Message {
                role: "assistant".to_string(),
                content: vec![ContentBlock::Text {
                    text: "HARNESS_OK".to_string(),
                }],
                reasoning_content: None,
            },
            usage: TokenUsage::default(),
            stop_reason: Some("stop".to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }

    fn release_manifest(port: u16) -> AgentReleaseManifest {
        AgentReleaseManifest::parse(&format!(
            r#"
agent_release {{
  schema = "a3s.code.agent-release.v1"
  protocol = "a3s.code.agent.v1"
  artifact {{
    digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111"
    media_type = "application/vnd.oci.image.manifest.v1+json"
  }}
  entrypoint {{
    command = "/usr/bin/a3s"
    args = ["code", "harness", "--manifest", "/app/.a3s/asset.acl"]
  }}
  health {{
    transport = "http"
    port = {port}
    readiness_path = "/health/ready"
    liveness_path = "/health/live"
    shutdown_grace_seconds = 2
  }}
  storage {{
    workspace = "ephemeral"
    cache = "ephemeral"
    persistent_data = "none"
  }}
  capability "runtime.service" {{ level = 1 }}
  capability "secrets.external" {{ level = 1 }}
  capability "workspace.local" {{ level = 1 }}
  provenance "source" {{
    uri = "https://github.com/A3S-Lab/Code"
    digest = "sha256:2222222222222222222222222222222222222222222222222222222222222222"
  }}
}}
"#
        ))
        .expect("admit Harness test manifest")
    }

    fn git(workspace: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(workspace)
            .output()
            .expect("run Git fixture command");
        assert!(
            output.status.success(),
            "Git fixture command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn initialize_git_workspace(workspace: &std::path::Path) {
        git(workspace, &["init"]);
        git(workspace, &["config", "user.name", "A3S Test"]);
        git(workspace, &["config", "user.email", "test@a3s.invalid"]);
        std::fs::write(workspace.join("seed.txt"), "seed\n").expect("write Git seed");
        git(workspace, &["add", "seed.txt"]);
        git(workspace, &["commit", "-m", "seed"]);
    }

    async fn test_harness(
        listener: &TcpListener,
    ) -> (Arc<AgentProtocolHarness>, tempfile::TempDir) {
        let workspace = tempfile::tempdir().expect("create Harness workspace");
        initialize_git_workspace(workspace.path());
        let config = CodeConfig::from_acl(
            r#"
default_model = "openai/test"
providers "openai" {
  apiKey = "not-used"
  baseUrl = "http://127.0.0.1:1"
  models "test" { name = "test" }
}
"#,
        )
        .expect("parse test config");
        let agent = Arc::new(Agent::from_config(config).await.expect("build test Agent"));
        let manifest = release_manifest(listener.local_addr().unwrap().port());
        let harness = AgentProtocolHarness::new(
            manifest,
            agent,
            workspace.path().to_string_lossy().to_string(),
        )
        .expect("build Harness")
        .with_session_options(
            SessionOptions::new()
                .with_planning_mode(PlanningMode::Disabled)
                .with_llm_client(Arc::new(StaticLlmClient)),
        );
        (Arc::new(harness), workspace)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_service_runs_commands_pages_events_and_shuts_down() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind Harness test listener");
        let address = listener.local_addr().unwrap();
        let (harness, _workspace) = test_harness(&listener).await;
        let cancellation = CancellationToken::new();
        let server = tokio::spawn(serve_listener(
            listener,
            Arc::clone(&harness),
            cancellation.clone(),
            Duration::from_secs(2),
        ));
        let client = reqwest::Client::new();

        let ready = client
            .get(format!("http://{address}/health/ready"))
            .send()
            .await
            .expect("read readiness");
        assert_eq!(ready.status().as_u16(), StatusCode::OK.as_u16());
        let ready_body = ready.text().await.unwrap();
        assert!(ready_body.contains(r#""status":"ready""#));
        assert!(!ready_body.contains("PROVIDER_API_KEY"));

        let malformed = client
            .post(format!(
                "http://{address}{AGENT_PROTOCOL_COMMAND_HTTP_PATH_V1}"
            ))
            .header("content-type", "application/json")
            .body(r#"{"action":"start","secret":"must-not-echo"}"#)
            .send()
            .await
            .expect("send malformed command");
        assert_eq!(
            malformed.status().as_u16(),
            StatusCode::BAD_REQUEST.as_u16()
        );
        let malformed_body = malformed.text().await.unwrap();
        assert!(malformed_body.contains("a3s.code.agent_protocol.invalid_json"));
        assert!(!malformed_body.contains("must-not-echo"));

        let identity = AgentProtocolRunIdentityV1 {
            schema: AgentProtocolRunIdentityV1::SCHEMA.to_string(),
            protocol: "a3s.code.agent.v1".to_string(),
            agent_release_identity: harness.agent_release_identity().to_string(),
            session_id: "harness-session".to_string(),
            run_id: "harness-run".to_string(),
        };
        let command = AgentProtocolCommandV1::Start {
            request: AgentProtocolRunStartV1 {
                schema: AgentProtocolRunStartV1::SCHEMA.to_string(),
                request_id: "harness-start".to_string(),
                identity: identity.clone(),
                prompt: "Reply with HARNESS_OK".to_string(),
            },
        };
        let receipt = client
            .post(format!(
                "http://{address}{AGENT_PROTOCOL_COMMAND_HTTP_PATH_V1}"
            ))
            .json(&command)
            .send()
            .await
            .expect("start Harness run");
        assert_eq!(receipt.status().as_u16(), StatusCode::OK.as_u16());

        let request = AgentProtocolEventPageRequestV1 {
            schema: AgentProtocolEventPageRequestV1::SCHEMA.to_string(),
            identity: identity.clone(),
            after_event_sequence: None,
            limit: 64,
        };
        let page = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let response = client
                    .post(format!(
                        "http://{address}{AGENT_PROTOCOL_EVENT_PAGE_HTTP_PATH_V1}"
                    ))
                    .json(&request)
                    .send()
                    .await
                    .expect("read Harness event page");
                assert_eq!(response.status().as_u16(), StatusCode::OK.as_u16());
                let page = response
                    .json::<AgentProtocolEventPageV1>()
                    .await
                    .expect("decode Harness event page");
                if page.state.is_terminal() {
                    break page;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("Harness run must terminate");
        assert_eq!(page.state, AgentProtocolRunStateV1::Completed);
        assert!(page.events.iter().any(|record| {
            record.event.event_type == "agent_end"
                && record.event.payload.to_string().contains("HARNESS_OK")
        }));

        let change_request = AgentProtocolChangeSetRequestV1 {
            schema: AgentProtocolChangeSetRequestV1::SCHEMA.to_string(),
            identity,
        };
        let change_set = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let response = client
                    .post(format!(
                        "http://{address}{AGENT_PROTOCOL_CHANGE_SET_HTTP_PATH_V1}"
                    ))
                    .json(&change_request)
                    .send()
                    .await
                    .expect("read Harness change set");
                if response.status().as_u16() == StatusCode::TOO_EARLY.as_u16() {
                    tokio::task::yield_now().await;
                    continue;
                }
                assert_eq!(response.status().as_u16(), StatusCode::OK.as_u16());
                break response
                    .json::<AgentProtocolChangeSetV1>()
                    .await
                    .expect("decode Harness change set");
            }
        })
        .await
        .expect("Harness change set must settle");
        change_set.validate().expect("valid Harness change set");
        assert!(base64::engine::general_purpose::STANDARD
            .decode(change_set.patch_base64)
            .unwrap()
            .is_empty());

        cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("Harness server shutdown timed out")
            .expect("join Harness server")
            .expect("Harness server shutdown failed");
        assert!(harness.is_closed());
    }

    #[test]
    fn health_documents_are_deliberately_secret_free() {
        let ready = serde_json::to_string(&HealthResponse {
            schema: HEALTH_SCHEMA_V1,
            status: "ready",
        })
        .unwrap();
        let live = serde_json::to_string(&HealthResponse {
            schema: HEALTH_SCHEMA_V1,
            status: "live",
        })
        .unwrap();
        for document in [ready, live] {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&document).unwrap(),
                json!({"schema": HEALTH_SCHEMA_V1, "status": if document.contains("ready") { "ready" } else { "live" }})
            );
            assert!(!document.contains("secret"));
            assert!(!document.contains("identity"));
        }
    }
}
