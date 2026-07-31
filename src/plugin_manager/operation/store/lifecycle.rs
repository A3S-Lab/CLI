use a3s_use::plugin_lifecycle::{PluginLifecycleCutoverEvidence, PluginLifecycleOperationBinding};

use super::super::super::capability::{PluginCapabilityEvidence, PluginCapabilityEvidenceStatus};
use super::super::super::{PluginManagerError, PluginManagerResult};
use super::{
    read_optional_record, run_blocking, write_new_record, write_replace_record,
    PluginOperationStore, StoredOperationResult, StoredPluginLifecycle, StoredPluginPlan,
    WriteDisposition,
};

const PLUGIN_LIFECYCLE_RECORD_SCHEMA: &str = "a3s.cli.plugin-lifecycle-record.v1";

impl PluginOperationStore {
    /// Persist the parent lifecycle binding before delegating package mutation.
    ///
    /// The currently live slice has no grant or Runtime child intents. Plans
    /// that require either child saga fail closed until the host injects and
    /// durably prepares those exact children.
    pub(in crate::plugin_manager::operation) async fn begin_lifecycle(
        &self,
        plan: &StoredPluginPlan,
        transitioned_at_ms: u64,
    ) -> PluginManagerResult<Option<StoredPluginLifecycle>> {
        let store = self.clone();
        let plan = plan.clone();
        run_blocking("persist plugin lifecycle binding", move || {
            store.begin_lifecycle_sync(&plan, transitioned_at_ms)
        })
        .await
    }

    pub(in crate::plugin_manager::operation) async fn complete_lifecycle(
        &self,
        plan: &StoredPluginPlan,
        capability_after: &PluginCapabilityEvidence,
        state_revision_after: u64,
        completed_at_ms: u64,
    ) -> PluginManagerResult<Option<StoredPluginLifecycle>> {
        let store = self.clone();
        let plan = plan.clone();
        let capability_after = capability_after.clone();
        run_blocking("persist plugin lifecycle cutover", move || {
            store.complete_lifecycle_sync(
                &plan,
                &capability_after,
                state_revision_after,
                completed_at_ms,
            )
        })
        .await
    }

    /// Validate the exact post-mutation capability publication before the
    /// manager advances its own durable planner revision.
    pub(in crate::plugin_manager::operation) async fn verify_lifecycle_observation(
        &self,
        plan: &StoredPluginPlan,
        capability_after: &PluginCapabilityEvidence,
    ) -> PluginManagerResult<Option<u64>> {
        let store = self.clone();
        let plan = plan.clone();
        let capability_after = capability_after.clone();
        run_blocking("verify plugin lifecycle observation", move || {
            let Some(_) = plan.plugin_operation_plan.as_ref() else {
                return Ok(None);
            };
            let path = store.lifecycle_path(&plan.operation_id);
            let record =
                read_optional_record::<StoredPluginLifecycle>(&path)?.ok_or_else(|| {
                    invalid_lifecycle_record("parent binding is absent after package mutation")
                })?;
            validate_lifecycle_record(&record, &plan)?;
            validate_capability_after(&record, &capability_after)?;
            Ok(Some(record.binding.state_revision_after()))
        })
        .await
    }

    fn begin_lifecycle_sync(
        &self,
        plan: &StoredPluginPlan,
        transitioned_at_ms: u64,
    ) -> PluginManagerResult<Option<StoredPluginLifecycle>> {
        let Some(operation_plan) = plan.plugin_operation_plan.as_ref() else {
            return Ok(None);
        };
        if requires_child_saga(&operation_plan.plan) {
            return Err(PluginManagerError::OperationFailed(
                "the reviewed plugin plan requires workspace-grant or Runtime child intents that are not injected into this Plugin Manager"
                    .to_string(),
            ));
        }
        let binding = PluginLifecycleOperationBinding::from_intents(
            operation_plan,
            transitioned_at_ms,
            &[],
            &[],
        )
        .map_err(lifecycle_operation_error)?;
        binding
            .verify_ready_for_cutover(&[], &[])
            .map_err(lifecycle_operation_error)?;
        let record = StoredPluginLifecycle {
            schema: PLUGIN_LIFECYCLE_RECORD_SCHEMA.to_string(),
            operation_id: plan.operation_id.clone(),
            plan_digest: plan.plan_digest.clone(),
            binding,
            cutover: None,
        };
        validate_lifecycle_record(&record, plan)?;
        let path = self.lifecycle_path(&plan.operation_id);
        match write_new_record(&path, &record)? {
            WriteDisposition::Created => Ok(Some(record)),
            WriteDisposition::AlreadyExists => {
                let existing =
                    read_optional_record::<StoredPluginLifecycle>(&path)?.ok_or_else(|| {
                        PluginManagerError::Infrastructure(
                            "durable plugin lifecycle record disappeared during replay".to_string(),
                        )
                    })?;
                validate_lifecycle_record(&existing, plan)?;
                if existing.binding != record.binding {
                    return Err(invalid_lifecycle_record(
                        "parent binding changed after apply intent persistence",
                    ));
                }
                Ok(Some(existing))
            }
        }
    }

