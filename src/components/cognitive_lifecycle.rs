//! A3S Code host composition for one cognitive-package lifecycle generation.

use std::path::PathBuf;
use std::sync::Arc;

use a3s_runtime::contract::{RuntimeObservation, RuntimeServiceEndpoint};
use a3s_runtime::RuntimeClientRegistry;
use a3s_use::cognitive_package::{
    CognitivePackageAuthorizationProvider, CognitivePackageLifecycleFactory,
    CognitivePackageManager, ManagedCognitivePackageLifecycleFactory,
    StandaloneCognitivePackageAuthorizationProvider,
};
use a3s_use::plugin_lifecycle::{
    PluginLifecycleCoordinator, PluginLifecycleIntent, PluginMcpServiceReadiness,
    PluginRuntimeServiceReadinessHost,
};
use a3s_use::plugin_runtime::{
    RuntimeEndpointRef, RuntimeProviderSelection, RuntimeServiceBindingReceipt, RuntimeSurfacePlan,
};
use a3s_use_core::{PlanScope, UseError, UseResult};
use a3s_use_extension::{
    ExtensionLifecyclePackage, ExtensionManifest, ExtensionPaths, ExtensionRegistry,
    PluginMcpSurface, ToolSurface,
};
use async_trait::async_trait;

use super::{CodePluginUiLifecycleHostFactory, ComponentPaths};

const FLOW_COMPILER_ENV: &str = "A3S_FLOW_NATIVE_TS_COMPILER";

/// Code's package host over the shared managed A3S Use lifecycle composition.
///
/// The default host deliberately carries no Runtime selection and an
/// unavailable Gateway port. Native Tool Tasks, stdio MCP, Skill/UI, OKF, and
/// an explicitly resolved A3S Flow compiler remain available; release-backed
/// Runtime workloads fail closed until Code injects exact providers.
#[derive(Clone)]
pub(crate) struct CodeCognitivePackageLifecycleFactory {
    inner: ManagedCognitivePackageLifecycleFactory,
}

impl std::fmt::Debug for CodeCognitivePackageLifecycleFactory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodeCognitivePackageLifecycleFactory")
            .field("inner", &self.inner)
            .finish()
    }
}

impl CodeCognitivePackageLifecycleFactory {
    pub(super) fn from_env(paths: &ComponentPaths) -> UseResult<Self> {
        Self::managed(
            RuntimeProviderSelection::default(),
            Arc::new(RuntimeClientRegistry::new()),
            Arc::new(UnavailableRuntimeServiceHost),
            paths,
        )
    }

    pub(crate) fn managed(
        selection: RuntimeProviderSelection,
        runtime_registry: Arc<RuntimeClientRegistry>,
        readiness: Arc<dyn PluginRuntimeServiceReadinessHost>,
        paths: &ComponentPaths,
    ) -> UseResult<Self> {
        let mut inner =
            ManagedCognitivePackageLifecycleFactory::new(selection, runtime_registry, readiness)
                .with_ui_lifecycle_factory(Arc::new(
                    CodePluginUiLifecycleHostFactory::from_component_paths(paths),
                ));
        if let Some(compiler) = configured_flow_compiler() {
            inner = inner.with_flow_compiler(compiler)?;
        }
        Ok(Self { inner })
    }

    #[cfg(test)]
    fn with_flow_compiler(compiler: impl Into<PathBuf>) -> UseResult<Self> {
        Ok(Self {
            inner: ManagedCognitivePackageLifecycleFactory::new(
                RuntimeProviderSelection::default(),
                Arc::new(RuntimeClientRegistry::new()),
                Arc::new(UnavailableRuntimeServiceHost),
            )
            .with_flow_compiler(compiler)?,
        })
    }
}

fn configured_flow_compiler() -> Option<PathBuf> {
    std::env::var_os(FLOW_COMPILER_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| find_on_path("a3s-flow-native-compiler"))
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let executable = format!("{binary}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&path)
        .map(|directory| directory.join(&executable))
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| candidate.canonicalize().ok())
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
        Arc::new(CodeCognitivePackageLifecycleFactory::from_env(paths)?),
        authorization,
    )
}

