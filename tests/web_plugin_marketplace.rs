#![cfg(unix)]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use a3s_use_core::{InstalledPluginPlanEvidence, INSTALLED_PLUGIN_PLAN_EVIDENCE_SCHEMA};
use a3s_use_extension::{ExtensionPaths, ExtensionRegistry};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use support::{a3s_bin, configure_component_env, make_executable, sh_quote, TempWorkspace};

#[path = "support/tuf_test_support.rs"]
mod tuf_test_support;

use tuf_test_support::{
    extension_archive, extension_target, host_target, package_directory_archive, TestRepository,
    TestServer, TestTarget, FUTURE, PACKAGE_VERSION,
};

const UPGRADED_PACKAGE_VERSION: &str = "0.1.2";

static WEB_PROCESS_TEST_LOCK: Mutex<()> = Mutex::new(());
const TEST_WEB_WORKER_STACK_BYTES: &str = "2097152";

fn web_process_test_guard() -> MutexGuard<'static, ()> {
    WEB_PROCESS_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[path = "web_plugin_marketplace/generic_real_e2e.rs"]
mod generic_real_e2e;
#[path = "web_plugin_marketplace/real_e2e.rs"]
mod real_e2e;
#[path = "web_plugin_marketplace/reviewed_enablement.rs"]
mod reviewed_enablement;