    fn complete_lifecycle_sync(
        &self,
        plan: &StoredPluginPlan,
        capability_after: &PluginCapabilityEvidence,
        state_revision_after: u64,
        completed_at_ms: u64,
    ) -> PluginManagerResult<Option<StoredPluginLifecycle>> {
        let Some(_) = plan.plugin_operation_plan.as_ref() else {
            return Ok(None);
        };
        let path = self.lifecycle_path(&plan.operation_id);
        let mut record =
            read_optional_record::<StoredPluginLifecycle>(&path)?.ok_or_else(|| {
                invalid_lifecycle_record("parent binding is absent after package mutation")
            })?;
        validate_lifecycle_record(&record, plan)?;
        validate_capability_after(&record, capability_after)?;
        if state_revision_after != record.binding.state_revision_after() {
            return Err(PluginManagerError::OperationFailed(
                "the post-mutation planner revision does not match the reviewed lifecycle cutover"
                    .to_string(),
            ));
        }
        let revision = capability_after.revision.as_deref().ok_or_else(|| {
            PluginManagerError::OperationFailed(
                "the post-mutation capability snapshot is unavailable; lifecycle cutover remains pending"
                    .to_string(),
            )
        })?;
        let snapshot_digest = format!("sha256:{revision}");
        if let Some(cutover) = record.cutover.as_ref() {
            if cutover.capability_snapshot_digest() != snapshot_digest {
                return Err(invalid_lifecycle_record(
                    "capability snapshot changed after durable cutover",
                ));
            }
            return Ok(Some(record));
        }
        let cutover = PluginLifecycleCutoverEvidence::new(
            &record.binding,
            snapshot_digest,
            completed_at_ms,
            completed_at_ms,
        )
        .map_err(lifecycle_operation_error)?;
        record
            .binding
            .verify_completed(&cutover, &[], &[], completed_at_ms)
            .map_err(lifecycle_operation_error)?;
        record.cutover = Some(cutover);
        validate_lifecycle_record(&record, plan)?;
        write_replace_record(&path, &record)?;
        Ok(Some(record))
    }

    pub(in crate::plugin_manager::operation) fn validate_lifecycle_result_sync(
        &self,
        plan: &StoredPluginPlan,
        result: &StoredOperationResult,
    ) -> PluginManagerResult<()> {
        let binding_digest = lifecycle_output_field(result, "lifecycleBindingDigest");
        let cutover_digest = lifecycle_output_field(result, "lifecycleCutoverDigest");
        let snapshot_digest = lifecycle_output_field(result, "capabilitySnapshotDigest");
        if plan.plugin_operation_plan.is_none() {
            if binding_digest.is_some() || cutover_digest.is_some() || snapshot_digest.is_some() {
                return Err(invalid_lifecycle_record(
                    "legacy result acquired unrelated lifecycle evidence",
                ));
            }
            return Ok(());
        }
        let path = self.lifecycle_path(&plan.operation_id);
        let stored = read_optional_record::<StoredPluginLifecycle>(&path)?;
        if !plan.lifecycle_required
            && stored.is_none()
            && binding_digest.is_none()
            && cutover_digest.is_none()
            && snapshot_digest.is_none()
        {
            return Ok(());
        }
        let record = stored
            .ok_or_else(|| invalid_lifecycle_record("completed result has no parent binding"))?;
        validate_lifecycle_record(&record, plan)?;
        let cutover = record
            .cutover
            .as_ref()
            .ok_or_else(|| invalid_lifecycle_record("completed result has no parent cutover"))?;
        let capability_snapshot_digest = result
            .capability_after
            .revision
            .as_deref()
            .map(|revision| format!("sha256:{revision}"));
        if binding_digest != Some(record.binding.binding_digest())
            || cutover_digest != Some(cutover.cutover_digest())
            || snapshot_digest != Some(cutover.capability_snapshot_digest())
            || capability_snapshot_digest.as_deref() != Some(cutover.capability_snapshot_digest())
            || result.completed_at_ms != cutover.committed_at_ms()
            || result
                .data
                .get("stateRevisionAfter")
                .and_then(serde_json::Value::as_u64)
                != Some(record.binding.state_revision_after())
        {
            return Err(invalid_lifecycle_record(
                "completed result does not match its durable parent cutover",
            ));
        }
        Ok(())
    }
}

