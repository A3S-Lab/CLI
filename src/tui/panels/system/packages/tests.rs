use super::*;
use a3s_use_core::{
    PluginHostPackageState, PluginSurfaceKind, PluginSurfaceRef, PLUGIN_HOST_APPLY_RESULT_SCHEMA,
    PLUGIN_HOST_ENABLEMENT_PLAN_RESULT_SCHEMA, PLUGIN_MANAGED_SCOPE_SCHEMA_V2,
};

const DIGEST_A: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST_B: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DIGEST_C: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const DIGEST_D: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn scope() -> PluginManagedScope {
    PluginManagedScope {
        schema: PLUGIN_MANAGED_SCOPE_SCHEMA_V2.to_string(),
        host_id: "host:a3s-code".to_string(),
        scope_kind: PlanScopeKind::User,
        scope_id: a3s_use::COGNITIVE_PACKAGE_DEFAULT_SCOPE.to_string(),
        authority_id: "user:current".to_string(),
        fence_generation: 1,
        fence_digest: DIGEST_A.to_string(),
    }
}

fn package_state(
    version: &str,
    generation: u64,
    desired: PluginDesiredState,
    observed: PluginObservedState,
) -> PluginHostPackageState {
    PluginHostPackageState {
        version: Some(version.to_string()),
        package_generation: Some(generation),
        package_digest: Some(DIGEST_A.to_string()),
        manifest_digest: Some(DIGEST_B.to_string()),
        receipt_digest: Some(DIGEST_C.to_string()),
        capability_generation: generation + 1,
        capability_revision: DIGEST_D.to_string(),
        desired,
        observed,
        selected_surfaces: vec![PluginSurfaceRef {
            kind: PluginSurfaceKind::Skill,
            id: "guide".to_string(),
        }],
    }
}

fn installed(
    package_id: &str,
    version: &str,
    desired: PluginDesiredState,
    observed: PluginObservedState,
) -> PluginManagerInstalledPackage {
    PluginManagerInstalledPackage {
        package_id: package_id.to_string(),
        state: package_state(version, 7, desired, observed),
    }
}

fn review() -> PackagePlanReview {
    PackagePlanReview {
        component_id: "use/acme/guide".to_string(),
        package_id: "acme/guide".to_string(),
        enabled: false,
        expected_package_generation: 7,
        operation_id: "manager:enablement:guide-review".to_string(),
        plan_digest: DIGEST_A.to_string(),
        expires_at_ms: u64::MAX,
        desired_before: PluginDesiredState::Enabled,
        assignment_generation: 1,
        capabilities_digest: DIGEST_B.to_string(),
        scope: scope(),
    }
}

fn no_change_plan() -> PluginHostEnablementPlanResult {
    PluginHostEnablementPlanResult {
        schema: PLUGIN_HOST_ENABLEMENT_PLAN_RESULT_SCHEMA.to_string(),
        request_id: "manager:enablement:no-change".to_string(),
        assignment_generation: 1,
        capabilities_digest: DIGEST_B.to_string(),
        scope: scope(),
        package_id: PluginPackageId::parse("acme/guide").unwrap(),
        expected_package_generation: 7,
        enabled: false,
        planned_at_ms: 10,
        status: PluginHostEnablementPlanStatus::NoChange,
        state: package_state(
            "2.0.0",
            7,
            PluginDesiredState::InstalledDisabled,
            PluginObservedState::Installed,
        ),
        plan: None,
        replayed: false,
    }
}

fn applied_result(review: &PackagePlanReview) -> PluginHostApplyResult {
    PluginHostApplyResult {
        schema: PLUGIN_HOST_APPLY_RESULT_SCHEMA.to_string(),
        request_id: "manager:apply:guide-review".to_string(),
        assignment_generation: review.assignment_generation,
        capabilities_digest: review.capabilities_digest.clone(),
        scope: review.scope.clone(),
        package_id: PluginPackageId::parse(review.package_id.clone()).unwrap(),
        operation_id: review.operation_id.clone(),
        plan_digest: review.plan_digest.clone(),
        completed_at_ms: 20,
        operation_result_digest: DIGEST_C.to_string(),
        state: package_state(
            "2.0.0",
            review.expected_package_generation + 1,
            PluginDesiredState::InstalledDisabled,
            PluginObservedState::Installed,
        ),
        replayed: false,
    }
}

