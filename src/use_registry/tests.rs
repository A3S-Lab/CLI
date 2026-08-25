use super::*;

#[tokio::test]
async fn registry_slot_waits_without_blocking_and_settles_when_setup_finishes() {
    let slot = UseRegistrySlot::preparing();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), slot.wait_until_settled())
            .await
            .is_err()
    );

    let waiting = tokio::spawn({
        let slot = slot.clone();
        async move { slot.wait_until_settled().await }
    });
    slot.set_unavailable("fixture setup failure");

    tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .expect("settled slot should wake waiters")
        .expect("slot waiter should not panic");
    assert!(slot.ready_handle().is_none());
}

#[tokio::test]
async fn registry_slot_reports_setup_state_and_exposes_the_ready_handle() {
    let temp = tempfile::tempdir().unwrap();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temp.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let slot = UseRegistrySlot::preparing();

    let preparing = slot.status_text(Arc::clone(&session), false).await;
    assert!(preparing.contains("discovering or installing in the background"));

    slot.set_unavailable("fixture setup failure");
    let unavailable = slot.status_text(Arc::clone(&session), false).await;
    assert!(unavailable.contains("binary  not discovered"));
    assert!(unavailable.contains("setup   fixture setup failure"));

    let handle =
        UseRegistryHandle::for_test_knowledge(test_extension_paths(temp.path()), 0, Vec::new());
    slot.set_ready(handle, Some("fixture setup warning".to_string()));
    let ready = slot.ready_handle().expect("ready registry handle");
    assert!(ready.knowledge_catalog().projections.is_empty());
    let unavailable = ready
        .search_knowledge("fixture", 5, None)
        .await
        .expect_err("an empty capability revision has no managed Knowledge");
    assert!(unavailable.to_string().contains("no managed OKF Knowledge"));

    session.close().await;
}

#[tokio::test]
async fn dropping_the_last_registry_handle_cancels_owned_background_work() {
    let temp = tempfile::tempdir().unwrap();
    let cancellation = CancellationToken::new();
    let (desired_tx, _) = watch::channel(Arc::new(DesiredCapabilities::default()));
    let extension_paths = test_extension_paths(temp.path());
    let knowledge = UseKnowledgeCarrier::new(desired_tx.clone(), &extension_paths);
    let handle = UseRegistryHandle {
        inner: Arc::new(UseRegistryInner {
            executable: PathBuf::from("unused-a3s-use"),
            directory: PathBuf::from("."),
            plugin_management: None,
            runtime_tasks: None,
            mcp_runtime: None,
            mode: ProjectionMode::FullCompatibility,
            desired_tx,
            knowledge,
            cancellation: cancellation.clone(),
            projections: Mutex::new(BTreeMap::new()),
            registry_task: Mutex::new(None),
        }),
    };
    let clone = handle.clone();

    drop(handle);
    assert!(!cancellation.is_cancelled());
    drop(clone);

    tokio::time::timeout(Duration::from_secs(1), cancellation.cancelled())
        .await
        .expect("dropping the final registry handle must cancel its tasks");
}

#[cfg(any(unix, windows))]
// Keep process-backed fixtures from competing for spawn and stdio scheduling;
// startup budget tests must measure the product path, not test-harness load.
static PROCESS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const FIXTURE_RUNTIME_TOOL: &str = "use_tool_report_convert_0123456789abcdef";

#[derive(Default)]
struct RecordingRuntimeTaskInvoker {
    providers: BTreeSet<String>,
    requests: Mutex<Vec<a3s_use::plugin_runtime::RuntimeTaskDispatchRequest>>,
}

impl RecordingRuntimeTaskInvoker {
    fn with_provider(provider_id: &str) -> Self {
        Self {
            providers: BTreeSet::from([provider_id.to_string()]),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn generations(&self) -> Vec<u64> {
        self.requests
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .map(|request| request.identity().generation())
            .collect()
    }
}

#[async_trait::async_trait]
impl RuntimeTaskInvoker for RecordingRuntimeTaskInvoker {
    fn has_runtime_provider(&self, provider_id: &str) -> bool {
        self.providers.contains(provider_id)
    }

    async fn invoke_runtime_task(
        &self,
        request: a3s_use::plugin_runtime::RuntimeTaskDispatchRequest,
    ) -> anyhow::Result<runtime_tasks::RuntimeTaskOutcome> {
        self.requests
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(request);
        Ok(runtime_tasks::RuntimeTaskOutcome {
            exit_code: 0,
            stdout: r#"{"fixture":"runtime-ok"}"#.to_string(),
            stderr: String::new(),
            truncated: false,
        })
    }
}

#[derive(Default)]
struct RuntimeToolCallingLlm {
    calls: std::sync::atomic::AtomicUsize,
}

impl RuntimeToolCallingLlm {
    fn response(
        &self,
        tools: &[a3s_code_core::llm::ToolDefinition],
    ) -> anyhow::Result<a3s_code_core::LlmResponse> {
        if !tools.iter().any(|tool| tool.name == FIXTURE_RUNTIME_TOOL) {
            anyhow::bail!("projected Runtime Tool definition is missing from the admitted Run");
        }
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (content, stop_reason) = if call.is_multiple_of(2) {
            (
                vec![a3s_code_core::ContentBlock::ToolUse {
                    id: format!("runtime-tool-{call}"),
                    name: FIXTURE_RUNTIME_TOOL.to_string(),
                    input: serde_json::json!({"argv": ["input"]}),
                }],
                "tool_use",
            )
        } else {
            (
                vec![a3s_code_core::ContentBlock::Text {
                    text: "Runtime Tool completed.".to_string(),
                }],
                "end_turn",
            )
        };
        Ok(a3s_code_core::LlmResponse {
            message: a3s_code_core::Message {
                role: "assistant".to_string(),
                content,
                reasoning_content: None,
            },
            usage: a3s_code_core::TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            stop_reason: Some(stop_reason.to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        })
    }
}

#[async_trait::async_trait]
impl a3s_code_core::LlmClient for RuntimeToolCallingLlm {
    async fn complete(
        &self,
        _messages: &[a3s_code_core::Message],
        _system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
    ) -> anyhow::Result<a3s_code_core::LlmResponse> {
        self.response(tools)
    }

    async fn complete_streaming(
        &self,
        _messages: &[a3s_code_core::Message],
        _system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<a3s_code_core::llm::StreamEvent>> {
        let response = self.response(tools)?;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            if let Some(text) = response
                .message
                .content
                .iter()
                .find_map(|block| match block {
                    a3s_code_core::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            {
                let _ = tx
                    .send(a3s_code_core::llm::StreamEvent::TextDelta(text))
                    .await;
            }
            let _ = tx
                .send(a3s_code_core::llm::StreamEvent::Done(response))
                .await;
        });
        Ok(rx)
    }
}

fn fixture_runtime_task_projection(
    generation: u64,
    package_digest: &str,
    manifest_digest: &str,
) -> serde_json::Value {
    serde_json::json!({
        "toolName": FIXTURE_RUNTIME_TOOL,
        "surfaceId": "convert",
        "command": "acme-convert",
        "jsonOutput": true,
        "timeoutMs": 30_000,
        "scope": {"kind": "workspace", "id": "workspace:fixture"},
        "lifecycleIdentity": {
            "packageId": "acme/report",
            "packageDigest": package_digest,
            "manifestDigest": manifest_digest,
            "generation": generation
        },
        "providerId": "test-runtime"
    })
}

fn fixture_runtime_capability(
    generation: u64,
    package_digest: &str,
    manifest_digest: &str,
) -> CapabilityBinding {
    serde_json::from_value(serde_json::json!({
        "id": "use/acme/report",
        "route": "report",
        "version": format!("{generation}.0.0"),
        "origin": "extension",
        "enabled": true,
        "readiness": "ready",
        "packageRoot": std::env::temp_dir(),
        "lifecycleGeneration": generation,
        "plannerEvidence": {
            "packageId": "acme/report",
            "packageSha256": package_digest,
            "manifestSha256": manifest_digest
        },
        "surfaces": ["tool"],
        "toolTasks": [fixture_runtime_task_projection(
            generation,
            package_digest,
            manifest_digest
        )]
    }))
    .unwrap()
}

fn fixture_mcp_projection(
    generation: u64,
    package_digest: &str,
    manifest_digest: &str,
) -> ProjectedMcpServer {
    ProjectedMcpServer {
        id: "library".to_string(),
        server_name: "use_mcp_report_library_0123456789abcdef".to_string(),
        activation: ProjectedMcpActivation::Lazy,
        lifecycle_identity: ProjectedLifecycleIdentity {
            package_id: "acme/report".to_string(),
            package_digest: package_digest.to_string(),
            manifest_digest: manifest_digest.to_string(),
            generation,
        },
        file_evidence_digest: format!("sha256:{}", "c".repeat(64)),
        launch: ProjectedMcpLaunch::Stdio {
            executable: PathBuf::from("bin/library"),
            args: vec!["--serve".to_string()],
        },
    }
}

fn fixture_mcp_capability(
    generation: u64,
    package_digest: &str,
    manifest_digest: &str,
) -> CapabilityBinding {
    let mut binding = fixture_runtime_capability(generation, package_digest, manifest_digest);
    binding.surfaces = vec!["mcp".to_string()];
    binding.tool_tasks.clear();
    binding.mcp_servers = vec![fixture_mcp_projection(
        generation,
        package_digest,
        manifest_digest,
    )];
    binding
}

#[cfg(any(unix, windows))]
fn assert_expected_real_process_startup_warning(warning: Option<&str>) {
    let Some(warning) = warning else {
        return;
    };
    assert!(
        warning.split("; ").all(|message| {
            message.starts_with("A3S Use startup discovery exceeded ")
                || message
                    .starts_with("A3S Use initial capability projection is still converging after ")
        }),
        "unexpected real-process startup warning: {warning}"
    );
}

fn test_config() -> a3s_code_core::CodeConfig {
    a3s_code_core::CodeConfig::from_acl(
        r#"
                default_model = "openai/gpt-4o"

                providers "openai" {
                  api_key = "sk-test"

                  models "gpt-4o" {
                    name = "GPT-4o"
                  }
                }
            "#,
    )
    .expect("valid test config")
}

fn test_extension_paths(root: &std::path::Path) -> ExtensionPaths {
    ExtensionPaths::new(root.join("use-data"), root.join("use-state"))
}

fn fixture_skill() -> &'static str {
    r#"---
name: fixture-report
description: Build fixture reports
allowed-tools: Read(*)
kind: instruction
---
# Fixture Report

Build a concise report.
"#
}

async fn prepare_atomic_skill_package(
    source: &Path,
    version: &str,
    marker: &str,
    lifecycle_generation: u64,
) -> (
    a3s_use_extension::ExtensionLifecyclePackage,
    a3s_use_extension::ExtensionLifecycleIdentity,
) {
    tokio::fs::create_dir_all(source.join("skills/fixture-report"))
        .await
        .unwrap();
    tokio::fs::write(source.join("README.md"), format!("# Report {version}\n"))
        .await
        .unwrap();
    tokio::fs::write(
        source.join("skills/fixture-report/SKILL.md"),
        format!(
            "---\nname: fixture-report\ndescription: {marker}\nkind: instruction\n---\n\n# Fixture Report\n\n{marker}\n"
        ),
    )
    .await
    .unwrap();
    tokio::fs::write(
        source.join("a3s-use-extension.acl"),
        format!(
            r#"extension "acme/report" {{
  schema_version = 3
  version        = "{version}"
  route          = "report"
  requires_use   = ">=0.3.0, <0.4.0"
  actions        = ["read"]

  repository {{
    url      = "https://github.com/acme/report"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }}

  skill "fixture-report" {{
    path          = "skills/fixture-report/SKILL.md"
    requires_tool = []
    requires_mcp  = []
    optional      = false
  }}
}}
"#
        ),
    )
    .await
    .unwrap();
    let package =
        a3s_use_extension::ExtensionLifecyclePackage::prepare_local("acme/report", source, true)
            .await
            .unwrap();
    let identity = a3s_use_extension::ExtensionLifecycleIdentity::new(
        package.package_id(),
        package.package_digest(),
        package.manifest_digest(),
        lifecycle_generation,
    )
    .unwrap();
    (package, identity)
}

async fn desired_atomic_skill_generation(
    registry: Arc<CapabilityRegistry>,
    snapshot: CapabilityRegistrySnapshot,
) -> DesiredCapabilities {
    let projected: RegistrySnapshot = serde_json::from_value(
        serde_json::to_value(&snapshot).expect("serialize native Use snapshot"),
    )
    .expect("native Use snapshot must match the CLI projection contract");
    let authority = CapabilitySnapshotAuthority::native(registry, snapshot)
        .expect("native Use snapshot must be fully leasable");
    let mut desired = DesiredCapabilities {
        generation: projected.generation,
        revision: projected.revision.clone(),
        capability_snapshot: Some(authority),
        ..DesiredCapabilities::default()
    };
    let report = projected
        .capabilities
        .iter()
        .find(|binding| binding.origin == CapabilityOrigin::Extension && binding.route == "report")
        .expect("native Use snapshot must contain the report extension");
    add_projected_capabilities(&mut desired, report, None)
        .await
        .expect("report Skill must resolve from one native snapshot");
    desired
}

fn fixture_skill_digest() -> String {
    use sha2::{Digest, Sha256};

    format!("{:x}", Sha256::digest(fixture_skill().as_bytes()))
}

fn fixture_asset_digest(value: &str) -> String {
    use sha2::{Digest, Sha256};

    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn fixture_flow() -> &'static str {
    "export async function run(input: unknown): Promise<unknown> { return input; }\n"
}

fn fixture_flow_digest() -> String {
    fixture_asset_digest(fixture_flow())
}

async fn staged_fixture_knowledge(paths: &ExtensionPaths) -> OkfCapabilityProjection {
    use a3s_use::okf_knowledge::{
        OkfKnowledgeClient, OkfKnowledgeStageRequest, OkfKnowledgeStageSpec,
        SqliteOkfKnowledgeAdapter,
    };
    use a3s_use_core::{
        inspect_okf_bundle_files, OkfBundleContract, OkfBundleFile, OkfBundleLimits,
        OkfFormatVersion, PlanQualifiedSurfaceRef, PlanScope, PlanScopeKind, PluginSurfaceKind,
        PluginSurfaceRef, OKF_BUNDLE_CONTRACT_SCHEMA,
    };

    let files = vec![OkfBundleFile::new(
        "concepts/hot-plug.md",
        "---\ntype: Decision\n---\n\n# Managed hot plug\n\nThe registry records registryhotplugneedle.\n",
    )];
    let limits = OkfBundleLimits::default();
    let inspection =
        inspect_okf_bundle_files(OkfFormatVersion::V0_2, limits.clone(), &files).unwrap();
    let bundle = OkfBundleContract {
        schema: OKF_BUNDLE_CONTRACT_SCHEMA.to_string(),
        format_version: inspection.format_version,
        root: "knowledge".to_string(),
        content_digest: inspection.content_digest,
        concept_count: inspection.concept_count,
        file_count: inspection.file_count,
        expanded_bytes: inspection.expanded_bytes,
        limits,
    };
    let client = OkfKnowledgeClient::new(Arc::new(
        SqliteOkfKnowledgeAdapter::from_extension_paths(paths),
    ));
    let staged = client
        .stage(
            OkfKnowledgeStageRequest::new(
                OkfKnowledgeStageSpec {
                    operation_id: "fixture-knowledge-install".to_string(),
                    scope: PlanScope {
                        kind: PlanScopeKind::Workspace,
                        id: "fixture-workspace".to_string(),
                    },
                    surface: PlanQualifiedSurfaceRef {
                        package_id: "acme/report".to_string(),
                        surface: PluginSurfaceRef {
                            kind: PluginSurfaceKind::Okf,
                            id: "domain-knowledge".to_string(),
                        },
                    },
                    generation: 1,
                    package_digest: format!("sha256:{}", "a".repeat(64)),
                    manifest_digest: format!("sha256:{}", "b".repeat(64)),
                    bundle,
                },
                files,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let binding = client.promote(&staged.receipt).await.unwrap();
    OkfCapabilityProjection::from_promoted(&binding.receipt, &binding.observation).unwrap()
}

fn fixture_flow_snapshot(package_root: &Path, source_path: &Path) -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 2,
        "generation": 4,
        "revision": "4444444444444444444444444444444444444444444444444444444444444444",
        "capabilities": [{
            "id": "use/acme/report",
            "route": "report",
            "version": "1.0.0",
            "origin": "extension",
            "enabled": true,
            "readiness": "ready",
            "packageRoot": package_root,
            "lifecycleGeneration": 9,
            "surfaces": ["flow"],
            "flows": [{
                "id": "review",
                "engine": "a3s-flow",
                "runtime": "native-ts",
                "source": {
                    "path": source_path,
                    "sha256": fixture_flow_digest(),
                    "mediaType": "text/typescript"
                },
                "exportName": "run",
                "requiresTools": ["convert"],
                "requiresMcp": ["library"],
                "requiresOkf": ["domain-knowledge"]
            }]
        }]
    })
}

fn fixture_flow_catalog() -> UseFlowCatalog {
    UseFlowCatalog {
        schema_version: 1,
        generation: 4,
        revision: "4".repeat(64),
        items: vec![UseFlowCatalogItem {
            key: "report:review".to_string(),
            package_id: "use/acme/report".to_string(),
            route: "report".to_string(),
            version: "1.0.0".to_string(),
            lifecycle_generation: 9,
            id: "review".to_string(),
            engine: UseFlowEngine::A3sFlow,
            runtime: UseFlowRuntime::NativeTs,
            package_root: PathBuf::from("/managed/acme-report"),
            source_path: PathBuf::from("/managed/acme-report/flows/review.ts"),
            export_name: "run".to_string(),
            sha256: fixture_flow_digest(),
            media_type: "text/typescript".to_string(),
            requires_tools: vec!["convert".to_string()],
            requires_mcp: vec!["library".to_string()],
            requires_okf: vec!["domain-knowledge".to_string()],
        }],
    }
}

fn fixture_bound_flow_design(
    version: &str,
    lifecycle_generation: u64,
    source_sha256: &str,
) -> String {
    serde_json::json!({
        "version": "a3s.workflow.design.v1",
        "name": "Daily report review",
        "description": "Review the installed report workflow",
        "installedFlow": {
            "schema": "a3s.use.installed-flow.v1",
            "packageId": "use/acme/report",
            "flowId": "review",
            "version": version,
            "lifecycleGeneration": lifecycle_generation,
            "sourceSha256": source_sha256,
        },
        "nodes": [],
        "edges": [],
    })
    .to_string()
}

#[test]
fn use_mcp_timeout_covers_the_longest_bounded_component_install() {
    const {
        assert!(
            MCP_REQUEST_TIMEOUT_SECS >= 15 * 60,
            "Use MCP calls must outlive the bounded 15-minute Browser installer"
        );
    }
}

#[cfg(unix)]
#[derive(Clone, Default)]
struct UseCallingLlm {
    mcp_tool_calls: Arc<std::sync::atomic::AtomicUsize>,
    runtime_tool_calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(unix)]
impl UseCallingLlm {
    fn response(
        &self,
        messages: &[a3s_code_core::Message],
        tools: &[a3s_code_core::llm::ToolDefinition],
    ) -> anyhow::Result<a3s_code_core::LlmResponse> {
        let tool_names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        let runtime_turn = messages.iter().any(|message| {
            message.content.iter().any(|block| match block {
                a3s_code_core::ContentBlock::Text { text } => text.contains("managed Runtime Tool"),
                a3s_code_core::ContentBlock::ToolResult { tool_use_id, .. } => {
                    tool_use_id.starts_with("runtime-fixture-call-")
                }
                _ => false,
            })
        });
        if runtime_turn {
            if !tool_names.contains(&FIXTURE_RUNTIME_TOOL) {
                anyhow::bail!(
                    "Use Runtime fixture tool was not inherited by the Run: {tool_names:?}"
                );
            }
        } else {
            if !tool_names.contains(&"mcp__use_report__fixture_tool") {
                anyhow::bail!(
                    "Use MCP fixture tool was not inherited by the child: {tool_names:?}"
                );
            }
            if let Some(disallowed) = tool_names
                .iter()
                .find(|name| !name.starts_with("mcp__use_") && !name.starts_with("use_tool_"))
            {
                anyhow::bail!("Use child was exposed to disallowed tool '{disallowed}'");
            }
        }
        let completed_tool_call = messages.iter().any(|message| {
            message.content.iter().any(|block| match block {
                a3s_code_core::ContentBlock::ToolResult { tool_use_id, .. } if runtime_turn => {
                    tool_use_id.starts_with("runtime-fixture-call-")
                }
                a3s_code_core::ContentBlock::ToolResult { tool_use_id, .. } => {
                    tool_use_id.starts_with("use-fixture-call-")
                }
                _ => false,
            })
        });
        let (content, stop_reason) = if completed_tool_call {
            (
                vec![a3s_code_core::ContentBlock::Text {
                    text: if runtime_turn {
                        "Use observed the exact managed Runtime Tool result."
                    } else {
                        "Use observed fixture-ok through the report capability."
                    }
                    .to_string(),
                }],
                "end_turn",
            )
        } else if runtime_turn {
            let call = self
                .runtime_tool_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (
                vec![a3s_code_core::ContentBlock::ToolUse {
                    id: format!("runtime-fixture-call-{call}"),
                    name: FIXTURE_RUNTIME_TOOL.to_string(),
                    input: serde_json::json!({"argv": ["from-tui"]}),
                }],
                "tool_use",
            )
        } else {
            let call = self
                .mcp_tool_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (
                vec![a3s_code_core::ContentBlock::ToolUse {
                    id: format!("use-fixture-call-{call}"),
                    name: "mcp__use_report__fixture_tool".to_string(),
                    input: serde_json::json!({}),
                }],
                "tool_use",
            )
        };
        Ok(a3s_code_core::LlmResponse {
            message: a3s_code_core::Message {
                role: "assistant".to_string(),
                content,
                reasoning_content: None,
            },
            usage: a3s_code_core::TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            stop_reason: Some(stop_reason.to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        })
    }
}

#[cfg(unix)]
#[async_trait::async_trait]
impl a3s_code_core::LlmClient for UseCallingLlm {
    async fn complete(
        &self,
        messages: &[a3s_code_core::Message],
        _system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
    ) -> anyhow::Result<a3s_code_core::LlmResponse> {
        self.response(messages, tools)
    }

