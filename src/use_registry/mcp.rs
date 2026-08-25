//! Exact-package MCP projection into the Code capability runtime.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use a3s_code_core::mcp::{McpProjectionAdapter, McpServerConfig, McpTransportConfig};
use a3s_use_extension::{PluginMcpLaunch, PluginMcpSurface, SurfaceActivation};
use anyhow::{bail, Context};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
use super::ProjectedMcpRuntime;
use super::{
    CapabilityBinding, DesiredManagedMcp, ProjectedMcpActivation, ProjectedMcpLaunch,
    ProjectedMcpServer, MCP_REQUEST_TIMEOUT_SECS,
};

/// Trusted host resolution of an opaque Runtime/Gateway endpoint reference.
///
/// The capability snapshot never supplies an HTTP authority or credentials.
/// A process-owned Runtime host must prove that it still owns the exact
/// provider and turn the opaque reference into a credential-free loopback URL.
#[async_trait]
pub(crate) trait McpRuntimeResolver: Send + Sync {
    async fn resolve_streamable_http(
        &self,
        provider_id: &str,
        endpoint_ref: &str,
        endpoint_path: &str,
        cancellation: CancellationToken,
    ) -> anyhow::Result<String>;
}

pub(super) async fn desired_managed_mcp(
    binding: &CapabilityBinding,
    projection: &ProjectedMcpServer,
) -> anyhow::Result<DesiredManagedMcp> {
    let surface = projected_surface(projection);
    let evidence = a3s_use_extension::inspect_mcp_surface_files(&surface, &binding.package_root)
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to reinspect A3S Use MCP '{}:{}': {}: {}",
                binding.id,
                projection.id,
                error.code,
                error.message
            )
        })?;
    if evidence.digest() != projection.file_evidence_digest {
        bail!(
            "A3S Use MCP '{}:{}' file evidence changed after the capability snapshot",
            binding.id,
            projection.id
        );
    }
    let resolved_executable = match &projection.launch {
        ProjectedMcpLaunch::Stdio { executable, .. } => {
            Some(canonical_package_file(&binding.package_root, executable).await?)
        }
        ProjectedMcpLaunch::StreamableHttp { .. } => None,
    };
    let lifecycle_identity = projection.lifecycle_identity.validated("MCP")?;
    let fingerprint = serde_json::to_string(&(
        &binding.id,
        &binding.version,
        binding.origin,
        &binding.package_root,
        projection,
    ))
    .context("failed to fingerprint exact A3S Use MCP evidence")?;
    Ok(DesiredManagedMcp {
        capability_id: binding.id.clone(),
        resolved_executable,
        projection: projection.clone(),
        lifecycle_identity,
        fingerprint,
    })
}

pub(super) async fn projection_adapter(
    desired: &DesiredManagedMcp,
    runtime: Option<&Arc<dyn McpRuntimeResolver>>,
    cancellation: CancellationToken,
) -> anyhow::Result<McpProjectionAdapter> {
    if cancellation.is_cancelled() {
        bail!("A3S Use MCP projection was cancelled before Runtime resolution");
    }
    let transport = match &desired.projection.launch {
        ProjectedMcpLaunch::Stdio { args, .. } => {
            let executable = desired.resolved_executable.as_ref().with_context(|| {
                format!(
                    "A3S Use MCP '{}:{}' omitted its verified executable",
                    desired.capability_id, desired.projection.id
                )
            })?;
            let command = executable
                .to_str()
                .context("managed MCP executable path is not valid UTF-8")?
                .to_string();
            McpTransportConfig::Stdio {
                command,
                args: args.clone(),
            }
        }
        ProjectedMcpLaunch::StreamableHttp {
            runtime: evidence, ..
        } => {
            let runtime = runtime.context(
                "A3S Use Streamable HTTP MCP is projected without a trusted Runtime/Gateway resolver",
            )?;
            let url = runtime
                .resolve_streamable_http(
                    &evidence.provider_id,
                    &evidence.endpoint_ref,
                    &evidence.endpoint_path,
                    cancellation.clone(),
                )
                .await?;
            if cancellation.is_cancelled() {
                bail!("A3S Use MCP projection was cancelled during Runtime resolution");
            }
            validate_resolved_endpoint(&url)?;
            McpTransportConfig::StreamableHttp {
                url,
                headers: HashMap::new(),
            }
        }
    };
    Ok(McpProjectionAdapter::new(McpServerConfig {
        name: desired.projection.server_name.clone(),
        transport,
        enabled: true,
        env: HashMap::new(),
        oauth: None,
        tool_timeout_secs: MCP_REQUEST_TIMEOUT_SECS,
    }))
}