fn lifecycle_output_field<'a>(result: &'a StoredOperationResult, field: &str) -> Option<&'a str> {
    result.data.get(field).and_then(serde_json::Value::as_str)
}

fn requires_child_saga(plan: &a3s_use_core::PluginOperationPlan) -> bool {
    !plan.workspace_impacts.is_empty()
        || !plan.providers.is_empty()
        || !plan.secret_changes.is_empty()
        || plan.impact.drain_required
        || plan.packages.iter().any(|package| {
            package
                .before
                .iter()
                .chain(package.after.iter())
                .any(|state| {
                    !state.permissions.surfaces.is_empty()
                        || state.release.surfaces.iter().any(|surface| {
                            matches!(
                                surface.kind,
                                a3s_use_core::PluginSurfaceKind::Tool
                                    | a3s_use_core::PluginSurfaceKind::Mcp
                            )
                        })
                })
        })
}

fn validate_capability_after(
    record: &StoredPluginLifecycle,
    capability_after: &PluginCapabilityEvidence,
) -> PluginManagerResult<()> {
    if capability_after.status != PluginCapabilityEvidenceStatus::Verified
        || capability_after.revision.is_none()
    {
        return Err(PluginManagerError::OperationFailed(
            "the post-mutation capability snapshot is unavailable; lifecycle cutover remains pending"
                .to_string(),
        ));
    }
    if capability_after.generation != Some(record.binding.capability_generation_after()) {
        return Err(PluginManagerError::OperationFailed(
            "the post-mutation capability generation does not match the reviewed lifecycle cutover"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_lifecycle_record(
    record: &StoredPluginLifecycle,
    plan: &StoredPluginPlan,
) -> PluginManagerResult<()> {
    let operation_plan = plan.plugin_operation_plan.as_ref().ok_or_else(|| {
        invalid_lifecycle_record("legacy reviewed plan acquired parent lifecycle evidence")
    })?;
    record
        .binding
        .validate_against_plan(operation_plan)
        .map_err(|error| invalid_lifecycle_record(error.to_string()))?;
    record
        .binding
        .verify_ready_for_cutover(&[], &[])
        .map_err(|error| invalid_lifecycle_record(error.to_string()))?;
    let binding_plan_digest = record
        .binding
        .plan_digest()
        .strip_prefix("sha256:")
        .unwrap_or(record.binding.plan_digest());
    if record.schema != PLUGIN_LIFECYCLE_RECORD_SCHEMA
        || record.operation_id != plan.operation_id
        || record.plan_digest != plan.plan_digest
        || binding_plan_digest != plan.plan_digest
    {
        return Err(invalid_lifecycle_record(
            "parent identity does not match its reviewed plan",
        ));
    }
    if let Some(cutover) = &record.cutover {
        cutover
            .validate_against(&record.binding, u64::MAX)
            .map_err(|error| invalid_lifecycle_record(error.to_string()))?;
        record
            .binding
            .verify_completed(cutover, &[], &[], u64::MAX)
            .map_err(|error| invalid_lifecycle_record(error.to_string()))?;
    }
    Ok(())
}

fn lifecycle_operation_error(error: a3s_use_core::UseError) -> PluginManagerError {
    PluginManagerError::OperationFailed(format!("plugin lifecycle gate failed: {error}"))
}

fn invalid_lifecycle_record(message: impl Into<String>) -> PluginManagerError {
    PluginManagerError::Infrastructure(format!(
        "durable plugin lifecycle record is invalid: {}",
        message.into()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_manager::policy::tests::install_plan;

    #[test]
    fn empty_parent_binding_is_limited_to_permission_free_skill_or_ui_plans() {
        let mut plan = install_plan();
        assert!(requires_child_saga(&plan));

        plan.providers.clear();
        plan.workspace_impacts.clear();
        plan.secret_changes.clear();
        plan.impact.drain_required = false;
        for package in &mut plan.packages {
            for state in package.before.iter_mut().chain(package.after.iter_mut()) {
                state.permissions.surfaces.clear();
                state.release.surfaces.retain(|surface| {
                    matches!(
                        surface.kind,
                        a3s_use_core::PluginSurfaceKind::Skill
                            | a3s_use_core::PluginSurfaceKind::Ui
                    )
                });
            }
        }

        assert!(!requires_child_saga(&plan));
    }
}
