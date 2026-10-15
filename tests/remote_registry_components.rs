#![cfg(unix)]

mod support;

use std::process::{Command, Output};

use support::{a3s_bin, configure_component_env, make_executable, sh_quote, TempWorkspace};

#[path = "support/tuf_test_support.rs"]
mod tuf_test_support;

use tuf_test_support::{
    extension_archive, TestRepository, TestServer, EXPIRED, FUTURE, PACKAGE_VERSION,
};

fn science_extension_receipt(temp: &TempWorkspace) -> std::path::PathBuf {
    a3s::registry::extension_receipt_path(
        temp.path("data/use"),
        temp.path("state/use"),
        a3s::registry::default_user_installation(),
        "a3s/science",
    )
    .expect("default user/current science receipt path")
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
        .is_some_and(|message| message.contains("is not registered")));
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
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(science_extension_receipt(&temp)).unwrap()).unwrap();
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
    let upgraded_receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(science_extension_receipt(&temp)).unwrap()).unwrap();
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

fn write_capability_snapshot(path: &std::path::Path, generation: u64, revision_digit: char) {
    std::fs::create_dir_all(path.parent().expect("capability snapshot parent")).unwrap();
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "ok": true,
            "data": {
                "registry": {
                    "schemaVersion": 2,
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