#[test]
fn marketplace_install_upgrade_uninstall_hot_plugs_verified_activity_skill_and_flow_catalog() {
    let _guard = web_process_test_guard();
    let temp = TempWorkspace::new("web-plugin-marketplace");
    let workspace = temp.path("workspace");
    let web_dir = temp.path("web");
    let config = temp.path("config/config.acl");
    let use_bin = temp.path("use-bin");
    let package_root = temp.path("managed-package-v1");
    let upgraded_package_root = temp.path("managed-package-v2");
    let extension_registry = temp.path("state/use/registry.json");
    let session_state = temp.path("web-session-state");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&web_dir).expect("create Web assets");
    fs::create_dir_all(config.parent().expect("config parent")).expect("create config parent");
    fs::write(
        web_dir.join("index.html"),
        "<!doctype html><title>A3S plugin Marketplace integration</title>",
    )
    .expect("write Web fixture");
    fs::write(&config, test_config()).expect("write config fixture");

    let activity_html =
        "<!doctype html><title>Science</title><main>Installed Marketplace Activity</main>";
    let activity_css = "main { color: rebeccapurple; }";
    let activity_js = r#"window.addEventListener('message', (event) => {
  const port = event.ports[0];
  if (event.source !== window.parent || event.data?.protocol !== 'a3s.activity.v3' || event.data.type !== 'host.init' || !port) return;
  port.start();
  port.postMessage({ protocol: 'a3s.activity.v3', type: 'activity.ready' });
});"#;
    let skill = "---\nname: science\ndescription: Use the installed Science extension.\n---\n# Science\n\nUse the verified Science capability.\n";
    let activity_path = package_root.join("web/activity.html");
    let activity_style_path = package_root.join("web/activity.css");
    let activity_script_path = package_root.join("web/activity.js");
    let skill_path = package_root.join("skills/science/SKILL.md");
    let flow_source =
        "export async function run(input: unknown): Promise<unknown> { return input; }\n";
    let flow_path = package_root.join("flows/research.ts");
    fs::create_dir_all(activity_path.parent().expect("activity parent"))
        .expect("create activity directory");
    fs::create_dir_all(skill_path.parent().expect("skill parent")).expect("create skill directory");
    fs::create_dir_all(flow_path.parent().expect("Flow parent")).expect("create Flow directory");
    fs::write(&activity_path, activity_html).expect("write activity asset");
    fs::write(&activity_style_path, activity_css).expect("write activity style");
    fs::write(&activity_script_path, activity_js).expect("write activity script");
    fs::write(&skill_path, skill).expect("write Skill asset");
    fs::write(&flow_path, flow_source).expect("write A3S Flow source");

    let upgraded_activity_html =
        "<!doctype html><title>Science 2</title><main>Upgraded Marketplace Activity</main>";
    let upgraded_activity_css = "main { color: seagreen; }";
    let upgraded_activity_js = r#"window.addEventListener('message', (event) => {
  const port = event.ports[0];
  if (event.source !== window.parent || event.data?.protocol !== 'a3s.activity.v3' || event.data.type !== 'host.init' || !port) return;
  port.start();
  port.postMessage({ protocol: 'a3s.activity.v3', type: 'activity.ready', version: 2 });
});"#;
    let upgraded_skill = "---\nname: science\ndescription: Use the upgraded Science extension.\n---\n# Science\n\nUse the verified upgraded Science capability.\n";
    let upgraded_flow_source = "export async function runV2(input: unknown): Promise<unknown> { return { version: 2, input }; }\n";
    let upgraded_activity_path = upgraded_package_root.join("web/activity.html");
    let upgraded_activity_style_path = upgraded_package_root.join("web/activity.css");
    let upgraded_activity_script_path = upgraded_package_root.join("web/activity.js");
    let upgraded_skill_path = upgraded_package_root.join("skills/science/SKILL.md");
    let upgraded_flow_path = upgraded_package_root.join("flows/research.ts");
    fs::create_dir_all(
        upgraded_activity_path
            .parent()
            .expect("upgraded activity parent"),
    )
    .expect("create upgraded activity directory");
    fs::create_dir_all(upgraded_skill_path.parent().expect("upgraded Skill parent"))
        .expect("create upgraded Skill directory");
    fs::create_dir_all(upgraded_flow_path.parent().expect("upgraded Flow parent"))
        .expect("create upgraded Flow directory");
    fs::write(&upgraded_activity_path, upgraded_activity_html)
        .expect("write upgraded activity asset");
    fs::write(&upgraded_activity_style_path, upgraded_activity_css)
        .expect("write upgraded activity style");
    fs::write(&upgraded_activity_script_path, upgraded_activity_js)
        .expect("write upgraded activity script");
    fs::write(&upgraded_skill_path, upgraded_skill).expect("write upgraded Skill asset");
    fs::write(&upgraded_flow_path, upgraded_flow_source).expect("write upgraded A3S Flow source");

    let empty_snapshot = snapshot_envelope(0, "0", Vec::new());
    let installed_capability = json!({
        "id": "use/a3s/science",
        "route": "science",
        "version": PACKAGE_VERSION,
        "origin": "extension",
        "enabled": true,
        "readiness": "ready",
        "packageRoot": package_root,
        "lifecycleGeneration": 1,
        "surfaces": ["flow", "skill"],
        "skills": [{
            "path": skill_path,
            "sha256": sha256(skill.as_bytes()),
        }],
        "flows": [{
            "id": "research",
            "engine": "a3s-flow",
            "runtime": "native-ts",
            "source": {
                "path": flow_path,
                "sha256": sha256(flow_source.as_bytes()),
                "mediaType": "text/typescript",
            },
            "exportName": "run",
            "requiresTools": ["collect"],
            "requiresMcp": ["papers"],
            "requiresOkf": ["research-domain"],
        }],
        "activityBar": [{
            "id": "research",
            "title": "科研",
            "description": "Prepare evidence-backed research with the installed Science capability.",
            "icon": "flask-conical",
            "entry": {
                "path": activity_path,
                "sha256": sha256(activity_html.as_bytes()),
                "mediaType": "text/html",
            },
            "styles": [{
                "path": activity_style_path,
                "sha256": sha256(activity_css.as_bytes()),
                "mediaType": "text/css",
            }],
            "scripts": [{
                "path": activity_script_path,
                "sha256": sha256(activity_js.as_bytes()),
                "mediaType": "text/javascript",
            }],
            "skill": "science",
            "order": 80,
        }],
    });
    let installed_snapshot = snapshot_envelope(1, "1", vec![installed_capability]);
    let upgraded_capability = json!({
        "id": "use/a3s/science",
        "route": "science",
        "version": UPGRADED_PACKAGE_VERSION,
        "origin": "extension",
        "enabled": true,
        "readiness": "ready",
        "packageRoot": upgraded_package_root,
        "lifecycleGeneration": 2,
        "surfaces": ["flow", "skill"],
        "skills": [{
            "path": upgraded_skill_path,
            "sha256": sha256(upgraded_skill.as_bytes()),
        }],
        "flows": [{
            "id": "research",
            "engine": "a3s-flow",
            "runtime": "native-ts",
            "source": {
                "path": upgraded_flow_path,
                "sha256": sha256(upgraded_flow_source.as_bytes()),
                "mediaType": "text/typescript",
            },
            "exportName": "runV2",
            "requiresTools": ["collect", "synthesize"],
            "requiresMcp": ["papers-v2"],
            "requiresOkf": ["research-domain-v2"],
        }],
        "activityBar": [{
            "id": "research",
            "title": "科研 2",
            "description": "Prepare upgraded evidence-backed research with Science.",
            "icon": "flask-conical",
            "entry": {
                "path": upgraded_activity_path,
                "sha256": sha256(upgraded_activity_html.as_bytes()),
                "mediaType": "text/html",
            },
            "styles": [{
                "path": upgraded_activity_style_path,
                "sha256": sha256(upgraded_activity_css.as_bytes()),
                "mediaType": "text/css",
            }],
            "scripts": [{
                "path": upgraded_activity_script_path,
                "sha256": sha256(upgraded_activity_js.as_bytes()),
                "mediaType": "text/javascript",
            }],
            "skill": "science",
            "order": 80,
        }],
    });
    let upgraded_snapshot = snapshot_envelope(2, "2", vec![upgraded_capability]);
    let empty_snapshot_path = temp.path("empty-snapshot.json");
    let installed_snapshot_path = temp.path("installed-snapshot.json");
    let changed_snapshot_path = temp.path("changed-snapshot.json");
    let upgraded_snapshot_path = temp.path("upgraded-snapshot.json");
    let upgraded_changed_snapshot_path = temp.path("upgraded-changed-snapshot.json");
    let unchanged_snapshot_path = temp.path("unchanged-snapshot.json");
    let removed_snapshot_path = temp.path("removed-snapshot.json");
    fs::write(
        &empty_snapshot_path,
        serde_json::to_vec(&empty_snapshot).unwrap(),
    )
    .expect("write empty snapshot");
    fs::write(
        &installed_snapshot_path,
        serde_json::to_vec(&installed_snapshot).unwrap(),
    )
    .expect("write installed snapshot");
    fs::write(
        &changed_snapshot_path,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {"changed": true, "registry": installed_snapshot["data"]["registry"]},
        }))
        .unwrap(),
    )
    .expect("write changed snapshot");
    fs::write(
        &upgraded_snapshot_path,
        serde_json::to_vec(&upgraded_snapshot).unwrap(),
    )
    .expect("write upgraded snapshot");
    fs::write(
        &upgraded_changed_snapshot_path,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {"changed": true, "registry": upgraded_snapshot["data"]["registry"]},
        }))
        .unwrap(),
    )
    .expect("write upgraded changed snapshot");
    fs::write(
        &unchanged_snapshot_path,
        br#"{"schemaVersion":1,"ok":true,"data":{"changed":false}}"#,
    )
    .expect("write unchanged snapshot");
    fs::write(
        &removed_snapshot_path,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "changed": true,
                "registry": snapshot_envelope(3, "3", Vec::new())["data"]["registry"],
            },
        }))
        .unwrap(),
    )
    .expect("write removed snapshot");
    let repository = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, FUTURE);
    let upgraded_repository = TestRepository::with_targets(
        vec![
            extension_target(extension_archive(PACKAGE_VERSION), PACKAGE_VERSION),
            extension_target(
                extension_archive(UPGRADED_PACKAGE_VERSION),
                UPGRADED_PACKAGE_VERSION,
            ),
        ],
        2,
        FUTURE,
    );
    assert_eq!(
        repository.root_sha256, upgraded_repository.root_sha256,
        "metadata rotation must retain the enrolled TUF root"
    );
    let registry_server = TestServer::start(repository.routes.clone());
    let registry_url = registry_server
        .base_url()
        .replacen("127.0.0.1", "localhost", 1);
    let installed_registry =
        resolved_registry_provenance(PACKAGE_VERSION, &registry_url, &repository.root_sha256, 1);
    let upgraded_registry = resolved_registry_provenance(
        UPGRADED_PACKAGE_VERSION,
        &registry_url,
        &upgraded_repository.root_sha256,
        2,
    );
    make_use_fixture(
        &use_bin,
        &extension_registry,
        &package_root,
        UseFixtureSnapshots {
            empty: &empty_snapshot_path,
            installed: &installed_snapshot_path,
            changed: &changed_snapshot_path,
            upgraded: &upgraded_snapshot_path,
            upgraded_changed: &upgraded_changed_snapshot_path,
            removed: &removed_snapshot_path,
            unchanged: &unchanged_snapshot_path,
        },
        &upgraded_package_root,
        &installed_registry,
        &upgraded_registry,
    );
    enroll_registry(
        &temp,
        &config,
        &use_bin,
        &registry_url,
        &repository.root_sha256,
    );

    let (mut daemon, address) = start_web(
        &temp,
        &workspace,
        &web_dir,
        &config,
        &use_bin,
        &session_state,
    );

    let initial_activities = http_json(&address, "GET", "/api/v1/plugins/activities", None);
    assert_eq!(initial_activities["items"], json!([]));
    let initial_flows = http_json(&address, "GET", "/api/v1/plugins/flows", None);
    assert_eq!(initial_flows["items"], json!([]));

    let marketplace = http_json(&address, "GET", "/api/v1/plugins/marketplace", None);
    let item = marketplace["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["componentId"] == "use/a3s/science")
        })
        .unwrap_or_else(|| panic!("signed Marketplace package: {marketplace:#}"));
    assert_eq!(item["installed"], false);
    assert_eq!(item["displayName"], "A3S Science");
    assert_eq!(item["sha256"], repository.target_sha256);

    let plan = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/plan",
        Some(&json!({
            "action": "install",
            "componentId": "use/a3s/science",
            "version": PACKAGE_VERSION,
            "channel": "stable",
        })),
    );
    assert_eq!(plan["dryRun"], true);
    let (operation_id, digest) = reviewed_identity(&plan);
    assert!(
        !extension_registry.exists(),
        "planning must not install the package"
    );

    let applied = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "operationId": operation_id,
            "planDigest": digest,
        })),
    );
    assert!(applied["operations"]
        .as_array()
        .is_some_and(|operations| operations
            .iter()
            .any(|operation| operation["changed"] == true)));
    assert_eq!(extension_registry_generation(&extension_registry), 1);
    publish_planning_evidence(
        &temp,
        "a3s/science",
        PACKAGE_VERSION,
        1,
        &installed_snapshot_path,
        &changed_snapshot_path,
        &use_bin,
    );

    let activities = wait_for_activity(&address, "science:research");
    let activity = activities["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == "science:research"))
        .expect("hot-plugged Activity Bar contribution");
    assert_eq!(activity["packageId"], "use/a3s/science");
    assert_eq!(activity["skill"], "science");
    assert_eq!(activity["enabled"], true);
    let installed_generation = activities["generation"]
        .as_u64()
        .expect("installed registry generation");
    let installed_document_url = activity_document_url(&activities, "science:research");
    let flows = wait_for_flow(&address, "science:research");
    let flow = flows["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == "science:research"))
        .expect("hot-plugged A3S Flow contribution");
    assert_eq!(flow["packageId"], "use/a3s/science");
    assert_eq!(flow["engine"], "a3s-flow");
    assert_eq!(flow["runtime"], "native-ts");
    assert_eq!(flow["lifecycleGeneration"], 1);
    assert_eq!(flow["exportName"], "run");
    assert_eq!(flow["sha256"], sha256(flow_source.as_bytes()));
    assert!(flow.get("sourcePath").is_none());
    assert_eq!(flow["requiresTools"], json!(["collect"]));
    assert_eq!(flow["requiresMcp"], json!(["papers"]));
    assert_eq!(flow["requiresOkf"], json!(["research-domain"]));
    let installed_design = bound_flow_design(PACKAGE_VERSION, 1, &sha256(flow_source.as_bytes()));
    let resolved_flow = http_json(
        &address,
        "POST",
        "/api/v1/plugins/flows/resolve",
        Some(&json!({"designJson": installed_design.clone()})),
    );
    assert_eq!(resolved_flow["schemaVersion"], 1);
    assert_eq!(resolved_flow["flow"]["packageId"], "use/a3s/science");
    assert_eq!(resolved_flow["flow"]["flowId"], "research");
    assert_eq!(resolved_flow["flow"]["lifecycleGeneration"], 1);
    assert_eq!(
        resolved_flow["flow"]["catalogGeneration"],
        installed_generation
    );
    assert!(resolved_flow["flow"].get("sourcePath").is_none());

    let installed_run = http_json(
        &address,
        "POST",
        "/api/v1/plugins/flows/run",
        Some(&json!({
            "designJson": installed_design.clone(),
            "runId": "installed-web-run",
            "input": {"topic": "durability"},
        })),
    );
    assert_eq!(installed_run["run"]["runId"], "installed-web-run");
    assert_eq!(installed_run["run"]["status"], "completed");
    assert_eq!(installed_run["run"]["output"]["marker"], "installed");
    assert_eq!(installed_run["run"]["eventCount"], 3);
    assert_path_free(&installed_run, &[&package_root, &flow_path, &workspace]);
    let installed_events = http_json(
        &address,
        "GET",
        "/api/v1/plugins/flows/runs/installed-web-run/events",
        None,
    );
    assert_eq!(installed_events["items"].as_array().unwrap().len(), 3);
    assert_eq!(installed_events["items"][0]["key"], "flow.run.created");
    assert!(installed_events["items"][0]["event"]
        .pointer("/spec/runtime/entrypoint")
        .is_none());
    assert_eq!(
        installed_events["items"][0]["event"]["spec"]["runtime"]["sourceSha256"],
        sha256(flow_source.as_bytes())
    );
    assert_path_free(&installed_events, &[&package_root, &flow_path, &workspace]);
    let idempotent_run = http_json(
        &address,
        "POST",
        "/api/v1/plugins/flows/run",
        Some(&json!({
            "designJson": installed_design.clone(),
            "runId": "installed-web-run",
            "input": {"topic": "durability"},
        })),
    );
    assert_eq!(idempotent_run["run"]["eventCount"], 3);

    let compile_log = temp.path("flow-native-compiler.log");
    let compiled_before_drift = fs::read_to_string(&compile_log)
        .unwrap_or_default()
        .lines()
        .count();
    fs::write(
        &flow_path,
        "export async function substituted() { return 'drift'; }\n",
    )
    .expect("substitute installed Flow source");
    let drifted_run = http_json_status(
        &address,
        "POST",
        "/api/v1/plugins/flows/run",
        Some(&json!({
            "designJson": installed_design.clone(),
            "runId": "drifted-web-run",
        })),
        "409",
    );
    assert!(drifted_run["message"]
        .as_str()
        .is_some_and(|message| message.contains("source verification failed")));
    assert_eq!(
        fs::read_to_string(&compile_log)
            .unwrap_or_default()
            .lines()
            .count(),
        compiled_before_drift,
        "source drift reached the compiler"
    );
    let absent_drifted_run = http_json_status(
        &address,
        "GET",
        "/api/v1/plugins/flows/runs/drifted-web-run",
        None,
        "404",
    );
    assert!(absent_drifted_run["message"]
        .as_str()
        .is_some_and(|message| message.contains("not found")));
    fs::write(&flow_path, flow_source).expect("restore installed Flow source");

    let content = http_json(
        &address,
        "GET",
        "/api/v1/plugins/activities/science%3Aresearch",
        None,
    );
    assert_eq!(content["html"], activity_html);
    assert_eq!(content["styles"], json!([activity_css]));
    assert_eq!(content["scripts"], json!([activity_js]));
    assert_eq!(content["sha256"], sha256(activity_html.as_bytes()));
    assert_eq!(content["skill"], "science");
    assert_activity_document(
        &address,
        &installed_document_url,
        "Installed Marketplace Activity",
        activity_css,
        activity_js,
        &[&package_root, &workspace],
    );

    let installed_marketplace = http_json(&address, "GET", "/api/v1/plugins/marketplace", None);
    assert!(installed_marketplace["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["componentId"] == "use/a3s/science")
        })
        .is_some_and(|item| item["installed"] == true && item["enabled"] == true));

    registry_server.replace_routes(upgraded_repository.routes.clone());
    let upgrade_plan = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/plan",
        Some(&json!({
            "action": "upgrade",
            "componentId": "use/a3s/science",
        })),
    );
    assert_eq!(upgrade_plan["dryRun"], true);
    let (upgrade_operation_id, upgrade_digest) = reviewed_identity(&upgrade_plan);
    assert_eq!(
        extension_registry_generation(&extension_registry),
        1,
        "upgrade planning must not mutate the installed generation"
    );

    let upgraded = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "operationId": upgrade_operation_id,
            "planDigest": upgrade_digest,
        })),
    );
    assert!(upgraded["operations"]
        .as_array()
        .is_some_and(|operations| operations
            .iter()
            .any(|operation| operation["changed"] == true)));
    assert_eq!(extension_registry_generation(&extension_registry), 2);
    publish_planning_evidence(
        &temp,
        "a3s/science",
        UPGRADED_PACKAGE_VERSION,
        2,
        &upgraded_snapshot_path,
        &upgraded_changed_snapshot_path,
        &use_bin,
    );

    let upgraded_activities =
        wait_for_activity_after(&address, "science:research", installed_generation);
    let upgraded_generation = upgraded_activities["generation"]
        .as_u64()
        .expect("upgraded registry generation");
    let upgraded_activity = upgraded_activities["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == "science:research"))
        .expect("upgraded Activity Bar contribution");
    assert_eq!(upgraded_activity["title"], "科研 2");
    assert_eq!(upgraded_activity["packageId"], "use/a3s/science");
    let upgraded_document_url = activity_document_url(&upgraded_activities, "science:research");
    assert_ne!(upgraded_document_url, installed_document_url);
    assert_http_status(&address, &installed_document_url, 410);

    let upgraded_flows = wait_for_flow_after(&address, "science:research", installed_generation);
    assert_eq!(upgraded_flows["generation"], upgraded_generation);
    let upgraded_flow = upgraded_flows["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == "science:research"))
        .expect("upgraded A3S Flow contribution");
    assert_eq!(upgraded_flow["lifecycleGeneration"], 2);
    assert_eq!(upgraded_flow["exportName"], "runV2");
    assert_eq!(
        upgraded_flow["sha256"],
        sha256(upgraded_flow_source.as_bytes())
    );
    assert_eq!(
        upgraded_flow["requiresTools"],
        json!(["collect", "synthesize"])
    );
    assert_eq!(upgraded_flow["requiresMcp"], json!(["papers-v2"]));
    assert_eq!(upgraded_flow["requiresOkf"], json!(["research-domain-v2"]));
    assert_ne!(upgraded_flow["sha256"], flow["sha256"]);

    let stale_resolution = http_json_status(
        &address,
        "POST",
        "/api/v1/plugins/flows/resolve",
        Some(&json!({"designJson": installed_design})),
        "409",
    );
    assert!(
        stale_resolution["message"]
            .as_str()
            .is_some_and(|message| message.contains("version mismatch")),
        "{stale_resolution:#}"
    );
    let upgraded_design = bound_flow_design(
        UPGRADED_PACKAGE_VERSION,
        2,
        &sha256(upgraded_flow_source.as_bytes()),
    );
    let upgraded_resolution = http_json(
        &address,
        "POST",
        "/api/v1/plugins/flows/resolve",
        Some(&json!({"designJson": upgraded_design.clone()})),
    );
    assert_eq!(upgraded_resolution["flow"]["lifecycleGeneration"], 2);
    assert_eq!(upgraded_resolution["flow"]["exportName"], "runV2");

    let old_run_after_upgrade = http_json(
        &address,
        "GET",
        "/api/v1/plugins/flows/runs/installed-web-run",
        None,
    );
    assert_eq!(
        old_run_after_upgrade["run"]["flow"]["version"],
        PACKAGE_VERSION
    );
    assert_eq!(
        old_run_after_upgrade["run"]["flow"]["lifecycleGeneration"],
        1
    );
    let upgraded_run = http_json(
        &address,
        "POST",
        "/api/v1/plugins/flows/run",
        Some(&json!({
            "designJson": upgraded_design.clone(),
            "runId": "upgraded-web-run",
            "input": {"topic": "upgrade-history"},
        })),
    );
    assert_eq!(upgraded_run["run"]["status"], "completed");
    assert_eq!(upgraded_run["run"]["output"]["marker"], "upgraded");
    assert_eq!(upgraded_run["run"]["flow"]["lifecycleGeneration"], 2);
    assert_path_free(
        &upgraded_run,
        &[&upgraded_package_root, &upgraded_flow_path, &workspace],
    );

    let upgraded_content = http_json(
        &address,
        "GET",
        "/api/v1/plugins/activities/science%3Aresearch",
        None,
    );
    assert_eq!(upgraded_content["html"], upgraded_activity_html);
    assert_eq!(upgraded_content["styles"], json!([upgraded_activity_css]));
    assert_eq!(upgraded_content["scripts"], json!([upgraded_activity_js]));
    assert_eq!(
        upgraded_content["sha256"],
        sha256(upgraded_activity_html.as_bytes())
    );
    assert_activity_document(
        &address,
        &upgraded_document_url,
        "Upgraded Marketplace Activity",
        upgraded_activity_css,
        upgraded_activity_js,
        &[&upgraded_package_root, &workspace],
    );

    let upgraded_marketplace = http_json(&address, "GET", "/api/v1/plugins/marketplace", None);
    assert!(upgraded_marketplace["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["componentId"] == "use/a3s/science")
        })
        .is_some_and(|item| {
            item["version"] == UPGRADED_PACKAGE_VERSION
                && item["installed"] == true
                && item["enabled"] == true
        }));

    let uninstall_plan = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/plan",
        Some(&json!({
            "action": "uninstall",
            "componentId": "use/a3s/science",
        })),
    );
    assert_eq!(uninstall_plan["dryRun"], true);
    let (uninstall_operation_id, uninstall_digest) = reviewed_identity(&uninstall_plan);
    let uninstalled = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "operationId": uninstall_operation_id,
            "planDigest": uninstall_digest,
        })),
    );
    assert!(uninstalled["operations"]
        .as_array()
        .is_some_and(|operations| operations
            .iter()
            .any(|operation| operation["changed"] == true)));
    wait_for_activity_absent(&address, "science:research", upgraded_generation);
    wait_for_flow_absent(&address, "science:research", upgraded_generation);
    assert_http_status(&address, &upgraded_document_url, 410);
    let removed_resolution = http_json_status(
        &address,
        "POST",
        "/api/v1/plugins/flows/resolve",
        Some(&json!({"designJson": upgraded_design})),
        "409",
    );
    assert!(
        removed_resolution["message"]
            .as_str()
            .is_some_and(|message| message.contains("not installed or ready")),
        "{removed_resolution:#}"
    );
    let removed_marketplace = http_json(&address, "GET", "/api/v1/plugins/marketplace", None);
    assert!(removed_marketplace["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["componentId"] == "use/a3s/science")
        })
        .is_some_and(|item| item["installed"] == false && item["enabled"] == false));

    let retained_runs = http_json(&address, "GET", "/api/v1/plugins/flows/runs?limit=10", None);
    let retained_ids = retained_runs["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|run| run["runId"].as_str())
        .collect::<Vec<_>>();
    assert!(
        retained_ids.contains(&"installed-web-run"),
        "{retained_runs:#}"
    );
    assert!(
        retained_ids.contains(&"upgraded-web-run"),
        "{retained_runs:#}"
    );
    let retained_old_run = http_json(
        &address,
        "GET",
        "/api/v1/plugins/flows/runs/installed-web-run",
        None,
    );
    let retained_upgraded_events = http_json(
        &address,
        "GET",
        "/api/v1/plugins/flows/runs/upgraded-web-run/events",
        None,
    );
    assert_eq!(retained_old_run["run"]["status"], "completed");
    assert_eq!(
        retained_upgraded_events["items"].as_array().unwrap().len(),
        3
    );
    assert_path_free(
        &retained_runs,
        &[&package_root, &upgraded_package_root, &workspace],
    );

    daemon.stop();
    wait_until_stopped(&address);

    let (mut restarted, restarted_address) = start_web(
        &temp,
        &workspace,
        &web_dir,
        &config,
        &use_bin,
        &session_state,
    );
    let recovered = http_json(
        &restarted_address,
        "GET",
        "/api/v1/plugins/flows/runs/installed-web-run",
        None,
    );
    let recovered_events = http_json(
        &restarted_address,
        "GET",
        "/api/v1/plugins/flows/runs/upgraded-web-run/events",
        None,
    );
    assert_eq!(recovered["run"]["status"], "completed");
    assert_eq!(recovered["run"]["flow"]["version"], PACKAGE_VERSION);
    assert_eq!(recovered_events["items"].as_array().unwrap().len(), 3);
    assert_path_free(
        &recovered_events,
        &[&package_root, &upgraded_package_root, &workspace],
    );
    restarted.stop();
    wait_until_stopped(&restarted_address);
}

