#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::Path;

use a3s_updater::{ComponentReceipt, InstallProvenance, RECEIPT_SCHEMA_VERSION};

use super::id::ComponentId;
use super::lifecycle::{install_component, uninstall_component, InstallRequest};
use super::paths::ComponentPaths;

#[test]
fn direct_uninstall_stops_use_and_removes_only_owned_files() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ComponentPaths::for_test(temp.path());
    let id = ComponentId::parse("use").unwrap();
    let install_root = paths.version_root(&id, "0.1.0");
    let executable = install_root.join("bin/a3s-use");
    write_executable(
        &executable,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf 'a3s-use 0.1.0\n'
  exit 0
fi
if [ "$1" = "mcp" ] && [ "$2" = "stop" ]; then
  printf '{"schemaVersion":1,"ok":true}\n'
  exit 0
fi
exit 2
"#,
    );
    let user_profile = paths.data_root.join("profiles/default/state");
    std::fs::create_dir_all(user_profile.parent().unwrap()).unwrap();
    std::fs::write(&user_profile, "keep").unwrap();
    let receipt = ComponentReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        component_id: id.to_string(),
        version: "0.1.0".to_string(),
        provenance: InstallProvenance::GithubRelease,
        install_root: install_root.clone(),
        executable_path: Some(executable),
        owned_paths: vec![install_root.clone()],
        source: None,
        artifact_checksums: BTreeMap::new(),
        installed_at: "2026-07-14T00:00:00Z".to_string(),
    };
    paths.receipt_store().write(&receipt).unwrap();

    let operation = uninstall_component(&id, false, false, &paths).unwrap();

    assert!(operation.changed);
    assert!(!install_root.exists());
    assert_eq!(std::fs::read_to_string(user_profile).unwrap(), "keep");
    assert!(paths.receipt_store().read("use").unwrap().is_none());
}

#[tokio::test]
async fn cognitive_package_install_requires_reviewed_registry_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ComponentPaths::for_test(temp.path());
    let request = InstallRequest::default();
    let id = ComponentId::parse("use/acme/slack").unwrap();

    let error = install_component(&id, &request, &paths).await.unwrap_err();

    assert!(error
        .to_string()
        .contains("no reviewed signed-Registry resolution"));
}

#[test]
fn uninstall_refuses_an_unowned_external_product() {
    let temp = tempfile::tempdir().unwrap();
    let mut paths = ComponentPaths::for_test(temp.path());
    let bin = temp.path().join("external");
    write_executable(
        &bin.join("a3s-use"),
        "#!/bin/sh\nprintf 'a3s-use 0.1.0\\n'\n",
    );
    paths.set_install_override("A3S_USE_INSTALL_DIR", bin);

    let error =
        uninstall_component(&ComponentId::parse("use").unwrap(), false, false, &paths).unwrap_err();

    assert!(error.to_string().contains("does not own"));
}

fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
