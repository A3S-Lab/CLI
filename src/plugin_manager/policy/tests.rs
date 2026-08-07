use a3s_use_core::{
    PlanActor, PlanAuthority, PlanEnforcementProfile, PlanPackageChangeKind, PlanPackageRole,
    PlanPolicyDecision, PlanQualifiedSurfaceRef, PlanScope, PlanScopeKind, PlannedOperationImpact,
    PlannedPackageState, PlannedPackageTransition, PlannedPluginRelease, PlannedProviderEvidence,
    PlannedSecretChange, PlannedSecretChangeKind, PlannedStateEvidence, PlannedSurfaceChange,
    PlannedWorkspaceImpact, PluginCatalogRecord, PluginOperationAction, PluginOperationPlan,
    PluginPlanSource, PluginSurfaceKind, PluginSurfaceRef, SurfaceChangeKind,
    VerifiedCatalogProvenance, PLUGIN_OPERATION_PLAN_SCHEMA_V4,
};

use super::{
    PluginAuthorizationPolicy, PluginPolicyHandoff, PluginPolicyViolationCode, MAX_POLICY_BYTES,
    PLUGIN_POLICY_SCHEMA,
};

const CATALOG_RECORD: &[u8] = include_bytes!("../fixtures/complete-catalog-record-v3.json");
const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const DIGEST_D: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const DIGEST_E: &str = "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const DIGEST_F: &str = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

const ALLOW_POLICY: &str = r#"
model {
  provider = "fixture"
}

plugins {
  schema = "a3s.plugin-policy.v1"
  agent_install = "allow"
  agent_upgrade = "allow"
  agent_uninstall = "allow"
  trusted_registries = ["official"]
  trusted_publishers = ["acme"]
  allowed_surfaces = ["ui", "tool", "skill", "mcp"]
  max_download_bytes = 8388608
  max_installed_bytes = 16777216
  max_packages = 4
  max_surfaces = 8
  allow_user_scope = false
  workspace_ids = ["workspace:research"]
  max_workspaces = 1

  permissions {
    plugin_data = "read-write"
    temporary = "read-write"
    native_execution = true
    child_process = false
    private_service = true
    secrets = false
    ui_http = true
    ui_methods = ["post", "get"]
    max_ui_path_prefixes = 4
    max_cpu_millis = 2000
    max_memory_bytes = 1073741824
    max_pids = 256
    max_ephemeral_storage_bytes = 2147483648
    max_task_timeout_ms = 300000
    max_stdout_bytes = 8388608
    max_stderr_bytes = 2097152

    network "api.example.com" {
      ports = [443]
    }

    workspace "inputs" {
      access = "read"
    }
  }
}
"#;

#[test]
fn authorization_policy_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PluginAuthorizationPolicy>();
}

#[tokio::test]
async fn bounded_acl_file_loader_matches_in_memory_parsing() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("config.acl");
    tokio::fs::write(&path, ALLOW_POLICY).await.unwrap();

    assert_eq!(
        PluginAuthorizationPolicy::from_acl_file(&path)
            .await
            .unwrap(),
        PluginAuthorizationPolicy::from_acl(ALLOW_POLICY).unwrap()
    );

    tokio::fs::write(&path, " \n\t").await.unwrap();
    assert_eq!(
        PluginAuthorizationPolicy::from_acl_file(&path)
            .await
            .unwrap(),
        PluginAuthorizationPolicy::default()
    );

    let oversized = temporary.path().join("oversized.acl");
    tokio::fs::write(&oversized, vec![b' '; MAX_POLICY_BYTES + 1])
        .await
        .unwrap();
    assert!(PluginAuthorizationPolicy::from_acl_file(&oversized)
        .await
        .unwrap_err()
        .to_string()
        .contains("must not exceed"));
}

#[tokio::test]
async fn subprocess_handoff_reloads_only_the_digest_locked_policy() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("operator.acl");
    tokio::fs::write(&path, ALLOW_POLICY).await.unwrap();
    let policy = PluginAuthorizationPolicy::from_acl_file(&path)
        .await
        .unwrap();
    let handoff = PluginPolicyHandoff::new(&policy, Some(path.clone())).unwrap();

    assert_eq!(handoff.load_verified().await.unwrap(), policy);

    tokio::fs::write(&path, "plugins { schema = \"a3s.plugin-policy.v1\" }")
        .await
        .unwrap();
    let error = handoff.load_verified().await.unwrap_err();
    assert!(error
        .to_string()
        .contains("host plugin authorization changed after launch"));
}

