use super::*;

use a3s_use_core::{
    CatalogArchive, CatalogAvailability, CatalogPackage, CatalogSurface, PluginCatalogRecord,
    PluginPermissionCeiling, PluginReleaseChannel, PluginSurfaceKind, PluginSurfaceRef,
    PLUGIN_CATALOG_SCHEMA_V3, PLUGIN_PERMISSION_SCHEMA,
};

const REVIEWED_PACKAGE_ID: &str = "acme/guide";
const REVIEWED_COMPONENT_ID: &str = "use/acme/guide";
const REVIEWED_VERSION: &str = "1.0.0";
const REVIEWED_ACTIVITY_KEY: &str = "guide:review";

#[test]
fn reviewed_enablement_hot_plugs_skill_ui_and_flow_and_replays_after_restart() {
    let temp = TempWorkspace::new("web-reviewed-enablement");
    let workspace = temp.path("workspace");
    let web_dir = temp.path("web");
    let config = temp.path("config/config.acl");
    let use_bin = temp.path("use-bin");
    let package_root = temp.path("reviewed-package");
    let session_state = temp.path("web-session-state");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&web_dir).expect("create Web assets");
    fs::create_dir_all(config.parent().expect("config parent")).expect("create config parent");
    fs::write(
        web_dir.join("index.html"),
        "<!doctype html><title>A3S reviewed enablement integration</title>",
    )
    .expect("write Web fixture");
    fs::write(&config, test_config()).expect("write config fixture");

    let skill = "---\nname: guide\ndescription: Reviewed guide fixture\n---\n# Guide\n";
    let activity_html =
        "<!doctype html><title>Guide</title><main>Reviewed enablement activity</main>";
    let activity_css = "main { color: rebeccapurple; }";
    let activity_js =
        "window.parent.postMessage({ protocol: 'a3s.activity.v1', type: 'activity.ready' }, '*');";
    let flow_source =
        "export async function run(input: unknown): Promise<unknown> { return input; }\n";
    let manifest = reviewed_manifest();
    fs::create_dir_all(package_root.join("skills/main")).expect("create Skill directory");
    fs::create_dir_all(package_root.join("web")).expect("create Activity directory");
    fs::create_dir_all(package_root.join("flows")).expect("create Flow directory");
    fs::write(package_root.join("a3s-use-extension.acl"), &manifest)
        .expect("write reviewed manifest");
    fs::write(package_root.join("README.md"), "# Reviewed Guide\n").expect("write package README");
    fs::write(package_root.join("skills/main/SKILL.md"), skill).expect("write Skill");
    fs::write(package_root.join("web/activity.html"), activity_html).expect("write Activity HTML");
    fs::write(package_root.join("web/activity.css"), activity_css).expect("write Activity CSS");
    fs::write(package_root.join("web/activity.js"), activity_js).expect("write Activity JS");
    fs::write(package_root.join("flows/review.ts"), flow_source).expect("write Flow source");

    let archive = package_directory_archive(&package_root);
    let target = host_target();
    let target_name = format!(
        "extensions/{REVIEWED_PACKAGE_ID}/{REVIEWED_VERSION}/stable/{target}/guide-{REVIEWED_VERSION}-{target}.tar.gz"
    );
    let (package_sha256, file_count, expanded_bytes) = package_fingerprint(&package_root);
    let permissions = PluginPermissionCeiling {
        schema: PLUGIN_PERMISSION_SCHEMA.to_string(),
        surfaces: Vec::new(),
    };
    let catalog = PluginCatalogRecord {
        schema: PLUGIN_CATALOG_SCHEMA_V3.to_string(),
        package_id: REVIEWED_PACKAGE_ID.to_string(),
        display_name: "Reviewed Guide".to_string(),
        description: "Signed schema-v3 package for Web enablement testing.".to_string(),
        publisher: "acme".to_string(),
        keywords: vec!["guide".to_string()],
        categories: vec!["productivity".to_string()],
        version: REVIEWED_VERSION.to_string(),
        channel: PluginReleaseChannel::Stable,
        requires_use: ">=0.3.0, <0.4.0".to_string(),
        dependencies: Vec::new(),
        target: target.to_string(),
        surfaces: vec![
            CatalogSurface {
                kind: PluginSurfaceKind::Flow,
                id: "review".to_string(),
                optional: false,
                workload: None,
                mcp_transport: None,
                mcp_tool_count: None,
                okf_bundle: None,
                requires: Vec::new(),
            },
            CatalogSurface {
                kind: PluginSurfaceKind::Skill,
                id: "main".to_string(),
                optional: false,
                workload: None,
                mcp_transport: None,
                mcp_tool_count: None,
                okf_bundle: None,
                requires: Vec::new(),
            },
            CatalogSurface {
                kind: PluginSurfaceKind::Ui,
                id: "review".to_string(),
                optional: false,
                workload: None,
                mcp_transport: None,
                mcp_tool_count: None,
                okf_bundle: None,
                requires: vec![PluginSurfaceRef {
                    kind: PluginSurfaceKind::Skill,
                    id: "main".to_string(),
                }],
            },
        ],
        permission_ceiling_digest: permissions.descriptor_digest().unwrap(),
        permission_ceiling: permissions,
        planning: None,
        archive: CatalogArchive {
            target_name: target_name.clone(),
            length: archive.len() as u64,
            sha256: format!("sha256:{}", sha256(&archive)),
        },
        package: CatalogPackage {
            expanded_bytes,
            file_count,
            sha256: Some(format!("sha256:{package_sha256}")),
            manifest_sha256: Some(format!("sha256:{}", sha256(manifest.as_bytes()))),
        },
        license: "MIT".to_string(),
        repository: "https://github.com/acme/guide".to_string(),
        availability: CatalogAvailability::Available,
    };
    catalog.validate().expect("valid schema-v3 catalog");
    let repository = TestRepository::with_targets(
        vec![TestTarget {
            archive,
            target_name,
            custom: Some(serde_json::to_value(catalog).unwrap()),
        }],
        91,
        FUTURE,
    );
    let registry_server = TestServer::start(repository.routes.clone());
    let registry_url = registry_server
        .base_url()
        .replacen("127.0.0.1", "localhost", 1);
    make_reviewed_use_fixture(
        &temp,
        &use_bin,
        &package_root,
        skill,
        activity_html,
        activity_css,
        activity_js,
        flow_source,
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
    assert_eq!(
        http_json(&address, "GET", "/api/v1/plugins/activities", None)["items"],
        json!([])
    );
    assert_eq!(
        http_json(&address, "GET", "/api/v1/plugins/flows", None)["items"],
        json!([])
    );

    let install_plan = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/plan",
        Some(&json!({
            "action": "install",
            "componentId": REVIEWED_COMPONENT_ID,
            "version": REVIEWED_VERSION,
            "channel": "stable",
        })),
    );
    let (install_operation_id, install_digest) = reviewed_identity(&install_plan);
    let installed = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "operationId": install_operation_id,
            "planDigest": install_digest,
        })),
    );
    assert_eq!(installed["replayed"], false);
    let installed_activities = wait_for_activity(&address, REVIEWED_ACTIVITY_KEY);
    assert_eq!(installed_activities["generation"], 1);
    assert_eq!(
        installed_activities["items"][0]["packageId"],
        REVIEWED_COMPONENT_ID
    );
    let installed_flows = wait_for_flow(&address, REVIEWED_ACTIVITY_KEY);
    assert_eq!(installed_flows["generation"], 1);
    assert_eq!(
        installed_flows["items"][0]["packageId"],
        REVIEWED_COMPONENT_ID
    );
    assert_eq!(installed_flows["items"][0]["engine"], "a3s-flow");
    assert_eq!(installed_flows["items"][0]["runtime"], "native-ts");
    assert_eq!(installed_flows["items"][0]["exportName"], "run");
    assert_eq!(
        fs::read_to_string(temp.path("flow-native-compiler.log"))
            .expect("Flow compiler log")
            .lines()
            .count(),
        1,
        "the real archived Flow must be compiled during reviewed install"
    );

    let disable_plan = http_json(
        &address,
        "POST",
        "/api/v1/plugins/packages/enablement/plan",
        Some(&json!({
            "componentId": REVIEWED_COMPONENT_ID,
            "enabled": false,
        })),
    );
    assert_eq!(disable_plan["status"], "planned");
    assert_eq!(disable_plan["state"]["desired"], "enabled");
    assert_eq!(disable_plan["plan"]["plan"]["action"], "disable");
    assert_eq!(disable_plan["plan"]["plan"]["authority"]["actor"], "user");
    let (disable_operation_id, disable_digest) = reviewed_identity(&disable_plan);
    let disable_request = json!({
        "operationId": disable_operation_id,
        "planDigest": disable_digest,
    });
    let disabled = http_json(
        &address,
        "POST",
        "/api/v1/plugins/packages/enablement/apply",
        Some(&disable_request),
    );
    assert_eq!(disabled["durableEnablement"], true);
    assert_eq!(disabled["changed"], true);
    assert_eq!(disabled["replayed"], false);
    assert_eq!(disabled["state"]["desired"], "installed-disabled");
    let registry_snapshot = fs::read_to_string(temp.path("state/use/registry.json"))
        .expect("read embedded Extension Registry after disable");
    assert!(
        registry_snapshot.contains("\"generation\": 2"),
        "disable did not publish Registry generation 2: {registry_snapshot}"
    );
    wait_for_reviewed_activity_state(&address, REVIEWED_ACTIVITY_KEY, false, 1);
    wait_for_flow_absent(&address, REVIEWED_ACTIVITY_KEY, 1);
    let disabled_content = http_json_status(
        &address,
        "GET",
        "/api/v1/plugins/activities/guide%3Areview",
        None,
        "404",
    );
    assert!(disabled_content["message"]
        .as_str()
        .is_some_and(|message| message.contains("not found")));

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
    let restarted_activities =
        wait_for_reviewed_activity_state(&restarted_address, REVIEWED_ACTIVITY_KEY, false, 1);
    assert_eq!(restarted_activities["generation"], 2);
    assert_eq!(restarted_activities["items"][0]["enabled"], false);
    assert_eq!(
        http_json(&restarted_address, "GET", "/api/v1/plugins/flows", None,)["items"],
        json!([])
    );

    let replayed = http_json(
        &restarted_address,
        "POST",
        "/api/v1/plugins/packages/enablement/apply",
        Some(&disable_request),
    );
    assert_eq!(replayed["replayed"], true);
    assert_eq!(
        replayed["operationResultDigest"],
        disabled["operationResultDigest"]
    );
    assert_eq!(replayed["state"], disabled["state"]);

    let no_change = http_json(
        &restarted_address,
        "POST",
        "/api/v1/plugins/packages/enablement/plan",
        Some(&json!({
            "componentId": REVIEWED_COMPONENT_ID,
            "enabled": false,
            "expectedPackageGeneration": disabled["state"]["packageGeneration"],
        })),
    );
    assert_eq!(no_change["status"], "no-change");
    assert!(no_change.get("operationId").is_none());
    assert!(no_change.get("canonicalPlanDigest").is_none());
    assert!(no_change.get("plan").is_none());

    let enable_plan = http_json(
        &restarted_address,
        "POST",
        "/api/v1/plugins/packages/enablement/plan",
        Some(&json!({
            "componentId": REVIEWED_COMPONENT_ID,
            "enabled": true,
            "expectedPackageGeneration": disabled["state"]["packageGeneration"],
        })),
    );
    assert_eq!(enable_plan["status"], "planned");
    let (enable_operation_id, enable_digest) = reviewed_identity(&enable_plan);
    let enabled = http_json(
        &restarted_address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "operationId": enable_operation_id,
            "planDigest": enable_digest,
        })),
    );
    assert_eq!(enabled["durableEnablement"], true);
    assert_eq!(enabled["changed"], true);
    assert_eq!(enabled["replayed"], false);
    assert_eq!(enabled["state"]["desired"], "enabled");
    let enabled_activities =
        wait_for_reviewed_activity_state(&restarted_address, REVIEWED_ACTIVITY_KEY, true, 2);
    assert_eq!(enabled_activities["generation"], 3);
    let enabled_flows = wait_for_flow_after(&restarted_address, REVIEWED_ACTIVITY_KEY, 2);
    assert_eq!(enabled_flows["generation"], 3);
    let content = http_json(
        &restarted_address,
        "GET",
        "/api/v1/plugins/activities/guide%3Areview",
        None,
    );
    assert_eq!(content["html"], activity_html);
    assert_eq!(content["styles"], json!([activity_css]));
    assert_eq!(content["scripts"], json!([activity_js]));

    restarted.stop();
    wait_until_stopped(&restarted_address);
}

