use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use a3s_runtime::contract::{
    ArtifactRef, IsolationLevel, RuntimeEvidence, RuntimeHealthObservation, RuntimeHealthState,
    RuntimeObservation, RuntimeServiceEndpoint,
};
use a3s_use::plugin_lifecycle::{
    PluginLifecycleAction, PluginLifecycleIntent, PluginLifecycleIntentSpec,
};
use a3s_use::plugin_runtime::{
    plan_mcp_service_release, plan_tool_service_release, RuntimeResourcePolicy,
    RuntimeServiceBindingReceipt, RuntimeServiceReadinessEvidence, RuntimeSurfaceContext,
    RuntimeWorkloadPolicy, RUNTIME_SERVICE_BINDING_SCHEMA,
};
use a3s_use_core::{
    McpReleaseDescriptor, PlanEnforcementProfile, PlanQualifiedSurfaceRef, PlanScope,
    PlanScopeKind, PluginSurfaceKind, PluginSurfaceRef, ToolReleaseDescriptor,
};
use a3s_use_extension::{
    ExtensionManifest, PluginMcpLaunch, PluginMcpSurface, SurfaceActivation, ToolServiceSurface,
    ToolSurface, ToolWorkload,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};

use super::*;

fn surface(kind: PluginSurfaceKind) -> PlanQualifiedSurfaceRef {
    PlanQualifiedSurfaceRef {
        package_id: "acme/worker".to_string(),
        surface: PluginSurfaceRef {
            kind,
            id: "serve".to_string(),
        },
    }
}

fn scope(kind: PlanScopeKind) -> PlanScope {
    PlanScope {
        kind,
        id: "workspace-7".to_string(),
    }
}

const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const LIFECYCLE_MANIFEST: &str = r#"
extension "acme/worker" {
  schema_version = 3
  version        = "1.0.0"
  route          = "worker"
  requires_use   = ">=0.3.0, <0.4.0"
  actions        = ["execute"]

  repository {
    url      = "https://github.com/acme/worker"
    revision = "1234567890abcdef1234567890abcdef12345678"
  }

  tool "serve" {
    workload   = "service"
    interface  = "http"
    release    = "releases/tool.json"
    base_path  = "/api"
    activation = "eager"
    optional   = false
  }

  mcp "library" {
    transport  = "streamable-http"
    release    = "releases/mcp.json"
    activation = "eager"
    optional   = false
  }
}
"#;
const TOOL_SERVICE_RELEASE: &str = r#"{"artifact":{"digest":"sha256:8888888888888888888888888888888888888888888888888888888888888888","mediaType":"application/vnd.oci.image.manifest.v1+json","sizeBytes":2097152},"compatibility":[{"component":"a3s-runtime","versionRequirement":">=0.3.0, <0.4.0"},{"component":"a3s-use","versionRequirement":">=0.3.0, <0.4.0"}],"dependencies":[],"kind":"tool","name":"acme/worker-service","provenance":{"buildOperationId":"test:worker-service","builderId":"test:a3s-cli","commitSha":"1234567890abcdef1234567890abcdef12345678","manifestDigest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","sourceRepository":"https://github.com/acme/worker.git"},"schema":"a3s.use.tool-release.v1","version":"1.0.0","workload":{"apiContractDigest":"sha256:9999999999999999999999999999999999999999999999999999999999999999","basePath":"/api","class":"service","health":{"failureThreshold":2,"intervalMs":20,"path":"/healthz","successThreshold":1,"timeoutMs":10},"interface":"http","network":"private","port":8080,"portName":"http","shutdownGraceMs":1000,"startupTimeoutMs":5000}}"#;
const MCP_SERVICE_RELEASE: &str = r#"{"artifact":{"digest":"sha256:9999999999999999999999999999999999999999999999999999999999999999","mediaType":"application/vnd.oci.image.manifest.v1+json","sizeBytes":1048576},"compatibility":[{"component":"a3s-runtime","versionRequirement":">=0.3.0, <0.4.0"},{"component":"a3s-use","versionRequirement":">=0.3.0, <0.4.0"}],"dependencies":[],"kind":"mcp","name":"acme/worker-mcp","provenance":{"buildOperationId":"test:worker-mcp","builderId":"test:a3s-cli","commitSha":"1234567890abcdef1234567890abcdef12345678","manifestDigest":"sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","sourceRepository":"https://github.com/acme/worker.git"},"schema":"a3s.use.mcp-release.v1","service":{"endpointPath":"/mcp","health":{"failureThreshold":2,"intervalMs":20,"path":"/healthz","successThreshold":1,"timeoutMs":10},"port":8080,"portName":"mcp","protocolVersion":"2025-06-18","shutdownGraceMs":1000,"startupTimeoutMs":5000,"transport":"streamable-http"},"version":"1.0.0"}"#;

