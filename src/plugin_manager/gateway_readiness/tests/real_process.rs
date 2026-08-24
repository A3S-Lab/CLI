use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use a3s_box_runtime::{
    BoxRuntimeDriver, BoxRuntimeDriverConfig, ExecutionIsolation, LocalExecutionBackend,
};
use a3s_runtime::contract::{ArtifactRef, RuntimeInspection};
use a3s_runtime::{
    FileRuntimeStateStore, ManagedRuntimeClient, ProviderId, RuntimeClient, RuntimeClientRegistry,
    RuntimeDriver, RuntimeProviderFactory, RuntimeResult, RuntimeStateStore,
};
use a3s_use::plugin_lifecycle::{
    PluginLifecycleAction, PluginLifecycleIntent, PluginLifecycleIntentSpec,
    PluginMcpLifecycleHost, PluginToolLifecycleHost, RuntimePluginSurfaceLifecycleHost,
};
use a3s_use::plugin_runtime::{
    plan_mcp_service_release, plan_tool_service_release, RuntimeBindingStore,
    RuntimeProviderAssignment, RuntimeProviderSelector, RuntimeSurfaceContext, RuntimeSurfacePlan,
};
use a3s_use_core::{
    McpReleaseDescriptor, PlanScopeKind, PluginSurfaceKind, PluginSurfaceRef, ToolReleaseDescriptor,
};
use a3s_use_extension::{
    ExtensionManifest, PluginMcpLaunch, PluginMcpSurface, SurfaceActivation, ToolServiceSurface,
    ToolSurface, ToolWorkload,
};
use async_trait::async_trait;

use super::*;
use crate::components::ComponentPaths;

use self::assertions::*;
use self::backend::{
    ExpectedService, QualificationProcessBackend, QualificationProcessCleanup,
    QualificationProcessIdentity, QualificationProcessStore, QualificationServiceKind,
};

mod assertions;
mod backend;
mod child;
mod recovery;

const PACKAGE_V1: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MANIFEST_V1: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PACKAGE_V2: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const MANIFEST_V2: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const PROVIDER_BUILD: &str = "a3s-box/cli-real-process-qualification-v1";

struct ServiceGeneration {
    intent: PluginLifecycleIntent,
    tool: ToolSurface,
    mcp: PluginMcpSurface,
    tool_plan: RuntimeSurfacePlan,
    mcp_plan: RuntimeSurfacePlan,
    package_root: PathBuf,
}

impl ServiceGeneration {
    fn new(
        package_root: PathBuf,
        package_digest: &str,
        manifest_digest: &str,
        generation: u64,
        action: PluginLifecycleAction,
    ) -> Self {
        let releases = package_root.join("releases");
        std::fs::create_dir_all(&releases).unwrap();
        std::fs::write(releases.join("tool.json"), TOOL_SERVICE_RELEASE).unwrap();
        std::fs::write(releases.join("mcp.json"), MCP_SERVICE_RELEASE).unwrap();

        let tool_descriptor =
            ToolReleaseDescriptor::from_json(TOOL_SERVICE_RELEASE.as_bytes()).unwrap();
        let tool_service = ToolServiceSurface {
            release: PathBuf::from("releases/tool.json"),
            base_path: "/api".to_string(),
            contract: None,
        };
        let tool = ToolSurface {
            id: "serve".to_string(),
            activation: SurfaceActivation::Eager,
            optional: false,
            workload: ToolWorkload::Service(tool_service.clone()),
        };
        let tool_context = runtime_context(
            package_digest,
            manifest_digest,
            PluginSurfaceKind::Tool,
            &tool.id,
            generation,
        );
        let tool_plan = plan_tool_service_release(
            tool_context,
            &tool_service,
            &tool_descriptor,
            artifact(
                "worker-service",
                &tool_descriptor.artifact.digest,
                &tool_descriptor.artifact.media_type,
            ),
            qualification_runtime_policy(),
        )
        .unwrap();

        let mcp_descriptor =
            McpReleaseDescriptor::from_json(MCP_SERVICE_RELEASE.as_bytes()).unwrap();
        let mcp = PluginMcpSurface {
            id: "library".to_string(),
            activation: SurfaceActivation::Eager,
            optional: false,
            launch: PluginMcpLaunch::StreamableHttp {
                release: PathBuf::from("releases/mcp.json"),
            },
        };
        let mcp_context = runtime_context(
            package_digest,
            manifest_digest,
            PluginSurfaceKind::Mcp,
            &mcp.id,
            generation,
        );
        let mcp_plan = plan_mcp_service_release(
            mcp_context,
            &mcp,
            &mcp_descriptor,
            artifact(
                "worker-mcp",
                &mcp_descriptor.artifact.digest,
                &mcp_descriptor.artifact.media_type,
            ),
            qualification_runtime_policy(),
        )
        .unwrap();

        let manifest = ExtensionManifest::parse_acl(LIFECYCLE_MANIFEST).unwrap();
        let intent = PluginLifecycleIntent::from_manifest(
            PluginLifecycleIntentSpec {
                operation_id: format!("real-process-{generation}"),
                plan_digest: package_digest.to_string(),
                scope: scope(PlanScopeKind::Workspace),
                package_id: "acme/worker".to_string(),
                package_digest: package_digest.to_string(),
                manifest_digest: manifest_digest.to_string(),
                generation,
                action,
                retained_ui_state_surfaces: Vec::new(),
            },
            &manifest,
        )
        .unwrap();

        Self {
            intent,
            tool,
            mcp,
            tool_plan,
            mcp_plan,
            package_root,
        }
    }

