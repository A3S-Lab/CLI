//! A3S Code host composition for one cognitive-package lifecycle generation.

use std::path::PathBuf;
use std::sync::Arc;

use a3s_runtime::contract::RuntimeObservation;
use a3s_use::cognitive_package::{
    CognitivePackageAuthorizationProvider, CognitivePackageLifecycleFactory,
    CognitivePackageManager, StandaloneCognitivePackageAuthorizationProvider,
};
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
use a3s_use_core::{PlanScope, UseError, UseResult};
use a3s_use_extension::{
    ExtensionLifecyclePackage, ExtensionManifest, ExtensionPaths, ExtensionRegistry,
    PluginMcpLaunch, PluginMcpSurface, PluginOkfSurface, ToolSurface, ToolTaskSource, ToolWorkload,
};
use async_trait::async_trait;

use super::ComponentPaths;

const FLOW_COMPILER_ENV: &str = "A3S_FLOW_NATIVE_TS_COMPILER";

/// Code's production package host.
///
/// Executable native Tool Tasks and stdio MCP use the package-bound launcher
/// implemented by the shared lifecycle host. Skill/UI use immutable static
/// validation, and Flow uses the real `A3sFlowLifecycleHost`. OCI/Service
/// workloads, HTTP MCP, and OKF stay fail-closed until Code receives concrete
/// Runtime/Gateway/Knowledge adapters.
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

/// Compose the exact Code lifecycle hosts for package observation and
/// permission-free enablement.
///
/// Local CLI/Web and managed-host adapters share this constructor so they use
/// one lifecycle and authorization composition for current packages.
/// Enablement rejects permission-bearing packages before authorization, while
/// reviewed graph mutations continue to use `apply_reviewed_cognitive_package`.
pub(crate) fn code_cognitive_package_manager(
    paths: &ComponentPaths,
    scope: PlanScope,
) -> UseResult<CognitivePackageManager> {
    code_cognitive_package_manager_with_authorization(
        paths,
        scope,
        Arc::new(StandaloneCognitivePackageAuthorizationProvider),
    )
}