fn snapshot_envelope(generation: u64, revision_digit: &str, capabilities: Vec<Value>) -> Value {
    json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {
            "registry": {
                "schemaVersion": 1,
                "generation": generation,
                "revision": revision_digit.repeat(64),
                "capabilities": capabilities,
            }
        }
    })
}

fn reviewed_identity(plan: &Value) -> (String, String) {
    let operation_id = plan["operationId"]
        .as_str()
        .unwrap_or_else(|| panic!("reviewed plan operation ID: {plan:#}"));
    let digest = plan["canonicalPlanDigest"]
        .as_str()
        .unwrap_or_else(|| panic!("reviewed plan digest: {plan:#}"));
    (operation_id.to_string(), digest.to_string())
}

fn resolved_registry_provenance(
    version: &str,
    registry_url: &str,
    root_sha256: &str,
    metadata_version: u64,
) -> Value {
    let target = host_target();
    let archive = extension_archive(version);
    let archive_name = format!("a3s-use-a3s-science-{version}-{target}.tar.gz");
    let target_name = format!("extensions/a3s/science/{version}/stable/{target}/{archive_name}");
    json!({
        "registryName": "localhost",
        "registryUrl": registry_url,
        "rootSha256": root_sha256,
        "rootVersion": 1,
        "timestampVersion": metadata_version,
        "snapshotVersion": metadata_version,
        "targetsVersion": metadata_version,
        "packageId": "a3s/science",
        "version": version,
        "channel": "stable",
        "target": target,
        "targetName": target_name,
        "archiveName": archive_name,
        "length": archive.len(),
        "sha256": sha256(&archive),
    })
}

