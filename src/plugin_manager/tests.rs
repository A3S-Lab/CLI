use std::collections::BTreeMap;

use a3s_use_core::{PluginCatalogRecord, VerifiedCatalogProvenance, VerifiedPluginCatalogRecord};
use a3s_use_extension::PluginCatalogHost;

use crate::components::ComponentPaths;
use crate::registry::RegistryStore;

use super::catalog::catalog_item;
use super::catalog::package_display_name;
use super::process::{normalize_plan_request, plugin_operation_args};
use super::{PluginLifecycleAction, PluginManager, PluginManagerPolicy, PluginPlanRequest};

const COMPLETE_CATALOG_RECORD: &[u8] = include_bytes!("fixtures/complete-catalog-record-v3.json");

#[test]
fn shared_manager_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PluginManager>();
}

#[test]
fn lifecycle_planning_arguments_are_dry_run_and_use_namespaced() {
    assert_eq!(
        plugin_operation_args(
            PluginLifecycleAction::Install,
            "use/a3s/science",
            Some("1.2.3"),
            Some("stable"),
        )
        .unwrap(),
        [
            "install",
            "use/a3s/science",
            "--version",
            "1.2.3",
            "--channel",
            "stable",
            "--dry-run",
        ]
    );
    assert!(plugin_operation_args(PluginLifecycleAction::Install, "code", None, None).is_err());
    assert!(plugin_operation_args(
        PluginLifecycleAction::Install,
        "use/a3s/science",
        None,
        None,
    )
    .is_ok());
    assert!(plugin_operation_args(
        PluginLifecycleAction::Install,
        "use/a3s/science",
        Some("1.2"),
        None,
    )
    .is_err());
}

#[test]
fn reviewed_plan_requests_are_stored_in_canonical_form() {
    let normalized = normalize_plan_request(&PluginPlanRequest {
        action: PluginLifecycleAction::Install,
        component_id: "  use/acme/research  ".to_string(),
        version: Some(" 2.0.0 ".to_string()),
        channel: Some(" stable ".to_string()),
    })
    .unwrap();

    assert_eq!(normalized.component_id, "use/acme/research");
    assert_eq!(normalized.version.as_deref(), Some("2.0.0"));
    assert_eq!(normalized.channel.as_deref(), Some("stable"));
}

#[test]
fn marketplace_display_names_remain_product_facing() {
    assert_eq!(package_display_name("a3s/science"), "\u{79d1}\u{7814}");
    assert_eq!(package_display_name("acme/data-tools"), "Data Tools");
}

#[test]
fn complete_catalog_record_preserves_surfaces_permissions_and_provenance() {
    let host = PluginCatalogHost::current().unwrap();
    let mut catalog_json: serde_json::Value =
        serde_json::from_slice(COMPLETE_CATALOG_RECORD).unwrap();
    catalog_json["target"] = serde_json::json!(host.target);
    catalog_json["archive"]["targetName"] = serde_json::json!(format!(
        "extensions/acme/research/2.0.0/stable/{}/acme-research-2.0.0-{}.tar.gz",
        host.target, host.target
    ));
    catalog_json["planning"]["targetName"] = serde_json::json!(format!(
        "extensions/acme/research/2.0.0/stable/{}/planning-v1.json",
        host.target
    ));
    let record =
        PluginCatalogRecord::from_json(&serde_json::to_vec(&catalog_json).unwrap()).unwrap();
    let catalog_record_digest = record.descriptor_digest().unwrap();
    let provenance = VerifiedCatalogProvenance {
        registry_name: "fixture".to_string(),
        registry_url: "http://127.0.0.1:43210/".to_string(),
        root_sha256: "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            .to_string(),
        root_version: 7,
        timestamp_version: 42,
        snapshot_version: 41,
        targets_version: 39,
        catalog_record_digest: catalog_record_digest.clone(),
    };
    let verified = VerifiedPluginCatalogRecord::new(record, provenance.clone()).unwrap();
    let installed = BTreeMap::from([("use/acme/research".to_string(), true)]);

    let item = catalog_item(verified, &installed).unwrap();

    assert_eq!(item.component_id, "use/acme/research");
    assert_eq!(
        item.catalog_schema.as_deref(),
        Some("a3s.use.plugin-catalog.v3")
    );
    assert_eq!(item.surface_kinds, ["mcp", "skill", "tool", "ui"]);
    assert_eq!(item.surfaces.len(), 5);
    assert_eq!(
        item.permission_ceiling_digest.as_deref(),
        Some("sha256:c30a64142e328d905af88be78d4141746e73c28cbd54d0bc0e57e0d52f3e4097")
    );
    assert!(item.permission_ceiling.is_some());
    assert_eq!(item.provenance, Some(provenance));
    assert_eq!(
        item.integrity_digest.as_deref(),
        Some(catalog_record_digest.as_str())
    );
    assert!(item.signed_plan_digest.as_ref().is_some_and(|value| {
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }));
    assert!(item.installed);
    assert!(item.enabled);
}