impl CognitivePackageLifecycleFactory for CodeCognitivePackageLifecycleFactory {
    fn name(&self) -> &'static str {
        "a3s-code"
    }

    fn validate_manifest(&self, manifest: &ExtensionManifest) -> UseResult<()> {
        self.inner.validate_manifest(manifest)
    }

    fn validate_manifest_for_planning(&self, manifest: &ExtensionManifest) -> UseResult<()> {
        self.inner.validate_manifest_for_planning(manifest)
    }

    fn validate_manifest_for_retirement(&self, manifest: &ExtensionManifest) -> UseResult<()> {
        self.inner.validate_manifest_for_retirement(manifest)
    }

    fn install_coordinator(
        &self,
        registry: ExtensionRegistry,
        candidate: ExtensionLifecyclePackage,
        package_root: PathBuf,
    ) -> UseResult<PluginLifecycleCoordinator> {
        self.inner
            .install_coordinator(registry, candidate, package_root)
    }

    fn published_install_coordinator(
        &self,
        registry: ExtensionRegistry,
        package_root: PathBuf,
    ) -> UseResult<PluginLifecycleCoordinator> {
        self.inner
            .published_install_coordinator(registry, package_root)
    }

    fn uninstall_coordinator(
        &self,
        registry: ExtensionRegistry,
        package_root: PathBuf,
    ) -> UseResult<PluginLifecycleCoordinator> {
        self.inner.uninstall_coordinator(registry, package_root)
    }
}

pub(crate) struct UnavailableRuntimeServiceHost;

#[async_trait]
impl PluginRuntimeServiceReadinessHost for UnavailableRuntimeServiceHost {
    async fn bind_tool_service(
        &self,
        _intent: &PluginLifecycleIntent,
        _surface: &ToolSurface,
        _plan: &RuntimeSurfacePlan,
        _observation: &RuntimeObservation,
        _runtime_endpoint: &RuntimeServiceEndpoint,
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
        _runtime_endpoint: &RuntimeServiceEndpoint,
        _idempotency_key: &str,
    ) -> UseResult<PluginMcpServiceReadiness> {
        Err(runtime_provider_error())
    }

    async fn drain_service(
        &self,
        _intent: &PluginLifecycleIntent,
        _receipt: &RuntimeServiceBindingReceipt,
        _idempotency_key: &str,
    ) -> UseResult<()> {
        Err(runtime_provider_error())
    }

    async fn remove_service(
        &self,
        _intent: &PluginLifecycleIntent,
        _receipt: &RuntimeServiceBindingReceipt,
        _idempotency_key: &str,
    ) -> UseResult<()> {
        Err(runtime_provider_error())
    }
}

fn runtime_provider_error() -> UseError {
    UseError::new(
        "use.plugin.runtime_provider_required",
        "No production Runtime Service and Gateway readiness adapter is configured in A3S Code.",
    )
}

#[cfg(test)]
mod tests {
    use a3s_use::flow_runtime::FlowRuntimeBindingStore;

    use super::*;

    #[test]
    fn code_host_accepts_a3s_flow_and_okf_knowledge() {
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
        let factory = CodeCognitivePackageLifecycleFactory::with_flow_compiler(
            std::env::current_dir()
                .unwrap()
                .join("a3s-flow-native-compiler"),
        )
        .unwrap();
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
        factory
            .validate_manifest(&okf)
            .expect("Code must compose the managed OKF Knowledge lifecycle host");
    }