#[tokio::test]
async fn default_policy_handoff_has_no_configuration_source() {
    let policy = PluginAuthorizationPolicy::default();
    let handoff = PluginPolicyHandoff::new(&policy, None).unwrap();

    assert!(handoff.source().is_none());
    assert_eq!(handoff.load_verified().await.unwrap(), policy);
}

fn qualified(kind: PluginSurfaceKind, id: &str) -> PlanQualifiedSurfaceRef {
    PlanQualifiedSurfaceRef {
        package_id: "acme/research".to_string(),
        surface: PluginSurfaceRef {
            kind,
            id: id.to_string(),
        },
    }
}

fn provider(
    kind: PluginSurfaceKind,
    id: &str,
    provider_id: &str,
    enforcement: PlanEnforcementProfile,
) -> PlannedProviderEvidence {
    PlannedProviderEvidence {
        surface: qualified(kind, id),
        provider_id: provider_id.to_string(),
        provider_build_id: "runtime:0.3.0:linux-x86_64".to_string(),
        capability_digest: DIGEST_D.to_string(),
        semantics_profile_digest: DIGEST_E.to_string(),
        enforcement,
    }
}

pub(crate) fn install_plan() -> PluginOperationPlan {
    let catalog = PluginCatalogRecord::from_json(CATALOG_RECORD).unwrap();
    let mut permissions = catalog.permission_ceiling.clone();
    permissions
        .surfaces
        .iter_mut()
        .find(|permission| permission.surface.id == "convert")
        .unwrap()
        .secrets
        .clear();
    let permission_ceiling_digest = permissions.descriptor_digest().unwrap();
    let surfaces = catalog
        .surfaces
        .iter()
        .map(|surface| PlannedSurfaceChange {
            surface: PluginSurfaceRef {
                kind: surface.kind,
                id: surface.id.clone(),
            },
            change: SurfaceChangeKind::Add,
            before_digest: None,
            after_digest: Some(surface.descriptor_digest().unwrap()),
        })
        .collect();
    let after = PlannedPackageState {
        release: PlannedPluginRelease {
            package_id: catalog.package_id.clone(),
            version: catalog.version.clone(),
            channel: catalog.channel,
            target: catalog.target.clone(),
            package_sha256: catalog.package.sha256.clone().unwrap(),
            manifest_sha256: DIGEST_C.to_string(),
            permission_ceiling_digest,
            surfaces: catalog.surfaces.clone(),
        },
        permissions,
    };
    let plan = PluginOperationPlan {
        schema: PLUGIN_OPERATION_PLAN_SCHEMA_V4.to_string(),
        operation_id: "install:acme-research:policy-0001".to_string(),
        created_at_ms: 1_785_360_000_000,
        expires_at_ms: 1_785_360_600_000,
        action: PluginOperationAction::Install,
        package_id: catalog.package_id.clone(),
        component_id: "runtime:local".to_string(),
        scope: PlanScope {
            kind: PlanScopeKind::Workspace,
            id: "workspace:research".to_string(),
        },
        prior_package_lock_digest: None,
        package_lock_digest: None,
        packages: vec![PlannedPackageTransition {
            package_id: catalog.package_id,
            role: PlanPackageRole::Root,
            change: PlanPackageChangeKind::Add,
            before: None,
            after: Some(after),
            source: Some(PluginPlanSource::Registry {
                provenance: VerifiedCatalogProvenance {
                    registry_name: "official".to_string(),
                    registry_url: "https://plugins.a3s.dev/catalog".to_string(),
                    root_sha256: DIGEST_F.to_string(),
                    root_version: 7,
                    timestamp_version: 42,
                    snapshot_version: 41,
                    targets_version: 39,
                    catalog_record_digest: DIGEST_E.to_string(),
                },
                archive: catalog.archive,
            }),
            surfaces,
        }],
        secret_changes: Vec::new(),
        providers: vec![
            provider(
                PluginSurfaceKind::Mcp,
                "library",
                "runtime-mcp-http",
                PlanEnforcementProfile::Container,
            ),
            provider(
                PluginSurfaceKind::Tool,
                "convert",
                "runtime-tool-task",
                PlanEnforcementProfile::Sandbox,
            ),
            provider(
                PluginSurfaceKind::Tool,
                "index",
                "runtime-tool-service",
                PlanEnforcementProfile::Container,
            ),
        ],
        workspace_impacts: vec![PlannedWorkspaceImpact {
            scope_id: "workspace:research".to_string(),
            grant_before_digest: None,
            grant_after_digest: Some(DIGEST_F.to_string()),
            enabled_before: false,
            enabled_after: true,
        }],
        impact: PlannedOperationImpact {
            download_bytes: 1_048_576,
            installed_bytes_after: 4_194_304,
            reclaimed_bytes: 0,
            drain_required: false,
            retained_data: false,
            okf_changes: Vec::new(),
        },
        authority: PlanAuthority {
            actor: PlanActor::Agent,
            decision: PlanPolicyDecision::Ask,
            policy_digest: DIGEST_A.to_string(),
            confirmation_required: true,
        },
        state: PlannedStateEvidence {
            state_revision: 3,
            capability_generation: 12,
            receipt_digest: None,
        },
    };
    plan.validate().unwrap();
    plan
}