struct UseFixtureSnapshots<'a> {
    empty: &'a Path,
    installed: &'a Path,
    changed: &'a Path,
    upgraded: &'a Path,
    upgraded_changed: &'a Path,
    removed: &'a Path,
    unchanged: &'a Path,
}

fn make_use_fixture(
    directory: &Path,
    extension_registry: &Path,
    package_root: &Path,
    snapshots: UseFixtureSnapshots<'_>,
    upgraded_package_root: &Path,
    installed_registry: &Value,
    upgraded_registry: &Value,
) {
    let installed_components = directory.join("installed-components.json");
    let upgraded_components = directory.join("upgraded-components.json");
    let installed_status = directory.join("installed-status.json");
    let upgraded_status = directory.join("upgraded-status.json");
    let planning_evidence = directory.join("planning-evidence.json");
    fs::create_dir_all(directory).expect("create A3S Use fixture directory");
    fs::write(
        &installed_components,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "components": [{
                    "id": "a3s/science",
                    "description": "Installed signed Science extension",
                    "presence": "managed",
                    "health": "ready",
                    "version": PACKAGE_VERSION,
                    "path": package_root,
                    "trust": "registry-tuf",
                }],
            },
        }))
        .expect("serialize installed component list"),
    )
    .expect("write installed component list");
    fs::write(
        &upgraded_components,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "components": [{
                    "id": "a3s/science",
                    "description": "Upgraded signed Science extension",
                    "presence": "managed",
                    "health": "ready",
                    "version": UPGRADED_PACKAGE_VERSION,
                    "path": upgraded_package_root,
                    "trust": "registry-tuf",
                }],
            },
        }))
        .expect("serialize upgraded component list"),
    )
    .expect("write upgraded component list");
    fs::write(
        &installed_status,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "component": {
                    "id": "a3s/science",
                    "presence": "managed",
                    "health": "ready",
                    "version": PACKAGE_VERSION,
                    "trust": "registry-tuf",
                    "registry": installed_registry,
                },
            },
        }))
        .expect("serialize installed component status"),
    )
    .expect("write installed component status");
    fs::write(
        &upgraded_status,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "component": {
                    "id": "a3s/science",
                    "presence": "managed",
                    "health": "ready",
                    "version": UPGRADED_PACKAGE_VERSION,
                    "trust": "registry-tuf",
                    "registry": upgraded_registry,
                },
            },
        }))
        .expect("serialize upgraded component status"),
    )
    .expect("write upgraded component status");
    make_executable(
        &directory.join("a3s-use"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then printf 'a3s-use 0.3.0\n'; exit 0; fi

current_generation() {{
  if [ -f {registry} ] && /usr/bin/grep -q '"generation": 3' {registry}; then
    printf '3\n'
  elif [ -f {registry} ] && /usr/bin/grep -q '"generation": 2' {registry}; then
    printf '2\n'
  elif [ -f {registry} ]; then
    printf '1\n'
  else
    printf '0\n'
  fi
}}

if [ "$1" = "capability" ] && [ "$2" = "snapshot" ]; then
  generation=$(current_generation)
  if [ "$generation" = "1" ]; then /bin/cat {installed}; elif [ "$generation" = "2" ]; then /bin/cat {upgraded}; elif [ "$generation" = "3" ]; then /bin/cat {removed}; else /bin/cat {empty}; fi
  exit 0
fi
if [ "$1" = "capability" ] && [ "$2" = "watch" ]; then
  generation=$(current_generation)
  if [ "$generation" -gt "$4" ]; then
    if [ "$generation" = "1" ]; then /bin/cat {changed}; elif [ "$generation" = "2" ]; then /bin/cat {upgraded_changed}; else /bin/cat {removed}; fi
  else
    /bin/sleep 0.05
    /bin/cat {unchanged}
  fi
  exit 0
fi
if [ "$1" = "component" ] && [ "$2" = "list" ]; then
  generation=$(current_generation)
  if [ "$generation" = "1" ]; then
    /bin/cat {installed_components}
  elif [ "$generation" = "2" ]; then
    /bin/cat {upgraded_components}
  else
    printf '{{"schemaVersion":1,"ok":true,"data":{{"components":[]}}}}\n'
  fi
  exit 0
fi
if [ "$1" = "component" ] && [ "$2" = "status" ]; then
  generation=$(current_generation)
  if [ "$generation" = "1" ]; then
    /bin/cat {installed_status}
  elif [ "$generation" = "2" ]; then
    /bin/cat {upgraded_status}
  else
    printf '{{"schemaVersion":1,"ok":true,"data":{{"component":{{"id":"%s","presence":"missing","health":"unknown"}}}}}}\n' "$3"
  fi
  exit 0
fi
if [ "$1" = "extension" ] && [ "$2" = "planning-evidence" ]; then
  /bin/cat {planning_evidence}
  exit 0
fi
exit 2
"#,
            registry = sh_quote(extension_registry),
            installed_components = sh_quote(&installed_components),
            upgraded_components = sh_quote(&upgraded_components),
            installed_status = sh_quote(&installed_status),
            upgraded_status = sh_quote(&upgraded_status),
            planning_evidence = sh_quote(&planning_evidence),
            installed = sh_quote(snapshots.installed),
            upgraded = sh_quote(snapshots.upgraded),
            empty = sh_quote(snapshots.empty),
            changed = sh_quote(snapshots.changed),
            upgraded_changed = sh_quote(snapshots.upgraded_changed),
            removed = sh_quote(snapshots.removed),
            unchanged = sh_quote(snapshots.unchanged),
        ),
    );
}

