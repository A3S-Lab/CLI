use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use a3s_use_core::{
    CatalogArchive, CatalogAvailability, CatalogPackage, CatalogSurface, PluginCatalogRecord,
    PluginPermissionCeiling, PluginReleaseChannel, PLUGIN_CATALOG_SCHEMA_V3,
    PLUGIN_PERMISSION_SCHEMA,
};
use a3s_use_extension::ExtensionManifest;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::support::{
    a3s_bin, command_output_with_timeout, configure_component_env, TempWorkspace,
};
use super::tuf_test_support::{
    expanded_archive_fingerprint, host_target, package_directory_archive, TestRepository,
    TestServer, TestTarget, FUTURE,
};

const PACKAGE_ID: &str = "acme/report";
const COMPONENT_ID: &str = "use/acme/report";
const INITIAL_VERSION: &str = "1.0.0";
const UPGRADED_VERSION: &str = "2.0.0";
const ACTIVITY_KEY: &str = "report:reports";
#[cfg(unix)]
const TEST_WEB_WORKER_STACK_BYTES: &str = "2097152";

static GENERIC_WEB_PROCESS_TEST_LOCK: Mutex<()> = Mutex::new(());
const PROCESS_COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
#[cfg(windows)]
const PROCESS_STOP_TIMEOUT: Duration = Duration::from_secs(20);

