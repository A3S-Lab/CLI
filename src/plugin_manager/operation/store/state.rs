use serde::{Deserialize, Serialize};

use super::{
    ensure_store_directories, invalid_store, read_optional_record, run_blocking, validate_digest,
    validate_operation_id, write_replace_record, PluginManagerError, PluginManagerResult,
    PluginOperationStore, StoredPluginPlan,
};

const PLANNER_STATE_SCHEMA: &str = "a3s.cli.plugin-planner-state.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredPlannerState {
    schema: String,
    revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_plan_digest: Option<String>,
}

impl StoredPlannerState {
    fn initial() -> Self {
        Self {
            schema: PLANNER_STATE_SCHEMA.to_string(),
            revision: 1,
            last_operation_id: None,
            last_plan_digest: None,
        }
    }

    fn validate(&self) -> PluginManagerResult<()> {
        if self.schema != PLANNER_STATE_SCHEMA || self.revision == 0 {
            return Err(invalid_store("plugin planner state identity is invalid"));
        }
        match (
            self.last_operation_id.as_deref(),
            self.last_plan_digest.as_deref(),
        ) {
            (None, None) if self.revision == 1 => Ok(()),
            (Some(operation_id), Some(plan_digest)) if self.revision > 1 => {
                if validate_operation_id(operation_id).is_err()
                    || validate_digest(plan_digest).is_err()
                {
                    return Err(invalid_store(
                        "plugin planner state operation evidence is invalid",
                    ));
                }
                Ok(())
            }
            _ => Err(invalid_store(
                "plugin planner state has incomplete operation evidence",
            )),
        }
    }

    fn matches(&self, plan: &StoredPluginPlan) -> bool {
        self.last_operation_id.as_deref() == Some(plan.operation_id.as_str())
            && self.last_plan_digest.as_deref() == Some(plan.plan_digest.as_str())
    }
}

impl PluginOperationStore {
    pub(in crate::plugin_manager::operation) async fn planner_state_revision(
        &self,
    ) -> PluginManagerResult<u64> {
        let store = self.clone();
        run_blocking("read plugin planner state", move || {
            Ok(store.read_planner_state_sync()?.revision)
        })
        .await
    }

    pub(in crate::plugin_manager::operation) async fn verify_planner_state(
        &self,
        plan: &StoredPluginPlan,
        intent_exists: bool,
    ) -> PluginManagerResult<()> {
        let store = self.clone();
        let plan = plan.clone();
        run_blocking("verify plugin planner state", move || {
            store.verify_planner_state_sync(&plan, intent_exists)
        })
        .await
    }

    pub(in crate::plugin_manager::operation) async fn advance_planner_state(
        &self,
        plan: &StoredPluginPlan,
    ) -> PluginManagerResult<u64> {
        let store = self.clone();
        let plan = plan.clone();
        run_blocking("advance plugin planner state", move || {
            store.advance_planner_state_sync(&plan)
        })
        .await
    }

    fn read_planner_state_sync(&self) -> PluginManagerResult<StoredPlannerState> {
        ensure_store_directories(self)?;
        let state = read_optional_record::<StoredPlannerState>(&self.planner_state_path())?
            .unwrap_or_else(StoredPlannerState::initial);
        state.validate()?;
        Ok(state)
    }

    fn verify_planner_state_sync(
        &self,
        plan: &StoredPluginPlan,
        intent_exists: bool,
    ) -> PluginManagerResult<()> {
        let Some(operation_plan) = plan.plugin_operation_plan.as_ref() else {
            return Ok(());
        };
        let expected = operation_plan.plan.state.state_revision;
        let state = self.read_planner_state_sync()?;
        if state.revision == expected
            || (intent_exists
                && state.revision == expected.saturating_add(1)
                && state.matches(plan))
        {
            return Ok(());
        }
        Err(PluginManagerError::OperationFailed(
            "durable plugin planner state changed after review; create and review a new plan"
                .to_string(),
        ))
    }

