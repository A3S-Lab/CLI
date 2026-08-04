//! A3S Code host composition for one cognitive-package lifecycle generation.

use std::path::PathBuf;
use std::sync::Arc;

use a3s_runtime::contract::RuntimeObservation;
use a3s_use::cognitive_package::CognitivePackageLifecycleFactory;
use a3s_use::flow_runtime::{A3sFlowLifecycleHost, FlowRuntimeBindingStore};
use a3s_use::plugin_lifecycle::{
    ExtensionCapabilityLifecycleHost, ExtensionPackageLifecycleHost, PluginLifecycleCoordinator,
    PluginLifecycleEvidence, PluginLifecycleHosts, PluginLifecycleIntent,
    PluginLifecycleJournalStore, PluginMcpServiceReadiness, PluginOkfLifecycleHost,
    PluginPackageLifecycleHost, PluginRuntimeServiceReadinessHost,
    RuntimePluginSurfaceLifecycleHost, StaticPluginSurfaceLifecycleHost,
};
use a3s_use::plugin_runtime::{
    RuntimeBindingStore, RuntimeEndpointRef, RuntimeProviderSelection, RuntimeSurfacePlan,
};
use a3s_use_core::{UseError, UseResult};
use a3s_use_extension::{
    ExtensionLifecyclePackage, ExtensionManifest, ExtensionRegistry, PluginMcpLaunch,
    PluginMcpSurface, PluginOkfSurface, ToolSurface, ToolTaskSource, ToolWorkload,
};
use async_trait::async_trait;

const FLOW_COMPILER_ENV: &str = "A3S_FLOW_NATIVE_TS_COMPILER";

/// Code's production package host.
///
/// Executable Tool Tasks and stdio MCP use the shared Runtime lifecycle host,
/// Skill/UI use immutable static validation, and Flow uses the real
/// `A3sFlowLifecycleHost`. Runtime Services, HTTP MCP, and OKF stay fail-closed
/// until Code receives their concrete Runtime/Gateway/Knowledge adapters.
#[derive(Debug, Clone)]
pub(super) struct CodeCognitivePackageLifecycleFactory {
    flow_compiler_binary: PathBuf,
}

impl Default for CodeCognitivePackageLifecycleFactory {
    fn default() -> Self {
        let flow_compiler_binary = std::env::var_os(FLOW_COMPILER_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("a3s-flow-native-compiler"));
        Self {
            flow_compiler_binary,
        }
    }
}

impl CognitivePackageLifecycleFactory for CodeCognitivePackageLifecycleFactory {
    fn name(&self) -> &'static str {
        "a3s-code"
    }

    fn validate_manifest(&self, manifest: &ExtensionManifest) -> UseResult<()> {
        validate_available_hosts(manifest, &self.flow_compiler_binary)
    }

    fn install_coordinator(
        &self,
        registry: ExtensionRegistry,
        candidate: ExtensionLifecyclePackage,
        package_root: PathBuf,
    ) -> UseResult<PluginLifecycleCoordinator> {
        let paths = registry.paths().clone();
        let package = Arc::new(ExtensionPackageLifecycleHost::new(
            registry.clone(),
            candidate,
        ));
        Ok(coordinator(
            registry,
            package,
            package_root,
            &paths,
            &self.flow_compiler_binary,
        ))
    }

    fn published_install_coordinator(
        &self,
        registry: ExtensionRegistry,
        package_root: PathBuf,
    ) -> UseResult<PluginLifecycleCoordinator> {
        installed_coordinator(registry, package_root, &self.flow_compiler_binary)
    }

    fn uninstall_coordinator(
        &self,
        registry: ExtensionRegistry,
        package_root: PathBuf,
    ) -> UseResult<PluginLifecycleCoordinator> {
        installed_coordinator(registry, package_root, &self.flow_compiler_binary)
    }
}