#[test]
#[ignore = "requires A3S_USE_E2E_BIN pointing to a real a3s-use binary"]
fn real_marketplace_hot_plugs_a_generic_signed_package_across_restart() {
    let _guard = web_process_test_guard();
    e2e_stage("fixture.prepare");
    let use_binary = required_file("A3S_USE_E2E_BIN");
    let use_bin = use_binary.parent().expect("A3S Use binary parent");
    let temp = TempWorkspace::new("generic-real-web-plugin-marketplace");
    let first = report_target(&temp.path("package-v1"), INITIAL_VERSION, "fixture-web-v1");
    let next = report_target(&temp.path("package-v2"), UPGRADED_VERSION, "fixture-web-v2");
    let repository = TestRepository::with_targets(vec![first, next], 97, FUTURE);
    let registry_server = TestServer::start(repository.routes.clone());

    let workspace = temp.path("workspace");
    let web_dir = temp.path("web");
    let config = temp.path("config/config.acl");
    let session_state = temp.path("web-session-state");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&web_dir).expect("create Web assets");
    fs::create_dir_all(config.parent().expect("config parent")).expect("create config parent");
    fs::write(
        web_dir.join("index.html"),
        "<!doctype html><title>A3S generic plugin Marketplace integration</title>",
    )
    .expect("write Web fixture");
    fs::write(&config, test_config()).expect("write config fixture");

    e2e_stage("registry.enroll.start");
    enroll_registry(
        &temp,
        &config,
        use_bin,
        registry_server.base_url(),
        &repository.root_sha256,
    );
    e2e_stage("registry.enroll.complete");

    e2e_stage("web.initial.start");
    let (mut daemon, address) = start_web(
        &temp,
        &workspace,
        &web_dir,
        &config,
        use_bin,
        &session_state,
    );
    e2e_stage("web.initial.ready");
    assert_eq!(
        http_json(&address, "GET", "/api/v1/plugins/activities", None)["items"],
        json!([])
    );
    let marketplace = http_json(&address, "GET", "/api/v1/plugins/marketplace", None);
    assert_eq!(report_marketplace_item(&marketplace)["installed"], false);

    e2e_stage("install.plan.start");
    let plan = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/plan",
        Some(&json!({
            "action": "install",
            "componentId": COMPONENT_ID,
            "version": INITIAL_VERSION,
            "channel": "stable",
        })),
    );
    e2e_stage("install.plan.complete");
    assert_eq!(plan["dryRun"], true);
    let (operation_id, plan_digest) = reviewed_identity(&plan);
    e2e_stage("install.apply.start");
    let applied = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "operationId": operation_id,
            "planDigest": plan_digest,
        })),
    );
    e2e_stage("install.apply.complete");
    assert!(operation_changed(&applied), "{applied:#}");

    e2e_stage("install.projection.start");
    let installed = wait_for_activity(&address, ACTIVITY_KEY);
    e2e_stage("install.projection.complete");
    let installed_generation = installed["generation"]
        .as_u64()
        .expect("installed activity generation");
    assert_activity_version(&installed, INITIAL_VERSION);
    assert_activity_content(
        &address,
        "<!doctype html><main>fixture-web-v1</main>\n",
        &[&workspace, &temp.path("package-v1")],
    );
    assert_lifecycle(&temp, &use_binary, "install", None);
    e2e_stage("install.verified");

    e2e_stage("web.initial.stop.start");
    daemon.stop();
    wait_until_stopped(&address);
    e2e_stage("web.initial.stop.complete");
    e2e_stage("web.restart.start");
    let (mut daemon, address) = start_web(
        &temp,
        &workspace,
        &web_dir,
        &config,
        use_bin,
        &session_state,
    );
    e2e_stage("web.restart.ready");
    let restored = wait_for_activity(&address, ACTIVITY_KEY);
    assert_activity_version(&restored, INITIAL_VERSION);
    assert_activity_content(
        &address,
        "<!doctype html><main>fixture-web-v1</main>\n",
        &[&workspace, &temp.path("package-v1")],
    );
    e2e_stage("web.restart.restored");

    e2e_stage("upgrade.plan.start");
    let upgrade_plan = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/plan",
        Some(&json!({
            "action": "upgrade",
            "componentId": COMPONENT_ID,
        })),
    );
    e2e_stage("upgrade.plan.complete");
    assert_eq!(upgrade_plan["dryRun"], true);
    let (upgrade_operation_id, upgrade_digest) = reviewed_identity(&upgrade_plan);
    e2e_stage("upgrade.apply.start");
    let upgraded = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "operationId": upgrade_operation_id,
            "planDigest": upgrade_digest,
        })),
    );
    e2e_stage("upgrade.apply.complete");
    assert!(operation_changed(&upgraded), "{upgraded:#}");

    e2e_stage("upgrade.projection.start");
    let upgraded_catalog = wait_for_activity_after(&address, ACTIVITY_KEY, installed_generation);
    e2e_stage("upgrade.projection.complete");
    let upgraded_generation = upgraded_catalog["generation"]
        .as_u64()
        .expect("upgraded activity generation");
    assert_activity_version(&upgraded_catalog, UPGRADED_VERSION);
    assert_activity_content(
        &address,
        "<!doctype html><main>fixture-web-v2</main>\n",
        &[&workspace, &temp.path("package-v2")],
    );
    assert_lifecycle(&temp, &use_binary, "uninstall", Some("upgrade"));
    e2e_stage("upgrade.verified");

    e2e_stage("uninstall.plan.start");
    let uninstall_plan = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/plan",
        Some(&json!({
            "action": "uninstall",
            "componentId": COMPONENT_ID,
        })),
    );
    e2e_stage("uninstall.plan.complete");
    assert_eq!(uninstall_plan["dryRun"], true);
    let (uninstall_operation_id, uninstall_digest) = reviewed_identity(&uninstall_plan);
    e2e_stage("uninstall.apply.start");
    let uninstalled = http_json(
        &address,
        "POST",
        "/api/v1/plugins/operations/apply",
        Some(&json!({
            "operationId": uninstall_operation_id,
            "planDigest": uninstall_digest,
        })),
    );
    e2e_stage("uninstall.apply.complete");
    assert!(operation_changed(&uninstalled), "{uninstalled:#}");
    wait_for_activity_absent(&address, ACTIVITY_KEY, upgraded_generation);

    let removed_marketplace = http_json(&address, "GET", "/api/v1/plugins/marketplace", None);
    let removed = report_marketplace_item(&removed_marketplace);
    assert_eq!(removed["installed"], false);
    assert_eq!(removed["enabled"], false);
    let component_list = run_use_json(&temp, &use_binary, &["component", "list", "--json"]);
    assert!(component_list["data"]["components"]
        .as_array()
        .is_some_and(|components| components
            .iter()
            .all(|component| component["id"] != PACKAGE_ID)));
    e2e_stage("uninstall.verified");

    e2e_stage("web.final.stop.start");
    daemon.stop();
    wait_until_stopped(&address);
    e2e_stage("web.final.stop.complete");
}