fn wait_for_reviewed_activity_state(
    address: &str,
    key: &str,
    enabled: bool,
    after_generation: u64,
) -> Value {
    let mut last = Value::Null;
    for _ in 0..200 {
        let catalog = http_json(address, "GET", "/api/v1/plugins/activities", None);
        let converged = catalog["items"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item["key"] == key && item["enabled"] == enabled)
        }) && catalog["generation"]
            .as_u64()
            .is_some_and(|generation| generation > after_generation);
        if converged {
            return catalog;
        }
        last = catalog;
        thread::sleep(Duration::from_millis(50));
    }
    panic!("Activity Bar contribution '{key}' did not converge to enabled={enabled}: {last:#}");
}

fn reviewed_manifest() -> String {
    r#"extension "acme/guide" {
  schema_version = 3
  version = "1.0.0"
  route = "guide"
  requires_use = ">=0.3.0, <0.4.0"
  actions = ["read"]

  repository {
    url = "https://github.com/acme/guide"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }

  flow "review" {
    engine = "a3s-flow"
    runtime = "native-ts"
    source = "flows/review.ts"
    export = "run"
    requires_tool = []
    requires_mcp = []
    requires_okf = []
    optional = false
  }

  skill "main" {
    path = "skills/main/SKILL.md"
    requires_tool = []
    requires_mcp = []
    requires_okf = []
    optional = false
  }

  ui "review" {
    title = "Reviewed Guide"
    description = "Review the signed guide package."
    icon = "book-open"
    order = 80
    entry = "web/activity.html"
    styles = ["web/activity.css"]
    scripts = ["web/activity.js"]
    skill = "main"
    bind_tool = []
    bind_mcp = []
    bind_flow = []
    optional = false
  }
}
"#
    .to_string()
}