#[test]
fn typed_no_change_plan_has_no_mutation_identity() {
    assert_eq!(
        review_plan(no_change_plan(), "use/acme/guide", false).unwrap(),
        PackagePlanOutcome::NoChange
    );

    let mut drifted = no_change_plan();
    drifted.enabled = true;
    assert!(review_plan(drifted, "use/acme/guide", false).is_err());
}

#[test]
fn confirmation_binds_the_exact_operation_digest_and_user_actor() {
    let review = review();
    let confirmation = review.confirmation().unwrap();
    assert_eq!(confirmation.operation_id, review.operation_id);
    assert_eq!(confirmation.plan_digest, review.plan_digest);
    assert_eq!(confirmation.confirmed_by, PlanActor::User);
    assert!(confirmation.confirmed_at_ms > 0);
}

#[test]
fn apply_result_requires_the_confirmed_identity_and_advanced_generation() {
    let review = review();
    assert_eq!(
        review_apply_result(applied_result(&review), &review).unwrap(),
        PackageApplyOutcome {
            generation: 8,
            replayed: false,
        }
    );

    let mut substituted = applied_result(&review);
    substituted.operation_id = "manager:enablement:substituted".to_string();
    assert!(review_apply_result(substituted, &review).is_err());

    let mut stale = applied_result(&review);
    stale.state.package_generation = Some(review.expected_package_generation);
    assert!(review_apply_result(stale, &review).is_err());
}

#[test]
fn review_rendering_preserves_the_complete_digest_and_width_bound() {
    let review = review();
    let lines = review_lines(&review, None, 28);
    assert!(lines
        .iter()
        .all(|line| a3s_tui::style::visible_len(line) <= 28));
    let compact = lines.join("").replace(' ', "");
    assert!(compact.contains(&review.plan_digest));
    assert!(compact.contains(&review.operation_id));
    assert!(lines.iter().any(|line| line.contains("apply this exact")));
}

#[test]
fn package_list_is_sorted_and_renders_desired_and_observed_separately() {
    let mut panel = PackagePanel::loading(1, 0, Vec::new(), None);
    panel
        .apply_snapshot(vec![
            installed(
                "zeta/report",
                "1.0.0",
                PluginDesiredState::Enabled,
                PluginObservedState::Ready,
            ),
            installed(
                "acme/guide",
                "2.0.0",
                PluginDesiredState::InstalledDisabled,
                PluginObservedState::Installed,
            ),
        ])
        .unwrap();
    assert_eq!(panel.rows[0].package_id, "acme/guide");
    let plain = list_lines(&panel, 72, 24)
        .into_iter()
        .map(|line| a3s_tui::style::strip_ansi(&line))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plain.contains("desired=installed-disabled"));
    assert!(plain.contains("desired=enabled"));
    assert!(plain.contains("observed=installed"));
    assert!(plain.contains("observed=ready"));

    panel.selected = 1;
    let mut refreshing = PackagePanel::loading(2, 1, panel.rows.clone(), None);
    refreshing
        .apply_snapshot(vec![
            installed(
                "alpha/new",
                "1.0.0",
                PluginDesiredState::InstalledDisabled,
                PluginObservedState::Installed,
            ),
            installed(
                "acme/guide",
                "2.0.0",
                PluginDesiredState::InstalledDisabled,
                PluginObservedState::Installed,
            ),
            installed(
                "zeta/report",
                "1.0.0",
                PluginDesiredState::Enabled,
                PluginObservedState::Ready,
            ),
        ])
        .unwrap();
    assert_eq!(refreshing.selected_row().unwrap().package_id, "zeta/report");
}

#[test]
fn applying_phase_cannot_be_dismissed_and_review_requires_confirmation() {
    let review = review();
    let enter = KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::NONE,
    };
    let escape = KeyEvent {
        code: KeyCode::Esc,
        modifiers: KeyModifiers::NONE,
    };
    let control_enter = KeyEvent {
        code: KeyCode::Enter,
        modifiers: KeyModifiers::CONTROL,
    };
    assert_eq!(
        key_action(&PackagePanelPhase::Review(review.clone()), &enter),
        PackagePanelKeyAction::Apply
    );
    assert_eq!(
        key_action(&PackagePanelPhase::Review(review.clone()), &escape),
        PackagePanelKeyAction::Back
    );
    assert_eq!(
        key_action(&PackagePanelPhase::Review(review.clone()), &control_enter),
        PackagePanelKeyAction::Ignore
    );
    assert_eq!(
        key_action(&PackagePanelPhase::Applying(review), &escape),
        PackagePanelKeyAction::Ignore
    );
}