/// Compose Code's production lifecycle hosts with a caller-owned trusted
/// authorization boundary. Planning and reviewed apply use this constructor
/// so they cannot drift from the lifecycle used by standalone enablement.
pub(crate) fn code_cognitive_package_manager_with_authorization(
    paths: &ComponentPaths,
    scope: PlanScope,
    authorization: Arc<dyn CognitivePackageAuthorizationProvider>,
) -> UseResult<CognitivePackageManager> {
    CognitivePackageManager::with_plan_scope_lifecycle_and_authorization(
        ExtensionRegistry::new(ExtensionPaths::new(
            paths.data_root.join("use"),
            paths.state_root.join("use"),
        )),
        scope,
        Arc::new(CodeCognitivePackageLifecycleFactory::default()),
        authorization,
    )
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
        use a3s_use_core::{
            CatalogArchive, CatalogAvailability, CatalogPackage, CatalogSurface,
            PlanQualifiedSurfaceRef, PluginCatalogRecord, PluginPermissionCeiling,
            PluginReleaseChannel, PluginSurfaceKind, PluginSurfaceRef, PLUGIN_CATALOG_SCHEMA_V3,
            PLUGIN_PERMISSION_SCHEMA,
        };
        use a3s_use_extension::{ExtensionPaths, TrustedRegistry};
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;

        use crate::tuf_test_support::{
            host_target, package_directory_archive, TestRepository, TestServer, TestTarget, FUTURE,
        };

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

        let archive = package_directory_archive(&source);
        let (package_sha256, file_count, expanded_bytes) = package_fingerprint(&source);
        let permissions = PluginPermissionCeiling {
            schema: PLUGIN_PERMISSION_SCHEMA.to_string(),
            surfaces: Vec::new(),
        };
        let target = host_target();
        let target_name =
            format!("extensions/acme/review/1.0.0/stable/{target}/review-1.0.0-{target}.tar.gz");
        let manifest_bytes = std::fs::read(source.join("a3s-use-extension.acl")).unwrap();
        let catalog = PluginCatalogRecord {
            schema: PLUGIN_CATALOG_SCHEMA_V3.to_string(),
            package_id: "acme/review".to_string(),
            display_name: "Review flow".to_string(),
            description: "A3S Flow lifecycle integration fixture.".to_string(),
            publisher: "acme".to_string(),
            keywords: vec!["flow".to_string()],
            categories: vec!["test".to_string()],
            version: "1.0.0".to_string(),
            channel: PluginReleaseChannel::Stable,
            requires_use: ">=0.3.0, <0.4.0".to_string(),
            dependencies: Vec::new(),
            target: target.to_string(),
            surfaces: vec![CatalogSurface {
                kind: PluginSurfaceKind::Flow,
                id: "review".to_string(),
                optional: false,
                workload: None,
                mcp_transport: None,
                mcp_tool_count: None,
                okf_bundle: None,
                requires: Vec::new(),
            }],
            permission_ceiling_digest: permissions.descriptor_digest().unwrap(),
            permission_ceiling: permissions,
            planning: None,
            archive: CatalogArchive {
                target_name: target_name.clone(),
                length: archive.len() as u64,
                sha256: format!("sha256:{:x}", Sha256::digest(&archive)),
            },
            package: CatalogPackage {
                expanded_bytes,
                file_count,
                sha256: Some(format!("sha256:{package_sha256}")),
                manifest_sha256: Some(format!("sha256:{:x}", Sha256::digest(&manifest_bytes))),
            },
            license: "MIT".to_string(),
            repository: "https://github.com/acme/review".to_string(),
            availability: CatalogAvailability::Available,
        };
        catalog.validate().unwrap();
        let repository = TestRepository::with_targets(
            vec![TestTarget {
                archive,
                target_name,
                custom: Some(serde_json::to_value(catalog).unwrap()),
            }],
            7,
            FUTURE,
        );
        let server = TestServer::start(repository.routes.clone());
        let paths = ExtensionPaths::new(temp.path().join("data"), temp.path().join("state"));
        let registry = ExtensionRegistry::new(paths.clone());
        let factory = CodeCognitivePackageLifecycleFactory {
            flow_compiler_binary: compiler,
        };
        let manager = CognitivePackageManager::with_scope_and_lifecycle(
            registry.clone(),
            "current",
            Arc::new(factory),
        )
        .unwrap();
        let trusted = TrustedRegistry::new(
            "fixture",
            server.base_url(),
            &repository.root_sha256,
            None,
            paths.state_root().join("remote-registries/fixture"),
        )
        .unwrap();
        let result = manager
            .install_remote(
                &trusted,
                &[],
                "acme/review",
                Some("1.0.0"),
                PluginReleaseChannel::Stable,
                None,
            )
            .await
            .unwrap();
        let installed = registry.get("acme/review").await.unwrap().unwrap();
        assert!(installed.receipt.enabled);
        assert_eq!(result.root, installed);
        let generation = installed.receipt.lifecycle_generation.unwrap();
        let package_root = installed.receipt.package_root.clone();

        let surface = PlanQualifiedSurfaceRef {
            package_id: "acme/review".to_string(),
            surface: PluginSurfaceRef {
                kind: PluginSurfaceKind::Flow,
                id: "review".to_string(),
            },
        };
        let store = FlowRuntimeBindingStore::from_extension_paths(&paths);
        let binding = store
            .get(manager.scope(), &surface, generation)
            .await
            .unwrap()
            .expect("exact-generation A3S Flow binding");
        assert_eq!(binding.generation(), generation);
        assert!(binding.artifact().is_file());
        binding
            .inspect(&installed.manifest.flows[0], &package_root)
            .await
            .unwrap();

        fn package_fingerprint(root: &std::path::Path) -> (String, u64, u64) {
            fn collect(
                root: &std::path::Path,
                directory: &std::path::Path,
                files: &mut Vec<(String, PathBuf)>,
            ) {
                for entry in std::fs::read_dir(directory).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        collect(root, &path, files);
                    } else {
                        files.push((
                            path.strip_prefix(root)
                                .unwrap()
                                .to_string_lossy()
                                .replace('\\', "/"),
                            path,
                        ));
                    }
                }
            }

            let mut files = Vec::new();
            collect(root, root, &mut files);
            files.sort_by(|left, right| left.0.cmp(&right.0));
            let mut digest = Sha256::new();
            digest.update(b"a3s-use-expanded-package-v1\0");
            let mut expanded_bytes = 0_u64;
            for (relative, path) in &files {
                let body = std::fs::read(path).unwrap();
                expanded_bytes += body.len() as u64;
                digest.update((relative.len() as u64).to_be_bytes());
                digest.update(relative.as_bytes());
                digest.update((body.len() as u64).to_be_bytes());
                digest.update(body);
            }
            (
                format!("{:x}", digest.finalize()),
                files.len() as u64,
                expanded_bytes,
            )
        }
    }
}