fn extension_registry_generation(path: &Path) -> u64 {
    serde_json::from_slice::<Value>(&fs::read(path).expect("read Extension Registry snapshot"))
        .expect("parse Extension Registry snapshot")["generation"]
        .as_u64()
        .expect("Extension Registry generation")
}

fn publish_planning_evidence(
    temp: &TempWorkspace,
    package_id: &str,
    version: &str,
    capability_generation: u64,
    snapshot_path: &Path,
    changed_snapshot_path: &Path,
    use_bin: &Path,
) {
    let registry = ExtensionRegistry::new(ExtensionPaths::new(
        temp.path("data/use"),
        temp.path("state/use"),
    ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build Extension Registry fixture runtime");
    let extension = runtime
        .block_on(registry.get(package_id))
        .expect("read installed cognitive package")
        .expect("installed cognitive package");
    assert_eq!(extension.receipt.version, version);
    let catalog = extension
        .plan_ready_catalog()
        .expect("current package catalog")
        .clone();
    let mut selected_surfaces = catalog
        .record
        .surfaces
        .iter()
        .map(|surface| surface.reference())
        .collect::<Vec<_>>();
    selected_surfaces.sort();
    let evidence = InstalledPluginPlanEvidence {
        schema: INSTALLED_PLUGIN_PLAN_EVIDENCE_SCHEMA.to_string(),
        component_id: extension.receipt.component_id.clone(),
        package_id: extension.receipt.package_id.clone(),
        version: extension.receipt.version.clone(),
        capability_generation,
        capability_revision: capability_generation.to_string().repeat(64),
        receipt_digest: extension.receipt.descriptor_digest().unwrap(),
        desired_enabled: extension.receipt.enabled,
        selected_surfaces: selected_surfaces.clone(),
        verified_catalog: catalog,
    };
    evidence
        .validate()
        .expect("valid installed planning evidence");

    let mut snapshot: Value =
        serde_json::from_slice(&fs::read(snapshot_path).expect("read capability fixture"))
            .expect("parse capability fixture");
    let capability = snapshot
        .pointer_mut("/data/registry/capabilities/0")
        .and_then(Value::as_object_mut)
        .expect("installed capability fixture");
    capability.insert(
        "plannerEvidence".to_string(),
        json!({
            "schemaVersion": 1,
            "packageId": evidence.package_id,
            "packageSha256": evidence
                .verified_catalog
                .record
                .package
                .sha256
                .as_deref()
                .expect("catalog package digest"),
            "manifestSha256": evidence
                .verified_catalog
                .record
                .package
                .manifest_sha256
                .as_deref()
                .expect("catalog manifest digest"),
            "receiptDigest": evidence.receipt_digest,
            "catalogRecordDigest": evidence.verified_catalog.provenance.catalog_record_digest,
            "desiredEnabled": evidence.desired_enabled,
            "selectedSurfaces": selected_surfaces,
        }),
    );
    capability.insert(
        "reconciliation".to_string(),
        json!({
            "schemaVersion": 1,
            "desired": if evidence.desired_enabled { "enabled" } else { "installed-disabled" },
            "capabilityReady": evidence.desired_enabled,
            "surfaces": selected_surfaces
                .iter()
                .map(|surface| json!({"surface": surface}))
                .collect::<Vec<_>>(),
        }),
    );
    let staged_snapshot = snapshot_path.with_extension("json.next");
    fs::write(&staged_snapshot, serde_json::to_vec(&snapshot).unwrap())
        .expect("stage capability fixture");
    fs::rename(staged_snapshot, snapshot_path).expect("publish capability fixture");
    let changed = json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {
            "changed": true,
            "registry": snapshot["data"]["registry"],
        },
    });
    let staged_changed = changed_snapshot_path.with_extension("json.next");
    fs::write(&staged_changed, serde_json::to_vec(&changed).unwrap())
        .expect("stage capability watch fixture");
    fs::rename(staged_changed, changed_snapshot_path).expect("publish capability watch fixture");
    fs::write(
        use_bin.join("planning-evidence.json"),
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {"planningEvidence": evidence},
        }))
        .unwrap(),
    )
    .expect("write installed planning-evidence fixture");
}

