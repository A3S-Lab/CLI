use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::commands::code::asset_types::AssetPathRequest;

use super::*;

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn asset_lifecycle_commands_use_os_api() {
    let _guard = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
    let origin = spawn_cli_lifecycle_os_mock(captured.clone()).await;
    let root = temp_dir("code-cli-lifecycle-os");
    let env = CliLifecycleEnv::new(&root, &origin);
    let context = AssetCommandContext::from_process().expect("asset command context");

    let agent = run_asset_request(
        AssetRequest::Agent(AgentAssetRequest::Publish {
            path: Some(env.agent_package.clone()),
            kind: AgentAssetKind::Agentic,
        }),
        &context,
    )
    .await
    .expect("agent publish should use the OS API");
    let mcp = run_asset_request(
        AssetRequest::Mcp(McpAssetRequest::Publish(AssetPathRequest {
            path: Some(env.mcp_package.clone()),
        })),
        &context,
    )
    .await
    .expect("mcp publish should use the OS API");
    let skill = run_asset_request(
        AssetRequest::Skill(SkillAssetRequest::Publish(AssetPathRequest {
            path: Some(env.skill_package.clone()),
        })),
        &context,
    )
    .await
    .expect("skill publish should use the OS API");
    let flow = run_asset_request(
        AssetRequest::Flow(FlowAssetRequest::Publish(AssetPathRequest {
            path: Some(env.flow_file.clone()),
        })),
        &context,
    )
    .await
    .expect("flow publish should use the OS API");
    let okf = run_asset_request(
        AssetRequest::Okf(OkfAssetRequest::Publish(AssetPathRequest {
            path: Some(env.okf_package.clone()),
        })),
        &context,
    )
    .await
    .expect("okf publish should use the OS API");

    for (family, output) in [
        ("agent", agent),
        ("mcp", mcp),
        ("skill", skill),
        ("flow", flow),
        ("okf", okf),
    ] {
        assert_eq!(output.data["family"], family);
        assert_eq!(output.data["action"], "publish");
        assert!(output.data["assetName"].is_string(), "{family}");
        assert!(output.data["assetId"].is_string(), "{family}");
        assert!(output.data["view"]["url"].is_string(), "{family}");
    }

    let requests = captured.lock().unwrap().clone();
    let joined = requests.join("\n---\n");
    for expected in [
        r#""category":"agent""#,
        r#""agentKind":"agentic""#,
        r#""category":"mcp""#,
        r#""category":"skill""#,
        r#""category":"workflow""#,
        r#""category":"knowledge""#,
        r#""path":"agent.md""#,
        r#""path":"server.js""#,
        r#""path":"SKILL.md""#,
        r#""path":"flow.json""#,
        r#""path":"README.md""#,
        r#""path":".a3s/asset.acl""#,
    ] {
        assert!(
            joined.contains(expected),
            "missing `{expected}` in:\n{joined}"
        );
    }
    for forbidden in [
        "agent.runtime-binding.json",
        "mcp.runtime-binding.json",
        "skill.runtime-binding.json",
        "knowledge.runtime-binding.json",
        "runtime-binding.json",
        "debug",
        "/runtime/functions/mcp-asset-1/run",
        "/runtime/functions/mcp-asset-1/batch",
        "/run-mcp",
    ] {
        assert!(
            !joined.contains(forbidden),
            "unexpected legacy/config fragment `{forbidden}` in:\n{joined}"
        );
    }

    drop(env);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn bound_flow_deploy_resolves_fake_use_catalog_before_os_mutation() {
    use sha2::Digest as _;
    use std::os::unix::fs::PermissionsExt;

    let _guard = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
    let origin = spawn_cli_lifecycle_os_mock(captured.clone()).await;
    let root = temp_dir("code-cli-bound-flow-deploy");
    let env = CliLifecycleEnv::new(&root, &origin);

    let package_root = root.join("use-package");
    let flow_source = package_root.join("flows/review.ts");
    std::fs::create_dir_all(flow_source.parent().unwrap()).unwrap();
    let source = "export async function run(): Promise<string> { return 'ok'; }\n";
    std::fs::write(&flow_source, source).unwrap();
    let source_sha256 = format!("{:x}", sha2::Sha256::digest(source.as_bytes()));
    let installed_flow = serde_json::json!({
        "schema": "a3s.use.installed-flow.v1",
        "packageId": "use/acme/report",
        "flowId": "review",
        "version": "1.0.0",
        "lifecycleGeneration": 7,
        "sourceSha256": source_sha256,
    });
    let mut design = serde_json::json!({
        "version": "a3s.workflow.design.v1",
        "name": "daily-digest",
        "description": "Daily digest",
        "installedFlow": installed_flow,
        "nodes": [
            {"id": "start", "kind": "start"},
            {"id": "end", "kind": "end"}
        ],
        "edges": [
            {"id": "e1", "sourceNodeID": "start", "targetNodeID": "end"}
        ]
    });
    std::fs::write(
        &env.flow_file,
        serde_json::to_string_pretty(&design).unwrap(),
    )
    .unwrap();

    let snapshot = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 1,
            "generation": 7,
            "revision": "7".repeat(64),
            "capabilities": [{
                "id": "use/acme/report",
                "route": "report",
                "version": "1.0.0",
                "origin": "extension",
                "packageRoot": package_root,
                "lifecycleGeneration": 7,
                "enabled": true,
                "readiness": "ready",
                "surfaces": ["flow"],
                "flows": [{
                    "id": "review",
                    "engine": "a3s-flow",
                    "runtime": "native-ts",
                    "source": {
                        "path": flow_source,
                        "sha256": source_sha256,
                        "mediaType": "text/typescript"
                    },
                    "exportName": "run",
                    "requiresTools": ["convert"],
                    "requiresMcp": ["library"],
                    "requiresOkf": ["domain-knowledge"]
                }]
            }]
        }}
    });
    let use_bin = root.join("a3s-use-fixture");
    let script = format!(
        "#!/bin/sh\ncase \"$1 $2\" in\n  \"capability snapshot\") printf '%s\\n' {} ;;\n  *) exit 2 ;;\nesac\n",
        shell_single_quote(&snapshot.to_string()),
    );
    std::fs::write(&use_bin, script).unwrap();
    let mut permissions = std::fs::metadata(&use_bin).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&use_bin, permissions).unwrap();

    let mut context = AssetCommandContext::from_process().expect("asset command context");
    context.use_executable = Ok(Some(use_bin));
    let output = run_asset_request(
        AssetRequest::Flow(FlowAssetRequest::Deploy(AssetPathRequest {
            path: Some(env.flow_file.clone()),
        })),
        &context,
    )
    .await
    .expect("an exact installed Flow identity should deploy");
    assert_eq!(output.data["family"], "flow");
    assert_eq!(output.data["action"], "deploy");

    let joined = captured.lock().unwrap().join("\n---\n");
    for expected in [
        "PUT /api/v1/assets/workflow-asset-1/runtime-binding HTTP/1.1",
        r#""packageId":"use/acme/report""#,
        r#""flowId":"review""#,
        r#""catalogGeneration":7"#,
        r#""lifecycleGeneration":7"#,
        r#""engine":"a3s-flow""#,
        r#""runtime":"native-ts""#,
        r#""exportName":"run""#,
    ] {
        assert!(
            joined.contains(expected),
            "missing `{expected}` in:\n{joined}"
        );
    }
    assert!(!joined.contains("sourcePath"), "{joined}");
    let package_root_text = package_root.to_string_lossy();
    assert!(
        !joined.contains(&*package_root_text),
        "managed package paths must not cross the OS boundary:\n{joined}"
    );
    let asset_acl =
        std::fs::read_to_string(env.flow_file.parent().unwrap().join(".a3s/asset.acl")).unwrap();
    assert!(asset_acl.contains("installed_flow_package_id = \"use/acme/report\""));
    assert!(asset_acl.contains("installed_flow_lifecycle_generation = \"7\""));
    assert!(asset_acl.contains("installed_flow_engine = \"a3s-flow\""));
    assert!(asset_acl.contains("installed_flow_runtime = \"native-ts\""));
    assert!(asset_acl.contains("installed_flow_export_name = \"run\""));
    assert!(asset_acl.contains(&format!(
        "installed_flow_source_sha256 = \"{source_sha256}\""
    )));
    assert!(!asset_acl.contains("source_path"));

    captured.lock().unwrap().clear();
    design["installedFlow"]["version"] = serde_json::json!("2.0.0");
    std::fs::write(&env.flow_file, design.to_string()).unwrap();
    let error = run_asset_request(
        AssetRequest::Flow(FlowAssetRequest::Deploy(AssetPathRequest {
            path: Some(env.flow_file.clone()),
        })),
        &context,
    )
    .await
    .expect_err("a stale design must fail before contacting OS");
    assert!(error.to_string().contains("version mismatch"), "{error:#}");
    assert!(
        captured.lock().unwrap().is_empty(),
        "stale identity resolution must precede every OS request"
    );

    drop(env);
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn bound_flow_run_status_and_logs_share_local_durable_runtime_without_os() {
    use sha2::Digest as _;
    use std::os::unix::fs::PermissionsExt;

    let _guard = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
    let origin = spawn_cli_lifecycle_os_mock(captured.clone()).await;
    let root = temp_dir("code-cli-bound-flow-local-runtime");
    let env = CliLifecycleEnv::new(&root, &origin);
    let package_root = root.join("use-package");
    let flow_source = package_root.join("flows/review.ts");
    std::fs::create_dir_all(flow_source.parent().unwrap()).unwrap();
    let source = r#"#!/bin/sh
set -eu
/bin/cat >/dev/null
printf '%s\n' '{"protocol":"a3s.flow.native_ts.v1","kind":"workflow","ok":true,"output":{"type":"complete","output":{"surface":"shared-local"}}}'
"#;
    std::fs::write(&flow_source, source).unwrap();
    let source_sha256 = format!("{:x}", sha2::Sha256::digest(source.as_bytes()));
    std::fs::write(
        &env.flow_file,
        serde_json::json!({
            "version": "a3s.workflow.design.v1",
            "name": "local-review",
            "installedFlow": {
                "schema": "a3s.use.installed-flow.v1",
                "packageId": "use/acme/report",
                "flowId": "review",
                "version": "1.0.0",
                "lifecycleGeneration": 7,
                "sourceSha256": source_sha256,
            },
            "nodes": [],
            "edges": [],
        })
        .to_string(),
    )
    .unwrap();
    let snapshot = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 1,
            "generation": 7,
            "revision": "7".repeat(64),
            "capabilities": [{
                "id": "use/acme/report",
                "route": "report",
                "version": "1.0.0",
                "origin": "extension",
                "packageRoot": package_root,
                "lifecycleGeneration": 7,
                "enabled": true,
                "readiness": "ready",
                "surfaces": ["flow"],
                "flows": [{
                    "id": "review",
                    "engine": "a3s-flow",
                    "runtime": "native-ts",
                    "source": {
                        "path": flow_source,
                        "sha256": source_sha256,
                        "mediaType": "text/typescript"
                    },
                    "exportName": "run"
                }]
            }]
        }}
    });
    let use_bin = root.join("a3s-use-local-runtime-fixture");
    std::fs::write(
        &use_bin,
        format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n  \"capability snapshot\") printf '%s\\n' {} ;;\n  *) exit 2 ;;\nesac\n",
            shell_single_quote(&snapshot.to_string()),
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&use_bin).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&use_bin, permissions).unwrap();
    let compiler = root.join("flow-native-compiler");
    std::fs::write(
        &compiler,
        "#!/bin/sh\nset -eu\n[ \"$1\" = compile ]\n[ \"$3\" = -o ]\n/bin/cp \"$2\" \"$4\"\n/bin/chmod +x \"$4\"\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&compiler).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&compiler, permissions).unwrap();
    let previous_compiler = std::env::var_os("A3S_FLOW_NATIVE_TS_COMPILER");
    std::env::set_var("A3S_FLOW_NATIVE_TS_COMPILER", &compiler);

    let mut context = AssetCommandContext::from_process().expect("asset command context");
    context.use_executable = Ok(Some(use_bin));
    let run = run_asset_request(
        AssetRequest::Flow(FlowAssetRequest::Run(AssetPathRequest {
            path: Some(env.flow_file.clone()),
        })),
        &context,
    )
    .await
    .expect("bound Flow should execute through local a3s-flow");
    assert_eq!(run.data["action"], "run");
    assert_eq!(run.data["run"]["status"], "completed");
    assert_eq!(run.data["run"]["output"]["surface"], "shared-local");
    let run_id = run.data["run"]["runId"].as_str().unwrap().to_string();
    let package_root_text = package_root.display().to_string();
    assert!(!run.data.to_string().contains(&package_root_text));
    assert!(captured.lock().unwrap().is_empty());

    std::fs::remove_dir_all(&package_root).unwrap();
    context.use_executable = Ok(None);
    let status = run_asset_request(
        AssetRequest::Flow(FlowAssetRequest::Status(AssetPathRequest {
            path: Some(env.flow_file.clone()),
        })),
        &context,
    )
    .await
    .expect("status should survive package removal without Use or OS");
    let logs = run_asset_request(
        AssetRequest::Flow(FlowAssetRequest::Logs(AssetPathRequest {
            path: Some(env.flow_file.clone()),
        })),
        &context,
    )
    .await
    .expect("logs should survive package removal without Use or OS");
    assert_eq!(status.data["run"]["runId"], run_id);
    assert_eq!(logs.data["events"].as_array().unwrap().len(), 3);
    assert!(!logs.data.to_string().contains("entrypoint"));
    assert!(!logs.data.to_string().contains(&package_root_text));
    assert!(captured.lock().unwrap().is_empty());

    restore_env("A3S_FLOW_NATIVE_TS_COMPILER", previous_compiler);
    drop(env);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn all_location_composes_local_and_os_results_in_the_typed_service() {
    let _guard = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
    let origin = spawn_cli_lifecycle_os_mock(captured.clone()).await;
    let root = temp_dir("code-cli-list-all");
    let env = CliLifecycleEnv::new(&root, &origin);
    let context = AssetCommandContext::from_process().expect("asset command context");

    let output = run_asset_request(
        AssetRequest::Agent(AgentAssetRequest::List(AssetListRequest {
            location: AssetListLocation::All,
            query: Some("reviewer".to_string()),
        })),
        &context,
    )
    .await
    .expect("all-location list should compose both sources");

    assert_eq!(output.data["family"], "agent");
    assert_eq!(output.data["location"], "all");
    assert_eq!(output.data["query"], "reviewer");
    assert_eq!(output.data["local"]["location"], "local");
    assert_eq!(output.data["os"]["location"], "os");
    assert!(output.data["local"]["assets"]
        .to_string()
        .contains("reviewer"));

    let requests = captured.lock().unwrap().join("\n---\n");
    assert!(requests.contains("GET /api/v1/assets?"), "{requests}");
    assert!(requests.contains("category=agent"), "{requests}");
    assert!(requests.contains("search=reviewer"), "{requests}");

    drop(env);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn code_cli_mcp_run_requires_mcp_runner_without_runtime_function_fallback() {
    let _guard = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let captured = std::sync::Arc::new(Mutex::new(Vec::new()));
    let origin = spawn_cli_lifecycle_os_mock(captured.clone()).await;
    let root = temp_dir("code-cli-mcp-run-no-fallback");
    let env = CliLifecycleEnv::new(&root, &origin);
    let context = AssetCommandContext::from_process().expect("asset command context");

    let err = run_asset_request(
        AssetRequest::Mcp(McpAssetRequest::Run(AssetPathRequest {
            path: Some(env.mcp_package.clone()),
        })),
        &context,
    )
    .await
    .expect_err("mcp run should not fall back to Runtime Function run");
    assert!(
        err.to_string()
            .contains("did not expose a runnable MCP capability"),
        "{err}"
    );

    let requests = captured.lock().unwrap().clone();
    let joined = requests.join("\n---\n");
    assert!(joined.contains(r#""category":"mcp""#), "{joined}");
    assert!(
        !joined.contains("/runtime/functions/mcp-asset-1/run"),
        "{joined}"
    );
    assert!(
        !joined.contains("/runtime/functions/mcp-asset-1/batch"),
        "{joined}"
    );
    drop(env);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn scoped_queries_match_tui_shape() {
    assert_eq!(os_asset_category_query("mcp", ""), "category:mcp");
    assert_eq!(
        os_asset_category_query("mcp", "weather"),
        "category:mcp weather"
    );
    assert_eq!(
        runtime_asset_query("workflow", "flow-demo", "failed"),
        "category:workflow flow-demo failed"
    );
}

#[test]
fn resolves_single_agent_from_current_directory() {
    let _guard = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = temp_dir("code-cli-agent");
    let package = dir.join("reviewer");
    std::fs::create_dir_all(&package).unwrap();
    let agent = package.join("agent.md");
    std::fs::write(
        &agent,
        "---\nname: reviewer\ndescription: Review code changes carefully\n---\nReview.\n",
    )
    .unwrap();
    let old_cwd = std::env::current_dir().unwrap();
    let old_agent_dir = std::env::var_os("A3S_AGENT_DIR");
    std::env::set_var("A3S_AGENT_DIR", &dir);
    std::env::set_current_dir(&dir).unwrap();

    let context = AssetCommandContext::from_process().expect("asset command context");
    let dev = resolve_agent_dev(None, &context).expect("single agent in cwd");
    let dev_path = std::fs::canonicalize(&dev.path).unwrap();
    let agent_path = std::fs::canonicalize(&agent).unwrap();

    std::env::set_current_dir(old_cwd).unwrap();
    restore_env("A3S_AGENT_DIR", old_agent_dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(dev.name, "reviewer");
    assert_eq!(dev.rel, "reviewer");
    assert_eq!(dev.definition_rel, "agent.md");
    assert_eq!(dev_path, agent_path);
}

#[test]
fn resolves_agent_package_from_entry_file_path_for_compatibility() {
    let _guard = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = temp_dir("code-cli-agent-entry");
    let package = dir.join("agents/reviewer");
    std::fs::create_dir_all(&package).unwrap();
    let agent = package.join("agent.md");
    std::fs::write(
        &agent,
        "---\nname: reviewer\ndescription: Review code changes carefully\n---\nReview.\n",
    )
    .unwrap();
    let old_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&dir).unwrap();

    let context = AssetCommandContext::from_process().expect("asset command context");
    let dev = resolve_agent_dev(Some(agent.clone()), &context)
        .expect("entry file should resolve to package");
    let dev_path = std::fs::canonicalize(&dev.path).unwrap();
    let agent_path = std::fs::canonicalize(&agent).unwrap();
    let package_path = std::fs::canonicalize(&dev.package_path).unwrap();
    let expected_package = std::fs::canonicalize(&package).unwrap();

    std::env::set_current_dir(old_cwd).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(dev.name, "reviewer");
    assert_eq!(dev.rel, "reviewer");
    assert_eq!(dev.definition_rel, "agent.md");
    assert_eq!(dev_path, agent_path);
    assert_eq!(package_path, expected_package);
}

#[test]
fn resolves_okf_package_from_visible_readme_path() {
    let _guard = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = temp_dir("code-cli-okf");
    let workspace = dir.join("workspace");
    let package = workspace.join("okf/ops");
    std::fs::create_dir_all(package.join("sources")).unwrap();
    let readme = package.join("README.md");
    std::fs::write(&readme, "# ops-knowledge\n\nOperations knowledge\n").unwrap();
    let old_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&workspace).unwrap();

    let context = AssetCommandContext::from_process().expect("asset command context");
    let dev = resolve_okf_dev(Some(readme.clone()), &context)
        .expect("README path should resolve to package dir");
    let dev_path = std::fs::canonicalize(&dev.path).unwrap();
    let package_path = std::fs::canonicalize(&package).unwrap();

    std::env::set_current_dir(old_cwd).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(dev.name, "ops-knowledge");
    assert_eq!(dev.rel, "ops");
    assert_eq!(dev_path, package_path);
}

struct CliLifecycleEnv {
    root: PathBuf,
    agent_package: PathBuf,
    mcp_package: PathBuf,
    skill_package: PathBuf,
    flow_file: PathBuf,
    okf_package: PathBuf,
    old_cwd: PathBuf,
    old_env: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl CliLifecycleEnv {
    fn new(root: &Path, origin: &str) -> Self {
        let workspace = root.join("workspace");
        let home = root.join("home");
        let agent_root = root.join("agents");
        let mcp_root = root.join("mcps");
        let skill_root = root.join("skills");
        let flow_root = root.join("flows");
        let memory_root = root.join("memory");
        let agent_package = agent_root.join("reviewer");
        let mcp_package = mcp_root.join("weather");
        let skill_package = skill_root.join("sql-checker");
        let flow_package = flow_root.join("daily-digest");
        let flow_file = flow_package.join("flow.json");
        let okf_package = workspace.join("okf").join("ops-runbook");
        for dir in [
            &workspace,
            &home,
            &agent_package,
            &mcp_package,
            &skill_package,
            &flow_package,
            &okf_package,
            &memory_root,
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::create_dir_all(agent_package.join(".a3s")).unwrap();
        std::fs::create_dir_all(mcp_package.join(".a3s")).unwrap();
        std::fs::create_dir_all(skill_package.join(".a3s")).unwrap();
        std::fs::create_dir_all(flow_package.join(".a3s")).unwrap();
        std::fs::create_dir_all(okf_package.join(".a3s")).unwrap();
        std::fs::create_dir_all(okf_package.join("sources")).unwrap();
        for dir in ["prompts", "workflows", "examples", "eval", "tests"] {
            std::fs::create_dir_all(agent_package.join(dir)).unwrap();
        }

        std::fs::write(
            agent_package.join("agent.md"),
            "---\nname: reviewer\ndescription: Review code changes carefully\nprompt: Review the target carefully.\n---\nReview code.\n",
        )
        .unwrap();
        std::fs::write(agent_package.join("README.md"), "# reviewer\n").unwrap();
        std::fs::write(
            agent_package.join("prompts/system.md"),
            "Review the target carefully.\n",
        )
        .unwrap();
        std::fs::write(
            agent_package.join("workflows/operating-procedure.md"),
            "Inspect, plan, execute, and report.\n",
        )
        .unwrap();
        std::fs::write(
            agent_package.join("examples/example-input.md"),
            "Review this diff.\n",
        )
        .unwrap();
        std::fs::write(
            agent_package.join("examples/example-output.md"),
            "Review complete.\n",
        )
        .unwrap();
        std::fs::write(agent_package.join("eval/smoke.md"), "Smoke eval.\n").unwrap();
        std::fs::write(agent_package.join("tests/smoke.md"), "Smoke test.\n").unwrap();
        std::fs::write(
            agent_package.join(".a3s/asset.acl"),
            "category = \"agent\"\n",
        )
        .unwrap();

        std::fs::write(
            mcp_package.join("README.md"),
            "# weather\n\nWeather MCP tools\n",
        )
        .unwrap();
        std::fs::write(mcp_package.join("server.js"), "process.stdin.resume();\n").unwrap();
        std::fs::write(mcp_package.join(".a3s/asset.acl"), "category = \"mcp\"\n").unwrap();

        std::fs::write(
            skill_package.join("SKILL.md"),
            "---\nname: sql-checker\ndescription: Check SQL safely\nkind: instruction\n---\nCheck SQL for risky patterns.\n",
        )
        .unwrap();
        std::fs::write(
            skill_package.join(".a3s/asset.acl"),
            "category = \"skill\"\n",
        )
        .unwrap();

        std::fs::write(
            &flow_file,
            r#"{"version":"a3s.workflow.design.v1","name":"daily-digest","description":"Daily digest","nodes":[{"id":"start","kind":"start"},{"id":"end","kind":"end"}],"edges":[{"id":"e1","sourceNodeID":"start","targetNodeID":"end"}]}"#,
        )
        .unwrap();
        std::fs::write(
            flow_package.join(".a3s/asset.acl"),
            "category = \"workflow\"\n",
        )
        .unwrap();

        std::fs::write(
            okf_package.join("README.md"),
            "# ops-runbook\n\nOperations response knowledge\n",
        )
        .unwrap();
        std::fs::write(
            okf_package.join("sources/overview.md"),
            "# Operations\n\nRestart and escalation notes.\n",
        )
        .unwrap();
        std::fs::write(
            okf_package.join(".a3s/asset.acl"),
            "category = \"knowledge\"\n",
        )
        .unwrap();

        let config = root.join("config.acl");
        write_lifecycle_config(&config, origin);
        write_lifecycle_auth_store(&home, origin);

        let keys = vec![
            "HOME",
            "A3S_CONFIG_FILE",
            "A3S_AGENT_DIR",
            "A3S_MCP_DIR",
            "A3S_SKILL_DIR",
            "A3S_FLOW_DIR",
            "A3S_MEMORY_DIR",
            crate::a3s_os::OS_ENV_BASE_URL,
            crate::a3s_os::OS_ENV_TOKEN,
            crate::a3s_os::OS_ENV_REFRESH_TOKEN,
        ];
        let old_env = keys
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        let old_cwd = std::env::current_dir().unwrap();

        std::env::set_var("HOME", &home);
        std::env::set_var("A3S_CONFIG_FILE", &config);
        std::env::set_var("A3S_AGENT_DIR", &agent_root);
        std::env::set_var("A3S_MCP_DIR", &mcp_root);
        std::env::set_var("A3S_SKILL_DIR", &skill_root);
        std::env::set_var("A3S_FLOW_DIR", &flow_root);
        std::env::set_var("A3S_MEMORY_DIR", &memory_root);
        std::env::remove_var(crate::a3s_os::OS_ENV_BASE_URL);
        std::env::remove_var(crate::a3s_os::OS_ENV_TOKEN);
        std::env::remove_var(crate::a3s_os::OS_ENV_REFRESH_TOKEN);
        std::env::set_current_dir(&workspace).unwrap();

        Self {
            root: root.to_path_buf(),
            agent_package,
            mcp_package,
            skill_package,
            flow_file,
            okf_package,
            old_cwd,
            old_env,
        }
    }
}

impl Drop for CliLifecycleEnv {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.old_cwd);
        for (key, value) in self.old_env.drain(..) {
            restore_env(key, value);
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write_lifecycle_config(path: &Path, origin: &str) {
    std::fs::write(
        path,
        format!(
            "default_model = \"openai/x\"\n\
             os = \"{origin}\"\n\
             providers \"openai\" {{\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
             models \"x\" {{ name = \"x\" }}\n}}\n\
             memory {{\n  llmExtraction = false\n}}\n"
        ),
    )
    .unwrap();
}

fn write_lifecycle_auth_store(home: &Path, origin: &str) {
    let store = home.join(".a3s").join("os-auth.json");
    std::fs::create_dir_all(store.parent().unwrap()).unwrap();
    std::fs::write(
        store,
        serde_json::to_string_pretty(&serde_json::json!({
            "sessions": [{
                "address": origin,
                "access_token": "token",
                "token_type": "Bearer",
                "login_at_ms": 1
            }]
        }))
        .unwrap(),
    )
    .unwrap();
}

async fn spawn_cli_lifecycle_os_mock(captured: std::sync::Arc<Mutex<Vec<String>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let captured = captured.clone();
            tokio::spawn(async move {
                let request = read_http_request(&mut sock).await;
                let line = request.lines().next().unwrap_or("").to_string();
                let body = request.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                captured.lock().unwrap().push(format!("{line}\n{body}"));
                let (status, payload) = cli_lifecycle_mock_response(&line, &body);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.flush().await;
            });
        }
    });
    origin
}

async fn read_http_request(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0_u8; 8192];
    let mut expected_len = None;
    while let Ok(n) = sock.read(&mut tmp).await {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if expected_len.is_none() {
            expected_len = expected_http_request_len(&buf);
        }
        if expected_len.is_some_and(|len| buf.len() >= len) {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn expected_http_request_len(buf: &[u8]) -> Option<usize> {
    let header_end = buf.windows(4).position(|window| window == b"\r\n\r\n")? + 4;
    let headers = String::from_utf8_lossy(&buf[..header_end]);
    let content_len = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    Some(header_end + content_len)
}

fn cli_lifecycle_mock_response(line: &str, body: &str) -> (&'static str, String) {
    if line.starts_with("GET /api/v1/assets?") {
        return ("200 OK", r#"{"data":{"items":[]}}"#.to_string());
    }
    if line.starts_with("PATCH /api/v1/assets/") {
        return ("200 OK", r#"{"data":{"ok":true}}"#.to_string());
    }
    if line.starts_with("POST /api/v1/assets HTTP/1.1") {
        let (id, name) = if body.contains(r#""category":"agent""#) {
            ("asset-agentic-1", "agentic-reviewer")
        } else if body.contains(r#""category":"mcp""#) {
            ("mcp-asset-1", "mcp-weather")
        } else if body.contains(r#""category":"skill""#) {
            ("skill-asset-1", "skill-sql-checker")
        } else if body.contains(r#""category":"workflow""#) {
            ("workflow-asset-1", "flow-daily-digest")
        } else if body.contains(r#""category":"knowledge""#) {
            ("knowledge-asset-1", "knowledge-ops-runbook")
        } else {
            return (
                "422 Unprocessable Entity",
                r#"{"code":422,"message":"unknown category"}"#.to_string(),
            );
        };
        return (
            "200 OK",
            format!(
                r#"{{"data":{{"id":"{id}","name":"{name}","ownerName":"admin","defaultBranch":"main"}}}}"#
            ),
        );
    }
    if line.contains("/repository/files ") {
        return ("200 OK", r#"{"data":{"ok":true}}"#.to_string());
    }
    if line.contains("/agent-config/validate ") {
        return (
            "200 OK",
            r#"{"code":200,"data":{"valid":true,"diagnostics":[]}}"#.to_string(),
        );
    }
    if line.contains("/agent-config ") {
        return (
            "200 OK",
            r#"{"code":200,"data":{"configured":true}}"#.to_string(),
        );
    }
    if line.contains("/runtime-binding/validate ") {
        return (
            "200 OK",
            r#"{"code":200,"data":{"configured":true,"valid":true,"requiredSecrets":[],"missingSecrets":[],"expiredSecrets":[],"issues":[]}}"#.to_string(),
        );
    }
    if line.contains("/runtime-binding ") {
        return (
            "200 OK",
            r#"{"code":200,"data":{"configured":true}}"#.to_string(),
        );
    }
    if line.starts_with("POST /api/v1/kernel/capabilities ") {
        return (
            "404 Not Found",
            r#"{"code":404,"message":"capabilities unavailable in mock"}"#.to_string(),
        );
    }
    (
        "404 Not Found",
        format!(r#"{{"code":404,"message":"unhandled mock request: {line}"}}"#),
    )
}

fn temp_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "a3s-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
    match value {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

#[cfg(unix)]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