    fn plans(&self) -> Vec<RuntimeSurfacePlan> {
        vec![self.tool_plan.clone(), self.mcp_plan.clone()]
    }
}

fn qualification_runtime_policy() -> RuntimeWorkloadPolicy {
    let mut policy = runtime_policy();
    // Box intentionally does not advertise ephemeral-storage enforcement.
    // This real-process qualification asks only for controls the provider can
    // actually guarantee; storage isolation remains an OCI/microVM gate.
    policy.resources.ephemeral_storage_bytes = None;
    policy
}

fn runtime_context(
    package_digest: &str,
    manifest_digest: &str,
    kind: PluginSurfaceKind,
    surface_id: &str,
    generation: u64,
) -> RuntimeSurfaceContext {
    RuntimeSurfaceContext::new(
        "acme/worker",
        package_digest,
        scope(PlanScopeKind::Workspace),
        manifest_digest,
        PluginSurfaceRef {
            kind,
            id: surface_id.to_string(),
        },
        generation,
    )
    .unwrap()
}

fn artifact(name: &str, digest: &str, media_type: &str) -> ArtifactRef {
    ArtifactRef {
        uri: format!("oci://registry.example/acme/{name}@{digest}"),
        digest: digest.to_string(),
        media_type: media_type.to_string(),
    }
}

fn expected_services(generations: &[&ServiceGeneration]) -> BTreeMap<String, ExpectedService> {
    generations
        .iter()
        .flat_map(|generation| {
            [
                (&generation.tool_plan, QualificationServiceKind::Tool),
                (&generation.mcp_plan, QualificationServiceKind::Mcp),
            ]
        })
        .map(|(plan, kind)| {
            let port = plan.spec().network.ports.first().unwrap().container_port;
            (
                plan.spec().unit_id.clone(),
                ExpectedService {
                    kind,
                    runtime_generation: plan.spec().generation,
                    container_port: NonZeroU16::new(port).unwrap(),
                },
            )
        })
        .collect()
}

struct SharedRuntimeProviderFactory {
    provider_id: ProviderId,
    client: Arc<dyn RuntimeClient>,
}

#[async_trait]
impl RuntimeProviderFactory for SharedRuntimeProviderFactory {
    fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    async fn create(&self) -> RuntimeResult<Arc<dyn RuntimeClient>> {
        Ok(self.client.clone())
    }
}

struct QualificationProvider {
    provider_id: ProviderId,
    registry: Arc<RuntimeClientRegistry>,
    client: Arc<dyn RuntimeClient>,
    bindings: RuntimeBindingStore,
}