    async fn complete_streaming(
        &self,
        messages: &[a3s_code_core::Message],
        _system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<a3s_code_core::llm::StreamEvent>> {
        let response = self.response(messages, tools)?;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            if let Some(text) = response
                .message
                .content
                .iter()
                .find_map(|block| match block {
                    a3s_code_core::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            {
                let _ = tx
                    .send(a3s_code_core::llm::StreamEvent::TextDelta(text))
                    .await;
            }
            let _ = tx
                .send(a3s_code_core::llm::StreamEvent::Done(response))
                .await;
        });
        Ok(rx)
    }
}

#[derive(Default)]
struct ScopedRuntimeTaskCallingLlm {
    calls: std::sync::atomic::AtomicUsize,
}

impl ScopedRuntimeTaskCallingLlm {
    fn response(
        &self,
        tools: &[a3s_code_core::llm::ToolDefinition],
    ) -> anyhow::Result<a3s_code_core::LlmResponse> {
        let tool_names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<BTreeSet<_>>();
        if !tool_names.contains(FIXTURE_RUNTIME_TOOL) {
            anyhow::bail!(
                "the scoped agent did not discover the reviewed Runtime Task: {tool_names:?}"
            );
        }
        if let Some(unexpected) = tool_names
            .iter()
            .find(|name| name.starts_with("mcp__use_") || **name == USE_KNOWLEDGE_SEARCH_TOOL)
        {
            anyhow::bail!(
                "the scoped agent received an excluded compatibility tool '{unexpected}'"
            );
        }

        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (content, stop_reason) = if call == 0 {
            (
                vec![a3s_code_core::ContentBlock::ToolUse {
                    id: "scoped-runtime-task-call".to_string(),
                    name: FIXTURE_RUNTIME_TOOL.to_string(),
                    input: serde_json::json!({"argv": ["from-scoped-agent"]}),
                }],
                "tool_use",
            )
        } else {
            (
                vec![a3s_code_core::ContentBlock::Text {
                    text: "Scoped Runtime Task completed.".to_string(),
                }],
                "end_turn",
            )
        };
        Ok(a3s_code_core::LlmResponse {
            message: a3s_code_core::Message {
                role: "assistant".to_string(),
                content,
                reasoning_content: None,
            },
            usage: a3s_code_core::TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            stop_reason: Some(stop_reason.to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        })
    }
}

#[async_trait::async_trait]
impl a3s_code_core::LlmClient for ScopedRuntimeTaskCallingLlm {
    async fn complete(
        &self,
        _messages: &[a3s_code_core::Message],
        _system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
    ) -> anyhow::Result<a3s_code_core::LlmResponse> {
        self.response(tools)
    }

    async fn complete_streaming(
        &self,
        _messages: &[a3s_code_core::Message],
        _system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<a3s_code_core::llm::StreamEvent>> {
        let response = self.response(tools)?;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            if let Some(text) = response
                .message
                .content
                .iter()
                .find_map(|block| match block {
                    a3s_code_core::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            {
                let _ = tx
                    .send(a3s_code_core::llm::StreamEvent::TextDelta(text))
                    .await;
            }
            let _ = tx
                .send(a3s_code_core::llm::StreamEvent::Done(response))
                .await;
        });
        Ok(rx)
    }
}

struct AtomicSkillCutoverLlm {
    calls: std::sync::atomic::AtomicUsize,
    observed_versions: Mutex<Vec<String>>,
    old_surface_observed: tokio::sync::Semaphore,
    release_old_call: tokio::sync::Semaphore,
}

impl AtomicSkillCutoverLlm {
    fn new() -> Self {
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            observed_versions: Mutex::new(Vec::new()),
            old_surface_observed: tokio::sync::Semaphore::new(0),
            release_old_call: tokio::sync::Semaphore::new(0),
        }
    }

    fn response(
        content: Vec<a3s_code_core::ContentBlock>,
        stop_reason: &str,
    ) -> a3s_code_core::LlmResponse {
        a3s_code_core::LlmResponse {
            message: a3s_code_core::Message {
                role: "assistant".to_string(),
                content,
                reasoning_content: None,
            },
            usage: a3s_code_core::TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            stop_reason: Some(stop_reason.to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        }
    }

    fn search_call(id: &str) -> a3s_code_core::LlmResponse {
        Self::response(
            vec![a3s_code_core::ContentBlock::ToolUse {
                id: id.to_string(),
                name: "search_skills".to_string(),
                input: serde_json::json!({"query": "fixture-report"}),
            }],
            "tool_use",
        )
    }

    fn final_text(text: &str) -> a3s_code_core::LlmResponse {
        Self::response(
            vec![a3s_code_core::ContentBlock::Text {
                text: text.to_string(),
            }],
            "end_turn",
        )
    }

    fn latest_tool_result(messages: &[a3s_code_core::Message]) -> anyhow::Result<String> {
        messages
            .iter()
            .rev()
            .flat_map(|message| message.content.iter().rev())
            .find_map(|block| match block {
                a3s_code_core::ContentBlock::ToolResult { content, .. } => Some(content.as_text()),
                _ => None,
            })
            .context("search_skills result is missing")
    }
}

#[async_trait::async_trait]
impl a3s_code_core::LlmClient for AtomicSkillCutoverLlm {
    async fn complete(
        &self,
        messages: &[a3s_code_core::Message],
        _system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
    ) -> anyhow::Result<a3s_code_core::LlmResponse> {
        if !tools.iter().any(|tool| tool.name == "search_skills") {
            return Ok(Self::final_text("auxiliary completion"));
        }

        match self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
            0 => {
                self.old_surface_observed.add_permits(1);
                self.release_old_call.acquire().await?.forget();
                Ok(Self::search_call("search-generation-one"))
            }
            1 => {
                let result = Self::latest_tool_result(messages)?;
                anyhow::ensure!(
                    result.contains("cli-use-generation-one"),
                    "the admitted N Run resolved another Skill generation: {result}"
                );
                self.observed_versions
                    .lock()
                    .unwrap()
                    .push("generation-one".to_string());
                Ok(Self::final_text("skill generation one complete"))
            }
            2 => Ok(Self::search_call("search-generation-two")),
            3 => {
                let result = Self::latest_tool_result(messages)?;
                anyhow::ensure!(
                    result.contains("cli-use-generation-two"),
                    "the N+1 Run resolved another Skill generation: {result}"
                );
                self.observed_versions
                    .lock()
                    .unwrap()
                    .push("generation-two".to_string());
                Ok(Self::final_text("skill generation two complete"))
            }
            call => anyhow::bail!("unexpected atomic Skill cutover LLM call {call}"),
        }
    }

    async fn complete_streaming(
        &self,
        messages: &[a3s_code_core::Message],
        system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<a3s_code_core::llm::StreamEvent>> {
        let response = self.complete(messages, system, tools).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tokio::spawn(async move {
            let _ = tx
                .send(a3s_code_core::llm::StreamEvent::Done(response))
                .await;
        });
        Ok(rx)
    }
}

#[test]
fn dedicated_use_worker_allows_only_use_mcp_tools() {
    let worker = use_worker_spec(&DesiredCapabilities::default()).into_agent_definition();
    assert_eq!(
        worker.confirmation_inheritance,
        Some(ConfirmationInheritance::InheritParent),
        "Use Ask decisions must reach the parent TUI instead of auto-approval"
    );
    assert_eq!(
        worker
            .permissions
            .check("mcp__use_browser__browser_open", &serde_json::json!({})),
        PermissionDecision::Ask
    );
    assert_eq!(
        worker.permissions.check(
            "mcp__use_browser__agent_browser_open",
            &serde_json::json!({})
        ),
        PermissionDecision::Allow
    );
    for collaboration_tool in [
        "mcp__use_office__office_collaboration_create",
        "mcp__use_office__office_collaboration_inspect",
        "mcp__use_office__office_collaboration_diff",
        "mcp__use_office__office_collaboration_events",
        "mcp__use_office__office_collaboration_apply",
        "mcp__use_office__office_collaboration_checkpoint",
    ] {
        assert_eq!(
            worker
                .permissions
                .check(collaboration_tool, &serde_json::json!({})),
            PermissionDecision::Allow,
            "{collaboration_tool} must run inside the dedicated Use boundary"
        );
        assert!(worker.permissions.expose_to_model(collaboration_tool));
    }
    for installer in [
        "mcp__use_browser__agent_browser_install",
        "mcp__use_office__office_install_compat",
        "mcp__use_ocr__ocr_install",
        "mcp__use_acme_report__install_provider",
    ] {
        assert_eq!(
            worker.permissions.check(installer, &serde_json::json!({})),
            PermissionDecision::Ask,
            "{installer} must inherit the parent confirmation path"
        );
        assert!(worker.permissions.expose_to_model(installer));
    }
    assert_eq!(
        worker
            .permissions
            .check("mcp__use_acme_report__render", &serde_json::json!({})),
        PermissionDecision::Ask
    );
    assert_eq!(
        worker
            .permissions
            .check("mcp__github__search", &serde_json::json!({})),
        PermissionDecision::Deny
    );
    assert_eq!(
        worker
            .permissions
            .check("read", &serde_json::json!({"file_path": "README.md"})),
        PermissionDecision::Deny
    );
    assert_eq!(
        worker
            .permissions
            .check("task", &serde_json::json!({"agent": "general"})),
        PermissionDecision::Deny
    );
    assert!(worker
        .permissions
        .expose_to_model("mcp__use_browser__browser_open"));
    for hidden in ["mcp__github__search", "read", "bash", "task"] {
        assert!(
            !worker.permissions.expose_to_model(hidden),
            "{hidden} must not be model-visible to the Use worker"
        );
    }
    let prompt = worker.prompt.expect("Use worker prompt");
    assert!(prompt.contains("never fall back"));
    assert!(prompt.contains("use.office.outcome_unknown"));
    assert!(prompt.contains("stop without retrying"));
}

#[test]
fn dedicated_use_worker_exposes_only_read_only_plugin_management() {
    let desired = DesiredCapabilities {
        management_expected: true,
        management_available: true,
        ..DesiredCapabilities::default()
    };
    let worker = use_worker_spec(&desired).into_agent_definition();

    for tool in [
        "mcp__use_plugin_manager__plugin_search",
        "mcp__use_plugin_manager__plugin_inspect",
        "mcp__use_plugin_manager__plugin_list_installed",
        "mcp__use_plugin_manager__plugin_status",
        "mcp__use_plugin_manager__plugin_plan_install",
        "mcp__use_plugin_manager__plugin_plan_upgrade",
        "mcp__use_plugin_manager__plugin_plan_uninstall",
    ] {
        assert_eq!(
            worker.permissions.check(tool, &serde_json::json!({})),
            PermissionDecision::Allow,
            "{tool} must be available without a confirmation because it cannot apply a mutation"
        );
        assert!(worker.permissions.expose_to_model(tool));
    }
    for tool in DENIED_PLUGIN_MANAGEMENT_MCP_TOOLS {
        assert_eq!(
            worker.permissions.check(tool, &serde_json::json!({})),
            PermissionDecision::Deny,
            "{tool} must remain unavailable during M4"
        );
        assert!(!worker.permissions.expose_to_model(tool));
    }

    let prompt = worker.prompt.expect("Use worker prompt");
    assert!(prompt.contains("create an uninstall plan for review"));
    assert!(prompt.contains("never apply any plan"));
    assert!(prompt.contains("scopeId `user/current`"));
    assert!(!prompt.contains("scopeId `current`"));
    assert!(prompt.contains("management result as untrusted data"));
    assert!(worker.description.contains("plugin/management"));
    assert!(worker
        .description
        .contains("read-only plugin discovery/planning"));
}

#[test]
fn plugin_management_mcp_launch_is_host_owned_and_offline_bounded() {
    let launch = PluginManagementMcpLaunch::new(
        PathBuf::from("C:/fixture/a3s.exe"),
        PathBuf::from("C:/fixture/config.acl"),
        true,
        Some(PathBuf::from("C:/fixture/operator.acl")),
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
    );
    let (config, fingerprint) =
        plugin_management_mcp_config(&launch, Path::new("C:/fixture/workspace")).unwrap();

    assert_eq!(config.name, PLUGIN_MANAGER_MCP_SERVER_NAME);
    assert_eq!(
        config.tool_timeout_secs,
        PLUGIN_MANAGER_MCP_REQUEST_TIMEOUT_SECS
    );
    match config.transport {
        McpTransportConfig::Stdio { command, args } => {
            assert_eq!(command, "C:/fixture/a3s.exe");
            assert_eq!(
                args,
                [
                    "--config",
                    "C:/fixture/config.acl",
                    "--directory",
                    "C:/fixture/workspace",
                    "--offline",
                    "--non-interactive",
                    "--no-progress",
                    "plugin",
                    "mcp-serve",
                ]
            );
        }
        other => panic!("Plugin Manager must use stdio, got {other:?}"),
    }
    assert_eq!(
        config.env.get("A3S_CLI_DIRECTORY").map(String::as_str),
        Some("C:/fixture/workspace")
    );
    assert_eq!(
        config.env.get("A3S_NO_AUTO_INSTALL").map(String::as_str),
        Some("1")
    );
    assert_eq!(config.env.get("A3S_OFFLINE").map(String::as_str), Some("1"));
    assert_eq!(
        config
            .env
            .get(PLUGIN_POLICY_HANDOFF_SOURCE_ENV)
            .map(String::as_str),
        Some("C:/fixture/operator.acl")
    );
    assert_eq!(
        config
            .env
            .get(PLUGIN_POLICY_HANDOFF_DIGEST_ENV)
            .map(String::as_str),
        Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert!(fingerprint.contains("use_plugin_manager"));
}

#[tokio::test]
async fn unavailable_reviewed_runtime_provider_skips_the_tool_with_a_warning() {
    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let binding = fixture_runtime_capability(1, &package_digest, &manifest_digest);
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 1,
        revision: "1".repeat(64),
        capabilities: vec![binding.clone()],
    };
    validate_snapshot(&snapshot).unwrap();
    let mut mismatched_generation = binding.clone();
    mismatched_generation.lifecycle_generation = Some(2);
    let mismatch = validate_snapshot(&RegistrySnapshot {
        capabilities: vec![mismatched_generation],
        ..snapshot.clone()
    })
    .expect_err("the projected Task must match the package lifecycle generation");
    assert!(
        mismatch.to_string().contains("lifecycle identity"),
        "{mismatch:#}"
    );

    let client = UseRegistryClient::for_test(PathBuf::from("unused"), PathBuf::from("."));
    let mut desired = DesiredCapabilities::default();
    let invoker = RecordingRuntimeTaskInvoker::default();
    client
        .add_projected_capabilities(&mut desired, &binding, Some(&invoker))
        .await
        .unwrap();

    assert!(desired.tool_tasks.is_empty());
    assert_eq!(desired.warnings.len(), 1);
    assert!(
        desired.warnings[0].contains(FIXTURE_RUNTIME_TOOL)
            && desired.warnings[0].contains("unavailable reviewed provider 'test-runtime'"),
        "{}",
        desired.warnings[0]
    );
}

#[tokio::test]
async fn scoped_agent_discovers_and_invokes_only_the_reviewed_runtime_task() {
    let temporary = tempfile::tempdir().unwrap();
    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let binding: CapabilityBinding = serde_json::from_value(serde_json::json!({
        "id": "use/acme/report",
        "route": "report",
        "version": "1.0.0",
        "origin": "extension",
        "enabled": true,
        "readiness": "ready",
        "packageRoot": temporary.path(),
        "lifecycleGeneration": 1,
        "plannerEvidence": {
            "packageId": "acme/report",
            "packageSha256": package_digest,
            "manifestSha256": manifest_digest
        },
        "surfaces": ["mcp", "tool"],
        "mcp": {"target": "acme/report", "transport": "stdio"},
        "toolTasks": [fixture_runtime_task_projection(
            1,
            &package_digest,
            &manifest_digest
        )]
    }))
    .unwrap();
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 1,
        revision: "1".repeat(64),
        capabilities: vec![binding.clone()],
    };
    validate_snapshot(&snapshot).unwrap();
    let authority = CapabilitySnapshotAuthority::fixture(&snapshot).unwrap();
    let invoker = Arc::new(RecordingRuntimeTaskInvoker::with_provider("test-runtime"));
    let runtime_tasks = Arc::clone(&invoker) as Arc<dyn RuntimeTaskInvoker>;
    let mut desired = DesiredCapabilities {
        generation: snapshot.generation,
        revision: snapshot.revision.clone(),
        capability_snapshot: Some(authority),
        ..DesiredCapabilities::default()
    };
    add_projected_capabilities_for_mode(
        &mut desired,
        &binding,
        Some(runtime_tasks.as_ref()),
        ProjectionMode::AtomicScoped,
    )
    .await
    .unwrap();
    assert!(desired.mcp.is_empty());
    assert!(desired.flows.is_empty());
    assert!(desired.knowledge.is_empty());
    assert_eq!(desired.tool_tasks.len(), 1);

    let llm = Arc::new(ScopedRuntimeTaskCallingLlm::default());
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(
                temporary.path().join("workspace").display().to_string(),
                Some(
                    a3s_code_core::SessionOptions::new()
                        .with_llm_client(Arc::clone(&llm) as Arc<dyn a3s_code_core::LlmClient>)
                        .with_confirmation_manager(Arc::new(
                            a3s_code_core::hitl::AutoApproveConfirmation,
                        )),
                ),
            )
            .await
            .unwrap(),
    );
    let paths = test_extension_paths(temporary.path());
    let (desired_tx, _) = watch::channel(Arc::new(desired.clone()));
    let knowledge = UseKnowledgeCarrier::new(desired_tx.clone(), &paths);
    let host = ProjectionHost::atomic_scoped(Some(runtime_tasks), None);
    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let (progress_tx, progress_rx) = watch::channel(SessionProjectionProgress::default());
    reconcile(
        Path::new("unused-a3s-use"),
        &host,
        &knowledge,
        &mut applied,
        &desired,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();

    let progress = progress_rx.borrow().clone();
    assert_eq!(progress.tools.len(), 1);
    assert!(progress.tools.contains(FIXTURE_RUNTIME_TOOL));
    assert!(atomic_projection_matches_current_catalog(
        session.as_ref(),
        &desired,
        &progress
    ));
    let expected_runtime_tasks = desired
        .tool_tasks
        .iter()
        .map(|(name, task)| (name.clone(), task.fingerprint().to_string()))
        .collect::<BTreeMap<_, _>>();
    let evidence = scoped_runtime_task_catalog_evidence(&expected_runtime_tasks).unwrap();
    assert_eq!(evidence.count, 1);
    assert!(evidence.digest.starts_with("sha256:"));
    let mut changed_tasks = expected_runtime_tasks;
    changed_tasks.insert(
        FIXTURE_RUNTIME_TOOL.to_string(),
        "different-reviewed-generation".to_string(),
    );
    assert_ne!(
        scoped_runtime_task_catalog_evidence(&changed_tasks)
            .unwrap()
            .digest,
        evidence.digest
    );
    let handle = UseRegistryHandle {
        inner: Arc::new(UseRegistryInner {
            executable: PathBuf::from("unused-a3s-use"),
            directory: temporary.path().to_path_buf(),
            plugin_management: None,
            runtime_tasks: None,
            mcp_runtime: None,
            mode: ProjectionMode::AtomicScoped,
            desired_tx,
            knowledge: knowledge.clone(),
            cancellation: CancellationToken::new(),
            projections: Mutex::new(BTreeMap::new()),
            registry_task: Mutex::new(None),
        }),
    };
    let frozen = handle
        .scoped_runtime_evidence_from(session.as_ref(), &progress)
        .expect("the exact reviewed Runtime Task catalog must freeze");
    assert_eq!(frozen.runtime_tasks, evidence);

    let response = session
        .send("Invoke the reviewed report conversion Runtime Task.", None)
        .await
        .unwrap();
    assert_eq!(response.text, "Scoped Runtime Task completed.");
    assert_eq!(invoker.generations(), vec![1]);
    {
        let requests = invoker
            .requests
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        assert_eq!(
            requests[0].invocation().args(),
            &["from-scoped-agent".to_string()]
        );
    }
    session.close().await;
}

#[tokio::test]
async fn runtime_tool_upgrade_replaces_exact_generation_and_disable_withdraws_it() {
    let llm = Arc::new(RuntimeToolCallingLlm::default());
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(
                ".",
                Some(
                    a3s_code_core::SessionOptions::new()
                        .with_llm_client(llm as Arc<dyn a3s_code_core::LlmClient>)
                        .with_confirmation_manager(Arc::new(
                            a3s_code_core::hitl::AutoApproveConfirmation,
                        )),
                ),
            )
            .await
            .unwrap(),
    );
    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let invoker = Arc::new(RecordingRuntimeTaskInvoker::with_provider("test-runtime"));
    let runtime_tasks = Arc::clone(&invoker) as Arc<dyn RuntimeTaskInvoker>;
    let client = UseRegistryClient::for_test(PathBuf::from("unused"), PathBuf::from("."));
    let (progress_tx, progress_rx) = watch::channel(SessionProjectionProgress::default());