#[allow(clippy::too_many_arguments)]
fn make_reviewed_use_fixture(
    temp: &TempWorkspace,
    directory: &Path,
    package_root: &Path,
    skill: &str,
    activity_html: &str,
    activity_css: &str,
    activity_js: &str,
    flow_source: &str,
) {
    fs::create_dir_all(directory).expect("create reviewed Use fixture");
    let registry = temp.path("state/use/registry.json");
    let route = json!({
        "id": REVIEWED_COMPONENT_ID,
        "route": "guide",
        "version": REVIEWED_VERSION,
        "origin": "extension",
        "enabled": true,
        "readiness": "ready",
        "packageRoot": package_root,
        "lifecycleGeneration": 1,
        "surfaces": ["flow", "skill", "ui"],
        "skills": [{
            "path": package_root.join("skills/main/SKILL.md"),
            "sha256": sha256(skill.as_bytes()),
        }],
        "flows": [{
            "id": "review",
            "engine": "a3s-flow",
            "runtime": "native-ts",
            "source": {
                "path": package_root.join("flows/review.ts"),
                "sha256": sha256(flow_source.as_bytes()),
                "mediaType": "text/typescript",
            },
            "exportName": "run",
            "requiresTools": [],
            "requiresMcp": [],
            "requiresOkf": [],
        }],
        "activityBar": [{
            "id": "review",
            "title": "Reviewed Guide",
            "description": "Review the signed guide package.",
            "icon": "book-open",
            "entry": {
                "path": package_root.join("web/activity.html"),
                "sha256": sha256(activity_html.as_bytes()),
                "mediaType": "text/html",
            },
            "styles": [{
                "path": package_root.join("web/activity.css"),
                "sha256": sha256(activity_css.as_bytes()),
                "mediaType": "text/css",
            }],
            "scripts": [{
                "path": package_root.join("web/activity.js"),
                "sha256": sha256(activity_js.as_bytes()),
                "mediaType": "text/javascript",
            }],
            "skill": "guide",
            "order": 80,
        }],
    });
    let mut disabled_route = route.clone();
    disabled_route["enabled"] = Value::Bool(false);
    disabled_route
        .as_object_mut()
        .expect("disabled route object")
        .remove("flows");
    let snapshots = [
        ("empty", snapshot_envelope(0, "0", Vec::new())),
        (
            "enabled-one",
            snapshot_envelope(1, "1", vec![route.clone()]),
        ),
        (
            "disabled-two",
            snapshot_envelope(2, "2", vec![disabled_route]),
        ),
        ("enabled-three", snapshot_envelope(3, "3", vec![route])),
    ];
    for (name, snapshot) in snapshots {
        fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .expect("write capability snapshot");
        let changed = json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "changed": true,
                "registry": snapshot["data"]["registry"],
            },
        });
        fs::write(
            directory.join(format!("{name}-changed.json")),
            serde_json::to_vec(&changed).unwrap(),
        )
        .expect("write capability watch result");
    }
    fs::write(
        directory.join("unchanged.json"),
        br#"{"schemaVersion":1,"ok":true,"data":{"changed":false}}"#,
    )
    .expect("write unchanged capability result");

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

