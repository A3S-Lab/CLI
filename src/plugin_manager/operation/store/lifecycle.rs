use serde::Serialize;
use sha2::{Digest, Sha256};

use super::super::super::capability::{PluginCapabilityEvidence, PluginCapabilityEvidenceStatus};
use super::super::super::{PluginManagerError, PluginManagerResult};
use super::{
    read_optional_record, run_blocking, write_new_record, write_replace_record,
    HostPluginLifecycleBinding, HostPluginLifecycleCutover, PluginOperationStore,
    StoredOperationResult, StoredPluginLifecycle, StoredPluginPlan, WriteDisposition,
};

const PLUGIN_LIFECYCLE_RECORD_SCHEMA: &str = "a3s.cli.plugin-lifecycle-record.v2";
const PLUGIN_LIFECYCLE_BINDING_SCHEMA: &str = "a3s.cli.plugin-lifecycle-binding.v2";
const PLUGIN_LIFECYCLE_CUTOVER_SCHEMA: &str = "a3s.cli.plugin-lifecycle-cutover.v2";

impl HostPluginLifecycleBinding {
    fn new(
        plan: &a3s_use_core::PluginOperationPlanEnvelope,
        transitioned_at_ms: u64,
    ) -> PluginManagerResult<Self> {
        plan.validate().map_err(|error| {
            PluginManagerError::OperationFailed(format!(
                "plugin lifecycle gate rejected the reviewed plan: {error}"
            ))
        })?;
        let state_revision_after =
            plan.plan
                .state
                .state_revision
                .checked_add(1)
                .ok_or_else(|| {
                    PluginManagerError::OperationFailed(
                        "plugin lifecycle state revision is exhausted".to_string(),
                    )
                })?;
        let capability_generation_after = plan
            .plan
            .state
            .capability_generation
            .checked_add(1)
            .ok_or_else(|| {
                PluginManagerError::OperationFailed(
                    "plugin lifecycle capability generation is exhausted".to_string(),
                )
            })?;
        let mut binding = Self {
            schema: PLUGIN_LIFECYCLE_BINDING_SCHEMA.to_string(),
            operation_id: plan.plan.operation_id.clone(),
            plugin_plan_digest: plan.plan_digest.clone(),
            state_revision_before: plan.plan.state.state_revision,
            state_revision_after,
            capability_generation_before: plan.plan.state.capability_generation,
            capability_generation_after,
            transitioned_at_ms,
            binding_digest: String::new(),
        };
        binding.binding_digest = binding.calculate_digest()?;
        binding.validate_against_plan(plan)?;
        Ok(binding)
    }

    fn validate(&self) -> PluginManagerResult<()> {
        if self.schema != PLUGIN_LIFECYCLE_BINDING_SCHEMA
            || self.operation_id.is_empty()
            || !valid_sha256(&self.plugin_plan_digest)
            || self.state_revision_before == 0
            || self.state_revision_before.checked_add(1) != Some(self.state_revision_after)
            || self.capability_generation_before.checked_add(1)
                != Some(self.capability_generation_after)
            || self.transitioned_at_ms == 0
            || !valid_sha256(&self.binding_digest)
            || self.calculate_digest()? != self.binding_digest
        {
            return Err(invalid_lifecycle_record(
                "host lifecycle binding has invalid identity, revision, or digest evidence",
            ));
        }
        Ok(())
    }

    fn validate_against_plan(
        &self,
        plan: &a3s_use_core::PluginOperationPlanEnvelope,
    ) -> PluginManagerResult<()> {
        self.validate()?;
        plan.validate()
            .map_err(|error| invalid_lifecycle_record(error.to_string()))?;
        if self.operation_id != plan.plan.operation_id
            || self.plugin_plan_digest != plan.plan_digest
            || self.state_revision_before != plan.plan.state.state_revision
            || self.capability_generation_before != plan.plan.state.capability_generation
            || self.transitioned_at_ms < plan.plan.created_at_ms
            || self.transitioned_at_ms >= plan.plan.expires_at_ms
        {
            return Err(invalid_lifecycle_record(
                "host lifecycle binding does not match its reviewed plan",
            ));
        }
        Ok(())
    }

    fn calculate_digest(&self) -> PluginManagerResult<String> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct DigestInput<'a> {
            schema: &'a str,
            operation_id: &'a str,
            plugin_plan_digest: &'a str,
            state_revision_before: u64,
            state_revision_after: u64,
            capability_generation_before: u64,
            capability_generation_after: u64,
            transitioned_at_ms: u64,
        }
        digest_json(&DigestInput {
            schema: &self.schema,
            operation_id: &self.operation_id,
            plugin_plan_digest: &self.plugin_plan_digest,
            state_revision_before: self.state_revision_before,
            state_revision_after: self.state_revision_after,
            capability_generation_before: self.capability_generation_before,
            capability_generation_after: self.capability_generation_after,
            transitioned_at_ms: self.transitioned_at_ms,
        })
    }

    fn plan_digest(&self) -> &str {
        &self.plugin_plan_digest
    }

    pub(in crate::plugin_manager::operation) fn state_revision_after(&self) -> u64 {
        self.state_revision_after
    }

    pub(in crate::plugin_manager::operation) fn capability_generation_after(&self) -> u64 {
        self.capability_generation_after
    }

    pub(in crate::plugin_manager::operation) fn binding_digest(&self) -> &str {
        &self.binding_digest
    }
}

