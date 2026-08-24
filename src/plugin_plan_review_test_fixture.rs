use a3s_use_core::{
    CatalogSurface, NetworkEgressPermission, PlanActor, PlanAuthority, PlanEnforcementProfile,
    PlanPackageChangeKind, PlanPackageRole, PlanPolicyDecision, PlanQualifiedSurfaceRef, PlanScope,
    PlanScopeKind, PlannedOperationImpact, PlannedPackageState, PlannedPackageTransition,
    PlannedPluginRelease, PlannedProviderEvidence, PlannedSecretChange, PlannedSecretChangeKind,
    PlannedStateEvidence, PluginOperationAction, PluginOperationPlan, PluginOperationPlanEnvelope,
    PluginPermissionCeiling, PluginPlanSource, PluginReleaseChannel, PluginSurfaceKind,
    PluginSurfaceRef, ResourcePermissionCeiling, SurfacePermissionCeiling, ToolWorkloadClass,
    PLUGIN_OPERATION_PLAN_SCHEMA_V4, PLUGIN_PERMISSION_SCHEMA,
};

const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DIGEST_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const DIGEST_D: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

pub(crate) fn install_envelope() -> PluginOperationPlanEnvelope {
    let packages = ["acme/base", "acme/guide"]
        .into_iter()
        .map(|package_id| {
            let state = package_state(package_id);
            PlannedPackageTransition::resolved(
                package_id,
                if package_id == "acme/guide" {
                    PlanPackageRole::Root
                } else {
                    PlanPackageRole::Dependency
                },
                PlanPackageChangeKind::Add,
                None,
                Some(state.clone()),
                Some(PluginPlanSource::ReleaseBundle {
                    bundle_digest: DIGEST_D.to_string(),
                    package_digest: state.release.package_sha256,
                }),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let qualified_surfaces = packages
        .iter()
        .map(|package| qualified(&package.package_id))
        .collect::<Vec<_>>();
    let plan = PluginOperationPlan {
        schema: PLUGIN_OPERATION_PLAN_SCHEMA_V4.to_string(),
        operation_id: "manager:install:acme-guide-review".to_string(),
        created_at_ms: 1_785_360_000_000,
        expires_at_ms: 1_785_360_600_000,
        action: PluginOperationAction::Install,
        package_id: "acme/guide".to_string(),
        component_id: "use/acme/guide".to_string(),
        scope: user_scope(),
        package_lock_digest: None,
        prior_package_lock_digest: None,
        secret_changes: qualified_surfaces
            .iter()
            .cloned()
            .map(|surface| PlannedSecretChange {
                surface,
                secret_name: "research-api".to_string(),
                change: PlannedSecretChangeKind::Grant,
            })
            .collect(),
        providers: qualified_surfaces
            .into_iter()
            .map(|surface| PlannedProviderEvidence {
                surface,
                provider_id: "runtime-tool-task".to_string(),
                provider_build_id: "runtime:fixture:linux-x86_64".to_string(),
                capability_digest: DIGEST_C.to_string(),
                semantics_profile_digest: DIGEST_D.to_string(),
                enforcement: PlanEnforcementProfile::Sandbox,
            })
            .collect(),
        packages,
        workspace_impacts: Vec::new(),
        impact: PlannedOperationImpact {
            download_bytes: 2_097_152,
            installed_bytes_after: 8_388_608,
            reclaimed_bytes: 0,
            drain_required: false,
            retained_data: false,
            okf_changes: Vec::new(),
        },
        authority: review_authority(),
        state: PlannedStateEvidence {
            state_revision: 7,
            capability_generation: 8,
            receipt_digest: None,
        },
    };
    PluginOperationPlanEnvelope::new(plan).unwrap()
}

pub(crate) fn disable_envelope() -> PluginOperationPlanEnvelope {
    let state = package_state("acme/guide");
    let plan = PluginOperationPlan {
        schema: PLUGIN_OPERATION_PLAN_SCHEMA_V4.to_string(),
        operation_id: "manager:disable:acme-guide-review".to_string(),
        created_at_ms: 1_785_360_000_000,
        expires_at_ms: 1_785_360_600_000,
        action: PluginOperationAction::Disable,
        package_id: "acme/guide".to_string(),
        component_id: "use/acme/guide".to_string(),
        scope: user_scope(),
        package_lock_digest: None,
        prior_package_lock_digest: None,
        packages: vec![PlannedPackageTransition::resolved(
            "acme/guide",
            PlanPackageRole::Root,
            PlanPackageChangeKind::Retain,
            Some(state.clone()),
            Some(state),
            None,
        )
        .unwrap()],
        secret_changes: vec![PlannedSecretChange {
            surface: qualified("acme/guide"),
            secret_name: "research-api".to_string(),
            change: PlannedSecretChangeKind::Revoke,
        }],
        providers: Vec::new(),
        workspace_impacts: Vec::new(),
        impact: PlannedOperationImpact {
            download_bytes: 0,
            installed_bytes_after: 4_194_304,
            reclaimed_bytes: 0,
            drain_required: false,
            retained_data: true,
            okf_changes: Vec::new(),
        },
        authority: review_authority(),
        state: PlannedStateEvidence {
            state_revision: 7,
            capability_generation: 8,
            receipt_digest: Some(DIGEST_C.to_string()),
        },
    };
    PluginOperationPlanEnvelope::new(plan).unwrap()
}

fn package_state(package_id: &str) -> PlannedPackageState {
    let surface = PluginSurfaceRef {
        kind: PluginSurfaceKind::Tool,
        id: "convert".to_string(),
    };
    let permissions = PluginPermissionCeiling {
        schema: PLUGIN_PERMISSION_SCHEMA.to_string(),
        surfaces: vec![SurfacePermissionCeiling {
            surface: surface.clone(),
            native_execution: true,
            child_process: false,
            filesystem: Vec::new(),
            network_egress: vec![NetworkEgressPermission {
                host: "api.example.com".to_string(),
                ports: vec![443],
            }],
            private_service: false,
            secrets: vec!["research-api".to_string()],
            resources: Some(ResourcePermissionCeiling {
                cpu_millis: 1_000,
                memory_bytes: 536_870_912,
                pids: 64,
                ephemeral_storage_bytes: 1_073_741_824,
                task_timeout_ms: Some(120_000),
                max_stdout_bytes: Some(4_194_304),
                max_stderr_bytes: Some(1_048_576),
            }),
            ui_http: Vec::new(),
        }],
    };
    let permission_ceiling_digest = permissions.descriptor_digest().unwrap();
    PlannedPackageState {
        release: PlannedPluginRelease {
            package_id: package_id.to_string(),
            version: "2.0.0".to_string(),
            channel: PluginReleaseChannel::Stable,
            target: "linux-x86_64".to_string(),
            package_sha256: DIGEST_A.to_string(),
            manifest_sha256: DIGEST_B.to_string(),
            permission_ceiling_digest,
            surfaces: vec![CatalogSurface {
                kind: PluginSurfaceKind::Tool,
                id: "convert".to_string(),
                optional: false,
                workload: Some(ToolWorkloadClass::Task),
                mcp_transport: None,
                mcp_tool_count: None,
                okf_bundle: None,
                requires: Vec::new(),
            }],
        },
        permissions,
    }
}

fn qualified(package_id: &str) -> PlanQualifiedSurfaceRef {
    PlanQualifiedSurfaceRef {
        package_id: package_id.to_string(),
        surface: PluginSurfaceRef {
            kind: PluginSurfaceKind::Tool,
            id: "convert".to_string(),
        },
    }
}

fn user_scope() -> PlanScope {
    PlanScope {
        kind: PlanScopeKind::User,
        id: "user/current".to_string(),
    }
}

fn review_authority() -> PlanAuthority {
    PlanAuthority {
        actor: PlanActor::User,
        decision: PlanPolicyDecision::Ask,
        policy_digest: DIGEST_D.to_string(),
        confirmation_required: true,
    }
}
