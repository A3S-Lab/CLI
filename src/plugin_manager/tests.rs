use std::collections::BTreeMap;

use a3s_use_core::{PluginCatalogRecord, VerifiedCatalogProvenance, VerifiedPluginCatalogRecord};
use a3s_use_extension::PluginCatalogHost;

use crate::components::ComponentPaths;
use crate::registry::RegistryStore;

use super::catalog::catalog_item;
use super::catalog::package_display_name;
use super::process::{
    json_invocation_args, normalize_plan_request, plugin_operation_args, use_extension_toggle_args,
    JsonOutputOwner,
};
use super::{PluginApplyRequest, PluginLifecycleAction, PluginManager, PluginPlanRequest};

const COMPLETE_CATALOG_RECORD: &[u8] = include_bytes!("fixtures/complete-catalog-record-v1.json");

#[test]
fn shared_manager_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PluginManager>();
}

#[test]
fn lifecycle_arguments_require_reviewed_digest_and_use_namespace() {
    assert_eq!(
        plugin_operation_args(
            PluginLifecycleAction::Install,
            "use/a3s/science",
            Some("1.2.3"),
            Some("stable"),
            None,
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
    let digest = "a".repeat(64);
    let apply = plugin_operation_args(
        PluginLifecycleAction::Uninstall,
        "use/a3s/science",
        None,
        None,
        Some(&digest),
    )
    .unwrap();
    assert!(apply
        .windows(2)
        .any(|args| args == ["--plan-digest", digest.as_str()]));
    assert!(
        plugin_operation_args(PluginLifecycleAction::Install, "code", None, None, None).is_err()
    );
    assert!(plugin_operation_args(
        PluginLifecycleAction::Install,
        "use/a3s/science",
        None,
        None,
        Some("unsigned")
    )
    .is_err());
    assert!(plugin_operation_args(
        PluginLifecycleAction::Install,
        "use/a3s/science",
        Some("1.2"),
        None,
        None
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
fn use_toggle_keeps_json_output_owned_by_the_child_cli() {
    let invocation = json_invocation_args(
        JsonOutputOwner::UseProxy,
        use_extension_toggle_args("a3s/science", false),
    );

    assert_eq!(
        invocation,
        [
            "--non-interactive",
            "--no-progress",
            "use",
            "extension",
            "disable",
            "a3s/science",
            "--json",
        ]
    );
    assert!(!invocation
        .windows(2)
        .any(|arguments| arguments == ["--output", "json"]));
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
        Some("a3s.use.plugin-catalog.v1")
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
async fn reviewed_operation_replays_without_a_second_child_mutation() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let config_path = temporary.path().join("config/a3s.acl");
    let calls_path = temporary.path().join("child-calls.log");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    let executable = write_fake_a3s(temporary.path(), &calls_path);
    let mut component_paths = ComponentPaths::for_test(temporary.path());
    component_paths.current_exe = executable;
    let manager = PluginManager::new(
        config_path,
        workspace,
        component_paths,
        RegistryStore::new(temporary.path().join("registries")),
    );
    let plan_digest = "a".repeat(64);
    let plan = manager
        .plan_operation(&PluginPlanRequest {
            action: PluginLifecycleAction::Install,
            component_id: "use/acme/research".to_string(),
            version: Some("2.0.0".to_string()),
            channel: Some("stable".to_string()),
        })
        .await
        .unwrap();
    let operation_id = plan
        .get("operationId")
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .to_string();
    assert_eq!(
        plan.pointer("/capabilityState/status")
            .and_then(serde_json::Value::as_str),
        Some("unavailable")
    );
    let request = PluginApplyRequest {
        operation_id: Some(operation_id.clone()),
        action: None,
        component_id: None,
        version: None,
        channel: None,
        plan_digest: format!("sha256:{plan_digest}"),
    };

    let first = manager.apply_operation(&request).await.unwrap();
    let replay = manager.apply_operation(&request).await.unwrap();

    assert_eq!(
        first.get("operationId").and_then(serde_json::Value::as_str),
        Some(operation_id.as_str())
    );
    assert_eq!(
        first.get("replayed").and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        replay.get("replayed").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        std::fs::read_to_string(calls_path).unwrap().lines().count(),
        2
    );
}

#[cfg(windows)]
fn write_fake_a3s(root: &std::path::Path, calls_path: &std::path::Path) -> std::path::PathBuf {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let executable = bin.join("a3s.cmd");
    let script = format!(
        "@echo off\r\n\
         echo %*>>\"{}\"\r\n\
         echo %* | %SystemRoot%\\System32\\findstr.exe /C:\"--dry-run\" >nul\r\n\
         if not errorlevel 1 goto plan\r\n\
         echo {{\"ok\":true,\"data\":{{\"planDigest\":\"{}\",\"operations\":[]}}}}\r\n\
         exit /b 0\r\n\
         :plan\r\n\
         echo {{\"ok\":true,\"data\":{{\"dryRun\":true,\"planSchemaVersion\":1,\"planCommand\":\"install\",\"planDigest\":\"{}\",\"plans\":[]}}}}\r\n",
        calls_path.display(),
        "a".repeat(64),
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
         case \" $* \" in\n\
           *' --dry-run '*) printf '%s\\n' '{{\"ok\":true,\"data\":{{\"dryRun\":true,\"planSchemaVersion\":1,\"planCommand\":\"install\",\"planDigest\":\"{}\",\"plans\":[]}}}}' ;;\n\
           *) printf '%s\\n' '{{\"ok\":true,\"data\":{{\"planDigest\":\"{}\",\"operations\":[]}}}}' ;;\n\
         esac\n",
        "a".repeat(64),
        "a".repeat(64),
    );
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    executable
}