    let package_v1 = format!("sha256:{}", "a".repeat(64));
    let manifest_v1 = format!("sha256:{}", "b".repeat(64));
    let binding_v1 = fixture_runtime_capability(1, &package_v1, &manifest_v1);
    let revision_v1 = "1".repeat(64);
    let snapshot_v1 = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 1,
        revision: revision_v1.clone(),
        capabilities: vec![binding_v1.clone()],
    };
    let mut desired_v1 = DesiredCapabilities {
        generation: 1,
        revision: revision_v1,
        capability_snapshot: Some(CapabilitySnapshotAuthority::fixture(&snapshot_v1).unwrap()),
        ..DesiredCapabilities::default()
    };
    client
        .add_projected_capabilities(&mut desired_v1, &binding_v1, Some(runtime_tasks.as_ref()))
        .await
        .unwrap();
    reconcile_atomic_projection(
        &mut applied,
        &desired_v1,
        None,
        Some(&runtime_tasks),
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();
    assert!(
        !session
            .tool_names()
            .iter()
            .any(|name| name == FIXTURE_RUNTIME_TOOL),
        "projected Runtime Tools must not be written into the compatibility registry"
    );
    assert_eq!(session.capability_catalog_stamp().generation().get(), 1);
    assert!(progress_rx.borrow().tools.contains(FIXTURE_RUNTIME_TOOL));
    let first = session
        .send("Run the exact managed Runtime Tool once.", None)
        .await
        .unwrap();
    assert_eq!(first.tool_calls_count, 1);
    assert_eq!(invoker.generations(), vec![1]);

    let package_v2 = format!("sha256:{}", "c".repeat(64));
    let manifest_v2 = format!("sha256:{}", "d".repeat(64));
    let binding_v2 = fixture_runtime_capability(2, &package_v2, &manifest_v2);
    let revision_v2 = "2".repeat(64);
    let snapshot_v2 = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 2,
        revision: revision_v2.clone(),
        capabilities: vec![binding_v2.clone()],
    };
    let mut desired_v2 = DesiredCapabilities {
        generation: 2,
        revision: revision_v2,
        capability_snapshot: Some(CapabilitySnapshotAuthority::fixture(&snapshot_v2).unwrap()),
        ..DesiredCapabilities::default()
    };
    client
        .add_projected_capabilities(&mut desired_v2, &binding_v2, Some(runtime_tasks.as_ref()))
        .await
        .unwrap();
    reconcile_atomic_projection(
        &mut applied,
        &desired_v2,
        None,
        Some(&runtime_tasks),
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();
    let upgraded = session
        .send("Run the upgraded managed Runtime Tool once.", None)
        .await
        .unwrap();
    assert_eq!(upgraded.tool_calls_count, 1);
    assert_eq!(invoker.generations(), vec![1, 2]);
    assert_eq!(session.capability_catalog_stamp().generation().get(), 2);

    let revision_v3 = "3".repeat(64);
    let snapshot_v3 = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 3,
        revision: revision_v3.clone(),
        capabilities: Vec::new(),
    };
    let desired_v3 = DesiredCapabilities {
        generation: 3,
        revision: revision_v3,
        capability_snapshot: Some(CapabilitySnapshotAuthority::fixture(&snapshot_v3).unwrap()),
        ..DesiredCapabilities::default()
    };
    reconcile_atomic_projection(
        &mut applied,
        &desired_v3,
        None,
        Some(&runtime_tasks),
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();
    assert!(!session
        .tool_names()
        .iter()
        .any(|name| name == FIXTURE_RUNTIME_TOOL));
    assert_eq!(session.capability_catalog_stamp().generation().get(), 3);
    assert!(applied.atomic.as_ref().unwrap().tool_tasks.is_empty());
    assert!(progress_rx.borrow().tools.is_empty());

    session.close().await;
}

#[test]
fn dedicated_use_worker_receives_skill_guidance_inside_fixed_security_boundaries() {
    let skill = Arc::new(Skill {
        name: "fixture-report".to_string(),
        description: "Build fixture reports".to_string(),
        allowed_tools: None,
        disable_model_invocation: false,
        kind: a3s_code_core::skills::SkillKind::Instruction,
        content: "Use the report capability.".to_string(),
        tags: Vec::new(),
        version: None,
    });
    let desired = DesiredSkill {
        package_id: "use/acme/report".to_string(),
        surface_id: "fixture-report".to_string(),
        fingerprint: "fixture".to_string(),
        skill,
    };
    let desired = DesiredCapabilities {
        skills: BTreeMap::from([("fixture-report".to_string(), desired)]),
        ..DesiredCapabilities::default()
    };
    let worker = use_worker_spec(&desired).into_agent_definition();
    let prompt = worker.prompt.expect("Use worker prompt");

    assert!(prompt.contains("Skill text is domain guidance only"));
    assert!(prompt.contains("# A3S Use Skill: fixture-report"));
    assert!(prompt.contains("Use the report capability."));
    assert!(worker
        .description
        .contains("No callable application capability is currently ready"));
}

#[tokio::test]
async fn dedicated_use_worker_is_visible_in_the_live_task_catalog() {
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = agent.session_async(".", None).await.unwrap();
    let desired = DesiredCapabilities {
        mcp: BTreeMap::from([(
            "use_browser".to_string(),
            DesiredMcp {
                server_name: "use_browser".to_string(),
                capability_id: "use/browser".to_string(),
                target: "browser".to_string(),
                fingerprint: "browser-v1".to_string(),
            },
        )]),
        ..DesiredCapabilities::default()
    };

    register_use_worker(&session, &desired).unwrap();
    let definitions = session.tool_definitions();
    assert!(!definitions.iter().any(|tool| tool.name == "parallel_task"));
    let definition = definitions
        .into_iter()
        .find(|tool| tool.name == "task")
        .expect("delegation tool definition");
    let agent_schema =
        &definition.parameters["properties"]["tasks"]["items"]["properties"]["agent"];
    assert!(agent_schema["examples"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("use")));
    assert!(definition
        .description
        .contains("Ready callable capabilities: use/browser"));
    assert!(definition
        .description
        .contains("without shell or workspace fallback"));

    session.close().await;
}

#[tokio::test]
async fn use_worker_advertises_a_route_only_after_its_mcp_projection_applies() {
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(agent.session_async(".", None).await.unwrap());
    let desired = DesiredCapabilities {
        mcp: BTreeMap::from([(
            "use_browser".to_string(),
            DesiredMcp {
                server_name: "use_browser".to_string(),
                capability_id: "use/browser".to_string(),
                target: "browser".to_string(),
                fingerprint: "browser-v1".to_string(),
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let mut applied = SessionProjectionState::new(Arc::clone(&session));

    let before = worker_capabilities_for_applied(&applied, &desired);
    assert!(use_worker_spec(&before)
        .description
        .contains("No callable application capability is currently ready"));

    applied
        .mcp
        .insert("use_browser".to_string(), "browser-v1".to_string());
    let after = worker_capabilities_for_applied(&applied, &desired);
    assert!(use_worker_spec(&after)
        .description
        .contains("Ready callable capabilities: use/browser"));

    session.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn process_client_resolves_unified_snapshot_and_managed_skill() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("package");
    std::fs::create_dir_all(package.join("skills/fixture-report")).unwrap();
    std::fs::write(
        package.join("skills/fixture-report/SKILL.md"),
        fixture_skill(),
    )
    .unwrap();
    std::fs::create_dir_all(package.join("flows")).unwrap();
    std::fs::write(package.join("flows/review.ts"), fixture_flow()).unwrap();

    let binding = serde_json::json!({
        "id": "use/acme/report",
        "route": "report",
        "version": "1.0.0",
        "origin": "extension",
        "packageRoot": package,
        "lifecycleGeneration": 7,
        "plannerEvidence": {
            "packageId": "acme/report",
            "packageSha256": format!("sha256:{}", "3".repeat(64)),
            "manifestSha256": format!("sha256:{}", "4".repeat(64))
        },
        "enabled": true,
        "readiness": "ready",
        "surfaces": ["flow", "skill"],
        "skills": [{
            "path": package.join("skills/fixture-report/SKILL.md"),
            "sha256": fixture_skill_digest()
        }],
        "flows": [{
            "id": "review",
            "engine": "a3s-flow",
            "runtime": "native-ts",
            "source": {
                "path": package.join("flows/review.ts"),
                "sha256": fixture_flow_digest(),
                "mediaType": "text/typescript"
            },
            "exportName": "run",
            "requiresTools": ["convert"],
            "requiresMcp": ["library"],
            "requiresOkf": ["domain-knowledge"]
        }]
    });
    let snapshot = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 2,
            "generation": 7,
            "revision": "1111111111111111111111111111111111111111111111111111111111111111",
            "capabilities": [binding]
        }}
    });
    let executable = temp.path().join("a3s-use-fixture");
    let script = format!(
        "#!/bin/sh\ncase \"$1 $2\" in\n  \"capability snapshot\") printf '%s\\n' '{}' ;;\n  *) exit 2 ;;\nesac\n",
        shell_single_quote(&snapshot.to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let client = UseRegistryClient::for_test(executable.clone(), temp.path().to_path_buf());
    let snapshot = client.snapshot().await.unwrap();
    let desired = client.stable_desired(snapshot, None).await.unwrap();

    assert_eq!(desired.generation, 7);
    assert!(desired.mcp.is_empty());
    assert_eq!(desired.skills.len(), 1);
    assert_eq!(desired.flows.len(), 1);
    assert!(desired.atomic_flows.is_empty());
    assert!(desired.warnings.iter().any(|warning| {
        warning.contains("report:review") && warning.contains("compatibility catalog")
    }));
    assert_eq!(
        desired.skills["fixture-report"].skill.description,
        "Build fixture reports"
    );
    let flow = &desired.flows["report:review"];
    assert_eq!(flow.package_id, "use/acme/report");
    assert_eq!(flow.lifecycle_generation, 7);
    assert_eq!(flow.engine, UseFlowEngine::A3sFlow);
    assert_eq!(flow.runtime, UseFlowRuntime::NativeTs);
    assert_eq!(flow.source_path, package.join("flows/review.ts"));
    assert_eq!(flow.export_name, "run");
    assert_eq!(flow.requires_tools, ["convert"]);
    assert_eq!(flow.requires_mcp, ["library"]);
    assert_eq!(flow.requires_okf, ["domain-knowledge"]);

    let one_shot = load_flow_catalog(executable, temp.path().to_path_buf())
        .await
        .expect("non-resident Code commands must load the same stable Flow catalog");
    assert_eq!(one_shot.generation, 7);
    assert_eq!(one_shot.items.len(), 1);
    assert_eq!(one_shot.items[0].key, "report:review");
}

#[cfg(unix)]
#[tokio::test]
async fn registry_command_timeout_kills_descendants() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("slow-a3s-use");
    let started = temp.path().join("started");
    let descendant_started = temp.path().join("descendant-started");
    let leak_trigger = temp.path().join("trigger-timeout-leak");
    let leaked = temp.path().join("timeout-leak");
    let script = format!(
        "#!/bin/sh\n: > '{}'\nexec 1>&- 2>&-\n(: > '{}'; while [ ! -e '{}' ]; do sleep 0.05; done; : > '{}') &\nwait\n",
        shell_single_quote(&started.display().to_string()),
        shell_single_quote(&descendant_started.display().to_string()),
        shell_single_quote(&leak_trigger.display().to_string()),
        shell_single_quote(&leaked.display().to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();
    let client = UseRegistryClient::for_test(executable, temp.path().to_path_buf());
    let request = tokio::spawn(async move {
        client
            .run_json::<serde_json::Value>(Vec::new(), Duration::from_secs(5))
            .await
    });
    let startup = tokio::time::timeout(Duration::from_secs(4), async {
        while !started.exists() || !descendant_started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if startup.is_err() {
        if request.is_finished() {
            panic!(
                "fixture registry command exited before starting its descendant: {:?}",
                request.await
            );
        }
        panic!("fixture registry command did not start its descendant");
    }

    let error = request.await.unwrap().unwrap_err();

    assert!(error.to_string().contains("timed out"), "{error:#}");
    std::fs::write(&leak_trigger, []).unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !leaked.exists(),
        "a timed-out registry command must not leave descendants"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn registry_command_cancellation_kills_descendants() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("cancelled-a3s-use");
    let started = temp.path().join("started");
    let descendant_started = temp.path().join("descendant-started");
    let leak_trigger = temp.path().join("trigger-cancellation-leak");
    let leaked = temp.path().join("cancellation-leak");
    let script = format!(
        "#!/bin/sh\n: > '{}'\nexec 1>&- 2>&-\n(: > '{}'; while [ ! -e '{}' ]; do sleep 0.05; done; : > '{}') &\nwait\n",
        shell_single_quote(&started.display().to_string()),
        shell_single_quote(&descendant_started.display().to_string()),
        shell_single_quote(&leak_trigger.display().to_string()),
        shell_single_quote(&leaked.display().to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();
    let cancellation = CancellationToken::new();
    let client =
        UseRegistryClient::new(executable, temp.path().to_path_buf(), cancellation.clone());
    let request = tokio::spawn(async move {
        client
            .run_json::<serde_json::Value>(Vec::new(), Duration::from_secs(10))
            .await
    });
    let startup = tokio::time::timeout(Duration::from_secs(4), async {
        while !started.exists() || !descendant_started.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if startup.is_err() {
        if request.is_finished() {
            panic!(
                "fixture registry command exited before starting its descendant: {:?}",
                request.await
            );
        }
        panic!("fixture registry command did not start");
    }

    cancellation.cancel();
    let error = request.await.unwrap().unwrap_err();

    assert!(error.to_string().contains("cancelled"), "{error:#}");
    std::fs::write(&leak_trigger, []).unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !leaked.exists(),
        "a cancelled registry command must not leave descendants"
    );
}

#[tokio::test]
async fn builtin_ocr_projects_as_use_ocr_tools_and_worker_guidance() {
    use sha2::Digest;

    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("package");
    let skill_path = package.join("skills/a3s-use-ocr/SKILL.md");
    std::fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    let content = r#"---
name: a3s-use-ocr
description: Extract text from local images through A3S Use OCR
---
# A3S Use OCR

Call mcp__use_ocr__ocr_doctor before extraction.
"#;
    std::fs::write(&skill_path, content).unwrap();
    let digest = format!("{:x}", sha2::Sha256::digest(content.as_bytes()));
    let binding = CapabilityBinding {
        id: "use/ocr".to_string(),
        route: "ocr".to_string(),
        version: "0.1.1".to_string(),
        origin: CapabilityOrigin::BuiltIn,
        enabled: true,
        readiness: CapabilityReadiness::Ready,
        package_root: package,
        lifecycle_generation: None,
        planner_evidence: None,
        surfaces: vec!["mcp".to_string(), "skill".to_string()],
        mcp: Some(ProjectedMcpSurface {
            target: "ocr-native".to_string(),
            transport: ProjectedMcpTransport::Stdio,
        }),
        mcp_servers: Vec::new(),
        skills: vec![ProjectedSkillSurface {
            id: "a3s-use-ocr".to_string(),
            path: skill_path,
            sha256: digest,
        }],
        flows: Vec::new(),
        knowledge: Vec::new(),
        activity_bar: Vec::new(),
        tool_tasks: Vec::new(),
    };
    let client = UseRegistryClient::for_test(
        temp.path().join("unused-a3s-use"),
        temp.path().to_path_buf(),
    );
    let mut desired = DesiredCapabilities::default();
    client
        .add_projected_capabilities(&mut desired, &binding, None)
        .await
        .unwrap();

    let ocr = desired.mcp.get("use_ocr").unwrap();
    assert_eq!(ocr.capability_id, "use/ocr");
    assert_eq!(ocr.target, "ocr-native");
    assert_eq!(desired.packages.get("use/ocr"), Some(&true));
    assert_eq!(desired.skills["a3s-use-ocr"].package_id, "use/ocr");
    let worker = use_worker_spec(&desired).into_agent_definition();
    assert!(worker
        .permissions
        .expose_to_model("mcp__use_ocr__ocr_extract"));
    let prompt = worker.prompt.unwrap();
    assert!(prompt.contains("mcp__use_ocr__*"));
    assert!(prompt.contains("mcp__use_ocr__ocr_doctor"));
}

#[tokio::test]
async fn registry_projects_ui_bytes_and_canonical_skill_identity_into_atomic_state() {
    use sha2::Digest;

    let temporary = tempfile::tempdir().unwrap();
    let package = temporary.path().join("package");
    let skill_path = package.join("skills/guide/SKILL.md");
    let entry_path = package.join("ui/review.html");
    tokio::fs::create_dir_all(skill_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::create_dir_all(entry_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&skill_path, fixture_skill())
        .await
        .unwrap();
    let entry = b"<main>exact review</main>";
    tokio::fs::write(&entry_path, entry).await.unwrap();
    let binding = CapabilityBinding {
        id: "use/acme/workbench".to_string(),
        route: "workbench".to_string(),
        version: "1.0.0".to_string(),
        origin: CapabilityOrigin::Extension,
        enabled: true,
        readiness: CapabilityReadiness::Ready,
        package_root: package,
        lifecycle_generation: Some(9),
        planner_evidence: None,
        surfaces: vec!["skill".to_string(), "ui".to_string()],
        mcp: None,
        mcp_servers: Vec::new(),
        skills: vec![ProjectedSkillSurface {
            id: "guide".to_string(),
            path: skill_path,
            sha256: fixture_skill_digest(),
        }],
        flows: Vec::new(),
        knowledge: Vec::new(),
        activity_bar: vec![ProjectedActivityBarContribution {
            id: "review".to_string(),
            title: "Evidence Review".to_string(),
            description: "Review exact evidence.".to_string(),
            icon: "panel-top".to_string(),
            entry: ProjectedManagedAsset {
                path: entry_path,
                sha256: format!("{:x}", sha2::Sha256::digest(entry)),
                media_type: "text/html".to_string(),
            },
            styles: Vec::new(),
            scripts: Vec::new(),
            skill: Some("guide".to_string()),
            dependency_evidence_schema: UI_DEPENDENCY_EVIDENCE_SCHEMA.to_string(),
            dependencies: vec![PluginSurfaceRef {
                kind: a3s_use_core::PluginSurfaceKind::Skill,
                id: "guide".to_string(),
            }],
            order: 80,
        }],
        tool_tasks: Vec::new(),
    };
    validate_snapshot(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 9,
        revision: "9".repeat(64),
        capabilities: vec![binding.clone()],
    })
    .unwrap();
    let mut legacy_without_evidence_schema = binding.clone();
    legacy_without_evidence_schema.activity_bar[0]
        .dependency_evidence_schema
        .clear();
    let error = validate_snapshot(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 9,
        revision: "9".repeat(64),
        capabilities: vec![legacy_without_evidence_schema],
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("missing or unsupported dependency evidence schema"),
        "{error:#}"
    );
    let mut legacy_without_dependencies = binding.clone();
    legacy_without_dependencies.activity_bar[0]
        .dependencies
        .clear();
    let error = validate_snapshot(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 9,
        revision: "9".repeat(64),
        capabilities: vec![legacy_without_dependencies],
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("without canonical dependency evidence"),
        "{error:#}"
    );
    let client = UseRegistryClient::for_test(
        temporary.path().join("unused-a3s-use"),
        temporary.path().to_path_buf(),
    );
    let mut desired = DesiredCapabilities::default();
    client
        .add_projected_capabilities(&mut desired, &binding, None)
        .await
        .unwrap();

    assert_eq!(desired.skills["fixture-report"].surface_id, "guide");
    let ui = &desired.ui["workbench:review"];
    assert_eq!(ui.surface_id, "review");
    assert_eq!(
        ui.binding.document().entry().content(),
        "<main>exact review</main>"
    );
    assert_eq!(ui.dependencies[0].id, "guide");

    let status = render_capability(
        &binding,
        None,
        None,
        &desired,
        &HashMap::new(),
        &BTreeSet::from(["fixture-report"]),
        &PublishedAtomicCapabilities {
            mcp: &BTreeSet::new(),
            tools: &BTreeSet::new(),
            flows: &BTreeSet::new(),
            ui: &BTreeSet::from(["workbench:review"]),
        },
    )
    .join("\n");
    assert!(status.contains("Skill verified + loaded (1/1)"), "{status}");
    assert!(status.contains("UI verified + atomic (1/1)"), "{status}");
}

#[test]
fn status_renderer_keeps_native_office_ready_when_officecli_is_missing() {
    let revision = "a".repeat(64);
    let office = CapabilityBinding {
        id: "use/office".to_string(),
        route: "office".to_string(),
        version: "0.4.0".to_string(),
        origin: CapabilityOrigin::BuiltIn,
        enabled: true,
        readiness: CapabilityReadiness::Ready,
        package_root: PathBuf::new(),
        lifecycle_generation: None,
        planner_evidence: None,
        surfaces: vec!["mcp".to_string(), "skill".to_string()],
        mcp: Some(ProjectedMcpSurface {
            target: "office-native".to_string(),
            transport: ProjectedMcpTransport::Stdio,
        }),
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        flows: Vec::new(),
        knowledge: Vec::new(),
        activity_bar: Vec::new(),
        tool_tasks: Vec::new(),
    };
    let office_compat = CapabilityBinding {
        id: "use/office-compat".to_string(),
        route: "office-compat".to_string(),
        version: "0.4.0".to_string(),
        origin: CapabilityOrigin::BuiltIn,
        enabled: true,
        readiness: CapabilityReadiness::Missing,
        package_root: PathBuf::new(),
        lifecycle_generation: None,
        planner_evidence: None,
        surfaces: vec!["mcp".to_string()],
        mcp: None,
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        flows: Vec::new(),
        knowledge: Vec::new(),
        activity_bar: Vec::new(),
        tool_tasks: Vec::new(),
    };
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 8,
        revision: revision.clone(),
        capabilities: vec![office, office_compat],
    };
    let desired = DesiredCapabilities {
        generation: 8,
        revision,
        mcp: BTreeMap::from([(
            "use_office".to_string(),
            DesiredMcp {
                server_name: "use_office".to_string(),
                capability_id: "use/office".to_string(),
                target: "office-native".to_string(),
                fingerprint: "office-v1".to_string(),
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let doctor = UseDoctorData {
        diagnostics: vec![UseDomainDiagnostic {
            domain: "office".to_string(),
            readiness: CapabilityReadiness::Missing,
            provider: None,
            version: None,
            path: None,
            message: "The optional OfficeCLI compatibility provider is missing.".to_string(),
        }],
    };
    let mcp_status = HashMap::from([(
        "use_office".to_string(),
        McpServerStatus {
            name: "use_office".to_string(),
            connected: true,
            enabled: true,
            tool_count: 18,
            error: None,
        },
    )]);

    let status = render_status(UseStatusInput {
        executable: Path::new("/opt/a3s-use"),
        version: Ok(UseVersionData {
            version: "0.4.0".to_string(),
        }),
        snapshot: Ok(snapshot),
        doctor: Ok(doctor),
        ocr_diagnostic: None,
        desired: &desired,
        mcp_status: &mcp_status,
        loaded_skills: &[],
        published_mcp: &[],
        published_tools: &[],
        published_flows: &[],
        published_ui: &[],
        include_repair_guidance: false,
    });

    assert!(
        status.contains("use/office · ready · v0.4.0 · provider native"),
        "{status}"
    );
    assert!(status.contains("MCP connected (18 tools)"), "{status}");
    assert!(status.contains("use/office-compat · missing"), "{status}");
    assert!(
        status.contains("optional OfficeCLI compatibility provider is missing"),
        "{status}"
    );
}

#[test]
fn status_renderer_discloses_local_ppocr_v6_and_never_runs_repairs() {
    let revision = "b".repeat(64);
    let ocr = CapabilityBinding {
        id: "use/ocr".to_string(),
        route: "ocr".to_string(),
        version: "0.1.1".to_string(),
        origin: CapabilityOrigin::BuiltIn,
        enabled: true,
        readiness: CapabilityReadiness::Ready,
        package_root: PathBuf::from("/opt/a3s-use-ocr"),
        lifecycle_generation: None,
        planner_evidence: None,
        surfaces: vec!["mcp".to_string(), "skill".to_string()],
        mcp: Some(ProjectedMcpSurface {
            target: "ocr-native".to_string(),
            transport: ProjectedMcpTransport::Stdio,
        }),
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        flows: Vec::new(),
        knowledge: Vec::new(),
        activity_bar: Vec::new(),
        tool_tasks: Vec::new(),
    };
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 3,
        revision: revision.clone(),
        capabilities: vec![ocr],
    };
    let desired = DesiredCapabilities {
        generation: 3,
        revision,
        mcp: BTreeMap::from([(
            "use_ocr".to_string(),
            DesiredMcp {
                server_name: "use_ocr".to_string(),
                capability_id: "use/ocr".to_string(),
                target: "ocr-native".to_string(),
                fingerprint: "ocr-v1".to_string(),
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let status = render_status(UseStatusInput {
        executable: Path::new("/opt/a3s-use"),
        version: Ok(UseVersionData {
            version: "0.4.0".to_string(),
        }),
        snapshot: Ok(snapshot.clone()),
        doctor: Ok(UseDoctorData {
            diagnostics: Vec::new(),
        }),
        ocr_diagnostic: Some(Ok(serde_json::json!({
            "readiness": "ready",
            "provider": "pp-ocr-v6",
            "engine": "onnx-runtime",
            "model": "PP-OCRv6_small",
            "sendsSourceOffDevice": false,
            "message": "Local PP-OCRv6 detection and recognition models are ready."
        }))),
        desired: &desired,
        mcp_status: &HashMap::new(),
        loaded_skills: &[],
        published_mcp: &[],
        published_tools: &[],
        published_flows: &[],
        published_ui: &[],
        include_repair_guidance: true,
    });

    assert!(
        status.contains("pp-ocr-v6 · PP-OCRv6_small · local ONNX"),
        "{status}"
    );
    assert!(
        status.contains("repair guidance (never run automatically)"),
        "{status}"
    );
    assert!(
        status.contains("a3s install use --source release"),
        "{status}"
    );
    assert!(
        !status.contains("a3s install use/ocr"),
        "ready PP-OCRv6 models must not receive model repair guidance: {status}"
    );

    let missing_status = render_status(UseStatusInput {
        executable: Path::new("/opt/a3s-use"),
        version: Ok(UseVersionData {
            version: "0.4.0".to_string(),
        }),
        snapshot: Ok(snapshot),
        doctor: Ok(UseDoctorData {
            diagnostics: Vec::new(),
        }),
        ocr_diagnostic: Some(Ok(serde_json::json!({
            "readiness": "missing",
            "provider": "pp-ocr-v6",
            "engine": "onnx-runtime",
            "model": "PP-OCRv6_small",
            "sendsSourceOffDevice": false,
            "message": "The local PP-OCRv6 model bundle is not installed."
        }))),
        desired: &desired,
        mcp_status: &HashMap::new(),
        loaded_skills: &[],
        published_mcp: &[],
        published_tools: &[],
        published_flows: &[],
        published_ui: &[],
        include_repair_guidance: true,
    });
    assert!(
        missing_status.contains("pp-ocr-v6 · PP-OCRv6_small · local ONNX"),
        "{missing_status}"
    );
    assert!(
        missing_status.contains("OCR model: a3s install use/ocr"),
        "{missing_status}"
    );

    let unavailable = unavailable_status_text(true);
    assert!(unavailable.contains("not discovered"), "{unavailable}");
    assert!(
        unavailable.contains("Built-in OCR: update or repair Use"),
        "{unavailable}"
    );
}

#[cfg(unix)]
#[test]
fn generation_watch_hot_plugs_skill_mcp_runtime_task_flow_and_knowledge_across_tui_replacement() {
    std::thread::Builder::new()
        .name("use-generation-watch".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("generation-watch runtime")
                .block_on(generation_watch_hot_plug_scenario());
        })
        .expect("generation-watch test thread")
        .join()
        .expect("generation-watch test thread panicked");
}

#[cfg(unix)]
async fn generation_watch_hot_plug_scenario() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("package");
    std::fs::create_dir_all(package.join("skills/fixture-report")).unwrap();
    std::fs::write(
        package.join("skills/fixture-report/SKILL.md"),
        fixture_skill(),
    )
    .unwrap();
    std::fs::create_dir_all(package.join("flows")).unwrap();
    std::fs::write(package.join("flows/review.ts"), fixture_flow()).unwrap();
    let state = temp.path().join("generation");
    let mcp_log = temp.path().join("mcp-args.log");
    std::fs::write(&state, "1\n").unwrap();
    let knowledge_paths = test_extension_paths(temp.path());
    let knowledge = staged_fixture_knowledge(&knowledge_paths).await;

    let route = serde_json::json!({
        "id": "use/acme/report",
        "route": "report",
        "version": "1.0.0",
        "origin": "extension",
        "packageRoot": package,
        "lifecycleGeneration": 1,
        "plannerEvidence": {
            "packageId": "acme/report",
            "packageSha256": format!("sha256:{}", "a".repeat(64)),
            "manifestSha256": format!("sha256:{}", "b".repeat(64))
        },
        "enabled": true,
        "readiness": "ready",
        "surfaces": ["flow", "mcp", "okf", "skill", "tool"],
        "mcp": {"target": "acme/report", "transport": "stdio"},
        "skills": [{
            "path": package.join("skills/fixture-report/SKILL.md"),
            "sha256": fixture_skill_digest()
        }],
        "flows": [{
            "id": "review",
            "engine": "a3s-flow",
            "runtime": "native-ts",
            "source": {
                "path": package.join("flows/review.ts"),
                "sha256": fixture_flow_digest(),
                "mediaType": "text/typescript"
            },
            "exportName": "run",
            "requiresTools": ["convert"],
            "requiresMcp": ["library"],
            "requiresOkf": ["domain-knowledge"]
        }],
        "knowledge": [knowledge],
        "toolTasks": [fixture_runtime_task_projection(
            1,
            &format!("sha256:{}", "a".repeat(64)),
            &format!("sha256:{}", "b".repeat(64))
        )]
    });
    let mut disabled_route = route.clone();
    disabled_route["enabled"] = serde_json::Value::Bool(false);
    disabled_route.as_object_mut().unwrap().remove("flows");
    disabled_route.as_object_mut().unwrap().remove("knowledge");
    disabled_route.as_object_mut().unwrap().remove("toolTasks");
    let snapshot_one = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 2,
            "generation": 1,
            "revision": "1111111111111111111111111111111111111111111111111111111111111111",
            "capabilities": [route.clone()]
        }}
    });
    let snapshot_two = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 2,
            "generation": 2,
            "revision": "2222222222222222222222222222222222222222222222222222222222222222",
            "capabilities": [disabled_route]
        }}
    });
    let snapshot_three = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 2,
            "generation": 3,
            "revision": "3333333333333333333333333333333333333333333333333333333333333333",
            "capabilities": [route]
        }}
    });
    let watch_two = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {
            "changed": true,
            "registry": snapshot_two["data"]["registry"]
        }
    });
    let watch_one = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {
            "changed": true,
            "registry": snapshot_one["data"]["registry"]
        }
    });
    let watch_three = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {
            "changed": true,
            "registry": snapshot_three["data"]["registry"]
        }
    });
    let executable = temp.path().join("a3s-use-fixture");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "mcp" ] && [ "$2" = "serve" ]; then
  printf '%s\n' "$*" > '{}'
  while IFS= read -r line; do
    case "$line" in
      *'"method":"initialize"'*)
        printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2024-11-05","capabilities":{{}},"serverInfo":{{"name":"fixture","version":"1.0.0"}}}}}}'
        ;;
      *'"method":"notifications/initialized"'*) ;;
      *'"method":"tools/list"'*)
        printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"fixture_tool","description":"Fixture tool","inputSchema":{{"type":"object"}},"annotations":{{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}}}}]}}}}'
        ;;
      *'"method":"tools/call"'*)
        printf '%s\n' '{{"jsonrpc":"2.0","id":3,"result":{{"content":[{{"type":"text","text":"fixture-ok"}}],"isError":false}}}}'
        ;;
    esac
  done
  exit 0