fn installed_coordinator(
    registry: ExtensionRegistry,
    package_root: PathBuf,
    flow_compiler_binary: &std::path::Path,
) -> UseResult<PluginLifecycleCoordinator> {
    let paths = registry.paths().clone();
    let package = Arc::new(ExtensionPackageLifecycleHost::for_installed(
        registry.clone(),
    ));
    Ok(coordinator(
        registry,
        package,
        package_root,
        &paths,
        flow_compiler_binary,
    ))
}

fn coordinator(
    registry: ExtensionRegistry,
    package: Arc<dyn PluginPackageLifecycleHost>,
    package_root: PathBuf,
    paths: &a3s_use_extension::ExtensionPaths,
    flow_compiler_binary: &std::path::Path,
) -> PluginLifecycleCoordinator {
    let capability = Arc::new(ExtensionCapabilityLifecycleHost::new(registry));
    let runtime = Arc::new(RuntimePluginSurfaceLifecycleHost::new(
        &package_root,
        RuntimeProviderSelection::default(),
        RuntimeBindingStore::from_extension_paths(paths),
        Arc::new(UnavailableRuntimeServiceHost),
    ));
    let static_surfaces = Arc::new(StaticPluginSurfaceLifecycleHost::new(&package_root));
    let flow = Arc::new(A3sFlowLifecycleHost::new(
        &package_root,
        flow_compiler_binary,
        paths.state_root().join("flow-runtime/cache"),
        FlowRuntimeBindingStore::from_extension_paths(paths),
    ));
    let hosts = PluginLifecycleHosts::new(
        package,
        capability,
        runtime.clone(),
        runtime,
        Arc::new(UnavailableOkfHost),
        flow,
        static_surfaces.clone(),
        static_surfaces,
    );
    PluginLifecycleCoordinator::new(
        PluginLifecycleJournalStore::from_extension_paths(paths),
        hosts,
    )
}

fn validate_available_hosts(
    manifest: &ExtensionManifest,
    flow_compiler_binary: &std::path::Path,
) -> UseResult<()> {
    if !manifest.flows.is_empty() && flow_compiler_binary.as_os_str().is_empty() {
        return Err(provider_error(
            "use.plugin.flow_provider_required",
            format!(
                "Cognitive package '{}' requires an a3s-flow Native TypeScript compiler.",
                manifest.package_id
            ),
        )
        .with_suggestion(format!(
            "Set {FLOW_COMPILER_ENV} to the reviewed a3s-flow native compiler binary."
        )));
    }
    if !manifest.okf.is_empty() {
        return Err(provider_error(
            "use.plugin.okf_provider_required",
            format!(
                "Cognitive package '{}' requires an injected A3S Knowledge provider for OKF surfaces.",
                manifest.package_id
            ),
        )
        .with_detail(
            "surfaces",
            serde_json::json!(manifest.okf.iter().map(|value| &value.id).collect::<Vec<_>>()),
        )
        .with_suggestion(
            "Install after Code is configured with a production A3S Knowledge lifecycle adapter.",
        ));
    }

    let runtime_tools = manifest
        .tools
        .iter()
        .filter(|surface| {
            !matches!(
                &surface.workload,
                ToolWorkload::Task(task)
                    if matches!(&task.source, ToolTaskSource::Executable { .. })
            )
        })
        .map(|surface| surface.id.as_str())
        .collect::<Vec<_>>();
    let runtime_mcp = manifest
        .mcp_servers
        .iter()
        .filter(|surface| matches!(surface.launch, PluginMcpLaunch::StreamableHttp { .. }))
        .map(|surface| surface.id.as_str())
        .collect::<Vec<_>>();
    if !runtime_tools.is_empty() || !runtime_mcp.is_empty() {
        return Err(provider_error(
            "use.plugin.runtime_provider_required",
            format!(
                "Cognitive package '{}' requires production Runtime and Gateway provider evidence.",
                manifest.package_id
            ),
        )
        .with_detail("toolSurfaces", serde_json::json!(runtime_tools))
        .with_detail("mcpSurfaces", serde_json::json!(runtime_mcp))
        .with_suggestion(
            "Configure Code with exact Runtime provider selections and service readiness adapters.",
        ));
    }
    Ok(())
}