snapshot_file() {{
  case "$1" in
    3) printf '%s\n' {enabled_three} ;;
    2) printf '%s\n' {disabled_two} ;;
    1) printf '%s\n' {enabled_one} ;;
    *) printf '%s\n' {empty} ;;
  esac
}}

if [ "$1" = "capability" ] && [ "$2" = "snapshot" ]; then
  generation=$(current_generation)
  /bin/cat "$(snapshot_file "$generation")"
  exit 0
fi
if [ "$1" = "capability" ] && [ "$2" = "watch" ]; then
  generation=$(current_generation)
  if [ "$generation" -gt "$4" ]; then
    case "$generation" in
      3) /bin/cat {enabled_three_changed} ;;
      2) /bin/cat {disabled_two_changed} ;;
      1) /bin/cat {enabled_one_changed} ;;
      *) /bin/cat {empty_changed} ;;
    esac
  else
    /bin/sleep 0.05
    /bin/cat {unchanged}
  fi
  exit 0
fi
if [ "$1" = "component" ] && [ "$2" = "list" ]; then
  printf '{{"schemaVersion":1,"ok":true,"data":{{"components":[]}}}}\n'
  exit 0
fi
exit 2
"#,
            registry = sh_quote(&registry),
            enabled_three = sh_quote(&directory.join("enabled-three.json")),
            disabled_two = sh_quote(&directory.join("disabled-two.json")),
            enabled_one = sh_quote(&directory.join("enabled-one.json")),
            empty = sh_quote(&directory.join("empty.json")),
            enabled_three_changed = sh_quote(&directory.join("enabled-three-changed.json")),
            disabled_two_changed = sh_quote(&directory.join("disabled-two-changed.json")),
            enabled_one_changed = sh_quote(&directory.join("enabled-one-changed.json")),
            empty_changed = sh_quote(&directory.join("empty-changed.json")),
            unchanged = sh_quote(&directory.join("unchanged.json")),
        ),
    );
}

fn reviewed_identity(plan: &Value) -> (String, String) {
    let operation_id = plan["operationId"]
        .as_str()
        .unwrap_or_else(|| panic!("reviewed plan operation ID: {plan:#}"));
    let digest = plan["canonicalPlanDigest"]
        .as_str()
        .or_else(|| plan["planDigest"].as_str())
        .unwrap_or_else(|| panic!("reviewed plan digest: {plan:#}"));
    (operation_id.to_string(), digest.to_string())
}

fn package_fingerprint(root: &Path) -> (String, u64, u64) {
    fn collect(root: &Path, directory: &Path, files: &mut Vec<(String, std::path::PathBuf)>) {
        let mut entries = fs::read_dir(directory)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
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
        let size = fs::metadata(path).unwrap().len();
        expanded_bytes += size;
        digest.update((relative.len() as u64).to_be_bytes());
        digest.update(relative.as_bytes());
        digest.update(size.to_be_bytes());
        let mut input = fs::File::open(path).unwrap();
        let mut buffer = Vec::new();
        input.read_to_end(&mut buffer).unwrap();
        digest.update(buffer);
    }
    (
        format!("{:x}", digest.finalize()),
        files.len() as u64,
        expanded_bytes,
    )
}
