//! Deterministic human review projection for one immutable Plugin Manager plan.
//!
//! Machine adapters continue to expose the standard A3S Use contracts. This
//! projection gives CLI and TUI users the same bounded, ordered view of the
//! exact plan identity, graph, source, permission, and confirmation evidence.

use a3s_use_core::{
    PlanPackageChangeKind, PlannedPackageState, PluginOperationPlanEnvelope, PluginPackageLock,
};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPlanReviewField {
    /// Stable field name used by terminal adapters.
    pub label: String,
    /// Compact JSON preserving the exact reviewed values.
    pub value: String,
}

/// Build the single review document consumed by every interactive adapter.
///
/// Each value is compact JSON so terminal renderers can wrap it without
/// dropping exact strings or inventing a second presentation-specific digest.
pub fn plan_review_fields(
    envelope: &PluginOperationPlanEnvelope,
) -> Result<Vec<PluginPlanReviewField>, String> {
    envelope
        .validate()
        .map_err(|error| format!("immutable plugin plan is invalid: {error}"))?;
    let plan = &envelope.plan;
    let mut fields = Vec::new();
    push_json(
        &mut fields,
        "plan",
        &json!({
            "schema": plan.schema,
            "operationId": plan.operation_id,
            "planDigest": envelope.plan_digest,
            "createdAtMs": plan.created_at_ms,
            "expiresAtMs": plan.expires_at_ms,
            "action": plan.action,
            "packageId": plan.package_id,
            "componentId": plan.component_id,
            "scope": plan.scope,
            "packageLockDigest": plan.package_lock_digest,
            "priorPackageLockDigest": plan.prior_package_lock_digest,
        }),
    )?;
    push_json(
        &mut fields,
        "packageGraph",
        &json!({
            "planPackageOrder": plan
                .packages
                .iter()
                .map(|package| package.package_id.as_str())
                .collect::<Vec<_>>(),
            "candidate": lock_review(envelope.package_lock.as_ref())?,
            "prior": lock_review(envelope.prior_package_lock.as_ref())?,
        }),
    )?;

    for package in &plan.packages {
        push_json(
            &mut fields,
            format!("transition.{}", package.package_id),
            package,
        )?;
        let retained_state = package.after.as_ref().or(package.before.as_ref());
        let source = match (&package.source, retained_state) {
            (Some(source), _) => serde_json::to_value(source)
                .map_err(|error| format!("could not serialize package source: {error}"))?,
            (None, Some(state)) => retained_source(package.change, state),
            (None, None) => Value::Null,
        };
        push_json(
            &mut fields,
            format!("source.{}", package.package_id),
            &source,
        )?;
        match (&package.before, &package.after) {
            (Some(before), Some(after)) if before == after => push_json(
                &mut fields,
                format!("permissionCeiling.{}.current", package.package_id),
                &before.permissions,
            )?,
            (before, after) => {
                if let Some(before) = before {
                    push_json(
                        &mut fields,
                        format!("permissionCeiling.{}.before", package.package_id),
                        &before.permissions,
                    )?;
                }
                if let Some(after) = after {
                    push_json(
                        &mut fields,
                        format!("permissionCeiling.{}.after", package.package_id),
                        &after.permissions,
                    )?;
                }
            }
        }
    }

    push_json(&mut fields, "secretChanges", &plan.secret_changes)?;
    push_json(&mut fields, "providers", &plan.providers)?;
    push_json(&mut fields, "workspaceImpacts", &plan.workspace_impacts)?;
    push_json(&mut fields, "impact", &plan.impact)?;
    push_json(&mut fields, "state", &plan.state)?;
    push_json(
        &mut fields,
        "confirmationBoundary",
        &json!({
            "operationId": plan.operation_id,
            "planDigest": envelope.plan_digest,
            "actor": plan.authority.actor,
            "decision": plan.authority.decision,
            "policyDigest": plan.authority.policy_digest,
            "confirmationRequired": plan.authority.confirmation_required,
        }),
    )?;
    Ok(fields)
}

fn push_json(
    fields: &mut Vec<PluginPlanReviewField>,
    label: impl Into<String>,
    value: &impl Serialize,
) -> Result<(), String> {
    fields.push(PluginPlanReviewField {
        label: label.into(),
        value: serde_json::to_string(value)
            .map_err(|error| format!("could not serialize plugin review evidence: {error}"))?,
    });
    Ok(())
}

fn lock_review(lock: Option<&PluginPackageLock>) -> Result<Value, String> {
    let Some(lock) = lock else {
        return Ok(Value::Null);
    };
    let digest = lock
        .descriptor_digest()
        .map_err(|error| format!("could not digest package graph: {error}"))?;
    Ok(json!({
        "digest": digest,
        "rootPackageId": lock.root_package_id,
        "host": lock.host,
        "nodes": lock
            .packages
            .iter()
            .map(|package| json!({
                "packageId": package.package_id(),
                "version": package.version(),
                "dependencies": package.dependencies,
                "target": package.catalog.record.target,
                "packageSha256": package.catalog.record.package.sha256,
                "manifestSha256": package.catalog.record.package.manifest_sha256,
                "permissionCeilingDigest": package.catalog.record.permission_ceiling_digest,
                "registry": package.catalog.provenance,
                "archive": package.catalog.record.archive,
            }))
            .collect::<Vec<_>>(),
    }))
}

fn retained_source(change: PlanPackageChangeKind, state: &PlannedPackageState) -> Value {
    json!({
        "kind": match change {
            PlanPackageChangeKind::Remove => "installed-prior",
            PlanPackageChangeKind::Retain => "retained-installed",
            PlanPackageChangeKind::Add | PlanPackageChangeKind::Replace => "missing",
        },
        "packageId": state.release.package_id,
        "version": state.release.version,
        "channel": state.release.channel,
        "target": state.release.target,
        "packageSha256": state.release.package_sha256,
        "manifestSha256": state.release.manifest_sha256,
    })
}