    fn advance_planner_state_sync(&self, plan: &StoredPluginPlan) -> PluginManagerResult<u64> {
        let state = self.read_planner_state_sync()?;
        if state.matches(plan) {
            return Ok(state.revision);
        }
        if let Some(operation_plan) = plan.plugin_operation_plan.as_ref() {
            if state.revision != operation_plan.plan.state.state_revision {
                return Err(PluginManagerError::OperationFailed(
                    "durable plugin planner state changed before commit".to_string(),
                ));
            }
        }
        let next_revision = state.revision.checked_add(1).ok_or_else(|| {
            PluginManagerError::Infrastructure(
                "durable plugin planner state revision is exhausted".to_string(),
            )
        })?;
        let next = StoredPlannerState {
            schema: PLANNER_STATE_SCHEMA.to_string(),
            revision: next_revision,
            last_operation_id: Some(plan.operation_id.clone()),
            last_plan_digest: Some(plan.plan_digest.clone()),
        };
        next.validate()?;
        write_replace_record(&self.planner_state_path(), &next)?;
        Ok(next_revision)
    }

    fn planner_state_path(&self) -> std::path::PathBuf {
        self.root.join("planner-state.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_manager::capability::{
        PluginCapabilityEvidence, PluginCapabilityEvidenceStatus,
    };
    use crate::plugin_manager::policy::tests::install_plan;
    use crate::plugin_manager::process::{PluginLifecycleAction, PluginPlanRequest};

    #[tokio::test]
    async fn planner_state_is_monotonic_and_idempotent() {
        let temporary = tempfile::tempdir().unwrap();
        let store = PluginOperationStore::new(temporary.path().join("operations"));
        let digest = "a".repeat(64);
        let plan = store
            .create_plan(
                PluginPlanRequest {
                    action: PluginLifecycleAction::Install,
                    component_id: "use/acme/research".to_string(),
                    version: Some("2.0.0".to_string()),
                    channel: Some("stable".to_string()),
                },
                digest.clone(),
                PluginCapabilityEvidence {
                    status: PluginCapabilityEvidenceStatus::Verified,
                    observed_at_ms: 1,
                    generation: Some(1),
                    revision: Some("b".repeat(64)),
                    error: None,
                },
                serde_json::json!({
                    "dryRun": true,
                    "planDigest": digest,
                    "plans": [],
                }),
            )
            .await
            .unwrap();

        assert_eq!(store.planner_state_revision().await.unwrap(), 1);
        assert_eq!(store.advance_planner_state(&plan).await.unwrap(), 2);
        assert_eq!(store.advance_planner_state(&plan).await.unwrap(), 2);
        assert_eq!(store.planner_state_revision().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn full_plan_state_must_match_until_its_intent_advances() {
        let temporary = tempfile::tempdir().unwrap();
        let store = PluginOperationStore::new(temporary.path().join("operations"));
        let mut plan = test_plan(&store).await;
        let mut operation = install_plan();
        operation.operation_id = plan.operation_id.clone();
        operation.state.state_revision = 1;
        let envelope = a3s_use_core::PluginOperationPlanEnvelope::new(operation).unwrap();
        plan.plan_digest = envelope
            .plan_digest
            .strip_prefix("sha256:")
            .unwrap()
            .to_string();
        plan.plugin_operation_plan = Some(envelope);

        store.verify_planner_state(&plan, false).await.unwrap();
        assert_eq!(store.advance_planner_state(&plan).await.unwrap(), 2);
        assert!(store.verify_planner_state(&plan, false).await.is_err());
        store.verify_planner_state(&plan, true).await.unwrap();
    }

    async fn test_plan(store: &PluginOperationStore) -> StoredPluginPlan {
        let digest = "a".repeat(64);
        store
            .create_plan(
                PluginPlanRequest {
                    action: PluginLifecycleAction::Install,
                    component_id: "use/acme/research".to_string(),
                    version: Some("2.0.0".to_string()),
                    channel: Some("stable".to_string()),
                },
                digest.clone(),
                PluginCapabilityEvidence {
                    status: PluginCapabilityEvidenceStatus::Verified,
                    observed_at_ms: 1,
                    generation: Some(12),
                    revision: Some("b".repeat(64)),
                    error: None,
                },
                serde_json::json!({
                    "dryRun": true,
                    "planDigest": digest,
                    "plans": [],
                }),
            )
            .await
            .unwrap()
    }
}