    #[tokio::test]
    async fn signed_okf_install_upgrade_restart_query_and_uninstall_use_code_host() {
        use a3s_use::okf_knowledge::{
            OkfKnowledgeBindingStore, OkfKnowledgeClient, OkfKnowledgeSearchRequest,
            SqliteOkfKnowledgeAdapter,
        };
        use a3s_use_core::{
            OkfCapabilityProjection, PlanQualifiedSurfaceRef, PlanScope, PlanScopeKind,
            PluginReleaseChannel, PluginSurfaceKind, PluginSurfaceRef,
        };
        use a3s_use_extension::{ExtensionPaths, TrustedRegistry};

        use crate::tuf_test_support::{TestRepository, TestServer, FUTURE};

        let temporary = tempfile::tempdir().unwrap();
        let target = crate::tuf_test_support::host_target();
        let first = signed_okf_target(
            &temporary.path().join("first"),
            "1.0.0",
            "legacyactivationneedle",
            target,
        );
        let replacement = signed_okf_target(
            &temporary.path().join("replacement"),
            "1.1.0",
            "replacementactivationneedle",
            target,
        );
        let repository = TestRepository::with_targets(vec![first, replacement], 31, FUTURE);
        let server = TestServer::start(repository.routes.clone());
        let component_paths = ComponentPaths::for_test(temporary.path());
        let scope = PlanScope {
            kind: PlanScopeKind::User,
            id: "current".to_string(),
        };
        let manager = code_cognitive_package_manager(&component_paths, scope.clone()).unwrap();
        let extension_paths = ExtensionPaths::new(
            component_paths.data_root.join("use"),
            component_paths.state_root.join("use"),
        );
        let trusted = TrustedRegistry::new(
            "fixture",
            server.base_url(),
            &repository.root_sha256,
            None,
            extension_paths
                .state_root()
                .join("remote-registries/fixture"),
        )
        .unwrap();

        let installed = manager
            .install_remote(
                &trusted,
                &[],
                "acme/knowledge",
                Some("1.0.0"),
                PluginReleaseChannel::Stable,
                None,
            )
            .await
            .unwrap();
        assert!(installed.changed);
        let first_generation = installed.root.receipt.lifecycle_generation.unwrap();
        let surface = PlanQualifiedSurfaceRef {
            package_id: "acme/knowledge".to_string(),
            surface: PluginSurfaceRef {
                kind: PluginSurfaceKind::Okf,
                id: "domain-knowledge".to_string(),
            },
        };
        let store = OkfKnowledgeBindingStore::from_extension_paths(&extension_paths);
        let first_binding = store
            .get(&scope, &surface, first_generation)
            .await
            .unwrap()
            .expect("Code must persist the installed OKF generation");
        let first_projection = OkfCapabilityProjection::from_promoted(
            &first_binding.receipt,
            &first_binding.observation,
        )
        .unwrap();

        // A newly constructed client represents a restarted Code/Web process.
        let restarted = OkfKnowledgeClient::new(Arc::new(
            SqliteOkfKnowledgeAdapter::from_extension_paths(&extension_paths),
        ));
        let first_search = restarted
            .search(
                &OkfKnowledgeSearchRequest::new(
                    scope.clone(),
                    "legacyactivationneedle",
                    5,
                    vec![first_projection.clone()],
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first_search.hits[0].citation.generation, first_generation);

        let upgraded = manager
            .upgrade_remote(
                &trusted,
                &[],
                "acme/knowledge",
                Some("1.1.0"),
                PluginReleaseChannel::Stable,
                None,
            )
            .await
            .unwrap();
        assert!(upgraded.changed);
        let next_generation = upgraded.root.receipt.lifecycle_generation.unwrap();
        assert!(next_generation > first_generation);
        let next_binding = store
            .get(&scope, &surface, next_generation)
            .await
            .unwrap()
            .expect("Code must persist the replacement OKF generation");
        let next_projection = OkfCapabilityProjection::from_promoted(
            &next_binding.receipt,
            &next_binding.observation,
        )
        .unwrap();
        let restarted = OkfKnowledgeClient::new(Arc::new(
            SqliteOkfKnowledgeAdapter::from_extension_paths(&extension_paths),
        ));
        let replacement = restarted
            .search(
                &OkfKnowledgeSearchRequest::new(
                    scope.clone(),
                    "replacementactivationneedle",
                    5,
                    vec![next_projection.clone()],
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replacement.hits[0].citation.generation, next_generation);
        assert!(restarted
            .search(
                &OkfKnowledgeSearchRequest::new(
                    scope.clone(),
                    "legacyactivationneedle",
                    5,
                    vec![next_projection.clone()],
                )
                .unwrap(),
            )
            .await
            .unwrap()
            .hits
            .is_empty());
        assert!(
            restarted
                .search(
                    &OkfKnowledgeSearchRequest::new(
                        scope.clone(),
                        "legacyactivationneedle",
                        5,
                        vec![first_projection],
                    )
                    .unwrap(),
                )
                .await
                .is_err(),
            "retired generation must not remain queryable after upgrade"
        );

        let removed = manager.uninstall("acme/knowledge").await.unwrap();
        assert!(removed.changed);
        let restarted = OkfKnowledgeClient::new(Arc::new(
            SqliteOkfKnowledgeAdapter::from_extension_paths(&extension_paths),
        ));
        assert!(
            restarted
                .search(
                    &OkfKnowledgeSearchRequest::new(
                        scope,
                        "replacementactivationneedle",
                        5,
                        vec![next_projection],
                    )
                    .unwrap(),
                )
                .await
                .is_err(),
            "uninstall must invalidate only the receipt-owned projection"
        );
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
        let factory = CodeCognitivePackageLifecycleFactory::with_flow_compiler(compiler).unwrap();
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
    }

    fn signed_okf_target(
        root: &std::path::Path,
        version: &str,
        needle: &str,
        target: &str,
    ) -> crate::tuf_test_support::TestTarget {
        use a3s_use_core::{
            inspect_okf_bundle_files, CatalogArchive, CatalogAvailability, CatalogPackage,
            CatalogSurface, OkfBundleFile, OkfBundleLimits, OkfFormatVersion, PluginCatalogRecord,
            PluginPermissionCeiling, PluginReleaseChannel, PluginSurfaceKind,
            PLUGIN_CATALOG_SCHEMA_V3, PLUGIN_PERMISSION_SCHEMA,
        };
        use sha2::{Digest, Sha256};

        let package = root.join("package");
        let okf_root = package.join("okf/domain-knowledge");
        std::fs::create_dir_all(okf_root.join("concepts")).unwrap();
        std::fs::write(package.join("README.md"), "# Knowledge package\n").unwrap();
        let concept = format!("---\ntype: Decision\n---\n\n# Package activation\n\n{needle}\n");
        std::fs::write(okf_root.join("concepts/package-lifecycle.md"), &concept).unwrap();
        let limits = OkfBundleLimits::default();
        let inspection = inspect_okf_bundle_files(
            OkfFormatVersion::V0_2,
            limits.clone(),
            &[OkfBundleFile::new(
                "concepts/package-lifecycle.md",
                concept.as_bytes(),
            )],
        )
        .unwrap();
        let manifest = format!(
            r#"
extension "acme/knowledge" {{
  schema_version = 3
  version = "{version}"
  route = "knowledge"
  requires_use = ">=0.3.0, <0.4.0"
  actions = ["read"]

  repository {{
    url = "https://github.com/acme/knowledge"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }}

  okf "domain-knowledge" {{
    format_version = "0.2"
    root = "okf/domain-knowledge"
    content_digest = "{}"
    concept_count = {}
    file_count = {}
    expanded_bytes = {}
    max_files = {}
    max_concepts = {}
    max_expanded_bytes = {}
    max_document_bytes = {}
    max_links_per_document = {}
    optional = false
  }}
}}
"#,
            inspection.content_digest,
            inspection.concept_count,
            inspection.file_count,
            inspection.expanded_bytes,
            limits.max_files,
            limits.max_concepts,
            limits.max_expanded_bytes,
            limits.max_document_bytes,
            limits.max_links_per_document,
        );
        let manifest_path = package.join("a3s-use-extension.acl");
        std::fs::write(&manifest_path, &manifest).unwrap();
        let parsed = ExtensionManifest::parse_acl(&manifest).unwrap();
        let archive = crate::tuf_test_support::package_directory_archive(&package);
        let (package_sha256, file_count, expanded_bytes) = package_fingerprint(&package);
        let permissions = PluginPermissionCeiling {
            schema: PLUGIN_PERMISSION_SCHEMA.to_string(),
            surfaces: Vec::new(),
        };
        let target_name = format!(
            "extensions/acme/knowledge/{version}/stable/{target}/acme-knowledge-{version}-{target}.tar.gz"
        );
        let catalog = PluginCatalogRecord {
            schema: PLUGIN_CATALOG_SCHEMA_V3.to_string(),
            package_id: "acme/knowledge".to_string(),
            display_name: "A3S Knowledge Pack".to_string(),
            description: "Cited OKF Knowledge managed by A3S Code.".to_string(),
            publisher: "acme".to_string(),
            keywords: vec!["knowledge".to_string(), "okf".to_string()],
            categories: vec!["knowledge".to_string()],
            version: version.to_string(),
            channel: PluginReleaseChannel::Stable,
            requires_use: ">=0.3.0, <0.4.0".to_string(),
            dependencies: Vec::new(),
            target: target.to_string(),
            surfaces: vec![CatalogSurface {
                kind: PluginSurfaceKind::Okf,
                id: "domain-knowledge".to_string(),
                optional: false,
                workload: None,
                mcp_transport: None,
                mcp_tool_count: None,
                okf_bundle: Some(parsed.okf[0].bundle.clone()),
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
                manifest_sha256: Some(format!("sha256:{:x}", Sha256::digest(manifest.as_bytes()))),
            },
            license: "MIT".to_string(),
            repository: "https://github.com/acme/knowledge".to_string(),
            availability: CatalogAvailability::Available,
        };
        catalog.validate().unwrap();
        crate::tuf_test_support::TestTarget {
            archive,
            target_name,
            custom: Some(serde_json::to_value(catalog).unwrap()),
        }
    }

    fn package_fingerprint(root: &std::path::Path) -> (String, u64, u64) {
        use sha2::{Digest, Sha256};

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