struct UnavailableRuntimeServiceHost;

#[async_trait]
impl PluginRuntimeServiceReadinessHost for UnavailableRuntimeServiceHost {
    async fn bind_tool_service(
        &self,
        _intent: &PluginLifecycleIntent,
        _surface: &ToolSurface,
        _plan: &RuntimeSurfacePlan,
        _observation: &RuntimeObservation,
        _idempotency_key: &str,
    ) -> UseResult<RuntimeEndpointRef> {
        Err(runtime_provider_error())
    }

    async fn bind_mcp_service(
        &self,
        _intent: &PluginLifecycleIntent,
        _surface: &PluginMcpSurface,
        _plan: &RuntimeSurfacePlan,
        _observation: &RuntimeObservation,
        _idempotency_key: &str,
    ) -> UseResult<PluginMcpServiceReadiness> {
        Err(runtime_provider_error())
    }
}

struct UnavailableOkfHost;

#[async_trait]
impl PluginOkfLifecycleHost for UnavailableOkfHost {
    async fn prepare_okf(
        &self,
        _intent: &PluginLifecycleIntent,
        _surface: &PluginOkfSurface,
        _idempotency_key: &str,
    ) -> UseResult<PluginLifecycleEvidence> {
        Err(okf_provider_error())
    }

    async fn stop_okf(
        &self,
        _intent: &PluginLifecycleIntent,
        _surface: &PluginOkfSurface,
        _idempotency_key: &str,
    ) -> UseResult<PluginLifecycleEvidence> {
        Err(okf_provider_error())
    }

    async fn remove_okf(
        &self,
        _intent: &PluginLifecycleIntent,
        _surface: &PluginOkfSurface,
        _idempotency_key: &str,
    ) -> UseResult<PluginLifecycleEvidence> {
        Err(okf_provider_error())
    }
}

fn runtime_provider_error() -> UseError {
    provider_error(
        "use.plugin.runtime_provider_required",
        "No production Runtime Service and Gateway readiness adapter is configured in A3S Code.",
    )
}

fn okf_provider_error() -> UseError {
    provider_error(
        "use.plugin.okf_provider_required",
        "No production A3S Knowledge lifecycle adapter is configured in A3S Code.",
    )
}