fn runtime_policy() -> RuntimeWorkloadPolicy {
    RuntimeWorkloadPolicy {
        isolation: IsolationLevel::Sandbox,
        resources: RuntimeResourcePolicy {
            cpu_millis: 500,
            memory_bytes: 256 * 1024 * 1024,
            pids: 64,
            ephemeral_storage_bytes: Some(512 * 1024 * 1024),
        },
        mounts: Vec::new(),
        secrets: Vec::new(),
        non_secret_environment: BTreeMap::new(),
        working_directory: None,
    }
}

fn tool_plan() -> (ToolSurface, a3s_use::plugin_runtime::RuntimeSurfacePlan) {
    tool_plan_for(PlanScopeKind::Workspace, 7)
}

fn tool_plan_for(
    scope_kind: PlanScopeKind,
    generation: u64,
) -> (ToolSurface, a3s_use::plugin_runtime::RuntimeSurfacePlan) {
    let descriptor = ToolReleaseDescriptor::from_json(TOOL_SERVICE_RELEASE.as_bytes()).unwrap();
    let service = ToolServiceSurface {
        release: PathBuf::from("releases/tool.json"),
        base_path: "/api".to_string(),
        contract: None,
    };
    let surface = ToolSurface {
        id: "serve".to_string(),
        activation: SurfaceActivation::Eager,
        optional: false,
        workload: ToolWorkload::Service(service.clone()),
    };
    let context = RuntimeSurfaceContext::new(
        "acme/worker",
        DIGEST_A,
        scope(scope_kind),
        DIGEST_B,
        PluginSurfaceRef {
            kind: PluginSurfaceKind::Tool,
            id: surface.id.clone(),
        },
        generation,
    )
    .unwrap();
    let plan = plan_tool_service_release(
        context,
        &service,
        &descriptor,
        ArtifactRef {
            uri: format!(
                "oci://registry.example/acme/worker@{}",
                descriptor.artifact.digest
            ),
            digest: descriptor.artifact.digest.clone(),
            media_type: descriptor.artifact.media_type.clone(),
        },
        runtime_policy(),
    )
    .unwrap();
    (surface, plan)
}

fn mcp_plan() -> (
    PluginMcpSurface,
    a3s_use::plugin_runtime::RuntimeSurfacePlan,
) {
    let descriptor = McpReleaseDescriptor::from_json(MCP_SERVICE_RELEASE.as_bytes()).unwrap();
    let surface = PluginMcpSurface {
        id: "library".to_string(),
        activation: SurfaceActivation::Eager,
        optional: false,
        launch: PluginMcpLaunch::StreamableHttp {
            release: PathBuf::from("releases/mcp.json"),
        },
    };
    let context = RuntimeSurfaceContext::new(
        "acme/worker",
        DIGEST_A,
        scope(PlanScopeKind::Workspace),
        DIGEST_B,
        PluginSurfaceRef {
            kind: PluginSurfaceKind::Mcp,
            id: surface.id.clone(),
        },
        7,
    )
    .unwrap();
    let plan = plan_mcp_service_release(
        context,
        &surface,
        &descriptor,
        ArtifactRef {
            uri: format!(
                "oci://registry.example/acme/worker@{}",
                descriptor.artifact.digest
            ),
            digest: descriptor.artifact.digest.clone(),
            media_type: descriptor.artifact.media_type.clone(),
        },
        runtime_policy(),
    )
    .unwrap();
    (surface, plan)
}

fn lifecycle_intent() -> PluginLifecycleIntent {
    lifecycle_intent_for(PlanScopeKind::Workspace, 7)
}