fn e2e_stage(stage: &str) {
    eprintln!("[generic-web-e2e] {stage}");
}

fn report_target(package_root: &Path, version: &str, marker: &str) -> TestTarget {
    fs::create_dir_all(package_root.join("skills/fixture-report"))
        .expect("create Skill fixture directory");
    fs::create_dir_all(package_root.join("ui/reports")).expect("create UI fixture directory");
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
    fs::write(package_root.join("a3s-use-extension.acl"), &manifest)
        .expect("write manifest fixture");
    fs::write(
        package_root.join("README.md"),
        format!("# Report {version}\n\nSigned Web hot-plug fixture.\n"),
    )
    .expect("write README fixture");
    fs::write(
        package_root.join("skills/fixture-report/SKILL.md"),
        "---\nname: fixture-report\ndescription: Report fixture\n---\n# Report\n",
    )
    .expect("write Skill fixture");
    fs::write(
        package_root.join("ui/reports/index.html"),
        format!("<!doctype html><main>{marker}</main>\n"),
    )
    .expect("write UI fixture");
    fs::write(
        package_root.join("ui/reports/index.css"),
        "main { color: rebeccapurple; }\n",
    )
    .expect("write UI style fixture");
    fs::write(
        package_root.join("ui/reports/index.js"),
        "document.querySelector('main').dataset.ready = 'true';\n",
    )
    .expect("write UI script fixture");

    let archive = package_directory_archive(package_root);
    let (package_sha256, file_count, expanded_bytes, manifest_bytes) =
        expanded_archive_fingerprint(&archive);
    let parsed = ExtensionManifest::parse_acl(&manifest).expect("parse manifest fixture");
    let graph = parsed.plugin_surfaces().expect("build surface graph");
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
        package_id: PACKAGE_ID.to_string(),
        display_name: format!("Report {version}"),
        description: "Signed Web hot-plug integration fixture.".to_string(),
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
        permission_ceiling_digest: permission_ceiling
            .descriptor_digest()
            .expect("digest permission ceiling"),
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
    catalog.validate().expect("validate catalog fixture");

    TestTarget {
        archive,
        target_name,
        custom: Some(serde_json::to_value(catalog).expect("serialize catalog fixture")),
    }
}

fn assert_activity_version(catalog: &Value, expected_version: &str) {
    let activity = catalog["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["key"] == ACTIVITY_KEY))
        .unwrap_or_else(|| panic!("report Activity Bar item: {catalog:#}"));
    assert_eq!(activity["packageId"], COMPONENT_ID);
    assert_eq!(activity["version"], expected_version);
    assert_eq!(activity["skill"], "fixture-report");
}

fn assert_activity_content(address: &str, expected_html: &str, forbidden_paths: &[&Path]) {
    let content = http_json(
        address,
        "GET",
        "/api/v1/plugins/activities/report%3Areports",
        None,
    );
    assert_eq!(content["html"], expected_html);
    assert_eq!(
        content["styles"],
        json!(["main { color: rebeccapurple; }\n"])
    );
    assert_eq!(
        content["scripts"],
        json!(["document.querySelector('main').dataset.ready = 'true';\n"])
    );
    assert_path_free(&content, forbidden_paths);
}

fn assert_lifecycle(
    temp: &TempWorkspace,
    use_binary: &Path,
    expected_action: &str,
    expected_previous_action: Option<&str>,
) {
    let inspected = run_use_json(
        temp,
        use_binary,
        &["extension", "inspect", PACKAGE_ID, "--json"],
    );
    let lifecycle = &inspected["data"]["lifecycle"];
    assert_eq!(
        lifecycle["schema"],
        "a3s.use.plugin-lifecycle-diagnostic.v1"
    );
    assert_eq!(lifecycle["latest"]["action"], expected_action);
    assert_eq!(lifecycle["latest"]["status"], "completed");
    match expected_previous_action {
        Some(action) => assert_eq!(lifecycle["previous"]["action"], action),
        None => assert!(lifecycle.get("previous").is_none()),
    }
    let encoded = lifecycle.to_string();
    assert!(!encoded.contains("idempotencyKey"));
    assert!(!encoded.contains("credential"));
    assert!(!encoded.contains("token"));
}

fn report_marketplace_item(marketplace: &Value) -> &Value {
    marketplace["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["componentId"] == COMPONENT_ID)
        })
        .unwrap_or_else(|| panic!("signed report Marketplace item: {marketplace:#}"))
}

