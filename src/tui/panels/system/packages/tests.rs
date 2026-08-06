use super::*;
use serde_json::json;

fn planned_value() -> Value {
    let digest = format!("sha256:{}", "a".repeat(64));
    json!({
        "schema": PACKAGE_PLAN_RESULT_SCHEMA,
        "componentId": "use/acme/guide",
        "packageId": "acme/guide",
        "expectedPackageGeneration": 7,
        "enabled": false,
        "status": "planned",
        "state": {"desired": "enabled", "packageGeneration": 7},
        "operationId": "plugin-enablement:guide-review",
        "canonicalPlanDigest": digest,
        "plan": {
            "planDigest": digest,
            "plan": {
                "operationId": "plugin-enablement:guide-review",
                "componentId": "use/acme/guide",
                "packageId": "acme/guide",
                "action": "disable",
                "expiresAtMs": 99,
                "authority": {"actor": "user"}
            }
        }
    })
}

fn installed(package_id: &str, version: &str, enabled: bool) -> PluginInstalledPackage {
    PluginInstalledPackage {
        component_id: format!("use/{package_id}"),
        package_id: package_id.to_string(),
        route: package_id.rsplit('/').next().unwrap().to_string(),
        version: version.to_string(),
        enabled,
        callable: false,
        readiness: PluginPackageReadiness::Ready,
        lifecycle_generation: Some(1),
        reconciliation: None,
        planner_evidence: None,
    }
}

fn snapshot(items: Vec<PluginInstalledPackage>) -> PluginInstallationSnapshot {
    PluginInstallationSnapshot {
        schema_version: 1,
        available: true,
        observed_at_ms: 1,
        generation: Some(2),
        revision: Some("a".repeat(64)),
        items,
        error: None,
    }
}

#[test]
fn plan_parser_requires_exact_user_identity_and_identity_free_no_change() {
    let outcome = parse_plan(&planned_value(), "use/acme/guide", false).unwrap();
    let PackagePlanOutcome::Planned(review) = outcome else {
        panic!("planned response should create a review");
    };
    assert_eq!(review.operation_id, "plugin-enablement:guide-review");
    assert_eq!(review.expected_package_generation, 7);

    let no_change = json!({
        "schema": PACKAGE_PLAN_RESULT_SCHEMA,
        "componentId": "use/acme/guide",
        "packageId": "acme/guide",
        "expectedPackageGeneration": 7,
        "enabled": false,
        "status": "no-change",
        "state": {"desired": "installed-disabled", "packageGeneration": 7}
    });
    assert_eq!(
        parse_plan(&no_change, "use/acme/guide", false).unwrap(),
        PackagePlanOutcome::NoChange
    );
    let mut invalid = no_change;
    invalid["operationId"] = json!("plugin-enablement:synthetic");
    assert!(parse_plan(&invalid, "use/acme/guide", false).is_err());

    let mut identity_drift = planned_value();
    identity_drift["packageId"] = json!("other/guide");
    assert!(parse_plan(&identity_drift, "use/acme/guide", false).is_err());

    let mut generation_drift = planned_value();
    generation_drift["state"]["packageGeneration"] = json!(8);
    assert!(parse_plan(&generation_drift, "use/acme/guide", false).is_err());
}

#[test]
fn apply_parser_requires_the_confirmed_identity_and_advanced_generation() {
    let PackagePlanOutcome::Planned(review) =
        parse_plan(&planned_value(), "use/acme/guide", false).unwrap()
    else {
        panic!("planned response should create a review");
    };
    let mut applied = json!({
        "schema": a3s_use::cognitive_package::COGNITIVE_PACKAGE_ENABLEMENT_RESULT_SCHEMA,
        "componentId": "use/acme/guide",
        "packageId": "acme/guide",
        "operationId": review.operation_id.clone(),
        "canonicalPlanDigest": review.plan_digest.clone(),
        "operationResultDigest": format!("sha256:{}", "b".repeat(64)),
        "durableEnablement": true,
        "changed": true,
        "replayed": false,
        "state": {"desired": "installed-disabled", "packageGeneration": 8}
    });
    assert_eq!(
        parse_apply_result(&applied, &review).unwrap(),
        PackageApplyOutcome {
            generation: 8,
            replayed: false,
        }
    );
    applied["operationId"] = json!("plugin-enablement:substituted");
    assert!(parse_apply_result(&applied, &review).is_err());
}

#[test]
fn review_rendering_preserves_the_complete_digest_and_width_bound() {
    let PackagePlanOutcome::Planned(review) =
        parse_plan(&planned_value(), "use/acme/guide", false).unwrap()
    else {
        panic!("planned response should create a review");
    };
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
fn package_list_is_sorted_and_renders_desired_and_callable_separately() {
    let mut panel = PackagePanel::loading(1, 0, Vec::new(), None);
    panel.apply_snapshot(snapshot(vec![
        installed("zeta/report", "1.0.0", true),
        installed("acme/guide", "2.0.0", false),
    ]));
    assert_eq!(panel.rows[0].package_id, "acme/guide");
    let plain = list_lines(&panel, 72, 24)
        .into_iter()
        .map(|line| a3s_tui::style::strip_ansi(&line))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plain.contains("desired=disabled"));
    assert!(plain.contains("desired=enabled"));
    assert!(plain.contains("callable=false"));

    panel.selected = 1;
    let mut refreshing = PackagePanel::loading(2, 1, panel.rows.clone(), None);
    refreshing.apply_snapshot(snapshot(vec![
        installed("alpha/new", "1.0.0", false),
        installed("acme/guide", "2.0.0", false),
        installed("zeta/report", "1.0.0", true),
    ]));
    assert_eq!(refreshing.selected_row().unwrap().package_id, "zeta/report");
}

#[test]
fn applying_phase_cannot_be_dismissed_and_review_requires_confirmation() {
    let PackagePlanOutcome::Planned(review) =
        parse_plan(&planned_value(), "use/acme/guide", false).unwrap()
    else {
        panic!("planned response should create a review");
    };
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