fn provider_error(code: &'static str, message: impl Into<String>) -> UseError {
    UseError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_host_accepts_a3s_flow_and_rejects_okf_without_knowledge() {
        let flow = ExtensionManifest::parse_acl(
            r#"
extension "acme/review" {
  schema_version = 3
  version = "1.0.0"
  route = "review"
  requires_use = ">=0.3.0, <0.4.0"
  actions = ["read", "execute"]

  repository {
    url = "https://github.com/acme/review"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }

  flow "review" {
    engine = "a3s-flow"
    runtime = "native-ts"
    source = "flows/review.ts"
    export = "run"
    optional = false
  }
}
"#,
        )
        .unwrap();
        let factory = CodeCognitivePackageLifecycleFactory::default();
        factory
            .validate_manifest(&flow)
            .expect("Code must compose the real A3S Flow lifecycle host");

        let okf = ExtensionManifest::parse_acl(
            r#"
extension "acme/knowledge" {
  schema_version = 3
  version = "1.0.0"
  route = "knowledge"
  requires_use = ">=0.3.0, <0.4.0"
  actions = ["read"]

  repository {
    url = "https://github.com/acme/knowledge"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }

  okf "domain" {
    format_version = "0.2"
    root = "okf/domain"
    content_digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    concept_count = 1
    file_count = 1
    expanded_bytes = 1
    max_files = 256
    max_concepts = 64
    max_expanded_bytes = 67108864
    max_document_bytes = 1048576
    max_links_per_document = 2048
    optional = false
  }
}
"#,
        )
        .unwrap();
        let error = factory.validate_manifest(&okf).unwrap_err();
        assert_eq!(error.code, "use.plugin.okf_provider_required");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn code_host_preflights_flow_and_persists_exact_generation_binding() {
        use a3s_use::plugin_lifecycle::{
            PluginLifecycleAction, PluginLifecycleIntentSpec, PluginLifecycleOperationStatus,
        };
        use a3s_use_core::{PlanQualifiedSurfaceRef, PluginSurfaceKind, PluginSurfaceRef};
        use a3s_use_extension::{ExtensionLifecycleIdentity, ExtensionPaths};
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir_all(source.join("flows")).unwrap();
        std::fs::write(source.join("README.md"), "# Review package\n").unwrap();
        std::fs::write(
            source.join("flows/review.ts"),
            "export async function run(input: unknown): Promise<unknown> { return input; }\n",
        )
        .unwrap();
        std::fs::write(
            source.join("a3s-use-extension.acl"),
            r#"
extension "acme/review" {
  schema_version = 3
  version = "1.0.0"
  route = "review"
  requires_use = ">=0.3.0, <0.4.0"
  actions = ["read", "execute"]

  repository {
    url = "https://github.com/acme/review"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }

  flow "review" {
    engine = "a3s-flow"
    runtime = "native-ts"
    source = "flows/review.ts"
    export = "run"
    optional = false
  }
}
"#,
        )
        .unwrap();

        let compiler = temp.path().join("a3s-flow-native-compiler");
        std::fs::write(
            &compiler,
            r#"#!/bin/sh
if [ "$1" != "compile" ] || [ "$3" != "-o" ]; then exit 2; fi
printf '#!/bin/sh\nexit 0\n' > "$4"
chmod +x "$4"
"#,
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&compiler).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&compiler, permissions).unwrap();

        let candidate = ExtensionLifecyclePackage::prepare_local("acme/review", &source, true)
            .await
            .unwrap();
        let manifest = candidate.manifest().clone();
        let identity = ExtensionLifecycleIdentity::new(
            "acme/review",
            candidate.package_digest(),
            candidate.manifest_digest(),
            7,
        )
        .unwrap();
        let paths = ExtensionPaths::new(temp.path().join("data"), temp.path().join("state"));
        let registry = ExtensionRegistry::new(paths.clone());
        let package_root = registry.lifecycle_package_root(&identity);
        let factory = CodeCognitivePackageLifecycleFactory {
            flow_compiler_binary: compiler,
        };
        let coordinator = factory
            .install_coordinator(registry.clone(), candidate, package_root.clone())
            .unwrap();
        let intent = PluginLifecycleIntent::from_manifest(
            PluginLifecycleIntentSpec {
                operation_id: "install-acme-review".to_string(),
                plan_digest: format!("sha256:{}", "1".repeat(64)),
                scope_id: "user/current".to_string(),
                package_id: "acme/review".to_string(),
                package_digest: identity.package_digest().to_string(),
                manifest_digest: identity.manifest_digest().to_string(),
                generation: identity.generation(),
                action: PluginLifecycleAction::Install,
            },
            &manifest,
        )
        .unwrap();

        let record = coordinator.apply(&intent, &manifest, || 42).await.unwrap();
        assert_eq!(record.status, PluginLifecycleOperationStatus::Completed);
        let installed = registry.get("acme/review").await.unwrap().unwrap();
        assert!(installed.receipt.enabled);
        assert_eq!(installed.receipt.lifecycle_generation, Some(7));

        let surface = PlanQualifiedSurfaceRef {
            package_id: "acme/review".to_string(),
            surface: PluginSurfaceRef {
                kind: PluginSurfaceKind::Flow,
                id: "review".to_string(),
            },
        };
        let store = FlowRuntimeBindingStore::from_extension_paths(&paths);
        let binding = store
            .get("user/current", &surface, 7)
            .await
            .unwrap()
            .expect("exact-generation A3S Flow binding");
        assert_eq!(binding.generation(), 7);
        assert!(binding.artifact().is_file());
        binding
            .inspect(&manifest.flows[0], &package_root)
            .await
            .unwrap();
    }
}