impl HostPluginLifecycleCutover {
    fn new(
        binding: &HostPluginLifecycleBinding,
        capability_snapshot_digest: impl Into<String>,
        committed_at_ms: u64,
        now_ms: u64,
    ) -> PluginManagerResult<Self> {
        binding.validate()?;
        let mut cutover = Self {
            schema: PLUGIN_LIFECYCLE_CUTOVER_SCHEMA.to_string(),
            operation_id: binding.operation_id.clone(),
            plugin_plan_digest: binding.plugin_plan_digest.clone(),
            lifecycle_binding_digest: binding.binding_digest.clone(),
            state_revision_after: binding.state_revision_after,
            capability_generation_after: binding.capability_generation_after,
            capability_snapshot_digest: capability_snapshot_digest.into(),
            committed_at_ms,
            cutover_digest: String::new(),
        };
        cutover.cutover_digest = cutover.calculate_digest()?;
        cutover.validate_against(binding, now_ms)?;
        Ok(cutover)
    }

    fn validate_against(
        &self,
        binding: &HostPluginLifecycleBinding,
        now_ms: u64,
    ) -> PluginManagerResult<()> {
        binding.validate()?;
        if self.schema != PLUGIN_LIFECYCLE_CUTOVER_SCHEMA
            || self.operation_id != binding.operation_id
            || self.plugin_plan_digest != binding.plugin_plan_digest
            || self.lifecycle_binding_digest != binding.binding_digest
            || self.state_revision_after != binding.state_revision_after
            || self.capability_generation_after != binding.capability_generation_after
            || !valid_sha256(&self.capability_snapshot_digest)
            || self.committed_at_ms < binding.transitioned_at_ms
            || self.committed_at_ms > now_ms
            || !valid_sha256(&self.cutover_digest)
            || self.calculate_digest()? != self.cutover_digest
        {
            return Err(invalid_lifecycle_record(
                "host lifecycle cutover does not match its binding or capability snapshot",
            ));
        }
        Ok(())
    }

    fn calculate_digest(&self) -> PluginManagerResult<String> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct DigestInput<'a> {
            schema: &'a str,
            operation_id: &'a str,
            plugin_plan_digest: &'a str,
            lifecycle_binding_digest: &'a str,
            state_revision_after: u64,
            capability_generation_after: u64,
            capability_snapshot_digest: &'a str,
            committed_at_ms: u64,
        }
        digest_json(&DigestInput {
            schema: &self.schema,
            operation_id: &self.operation_id,
            plugin_plan_digest: &self.plugin_plan_digest,
            lifecycle_binding_digest: &self.lifecycle_binding_digest,
            state_revision_after: self.state_revision_after,
            capability_generation_after: self.capability_generation_after,
            capability_snapshot_digest: &self.capability_snapshot_digest,
            committed_at_ms: self.committed_at_ms,
        })
    }

    pub(in crate::plugin_manager::operation) fn capability_snapshot_digest(&self) -> &str {
        &self.capability_snapshot_digest
    }

    pub(in crate::plugin_manager::operation) fn cutover_digest(&self) -> &str {
        &self.cutover_digest
    }

    pub(in crate::plugin_manager::operation) fn committed_at_ms(&self) -> u64 {
        self.committed_at_ms
    }
}

fn digest_json(value: &impl Serialize) -> PluginManagerResult<String> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        PluginManagerError::Infrastructure(format!(
            "failed to encode host plugin lifecycle evidence: {error}"
        ))
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

impl PluginOperationStore {
    /// Persist the parent lifecycle binding before delegating package mutation.
    ///
    /// A3S Use owns the locked cognitive-package graph, provider, and Grant
    /// saga. Generic reviewed plans still fail closed when they require a
    /// child lifecycle that this host has not durably injected.
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
        if requires_unavailable_host_child(
            &operation_plan.plan,
            operation_plan.package_lock.is_some(),
        ) {
            return Err(PluginManagerError::OperationFailed(
                "the reviewed plugin plan requires provider, secret, drain, Tool, MCP, or OKF child lifecycle evidence that is not injected into this Plugin Manager"
                    .to_string(),
            ));
        }
        let binding = HostPluginLifecycleBinding::new(operation_plan, transitioned_at_ms)?;
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
        let cutover = HostPluginLifecycleCutover::new(
            &record.binding,
            snapshot_digest,
            completed_at_ms,
            completed_at_ms,
        )?;
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

fn requires_unavailable_host_child(
    plan: &a3s_use_core::PluginOperationPlan,
    use_owns_package_graph: bool,
) -> bool {
    if use_owns_package_graph {
        return false;
    }
    !plan.providers.is_empty()
        || !plan.secret_changes.is_empty()
        || plan.impact.drain_required
        || plan.packages.iter().any(|package| {
            package
                .before
                .iter()
                .chain(package.after.iter())
                .any(|state| {
                    state.release.surfaces.iter().any(|surface| {
                        matches!(
                            surface.kind,
                            a3s_use_core::PluginSurfaceKind::Tool
                                | a3s_use_core::PluginSurfaceKind::Mcp
                                | a3s_use_core::PluginSurfaceKind::Okf
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
    record.binding.validate_against_plan(operation_plan)?;
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
        cutover.validate_against(&record.binding, u64::MAX)?;
    }
    Ok(())
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
    fn locked_cognitive_package_children_are_delegated_to_use() {
        let mut plan = install_plan();
        assert!(requires_unavailable_host_child(&plan, false));
        assert!(!requires_unavailable_host_child(&plan, true));
        assert!(!plan.workspace_impacts.is_empty());

        plan.providers.clear();
        plan.secret_changes.clear();
        plan.impact.drain_required = false;
        for package in &mut plan.packages {
            for state in package.before.iter_mut().chain(package.after.iter_mut()) {
                state.release.surfaces.retain(|surface| {
                    matches!(
                        surface.kind,
                        a3s_use_core::PluginSurfaceKind::Skill
                            | a3s_use_core::PluginSurfaceKind::Ui
                    )
                });
            }
        }

        assert!(!requires_unavailable_host_child(&plan, false));
    }
}