impl QualificationProvider {
    fn start(
        root: &Path,
        process_store: QualificationProcessStore,
        expected: BTreeMap<String, ExpectedService>,
    ) -> Self {
        let backend = Arc::new(QualificationProcessBackend::new(process_store, expected).unwrap());
        let connector = Arc::new(backend.connector());
        let driver = BoxRuntimeDriver::new_for_runtime_provider_qualification(
            BoxRuntimeDriverConfig {
                home_dir: root.join("box-runtime"),
                secret_root: root.join("runtime-secrets"),
                control_timeout: Duration::from_secs(10),
                task_poll_interval: Duration::from_millis(10),
            },
            backend as Arc<dyn LocalExecutionBackend>,
            connector,
            ExecutionIsolation::Sandbox,
            PROVIDER_BUILD,
        )
        .unwrap();
        let provider_id = driver.provider_id().clone();
        let state: Arc<dyn RuntimeStateStore> = Arc::new(FileRuntimeStateStore::new(
            root.join("managed-runtime-state"),
        ));
        let driver: Arc<dyn RuntimeDriver> = Arc::new(driver);
        let client: Arc<dyn RuntimeClient> = Arc::new(ManagedRuntimeClient::new(state, driver));
        let mut registry = RuntimeClientRegistry::new();
        registry
            .register(Arc::new(SharedRuntimeProviderFactory {
                provider_id: provider_id.clone(),
                client: client.clone(),
            }))
            .unwrap();
        Self {
            provider_id,
            registry: Arc::new(registry),
            client,
            bindings: RuntimeBindingStore::new(root.join("use-state")),
        }
    }

    async fn lifecycle(
        &self,
        generation: &ServiceGeneration,
        gateway: Arc<GatewayRuntimeServiceHost>,
    ) -> RuntimePluginSurfaceLifecycleHost {
        let plans = generation.plans();
        let assignments = plans
            .iter()
            .map(|plan| {
                RuntimeProviderAssignment::new(
                    plan.surface(),
                    self.provider_id.as_str().to_string(),
                )
                .unwrap()
            })
            .collect();
        let selection = RuntimeProviderSelector::new(self.registry.as_ref())
            .select(plans, assignments)
            .await
            .unwrap();
        RuntimePluginSurfaceLifecycleHost::new(
            generation.package_root.clone(),
            selection,
            self.registry.clone(),
            self.bindings.clone(),
            gateway,
        )
        .with_deadline_at_ms(Some(epoch_millis("test.clock").unwrap() + 30_000))
        .unwrap()
    }
}

