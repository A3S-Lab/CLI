#![cfg(unix)]

mod support;

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use support::{a3s_bin, configure_component_env, make_executable, sh_quote, TempWorkspace};

#[path = "support/tuf_test_support.rs"]
mod tuf_test_support;

use tuf_test_support::{
    extension_archive, host_target, TestRepository, TestServer, TestTarget, FUTURE, PACKAGE_VERSION,
};

const UPGRADED_PACKAGE_VERSION: &str = "0.1.2";

#[path = "web_plugin_marketplace/real_e2e.rs"]
mod real_e2e;

#[test]
fn marketplace_install_upgrade_uninstall_hot_plugs_verified_activity_skill_and_flow_catalog() {
    let temp = TempWorkspace::new("web-plugin-marketplace");
    let workspace = temp.path("workspace");
    let web_dir = temp.path("web");
    let config = temp.path("config/config.acl");
    let use_bin = temp.path("use-bin");
    let package_root = temp.path("managed-package-v1");
    let upgraded_package_root = temp.path("managed-package-v2");
    let installed_marker = temp.path("installed");
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
    let activity_js =
        "window.parent.postMessage({ protocol: 'a3s.activity.v1', type: 'activity.ready' }, '*');";
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
    let upgraded_activity_js = "window.parent.postMessage({ protocol: 'a3s.activity.v1', type: 'activity.ready', version: 2 }, '*');";
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

    let empty_snapshot = snapshot_envelope(1, "1", Vec::new());
    let installed_capability = json!({
        "id": "use/a3s/science",
        "route": "science",
        "version": PACKAGE_VERSION,
        "origin": "extension",
        "enabled": true,
        "readiness": "ready",
        "packageRoot": package_root,
        "lifecycleGeneration": 2,
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
    let installed_snapshot = snapshot_envelope(2, "2", vec![installed_capability]);
    let upgraded_capability = json!({
        "id": "use/a3s/science",
        "route": "science",
        "version": UPGRADED_PACKAGE_VERSION,
        "origin": "extension",
        "enabled": true,
        "readiness": "ready",
        "packageRoot": upgraded_package_root,
        "lifecycleGeneration": 3,
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
    let upgraded_snapshot = snapshot_envelope(3, "3", vec![upgraded_capability]);
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
                "registry": snapshot_envelope(4, "4", Vec::new())["data"]["registry"],
            },
        }))
        .unwrap(),
    )
    .expect("write removed snapshot");
    let repository = TestRepository::new(extension_archive(PACKAGE_VERSION), 1, FUTURE);
    let upgraded_repository = TestRepository::with_targets(
        vec![
            legacy_test_target(PACKAGE_VERSION),
            legacy_test_target(UPGRADED_PACKAGE_VERSION),
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
        &installed_marker,
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
    assert_eq!(item["displayName"], "科研");
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
    let digest = plan["planDigest"]
        .as_str()
        .expect("reviewed plan digest")
        .to_string();
    assert!(
        !installed_marker.exists(),
        "planning must not install the package"
    );

    let applied = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "action": "install",
            "componentId": "use/a3s/science",
            "version": PACKAGE_VERSION,
            "channel": "stable",
            "planDigest": digest,
        })),
    );
    assert!(applied["operations"]
        .as_array()
        .is_some_and(|operations| operations
            .iter()
            .any(|operation| operation["changed"] == true)));
    assert!(installed_marker.is_file());

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
    let flows = wait_for_flow(&address, "science:research");
    let flow = flows["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == "science:research"))
        .expect("hot-plugged A3S Flow contribution");
    assert_eq!(flow["packageId"], "use/a3s/science");
    assert_eq!(flow["engine"], "a3s-flow");
    assert_eq!(flow["runtime"], "native-ts");
    assert_eq!(flow["lifecycleGeneration"], 2);
    assert_eq!(flow["exportName"], "run");
    assert_eq!(flow["sha256"], sha256(flow_source.as_bytes()));
    assert_eq!(flow["requiresTools"], json!(["collect"]));
    assert_eq!(flow["requiresMcp"], json!(["papers"]));
    assert_eq!(flow["requiresOkf"], json!(["research-domain"]));

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
    let upgrade_digest = upgrade_plan["planDigest"]
        .as_str()
        .expect("reviewed upgrade digest")
        .to_string();
    assert_eq!(
        fs::read_to_string(&installed_marker).unwrap().trim(),
        "installed",
        "upgrade planning must not mutate the installed generation"
    );

    let upgraded = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "action": "upgrade",
            "componentId": "use/a3s/science",
            "planDigest": upgrade_digest,
        })),
    );
    assert!(upgraded["operations"]
        .as_array()
        .is_some_and(|operations| operations
            .iter()
            .any(|operation| operation["changed"] == true)));
    assert_eq!(
        fs::read_to_string(&installed_marker).unwrap().trim(),
        "upgraded"
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

    let upgraded_flows = wait_for_flow_after(&address, "science:research", installed_generation);
    assert_eq!(upgraded_flows["generation"], upgraded_generation);
    let upgraded_flow = upgraded_flows["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == "science:research"))
        .expect("upgraded A3S Flow contribution");
    assert_eq!(upgraded_flow["lifecycleGeneration"], 3);
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
    let uninstall_digest = uninstall_plan["planDigest"]
        .as_str()
        .expect("reviewed uninstall digest");
    let uninstalled = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "action": "uninstall",
            "componentId": "use/a3s/science",
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
    let removed_marketplace = http_json(&address, "GET", "/api/v1/plugins/marketplace", None);
    assert!(removed_marketplace["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["componentId"] == "use/a3s/science")
        })
        .is_some_and(|item| item["installed"] == false && item["enabled"] == false));

    daemon.stop();
    wait_until_stopped(&address);
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

fn legacy_test_target(version: &str) -> TestTarget {
    let target = host_target();
    let archive_name = format!("a3s-use-a3s-science-{version}-{target}.tar.gz");
    TestTarget {
        archive: extension_archive(version),
        target_name: format!("extensions/a3s/science/{version}/stable/{target}/{archive_name}"),
        custom: Some(json!({
            "schemaVersion": 1,
            "packageId": "a3s/science",
            "version": version,
            "channel": "stable",
            "target": target,
        })),
    }
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
    installed_marker: &Path,
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
if [ "$1" = "--version" ]; then printf 'a3s-use 0.1.2\n'; exit 0; fi
if [ "$1" = "capability" ] && [ "$2" = "snapshot" ]; then
  state=$(/bin/cat {marker} 2>/dev/null || true)
  if [ "$state" = "installed" ]; then /bin/cat {installed}; elif [ "$state" = "upgraded" ]; then /bin/cat {upgraded}; elif [ "$state" = "removed" ]; then /bin/cat {removed}; else /bin/cat {empty}; fi
  exit 0
fi
if [ "$1" = "capability" ] && [ "$2" = "watch" ]; then
  state=$(/bin/cat {marker} 2>/dev/null || true)
  if [ "$state" = "installed" ] && [ "$4" = "1" ]; then /bin/cat {changed}; elif [ "$state" = "upgraded" ] && [ "$4" = "2" ]; then /bin/cat {upgraded_changed}; elif [ "$state" = "removed" ] && [ "$4" = "3" ]; then /bin/cat {removed}; else /bin/sleep 0.05; /bin/cat {unchanged}; fi
  exit 0
fi
if [ "$1" = "component" ] && [ "$2" = "list" ]; then
  state=$(/bin/cat {marker} 2>/dev/null || true)
  if [ "$state" = "installed" ]; then
    /bin/cat {installed_components}
  elif [ "$state" = "upgraded" ]; then
    /bin/cat {upgraded_components}
  else
    printf '{{"schemaVersion":1,"ok":true,"data":{{"components":[]}}}}\n'
  fi
  exit 0
fi
if [ "$1" = "component" ] && [ "$2" = "status" ]; then
  state=$(/bin/cat {marker} 2>/dev/null || true)
  if [ "$state" = "installed" ]; then
    /bin/cat {installed_status}
  elif [ "$state" = "upgraded" ]; then
    /bin/cat {upgraded_status}
  else
    printf '{{"schemaVersion":1,"ok":true,"data":{{"component":{{"id":"%s","presence":"missing","health":"unknown"}}}}}}\n' "$3"
  fi
  exit 0
fi
if [ "$1" = "component" ] && [ "$2" = "install" ]; then
  requested_version=''
  previous=''
  for argument in "$@"; do
    if [ "$previous" = "--version" ]; then requested_version=$argument; break; fi
    previous=$argument
  done
  if [ "$requested_version" = "{upgraded_version}" ]; then
    printf 'upgraded\n' > {marker}
    printf '{{"schemaVersion":1,"ok":true,"data":{{"changed":true,"component":{{"id":"%s","version":"{upgraded_version}","trust":"registry-tuf"}}}}}}\n' "$3"
  else
    printf 'installed\n' > {marker}
    printf '{{"schemaVersion":1,"ok":true,"data":{{"changed":true,"component":{{"id":"%s","version":"{version}","trust":"registry-tuf"}}}}}}\n' "$3"
  fi
  exit 0
fi
if [ "$1" = "component" ] && [ "$2" = "uninstall" ]; then
  printf 'removed\n' > {marker}
  printf '{{"schemaVersion":1,"ok":true,"data":{{"changed":true,"component":"%s"}}}}\n' "$3"
  exit 0
fi
exit 2
"#,
            marker = sh_quote(installed_marker),
            installed_components = sh_quote(&installed_components),
            upgraded_components = sh_quote(&upgraded_components),
            installed_status = sh_quote(&installed_status),
            upgraded_status = sh_quote(&upgraded_status),
            installed = sh_quote(snapshots.installed),
            upgraded = sh_quote(snapshots.upgraded),
            empty = sh_quote(snapshots.empty),
            changed = sh_quote(snapshots.changed),
            upgraded_changed = sh_quote(snapshots.upgraded_changed),
            removed = sh_quote(snapshots.removed),
            unchanged = sh_quote(snapshots.unchanged),
            version = PACKAGE_VERSION,
            upgraded_version = UPGRADED_PACKAGE_VERSION,
        ),
    );
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
            url,
            "--trust-root",
            &format!("sha256:{root_sha256}"),
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
    (DaemonGuard::new(pid), address)
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
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "{method} {path} returned an unexpected response:\n{response}"
    );
    let (_, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("HTTP response has no body: {response}"));
    serde_json::from_str(body).unwrap_or_else(|error| panic!("invalid JSON ({error}): {body}"))
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

struct DaemonGuard {
    pid: u32,
    active: bool,
}

impl DaemonGuard {
    fn new(pid: u32) -> Self {
        Self { pid, active: true }
    }

    fn stop(&mut self) {
        if !self.active {
            return;
        }
        let _ = Command::new("kill")
            .args(["-INT", &self.pid.to_string()])
            .status();
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