fi

case "$1 $2" in
  "capability snapshot")
    case "$(tr -d '\n' < '{}')" in
      1) printf '%s\n' '{}' ;;
      2) printf '%s\n' '{}' ;;
      *) printf '%s\n' '{}' ;;
    esac
    ;;
  "capability watch")
    if [ "$4" = "0" ]; then
      printf '%s\n' '{}'
    elif [ "$4" = "1" ]; then
      while [ "$(tr -d '\n' < '{}')" = "1" ]; do sleep 0.05; done
      printf '%s\n' '{}'
    else
      while [ "$(tr -d '\n' < '{}')" != "3" ]; do sleep 0.05; done
      printf '%s\n' '{}'
    fi
    ;;
  *) exit 2 ;;
esac
"#,
        mcp_log.display(),
        state.display(),
        shell_single_quote(&snapshot_one.to_string()),
        shell_single_quote(&snapshot_two.to_string()),
        shell_single_quote(&snapshot_three.to_string()),
        shell_single_quote(&watch_one.to_string()),
        state.display(),
        shell_single_quote(&watch_two.to_string()),
        state.display(),
        shell_single_quote(&watch_three.to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let workspace = tempfile::tempdir().unwrap();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(workspace.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let runtime_invoker = Arc::new(RecordingRuntimeTaskInvoker::with_provider("test-runtime"));
    let runtime_tasks = Arc::clone(&runtime_invoker) as Arc<dyn RuntimeTaskInvoker>;
    let cancellation = CancellationToken::new();
    let discovery = RegistryDiscoveryClient::Fixture(UseRegistryClient::new(
        executable.clone(),
        workspace.path().to_path_buf(),
        cancellation.clone(),
    ));
    let (handle, warning) = start_with_budgets(
        RegistryProcess::new(executable, workspace.path().to_path_buf()),
        knowledge_paths,
        cancellation,
        Arc::clone(&session),
        discovery,
        ProjectionHost::new(None, Some(runtime_tasks), None),
        StartupBudgets::new(STARTUP_DISCOVERY_BUDGET, STARTUP_PROJECTION_BUDGET),
    )
    .await;
    if let Some(warning) = warning {
        assert!(
            warning.contains("startup discovery exceeded"),
            "unexpected startup warning: {warning}"
        );
    }
    wait_for_capabilities(&session, &handle, true).await;
    assert!(
        !session
            .skill_names()
            .iter()
            .any(|name| name == "fixture-report"),
        "Use-projected Skills must not be written into the compatibility registry"
    );
    let installed_flows = handle.flow_catalog();
    assert_eq!(installed_flows.generation, 1);
    assert_eq!(installed_flows.items.len(), 1);
    assert_eq!(installed_flows.items[0].key, "report:review");
    assert_eq!(installed_flows.items[0].lifecycle_generation, 1);
    assert_eq!(handle.knowledge_catalog().generation, 1);
    assert_eq!(handle.knowledge_catalog().projections.len(), 1);
    assert!(session
        .tool_names()
        .iter()
        .any(|name| name == USE_KNOWLEDGE_SEARCH_TOOL));
    assert!(
        handle
            .primary_projection_progress()
            .is_some_and(|progress| progress.tools.contains(FIXTURE_RUNTIME_TOOL)),
        "the managed Runtime Tool must be present in the atomic projection receipt"
    );
    assert!(!session
        .tool_names()
        .iter()
        .any(|name| name == FIXTURE_RUNTIME_TOOL));
    let installed_status = handle.status_text(Arc::clone(&session), false).await;
    assert!(
        installed_status.contains("registry generation 1 · converged"),
        "{installed_status}"
    );
    assert!(
        installed_status.contains("use/acme/report"),
        "{installed_status}"
    );
    assert!(
        installed_status.contains("A3S Flow ready (1/1)"),
        "{installed_status}"
    );
    assert!(
        installed_status.contains("OKF Knowledge ready (1/1)"),
        "{installed_status}"
    );
    assert!(
        installed_status.contains("surfaces flow,mcp,okf,skill,tool"),
        "{installed_status}"
    );
    assert_eq!(
        std::fs::read_to_string(&mcp_log).unwrap(),
        "mcp serve acme/report\n"
    );

    let replacement_workspace = tempfile::tempdir().unwrap();
    let use_client = Arc::new(UseCallingLlm::default());
    let replacement = Arc::new(
        agent
            .session_async(
                replacement_workspace.path().display().to_string(),
                Some(a3s_code_core::SessionOptions::new().with_llm_client(use_client.clone())),
            )
            .await
            .unwrap(),
    );
    let replacement_started = std::time::Instant::now();
    handle.replace_session(Arc::clone(&replacement));
    assert!(replacement_started.elapsed() < Duration::from_millis(100));
    assert!(
        handle
            .wait_until_projection_visible(&replacement, Duration::from_secs(5))
            .await,
        "replacement must publish the current atomic generation"
    );
    assert!(
        !replacement
            .skill_names()
            .iter()
            .any(|name| name == "fixture-report"),
        "replacement must keep Use-projected Skills out of the compatibility registry"
    );
    assert!(
        handle
            .capability_projection("use/acme/report", "fixture-report")
            .skill_ready,
        "replacement must expose the projected Skill through its atomic catalog"
    );
    assert!(
        replacement
            .tool_names()
            .iter()
            .any(|name| name == USE_KNOWLEDGE_SEARCH_TOOL),
        "replacement must receive managed Knowledge synchronously"
    );
    assert!(
        handle
            .primary_projection_progress()
            .is_some_and(|progress| progress.tools.contains(FIXTURE_RUNTIME_TOOL)),
        "replacement TUI session must receive the managed Runtime Tool atomically"
    );
    assert!(!replacement
        .tool_names()
        .iter()
        .any(|name| name == FIXTURE_RUNTIME_TOOL));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if replacement
                .tool_names()
                .iter()
                .any(|name| name == "mcp__use_report__fixture_tool")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("replacement session must reconnect live MCP");
    let knowledge_result = replacement
        .tool(
            USE_KNOWLEDGE_SEARCH_TOOL,
            serde_json::json!({
                "query": "registryhotplugneedle",
                "scope_kind": "workspace",
                "scope_id": "fixture-workspace"
            }),
        )
        .await
        .unwrap();
    assert_eq!(knowledge_result.exit_code, 0, "{}", knowledge_result.output);
    let knowledge_result: serde_json::Value =
        serde_json::from_str(&knowledge_result.output).unwrap();
    assert_eq!(knowledge_result["registryGeneration"], 1);
    assert_eq!(knowledge_result["hits"][0]["citation"]["generation"], 1);
    let runtime_result = replacement
        .send("Run the managed Runtime Tool exactly once.", None)
        .await
        .unwrap();
    assert_eq!(runtime_result.tool_calls_count, 1);
    session.close().await;

    assert_eq!(runtime_invoker.generations(), vec![1]);

    let called = replacement
        .tool("mcp__use_report__fixture_tool", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(called.exit_code, 0, "{}", called.output);
    assert!(called.output.contains("fixture-ok"));

    let delegated = replacement
        .tool(
            "task",
            serde_json::json!({
                "agent": "use",
                "description": "Call the report capability",
                "prompt": "Call the report fixture and return the observed result.",
                "max_steps": 3
            }),
        )
        .await
        .unwrap();
    assert_eq!(delegated.exit_code, 0, "{}", delegated.output);
    assert!(delegated.output.contains("Use observed fixture-ok"));
    assert_eq!(
        use_client
            .mcp_tool_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the Use child should call MCP exactly once"
    );
    assert_eq!(
        use_client
            .runtime_tool_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parent Run should call the Runtime Tool exactly once"
    );

    std::fs::write(&state, "2\n").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let skill_projection =
                handle.capability_projection("use/acme/report", "fixture-report");
            let skill_gone = skill_projection.generation == 2
                && !skill_projection.package_enabled
                && !skill_projection.skill_ready;
            let mcp_gone = !replacement
                .tool_names()
                .iter()
                .any(|name| name == "mcp__use_report__fixture_tool");
            let knowledge_gone = !replacement
                .tool_names()
                .iter()
                .any(|name| name == USE_KNOWLEDGE_SEARCH_TOOL);
            let runtime_tool_gone = handle
                .primary_projection_progress()
                .is_some_and(|progress| progress.tools.is_empty());
            let flow_catalog = handle.flow_catalog();
            let flow_gone = flow_catalog.generation == 2 && flow_catalog.items.is_empty();
            let knowledge_catalog = handle.knowledge_catalog();
            let knowledge_catalog_gone =
                knowledge_catalog.generation == 2 && knowledge_catalog.projections.is_empty();
            if skill_gone
                && mcp_gone
                && knowledge_gone
                && runtime_tool_gone
                && flow_gone
                && knowledge_catalog_gone
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("generation 2 must remove live capabilities");
    assert!(
        !replacement
            .skill_names()
            .iter()
            .any(|name| name == "fixture-report"),
        "withdrawn projected Skills must remain absent from the compatibility registry"
    );
    let task_definition = replacement
        .tool_definitions()
        .into_iter()
        .find(|tool| tool.name == "task")
        .expect("task definition after capability removal");
    assert!(task_definition
        .description
        .contains("No callable application capability is currently ready"));
    assert!(!task_definition.description.contains("use/acme/report"));
    let disabled_status = handle.status_text(Arc::clone(&replacement), false).await;
    assert!(
        disabled_status.contains("registry generation 2 · converged"),
        "{disabled_status}"
    );
    assert!(
        disabled_status.contains("use/acme/report · disabled"),
        "{disabled_status}"
    );
    assert!(
        disabled_status.contains("A3S Flow disabled"),
        "{disabled_status}"
    );
    assert!(
        disabled_status.contains("OKF Knowledge disabled"),
        "{disabled_status}"
    );
    assert!(
        !disabled_status.contains("A3S Flow ready (1/1)"),
        "{disabled_status}"
    );

    std::fs::write(&state, "3\n").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let skill_projection =
                handle.capability_projection("use/acme/report", "fixture-report");
            let skill_ready = skill_projection.generation == 3
                && skill_projection.package_enabled
                && skill_projection.skill_ready;
            let mcp_ready = replacement
                .tool_names()
                .iter()
                .any(|name| name == "mcp__use_report__fixture_tool");
            let knowledge_ready = replacement
                .tool_names()
                .iter()
                .any(|name| name == USE_KNOWLEDGE_SEARCH_TOOL);
            let runtime_tool_ready = handle
                .primary_projection_progress()
                .is_some_and(|progress| progress.tools.contains(FIXTURE_RUNTIME_TOOL));
            let flow_catalog = handle.flow_catalog();
            let flow_ready = flow_catalog.generation == 3
                && flow_catalog
                    .items
                    .iter()
                    .any(|flow| flow.key == "report:review");
            let knowledge_catalog = handle.knowledge_catalog();
            let knowledge_catalog_ready =
                knowledge_catalog.generation == 3 && knowledge_catalog.projections.len() == 1;
            if skill_ready
                && mcp_ready
                && knowledge_ready
                && runtime_tool_ready
                && flow_ready
                && knowledge_catalog_ready
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("generation 3 must restore live capabilities");
    assert!(
        !replacement
            .skill_names()
            .iter()
            .any(|name| name == "fixture-report"),
        "restored projected Skills must remain absent from the compatibility registry"
    );
    let restored_tui = replacement
        .send(
            "Run the managed Runtime Tool exactly once after restore.",
            None,
        )
        .await
        .unwrap();
    assert_eq!(restored_tui.tool_calls_count, 1);
    assert_eq!(runtime_invoker.generations(), vec![1, 1]);
    assert_eq!(
        use_client
            .runtime_tool_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    let enabled_status = handle.status_text(Arc::clone(&replacement), false).await;
    assert!(
        enabled_status.contains("registry generation 3 · converged"),
        "{enabled_status}"
    );
    assert!(
        enabled_status.contains("A3S Flow ready (1/1)"),
        "{enabled_status}"
    );
    assert!(
        enabled_status.contains("OKF Knowledge ready (1/1)"),
        "{enabled_status}"
    );

    handle.shutdown().await;
    replacement.close().await;
}

/// Crosses the real `a3s-use` process boundary instead of using the shell
/// contract fixture above. The monorepo orchestration recipe builds Use and
/// supplies `A3S_USE_E2E_BIN`; the test stays ignored for standalone CLI
/// checkouts where that independently released component is unavailable.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires A3S_USE_E2E_BIN pointing to a real a3s-use binary"]
async fn real_use_process_converges_signed_install_upgrade_rebuild_and_uninstall() {
    use std::os::unix::fs::PermissionsExt;

    use crate::tuf_test_support::{TestRepository, TestServer, FUTURE};

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let binary = std::env::var_os("A3S_USE_E2E_BIN")
        .map(PathBuf::from)
        .expect("A3S_USE_E2E_BIN must point to the real a3s-use binary");
    let binary = std::fs::canonicalize(&binary)
        .unwrap_or_else(|error| panic!("failed to resolve {}: {error}", binary.display()));
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let first = signed_report_target(&temp.path().join("first"), "1.0.0", "fixture-v1");
    let next = signed_report_target(&temp.path().join("next"), "2.0.0", "fixture-v2");
    let repository = TestRepository::with_targets(vec![first, next], 97, FUTURE);
    let server = TestServer::start(repository.routes.clone());

    let executable = temp.path().join("a3s-use-e2e");
    let script = format!(
        "#!/bin/sh\nexport A3S_USE_HOME='{}'\nexec '{}' \"$@\"\n",
        shell_single_quote(&home.display().to_string()),
        shell_single_quote(&binary.display().to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    configure_real_use_registry(&executable, &server, &repository).await;
    run_signed_real_use(&executable, "install", "1.0.0").await;
    assert_completed_real_use_lifecycle(&executable, "install", None).await;

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    let cancellation = CancellationToken::new();
    let (handle, warning) = start(
        executable.clone(),
        workspace.clone(),
        ExtensionPaths::new(home.join("data"), home.join("state")),
        cancellation.clone(),
        Arc::clone(&session),
        None,
        None,
        None,
    )
    .await;
    // Cold CI runners may exceed the bounded startup window. This test proves
    // eventual real-process convergence; dedicated tests above own the budget.
    assert_expected_real_process_startup_warning(warning.as_deref());
    let installed_generation = wait_for_signed_report(&session, &handle, true, 1).await;
    wait_for_builtin_use_surfaces(&session, &handle).await;
    let ocr_doctor = session
        .tool("mcp__use_ocr__ocr_doctor", serde_json::json!({}))
        .await
        .expect("built-in OCR doctor must be callable");
    assert!(ocr_doctor.output.contains("readiness"), "{ocr_doctor:?}");
    let status = handle.status_text(Arc::clone(&session), false).await;
    assert!(status.contains("A3S Use status"), "{status}");
    assert!(status.contains("use/acme/report"), "{status}");
    assert!(status.contains("use/browser"), "{status}");
    assert!(status.contains("use/ocr"), "{status}");
    assert!(status.contains("Skill verified + loaded (1/1)"), "{status}");
    run_signed_real_use(&executable, "upgrade", "2.0.0").await;
    assert_completed_real_use_lifecycle(&executable, "uninstall", Some("upgrade")).await;
    let upgraded_generation =
        wait_for_signed_report(&session, &handle, true, installed_generation + 1).await;

    let replacement_workspace = temp.path().join("replacement-workspace");
    std::fs::create_dir_all(&replacement_workspace).unwrap();
    let replacement = Arc::new(
        agent
            .session_async(replacement_workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    handle.replace_session(Arc::clone(&replacement));
    wait_for_signed_report(&replacement, &handle, true, upgraded_generation).await;
    wait_for_builtin_use_surfaces(&replacement, &handle).await;
    session.close().await;

    run_real_use(
        &executable,
        vec!["uninstall".into(), "acme/report".into(), "--json".into()],
    )
    .await;
    wait_for_signed_report(&replacement, &handle, false, upgraded_generation + 1).await;
    wait_for_builtin_use_surfaces(&replacement, &handle).await;

    cancellation.cancel();
    drop(handle);
    replacement.close().await;
}

/// Exercises the same signed real-process boundary on Windows. The separately
/// built Browser provider still proves a standard MCP child process while the
/// cognitive fixture stays portable and static.
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires A3S_USE_E2E_BIN pointing to a real a3s-use binary"]
async fn real_use_process_converges_signed_install_upgrade_rebuild_and_uninstall() {
    use crate::tuf_test_support::{TestRepository, TestServer, FUTURE};

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let binary = required_e2e_binary("A3S_USE_E2E_BIN");
    let use_home = std::env::var_os("A3S_USE_HOME")
        .map(PathBuf::from)
        .expect("A3S_USE_HOME must isolate the real Use process test");
    assert!(
        use_home.is_absolute(),
        "A3S_USE_HOME must be absolute: {}",
        use_home.display()
    );

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let first = signed_report_target(&temp.path().join("first"), "1.0.0", "fixture-v1");
    let next = signed_report_target(&temp.path().join("next"), "2.0.0", "fixture-v2");
    let repository = TestRepository::with_targets(vec![first, next], 97, FUTURE);
    let server = TestServer::start(repository.routes.clone());

    configure_real_use_registry(&binary, &server, &repository).await;
    run_signed_real_use(&binary, "install", "1.0.0").await;
    assert_completed_real_use_lifecycle(&binary, "install", None).await;

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    let cancellation = CancellationToken::new();
    let (handle, warning) = start(
        binary.clone(),
        workspace,
        ExtensionPaths::new(use_home.join("data"), use_home.join("state")),
        cancellation,
        Arc::clone(&session),
        ProjectionHost::default(),
    )
    .await;
    // Cold CI runners may exceed the bounded startup window. This test proves
    // eventual real-process convergence; dedicated tests above own the budget.
    assert_expected_real_process_startup_warning(warning.as_deref());

    let installed_generation = wait_for_signed_report(&session, &handle, true, 1).await;
    wait_for_builtin_use_surfaces(&session, &handle).await;
    let profiles = session
        .tool(
            "mcp__use_browser__agent_browser_tools_profiles",
            serde_json::json!({}),
        )
        .await
        .expect("the built-in Browser MCP tool must be callable");
    assert_eq!(profiles.exit_code, 0, "{}", profiles.output);
    assert!(profiles.output.contains("core"), "{}", profiles.output);
    run_signed_real_use(&binary, "upgrade", "2.0.0").await;
    assert_completed_real_use_lifecycle(&binary, "uninstall", Some("upgrade")).await;
    let upgraded_generation =
        wait_for_signed_report(&session, &handle, true, installed_generation + 1).await;

    let replacement_workspace = temp.path().join("replacement-workspace");
    std::fs::create_dir_all(&replacement_workspace).unwrap();
    let replacement = Arc::new(
        agent
            .session_async(replacement_workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    handle.replace_session(Arc::clone(&replacement));
    wait_for_signed_report(&replacement, &handle, true, upgraded_generation).await;
    wait_for_builtin_use_surfaces(&replacement, &handle).await;
    session.close().await;

    run_real_use(
        &binary,
        vec!["uninstall".into(), "acme/report".into(), "--json".into()],
    )
    .await;
    wait_for_signed_report(&replacement, &handle, false, upgraded_generation + 1).await;
    wait_for_builtin_use_surfaces(&replacement, &handle).await;

    handle.shutdown().await;
    replacement.close().await;
}

#[cfg(windows)]
fn required_e2e_binary(name: &str) -> PathBuf {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} must point to a real executable"));
    std::fs::canonicalize(&path)
        .unwrap_or_else(|error| panic!("failed to resolve {}: {error}", path.display()))
}

#[cfg(any(unix, windows))]
fn signed_report_target(
    fixture_root: &Path,
    version: &str,
    content: &str,
) -> crate::tuf_test_support::TestTarget {
    use a3s_use_core::{
        CatalogArchive, CatalogAvailability, CatalogPackage, CatalogSurface, PluginCatalogRecord,
        PluginPermissionCeiling, PluginReleaseChannel, PLUGIN_CATALOG_SCHEMA_V3,
        PLUGIN_PERMISSION_SCHEMA,
    };
    use a3s_use_extension::ExtensionManifest;
    use sha2::{Digest, Sha256};

    use crate::tuf_test_support::{
        expanded_archive_fingerprint, host_target, package_directory_archive, TestTarget,
    };

    let package_root = fixture_root.join("package");
    std::fs::create_dir_all(package_root.join("skills/fixture-report")).unwrap();
    std::fs::create_dir_all(package_root.join("ui/reports")).unwrap();
    let manifest = format!(
        r#"extension "acme/report" {{
  schema_version = 3
  version        = "{version}"
  route          = "report"
  requires_use   = ">=0.3.0, <0.4.0"
  actions        = ["read"]

  repository {{
    url      = "https://github.com/acme/report"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }}

  skill "fixture-report" {{
    path          = "skills/fixture-report/SKILL.md"
    requires_tool = []
    requires_mcp  = []
    requires_okf  = []
    requires_flow = []
    optional      = false
  }}

  ui "reports" {{
    title       = "Reports"
    description = "Verified report activity fixture."
    icon        = "file-text"
    entry       = "ui/reports/index.html"
    styles      = ["ui/reports/index.css"]
    scripts     = ["ui/reports/index.js"]
    skill       = "fixture-report"
    bind_tool   = []
    bind_mcp    = []
    bind_flow   = []
    order       = 10
    optional    = false
  }}
}}
"#,
    );
    std::fs::write(package_root.join("a3s-use-extension.acl"), &manifest).unwrap();
    std::fs::write(
        package_root.join("README.md"),
        format!("# Report {version}\n\nSigned Code hot-plug fixture.\n"),
    )
    .unwrap();
    std::fs::write(
        package_root.join("skills/fixture-report/SKILL.md"),
        fixture_skill(),
    )
    .unwrap();
    std::fs::write(
        package_root.join("ui/reports/index.html"),
        format!("<!doctype html><main>{content}</main>\n"),
    )
    .unwrap();
    std::fs::write(
        package_root.join("ui/reports/index.css"),
        "main { color: rebeccapurple; }\n",
    )
    .unwrap();
    std::fs::write(
        package_root.join("ui/reports/index.js"),
        "document.querySelector('main').dataset.ready = 'true';\n",
    )
    .unwrap();

    let archive = package_directory_archive(&package_root);
    let (package_sha256, file_count, expanded_bytes, manifest_bytes) =
        expanded_archive_fingerprint(&archive);
    let parsed = ExtensionManifest::parse_acl(&manifest).unwrap();
    let graph = parsed.plugin_surfaces().unwrap();
    let permission_ceiling = PluginPermissionCeiling {
        schema: PLUGIN_PERMISSION_SCHEMA.to_string(),
        surfaces: Vec::new(),
    };
    let target = host_target();
    let target_name = format!(
        "extensions/acme/report/{version}/stable/{target}/report-{version}-{target}.tar.gz"
    );
    let catalog = PluginCatalogRecord {
        schema: PLUGIN_CATALOG_SCHEMA_V3.to_string(),
        package_id: "acme/report".to_string(),
        display_name: format!("Report {version}"),
        description: "Signed Code hot-plug integration fixture.".to_string(),
        publisher: "acme".to_string(),
        keywords: vec!["fixture".to_string()],
        categories: vec!["test".to_string()],
        version: version.to_string(),
        channel: PluginReleaseChannel::Stable,
        requires_use: ">=0.3.0, <0.4.0".to_string(),
        dependencies: Vec::new(),
        target: target.to_string(),
        surfaces: graph
            .iter()
            .map(|surface| CatalogSurface {
                kind: surface.surface.kind,
                id: surface.surface.id.clone(),
                optional: surface.optional,
                workload: None,
                mcp_transport: None,
                mcp_tool_count: None,
                okf_bundle: None,
                requires: surface.dependencies.clone(),
            })
            .collect(),
        permission_ceiling_digest: permission_ceiling.descriptor_digest().unwrap(),
        permission_ceiling,
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
        repository: "https://github.com/acme/report".to_string(),
        availability: CatalogAvailability::Available,
    };
    catalog.validate().unwrap();

    TestTarget {
        archive,
        target_name,
        custom: Some(serde_json::to_value(catalog).unwrap()),
    }
}

#[cfg(any(unix, windows))]
async fn run_signed_real_use(executable: &Path, action: &str, version: &str) -> serde_json::Value {
    run_real_use(
        executable,
        vec![
            action.to_string(),
            "acme/report".to_string(),
            "--registry-name".to_string(),
            "fixture".to_string(),
            "--version".to_string(),
            version.to_string(),
            "--json".to_string(),
        ],
    )
    .await
}

#[cfg(any(unix, windows))]
async fn configure_real_use_registry(
    executable: &Path,
    server: &crate::tuf_test_support::TestServer,
    repository: &crate::tuf_test_support::TestRepository,
) {
    let configured = run_real_use(
        executable,
        vec![
            "registry".to_string(),
            "source".to_string(),
            "add".to_string(),
            "fixture".to_string(),
            "--url".to_string(),
            server.base_url().to_string(),
            "--trust-root".to_string(),
            repository.root_sha256.clone(),
            "--json".to_string(),
        ],
    )
    .await;
    assert_eq!(
        configured["data"]["registrySources"]["snapshot"]["defaultRegistry"],
        "fixture"
    );
}

#[cfg(any(unix, windows))]
async fn assert_completed_real_use_lifecycle(
    executable: &Path,
    expected_action: &str,
    expected_previous_action: Option<&str>,
) {
    let inspected = run_real_use(
        executable,
        vec![
            "extension".to_string(),
            "inspect".to_string(),
            "acme/report".to_string(),
            "--json".to_string(),
        ],
    )
    .await;
    let lifecycle = &inspected["data"]["lifecycle"];
    assert_eq!(
        lifecycle["schema"],
        "a3s.use.plugin-lifecycle-diagnostic.v1"
    );
    assert_eq!(lifecycle["latest"]["action"], expected_action);
    assert_eq!(lifecycle["latest"]["status"], "completed");
    assert_eq!(
        lifecycle["latest"]["completedCheckpoints"],
        lifecycle["latest"]["totalCheckpoints"]
    );
    match expected_previous_action {
        Some(action) => assert_eq!(lifecycle["previous"]["action"], action),
        None => assert!(lifecycle.get("previous").is_none()),
    }
    let encoded = serde_json::to_string(lifecycle).unwrap();
    assert!(!encoded.contains("idempotencyKey"));
    assert!(!encoded.contains("credential"));
    assert!(!encoded.contains("token"));
}

#[cfg(any(unix, windows))]
async fn run_real_use(executable: &Path, args: Vec<String>) -> serde_json::Value {
    let output = tokio::process::Command::new(executable)
        .args(&args)
        .output()
        .await
        .unwrap_or_else(|error| panic!("failed to run {:?}: {error}", args));
    assert!(
        output.status.success(),
        "a3s-use {:?} failed with {}:\nstdout: {}\nstderr: {}",
        args,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "a3s-use {:?} returned invalid JSON: {error}: {}",
            args,
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[cfg(any(unix, windows))]
async fn wait_for_signed_report(
    session: &AgentSession,
    handle: &UseRegistryHandle,
    expected_present: bool,
    minimum_generation: u64,
) -> u64 {
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let projection = handle.capability_projection("use/acme/report", "fixture-report");
            let converged = projection.generation >= minimum_generation
                && projection.package_enabled == expected_present
                && projection.skill_ready == expected_present;
            if converged {
                break projection.generation;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    let generation = match result {
        Ok(generation) => generation,
        Err(_) => {
            let projection = handle.capability_projection("use/acme/report", "fixture-report");
            panic!(
                "signed report did not converge to present={expected_present} at generation >= {minimum_generation}; projection={projection:?}",
            );
        }
    };
    assert!(
        !session
            .skill_names()
            .iter()
            .any(|name| name == "fixture-report"),
        "signed Use Skills must stay out of the compatibility registry"
    );
    generation
}

#[cfg(unix)]
async fn wait_for_capabilities(session: &AgentSession, handle: &UseRegistryHandle, present: bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let skill_present = handle
                .capability_projection("use/acme/report", "fixture-report")
                .skill_ready;
            let tool_present = session
                .tool_names()
                .iter()
                .any(|name| name == "mcp__use_report__fixture_tool");
            if skill_present == present && tool_present == present {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("capabilities did not converge to present={present}"));
}

#[cfg(any(unix, windows))]
async fn wait_for_builtin_use_surfaces(session: &AgentSession, handle: &UseRegistryHandle) {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let tools = session.tool_names();
            let skills_present = [
                ("use/browser", "a3s-use-browser"),
                ("use/ocr", "a3s-use-ocr"),
            ]
            .iter()
            .all(|(capability_id, skill_name)| {
                handle
                    .capability_projection(capability_id, skill_name)
                    .skill_ready
            });
            let tools_present = [
                "mcp__use_browser__agent_browser_tools_profiles",
                "mcp__use_browser__agent_browser_open",
                "mcp__use_ocr__ocr_doctor",
            ]
            .iter()
            .all(|expected| tools.iter().any(|name| name == expected));
            if skills_present && tools_present {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    if result.is_err() {
        panic!(
            "built-in Browser and OCR did not project into the Code session; browser={:?}; ocr={:?}; tools={:?}",
            handle.capability_projection("use/browser", "a3s-use-browser"),
            handle.capability_projection("use/ocr", "a3s-use-ocr"),
            session.tool_names()
        );
    }
    assert!(
        ["a3s-use-browser", "a3s-use-ocr"]
            .iter()
            .all(|expected| !session.skill_names().iter().any(|name| name == expected)),
        "built-in Use Skills must stay out of the compatibility registry"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn startup_discovery_respects_its_budget() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("slow-a3s-use");
    std::fs::write(&executable, "#!/bin/sh\nsleep 5\n").unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temp.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let started = std::time::Instant::now();
    let (handle, warning) = start_with_budget(
        executable,
        temp.path().to_path_buf(),
        test_extension_paths(temp.path()),
        CancellationToken::new(),
        Arc::clone(&session),
        Duration::from_millis(50),
    )
    .await;

    assert!(
        started.elapsed() < Duration::from_millis(500),
        "startup blocked for {:?}",
        started.elapsed()
    );
    assert!(
        warning
            .as_deref()
            .is_some_and(|message| message.contains("exceeded 50 ms")),
        "{warning:?}"
    );

    drop(handle);
    session.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn startup_gives_initial_mcp_more_time_than_registry_discovery() {
    use std::os::unix::fs::PermissionsExt;

    const TEST_DISCOVERY_BUDGET: Duration = Duration::from_secs(3);
    const TEST_PROJECTION_BUDGET: Duration = Duration::from_secs(6);

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("slow-initial-mcp");
    let snapshot = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 2,
            "generation": 1,
            "revision": "1111111111111111111111111111111111111111111111111111111111111111",
            "capabilities": [{
                "id": "use/acme/report",
                "route": "report",
                "version": "1.0.0",
                "origin": "extension",
                "enabled": true,
                "packageRoot": temp.path(),
                "surfaces": ["mcp"],
                "mcp": {"target": "acme/report", "transport": "stdio"},
                "skills": []
            }]
        }}
    });
    let unchanged = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"changed": false, "registry": null}
    });
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "mcp" ] && [ "$2" = "serve" ]; then
  while IFS= read -r line; do
    case "$line" in
      *'"method":"initialize"'*)
        sleep 3.25
        printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2024-11-05","capabilities":{{}},"serverInfo":{{"name":"slow-fixture","version":"1.0.0"}}}}}}'
        ;;
      *'"method":"notifications/initialized"'*) ;;
      *'"method":"tools/list"'*)
        printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"fixture_tool","description":"Fixture tool","inputSchema":{{"type":"object"}},"annotations":{{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}}}}]}}}}'
        ;;
    esac
  done
  exit 0
