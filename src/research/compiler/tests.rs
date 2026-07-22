use super::test_support::*;
use super::*;

#[test]
fn one_dimension_can_require_workspace_and_web_authorities() {
    let spec = spec(
        EvidenceScope::WebAndWorkspace,
        vec![dimension(
            "dependency-policy",
            &["manifest", "upstream-policy"],
        )],
        vec![
            named_target(
                "manifest",
                "workspace_dependency_state",
                SourceRole::Primary,
                SourceIdentity::WorkspacePath("Cargo.toml".to_string()),
            ),
            named_target(
                "upstream-policy",
                "upstream_support_policy",
                SourceRole::Canonical,
                SourceIdentity::Repository("example/runtime".to_string()),
            ),
        ],
        budget(2, 2),
    );
    let plan = plan(
        &spec,
        vec![
            query(
                "q-workspace",
                AcquisitionTransport::Workspace,
                QueryMode::Exact,
                &["dependency-policy"],
                &["manifest"],
                1,
            ),
            query(
                "q-policy",
                AcquisitionTransport::Web,
                QueryMode::Exact,
                &["dependency-policy"],
                &["upstream-policy"],
                1,
            ),
        ],
        vec![],
    );

    let contract = validate_research_contract(spec, plan).expect("mixed contract");

    assert_eq!(contract.spec.dimensions.len(), 1);
    assert_eq!(contract.plan.queries.len(), 2);
}

#[test]
fn exploratory_target_requires_discovery_mode() {
    let spec = spec(
        EvidenceScope::Web,
        vec![dimension("framework-landscape", &["landscape"])],
        vec![exploratory_target(
            "landscape",
            "framework_landscape",
            "Select a bounded and diverse sample of current framework authorities.",
        )],
        budget(1, 2),
    );
    let exact = plan(
        &spec,
        vec![query(
            "q-landscape",
            AcquisitionTransport::Web,
            QueryMode::Exact,
            &["framework-landscape"],
            &["landscape"],
            2,
        )],
        vec![],
    );

    assert!(matches!(
        validate_research_contract(spec.clone(), exact),
        Err(ContractError::ExactQueryUsesExploratoryTarget { .. })
    ));

    let discovery = plan(
        &spec,
        vec![query(
            "q-landscape",
            AcquisitionTransport::Web,
            QueryMode::Discovery,
            &["framework-landscape"],
            &["landscape"],
            2,
        )],
        vec![],
    );
    validate_research_contract(spec, discovery).expect("exploratory discovery contract");
}

#[test]
fn unscheduled_material_dimension_requires_a_planning_gap() {
    let spec = spec(
        EvidenceScope::Web,
        vec![
            dimension("maintenance", &["maintenance-source"]),
            dimension("migration-cost", &["migration-source"]),
        ],
        vec![
            named_target(
                "maintenance-source",
                "maintenance_records",
                SourceRole::Primary,
                SourceIdentity::Domain("maintenance.example.test".to_string()),
            ),
            named_target(
                "migration-source",
                "migration_evidence",
                SourceRole::Primary,
                SourceIdentity::Domain("migration.example.test".to_string()),
            ),
        ],
        budget(1, 1),
    );
    let covered_query = query(
        "q-maintenance",
        AcquisitionTransport::Web,
        QueryMode::Exact,
        &["maintenance"],
        &["maintenance-source"],
        1,
    );

    let missing = plan(&spec, vec![covered_query.clone()], vec![]);
    assert!(matches!(
        validate_research_contract(spec.clone(), missing),
        Err(ContractError::MissingDimensionCoverage { dimension_id })
            if dimension_id == "migration-cost"
    ));

    let bounded = plan(
        &spec,
        vec![covered_query],
        vec![PlanningGap {
            dimension_id: "migration-cost".to_string(),
            missing_source_target_ids: vec!["migration-source".to_string()],
            reason:
                "The shared fetch budget was allocated to the higher-priority maintenance decision."
                    .to_string(),
        }],
    );
    validate_research_contract(spec, bounded).expect("explicit planning gap");
}

#[test]
fn named_targets_must_fit_their_query_fetch_allocation() {
    let spec = spec(
        EvidenceScope::Web,
        vec![dimension("policy", &["primary", "independent"])],
        vec![
            named_target(
                "primary",
                "policy_sources",
                SourceRole::Primary,
                SourceIdentity::Domain("primary.example.test".to_string()),
            ),
            named_target(
                "independent",
                "policy_sources",
                SourceRole::Independent,
                SourceIdentity::Domain("independent.example.test".to_string()),
            ),
        ],
        budget(1, 2),
    );
    let plan = plan(
        &spec,
        vec![query(
            "q-policy",
            AcquisitionTransport::Web,
            QueryMode::Discovery,
            &["policy"],
            &["primary", "independent"],
            1,
        )],
        vec![],
    );

    assert!(matches!(
        validate_research_contract(spec, plan),
        Err(ContractError::NamedTargetsExceedFetchAllocation {
            named_target_count: 2,
            fetch_slots: 1,
            ..
        })
    ));
}

#[test]
fn query_transport_must_match_every_target() {
    let spec = spec(
        EvidenceScope::WebAndWorkspace,
        vec![dimension("manifest", &["manifest-file"])],
        vec![named_target(
            "manifest-file",
            "workspace_files",
            SourceRole::Primary,
            SourceIdentity::WorkspacePath("Cargo.toml".to_string()),
        )],
        budget(1, 1),
    );
    let plan = plan(
        &spec,
        vec![query(
            "q-manifest",
            AcquisitionTransport::Web,
            QueryMode::Exact,
            &["manifest"],
            &["manifest-file"],
            1,
        )],
        vec![],
    );

    assert!(matches!(
        validate_research_contract(spec, plan),
        Err(ContractError::QueryTargetTransportMismatch { .. })
    ));
}

#[test]
fn partial_dimension_planning_gap_must_name_each_unscheduled_target() {
    let spec = spec(
        EvidenceScope::Web,
        vec![dimension("policy", &["primary", "independent"])],
        vec![
            named_target(
                "primary",
                "policy_primary",
                SourceRole::Primary,
                SourceIdentity::Domain("primary.example.test".to_string()),
            ),
            named_target(
                "independent",
                "policy_independent",
                SourceRole::Independent,
                SourceIdentity::Domain("independent.example.test".to_string()),
            ),
        ],
        budget(1, 1),
    );
    let primary_query = query(
        "q-primary",
        AcquisitionTransport::Web,
        QueryMode::Exact,
        &["policy"],
        &["primary"],
        1,
    );
    let unscoped_gap = plan(
        &spec,
        vec![primary_query.clone()],
        vec![PlanningGap {
            dimension_id: "policy".to_string(),
            missing_source_target_ids: vec![],
            reason: "The independent source did not fit the shared fetch budget.".to_string(),
        }],
    );
    assert!(matches!(
        validate_research_contract(spec.clone(), unscoped_gap),
        Err(ContractError::MissingTargetCoverage { target_id, .. })
            if target_id == "independent"
    ));

    let target_scoped_gap = plan(
        &spec,
        vec![primary_query],
        vec![PlanningGap {
            dimension_id: "policy".to_string(),
            missing_source_target_ids: vec!["independent".to_string()],
            reason: "The independent source did not fit the shared fetch budget.".to_string(),
        }],
    );
    validate_research_contract(spec, target_scoped_gap).expect("target-scoped planning gap");
}