fn operation_changed(result: &Value) -> bool {
    result["operations"].as_array().is_some_and(|operations| {
        operations
            .iter()
            .any(|operation| operation["changed"] == true)
    })
}

fn run_use_json(temp: &TempWorkspace, use_binary: &Path, args: &[&str]) -> Value {
    let mut command = Command::new(use_binary);
    configure_component_env(&mut command, temp);
    command.args(args).env_remove("A3S_USE_HOME");
    let output = command_output_with_timeout(
        &mut command,
        PROCESS_COMMAND_TIMEOUT,
        &format!("run {} {}", use_binary.display(), args.join(" ")),
    );
    assert!(
        output.status.success(),
        "{} {} failed:\nstdout:\n{}\nstderr:\n{}",
        use_binary.display(),
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{} {} returned invalid JSON ({error}): {}",
            use_binary.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn required_file(name: &str) -> PathBuf {
    let value = std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required"));
    let path = PathBuf::from(value);
    assert!(path.is_absolute(), "{name} must be absolute");
    assert!(path.is_file(), "{name} is not a file: {}", path.display());
    path
}

fn web_process_test_guard() -> MutexGuard<'static, ()> {
    GENERIC_WEB_PROCESS_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    configure_windows_home(&mut command, temp);
    command
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
        .env_remove("A3S_USE_HOME");
    let output = command_output_with_timeout(
        &mut command,
        PROCESS_COMMAND_TIMEOUT,
        "enroll signed registry",
    );
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
    configure_windows_home(&mut command, temp);
    command
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
        .env_remove("A3S_FLOW_NATIVE_TS_COMPILER")
        .env_remove("A3S_USE_HOME")
        .current_dir(workspace);
    #[cfg(unix)]
    command.env("RUST_MIN_STACK", TEST_WEB_WORKER_STACK_BYTES);
    let output =
        command_output_with_timeout(&mut command, PROCESS_COMMAND_TIMEOUT, "start detached Web");
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

fn configure_windows_home(command: &mut Command, temp: &TempWorkspace) {
    #[cfg(windows)]
    command
        .env("USERPROFILE", temp.path("home"))
        .env("LOCALAPPDATA", temp.path("local-app-data"))
        .env("APPDATA", temp.path("app-data"));

    #[cfg(not(windows))]
    let _ = (command, temp);
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

fn reviewed_identity(plan: &Value) -> (String, String) {
    let operation_id = plan["operationId"]
        .as_str()
        .unwrap_or_else(|| panic!("reviewed plan operation ID: {plan:#}"));
    let digest = plan["canonicalPlanDigest"]
        .as_str()
        .unwrap_or_else(|| panic!("reviewed plan digest: {plan:#}"));
    (operation_id.to_string(), digest.to_string())
}

fn assert_path_free(value: &Value, forbidden_paths: &[&Path]) {
    let encoded = value.to_string();
    for path in forbidden_paths {
        let path = path.display().to_string();
        assert!(
            !encoded.contains(&path),
            "public Activity response leaked managed or workspace path `{path}`: {value:#}"
        );
    }
    assert!(!encoded.contains("sourcePath"), "{value:#}");
    assert!(!encoded.contains("packageRoot"), "{value:#}");
    assert!(!encoded.contains("entrypoint"), "{value:#}");
}

fn output_value<'a>(output: &'a str, prefix: &str) -> &'a str {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix).map(str::trim))
        .unwrap_or_else(|| panic!("missing '{prefix}' in output:\n{output}"))
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
        if !stop_process(self.pid) {
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

#[cfg(unix)]
fn stop_process(pid: u32) -> bool {
    Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(windows)]
fn stop_process(pid: u32) -> bool {
    let mut command = Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T", "/F"]);
    command_output_with_timeout(&mut command, PROCESS_STOP_TIMEOUT, "stop detached Web")
        .status
        .success()
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