fn enable_plan() -> PluginOperationPlan {
    let mut plan = install_plan();
    let state = plan.packages[0].after.clone().unwrap();
    plan.schema = PLUGIN_OPERATION_PLAN_SCHEMA_V4.to_string();
    plan.operation_id = "enable:acme-research:policy-0001".to_string();
    plan.action = PluginOperationAction::Enable;
    plan.packages = vec![PlannedPackageTransition::resolved(
        plan.package_id.clone(),
        PlanPackageRole::Root,
        PlanPackageChangeKind::Retain,
        Some(state.clone()),
        Some(state),
        None,
    )
    .unwrap()];
    plan.secret_changes.clear();
    plan.workspace_impacts[0].grant_before_digest = Some(DIGEST_E.to_string());
    plan.impact.download_bytes = 0;
    plan.impact.reclaimed_bytes = 0;
    plan.impact.drain_required = false;
    plan.impact.retained_data = false;
    plan.state.receipt_digest = Some(DIGEST_C.to_string());
    plan.validate().unwrap();
    plan
}

#[test]
fn acl_policy_is_strict_normalized_and_digest_stable() {
    a3s_code_core::CodeConfig::from_acl(ALLOW_POLICY)
        .expect("the host policy block must coexist with the normal A3S ACL parser");
    let first = PluginAuthorizationPolicy::from_acl(ALLOW_POLICY).unwrap();
    let reordered = ALLOW_POLICY
        .replace(
            r#"allowed_surfaces = ["ui", "tool", "skill", "mcp"]"#,
            r#"allowed_surfaces = ["mcp", "skill", "tool", "ui"]"#,
        )
        .replace(
            r#"ui_methods = ["post", "get"]"#,
            r#"ui_methods = ["get", "post"]"#,
        );
    let second = PluginAuthorizationPolicy::from_acl(&reordered).unwrap();

    assert_eq!(first, second);
    assert_eq!(
        first.descriptor_digest().unwrap(),
        second.descriptor_digest().unwrap()
    );
    assert!(first
        .descriptor_digest()
        .unwrap()
        .strip_prefix("sha256:")
        .is_some_and(|digest| digest.len() == 64));
}

#[test]
fn default_policy_requires_confirmation() {
    let policy = PluginAuthorizationPolicy::from_acl("model { name = \"fixture\" }").unwrap();
    let evaluation = policy.evaluate_plan(&install_plan()).unwrap();

    assert_eq!(evaluation.configured_decision, PlanPolicyDecision::Ask);
    assert_eq!(evaluation.decision, PlanPolicyDecision::Ask);
    assert!(evaluation.confirmation_required);
}