fn projected_surface(projection: &ProjectedMcpServer) -> PluginMcpSurface {
    let activation = match projection.activation {
        ProjectedMcpActivation::Eager => SurfaceActivation::Eager,
        ProjectedMcpActivation::Lazy => SurfaceActivation::Lazy,
    };
    let launch = match &projection.launch {
        ProjectedMcpLaunch::Stdio { executable, args } => PluginMcpLaunch::Stdio {
            executable: executable.clone(),
            args: args.clone(),
        },
        ProjectedMcpLaunch::StreamableHttp { release, .. } => PluginMcpLaunch::StreamableHttp {
            release: release.clone(),
        },
    };
    PluginMcpSurface {
        id: projection.id.clone(),
        activation,
        optional: false,
        launch,
    }
}

async fn canonical_package_file(
    package_root: &Path,
    relative: &Path,
) -> anyhow::Result<std::path::PathBuf> {
    let root = tokio::fs::canonicalize(package_root)
        .await
        .with_context(|| {
            format!(
                "failed to resolve managed MCP package root {}",
                package_root.display()
            )
        })?;
    let candidate = package_root.join(relative);
    let metadata = tokio::fs::symlink_metadata(&candidate)
        .await
        .with_context(|| format!("failed to inspect managed MCP file {}", candidate.display()))?;
    if a3s_use_core::metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        bail!(
            "managed MCP file '{}' is not a regular package file",
            candidate.display()
        );
    }
    let canonical = tokio::fs::canonicalize(&candidate)
        .await
        .with_context(|| format!("failed to resolve managed MCP file {}", candidate.display()))?;
    if !canonical.starts_with(root) {
        bail!(
            "managed MCP file '{}' escapes its exact package root",
            canonical.display()
        );
    }
    Ok(canonical)
}