fn lifecycle_intent_for(scope_kind: PlanScopeKind, generation: u64) -> PluginLifecycleIntent {
    let manifest = ExtensionManifest::parse_acl(LIFECYCLE_MANIFEST).unwrap();
    PluginLifecycleIntent::from_manifest(
        PluginLifecycleIntentSpec {
            operation_id: format!("operation-{}-{generation}", scope_kind.as_str()),
            plan_digest: DIGEST_A.to_string(),
            scope: scope(scope_kind),
            package_id: "acme/worker".to_string(),
            package_digest: DIGEST_A.to_string(),
            manifest_digest: DIGEST_B.to_string(),
            generation,
            action: PluginLifecycleAction::Install,
            retained_ui_state_surfaces: Vec::new(),
        },
        &manifest,
    )
    .unwrap()
}

fn running_observation(
    plan: &a3s_use::plugin_runtime::RuntimeSurfacePlan,
    endpoint: &RuntimeServiceEndpoint,
) -> RuntimeObservation {
    let now = epoch_millis("test.clock").unwrap();
    let mut claims = BTreeMap::new();
    endpoint.insert_claim(&mut claims).unwrap();
    let spec_digest = plan.spec().digest().unwrap();
    RuntimeObservation {
        schema: RuntimeObservation::SCHEMA.to_string(),
        unit_id: plan.spec().unit_id.clone(),
        generation: plan.spec().generation,
        spec_digest: spec_digest.clone(),
        class: plan.spec().class,
        state: a3s_runtime::contract::RuntimeUnitState::Running,
        provider_resource_id: Some("test-runtime-service".to_string()),
        provider_build: Some("test-runtime-build".to_string()),
        observed_at_ms: now,
        started_at_ms: Some(now.saturating_sub(1)),
        finished_at_ms: None,
        health: Some(RuntimeHealthObservation {
            state: RuntimeHealthState::Healthy,
            checked_at_ms: now,
            message: None,
        }),
        outputs: Vec::new(),
        usage: None,
        evidence: Some(RuntimeEvidence {
            provider_build: "test-runtime-build".to_string(),
            spec_digest,
            semantics_profile_digest: plan.spec().semantics_profile_digest.clone(),
            claims,
        }),
        provider_attestation: None,
        failure: None,
    }
}

fn tool_receipt(
    plan: &a3s_use::plugin_runtime::RuntimeSurfacePlan,
    observation: &RuntimeObservation,
    endpoint_ref: RuntimeEndpointRef,
) -> RuntimeServiceBindingReceipt {
    RuntimeServiceBindingReceipt {
        schema: RUNTIME_SERVICE_BINDING_SCHEMA.to_string(),
        surface: plan.surface(),
        package_digest: DIGEST_A.to_string(),
        scope: plan.context().scope().clone(),
        descriptor_digest: plan.descriptor_digest().to_string(),
        provider_id: "test-runtime".to_string(),
        provider_build_id: "test-runtime-build".to_string(),
        capability_digest: DIGEST_B.to_string(),
        enforcement: PlanEnforcementProfile::Sandbox,
        unit_id: observation.unit_id.clone(),
        generation: observation.generation,
        spec_digest: observation.spec_digest.clone(),
        semantics_profile_digest: plan.spec().semantics_profile_digest.clone().unwrap(),
        endpoint_ref,
        runtime_started_at_ms: observation.started_at_ms.unwrap(),
        observation_revision: observation.observed_at_ms,
        last_healthy_at_ms: observation.observed_at_ms,
        contract: plan.contract().clone(),
        readiness: RuntimeServiceReadinessEvidence::HttpHealthy,
    }
}

#[derive(Clone)]
struct McpServerState {
    initializes: Arc<AtomicUsize>,
    initialized: Arc<AtomicUsize>,
}

async fn mcp_post(
    State(state): State<McpServerState>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    match request.get("method").and_then(serde_json::Value::as_str) {
        Some("initialize")
            if request["params"]["protocolVersion"]
                == serde_json::Value::String("2025-06-18".to_string()) =>
        {
            state.initializes.fetch_add(1, Ordering::SeqCst);
            let mut response = Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request["id"].clone(),
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "serverInfo": { "name": "a3s-test-mcp", "version": "1.0.0" }
                }
            }))
            .into_response();
            response.headers_mut().insert(
                "mcp-session-id",
                axum::http::HeaderValue::from_static("readiness-session"),
            );
            response
        }
        Some("notifications/initialized") => {
            state.initialized.fetch_add(1, Ordering::SeqCst);
            StatusCode::ACCEPTED.into_response()
        }
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