fn enroll_registry(
    temp: &TempWorkspace,
    config: &Path,
    use_bin: &Path,
    url: &str,
    root_sha256: &str,
) {
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, temp);
    let output = command
        .arg("--config")
        .arg(config)
        .args([
            "--output",
            "json",
            "registry",
            "add",
            "localhost",
            url,
            "--root-sha256",
            root_sha256,
            "--yes",
        ])
        .env("A3S_USE_INSTALL_DIR", use_bin)
        .env_remove("A3S_USE_HOME")
        .output()
        .expect("enroll signed registry");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn start_web(
    temp: &TempWorkspace,
    workspace: &Path,
    web_dir: &Path,
    config: &Path,
    use_bin: &Path,
    session_state: &Path,
) -> (DaemonGuard, String) {
    let flow_compiler = temp.path("flow-native-compiler");
    let flow_compile_log = temp.path("flow-native-compiler.log");
    make_flow_compiler(&flow_compiler, &flow_compile_log);
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, temp);
    let output = command
        .arg("--config")
        .arg(config)
        .args([
            "web",
            "-d",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--workspace",
        ])
        .arg(workspace)
        .arg("--web-dir")
        .arg(web_dir)
        .env("A3S_USE_INSTALL_DIR", use_bin)
        .env("A3S_CODE_WEB_STATE_DIR", session_state)
        .env("A3S_FLOW_NATIVE_TS_COMPILER", flow_compiler)
        // Exercise the Linux Tokio worker-stack baseline on every Unix host.
        // Reviewed enablement must remain safe below the full Web dispatch stack.
        .env("RUST_MIN_STACK", TEST_WEB_WORKER_STACK_BYTES)
        .env_remove("A3S_USE_HOME")
        .current_dir(workspace)
        .output()
        .expect("start detached Web");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let pid = output_value(&stdout, "Background PID:")
        .parse::<u32>()
        .expect("Web PID");
    let address = output_value(&stdout, "A3S Web:")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    let log_path = PathBuf::from(output_value(&stdout, "Log:"));
    (DaemonGuard::new(pid, log_path), address)
}