fn validate_resolved_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let url = url::Url::parse(endpoint)
        .context("trusted MCP Runtime resolver returned an invalid URL")?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(_)) | None => false,
    };
    if url.scheme() != "http"
        || !loopback
        || url.port().is_none_or(|port| port == 0)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("trusted MCP Runtime resolver must return credential-free loopback HTTP");
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn runtime_evidence(endpoint_ref: &str, endpoint_path: &str) -> ProjectedMcpRuntime {
    ProjectedMcpRuntime {
        scope: a3s_use_core::PlanScope {
            kind: a3s_use_core::PlanScopeKind::User,
            id: a3s_use::cognitive_package::COGNITIVE_PACKAGE_DEFAULT_SCOPE.to_string(),
        },
        endpoint_ref: endpoint_ref.to_string(),
        endpoint_path: endpoint_path.to_string(),
        protocol_version: "2025-06-18".to_string(),
        initialized_at_ms: 10,
        provider_id: "test-runtime".to_string(),
        provider_build_id: "build-1".to_string(),
        runtime_generation: 7,
        descriptor_digest: format!("sha256:{}", "b".repeat(64)),
        binding_digest: format!("sha256:{}", "c".repeat(64)),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    use tokio::sync::Notify;

    use super::super::{
        CapabilityOrigin, CapabilityReadiness, ProjectedLifecycleIdentity,
        ProjectedPluginPlannerEvidence,
    };
    use super::*;

    fn lifecycle_identity() -> ProjectedLifecycleIdentity {
        ProjectedLifecycleIdentity {
            package_id: "acme/research".to_string(),
            package_digest: format!("sha256:{}", "a".repeat(64)),
            manifest_digest: format!("sha256:{}", "d".repeat(64)),
            generation: 7,
        }
    }

    fn binding(package_root: PathBuf, projection: ProjectedMcpServer) -> CapabilityBinding {
        CapabilityBinding {
            id: "use/acme/research".to_string(),
            route: "research".to_string(),
            version: "1.0.0".to_string(),
            origin: CapabilityOrigin::Extension,
            enabled: true,
            readiness: CapabilityReadiness::Ready,
            package_root,
            lifecycle_generation: Some(7),
            planner_evidence: Some(ProjectedPluginPlannerEvidence {
                package_id: "acme/research".to_string(),
                package_sha256: "a".repeat(64),
                manifest_sha256: "d".repeat(64),
            }),
            surfaces: vec!["mcp".to_string()],
            mcp: None,
            mcp_servers: vec![projection],
            skills: Vec::new(),
            flows: Vec::new(),
            knowledge: Vec::new(),
            activity_bar: Vec::new(),
            tool_tasks: Vec::new(),
        }
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    #[tokio::test]
    async fn exact_stdio_file_evidence_is_rechecked_before_adapter_staging() {
        let temporary = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(temporary.path().join("bin"))
            .await
            .unwrap();
        let executable = temporary.path().join("bin/catalog");
        tokio::fs::write(&executable, b"catalog-v1").await.unwrap();
        make_executable(&executable);
        let mut projection = ProjectedMcpServer {
            id: "catalog".to_string(),
            server_name: "use_mcp_research_catalog_0123456789abcdef".to_string(),
            activation: ProjectedMcpActivation::Lazy,
            lifecycle_identity: lifecycle_identity(),
            file_evidence_digest: String::new(),
            launch: ProjectedMcpLaunch::Stdio {
                executable: PathBuf::from("bin/catalog"),
                args: vec!["--catalog".to_string()],
            },
        };
        projection.file_evidence_digest = a3s_use_extension::inspect_mcp_surface_files(
            &projected_surface(&projection),
            temporary.path(),
        )
        .await
        .unwrap()
        .digest()
        .to_string();
        let binding = binding(temporary.path().to_path_buf(), projection.clone());

        let desired = desired_managed_mcp(&binding, &projection).await.unwrap();
        let canonical = std::fs::canonicalize(&executable).unwrap();
        assert_eq!(
            desired.resolved_executable.as_deref(),
            Some(canonical.as_path())
        );

        let resolver = Arc::new(RecordingResolver {
            endpoint: "http://127.0.0.1:43129/mcp".to_string(),
            calls: Mutex::new(Vec::new()),
        });
        let resolver_trait = Arc::clone(&resolver) as Arc<dyn McpRuntimeResolver>;
        projection_adapter(&desired, Some(&resolver_trait), CancellationToken::new())
            .await
            .expect("stdio projection must not consult the HTTP Runtime resolver");
        assert!(resolver.calls.lock().unwrap().is_empty());

        tokio::fs::write(&executable, b"catalog-v2").await.unwrap();
        make_executable(&executable);
        let error = desired_managed_mcp(&binding, &projection)
            .await
            .err()
            .expect("mutated executable must not retain old projection evidence");
        assert!(error.to_string().contains("file evidence changed"));
    }

    struct RecordingResolver {
        endpoint: String,
        calls: Mutex<Vec<(String, String, String)>>,
    }

    #[async_trait]
    impl McpRuntimeResolver for RecordingResolver {
        async fn resolve_streamable_http(
            &self,
            provider_id: &str,
            endpoint_ref: &str,
            endpoint_path: &str,
            cancellation: CancellationToken,
        ) -> anyhow::Result<String> {
            if cancellation.is_cancelled() {
                bail!("fixture Runtime resolution was cancelled");
            }
            self.calls.lock().unwrap().push((
                provider_id.to_string(),
                endpoint_ref.to_string(),
                endpoint_path.to_string(),
            ));
            Ok(self.endpoint.clone())
        }
    }

    fn desired_http() -> DesiredManagedMcp {
        let projection = ProjectedMcpServer {
            id: "library".to_string(),
            server_name: "use_mcp_research_library_0123456789abcdef".to_string(),
            activation: ProjectedMcpActivation::Eager,
            lifecycle_identity: lifecycle_identity(),
            file_evidence_digest: format!("sha256:{}", "e".repeat(64)),
            launch: ProjectedMcpLaunch::StreamableHttp {
                release: PathBuf::from("releases/mcp.json"),
                runtime: runtime_evidence("gateway:managed-services/research-library-7", "/mcp"),
            },
        };
        DesiredManagedMcp {
            capability_id: "use/acme/research".to_string(),
            resolved_executable: None,
            lifecycle_identity: projection.lifecycle_identity.validated("MCP").unwrap(),
            fingerprint: "exact-http".to_string(),
            projection,
        }
    }

    #[tokio::test]
    async fn http_adapter_uses_only_the_trusted_loopback_resolver() {
        let resolver = Arc::new(RecordingResolver {
            endpoint: "http://127.0.0.1:43129/_a3s/runtime/research-library-7/mcp".to_string(),
            calls: Mutex::new(Vec::new()),
        });
        let resolver_trait = Arc::clone(&resolver) as Arc<dyn McpRuntimeResolver>;

        let adapter = projection_adapter(
            &desired_http(),
            Some(&resolver_trait),
            CancellationToken::new(),
        )
        .await
        .unwrap();

        assert!(format!("{adapter:?}").contains("use_mcp_research_library"));
        assert_eq!(
            resolver.calls.lock().unwrap().as_slice(),
            [(
                "test-runtime".to_string(),
                "gateway:managed-services/research-library-7".to_string(),
                "/mcp".to_string(),
            )]
        );
    }

    #[tokio::test]
    async fn http_adapter_rejects_non_loopback_resolver_output() {
        let resolver: Arc<dyn McpRuntimeResolver> = Arc::new(RecordingResolver {
            endpoint: "https://example.com/mcp".to_string(),
            calls: Mutex::new(Vec::new()),
        });
        let error = projection_adapter(&desired_http(), Some(&resolver), CancellationToken::new())
            .await
            .expect_err("non-loopback Runtime endpoint must be rejected");
        assert!(error.to_string().contains("credential-free loopback HTTP"));
    }

    #[tokio::test]
    async fn http_adapter_fails_closed_without_a_trusted_runtime_resolver() {
        let error = projection_adapter(&desired_http(), None, CancellationToken::new())
            .await
            .expect_err("HTTP projection without host Runtime authority must fail closed");
        assert!(
            error
                .to_string()
                .contains("trusted Runtime/Gateway resolver"),
            "{error:#}"
        );
    }

    struct CancellationResolver {
        entered: Arc<Notify>,
        cancelled: Arc<AtomicBool>,
    }

    #[async_trait]
    impl McpRuntimeResolver for CancellationResolver {
        async fn resolve_streamable_http(
            &self,
            _provider_id: &str,
            _endpoint_ref: &str,
            _endpoint_path: &str,
            cancellation: CancellationToken,
        ) -> anyhow::Result<String> {
            self.entered.notify_one();
            cancellation.cancelled().await;
            self.cancelled.store(true, Ordering::SeqCst);
            bail!("fixture Runtime resolution was cancelled")
        }
    }

    #[tokio::test]
    async fn http_runtime_resolution_is_cancellation_bounded() {
        let entered = Arc::new(Notify::new());
        let cancelled = Arc::new(AtomicBool::new(false));
        let resolver: Arc<dyn McpRuntimeResolver> = Arc::new(CancellationResolver {
            entered: Arc::clone(&entered),
            cancelled: Arc::clone(&cancelled),
        });
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let task = tokio::spawn(async move {
            projection_adapter(&desired_http(), Some(&resolver), task_cancellation).await
        });
        entered.notified().await;
        cancellation.cancel();
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("cancelled Runtime resolution must settle")
            .expect("resolver task must join")
            .expect_err("cancelled Runtime resolution must fail closed");
        assert!(error.to_string().contains("cancelled"), "{error:#}");
        assert!(cancelled.load(Ordering::SeqCst));
    }

    #[test]
    fn mcp_projection_owners_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<DesiredManagedMcp>();
        assert_send_sync::<Arc<dyn McpRuntimeResolver>>();
    }
}
