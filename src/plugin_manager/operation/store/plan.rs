#[cfg(test)]
use a3s_use_core::PlanActor;
#[cfg(test)]
use serde_json::Value;

use super::{
    ensure_request_valid, new_operation_id, now_ms, run_blocking, validate_capability_evidence,
    validate_digest, validate_plan_record, validate_plan_value, write_new_record, NewPluginPlan,
    PluginManagerError, PluginManagerResult, PluginOperationStore, PluginPlanIdentity,
    StoredPluginPlan, WriteDisposition, OPERATION_RECORD_SCHEMA, PLAN_LIFETIME_MS,
};
#[cfg(test)]
use super::{PluginCapabilityEvidence, PluginPlanRequest};
use crate::plugin_manager::process::PluginLifecycleAction;

impl PluginOperationStore {
    #[cfg(test)]
    pub(super) async fn create_plan(
        &self,
        request: PluginPlanRequest,
        plan_digest: String,
        capability_state: PluginCapabilityEvidence,
        plan: Value,
    ) -> PluginManagerResult<StoredPluginPlan> {
        let identity = self.allocate_plan_identity(request.action).await?;
        self.create_plan_for_actor(NewPluginPlan {
            identity,
            request,
            actor: PlanActor::User,
            plan_digest,
            upstream_plan_digest: None,
            capability_state,
            plan,
            plugin_operation_plan: None,
        })
        .await
    }

    pub(in crate::plugin_manager::operation) async fn allocate_plan_identity(
        &self,
        action: PluginLifecycleAction,
    ) -> PluginManagerResult<PluginPlanIdentity> {
        let store = self.clone();
        run_blocking("allocate reviewed plugin plan identity", move || {
            store.allocate_plan_identity_sync(action)
        })
        .await
    }

    pub(in crate::plugin_manager::operation) async fn create_plan_for_actor(
        &self,
        plan: NewPluginPlan,
    ) -> PluginManagerResult<StoredPluginPlan> {
        let store = self.clone();
        run_blocking("create reviewed plugin plan", move || {
            store.create_plan_sync(plan)
        })
        .await
    }

    fn create_plan_sync(&self, new_plan: NewPluginPlan) -> PluginManagerResult<StoredPluginPlan> {
        let NewPluginPlan {
            identity,
            request,
            actor,
            plan_digest,
            upstream_plan_digest,
            capability_state,
            plan,
            plugin_operation_plan,
        } = new_plan;
        ensure_request_valid(&request)?;
        validate_digest(&plan_digest)?;
        validate_capability_evidence(&capability_state)?;
        validate_plan_value(&plan, &plan_digest)?;
        let lifecycle_required = plugin_operation_plan.is_some();
        let record = StoredPluginPlan {
            schema: OPERATION_RECORD_SCHEMA.to_string(),
            operation_id: identity.operation_id,
            created_at_ms: identity.created_at_ms,
            expires_at_ms: identity.expires_at_ms,
            request,
            actor,
            plan_digest,
            upstream_plan_digest,
            capability_state,
            plan,
            plugin_operation_plan,
            lifecycle_required,
        };
        validate_plan_record(&record)?;
        let path = self.plan_path(&record.operation_id);
        match write_new_record(&path, &record)? {
            WriteDisposition::Created => Ok(record),
            WriteDisposition::AlreadyExists => Err(PluginManagerError::Infrastructure(
                "the allocated plugin operation ID already exists".to_string(),
            )),
        }
    }

    fn allocate_plan_identity_sync(
        &self,
        action: PluginLifecycleAction,
    ) -> PluginManagerResult<PluginPlanIdentity> {
        let now = now_ms()?;
        self.prune_sync(now)?;
        for _ in 0..4 {
            let operation_id = new_operation_id(action)?;
            if !self.plan_path(&operation_id).exists() {
                return Ok(PluginPlanIdentity {
                    operation_id,
                    created_at_ms: now,
                    expires_at_ms: now.saturating_add(PLAN_LIFETIME_MS),
                });
            }
        }
        Err(PluginManagerError::Infrastructure(
            "could not allocate a unique plugin operation ID".to_string(),
        ))
    }
}