fn make_flow_compiler(path: &Path, compile_log: &Path) {
    make_executable(
        path,
        &format!(
            r#"#!/bin/sh
set -eu
printf 'compile\n' >> {compile_log}
[ "$1" = "compile" ]
[ "$3" = "-o" ]
/bin/cat > "$4" <<'A3S_FLOW_RUNTIME'
#!/bin/sh
set -eu
request=$(/bin/cat)
case "$request" in
  *'"exportName":"runV2"'*) marker=upgraded ;;
  *) marker=installed ;;
esac
printf '{{"protocol":"a3s.flow.native_ts.v1","kind":"workflow","ok":true,"output":{{"type":"complete","output":{{"marker":"%s"}}}}}}\n' "$marker"
A3S_FLOW_RUNTIME
/bin/chmod +x "$4"
"#,
            compile_log = sh_quote(compile_log),
        ),
    );
}

fn wait_for_activity(address: &str, key: &str) -> Value {
    for _ in 0..100 {
        let catalog = http_json(address, "GET", "/api/v1/plugins/activities", None);
        if catalog["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["key"] == key))
        {
            return catalog;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Activity Bar contribution '{key}' did not hot-plug");
}

fn wait_for_activity_after(address: &str, key: &str, after_generation: u64) -> Value {
    for _ in 0..200 {
        let catalog = http_json(address, "GET", "/api/v1/plugins/activities", None);
        if catalog["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["key"] == key))
            && catalog["generation"]
                .as_u64()
                .is_some_and(|generation| generation > after_generation)
        {
            return catalog;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Activity Bar contribution '{key}' did not hot-upgrade");
}

fn wait_for_flow(address: &str, key: &str) -> Value {
    for _ in 0..100 {
        let catalog = http_json(address, "GET", "/api/v1/plugins/flows", None);
        if catalog["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["key"] == key))
        {
            return catalog;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("A3S Flow contribution '{key}' did not hot-plug");
}

fn wait_for_flow_after(address: &str, key: &str, after_generation: u64) -> Value {
    for _ in 0..200 {
        let catalog = http_json(address, "GET", "/api/v1/plugins/flows", None);
        if catalog["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["key"] == key))
            && catalog["generation"]
                .as_u64()
                .is_some_and(|generation| generation > after_generation)
        {
            return catalog;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("A3S Flow contribution '{key}' did not hot-upgrade");
}

fn wait_for_activity_absent(address: &str, key: &str, after_generation: u64) {
    for _ in 0..200 {
        let catalog = http_json(address, "GET", "/api/v1/plugins/activities", None);
        if catalog["items"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["key"] != key))
            && catalog["generation"]
                .as_u64()
                .is_some_and(|generation| generation > after_generation)
        {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Activity Bar contribution '{key}' did not disappear after uninstall");
}

fn activity_document_url(catalog: &Value, key: &str) -> String {
    let item = catalog["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == key))
        .unwrap_or_else(|| panic!("Activity Bar item `{key}`: {catalog:#}"));
    let url = item["documentUrl"]
        .as_str()
        .unwrap_or_else(|| panic!("generation-bound Activity document URL: {catalog:#}"));
    assert_eq!(
        url,
        format!(
            "/api/v1/plugins/activities/{}/document?generation={}&revision={}",
            key.replace(':', "%3A"),
            catalog["generation"].as_u64().expect("catalog generation"),
            catalog["revision"].as_str().expect("catalog revision")
        )
    );
    url.to_string()
}

fn assert_activity_document(
    address: &str,
    document_url: &str,
    marker: &str,
    expected_style: &str,
    expected_script: &str,
    forbidden_paths: &[&Path],
) {
    let response = http_response(address, "GET", document_url, None);
    assert_eq!(response.status, 200, "{response:#?}");
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        response.headers.get("cache-control").map(String::as_str),
        Some("no-store")
    );
    assert_eq!(
        response.headers.get("referrer-policy").map(String::as_str),
        Some("no-referrer")
    );
    assert_eq!(
        response
            .headers
            .get("x-content-type-options")
            .map(String::as_str),
        Some("nosniff")
    );
    assert_eq!(
        response.headers.get("x-frame-options").map(String::as_str),
        Some("SAMEORIGIN")
    );
    assert_eq!(
        response
            .headers
            .get("cross-origin-resource-policy")
            .map(String::as_str),
        Some("same-origin")
    );
    let permissions = response
        .headers
        .get("permissions-policy")
        .expect("Permissions-Policy header");
    for denied in ["camera=()", "geolocation=()", "microphone=()", "usb=()"] {
        assert!(permissions.contains(denied), "{permissions}");
    }
    let csp = response
        .headers
        .get("content-security-policy")
        .expect("Content-Security-Policy header");
    for directive in [
        "sandbox allow-scripts",
        "default-src 'none'",
        "connect-src 'none'",
        "frame-src 'none'",
        "object-src 'none'",
        "form-action 'none'",
        "base-uri 'none'",
        "frame-ancestors 'self'",
    ] {
        assert!(csp.contains(directive), "missing `{directive}` in {csp}");
    }
    assert!(!csp.contains("allow-same-origin"), "{csp}");
    assert!(response.body.contains(marker), "{}", response.body);
    assert!(response.body.contains(expected_style), "{}", response.body);
    assert!(response.body.contains(expected_script), "{}", response.body);
    for path in forbidden_paths {
        let path = path.display().to_string();
        assert!(
            !response.body.contains(&path),
            "Activity document leaked `{path}`: {}",
            response.body
        );
    }
}

fn assert_http_status(address: &str, path: &str, expected: u16) {
    let response = http_response(address, "GET", path, None);
    assert_eq!(response.status, expected, "{response:#?}");
}

fn wait_for_flow_absent(address: &str, key: &str, after_generation: u64) {
    for _ in 0..200 {
        let catalog = http_json(address, "GET", "/api/v1/plugins/flows", None);
        if catalog["items"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["key"] != key))
            && catalog["generation"]
                .as_u64()
                .is_some_and(|generation| generation > after_generation)
        {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("A3S Flow contribution '{key}' did not disappear after uninstall");
}

fn http_json(address: &str, method: &str, path: &str, body: Option<&Value>) -> Value {
    http_json_status(address, method, path, body, "200")
}

fn http_json_status(
    address: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
    expected_status: &str,
) -> Value {
    let response = http_response(address, method, path, body);
    let expected_status = expected_status.parse::<u16>().unwrap_or_else(|error| {
        panic!("invalid expected HTTP status `{expected_status}`: {error}")
    });
    assert_eq!(
        response.status, expected_status,
        "{method} {path} returned an unexpected response:\n{response:#?}"
    );
    serde_json::from_str(&response.body)
        .unwrap_or_else(|error| panic!("invalid JSON ({error}): {}", response.body))
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

fn http_response(address: &str, method: &str, path: &str, body: Option<&Value>) -> HttpResponse {
    let body = body.map(Value::to_string).unwrap_or_default();
    let mut stream = TcpStream::connect(address).expect("connect to Web API");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("set response timeout");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write Web API request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read Web API response");
    let (head, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("HTTP response has no body: {response}"));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .unwrap_or_else(|| panic!("HTTP response has no status: {response}"));
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    HttpResponse {
        status,
        headers,
        body: body.to_string(),
    }
}

fn output_value<'a>(output: &'a str, prefix: &str) -> &'a str {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::trim))
        .unwrap_or_else(|| panic!("missing '{prefix}' in output:\n{output}"))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn assert_path_free(value: &Value, forbidden_paths: &[&Path]) {
    let encoded = value.to_string();
    for path in forbidden_paths {
        let path = path.display().to_string();
        assert!(
            !encoded.contains(&path),
            "public Flow response leaked managed or workspace path `{path}`: {value:#}"
        );
    }
    assert!(!encoded.contains("sourcePath"), "{value:#}");
    assert!(!encoded.contains("packageRoot"), "{value:#}");
    assert!(!encoded.contains("entrypoint"), "{value:#}");
}

fn bound_flow_design(version: &str, lifecycle_generation: u64, source_sha256: &str) -> String {
    json!({
        "version": "a3s.workflow.design.v1",
        "name": "Science research",
        "description": "Run the installed Science research Flow",
        "installedFlow": {
            "schema": "a3s.use.installed-flow.v1",
            "packageId": "use/a3s/science",
            "flowId": "research",
            "version": version,
            "lifecycleGeneration": lifecycle_generation,
            "sourceSha256": source_sha256,
        },
        "nodes": [],
        "edges": [],
    })
    .to_string()
}

struct DaemonGuard {
    pid: u32,
    log_path: PathBuf,
    active: bool,
}

impl DaemonGuard {
    fn new(pid: u32, log_path: PathBuf) -> Self {
        Self {
            pid,
            log_path,
            active: true,
        }
    }

    fn stop(&mut self) {
        if !self.active {
            return;
        }
        let stopped = Command::new("kill")
            .args(["-INT", &self.pid.to_string()])
            .output()
            .is_ok_and(|output| output.status.success());
        if !stopped {
            let log = fs::read_to_string(&self.log_path)
                .unwrap_or_else(|error| format!("<could not read Web log: {error}>"));
            eprintln!(
                "A3S Web process {} exited before test cleanup; log {} follows:\n{}",
                self.pid,
                self.log_path.display(),
                log
            );
        }
        self.active = false;
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn wait_until_stopped(address: &str) {
    for _ in 0..100 {
        if TcpStream::connect(address).is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Web process still listens on {address}");
}

fn test_config() -> &'static str {
    r#"default_model = "openai/test"
providers "openai" {
  apiKey = "test"
  baseUrl = "http://127.0.0.1:1"
  models "test" {
    name = "Test"
    toolCall = true
  }
}
memory { llmExtraction = false }
"#
}