#[test]
fn target_identity_is_stable_and_scope_kind_isolated() {
    let first = managed_target(
        &surface(PluginSurfaceKind::Tool),
        &scope(PlanScopeKind::Workspace),
        "service/acme-worker/7",
        11,
    )
    .unwrap();
    let replay = managed_target(
        &surface(PluginSurfaceKind::Tool),
        &scope(PlanScopeKind::Workspace),
        "service/acme-worker/7",
        11,
    )
    .unwrap();
    let user = managed_target(
        &surface(PluginSurfaceKind::Tool),
        &scope(PlanScopeKind::User),
        "service/acme-worker/7",
        11,
    )
    .unwrap();

    assert_eq!(first, replay);
    assert_eq!(
        first.target_id.to_string(),
        "632d3c36-6f72-515c-8c97-fc8e1d5bd3e8"
    );
    assert_ne!(first.target_id, user.target_id);
    assert!(!first.target_id.is_nil());
}

#[test]
fn target_identity_is_surface_and_generation_specific() {
    let tool = managed_target(
        &surface(PluginSurfaceKind::Tool),
        &scope(PlanScopeKind::Workspace),
        "service/acme-worker/7",
        11,
    )
    .unwrap();
    let mcp = managed_target(
        &surface(PluginSurfaceKind::Mcp),
        &scope(PlanScopeKind::Workspace),
        "service/acme-worker/7",
        11,
    )
    .unwrap();
    let next = managed_target(
        &surface(PluginSurfaceKind::Tool),
        &scope(PlanScopeKind::Workspace),
        "service/acme-worker/7",
        12,
    )
    .unwrap();

    assert_ne!(tool.target_id, mcp.target_id);
    assert_ne!(tool.target_id, next.target_id);
}

#[test]
fn mcp_probe_accepts_only_credential_free_loopback_http() {
    for endpoint in [
        "http://127.0.0.1:43129/_a3s/runtime/example/mcp",
        "http://[::1]:43129/_a3s/runtime/example/mcp",
    ] {
        validate_gateway_endpoint(endpoint).unwrap();
    }
    for endpoint in [
        "https://127.0.0.1:43129/mcp",
        "http://localhost:43129/mcp",
        "http://127.0.0.1:43129/mcp?token=secret",
        "http://user@127.0.0.1:43129/mcp",
        "http://192.0.2.10:43129/mcp",
        "not-a-url",
    ] {
        assert!(validate_gateway_endpoint(endpoint).is_err(), "{endpoint}");
    }
}

#[test]
fn expired_epoch_deadline_fails_before_gateway_mutation() {
    let error = deadline_from_epoch_ms(Some(1), "use.plugin.gateway_bind_failed").unwrap_err();
    assert_eq!(error.code, "use.plugin.gateway_bind_failed");
    assert!(error.message.contains("expired"));
}