#[tokio::test]
async fn marketplace_reports_disabled_registry_without_browsing_it() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let config_path = temporary.path().join("config/a3s.acl");
    let registry_root = temporary.path().join("registries");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&registry_root).unwrap();
    std::fs::write(
        registry_root.join("disabled.acl"),
        format!(
            "registry \"disabled\" {{\n  enabled = false\n  managed_root = false\n  trust_root = \"sha256:{}\"\n  url = \"https://disabled.example/\"\n}}\n",
            "f".repeat(64)
        ),
    )
    .unwrap();
    let manager = PluginManager::new_with_policy(
        config_path,
        workspace,
        ComponentPaths::for_test(temporary.path()),
        RegistryStore::new(registry_root),
        PluginManagerPolicy {
            offline: true,
            authorization: super::PluginAuthorizationPolicy::default(),
        },
    );

    let snapshot = manager.marketplace_cached(&BTreeMap::new()).await.unwrap();
    let disabled = snapshot
        .registries
        .iter()
        .find(|source| source.name == "disabled")
        .unwrap();

    assert!(disabled.configured);
    assert!(!disabled.enabled);
    assert!(!disabled.verified);
    assert!(disabled.error.is_none());
    assert!(snapshot
        .items
        .iter()
        .all(|item| item.registry_name != "disabled"));
}

#[tokio::test]
async fn current_protocol_rejects_an_unlocked_plan_before_apply() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let config_path = temporary.path().join("config/a3s.acl");
    let calls_path = temporary.path().join("child-calls.log");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let executable = write_fake_a3s(temporary.path(), &calls_path);
    let mut component_paths = ComponentPaths::for_test(temporary.path());
    component_paths.current_exe = executable;
    let manager = PluginManager::new_with_policy(
        config_path,
        workspace,
        component_paths,
        RegistryStore::new(temporary.path().join("registries")),
        PluginManagerPolicy {
            offline: true,
            authorization: super::PluginAuthorizationPolicy::default(),
        },
    );
    let error = manager
        .plan_operation(&PluginPlanRequest {
            action: PluginLifecycleAction::Install,
            component_id: "use/acme/research".to_string(),
            version: Some("2.0.0".to_string()),
            channel: Some("stable".to_string()),
        })
        .await
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("requires exactly one component plan"));
    let calls = std::fs::read_to_string(calls_path).unwrap();
    assert_eq!(calls.lines().count(), 1);
    assert!(calls.lines().all(|call| call.contains("--offline")));
    assert!(calls.lines().all(|call| call.contains("--dry-run")));
}

#[cfg(windows)]
fn write_fake_a3s(root: &std::path::Path, calls_path: &std::path::Path) -> std::path::PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let executable = bin.join("a3s.cmd");
    let script = format!(
        "@echo off\r\n\
         echo %*>>\"{}\"\r\n\
         echo {{\"ok\":true,\"data\":{{\"dryRun\":true,\"planSchemaVersion\":1,\"planCommand\":\"install\",\"planDigest\":\"{}\",\"plans\":[]}}}}\r\n",
        calls_path.display(),
        "a".repeat(64),
    );
    std::fs::write(&executable, script).unwrap();
    executable
}

#[cfg(unix)]
fn write_fake_a3s(root: &std::path::Path, calls_path: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let executable = bin.join("a3s");
    let calls_path = calls_path.display().to_string().replace('\'', "'\"'\"'");
    let script = format!(
        "#!/bin/sh\n\
         printf '%s\\n' \"$*\" >> '{calls_path}'\n\
         printf '%s\\n' '{{\"ok\":true,\"data\":{{\"dryRun\":true,\"planSchemaVersion\":1,\"planCommand\":\"install\",\"planDigest\":\"{}\",\"plans\":[]}}}}'\n",
        "a".repeat(64),
    );
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    executable
}
