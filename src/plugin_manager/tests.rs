use std::collections::BTreeMap;

use a3s_use_core::{PluginCatalogRecord, VerifiedCatalogProvenance, VerifiedPluginCatalogRecord};
use a3s_use_extension::PluginCatalogHost;

use super::catalog::catalog_item;
use super::catalog::package_display_name;
use super::process::{
    json_invocation_args, plugin_operation_args, use_extension_toggle_args, JsonOutputOwner,
};
use super::{PluginLifecycleAction, PluginManager};

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
