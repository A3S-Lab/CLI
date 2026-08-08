#![cfg(unix)]

mod support;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Output, Stdio};

use a3s_use_extension::ResolvedRemotePackage;
use support::{a3s_bin, configure_component_env, make_executable, sh_quote, TempWorkspace};

#[path = "support/tuf_test_support.rs"]
mod tuf_test_support;

use tuf_test_support::{
    extension_archive, TestRepository, TestServer, EXPIRED, FUTURE, PACKAGE_VERSION,
};

#[test]
fn read_only_plugin_manager_mcp_discovers_and_plans_one_signed_plugin() {
    let temp = TempWorkspace::new("plugin-manager-mcp-signed-plan");
    let repository = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, FUTURE);
    let server = TestServer::start(repository.routes.clone());
    let registry_url = localhost_url(&server);
    let config = temp.path("config/config.acl");
    let workspace = temp.path("workspace");
    let use_bin = temp.path("use-bin");
    let install_log = temp.path("plugin-manager-mcp-install.log");
    std::fs::create_dir_all(&workspace).unwrap();
    make_use_fixture(&use_bin, &install_log);
    add_registry(
        &temp,
        &config,
        &use_bin,
        &registry_url,
        &format!("sha256:{}", repository.root_sha256),
    );

    server.clear_requests();
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let child = command
        .arg("--config")
        .arg(&config)
        .arg("--directory")
        .arg(&workspace)
        .args(["--non-interactive", "--no-progress", "plugin", "mcp-serve"])
        .env("A3S_USE_INSTALL_DIR", &use_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch Plugin Manager MCP");
    let mut child = McpChildGuard(child);
    let mut stdin = child.0.stdin.take().expect("Plugin Manager MCP stdin");
    let stdout = child.0.stdout.take().expect("Plugin Manager MCP stdout");
    let mut reader = BufReader::new(stdout);

    write_mcp_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "a3s-plugin-manager-e2e", "version": "1"}
            }
        }),
    );
    let initialized = read_mcp_response(&mut reader, 1);
    assert_eq!(
        initialized["result"]["serverInfo"]["name"],
        "a3s-plugin-manager"
    );
    write_mcp_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    );

    write_mcp_message(
        &mut stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );
    let listed = read_mcp_response(&mut reader, 2);
    let tools = listed["result"]["tools"]
        .as_array()
        .expect("Plugin Manager MCP tools");
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "plugin_search",
            "plugin_inspect",
            "plugin_list_installed",
            "plugin_status",
            "plugin_plan_install",
            "plugin_plan_upgrade",
            "plugin_plan_uninstall",
        ]
    );
    assert!(tools.iter().all(|tool| {
        tool["annotations"]["readOnlyHint"] == true
            && tool["annotations"]["destructiveHint"] == false
    }));

    write_mcp_tool_call(
        &mut stdin,
        3,
        "plugin_search",
        serde_json::json!({"query": "science", "channel": "stable", "limit": 10}),
    );
    let searched = read_mcp_response(&mut reader, 3);
    assert_eq!(
        searched["result"]["structuredContent"]["items"][0]["packageId"],
        "a3s/science"
    );
    assert_eq!(
        searched["result"]["structuredContent"]["items"][0]["source"]["kind"],
        "registry"
    );
    assert!(
        searched["result"]["structuredContent"]["items"][0]["archiveSha256"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64)
    );

    write_mcp_tool_call(
        &mut stdin,
        4,
        "plugin_inspect",
        serde_json::json!({
            "packageId": "a3s/science",
            "version": PACKAGE_VERSION,
            "channel": "stable"
        }),
    );
    let inspected = read_mcp_response(&mut reader, 4);
    assert_eq!(
        inspected["result"]["structuredContent"]["matches"][0]["packageId"],
        "a3s/science"
    );

    write_mcp_tool_call(
        &mut stdin,
        5,
        "plugin_plan_install",
        serde_json::json!({
            "packageId": "a3s/science",
            "versionRequirement": format!("={PACKAGE_VERSION}"),
            "channel": "stable",
            "scopeKind": "user",
            "scopeId": "current"
        }),
    );
    let planned = read_mcp_response(&mut reader, 5);
    assert_eq!(
        planned["result"]["structuredContent"]["plan"]["dryRun"],
        true
    );
    assert_eq!(
        planned["result"]["structuredContent"]["plan"]["plans"][0]["resolvedRegistryPackages"]
            ["use/a3s/science"]["sha256"],
        repository.target_sha256
    );
    assert!(
        planned["result"]["structuredContent"]["plan"]["operationId"]
            .as_str()
            .is_some_and(|value| value.starts_with("plugin-install-"))
    );
    assert!(
        planned["result"]["structuredContent"]["plan"]["canonicalPlanDigest"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    );
    assert!(
        !install_log.exists(),
        "creating an agent plan must not apply it"
    );
    assert_no_target_request(&server);

    write_mcp_tool_call(
        &mut stdin,
        6,
        "plugin_search",
        serde_json::json!({
            "query": "science",
            "registryUrl": "https://example.invalid"
        }),
    );
    let arbitrary_source = read_mcp_response(&mut reader, 6);
    assert_eq!(arbitrary_source["result"]["isError"], true);
    assert_eq!(
        arbitrary_source["result"]["structuredContent"]["code"],
        "plugin.request_invalid"
    );

    write_mcp_tool_call(
        &mut stdin,
        7,
        "plugin_apply_plan",
        serde_json::json!({
            "operationId": planned["result"]["structuredContent"]["plan"]["operationId"],
            "planDigest": planned["result"]["structuredContent"]["plan"]["canonicalPlanDigest"]
        }),
    );
    let forbidden = read_mcp_response(&mut reader, 7);
    assert_eq!(forbidden["error"]["code"], -32602);
    assert!(forbidden["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("not exposed")));
    assert!(
        !install_log.exists(),
        "the read-only management MCP must not apply a plan"
    );
}

#[test]
fn first_class_plugin_cli_applies_and_replays_one_reviewed_signed_plan() {
    let temp = TempWorkspace::new("plugin-cli-signed-install");
    let repository = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, FUTURE);
    let server = TestServer::start(repository.routes.clone());
    let registry_url = localhost_url(&server);
    let config = temp.path("config/config.acl");
    let use_bin = temp.path("use-bin");
    let install_log = temp.path("plugin-cli-install.log");
    make_use_fixture(&use_bin, &install_log);
    add_registry(
        &temp,
        &config,
        &use_bin,
        &registry_url,
        &format!("sha256:{}", repository.root_sha256),
    );

    let plan = run(
        &temp,
        &config,
        &use_bin,
        &[
            "plugin",
            "install",
            "a3s/science",
            "--version",
            PACKAGE_VERSION,
            "--channel",
            "stable",
            "--dry-run",
        ],
    );
    assert!(plan.status.success(), "{plan:?}");
    let plan = json(&plan);
    assert_eq!(plan["command"], "plugin.install");
    assert_eq!(plan["data"]["dryRun"], true);
    assert_eq!(plan["data"]["capabilityState"]["status"], "verified");
    assert_eq!(plan["data"]["capabilityState"]["generation"], 0);
    let operation_id = plan["data"]["operationId"].as_str().unwrap().to_string();
    let plan_digest = plan["data"]["canonicalPlanDigest"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(plan_digest.starts_with("sha256:"));
    assert!(
        !install_log.exists(),
        "planning must not invoke installation"
    );

    server.clear_requests();
    let applied = run(
        &temp,
        &config,
        &use_bin,
        &[
            "plugin",
            "apply",
            &operation_id,
            "--plan-digest",
            &plan_digest,
            "--yes",
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    let applied = json(&applied);
    assert_eq!(applied["command"], "plugin.apply");
    assert_eq!(applied["data"]["operationId"], operation_id);
    assert_eq!(applied["data"]["canonicalPlanDigest"], plan_digest);
    assert_eq!(applied["data"]["replayed"], false);
    assert_eq!(applied["data"]["capabilityBefore"]["generation"], 0);
    assert_eq!(applied["data"]["capabilityAfter"]["generation"], 1);
    let graph = &applied["data"]["operations"][0]["packageGraph"];
    assert_eq!(graph["plan"]["plan"]["operationId"], operation_id);
    assert_eq!(graph["root"]["receipt"]["packageId"], "a3s/science");
    assert_eq!(graph["root"]["receipt"]["version"], PACKAGE_VERSION);
    assert_eq!(graph["root"]["receipt"]["trust"], "registry-tuf");
    assert!(temp.path("state/use/extensions/a3s/science.json").is_file());
    assert_eq!(target_request_count(&server), 1);
    assert!(
        !install_log.exists(),
        "reviewed apply must not invoke a child A3S Use mutation"
    );

    server.clear_requests();
    let replayed = run(
        &temp,
        &config,
        &use_bin,
        &[
            "plugin",
            "apply",
            &operation_id,
            "--plan-digest",
            &plan_digest,
            "--yes",
        ],
    );
    assert!(replayed.status.success(), "{replayed:?}");
    let replayed = json(&replayed);
    assert_eq!(replayed["data"]["operationId"], operation_id);
    assert_eq!(replayed["data"]["replayed"], true);
    assert_eq!(
        replayed["data"]["operations"],
        applied["data"]["operations"]
    );
    assert_eq!(target_request_count(&server), 0);
    assert!(!install_log.exists());
}

#[test]
fn signed_registry_plan_is_bound_before_in_process_apply() {
    let temp = TempWorkspace::new("signed-registry-install");
    let version_one = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, FUTURE);
    let server = TestServer::start(version_one.routes.clone());
    let registry_url = localhost_url(&server);
    let config = temp.path("config/config.acl");
    let use_bin = temp.path("use-bin");
    let install_log = temp.path("remote-install.log");
    make_use_fixture(&use_bin, &install_log);
    add_registry(
        &temp,
        &config,
        &use_bin,
        &registry_url,
        &format!("sha256:{}", version_one.root_sha256),
    );

    server.clear_requests();
    let refreshed = run(
        &temp,
        &config,
        &use_bin,
        &["registry", "refresh", "localhost"],
    );
    assert!(refreshed.status.success(), "{refreshed:?}");
    let refreshed = json(&refreshed);
    assert_eq!(
        refreshed["data"]["registries"][0]["metadata"]["targetsVersion"],
        1
    );
    assert_no_target_request(&server);

    server.clear_requests();
    let first_plan = run(
        &temp,
        &config,
        &use_bin,
        &["install", "use/a3s/science", "--dry-run"],
    );
    assert!(first_plan.status.success(), "{first_plan:?}");
    let first_plan = json(&first_plan);
    let first_digest = first_plan["data"]["planDigest"]
        .as_str()
        .unwrap()
        .to_string();
    let first_package =
        &first_plan["data"]["plans"][0]["resolvedRegistryPackages"]["use/a3s/science"];
    assert_eq!(first_package["registryName"], "localhost");
    assert_eq!(first_package["targetsVersion"], 1);
    assert_eq!(first_package["sha256"], version_one.target_sha256);
    assert_no_target_request(&server);
    assert!(!install_log.exists());

    let version_two = TestRepository::new(extension_archive(PACKAGE_VERSION), 2, FUTURE);
    assert_eq!(version_one.root_sha256, version_two.root_sha256);
    server.replace_routes(version_two.routes.clone());
    server.clear_requests();
    let stale = run(
        &temp,
        &config,
        &use_bin,
        &["install", "use/a3s/science", "--plan-digest", &first_digest],
    );
    assert!(!stale.status.success(), "{stale:?}");
    assert_eq!(json(&stale)["error"]["code"], "component.plan_mismatch");
    assert_no_target_request(&server);
    assert!(!install_log.exists());

    let current_plan = run(
        &temp,
        &config,
        &use_bin,
        &["install", "use/a3s/science", "--dry-run"],
    );
    assert!(current_plan.status.success(), "{current_plan:?}");
    let current_plan = json(&current_plan);
    let current_digest = current_plan["data"]["planDigest"].as_str().unwrap();
    let package: ResolvedRemotePackage = serde_json::from_value(
        current_plan["data"]["plans"][0]["resolvedRegistryPackages"]["use/a3s/science"].clone(),
    )
    .unwrap();
    assert_eq!(package.targets_version, 2);
    let registry_plan_digest = package.plan_digest().unwrap();

    server.clear_requests();
    let applied = run(
        &temp,
        &config,
        &use_bin,
        &[
            "install",
            "use/a3s/science",
            "--plan-digest",
            current_digest,
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    let applied = json(&applied);
    assert_eq!(applied["data"]["planDigest"], current_digest);
    let graph = &applied["data"]["operations"][0]["packageGraph"];
    assert_eq!(graph["plan"]["plan"]["packageId"], "a3s/science");
    assert_eq!(graph["root"]["receipt"]["version"], PACKAGE_VERSION);
    let applied_registry: ResolvedRemotePackage =
        serde_json::from_value(graph["root"]["receipt"]["registry"].clone()).unwrap();
    assert_eq!(applied_registry, package);
    assert_eq!(
        applied_registry.plan_digest().unwrap(),
        registry_plan_digest
    );
    assert_eq!(target_request_count(&server), 1);
    assert!(temp.path("state/use/extensions/a3s/science.json").is_file());
    assert!(
        !install_log.exists(),
        "reviewed component apply must not invoke a child A3S Use mutation"
    );

    server.clear_requests();
    let unsigned = run(
        &temp,
        &config,
        &use_bin,
        &["install", "use/a3s/science", "--allow-unsigned"],
    );
    assert!(!unsigned.status.success(), "{unsigned:?}");
    assert_eq!(json(&unsigned)["error"]["code"], "usage.invalid");
    assert!(server.requests().is_empty());
    assert!(!install_log.exists());
}

#[test]
fn signed_registry_upgrade_restores_the_recorded_source_and_binds_the_new_target() {
    const NEXT_VERSION: &str = "0.2.0";

    let temp = TempWorkspace::new("signed-registry-upgrade");
    let version_one = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, FUTURE);
    let server = TestServer::start(version_one.routes.clone());
    let registry_url = localhost_url(&server);
    let config = temp.path("config/config.acl");
    let use_bin = temp.path("use-bin");
    let install_log = temp.path("remote-upgrade.log");
    make_use_fixture(&use_bin, &install_log);
    add_registry(
        &temp,
        &config,
        &use_bin,
        &registry_url,
        &format!("sha256:{}", version_one.root_sha256),
    );

    let initial = run(
        &temp,
        &config,
        &use_bin,
        &["install", "use/a3s/science", "--dry-run"],
    );
    assert!(initial.status.success(), "{initial:?}");
    let initial = json(&initial);
    let initial_digest = initial["data"]["planDigest"].as_str().unwrap().to_string();
    let installed: ResolvedRemotePackage = serde_json::from_value(
        initial["data"]["plans"][0]["resolvedRegistryPackages"]["use/a3s/science"].clone(),
    )
    .unwrap();
    server.clear_requests();
    let initial_applied = run(
        &temp,
        &config,
        &use_bin,
        &[
            "install",
            "use/a3s/science",
            "--plan-digest",
            &initial_digest,
        ],
    );
    assert!(initial_applied.status.success(), "{initial_applied:?}");
    let initial_applied = json(&initial_applied);
    assert_eq!(
        initial_applied["data"]["operations"][0]["packageGraph"]["root"]["receipt"]["version"],
        PACKAGE_VERSION
    );
    assert_eq!(target_request_count(&server), 1);
    assert!(!install_log.exists());
    make_installed_use_fixture(&use_bin, &install_log, &installed);

    let version_two = TestRepository::with_package_version(
        extension_archive(NEXT_VERSION),
        NEXT_VERSION,
        2,
        FUTURE,
    );
    assert_eq!(version_one.root_sha256, version_two.root_sha256);
    server.replace_routes(version_two.routes.clone());
    server.clear_requests();
    let available = run(&temp, &config, &use_bin, &["upgrade"]);
    assert!(available.status.success(), "{available:?}");
    let available = json(&available);
    let components = available["data"]["components"].as_array().unwrap();
    let science = components
        .iter()
        .find(|component| component["id"] == "use/a3s/science")
        .expect("signed extension should be listed as upgradeable");
    assert_eq!(science["update"], "available");
    assert_no_target_request(&server);

    server.clear_requests();
    let all_plan = run(&temp, &config, &use_bin, &["upgrade", "--all", "--dry-run"]);
    assert!(all_plan.status.success(), "{all_plan:?}");
    let all_plan = json(&all_plan);
    assert_eq!(all_plan["data"]["plans"].as_array().unwrap().len(), 1);
    assert_eq!(all_plan["data"]["plans"][0]["component"], "use/a3s/science");
    assert_no_target_request(&server);

    server.clear_requests();
    let first_plan = run(
        &temp,
        &config,
        &use_bin,
        &["upgrade", "use/a3s/science", "--dry-run"],
    );
    assert!(first_plan.status.success(), "{first_plan:?}");
    let first_plan = json(&first_plan);
    let first_digest = first_plan["data"]["planDigest"]
        .as_str()
        .unwrap()
        .to_string();
    let operation = &first_plan["data"]["plans"][0];
    assert_eq!(operation["action"], "upgrade");
    assert_eq!(operation["source"], "registry:localhost");
    assert_eq!(operation["channel"], "stable");
    assert_eq!(operation["mutates"], true);
    assert_eq!(operation["force"], true);
    assert_eq!(
        operation["resolvedRegistryPackages"]["use/a3s/science"]["version"],
        NEXT_VERSION
    );
    assert_no_target_request(&server);
    assert!(!install_log.exists());

    let version_three = TestRepository::with_package_version(
        extension_archive(NEXT_VERSION),
        NEXT_VERSION,
        3,
        FUTURE,
    );
    server.replace_routes(version_three.routes.clone());
    server.clear_requests();
    let stale = run(
        &temp,
        &config,
        &use_bin,
        &["upgrade", "use/a3s/science", "--plan-digest", &first_digest],
    );
    assert!(!stale.status.success(), "{stale:?}");
    assert_eq!(json(&stale)["error"]["code"], "component.plan_mismatch");
    assert_no_target_request(&server);
    assert!(!install_log.exists());

    let current = run(
        &temp,
        &config,
        &use_bin,
        &["upgrade", "use/a3s/science", "--dry-run"],
    );
    assert!(current.status.success(), "{current:?}");
    let current = json(&current);
    let current_digest = current["data"]["planDigest"].as_str().unwrap();
    let package: ResolvedRemotePackage = serde_json::from_value(
        current["data"]["plans"][0]["resolvedRegistryPackages"]["use/a3s/science"].clone(),
    )
    .unwrap();
    let registry_plan_digest = package.plan_digest().unwrap();

    server.clear_requests();
    let applied = run(
        &temp,
        &config,
        &use_bin,
        &[
            "upgrade",
            "use/a3s/science",
            "--plan-digest",
            current_digest,
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    let applied = json(&applied);
    let graph = &applied["data"]["operations"][0]["packageGraph"];
    assert_eq!(graph["root"]["receipt"]["version"], NEXT_VERSION);
    assert_eq!(graph["replacedPackages"][0], "a3s/science");
    let applied_registry: ResolvedRemotePackage =
        serde_json::from_value(graph["root"]["receipt"]["registry"].clone()).unwrap();
    assert_eq!(applied_registry, package);
    assert_eq!(
        applied_registry.plan_digest().unwrap(),
        registry_plan_digest
    );
    assert_eq!(target_request_count(&server), 1);
    assert!(
        !install_log.exists(),
        "reviewed upgrade must not invoke a child A3S Use mutation"
    );

    make_installed_use_fixture(&use_bin, &install_log, &package);
    server.clear_requests();
    let converged = run(
        &temp,
        &config,
        &use_bin,
        &["upgrade", "use/a3s/science", "--dry-run"],
    );
    assert!(converged.status.success(), "{converged:?}");
    let converged = json(&converged);
    assert_eq!(converged["data"]["plans"][0]["mutates"], false);
    assert_eq!(converged["data"]["plans"][0]["force"], false);
    assert_no_target_request(&server);
    assert!(!install_log.exists());

    let downgrade = TestRepository::with_package_version(
        extension_archive(PACKAGE_VERSION),
        PACKAGE_VERSION,
        4,
        FUTURE,
    );
    server.replace_routes(downgrade.routes.clone());
    server.clear_requests();
    let rejected = run(
        &temp,
        &config,
        &use_bin,
        &["upgrade", "use/a3s/science", "--dry-run"],
    );
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(json(&rejected)["error"]["message"]
        .as_str()
        .unwrap()
        .contains("attempted to downgrade"));
    assert_no_target_request(&server);
    assert!(!install_log.exists());
}

#[test]
fn registry_refresh_rejects_wrong_roots_and_expired_metadata() {
    let wrong_temp = TempWorkspace::new("registry-wrong-root");
    let repository = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, FUTURE);
    let server = TestServer::start(repository.routes.clone());
    let registry_url = localhost_url(&server);
    let config = wrong_temp.path("config/config.acl");
    let use_bin = wrong_temp.path("use-bin");
    make_use_fixture(&use_bin, &wrong_temp.path("unused.log"));
    add_registry(
        &wrong_temp,
        &config,
        &use_bin,
        &registry_url,
        &format!("sha256:{}", "f".repeat(64)),
    );
    server.clear_requests();
    let wrong = run(
        &wrong_temp,
        &config,
        &use_bin,
        &["registry", "refresh", "localhost"],
    );
    assert!(!wrong.status.success(), "{wrong:?}");
    assert_no_target_request(&server);

    let expired_temp = TempWorkspace::new("registry-expired");
    let expired = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, EXPIRED);
    let expired_server = TestServer::start(expired.routes.clone());
    let expired_url = localhost_url(&expired_server);
    let expired_config = expired_temp.path("config/config.acl");
    let expired_use_bin = expired_temp.path("use-bin");
    make_use_fixture(&expired_use_bin, &expired_temp.path("unused.log"));
    add_registry(
        &expired_temp,
        &expired_config,
        &expired_use_bin,
        &expired_url,
        &format!("sha256:{}", expired.root_sha256),
    );
    expired_server.clear_requests();
    let output = run(
        &expired_temp,
        &expired_config,
        &expired_use_bin,
        &["registry", "refresh", "localhost"],
    );
    assert!(!output.status.success(), "{output:?}");
    assert_no_target_request(&expired_server);
}

#[test]
fn registry_disable_and_stable_name_replace_control_network_resolution() {
    let temp = TempWorkspace::new("registry-source-controls");
    let first_repository = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, FUTURE);
    let first_server = TestServer::start(first_repository.routes.clone());
    let first_url = localhost_url(&first_server);
    let second_repository = TestRepository::new(extension_archive(PACKAGE_VERSION), 2, FUTURE);
    let second_server = TestServer::start(second_repository.routes.clone());
    let second_url = localhost_url(&second_server);
    let config = temp.path("config/config.acl");
    let use_bin = temp.path("use-bin");
    make_use_fixture(&use_bin, &temp.path("unused.log"));
    add_registry(
        &temp,
        &config,
        &use_bin,
        &first_url,
        &format!("sha256:{}", first_repository.root_sha256),
    );
    let added_revision = registry_revision(&temp, &config, &use_bin);

    let disabled = run(
        &temp,
        &config,
        &use_bin,
        &[
            "registry",
            "disable",
            "localhost",
            "--revision",
            &added_revision,
            "--yes",
        ],
    );
    assert!(disabled.status.success(), "{disabled:?}");
    let disabled = json(&disabled);
    assert_eq!(
        disabled["data"]["registrySources"]["snapshot"]["sources"][0]["enabled"],
        false
    );
    let disabled_revision = disabled["data"]["registrySources"]["snapshot"]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    first_server.clear_requests();
    let rejected = run(
        &temp,
        &config,
        &use_bin,
        &["install", "use/a3s/science", "--dry-run"],
    );
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(json(&rejected)["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("No default Registry source")));
    assert!(first_server.requests().is_empty());

    let enabled = run(
        &temp,
        &config,
        &use_bin,
        &[
            "registry",
            "enable",
            "localhost",
            "--revision",
            &disabled_revision,
            "--yes",
        ],
    );
    assert!(enabled.status.success(), "{enabled:?}");
    let enabled = json(&enabled);
    let enabled_revision = enabled["data"]["registrySources"]["snapshot"]["revision"]
        .as_str()
        .unwrap()
        .to_string();
    let replaced = run(
        &temp,
        &config,
        &use_bin,
        &[
            "registry",
            "replace",
            "localhost",
            &second_url,
            "--root-sha256",
            &second_repository.root_sha256,
            "--revision",
            &enabled_revision,
            "--yes",
        ],
    );
    assert!(replaced.status.success(), "{replaced:?}");
    let replaced = json(&replaced);
    let source = replaced["data"]["registrySources"]["snapshot"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["name"] == "localhost")
        .unwrap();
    assert_eq!(source["name"], "localhost");
    assert_eq!(source["registryUrl"], second_url);
    first_server.clear_requests();
    second_server.clear_requests();

    let refreshed = run(
        &temp,
        &config,
        &use_bin,
        &["registry", "refresh", "localhost"],
    );
    assert!(refreshed.status.success(), "{refreshed:?}");
    assert!(first_server.requests().is_empty());
    assert!(!second_server.requests().is_empty());
    assert_no_target_request(&second_server);
}

#[test]
#[ignore = "requires A3S_USE_E2E_BIN pointing to a real a3s-use binary"]
fn full_stack_registry_install_and_upgrade_activate_only_reviewed_targets() {
    const NEXT_VERSION: &str = "0.2.0";

    let use_executable = std::path::PathBuf::from(
        std::env::var_os("A3S_USE_E2E_BIN")
            .expect("A3S_USE_E2E_BIN must point to the real a3s-use binary"),
    );
    assert!(use_executable.is_file(), "{}", use_executable.display());
    let use_bin = use_executable.parent().unwrap();
    let temp = TempWorkspace::new("signed-registry-full-stack");
    let repository = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, FUTURE);
    let server = TestServer::start(repository.routes.clone());
    let registry_url = localhost_url(&server);
    let config = temp.path("config/config.acl");
    add_registry(
        &temp,
        &config,
        use_bin,
        &registry_url,
        &format!("sha256:{}", repository.root_sha256),
    );

    server.clear_requests();
    let plan = run(
        &temp,
        &config,
        use_bin,
        &["install", "use/a3s/science", "--dry-run"],
    );
    assert!(plan.status.success(), "{plan:?}");
    let plan = json(&plan);
    let digest = plan["data"]["planDigest"].as_str().unwrap();
    assert_no_target_request(&server);

    let installed = run(
        &temp,
        &config,
        use_bin,
        &["install", "use/a3s/science", "--plan-digest", digest],
    );
    assert!(installed.status.success(), "{installed:?}");
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|request| request.starts_with("/targets/"))
            .count(),
        1
    );
    let receipt: serde_json::Value = serde_json::from_slice(
        &std::fs::read(temp.path("state/use/extensions/a3s/science.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["trust"], "registry-tuf");
    assert_eq!(receipt["registry"]["sha256"], repository.target_sha256);

    let upgraded_repository = TestRepository::with_package_version(
        extension_archive(NEXT_VERSION),
        NEXT_VERSION,
        2,
        FUTURE,
    );
    assert_eq!(repository.root_sha256, upgraded_repository.root_sha256);
    server.replace_routes(upgraded_repository.routes.clone());
    server.clear_requests();
    let upgrade_plan = run(
        &temp,
        &config,
        use_bin,
        &["upgrade", "use/a3s/science", "--dry-run"],
    );
    assert!(upgrade_plan.status.success(), "{upgrade_plan:?}");
    let upgrade_plan = json(&upgrade_plan);
    let upgrade_digest = upgrade_plan["data"]["planDigest"].as_str().unwrap();
    assert_eq!(upgrade_plan["data"]["plans"][0]["action"], "upgrade");
    assert_eq!(
        upgrade_plan["data"]["plans"][0]["resolvedRegistryPackages"]["use/a3s/science"]["version"],
        NEXT_VERSION
    );
    assert_no_target_request(&server);

    let upgraded = run(
        &temp,
        &config,
        use_bin,
        &[
            "upgrade",
            "use/a3s/science",
            "--plan-digest",
            upgrade_digest,
        ],
    );
    assert!(upgraded.status.success(), "{upgraded:?}");
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|request| request.starts_with("/targets/"))
            .count(),
        1
    );
    let upgraded_receipt: serde_json::Value = serde_json::from_slice(
        &std::fs::read(temp.path("state/use/extensions/a3s/science.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(upgraded_receipt["version"], NEXT_VERSION);
    assert_eq!(
        upgraded_receipt["registry"]["sha256"],
        upgraded_repository.target_sha256
    );

    server.clear_requests();
    let converged_plan = run(
        &temp,
        &config,
        use_bin,
        &["upgrade", "use/a3s/science", "--dry-run"],
    );
    assert!(converged_plan.status.success(), "{converged_plan:?}");
    let converged_plan = json(&converged_plan);
    assert_eq!(converged_plan["data"]["plans"][0]["mutates"], false);
    let converged_digest = converged_plan["data"]["planDigest"].as_str().unwrap();
    assert_no_target_request(&server);
    let converged = run(
        &temp,
        &config,
        use_bin,
        &[
            "upgrade",
            "use/a3s/science",
            "--plan-digest",
            converged_digest,
        ],
    );
    assert!(converged.status.success(), "{converged:?}");
    assert_eq!(converged_plan["data"]["plans"][0]["force"], false);
    assert_no_target_request(&server);
}

fn add_registry(
    temp: &TempWorkspace,
    config: &std::path::Path,
    use_bin: &std::path::Path,
    url: &str,
    trust_root: &str,
) {
    if !config.exists() {
        std::fs::create_dir_all(config.parent().expect("config parent")).unwrap();
        std::fs::write(config, "").unwrap();
    }
    let trust_root = trust_root.strip_prefix("sha256:").unwrap_or(trust_root);
    let output = run(
        temp,
        config,
        use_bin,
        &[
            "registry",
            "add",
            "localhost",
            url,
            "--root-sha256",
            trust_root,
            "--yes",
        ],
    );
    assert!(output.status.success(), "{output:?}");
}

fn registry_revision(
    temp: &TempWorkspace,
    config: &std::path::Path,
    use_bin: &std::path::Path,
) -> String {
    let output = run(temp, config, use_bin, &["registry", "list"]);
    assert!(output.status.success(), "{output:?}");
    json(&output)["data"]["registrySources"]["revision"]
        .as_str()
        .unwrap()
        .to_string()
}

fn run(
    temp: &TempWorkspace,
    config: &std::path::Path,
    use_bin: &std::path::Path,
    args: &[&str],
) -> Output {
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, temp);
    command
        .arg("--config")
        .arg(config)
        .args(["--output", "json"])
        .args(args)
        .env("A3S_USE_INSTALL_DIR", use_bin)
        .output()
        .unwrap()
}

fn make_use_fixture(directory: &std::path::Path, install_log: &std::path::Path) {
    let registry = directory
        .parent()
        .expect("A3S Use fixture parent")
        .join("state/use/registry.json");
    let empty_snapshot = directory.join("capability-empty.json");
    let installed_snapshot = directory.join("capability-installed.json");
    write_capability_snapshot(&empty_snapshot, 0, '0');
    write_capability_snapshot(&installed_snapshot, 1, '1');
    make_executable(
        &directory.join("a3s-use"),
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'a3s-use 0.3.0\\n'; exit 0; fi\nif [ \"$1\" = \"capability\" ] && [ \"$2\" = \"snapshot\" ]; then if [ -f {} ]; then /bin/cat {}; else /bin/cat {}; fi; exit 0; fi\nif [ \"$1\" = \"component\" ] && [ \"$2\" = \"list\" ]; then printf '{{\"schemaVersion\":1,\"ok\":true,\"data\":{{\"components\":[]}}}}\\n'; exit 0; fi\nif [ \"$1\" = \"component\" ] && [ \"$2\" = \"status\" ]; then printf '{{\"schemaVersion\":1,\"ok\":true,\"data\":{{\"component\":{{\"id\":\"%s\",\"presence\":\"missing\",\"health\":\"unknown\"}}}}}}\\n' \"$3\"; exit 0; fi\nif [ \"$1\" = \"component\" ] && {{ [ \"$2\" = \"install\" ] || [ \"$2\" = \"upgrade\" ] || [ \"$2\" = \"uninstall\" ]; }}; then printf '%s\\n' \"$@\" > {}; fi\nexit 2\n",
            sh_quote(&registry),
            sh_quote(&installed_snapshot),
            sh_quote(&empty_snapshot),
            sh_quote(install_log),
        ),
    );
}

fn make_installed_use_fixture(
    directory: &std::path::Path,
    install_log: &std::path::Path,
    installed: &ResolvedRemotePackage,
) {
    let component = serde_json::json!({
        "id": "a3s/science",
        "description": "Installed signed science extension",
        "presence": "managed",
        "health": "ready",
        "version": installed.version,
        "path": "/tmp/a3s-use-science",
        "trust": "registry-tuf",
        "registry": installed
    });
    let list_path = directory.join("component-list.json");
    let status_path = directory.join("component-status.json");
    std::fs::write(
        &list_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {"components": [component.clone()]}
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        &status_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {"component": component}
        }))
        .unwrap(),
    )
    .unwrap();
    make_executable(
        &directory.join("a3s-use"),
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'a3s-use 0.3.0\\n'; exit 0; fi\nif [ \"$1\" = \"component\" ] && [ \"$2\" = \"list\" ]; then /bin/cat {}; exit 0; fi\nif [ \"$1\" = \"component\" ] && [ \"$2\" = \"status\" ]; then /bin/cat {}; exit 0; fi\nif [ \"$1\" = \"component\" ] && {{ [ \"$2\" = \"install\" ] || [ \"$2\" = \"upgrade\" ] || [ \"$2\" = \"uninstall\" ]; }}; then printf '%s\\n' \"$@\" > {}; fi\nexit 2\n",
            sh_quote(&list_path),
            sh_quote(&status_path),
            sh_quote(install_log)
        ),
    );
}

fn write_capability_snapshot(path: &std::path::Path, generation: u64, revision_digit: char) {
    std::fs::create_dir_all(path.parent().expect("capability snapshot parent")).unwrap();
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "registry": {
                    "schemaVersion": 1,
                    "generation": generation,
                    "revision": revision_digit.to_string().repeat(64),
                    "capabilities": [],
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

fn localhost_url(server: &TestServer) -> String {
    server.base_url().replacen("127.0.0.1", "localhost", 1)
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON ({error}): stdout={:?}, stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn assert_no_target_request(server: &TestServer) {
    assert_eq!(target_request_count(server), 0);
}

fn target_request_count(server: &TestServer) -> usize {
    server
        .requests()
        .iter()
        .filter(|request| request.starts_with("/targets/"))
        .count()
}

fn write_mcp_tool_call(stdin: &mut impl Write, id: u64, name: &str, arguments: serde_json::Value) {
    write_mcp_message(
        stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }),
    );
}

fn write_mcp_message(stdin: &mut impl Write, value: serde_json::Value) {
    serde_json::to_writer(&mut *stdin, &value).expect("write Plugin Manager MCP message");
    stdin
        .write_all(b"\n")
        .expect("terminate Plugin Manager MCP message");
    stdin.flush().expect("flush Plugin Manager MCP message");
}

fn read_mcp_response(reader: &mut impl BufRead, id: u64) -> serde_json::Value {
    for _ in 0..20 {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .expect("read Plugin Manager MCP response");
        assert!(
            bytes > 0,
            "Plugin Manager MCP closed before responding to request {id}"
        );
        let value: serde_json::Value = serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!("invalid Plugin Manager MCP response ({error}): {line}")
        });
        if value["id"].as_u64() == Some(id) {
            return value;
        }
    }
    panic!("Plugin Manager MCP did not respond to request {id}");
}

struct McpChildGuard(Child);

impl Drop for McpChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