#[tokio::test]
async fn real_box_services_upgrade_restart_drain_and_remove_without_residue() {
    let temporary = tempfile::tempdir().unwrap();
    let v1 = ServiceGeneration::new(
        temporary.path().join("package-v1"),
        PACKAGE_V1,
        MANIFEST_V1,
        7,
        PluginLifecycleAction::Install,
    );
    let v2 = ServiceGeneration::new(
        temporary.path().join("package-v2"),
        PACKAGE_V2,
        MANIFEST_V2,
        8,
        PluginLifecycleAction::Upgrade,
    );
    let process_store =
        QualificationProcessStore::new(temporary.path().join("processes/state.json"));
    let _cleanup = QualificationProcessCleanup::new(&process_store);
    let provider = QualificationProvider::start(
        temporary.path(),
        process_store.clone(),
        expected_services(&[&v1, &v2]),
    );
    let gateway_address = reserve_loopback_address();
    let paths = ComponentPaths::for_test(temporary.path());
    let gateway = start_gateway(gateway_address, &paths).await;
    let v1_host = provider.lifecycle(&v1, gateway.clone()).await;
    let v2_host = provider.lifecycle(&v2, gateway.clone()).await;

    prepare_generation(&v1_host, &v1, '1', '2').await;
    let v1_tool_receipt = service_receipt(&provider.bindings, &v1.intent, &v1.tool_plan).await;
    let v1_mcp_receipt = service_receipt(&provider.bindings, &v1.intent, &v1.mcp_plan).await;
    assert_tool_generation(gateway_address, &v1_tool_receipt, 7, &v1.tool_plan).await;
    assert_mcp_generation(gateway_address, &v1_mcp_receipt, 7, &v1.mcp_plan).await;

    prepare_generation(&v2_host, &v2, '3', '4').await;
    let v2_tool_receipt = service_receipt(&provider.bindings, &v2.intent, &v2.tool_plan).await;
    let v2_mcp_receipt = service_receipt(&provider.bindings, &v2.intent, &v2.mcp_plan).await;
    assert_eq!(process_store.active_records().await.unwrap().len(), 4);
    assert_tool_generation(gateway_address, &v2_tool_receipt, 8, &v2.tool_plan).await;
    assert_mcp_generation(gateway_address, &v2_mcp_receipt, 8, &v2.mcp_plan).await;
    assert_tool_generation(gateway_address, &v1_tool_receipt, 7, &v1.tool_plan).await;
    assert_mcp_generation(gateway_address, &v1_mcp_receipt, 7, &v1.mcp_plan).await;

    drop(v1_host);
    drop(v2_host);
    gateway.shutdown().await;
    drop(gateway);

    let gateway = start_gateway(gateway_address, &paths).await;
    assert_tool_generation(gateway_address, &v1_tool_receipt, 7, &v1.tool_plan).await;
    assert_mcp_generation(gateway_address, &v2_mcp_receipt, 8, &v2.mcp_plan).await;
    let v1_host = provider.lifecycle(&v1, gateway.clone()).await;
    let v2_host = provider.lifecycle(&v2, gateway.clone()).await;
    prepare_generation(&v1_host, &v1, '1', '2').await;
    prepare_generation(&v2_host, &v2, '3', '4').await;
    assert_eq!(process_store.active_records().await.unwrap().len(), 4);

    v1_host
        .stop_tool(&v1.intent, &v1.tool, &key('5'))
        .await
        .unwrap();
    assert_route_hidden(gateway_address, &v1_tool_receipt, "/api").await;
    v1_host
        .remove_tool(&v1.intent, &v1.tool, &key('6'))
        .await
        .unwrap();
    assert_binding_removed(gateway.as_ref(), &v1_tool_receipt);
    assert_no_receipt(&provider.bindings, &v1.intent, &v1.tool_plan).await;

    v1_host
        .remove_mcp(&v1.intent, &v1.mcp, &key('7'))
        .await
        .unwrap();
    assert_binding_removed(gateway.as_ref(), &v1_mcp_receipt);
    assert_no_receipt(&provider.bindings, &v1.intent, &v1.mcp_plan).await;
    assert_eq!(process_store.active_records().await.unwrap().len(), 2);
    assert_tool_generation(gateway_address, &v2_tool_receipt, 8, &v2.tool_plan).await;
    assert_mcp_generation(gateway_address, &v2_mcp_receipt, 8, &v2.mcp_plan).await;

    v2_host
        .stop_tool(&v2.intent, &v2.tool, &key('8'))
        .await
        .unwrap();
    assert_route_hidden(gateway_address, &v2_tool_receipt, "/api").await;
    v2_host
        .remove_tool(&v2.intent, &v2.tool, &key('9'))
        .await
        .unwrap();
    v2_host
        .stop_mcp(&v2.intent, &v2.mcp, &key('a'))
        .await
        .unwrap();
    assert_route_hidden(gateway_address, &v2_mcp_receipt, "/mcp").await;
    v2_host
        .remove_mcp(&v2.intent, &v2.mcp, &key('b'))
        .await
        .unwrap();

    for (intent, plan, receipt) in [
        (&v2.intent, &v2.tool_plan, &v2_tool_receipt),
        (&v2.intent, &v2.mcp_plan, &v2_mcp_receipt),
    ] {
        assert_binding_removed(gateway.as_ref(), receipt);
        assert_no_receipt(&provider.bindings, intent, plan).await;
        match provider.client.inspect(&plan.spec().unit_id).await.unwrap() {
            RuntimeInspection::NotFound { unit_id, .. } => {
                assert_eq!(unit_id, plan.spec().unit_id)
            }
            observation => panic!("removed Runtime unit remained visible: {observation:?}"),
        }
    }
    assert!(process_store.active_records().await.unwrap().is_empty());
    gateway.shutdown().await;
}