fi

case "$1 $2" in
  "capability snapshot") printf '%s\n' '{}' ;;
  "capability watch") printf '%s\n' '{}' ;;
  *) exit 2 ;;
esac
"#,
        shell_single_quote(&snapshot.to_string()),
        shell_single_quote(&unchanged.to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temp.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let started = std::time::Instant::now();
    let cancellation = CancellationToken::new();
    let discovery = RegistryDiscoveryClient::Fixture(UseRegistryClient::new(
        executable.clone(),
        temp.path().to_path_buf(),
        cancellation.clone(),
    ));
    let (handle, warning) = start_with_budgets(
        RegistryProcess::new(executable, temp.path().to_path_buf()),
        test_extension_paths(temp.path()),
        cancellation,
        Arc::clone(&session),
        discovery,
        ProjectionHost::default(),
        StartupBudgets::new(TEST_DISCOVERY_BUDGET, TEST_PROJECTION_BUDGET),
    )
    .await;

    assert!(warning.is_none(), "{warning:?}");
    assert!(
        started.elapsed() >= TEST_DISCOVERY_BUDGET,
        "fixture did not exercise the longer projection budget"
    );
    assert!(
        session
            .tool_names()
            .iter()
            .any(|name| name == "mcp__use_report__fixture_tool"),
        "initial MCP route was not ready when startup returned"
    );

    handle.shutdown().await;
    session.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn timed_out_startup_discovery_converges_within_the_projection_budget() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("package");
    std::fs::create_dir_all(package.join("skills/fixture-report")).unwrap();
    std::fs::write(
        package.join("skills/fixture-report/SKILL.md"),
        fixture_skill(),
    )
    .unwrap();

    let registry = serde_json::json!({
        "schemaVersion": 2,
        "generation": 1,
        "revision": "1111111111111111111111111111111111111111111111111111111111111111",
        "capabilities": [{
            "id": "use/acme/report",
            "route": "report",
            "version": "1.0.0",
            "origin": "extension",
            "packageRoot": package,
            "enabled": true,
            "surfaces": ["skill"],
            "skills": [{
                "path": package.join("skills/fixture-report/SKILL.md"),
                "sha256": fixture_skill_digest()
            }]
        }]
    });
    let snapshot = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": registry}
    });
    let changed = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"changed": true, "registry": registry}
    });
    let unchanged = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"changed": false}
    });
    let executable = temp.path().join("slow-first-snapshot");
    let script = format!(
        r#"#!/bin/sh
case "$1 $2" in
  "capability snapshot")
    sleep 0.1
    printf '%s\n' '{}'
    ;;
  "capability watch")
    if [ "$4" = "0" ]; then
      printf '%s\n' '{}'
    else
      printf '%s\n' '{}'
    fi
    ;;
  *) exit 2 ;;
esac
"#,
        shell_single_quote(&snapshot.to_string()),
        shell_single_quote(&changed.to_string()),
        shell_single_quote(&unchanged.to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temp.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let cancellation = CancellationToken::new();
    let discovery = RegistryDiscoveryClient::Fixture(UseRegistryClient::new(
        executable.clone(),
        temp.path().to_path_buf(),
        cancellation.clone(),
    ));
    let (handle, warning) = start_with_budgets(
        RegistryProcess::new(executable, temp.path().to_path_buf()),
        test_extension_paths(temp.path()),
        cancellation,
        Arc::clone(&session),
        discovery,
        ProjectionHost::default(),
        StartupBudgets::new(Duration::from_millis(20), Duration::from_secs(2)),
    )
    .await;

    assert!(
        warning
            .as_deref()
            .is_some_and(|message| message.contains("exceeded 20 ms")),
        "{warning:?}"
    );
    let projection = handle.capability_projection("use/acme/report", "fixture-report");
    assert!(
        projection.skill_ready,
        "the projection budget must include atomic Skill publication: {projection:?}"
    );
    assert!(
        !session
            .skill_names()
            .iter()
            .any(|name| name == "fixture-report"),
        "Use-projected Skills must stay out of the compatibility registry"
    );

    drop(handle);
    session.close().await;
}