#[test]
fn exact_plan_within_every_ceiling_is_allowed() {
    let policy = PluginAuthorizationPolicy::from_acl(ALLOW_POLICY).unwrap();
    let mut plan = install_plan();
    let evaluation = policy.evaluate_plan(&plan).unwrap();

    assert_eq!(evaluation.configured_decision, PlanPolicyDecision::Allow);
    assert_eq!(evaluation.decision, PlanPolicyDecision::Allow);
    assert!(!evaluation.confirmation_required);
    assert!(evaluation.violations.is_empty());

    plan.authority = evaluation.authority();
    plan.validate().unwrap();
    assert_eq!(policy.verify_plan_authority(&plan).unwrap(), evaluation);
}

#[test]
fn agent_enable_uses_install_policy_and_rechecks_retained_permissions() {
    let allow = PluginAuthorizationPolicy::from_acl(ALLOW_POLICY).unwrap();
    let allowed = allow.evaluate_plan(&enable_plan()).unwrap();
    assert_eq!(allowed.configured_decision, PlanPolicyDecision::Allow);
    assert_eq!(allowed.decision, PlanPolicyDecision::Allow);

    let restricted = PluginAuthorizationPolicy::from_acl(
        &ALLOW_POLICY.replace("native_execution = true", "native_execution = false"),
    )
    .unwrap();
    let rejected = restricted.evaluate_plan(&enable_plan()).unwrap();
    assert_eq!(rejected.configured_decision, PlanPolicyDecision::Allow);
    assert_eq!(rejected.decision, PlanPolicyDecision::Ask);
    assert!(rejected.violations.iter().any(|violation| {
        violation.code == PluginPolicyViolationCode::NativeExecutionNotAllowed
    }));
}

#[test]
fn same_scope_user_grant_impact_is_not_treated_as_a_workspace() {
    let policy = PluginAuthorizationPolicy::from_acl(
        &ALLOW_POLICY
            .replace("allow_user_scope = false", "allow_user_scope = true")
            .replace(
                r#"workspace_ids = ["workspace:research"]"#,
                "workspace_ids = []",
            ),
    )
    .unwrap();
    let mut plan = install_plan();
    plan.scope = PlanScope {
        kind: PlanScopeKind::User,
        id: "current".to_string(),
    };
    plan.workspace_impacts[0].scope_id = plan.scope.id.clone();
    plan.validate().unwrap();

    let evaluation = policy.evaluate_plan(&plan).unwrap();

    assert_eq!(evaluation.decision, PlanPolicyDecision::Allow);
    assert!(!evaluation
        .violations
        .iter()
        .any(|violation| { violation.code == PluginPolicyViolationCode::WorkspaceNotAllowed }));
}

#[test]
fn manager_exposes_one_immutable_policy_to_every_adapter() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let policy = PluginAuthorizationPolicy::from_acl(ALLOW_POLICY).unwrap();
    let manager = crate::plugin_manager::PluginManager::new_with_policy(
        temporary.path().join("config.acl"),
        workspace,
        crate::components::ComponentPaths::for_test(temporary.path()),
        crate::registry::RegistryStore::new(temporary.path().join("registries")),
        crate::plugin_manager::PluginManagerPolicy {
            offline: true,
            authorization: policy.clone(),
        },
    );
    let mut plan = install_plan();
    let evaluation = manager.evaluate_plan_authority(&plan).unwrap();
    plan.authority = evaluation.authority();

    assert_eq!(manager.authorization_policy(), &policy);
    assert_eq!(manager.verify_plan_authority(&plan).unwrap(), evaluation);
}

#[test]
fn a_changed_policy_invalidates_reviewed_authority() {
    let policy = PluginAuthorizationPolicy::from_acl(ALLOW_POLICY).unwrap();
    let mut plan = install_plan();
    plan.authority = policy.evaluate_plan(&plan).unwrap().authority();
    let changed = PluginAuthorizationPolicy::from_acl(&ALLOW_POLICY.replace(
        "max_download_bytes = 8388608",
        "max_download_bytes = 524288",
    ))
    .unwrap();

    let error = changed.verify_plan_authority(&plan).unwrap_err();
    assert!(error.to_string().contains("changed after review"));
}