#[tokio::test]
async fn tool_service_routes_replay_drain_and_remove_by_exact_receipt() {
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream_listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        let app = Router::new()
            .route("/healthz", get(|| async { StatusCode::OK }))
            .route("/api", get(|| async { "generation-7" }));
        axum::serve(upstream_listener, app).await.unwrap();
    });
    let gateway_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let gateway_address = gateway_listener.local_addr().unwrap();
    drop(gateway_listener);
    let temporary = tempfile::tempdir().unwrap();
    let paths = ComponentPaths::for_test(temporary.path());
    let host = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: gateway_address,
        },
        &paths,
    )
    .await
    .unwrap();
    let (surface, plan) = tool_plan();
    let runtime_endpoint =
        RuntimeServiceEndpoint::node_local_tcp("http", upstream_address.port()).unwrap();
    let observation = running_observation(&plan, &runtime_endpoint);
    let intent = lifecycle_intent();
    let deadline = epoch_millis("test.clock").unwrap() + 5_000;
    let bind_key = format!("sha256:{}", "1".repeat(64));

    let endpoint_ref = host
        .bind_tool_service(
            &intent,
            &surface,
            &plan,
            &observation,
            &runtime_endpoint,
            &bind_key,
            Some(deadline),
        )
        .await
        .unwrap();
    let replayed = host
        .bind_tool_service(
            &intent,
            &surface,
            &plan,
            &observation,
            &runtime_endpoint,
            &bind_key,
            Some(deadline),
        )
        .await
        .unwrap();
    assert_eq!(endpoint_ref, replayed);
    let binding_id = endpoint_ref
        .as_str()
        .strip_prefix("gateway:managed-services/")
        .unwrap();
    let endpoint = format!("http://{gateway_address}/_a3s/runtime/{binding_id}/api");
    let client = reqwest_rmcp::Client::builder().no_proxy().build().unwrap();
    let response = client.get(&endpoint).send().await.unwrap();
    assert_eq!(response.status(), reqwest_rmcp::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "generation-7");

    let target = managed_target(
        &plan.surface(),
        plan.context().scope(),
        &observation.unit_id,
        observation.generation,
    )
    .unwrap();
    let identity = ManagedServiceBindingIdentity::new(endpoint_ref.as_str(), target).unwrap();
    assert!(host
        .gateway()
        .managed_service_status(&identity)
        .unwrap()
        .unwrap()
        .ready());
    let receipt = tool_receipt(&plan, &observation, endpoint_ref);
    host.shutdown().await;
    drop(host);

    let host = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: gateway_address,
        },
        &paths,
    )
    .await
    .unwrap();
    assert!(host
        .gateway()
        .managed_service_status(&identity)
        .unwrap()
        .unwrap()
        .ready());
    let recovered = host
        .bind_tool_service(
            &intent,
            &surface,
            &plan,
            &observation,
            &runtime_endpoint,
            &bind_key,
            Some(deadline),
        )
        .await
        .unwrap();
    assert_eq!(recovered.as_str(), receipt.endpoint_ref.as_str());
    assert_eq!(
        client.get(&endpoint).send().await.unwrap().status(),
        reqwest_rmcp::StatusCode::OK
    );
    host.drain_service(
        &intent,
        &receipt,
        &format!("sha256:{}", "2".repeat(64)),
        Some(deadline),
    )
    .await
    .unwrap();
    assert_eq!(
        client.get(&endpoint).send().await.unwrap().status(),
        reqwest_rmcp::StatusCode::NOT_FOUND
    );
    host.remove_service(
        &intent,
        &receipt,
        &format!("sha256:{}", "3".repeat(64)),
        Some(deadline),
    )
    .await
    .unwrap();
    assert!(host
        .gateway()
        .managed_service_status(&identity)
        .unwrap()
        .is_none());

    host.shutdown().await;
    upstream.abort();
    let _ = upstream.await;
}

#[tokio::test]
async fn mcp_service_initializes_through_the_returned_gateway_endpoint() {
    let state = McpServerState {
        initializes: Arc::new(AtomicUsize::new(0)),
        initialized: Arc::new(AtomicUsize::new(0)),
    };
    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_address = upstream_listener.local_addr().unwrap();
    let upstream_state = state.clone();
    let upstream = tokio::spawn(async move {
        let app = Router::new()
            .route("/healthz", get(|| async { StatusCode::OK }))
            .route("/mcp", axum::routing::post(mcp_post))
            .with_state(upstream_state);
        axum::serve(upstream_listener, app).await.unwrap();
    });
    let gateway_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let gateway_address = gateway_listener.local_addr().unwrap();
    drop(gateway_listener);
    let temporary = tempfile::tempdir().unwrap();
    let paths = ComponentPaths::for_test(temporary.path());
    let host = GatewayRuntimeServiceHost::start(
        PrivateGatewayConfig {
            address: gateway_address,
        },
        &paths,
    )
    .await
    .unwrap();
    let (surface, plan) = mcp_plan();
    let runtime_endpoint =
        RuntimeServiceEndpoint::node_local_tcp("mcp", upstream_address.port()).unwrap();
    let observation = running_observation(&plan, &runtime_endpoint);
    let intent = lifecycle_intent();
    let deadline = epoch_millis("test.clock").unwrap() + 5_000;
    let bind_key = format!("sha256:{}", "4".repeat(64));

    host.bind_mcp_service(
        &intent,
        &surface,
        &plan,
        &observation,
        &runtime_endpoint,
        &bind_key,
        Some(deadline),
    )
    .await
    .unwrap();

    assert_eq!(state.initializes.load(Ordering::SeqCst), 1);
    assert_eq!(state.initialized.load(Ordering::SeqCst), 1);
    let replayed = host
        .bind_route(
            &intent,
            PluginSurfaceKind::Mcp,
            &surface.id,
            &plan,
            &observation,
            &runtime_endpoint,
            &bind_key,
            Some(deadline),
            "/mcp",
        )
        .await
        .unwrap();
    assert!(replayed.replayed());

    host.shutdown().await;
    upstream.abort();
    let _ = upstream.await;
}

mod negative;