#[tokio::test]
async fn replacement_session_publishes_live_skills_through_atomic_projection() {
    let temp = tempfile::tempdir().unwrap();
    let first_workspace = temp.path().join("first");
    let second_workspace = temp.path().join("second");
    std::fs::create_dir_all(&first_workspace).unwrap();
    std::fs::create_dir_all(&second_workspace).unwrap();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let first = Arc::new(
        agent
            .session_async(first_workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    let second = Arc::new(
        agent
            .session_async(second_workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    let skill_path = temp.path().join("SKILL.md");
    std::fs::write(&skill_path, fixture_skill()).unwrap();
    let skill = Arc::new(Skill::from_file(&skill_path).unwrap());
    let revision = "2222222222222222222222222222222222222222222222222222222222222222".to_string();
    let capability_snapshot = CapabilitySnapshotAuthority::fixture(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 2,
        revision: revision.clone(),
        capabilities: Vec::new(),
    })
    .unwrap();
    let desired = DesiredCapabilities {
        generation: 2,
        revision,
        capability_snapshot: Some(capability_snapshot),
        packages: BTreeMap::from([("use/acme/report".to_string(), true)]),
        skills: BTreeMap::from([(
            "fixture-report".to_string(),
            DesiredSkill {
                package_id: "use/acme/report".to_string(),
                surface_id: "fixture-report".to_string(),
                fingerprint: "v2".to_string(),
                skill,
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let (desired_tx, _) = watch::channel(Arc::new(desired));
    let extension_paths = test_extension_paths(temp.path());
    let knowledge = UseKnowledgeCarrier::new(desired_tx.clone(), &extension_paths);
    let handle = UseRegistryHandle {
        inner: Arc::new(UseRegistryInner {
            executable: temp.path().join("unused-a3s-use"),
            directory: temp.path().to_path_buf(),
            plugin_management: None,
            runtime_tasks: None,
            mcp_runtime: None,
            mode: ProjectionMode::FullCompatibility,
            desired_tx,
            knowledge,
            cancellation: CancellationToken::new(),
            projections: Mutex::new(BTreeMap::new()),
            registry_task: Mutex::new(None),
        }),
    };
    handle.replace_session(Arc::clone(&first));

    let started = std::time::Instant::now();
    handle.replace_session(Arc::clone(&second));
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(
        handle
            .wait_until_projection_visible(&second, Duration::from_secs(2))
            .await,
        "replacement must publish the complete current generation"
    );
    assert!(
        !second
            .skill_names()
            .iter()
            .any(|name| name == "fixture-report"),
        "projected Skills must not be written into the compatibility registry"
    );
    assert_eq!(second.capability_catalog_stamp().generation().get(), 1);
    assert_eq!(
        handle
            .inner
            .projections
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![PRIMARY_ATTACHMENT.to_string()]
    );
    assert_eq!(
        handle.package_statuses(),
        BTreeMap::from([("use/acme/report".to_string(), true)])
    );
    assert_eq!(
        handle.capability_projection("use/acme/report", "fixture-report"),
        UseCapabilityProjection {
            generation: 2,
            revision: "2222222222222222222222222222222222222222222222222222222222222222"
                .to_string(),
            package_enabled: true,
            mcp_ready: false,
            skill_ready: true,
        }
    );
    handle.shutdown().await;
    first.close().await;
    second.close().await;
}

#[tokio::test]
async fn atomic_flow_preflight_failure_leaves_the_current_generation_unchanged() {
    let temporary = tempfile::tempdir().unwrap();
    let package = temporary.path().join("package");
    let source = package.join("flows/review.ts");
    tokio::fs::create_dir_all(source.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&source, fixture_flow()).await.unwrap();

    let revision = "7".repeat(64);
    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let mut value = fixture_flow_snapshot(&package, &source);
    value["generation"] = serde_json::json!(7);
    value["revision"] = serde_json::json!(revision);
    value["capabilities"][0]["lifecycleGeneration"] = serde_json::json!(7);
    value["capabilities"][0]["plannerEvidence"] = serde_json::json!({
        "packageId": "acme/report",
        "packageSha256": package_digest,
        "manifestSha256": manifest_digest,
    });
    value["capabilities"][0]["flows"][0]["requiresTools"] = serde_json::json!([]);
    value["capabilities"][0]["flows"][0]["requiresMcp"] = serde_json::json!([]);
    value["capabilities"][0]["flows"][0]["requiresOkf"] = serde_json::json!([]);
    let snapshot: RegistrySnapshot = serde_json::from_value(value).unwrap();
    let binding = snapshot.capabilities[0].clone();
    let client = UseRegistryClient::for_test(PathBuf::from("unused"), temporary.path().into());
    let mut desired = DesiredCapabilities {
        generation: snapshot.generation,
        revision: snapshot.revision.clone(),
        capability_snapshot: Some(CapabilitySnapshotAuthority::fixture(&snapshot).unwrap()),
        ..DesiredCapabilities::default()
    };
    client
        .add_projected_capabilities(&mut desired, &binding, None)
        .await
        .unwrap();
    assert_eq!(desired.flows.len(), 1);
    assert_eq!(desired.atomic_flows.len(), 1);

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(
                temporary.path().join("workspace").display().to_string(),
                None,
            )
            .await
            .unwrap(),
    );
    let initial = session.capability_catalog_stamp();
    let runtime = flow_runtime::InstalledFlowRuntime::with_compiler(
        session.workspace(),
        temporary.path().join("missing-flow-compiler"),
    );
    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let (progress_tx, progress_rx) = watch::channel(SessionProjectionProgress::default());

    let error = reconcile_atomic_projection(
        &mut applied,
        &desired,
        None,
        None,
        Some(&runtime),
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .expect_err("native Flow preflight must complete before atomic publication");

    assert!(
        format!("{error:#}").contains("native TypeScript preflight failed"),
        "{error:#}"
    );
    assert_eq!(session.capability_catalog_stamp(), initial);
    assert!(applied.atomic.is_none());
    assert!(progress_rx.borrow().atomic.is_none());
    assert!(session
        .projected_flow("report:review")
        .await
        .unwrap()
        .is_none());
    session.close().await;
}

#[tokio::test]
async fn atomic_flow_resolves_its_runtime_tool_in_the_same_exact_package() {
    let temporary = tempfile::tempdir().unwrap();
    let package = temporary.path().join("package");
    let source = package.join("flows/review.ts");
    tokio::fs::create_dir_all(source.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&source, fixture_flow()).await.unwrap();

    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let mut binding = fixture_runtime_capability(7, &package_digest, &manifest_digest);
    binding.package_root = package.clone();
    binding.surfaces.push("flow".to_string());
    binding.flows.push(ProjectedFlowSurface {
        id: "review".to_string(),
        engine: UseFlowEngine::A3sFlow,
        runtime: UseFlowRuntime::NativeTs,
        source: ProjectedManagedAsset {
            path: source,
            sha256: fixture_flow_digest(),
            media_type: "text/typescript".to_string(),
        },
        export_name: "run".to_string(),
        requires_tools: vec!["convert".to_string()],
        requires_mcp: Vec::new(),
        requires_okf: Vec::new(),
    });
    let revision = "7".repeat(64);
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 7,
        revision: revision.clone(),
        capabilities: vec![binding.clone()],
    };
    let runtime_tasks: Arc<dyn RuntimeTaskInvoker> =
        Arc::new(RecordingRuntimeTaskInvoker::with_provider("test-runtime"));
    let mut desired = DesiredCapabilities {
        generation: 7,
        revision,
        ..DesiredCapabilities::default()
    };
    let client = UseRegistryClient::for_test(PathBuf::from("unused"), temporary.path().into());
    client
        .add_projected_capabilities(&mut desired, &binding, Some(runtime_tasks.as_ref()))
        .await
        .unwrap();
    let authority = CapabilitySnapshotAuthority::fixture(&snapshot).unwrap();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(
                temporary.path().join("workspace").display().to_string(),
                None,
            )
            .await
            .unwrap(),
    );
    let flow_runtime = flow_runtime::InstalledFlowRuntime::with_compiler(
        session.workspace(),
        temporary.path().join("unused-flow-compiler"),
    );
    let batch = capability_batch::capability_batch(
        &session,
        &authority,
        capability_batch::CapabilityBatchInputs {
            mcp: &desired.managed_mcp,
            mcp_runtime: None,
            skills: &desired.skills,
            tool_tasks: &desired.tool_tasks,
            runtime_tasks: Some(&runtime_tasks),
            knowledge_surfaces: &desired.knowledge_surfaces,
            flows: &desired.atomic_flows,
            flow_runtime: Some(&flow_runtime),
            ui: &desired.ui,
        },
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let (_, flow) = batch
        .target()
        .iter()
        .find(|(id, _)| id.kind() == a3s_code_core::capability::CapabilityKind::Flow)
        .expect("atomic Flow descriptor");
    assert_eq!(flow.public_name(), "report:review");
    assert_eq!(flow.dependencies().len(), 1);
    let dependency = &flow.dependencies()[0];
    assert_eq!(
        dependency.kind(),
        a3s_code_core::capability::CapabilityKind::Tool
    );
    assert_eq!(dependency.local_id(), "convert");
    assert_eq!(dependency.source(), flow.id().source());
    assert!(batch.target().contains(dependency));
    session.close().await;
}

#[tokio::test]
async fn atomic_flow_resolves_one_digest_bound_okf_surface_across_exact_scopes() {
    let temporary = tempfile::tempdir().unwrap();
    let package = temporary.path().join("package");
    let source = package.join("flows/review.ts");
    tokio::fs::create_dir_all(source.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&source, fixture_flow()).await.unwrap();

    let paths = test_extension_paths(temporary.path());
    let workspace_projection = staged_fixture_knowledge(&paths).await;
    let mut user_projection = workspace_projection.clone();
    user_projection.scope.kind = a3s_use_core::PlanScopeKind::User;
    user_projection.scope.id = "fixture-user".to_string();

    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let binding = CapabilityBinding {
        id: "use/acme/report".to_string(),
        route: "report".to_string(),
        version: "1.0.0".to_string(),
        origin: CapabilityOrigin::Extension,
        enabled: true,
        readiness: CapabilityReadiness::Ready,
        package_root: package,
        lifecycle_generation: Some(1),
        planner_evidence: Some(ProjectedPluginPlannerEvidence {
            package_id: "acme/report".to_string(),
            package_sha256: package_digest,
            manifest_sha256: manifest_digest,
        }),
        surfaces: vec!["flow".to_string(), "okf".to_string()],
        mcp: None,
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        flows: vec![ProjectedFlowSurface {
            id: "review".to_string(),
            engine: UseFlowEngine::A3sFlow,
            runtime: UseFlowRuntime::NativeTs,
            source: ProjectedManagedAsset {
                path: source,
                sha256: fixture_flow_digest(),
                media_type: "text/typescript".to_string(),
            },
            export_name: "run".to_string(),
            requires_tools: Vec::new(),
            requires_mcp: Vec::new(),
            requires_okf: vec!["domain-knowledge".to_string()],
        }],
        knowledge: vec![user_projection, workspace_projection],
        activity_bar: Vec::new(),
        tool_tasks: Vec::new(),
    };
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 1,
        revision: "1".repeat(64),
        capabilities: vec![binding.clone()],
    };
    let client = UseRegistryClient::for_test(PathBuf::from("unused"), temporary.path().into());
    let mut desired = DesiredCapabilities {
        generation: snapshot.generation,
        revision: snapshot.revision.clone(),
        capability_snapshot: Some(CapabilitySnapshotAuthority::fixture(&snapshot).unwrap()),
        ..DesiredCapabilities::default()
    };
    client
        .add_projected_capabilities(&mut desired, &binding, None)
        .await
        .unwrap();

    assert_eq!(desired.atomic_flows.len(), 1);
    assert_eq!(desired.knowledge_surfaces.len(), 1);
    let knowledge = &desired.knowledge_surfaces["report:domain-knowledge"];
    assert_eq!(knowledge.binding.projection_digests().len(), 2);
    assert!(knowledge
        .binding
        .projection_digests()
        .windows(2)
        .all(|pair| pair[0] < pair[1]));

    let authority = desired.capability_snapshot.as_ref().unwrap().clone();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temporary.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let flow_runtime = flow_runtime::InstalledFlowRuntime::with_compiler(
        session.workspace(),
        temporary.path().join("unused-flow-compiler"),
    );
    let batch = capability_batch::capability_batch(
        &session,
        &authority,
        capability_batch::CapabilityBatchInputs {
            mcp: &desired.managed_mcp,
            mcp_runtime: None,
            skills: &desired.skills,
            tool_tasks: &desired.tool_tasks,
            runtime_tasks: None,
            knowledge_surfaces: &desired.knowledge_surfaces,
            flows: &desired.atomic_flows,
            flow_runtime: Some(&flow_runtime),
            ui: &desired.ui,
        },
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let (_, knowledge_descriptor) = batch
        .target()
        .iter()
        .find(|(id, _)| id.kind() == a3s_code_core::capability::CapabilityKind::KnowledgeSurface)
        .expect("digest-bound Knowledge surface descriptor");
    let (_, flow_descriptor) = batch
        .target()
        .iter()
        .find(|(id, _)| id.kind() == a3s_code_core::capability::CapabilityKind::Flow)
        .expect("OKF-dependent Flow descriptor");
    assert_eq!(
        flow_descriptor.dependencies(),
        [knowledge_descriptor.id().clone()]
    );
    assert_eq!(
        flow_descriptor.id().source(),
        knowledge_descriptor.id().source()
    );
    assert_eq!(batch.len(), 2);

    let mut tampered_surfaces = desired.knowledge_surfaces.clone();
    tampered_surfaces
        .get_mut("report:domain-knowledge")
        .unwrap()
        .package_digest = format!("sha256:{}", "c".repeat(64));
    let error = capability_batch::capability_batch(
        &session,
        &authority,
        capability_batch::CapabilityBatchInputs {
            mcp: &desired.managed_mcp,
            mcp_runtime: None,
            skills: &desired.skills,
            tool_tasks: &desired.tool_tasks,
            runtime_tasks: None,
            knowledge_surfaces: &tampered_surfaces,
            flows: &desired.atomic_flows,
            flow_runtime: Some(&flow_runtime),
            ui: &desired.ui,
        },
        CancellationToken::new(),
    )
    .await
    .expect_err("tampered OKF package evidence must fail exact cursor validation");
    assert!(
        format!("{error:#}").contains("does not match its exact package cursor"),
        "{error:#}"
    );
    session.close().await;
}

#[tokio::test]
async fn missing_required_okf_surface_leaves_the_atomic_generation_unchanged() {
    let temporary = tempfile::tempdir().unwrap();
    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let binding = fixture_runtime_capability(7, &package_digest, &manifest_digest);
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 7,
        revision: "7".repeat(64),
        capabilities: vec![binding],
    };
    let authority = CapabilitySnapshotAuthority::fixture(&snapshot).unwrap();
    let flow = UseFlowCatalogItem {
        package_id: "use/acme/report".to_string(),
        route: "report".to_string(),
        version: "7.0.0".to_string(),
        package_root: temporary.path().to_path_buf(),
        lifecycle_generation: 7,
        key: "report:review".to_string(),
        id: "review".to_string(),
        engine: UseFlowEngine::A3sFlow,
        runtime: UseFlowRuntime::NativeTs,
        source_path: temporary.path().join("flows/review.ts"),
        sha256: fixture_flow_digest(),
        export_name: "run".to_string(),
        media_type: "text/typescript".to_string(),
        requires_tools: Vec::new(),
        requires_mcp: Vec::new(),
        requires_okf: vec!["domain-knowledge".to_string()],
    };
    let desired = DesiredCapabilities {
        generation: 7,
        revision: snapshot.revision.clone(),
        capability_snapshot: Some(authority),
        packages: BTreeMap::from([("use/acme/report".to_string(), true)]),
        atomic_flows: BTreeMap::from([(flow.key.clone(), flow)]),
        ..DesiredCapabilities::default()
    };
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temporary.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let initial = session.capability_catalog_stamp();
    let runtime = flow_runtime::InstalledFlowRuntime::with_compiler(
        session.workspace(),
        temporary.path().join("unused-flow-compiler"),
    );
    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let (progress_tx, progress_rx) = watch::channel(SessionProjectionProgress::default());

    let error = reconcile_atomic_projection(
        &mut applied,
        &desired,
        None,
        None,
        Some(&runtime),
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .expect_err("an unresolved OKF dependency must reject the complete atomic batch");

    assert!(
        format!("{error:#}").contains("depends on missing capability"),
        "{error:#}"
    );
    assert_eq!(session.capability_catalog_stamp(), initial);
    assert!(applied.atomic.is_none());
    assert!(progress_rx.borrow().atomic.is_none());
    session.close().await;
}

#[tokio::test]
async fn atomic_mcp_closes_flow_and_ui_dependencies_in_the_same_exact_package() {
    let temporary = tempfile::tempdir().unwrap();
    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let binding: CapabilityBinding = serde_json::from_value(serde_json::json!({
        "id": "use/acme/report",
        "route": "report",
        "version": "7.0.0",
        "origin": "extension",
        "enabled": true,
        "readiness": "ready",
        "packageRoot": temporary.path(),
        "lifecycleGeneration": 7,
        "plannerEvidence": {
            "packageId": "acme/report",
            "packageSha256": package_digest,
            "manifestSha256": manifest_digest
        },
        "surfaces": ["mcp", "flow", "ui"]
    }))
    .unwrap();
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 7,
        revision: "7".repeat(64),
        capabilities: vec![binding],
    };
    let authority = CapabilitySnapshotAuthority::fixture(&snapshot).unwrap();
    let lifecycle_identity = ProjectedLifecycleIdentity {
        package_id: "acme/report".to_string(),
        package_digest: package_digest.clone(),
        manifest_digest: manifest_digest.clone(),
        generation: 7,
    };
    let server_name = "use_mcp_report_library_0123456789abcdef".to_string();
    let managed_mcp = DesiredManagedMcp {
        capability_id: "use/acme/report".to_string(),
        resolved_executable: Some(temporary.path().join("bin/library")),
        projection: ProjectedMcpServer {
            id: "library".to_string(),
            server_name: server_name.clone(),
            activation: ProjectedMcpActivation::Lazy,
            lifecycle_identity: lifecycle_identity.clone(),
            file_evidence_digest: format!("sha256:{}", "c".repeat(64)),
            launch: ProjectedMcpLaunch::Stdio {
                executable: PathBuf::from("bin/library"),
                args: Vec::new(),
            },
        },
        lifecycle_identity: lifecycle_identity.validated("MCP").unwrap(),
        fingerprint: "mcp-library-generation-7".to_string(),
    };
    let flow = UseFlowCatalogItem {
        package_id: "use/acme/report".to_string(),
        route: "report".to_string(),
        version: "7.0.0".to_string(),
        package_root: temporary.path().to_path_buf(),
        lifecycle_generation: 7,
        key: "report:review".to_string(),
        id: "review".to_string(),
        engine: UseFlowEngine::A3sFlow,
        runtime: UseFlowRuntime::NativeTs,
        source_path: temporary.path().join("flows/review.ts"),
        sha256: fixture_flow_digest(),
        export_name: "run".to_string(),
        media_type: "text/typescript".to_string(),
        requires_tools: Vec::new(),
        requires_mcp: vec!["library".to_string()],
        requires_okf: Vec::new(),
    };
    let desired = DesiredCapabilities {
        generation: 7,
        revision: snapshot.revision.clone(),
        capability_snapshot: Some(authority.clone()),
        packages: BTreeMap::from([("use/acme/report".to_string(), true)]),
        managed_mcp: BTreeMap::from([(server_name.clone(), managed_mcp)]),
        atomic_flows: BTreeMap::from([(flow.key.clone(), flow)]),
        ui: BTreeMap::from([(
            "report:review-panel".to_string(),
            DesiredUi {
                package_id: "use/acme/report".to_string(),
                surface_id: "review-panel".to_string(),
                fingerprint: "ui-mcp-dependent-v1".to_string(),
                dependencies: vec![PluginSurfaceRef {
                    kind: a3s_use_core::PluginSurfaceKind::Mcp,
                    id: "library".to_string(),
                }],
                binding: fixture_ui_binding("report:review-panel", "mcp-dependent"),
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temporary.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let flow_runtime = flow_runtime::InstalledFlowRuntime::with_compiler(
        session.workspace(),
        temporary.path().join("unused-flow-compiler"),
    );

    let batch = capability_batch::capability_batch(
        &session,
        &authority,
        capability_batch::CapabilityBatchInputs {
            mcp: &desired.managed_mcp,
            mcp_runtime: None,
            skills: &desired.skills,
            tool_tasks: &desired.tool_tasks,
            runtime_tasks: None,
            knowledge_surfaces: &desired.knowledge_surfaces,
            flows: &desired.atomic_flows,
            flow_runtime: Some(&flow_runtime),
            ui: &desired.ui,
        },
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let (_, mcp) = batch
        .target()
        .iter()
        .find(|(id, _)| id.kind() == a3s_code_core::capability::CapabilityKind::Mcp)
        .expect("managed MCP descriptor");
    assert_eq!(mcp.id().local_id(), "library");
    assert_eq!(mcp.public_name(), server_name);
    for kind in [
        a3s_code_core::capability::CapabilityKind::Flow,
        a3s_code_core::capability::CapabilityKind::Ui,
    ] {
        let (_, dependent) = batch
            .target()
            .iter()
            .find(|(id, _)| id.kind() == kind)
            .expect("MCP-dependent descriptor");
        assert_eq!(dependent.dependencies().len(), 1);
        assert_eq!(&dependent.dependencies()[0], mcp.id());
        assert_eq!(
            dependent.dependencies()[0].source(),
            dependent.id().source()
        );
    }
    assert_eq!(batch.len(), 3);
    let mut status_binding = snapshot.capabilities[0].clone();
    status_binding
        .mcp_servers
        .push(desired.managed_mcp[&server_name].projection.clone());
    let status = render_capability(
        &status_binding,
        None,
        None,
        &desired,
        &HashMap::new(),
        &BTreeSet::new(),
        &PublishedAtomicCapabilities {
            mcp: &BTreeSet::from([server_name.as_str()]),
            tools: &BTreeSet::new(),
            flows: &BTreeSet::new(),
            ui: &BTreeSet::new(),
        },
    )
    .join("\n");
    assert!(status.contains("MCP verified + atomic (1/1)"), "{status}");
    session.close().await;
}

fn fixture_ui_binding(public_name: &str, marker: &str) -> Arc<UiBinding> {
    let document = UiDocument::new(
        a3s_code_core::capability::UiAsset::new(
            a3s_code_core::capability::UiAssetKind::Html,
            format!("<main>{marker}</main>"),
        )
        .unwrap(),
        [],
        [],
    )
    .unwrap();
    Arc::new(
        UiBinding::new(UiBindingSpec {
            public_name: public_name.to_string(),
            title: "Evidence Review".to_string(),
            description: "Review exact package evidence.".to_string(),
            icon: "panel-top".to_string(),
            order: 80,
            document,
        })
        .unwrap(),
    )
}

#[tokio::test]
async fn projected_runtime_tool_satisfies_ui_dependency_in_the_same_atomic_generation() {
    let temporary = tempfile::tempdir().unwrap();
    let llm = Arc::new(RuntimeToolCallingLlm::default());
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(
                temporary.path().display().to_string(),
                Some(
                    a3s_code_core::SessionOptions::new()
                        .with_llm_client(llm as Arc<dyn a3s_code_core::LlmClient>)
                        .with_confirmation_manager(Arc::new(
                            a3s_code_core::hitl::AutoApproveConfirmation,
                        )),
                ),
            )
            .await
            .unwrap(),
    );
    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let binding = fixture_runtime_capability(7, &package_digest, &manifest_digest);
    let revision = "7".repeat(64);
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 7,
        revision: revision.clone(),
        capabilities: vec![binding.clone()],
    };
    let authority = CapabilitySnapshotAuthority::fixture(&snapshot).unwrap();
    let invoker = Arc::new(RecordingRuntimeTaskInvoker::with_provider("test-runtime"));
    let runtime_tasks = Arc::clone(&invoker) as Arc<dyn RuntimeTaskInvoker>;
    let client = UseRegistryClient::for_test(PathBuf::from("unused"), PathBuf::from("."));
    let mut desired = DesiredCapabilities {
        generation: 7,
        revision,
        capability_snapshot: Some(authority),
        ..DesiredCapabilities::default()
    };
    client
        .add_projected_capabilities(&mut desired, &binding, Some(runtime_tasks.as_ref()))
        .await
        .unwrap();
    desired.ui.insert(
        "report:review".to_string(),
        DesiredUi {
            package_id: "use/acme/report".to_string(),
            surface_id: "review".to_string(),
            fingerprint: "ui-tool-dependent-v1".to_string(),
            dependencies: vec![PluginSurfaceRef {
                kind: a3s_use_core::PluginSurfaceKind::Tool,
                id: "convert".to_string(),
            }],
            binding: fixture_ui_binding("report:review", "tool-dependent"),
        },
    );

    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let (progress_tx, progress_rx) = watch::channel(SessionProjectionProgress::default());
    reconcile_atomic_projection(
        &mut applied,
        &desired,
        None,
        Some(&runtime_tasks),
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();

    let ui = session
        .projected_ui("report:review")
        .await
        .unwrap()
        .expect("Tool-dependent UI must publish with its Tool");
    assert_eq!(ui.use_generation().unwrap().generation(), 7);
    assert_eq!(ui.dependencies().len(), 1);
    assert_eq!(
        ui.dependencies()[0].kind(),
        a3s_code_core::capability::CapabilityKind::Tool
    );
    assert_eq!(ui.dependencies()[0].local_id(), "convert");
    assert!(progress_rx.borrow().tools.contains(FIXTURE_RUNTIME_TOOL));
    let status = render_capability(
        &binding,
        None,
        None,
        &desired,
        &HashMap::new(),
        &BTreeSet::new(),
        &PublishedAtomicCapabilities {
            mcp: &BTreeSet::new(),
            tools: &BTreeSet::from([FIXTURE_RUNTIME_TOOL]),
            flows: &BTreeSet::new(),
            ui: &BTreeSet::from(["report:review"]),
        },
    )
    .join("\n");
    assert!(
        status.contains("Runtime Tool verified + atomic (1/1)"),
        "{status}"
    );
    assert!(!session
        .tool_names()
        .iter()
        .any(|name| name == FIXTURE_RUNTIME_TOOL));
    let outcome = session
        .send("Run the Tool required by this UI once.", None)
        .await
        .unwrap();
    assert_eq!(outcome.tool_calls_count, 1);
    assert_eq!(invoker.generations(), vec![7]);

    ui.close().await.unwrap();
    session.close().await;
}

#[tokio::test]
async fn projected_ui_and_skill_share_exact_use_generations_across_atomic_cutover() {
    let temporary = tempfile::tempdir().unwrap();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temporary.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let skill = Arc::new(Skill {
        name: "guide".to_string(),
        description: "Guide the evidence review.".to_string(),
        allowed_tools: None,
        disable_model_invocation: false,
        kind: a3s_code_core::skills::SkillKind::Instruction,
        content: "Review the exact evidence.".to_string(),
        tags: Vec::new(),
        version: None,
    });
    let dependency = PluginSurfaceRef {
        kind: a3s_use_core::PluginSurfaceKind::Skill,
        id: "guide".to_string(),
    };

    let first_revision = "5".repeat(64);
    let first_authority = CapabilitySnapshotAuthority::fixture(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 5,
        revision: first_revision.clone(),
        capabilities: Vec::new(),
    })
    .unwrap();
    let first = DesiredCapabilities {
        generation: 5,
        revision: first_revision,
        capability_snapshot: Some(first_authority),
        packages: BTreeMap::from([("use/acme/workbench".to_string(), true)]),
        skills: BTreeMap::from([(
            "guide".to_string(),
            DesiredSkill {
                package_id: "use/acme/workbench".to_string(),
                surface_id: "guide".to_string(),
                fingerprint: "skill-v1".to_string(),
                skill: Arc::clone(&skill),
            },
        )]),
        ui: BTreeMap::from([(
            "workbench:review".to_string(),
            DesiredUi {
                package_id: "use/acme/workbench".to_string(),
                surface_id: "review".to_string(),
                fingerprint: "ui-v1".to_string(),
                dependencies: vec![dependency.clone()],
                binding: fixture_ui_binding("workbench:review", "generation-five"),
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let (progress_tx, progress_rx) = watch::channel(SessionProjectionProgress::default());
    reconcile_atomic_projection(
        &mut applied,
        &first,
        None,
        None,
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();
    let first_progress = progress_rx.borrow().clone();
    assert!(atomic_projection_matches_current_catalog(
        &session,
        &first,
        &first_progress
    ));

    let first_handle = session
        .projected_ui("workbench:review")
        .await
        .unwrap()
        .expect("generation five UI must be published");
    assert_eq!(
        first_handle.document().entry().content(),
        "<main>generation-five</main>"
    );
    assert_eq!(first_handle.dependencies().len(), 1);
    assert_eq!(
        first_handle.dependencies()[0].kind(),
        a3s_code_core::capability::CapabilityKind::Skill
    );
    assert_eq!(first_handle.dependencies()[0].local_id(), "guide");
    assert_eq!(first_handle.use_generation().unwrap().generation(), 5);

    let second_revision = "6".repeat(64);
    let second_authority = CapabilitySnapshotAuthority::fixture(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 6,
        revision: second_revision.clone(),
        capabilities: Vec::new(),
    })
    .unwrap();
    let second = DesiredCapabilities {
        generation: 6,
        revision: second_revision,
        capability_snapshot: Some(second_authority),
        packages: first.packages.clone(),
        skills: first.skills.clone(),
        ui: BTreeMap::from([(
            "workbench:review".to_string(),
            DesiredUi {
                package_id: "use/acme/workbench".to_string(),
                surface_id: "review".to_string(),
                fingerprint: "ui-v2".to_string(),
                dependencies: vec![dependency],
                binding: fixture_ui_binding("workbench:review", "generation-six"),
            },
        )]),
        ..DesiredCapabilities::default()
    };
    reconcile_atomic_projection(
        &mut applied,
        &second,
        None,
        None,
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();

    let second_handle = session
        .projected_ui("workbench:review")
        .await
        .unwrap()
        .expect("generation six UI must be published");
    assert_eq!(
        second_handle.document().entry().content(),
        "<main>generation-six</main>"
    );
    assert_eq!(second_handle.use_generation().unwrap().generation(), 6);
    assert_eq!(
        first_handle.document().entry().content(),
        "<main>generation-five</main>",
        "the admitted generation-five handle must retain its exact document"
    );
    assert_eq!(first_handle.use_generation().unwrap().generation(), 5);
    assert!(progress_rx.borrow().ui.contains("workbench:review"));
    assert!(
        !atomic_projection_matches_current_catalog(&session, &first, &first_progress),
        "a receipt from N must not describe the current catalog after N+1"
    );
    let second_progress = progress_rx.borrow().clone();
    assert!(atomic_projection_matches_current_catalog(
        &session,
        &second,
        &second_progress
    ));

    second_handle.close().await.unwrap();
    first_handle.close().await.unwrap();
    session.close().await;
}

#[tokio::test]
async fn projected_ui_with_a_missing_dependency_cannot_advance_the_catalog() {
    let temporary = tempfile::tempdir().unwrap();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temporary.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let revision = "7".repeat(64);
    let authority = CapabilitySnapshotAuthority::fixture(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 7,
        revision: revision.clone(),
        capabilities: Vec::new(),
    })
    .unwrap();
    let desired = DesiredCapabilities {
        generation: 7,
        revision,
        capability_snapshot: Some(authority),
        ui: BTreeMap::from([(
            "workbench:review".to_string(),
            DesiredUi {
                package_id: "use/acme/workbench".to_string(),
                surface_id: "review".to_string(),
                fingerprint: "ui-v1".to_string(),
                dependencies: vec![PluginSurfaceRef {
                    kind: a3s_use_core::PluginSurfaceKind::Skill,
                    id: "missing-guide".to_string(),
                }],
                binding: fixture_ui_binding("workbench:review", "unpublished"),
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let (progress_tx, _) = watch::channel(SessionProjectionProgress::default());
    let error = reconcile_atomic_projection(
        &mut applied,
        &desired,
        None,
        None,
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .expect_err("an incomplete UI dependency graph must fail before publication");
    let diagnostic = format!("{error:#}");
    assert!(
        diagnostic.contains("depends on missing capability"),
        "{diagnostic}"
    );
    assert_eq!(session.capability_catalog_stamp().generation().get(), 0);
    assert!(applied.atomic.is_none());

    session.close().await;
}

#[tokio::test]
async fn host_skill_change_republishes_without_a_use_cursor_change() {
    let temporary = tempfile::tempdir().unwrap();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temporary.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let revision = "4".repeat(64);
    let authority = CapabilitySnapshotAuthority::fixture(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 4,
        revision: revision.clone(),
        capabilities: Vec::new(),
    })
    .unwrap();
    let first_skill = Arc::new(Skill {
        name: "a3s-use-ocr".to_string(),
        description: "host-overlay-generation-one".to_string(),
        allowed_tools: None,
        disable_model_invocation: false,
        kind: a3s_code_core::skills::SkillKind::Instruction,
        content: "host-overlay-generation-one".to_string(),
        tags: Vec::new(),
        version: None,
    });
    let second_skill = Arc::new(Skill {
        description: "host-overlay-generation-two".to_string(),
        content: "host-overlay-generation-two".to_string(),
        ..first_skill.as_ref().clone()
    });
    let first = DesiredCapabilities {
        generation: 4,
        revision: revision.clone(),
        capability_snapshot: Some(authority.clone()),
        packages: BTreeMap::from([("use/ocr".to_string(), true)]),
        skills: BTreeMap::from([(
            "a3s-use-ocr".to_string(),
            DesiredSkill {
                package_id: "use/ocr".to_string(),
                surface_id: "a3s-use-ocr".to_string(),
                fingerprint: "host-overlay-one".to_string(),
                skill: first_skill,
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let second = DesiredCapabilities {
        generation: 4,
        revision,
        capability_snapshot: Some(authority),
        packages: BTreeMap::from([("use/ocr".to_string(), true)]),
        skills: BTreeMap::from([(
            "a3s-use-ocr".to_string(),
            DesiredSkill {
                package_id: "use/ocr".to_string(),
                surface_id: "a3s-use-ocr".to_string(),
                fingerprint: "host-overlay-two".to_string(),
                skill: second_skill,
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let (progress_tx, progress_rx) = watch::channel(SessionProjectionProgress::default());

    reconcile_atomic_projection(
        &mut applied,
        &first,
        None,
        None,
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();
    let first_identity = applied.atomic.as_ref().unwrap().identity.clone();
    assert_eq!(session.capability_catalog_stamp().generation().get(), 1);

    reconcile_atomic_projection(
        &mut applied,
        &second,
        None,
        None,
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();
    let current = applied.atomic.as_ref().unwrap();
    assert_ne!(current.identity, first_identity);
    assert_eq!(current.identity.snapshot, first_identity.snapshot);
    assert_eq!(
        current.skills["a3s-use-ocr"].fingerprint,
        "host-overlay-two"
    );
    assert_eq!(session.capability_catalog_stamp().generation().get(), 2);
    assert_eq!(
        progress_rx
            .borrow()
            .atomic
            .as_ref()
            .map(|receipt| &receipt.identity),
        Some(&current.identity)
    );
    assert!(!session
        .skill_names()
        .iter()
        .any(|name| name == "a3s-use-ocr"));

    session.close().await;
}

#[test]
fn active_run_pins_native_use_skill_generation_across_atomic_cutover() {
    std::thread::Builder::new()
        .name("cli-use-skill-cutover".to_owned())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("atomic Skill cutover runtime")
                .block_on(active_run_native_use_skill_cutover_scenario());
        })
        .expect("atomic Skill cutover test thread")
        .join()
        .expect("atomic Skill cutover test thread panicked");
}

async fn active_run_native_use_skill_cutover_scenario() {
    let temporary = tempfile::tempdir().unwrap();
    let extension_registry =
        a3s_use_extension::ExtensionRegistry::new(a3s_use_extension::ExtensionPaths::new(
            temporary.path().join("use-data"),
            temporary.path().join("use-state"),
        ));
    let source = temporary.path().join("source");
    let (first_package, first_identity) =
        prepare_atomic_skill_package(&source, "1.0.0", "cli-use-generation-one", 31).await;
    extension_registry
        .commit_lifecycle_package(&first_identity, &first_package)
        .await
        .unwrap();
    extension_registry
        .publish_lifecycle_package(&first_identity)
        .await
        .unwrap();

    let registry = Arc::new(CapabilityRegistry::new(extension_registry.clone()));
    let first_snapshot = registry.snapshot().await.unwrap();
    let first_cursor = first_snapshot.cursor().clone();
    let first_desired =
        desired_atomic_skill_generation(Arc::clone(&registry), first_snapshot).await;
    assert!(first_desired.mcp.is_empty());
    assert!(first_desired.knowledge.is_empty());
    assert!(first_desired.tool_tasks.is_empty());

    let client = Arc::new(AtomicSkillCutoverLlm::new());
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(
                temporary.path().join("workspace").display().to_string(),
                Some(
                    a3s_code_core::SessionOptions::new()
                        .with_llm_client(Arc::clone(&client) as Arc<dyn a3s_code_core::LlmClient>)
                        .with_confirmation_manager(Arc::new(
                            a3s_code_core::hitl::AutoApproveConfirmation,
                        )),
                ),
            )
            .await
            .unwrap(),
    );
    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let (progress_tx, progress_rx) = watch::channel(SessionProjectionProgress::default());
    reconcile_atomic_projection(
        &mut applied,
        &first_desired,
        None,
        None,
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();
    assert_eq!(session.capability_catalog_stamp().generation().get(), 1);
    assert!(progress_rx.borrow().skills.contains("fixture-report"));
    assert!(!session
        .skill_names()
        .iter()
        .any(|name| name == "fixture-report"));

    let old_run = tokio::spawn({
        let session = Arc::clone(&session);
        async move {
            session
                .send("Search the exact projected fixture-report Skill.", None)
                .await
        }
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        client.old_surface_observed.acquire(),
    )
    .await
    .expect("generation N Run did not reach its pinned Skill surface")
    .expect("generation N observation semaphore closed")
    .forget();

    let (second_package, second_identity) =
        prepare_atomic_skill_package(&source, "2.0.0", "cli-use-generation-two", 32).await;
    extension_registry
        .commit_lifecycle_package(&second_identity, &second_package)
        .await
        .unwrap();
    extension_registry
        .publish_lifecycle_package(&second_identity)
        .await
        .unwrap();
    let second_snapshot = registry.snapshot().await.unwrap();
    assert_ne!(second_snapshot.cursor(), &first_cursor);
    let second_desired =
        desired_atomic_skill_generation(Arc::clone(&registry), second_snapshot).await;
    reconcile_atomic_projection(
        &mut applied,
        &second_desired,
        None,
        None,
        None,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .unwrap();
    assert_eq!(session.capability_catalog_stamp().generation().get(), 2);
    assert_eq!(
        progress_rx
            .borrow()
            .atomic
            .as_ref()
            .expect("N+1 atomic projection identity")
            .identity
            .snapshot
            .generation,
        second_desired.generation
    );
    assert!(!session
        .skill_names()
        .iter()
        .any(|name| name == "fixture-report"));

    let stale_provider = first_desired.capability_snapshot.as_ref().unwrap();
    let stale_error = match a3s_code_core::capability::UseGenerationLeaseProvider::acquire(
        stale_provider,
        CancellationToken::new(),
    )
    .await
    {
        Ok(_) => panic!("the retired N provider unexpectedly admitted another Run"),
        Err(error) => error,
    };
    assert!(stale_error
        .message()
        .contains("became stale, hidden, or unleasable"));
    extension_registry
        .retire_hidden_lifecycle_package(&first_identity)
        .await
        .unwrap();
    let blocked = extension_registry
        .drain_lifecycle_package(&first_identity, Duration::from_millis(10))
        .await
        .expect_err("the active N Run must retain its real Use generation lease");
    assert_eq!(blocked.code, "use.extension.drain_timeout");

    client.release_old_call.add_permits(1);
    let old_result = old_run.await.unwrap().unwrap();
    assert_eq!(old_result.text, "skill generation one complete");
    extension_registry
        .drain_lifecycle_package(&first_identity, Duration::from_secs(1))
        .await
        .expect("N must drain after its admitted Run completes");

    let new_result = session
        .send("Search the newly projected fixture-report Skill.", None)
        .await
        .unwrap();
    assert_eq!(new_result.text, "skill generation two complete");
    assert_eq!(
        &*client.observed_versions.lock().unwrap(),
        &["generation-one", "generation-two"]
    );

    session.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn compatibility_mcp_failure_does_not_roll_back_the_atomic_skill_generation() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("rejecting-mcp");
    std::fs::write(
        &executable,
        r#"#!/bin/sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"fixture failure"}}'
"#,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();
    let skill_path = temp.path().join("SKILL.md");
    std::fs::write(&skill_path, fixture_skill()).unwrap();
    let skill = Arc::new(Skill::from_file(&skill_path).unwrap());
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temp.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let mut applied = SessionProjectionState::new(Arc::clone(&session));
    let revision = "9999999999999999999999999999999999999999999999999999999999999999".to_string();
    let capability_snapshot = CapabilitySnapshotAuthority::fixture(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 9,
        revision: revision.clone(),
        capabilities: Vec::new(),
    })
    .unwrap();
    let desired = DesiredCapabilities {
        generation: 9,
        revision,
        capability_snapshot: Some(capability_snapshot),
        management_expected: false,
        management_available: false,
        packages: BTreeMap::new(),
        mcp: BTreeMap::from([(
            "use_broken".to_string(),
            DesiredMcp {
                server_name: "use_broken".to_string(),
                capability_id: "use/acme/broken".to_string(),
                target: "acme/broken".to_string(),
                fingerprint: "mcp-v1".to_string(),
            },
        )]),
        skills: BTreeMap::from([(
            "fixture-report".to_string(),
            DesiredSkill {
                package_id: "acme/broken".to_string(),
                surface_id: "fixture-report".to_string(),
                fingerprint: "skill-v1".to_string(),
                skill,
            },
        )]),
        ui: BTreeMap::new(),
        flows: BTreeMap::new(),
        atomic_flows: BTreeMap::new(),
        knowledge: Vec::new(),
        tool_tasks: BTreeMap::new(),
        warnings: Vec::new(),
    };

    let (desired_tx, _) = watch::channel(Arc::new(desired.clone()));
    let knowledge = UseKnowledgeCarrier::new(desired_tx, &test_extension_paths(temp.path()));
    let (progress_tx, progress_rx) = watch::channel(SessionProjectionProgress::default());
    let error = reconcile(
        &executable,
        &ProjectionHost::default(),
        &knowledge,
        &mut applied,
        &desired,
        CancellationToken::new(),
        &progress_tx,
    )
    .await
    .expect_err("a server that rejects initialization cannot become an MCP server");
    assert!(error.to_string().contains("failed to attach"), "{error:#}");
    let atomic = applied.atomic.as_ref().unwrap();
    assert_eq!(atomic.generation, 9);
    assert_eq!(atomic.skills["fixture-report"].fingerprint, "skill-v1");
    assert_eq!(session.capability_catalog_stamp().generation().get(), 1);
    assert!(progress_rx.borrow().skills.contains("fixture-report"));
    assert!(applied.mcp.is_empty());
    assert!(!session
        .skill_names()
        .iter()
        .any(|name| name == "fixture-report"));

    session.close().await;
}

#[test]
fn retry_delay_is_bounded() {
    assert_eq!(next_retry_delay(Duration::from_secs(20)), MAX_RETRY_DELAY);
    assert_eq!(next_retry_delay(MAX_RETRY_DELAY), MAX_RETRY_DELAY);
}

#[test]
fn response_envelope_requires_the_supported_schema() {
    validate_envelope_schema(&serde_json::json!({"schemaVersion": 1})).unwrap();

    let future = validate_envelope_schema(&serde_json::json!({"schemaVersion": 2}))
        .unwrap_err()
        .to_string();
    assert!(future.contains("schema version 2"), "{future}");

    let missing = validate_envelope_schema(&serde_json::json!({}))
        .unwrap_err()
        .to_string();
    assert!(missing.contains("schema version missing"), "{missing}");
}

#[test]
fn capability_snapshot_rejects_an_invalid_skill_digest() {
    let temp = tempfile::tempdir().unwrap();
    let snapshot: RegistrySnapshot = serde_json::from_value(serde_json::json!({
        "schemaVersion": 2,
        "generation": 1,
        "revision": "1111111111111111111111111111111111111111111111111111111111111111",
        "capabilities": [{
            "id": "use/acme/report",
            "route": "report",
            "version": "1.0.0",
            "origin": "extension",
            "packageRoot": temp.path(),
            "enabled": true,
            "surfaces": ["skill"],
            "skills": [{
                "path": temp.path().join("SKILL.md"),
                "sha256": "not-a-sha256"
            }]
        }]
    }))
    .unwrap();

    let error = validate_snapshot(&snapshot)
        .expect_err("Skill content identities must be lowercase SHA-256 digests");
    assert!(error.to_string().contains("Skill digest"), "{error:#}");
}

#[tokio::test]
async fn capability_snapshot_accepts_one_exact_okf_generation_and_rejects_ambiguity() {
    let temp = tempfile::tempdir().unwrap();
    let paths = test_extension_paths(temp.path());
    let projection = staged_fixture_knowledge(&paths).await;
    let binding = CapabilityBinding {
        id: "use/acme/report".to_string(),
        route: "report".to_string(),
        version: "1.0.0".to_string(),
        origin: CapabilityOrigin::Extension,
        enabled: true,
        readiness: CapabilityReadiness::Ready,
        package_root: PathBuf::new(),
        lifecycle_generation: Some(1),
        planner_evidence: None,
        surfaces: vec!["okf".to_string()],
        mcp: None,
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        flows: Vec::new(),
        knowledge: vec![projection.clone()],
        activity_bar: Vec::new(),
        tool_tasks: Vec::new(),
    };
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 1,
        revision: "1".repeat(64),
        capabilities: vec![binding.clone()],
    };
    validate_snapshot(&snapshot).expect("one exact promoted OKF generation must be accepted");

    let mut missing_surface = snapshot.clone();
    missing_surface.capabilities[0].surfaces.clear();
    let error = validate_snapshot(&missing_surface)
        .expect_err("OKF evidence without the declared surface must fail closed");
    assert!(
        error.to_string().contains("without declaring the surface"),
        "{error:#}"
    );

    let mut invalid_evidence = snapshot;
    invalid_evidence.capabilities[0].knowledge[0].schema = "forged.v1".to_string();
    let error = validate_snapshot(&invalid_evidence)
        .expect_err("invalid promoted OKF evidence must fail closed");
    assert!(
        error.to_string().contains("invalid OKF Knowledge evidence"),
        "{error:#}"
    );

    let mut second_generation = projection;
    second_generation.generation = 2;
    let mut ambiguous = binding;
    ambiguous.knowledge.push(second_generation);
    let client = UseRegistryClient::for_test(
        temp.path().join("unused-a3s-use"),
        temp.path().to_path_buf(),
    );
    let mut desired = DesiredCapabilities::default();
    let error = client
        .add_projected_capabilities(&mut desired, &ambiguous, None)
        .await
        .expect_err("one surface cannot expose two active OKF generations");
    assert!(
        error
            .to_string()
            .contains("multiple active generations for OKF Knowledge"),
        "{error:#}"
    );
}

#[test]
fn capability_snapshot_accepts_only_exact_managed_mcp_identity_and_runtime_evidence() {
    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let binding = fixture_mcp_capability(7, &package_digest, &manifest_digest);
    let snapshot = RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 7,
        revision: "7".repeat(64),
        capabilities: vec![binding.clone()],
    };
    validate_snapshot(&snapshot).expect("exact stdio MCP evidence must be accepted");

    let mut mismatched_generation = snapshot.clone();
    mismatched_generation.capabilities[0].mcp_servers[0]
        .lifecycle_identity
        .generation = 8;
    let error = validate_snapshot(&mismatched_generation)
        .expect_err("MCP lifecycle evidence from another generation must fail closed");
    assert!(
        error.to_string().contains("different package generation"),
        "{error:#}"
    );

    let mut duplicate_surface = snapshot.clone();
    duplicate_surface.capabilities[0]
        .mcp_servers
        .push(binding.mcp_servers[0].clone());
    let error = validate_snapshot(&duplicate_surface)
        .expect_err("duplicate MCP surface identities must fail closed");
    assert!(
        error.to_string().contains("duplicate MCP surface ID"),
        "{error:#}"
    );

    let mut invalid_http = snapshot;
    invalid_http.capabilities[0].mcp_servers[0].activation = ProjectedMcpActivation::Eager;
    invalid_http.capabilities[0].mcp_servers[0].launch = ProjectedMcpLaunch::StreamableHttp {
        release: PathBuf::from("releases/library.json"),
        runtime: {
            let mut runtime =
                managed_mcp::runtime_evidence("gateway:managed-services/report-library-7", "/mcp");
            runtime.endpoint_path = "/../mcp".to_string();
            runtime
        },
    };
    let error = validate_snapshot(&invalid_http)
        .expect_err("unsafe MCP Runtime endpoint paths must fail closed");
    assert!(
        error
            .to_string()
            .contains("invalid MCP Runtime readiness evidence"),
        "{error:#}"
    );
}

#[test]
fn capability_snapshot_rejects_managed_mcp_server_name_collisions_across_packages() {
    let package_digest = format!("sha256:{}", "a".repeat(64));
    let manifest_digest = format!("sha256:{}", "b".repeat(64));
    let first = fixture_mcp_capability(7, &package_digest, &manifest_digest);
    let mut second = first.clone();
    second.id = "use/acme/other".to_string();
    second.route = "other".to_string();
    second.planner_evidence.as_mut().unwrap().package_id = "acme/other".to_string();
    second.mcp_servers[0].id = "catalog".to_string();
    second.mcp_servers[0].lifecycle_identity.package_id = "acme/other".to_string();

    let error = validate_snapshot(&RegistrySnapshot {
        schema_version: SCHEMA_VERSION,
        generation: 7,
        revision: "7".repeat(64),
        capabilities: vec![first, second],
    })
    .expect_err("MCP host names must be globally unique across one registry snapshot");
    assert!(
        error.to_string().contains("same MCP server name"),
        "{error:#}"
    );
}

#[test]
fn capability_snapshot_accepts_only_exact_ready_a3s_flow_contracts() {
    let package = tempfile::tempdir().unwrap();
    let source = package.path().join("flows/review.ts");
    let base = fixture_flow_snapshot(package.path(), &source);
    let valid: RegistrySnapshot = serde_json::from_value(base.clone()).unwrap();
    validate_snapshot(&valid).expect("the exact a3s-flow/native-ts contract must be accepted");

    for (pointer, replacement, expected) in [
        (
            "/capabilities/0/enabled",
            serde_json::json!(false),
            "ready enabled extension generation",
        ),
        (
            "/capabilities/0/lifecycleGeneration",
            serde_json::json!(0),
            "exact lifecycle generation",
        ),
        (
            "/capabilities/0/surfaces",
            serde_json::json!([]),
            "without declaring the surface",
        ),
        (
            "/capabilities/0/flows/0/source/path",
            serde_json::json!("flows/review.ts"),
            "source evidence",
        ),
        (
            "/capabilities/0/flows/0/source/sha256",
            serde_json::json!("A".repeat(64)),
            "source evidence",
        ),
        (
            "/capabilities/0/flows/0/source/mediaType",
            serde_json::json!("application/javascript"),
            "source evidence",
        ),
        (
            "/capabilities/0/flows/0/exportName",
            serde_json::json!("run-review"),
            "invalid A3S Flow export",
        ),
        (
            "/capabilities/0/flows/0/requiresTools",
            serde_json::json!(["convert", "convert"]),
            "duplicate Tool dependency",
        ),
        (
            "/capabilities/0/flows/0/requiresMcp",
            serde_json::json!(["invalid_name"]),
            "invalid or duplicate MCP dependency",
        ),
        (
            "/capabilities/0/flows/0/requiresOkf",
            serde_json::json!(["DomainKnowledge"]),
            "invalid or duplicate OKF dependency",
        ),
    ] {
        let mut value = base.clone();
        *value.pointer_mut(pointer).expect("fixture pointer") = replacement;
        let snapshot: RegistrySnapshot = serde_json::from_value(value).unwrap();
        let error = validate_snapshot(&snapshot).expect_err(expected);
        assert!(error.to_string().contains(expected), "{error:#}");
    }

    for (pointer, replacement) in [
        (
            "/capabilities/0/flows/0/engine",
            serde_json::json!("other-flow"),
        ),
        (
            "/capabilities/0/flows/0/runtime",
            serde_json::json!("javascript"),
        ),
    ] {
        let mut value = base.clone();
        *value.pointer_mut(pointer).expect("fixture pointer") = replacement;
        serde_json::from_value::<RegistrySnapshot>(value)
            .expect_err("unsupported Flow engines and runtimes must not deserialize");
    }
}

#[test]
fn flow_design_resolves_one_exact_installed_flow_generation() {
    let catalog = fixture_flow_catalog();
    let public_catalog = serde_json::to_value(&catalog).unwrap();
    assert!(public_catalog["items"][0].get("sourcePath").is_none());
    let parsed = flow::parse_flow_design(&fixture_bound_flow_design(
        "1.0.0",
        9,
        &fixture_flow_digest(),
    ))
    .expect("the typed flow design should parse");

    let resolved = catalog
        .resolve_design(&parsed)
        .expect("the exact installed Flow should resolve");

    assert_eq!(resolved.schema, "a3s.use.resolved-flow.v1");
    assert_eq!(resolved.catalog_generation, 4);
    assert_eq!(resolved.catalog_revision, "4".repeat(64));
    assert_eq!(resolved.key, "report:review");
    assert_eq!(resolved.package_id, "use/acme/report");
    assert_eq!(resolved.flow_id, "review");
    assert_eq!(resolved.version, "1.0.0");
    assert_eq!(resolved.lifecycle_generation, 9);
    assert_eq!(resolved.source_sha256, fixture_flow_digest());
    assert_eq!(resolved.export_name, "run");
    let resolved_json = resolved.to_json();
    assert_eq!(resolved_json["engine"], "a3s-flow");
    assert_eq!(resolved_json["runtime"], "native-ts");
    assert!(resolved_json.get("sourcePath").is_none());

    let mut unrelated_change = catalog;
    unrelated_change.generation = 99;
    unrelated_change.revision = "9".repeat(64);
    let resolved = unrelated_change
        .resolve_design(&parsed)
        .expect("unrelated package changes must not invalidate the installed Flow reference");
    assert_eq!(resolved.catalog_generation, 99);
    assert_eq!(resolved.catalog_revision, "9".repeat(64));
}

#[test]
fn flow_design_fails_closed_after_upgrade_uninstall_or_ambiguous_projection() {
    let old_design = flow::parse_flow_design(&fixture_bound_flow_design(
        "1.0.0",
        9,
        &fixture_flow_digest(),
    ))
    .unwrap();
    let mut upgraded = fixture_flow_catalog();
    upgraded.generation = 5;
    upgraded.revision = "5".repeat(64);
    upgraded.items[0].version = "2.0.0".to_string();
    upgraded.items[0].lifecycle_generation = 10;
    upgraded.items[0].sha256 = "5".repeat(64);

    let error = upgraded
        .resolve_design(&old_design)
        .expect_err("an old design must not bind to an upgraded generation");
    assert!(error.to_string().contains("version"), "{error:#}");

    let new_design =
        flow::parse_flow_design(&fixture_bound_flow_design("2.0.0", 10, &"5".repeat(64))).unwrap();
    upgraded
        .resolve_design(&new_design)
        .expect("the explicitly rebound generation should resolve");

    let mut uninstalled = upgraded.clone();
    uninstalled.items.clear();
    let error = uninstalled
        .resolve_design(&new_design)
        .expect_err("an uninstalled or disabled Flow must be withdrawn");
    assert!(
        error.to_string().contains("not installed or ready"),
        "{error:#}"
    );

    let mut ambiguous = upgraded;
    let mut duplicate = ambiguous.items[0].clone();
    duplicate.key = "report-copy:review".to_string();
    duplicate.route = "report-copy".to_string();
    ambiguous.items.push(duplicate);
    let error = ambiguous
        .resolve_design(&new_design)
        .expect_err("duplicate installed identities must fail closed");
    assert!(error.to_string().contains("ambiguous"), "{error:#}");
}

#[test]
fn flow_design_rejects_mismatched_or_path_bearing_installed_identity() {
    let catalog = fixture_flow_catalog();
    for (version, generation, digest, expected) in [
        ("2.0.0", 9, fixture_flow_digest(), "version"),
        ("1.0.0", 10, fixture_flow_digest(), "lifecycle generation"),
        ("1.0.0", 9, "0".repeat(64), "source digest"),
    ] {
        let design =
            flow::parse_flow_design(&fixture_bound_flow_design(version, generation, &digest))
                .unwrap();
        let error = catalog.resolve_design(&design).expect_err(expected);
        assert!(error.to_string().contains(expected), "{error:#}");
    }

    let clean: serde_json::Value = serde_json::from_str(&fixture_bound_flow_design(
        "1.0.0",
        9,
        &fixture_flow_digest(),
    ))
    .unwrap();
    let mut injected = clean.clone();
    injected["installedFlow"]["sourcePath"] = serde_json::json!("../../forged.ts");
    let error = flow::parse_flow_design(&injected.to_string())
        .expect_err("flow.json must not carry or forge managed source paths");
    assert!(error.to_string().contains("sourcePath"), "{error:#}");

    let duplicate = format!(
        r#"{{"version":"a3s.workflow.design.v1","name":"duplicate","installedFlow":{},"installedFlow":{},"nodes":[],"edges":[]}}"#,
        serde_json::to_string(&clean["installedFlow"]).unwrap(),
        serde_json::to_string(&clean["installedFlow"]).unwrap(),
    );
    let error = flow::parse_flow_design(&duplicate)
        .expect_err("duplicate installedFlow keys must not use last-value-wins semantics");
    assert!(error.to_string().contains("duplicate field"), "{error:#}");
}

#[test]
fn flow_design_rejects_noncanonical_identity_and_bounded_envelope_violations() {
    let base: serde_json::Value = serde_json::from_str(&fixture_bound_flow_design(
        "1.0.0",
        9,
        &fixture_flow_digest(),
    ))
    .unwrap();
    for (pointer, replacement, expected) in [
        (
            "/installedFlow/schema",
            serde_json::json!("a3s.use.installed-flow.v2"),
            "installedFlow schema",
        ),
        (
            "/installedFlow/packageId",
            serde_json::json!("use/Acme/report"),
            "installedFlow packageId",
        ),
        (
            "/installedFlow/flowId",
            serde_json::json!("Review"),
            "installedFlow flowId",
        ),
        (
            "/installedFlow/version",
            serde_json::json!("1.0"),
            "canonical SemVer",
        ),
        (
            "/installedFlow/lifecycleGeneration",
            serde_json::json!(0),
            "lifecycleGeneration",
        ),
        (
            "/installedFlow/sourceSha256",
            serde_json::json!("A".repeat(64)),
            "lowercase SHA-256",
        ),
        (
            "/version",
            serde_json::json!("a3s.workflow.design.v2"),
            "workflow design version",
        ),
        ("/name", serde_json::json!("   "), "workflow design name"),
    ] {
        let mut design = base.clone();
        *design.pointer_mut(pointer).expect("fixture pointer") = replacement;
        let error = flow::parse_flow_design(&design.to_string()).expect_err(expected);
        assert!(error.to_string().contains(expected), "{error:#}");
    }

    let mut long_name = base.clone();
    long_name["name"] = serde_json::json!("n".repeat(257));
    let error = flow::parse_flow_design(&long_name.to_string()).expect_err("bounded name");
    assert!(
        error.to_string().contains("workflow design name"),
        "{error:#}"
    );

    let mut long_description = base.clone();
    long_description["description"] = serde_json::json!("d".repeat(4097));
    let error =
        flow::parse_flow_design(&long_description.to_string()).expect_err("bounded description");
    assert!(
        error.to_string().contains("description exceeds"),
        "{error:#}"
    );

    let mut large_graph = base.clone();
    large_graph["nodes"] = serde_json::Value::Array(vec![serde_json::Value::Null; 10_001]);
    let error = flow::parse_flow_design(&large_graph.to_string()).expect_err("bounded graph");
    assert!(error.to_string().contains("graph exceeds"), "{error:#}");

    let mut extensions = base;
    let object = extensions.as_object_mut().unwrap();
    for index in 0..257 {
        object.insert(format!("extension{index}"), serde_json::Value::Null);
    }
    let error = flow::parse_flow_design(&extensions.to_string()).expect_err("bounded extensions");
    assert!(
        error.to_string().contains("top-level extension"),
        "{error:#}"
    );

    let oversized = "x".repeat(4 * 1024 * 1024 + 1);
    let error = flow::parse_flow_design(&oversized).expect_err("bounded design bytes");
    assert!(error.to_string().contains("4194304"), "{error:#}");
}

#[tokio::test]
async fn managed_flow_source_rejects_digest_substitution_and_package_escape() {
    let package = tempfile::tempdir().unwrap();
    let source = package.path().join("flows/review.ts");
    tokio::fs::create_dir_all(source.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&source, fixture_flow()).await.unwrap();
    let value = fixture_flow_snapshot(package.path(), &source);
    let mut flow: ProjectedFlowSurface =
        serde_json::from_value(value["capabilities"][0]["flows"][0].clone()).unwrap();

    flow.source.sha256 = "0".repeat(64);
    let error = flow::verify_managed_source(package.path(), &flow)
        .await
        .expect_err("source substitution must fail closed");
    assert!(
        error.to_string().contains("digest does not match"),
        "{error:#}"
    );

    let outside = tempfile::tempdir().unwrap();
    let outside_source = outside.path().join("review.ts");
    tokio::fs::write(&outside_source, fixture_flow())
        .await
        .unwrap();
    flow.source.path = outside_source.clone();
    flow.source.sha256 = fixture_flow_digest();
    let error = flow::verify_managed_source(package.path(), &flow)
        .await
        .expect_err("a source outside its immutable package must fail closed");
    assert!(
        error.to_string().contains("escapes its managed package"),
        "{error:#}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let linked_source = package.path().join("flows/linked-review.ts");
        symlink(&outside_source, &linked_source).unwrap();
        flow.source.path = linked_source;
        let error = flow::verify_managed_source(package.path(), &flow)
            .await
            .expect_err("a symlink must never forge managed Flow source identity");
        assert!(
            error.to_string().contains("bounded regular package file"),
            "{error:#}"
        );
    }
}

#[tokio::test]
async fn managed_skill_rejects_content_that_does_not_match_its_digest() {
    let package = tempfile::tempdir().unwrap();
    let path = package.path().join("SKILL.md");
    tokio::fs::write(&path, fixture_skill()).await.unwrap();

    let wrong_digest = "0".repeat(64);
    let error = load_managed_skill(package.path(), &path, Some(&wrong_digest))
        .await
        .expect_err("the registry digest must bind the exact Skill bytes");
    assert!(
        error.to_string().contains("digest does not match"),
        "{error:#}"
    );
}

#[test]
fn skill_content_fingerprint_changes_without_restarting_its_mcp_surface() {
    let package = tempfile::tempdir().unwrap();
    let mcp = ProjectedMcpSurface {
        target: "acme/report".to_string(),
        transport: ProjectedMcpTransport::Stdio,
    };
    let mut skill = ProjectedSkillSurface {
        id: "report".to_string(),
        path: package.path().join("SKILL.md"),
        sha256: "1".repeat(64),
    };
    let binding = CapabilityBinding {
        id: "use/acme/report".to_string(),
        route: "report".to_string(),
        version: "1.0.0".to_string(),
        origin: CapabilityOrigin::Extension,
        enabled: true,
        readiness: CapabilityReadiness::Ready,
        package_root: package.path().to_path_buf(),
        lifecycle_generation: None,
        planner_evidence: None,
        surfaces: vec!["mcp".to_string(), "skill".to_string()],
        mcp: Some(mcp.clone()),
        mcp_servers: Vec::new(),
        skills: vec![skill.clone()],
        flows: Vec::new(),
        knowledge: Vec::new(),
        activity_bar: Vec::new(),
        tool_tasks: Vec::new(),
    };

    let mcp_before = mcp_fingerprint(&binding, &mcp).unwrap();
    let skill_before = skill_fingerprint(&binding, &skill).unwrap();
    skill.sha256 = "2".repeat(64);

    assert_eq!(mcp_fingerprint(&binding, &mcp).unwrap(), mcp_before);
    assert_ne!(skill_fingerprint(&binding, &skill).unwrap(), skill_before);
}

#[tokio::test]
async fn managed_ui_assets_are_bounded_to_verified_utf8_package_content() {
    use sha2::Digest;

    let temporary = tempfile::tempdir().unwrap();
    let package = temporary.path().join("package");
    tokio::fs::create_dir_all(&package).await.unwrap();
    let path = package.join("review.html");
    let content = b"<main>reviewed</main>";
    tokio::fs::write(&path, content).await.unwrap();
    let asset = ProjectedManagedAsset {
        path: path.clone(),
        sha256: format!("{:x}", sha2::Sha256::digest(content)),
        media_type: "text/html".to_string(),
    };

    let loaded = load_managed_ui_asset(
        &package,
        &asset,
        a3s_code_core::capability::UiAssetKind::Html,
    )
    .await
    .unwrap();
    assert_eq!(loaded.content(), "<main>reviewed</main>");

    tokio::fs::write(&path, b"<main>drifted</main>")
        .await
        .unwrap();
    let drift = load_managed_ui_asset(
        &package,
        &asset,
        a3s_code_core::capability::UiAssetKind::Html,
    )
    .await
    .expect_err("asset drift must fail reviewed-digest admission");
    assert!(drift.to_string().contains("reviewed evidence"));

    let outside_path = temporary.path().join("outside.html");
    let outside_content = b"<main>outside</main>";
    tokio::fs::write(&outside_path, outside_content)
        .await
        .unwrap();
    let outside = ProjectedManagedAsset {
        path: outside_path,
        sha256: format!("{:x}", sha2::Sha256::digest(outside_content)),
        media_type: "text/html".to_string(),
    };
    let escape = load_managed_ui_asset(
        &package,
        &outside,
        a3s_code_core::capability::UiAssetKind::Html,
    )
    .await
    .expect_err("a reviewed digest cannot authorize an asset outside the package");
    assert!(escape.to_string().contains("escapes its managed package"));
}

#[tokio::test]
async fn command_output_reader_discards_bytes_beyond_its_limit() {
    use tokio::io::AsyncWriteExt;

    let (mut writer, reader) = tokio::io::duplex(128);
    let write = tokio::spawn(async move {
        writer.write_all(&[b'x'; 64]).await.unwrap();
        writer.shutdown().await.unwrap();
    });
    let output = read_limited(reader, 16).await.unwrap();
    write.await.unwrap();

    assert_eq!(output.bytes, vec![b'x'; 16]);
    assert!(output.exceeded);
}

#[cfg(unix)]
fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}