#[test]
fn exceeded_ceilings_downgrade_unattended_allow_to_ask() {
    let policy = PluginAuthorizationPolicy::from_acl(
        &ALLOW_POLICY
            .replace(
                "max_download_bytes = 8388608",
                "max_download_bytes = 524288",
            )
            .replace(
                r#"trusted_registries = ["official"]"#,
                r#"trusted_registries = ["other"]"#,
            )
            .replace(r#"workspace "inputs""#, r#"workspace "other""#),
    )
    .unwrap();
    let evaluation = policy.evaluate_plan(&install_plan()).unwrap();
    let codes = evaluation
        .violations
        .iter()
        .map(|violation| violation.code)
        .collect::<Vec<_>>();

    assert_eq!(evaluation.decision, PlanPolicyDecision::Ask);
    assert!(codes.contains(&PluginPolicyViolationCode::DownloadSizeExceeded));
    assert!(codes.contains(&PluginPolicyViolationCode::UntrustedRegistry));
    assert!(codes.contains(&PluginPolicyViolationCode::FilesystemNotAllowed));
}

#[test]
fn native_unconfined_provider_never_uses_unattended_allow() {
    let policy = PluginAuthorizationPolicy::from_acl(ALLOW_POLICY).unwrap();
    let mut plan = install_plan();
    plan.providers
        .iter_mut()
        .find(|provider| provider.surface.surface.id == "convert")
        .unwrap()
        .enforcement = PlanEnforcementProfile::NativeUnconfined;
    plan.validate().unwrap();

    let evaluation = policy.evaluate_plan(&plan).unwrap();
    assert_eq!(evaluation.decision, PlanPolicyDecision::Ask);
    assert!(evaluation
        .violations
        .iter()
        .any(|violation| violation.code == PluginPolicyViolationCode::NativeUnconfined));
}

#[test]
fn agent_secret_grant_is_denied_instead_of_prompted() {
    let policy = PluginAuthorizationPolicy::from_acl(ALLOW_POLICY).unwrap();
    let mut plan = install_plan();
    let after = plan.packages[0].after.as_mut().unwrap();
    let permission = after
        .permissions
        .surfaces
        .iter_mut()
        .find(|permission| permission.surface.id == "convert")
        .unwrap();
    permission.secrets.push("research-api".to_string());
    after.release.permission_ceiling_digest = after.permissions.descriptor_digest().unwrap();
    plan.secret_changes.push(PlannedSecretChange {
        surface: qualified(PluginSurfaceKind::Tool, "convert"),
        secret_name: "research-api".to_string(),
        change: PlannedSecretChangeKind::Grant,
    });
    plan.validate().unwrap();

    let evaluation = policy.evaluate_plan(&plan).unwrap();
    assert_eq!(evaluation.decision, PlanPolicyDecision::Deny);
    assert!(evaluation
        .violations
        .iter()
        .any(|violation| violation.code == PluginPolicyViolationCode::SecretsNotAllowed));
}

#[test]
fn unsupported_or_privilege_broadening_acl_fails_closed() {
    let unknown = ALLOW_POLICY.replace(
        "agent_install = \"allow\"",
        "agent_install = \"allow\"\n  package_script = true",
    );
    assert!(PluginAuthorizationPolicy::from_acl(&unknown)
        .unwrap_err()
        .to_string()
        .contains("package_script"));

    let secrets = ALLOW_POLICY.replace("secrets = false", "secrets = true");
    assert!(PluginAuthorizationPolicy::from_acl(&secrets)
        .unwrap_err()
        .to_string()
        .contains("unsupported"));

    let duplicate = format!("{ALLOW_POLICY}\nplugins {{ schema = \"{PLUGIN_POLICY_SCHEMA}\" }}");
    assert!(PluginAuthorizationPolicy::from_acl(&duplicate)
        .unwrap_err()
        .to_string()
        .contains("more than one"));

    let duplicate_network = ALLOW_POLICY.replace(
        r#"network "api.example.com" {
      ports = [443]
    }"#,
        r#"network "api.example.com" {
      ports = [443]
    }
    network "api.example.com" {
      ports = [8443]
    }"#,
    );
    assert!(PluginAuthorizationPolicy::from_acl(&duplicate_network)
        .unwrap_err()
        .to_string()
        .contains("duplicate network host"));
}
