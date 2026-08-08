use std::path::{Path, PathBuf};

use a3s_use::cognitive_package::CognitivePackageEnablementResult;
use a3s_use_core::PluginOperationConfirmation;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    cognitive_request, operation_conflict, plan_invalid, store_invalid,
    PluginEnablementApplyRequest, PluginEnablementPlanResult, PluginEnablementPlanStatus,
};
use crate::plugin_manager::operation::store::io::{
    ensure_real_directory, read_optional_record, write_new_record, WriteDisposition,
};
use crate::plugin_manager::runtime_host::ReviewedEnablementRuntimeEvidence;
use crate::plugin_manager::{PluginManagerError, PluginManagerResult};

const PLAN_SCHEMA: &str = "a3s.cli.reviewed-enablement-plan.v2";
const APPLY_INTENT_SCHEMA: &str = "a3s.cli.reviewed-enablement-apply-intent.v1";
const APPLY_RESULT_SCHEMA: &str = "a3s.cli.reviewed-enablement-apply-result.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StoredEnablementPlan {
    schema: String,
    pub(super) result: PluginEnablementPlanResult,
    pub(super) runtime: ReviewedEnablementRuntimeEvidence,
}

impl StoredEnablementPlan {
    pub(super) fn new(
        result: PluginEnablementPlanResult,
        runtime: ReviewedEnablementRuntimeEvidence,
    ) -> PluginManagerResult<Self> {
        let record = Self {
            schema: PLAN_SCHEMA.to_string(),
            result,
            runtime,
        };
        record.validate()?;
        Ok(record)
    }

    pub(super) fn validate(&self) -> PluginManagerResult<()> {
        self.result.validate()?;
        let plan = self.result.plan.as_ref().ok_or_else(plan_invalid)?;
        self.runtime
            .validate_for(&plan.plan)
            .map_err(|_| plan_invalid())?;
        if self.schema != PLAN_SCHEMA
            || self.result.status != PluginEnablementPlanStatus::Planned
            || self.result.operation_id.is_none()
            || self.result.canonical_plan_digest.is_none()
            || self.result.plan.is_none()
        {
            return Err(plan_invalid());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StoredEnablementApplyIntent {
    schema: String,
    pub(super) request: PluginEnablementApplyRequest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) confirmation: Option<PluginOperationConfirmation>,
}

impl StoredEnablementApplyIntent {
    pub(super) fn new(
        request: PluginEnablementApplyRequest,
        confirmation: Option<PluginOperationConfirmation>,
    ) -> PluginManagerResult<Self> {
        let record = Self {
            schema: APPLY_INTENT_SCHEMA.to_string(),
            request,
            confirmation,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> PluginManagerResult<()> {
        let normalized = self.request.normalize()?;
        if self.schema != APPLY_INTENT_SCHEMA || self.request != normalized {
            return Err(store_invalid());
        }
        if let Some(confirmation) = &self.confirmation {
            confirmation.validate().map_err(|_| store_invalid())?;
            if confirmation.operation_id != self.request.operation_id
                || confirmation.plan_digest != self.request.plan_digest
            {
                return Err(store_invalid());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StoredEnablementApplyResult {
    schema: String,
    request: PluginEnablementApplyRequest,
    pub(super) cognitive_result: CognitivePackageEnablementResult,
}

impl StoredEnablementApplyResult {
    pub(super) fn new(
        request: PluginEnablementApplyRequest,
        mut cognitive_result: CognitivePackageEnablementResult,
        plan: &PluginEnablementPlanResult,
    ) -> PluginManagerResult<Self> {
        cognitive_result.replayed = false;
        let record = Self {
            schema: APPLY_RESULT_SCHEMA.to_string(),
            request,
            cognitive_result,
        };
        record.validate_for(plan)?;
        Ok(record)
    }

    fn validate_for(&self, plan: &PluginEnablementPlanResult) -> PluginManagerResult<()> {
        let request = self.request.normalize()?;
        let cognitive_request = cognitive_request(plan)?;
        self.cognitive_result
            .validate_for(&cognitive_request)
            .map_err(|_| store_invalid())?;
        if self.schema != APPLY_RESULT_SCHEMA
            || self.request != request
            || self.cognitive_result.replayed
            || self.request.operation_id != self.cognitive_result.operation_id
            || self.request.plan_digest
                != plan
                    .canonical_plan_digest
                    .as_deref()
                    .ok_or_else(store_invalid)?
        {
            return Err(store_invalid());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) struct ReviewedEnablementStore {
    root: PathBuf,
}

impl ReviewedEnablementStore {
    pub(super) fn from_state_root(state_root: &Path) -> Self {
        Self {
            root: state_root.join("plugin-manager/reviewed-enablement"),
        }
    }

    pub(super) async fn plan(
        &self,
        operation_id: &str,
    ) -> PluginManagerResult<Option<Box<StoredEnablementPlan>>> {
        let record: Option<StoredEnablementPlan> =
            self.load(self.plans_root(), operation_id).await?;
        if let Some(record) = &record {
            record.validate()?;
        }
        Ok(record.map(Box::new))
    }

    pub(super) async fn persist_plan(
        &self,
        record: StoredEnablementPlan,
    ) -> PluginManagerResult<bool> {
        record.validate()?;
        let key = record
            .result
            .operation_id
            .clone()
            .ok_or_else(plan_invalid)?;
        self.persist(self.plans_root(), &key, record).await
    }

    pub(super) async fn intent(
        &self,
        operation_id: &str,
        plan: &PluginEnablementPlanResult,
    ) -> PluginManagerResult<Option<StoredEnablementApplyIntent>> {
        let record: Option<StoredEnablementApplyIntent> =
            self.load(self.intents_root(), operation_id).await?;
        if let Some(record) = &record {
            record.validate()?;
            if record.request.operation_id
                != plan.operation_id.as_deref().ok_or_else(plan_invalid)?
                || record.request.plan_digest
                    != plan
                        .canonical_plan_digest
                        .as_deref()
                        .ok_or_else(plan_invalid)?
            {
                return Err(store_invalid());
            }
        }
        Ok(record)
    }

    pub(super) async fn persist_intent(
        &self,
        record: StoredEnablementApplyIntent,
        plan: &PluginEnablementPlanResult,
    ) -> PluginManagerResult<bool> {
        record.validate()?;
        if record.request.operation_id != plan.operation_id.as_deref().ok_or_else(plan_invalid)?
            || record.request.plan_digest
                != plan
                    .canonical_plan_digest
                    .as_deref()
                    .ok_or_else(plan_invalid)?
        {
            return Err(store_invalid());
        }
        let key = record.request.operation_id.clone();
        self.persist(self.intents_root(), &key, record).await
    }

    pub(super) async fn result(
        &self,
        operation_id: &str,
        plan: &PluginEnablementPlanResult,
    ) -> PluginManagerResult<Option<StoredEnablementApplyResult>> {
        let record: Option<StoredEnablementApplyResult> =
            self.load(self.results_root(), operation_id).await?;
        if let Some(record) = &record {
            record.validate_for(plan)?;
        }
        Ok(record)
    }

    pub(super) async fn persist_result(
        &self,
        record: StoredEnablementApplyResult,
        plan: &PluginEnablementPlanResult,
    ) -> PluginManagerResult<bool> {
        record.validate_for(plan)?;
        let key = record.request.operation_id.clone();
        self.persist(self.results_root(), &key, record).await
    }

    async fn load<T>(&self, root: PathBuf, key: &str) -> PluginManagerResult<Option<T>>
    where
        T: DeserializeOwned + Send + 'static,
    {
        let path = record_path(&root, key);
        run_store(move || {
            ensure_real_directory(&root)?;
            read_optional_record(&path)
        })
        .await
    }

    async fn persist<T>(&self, root: PathBuf, key: &str, candidate: T) -> PluginManagerResult<bool>
    where
        T: Clone + PartialEq + Serialize + DeserializeOwned + Send + 'static,
    {
        let path = record_path(&root, key);
        let load_root = root.clone();
        let written = candidate.clone();
        let disposition = run_store(move || {
            ensure_real_directory(&root)?;
            write_new_record(&path, &written)
        })
        .await?;
        if disposition == WriteDisposition::Created {
            return Ok(true);
        }
        let current = self.load(load_root, key).await?;
        if current.as_ref() == Some(&candidate) {
            Ok(false)
        } else {
            Err(operation_conflict())
        }
    }

    fn plans_root(&self) -> PathBuf {
        self.root.join("plans")
    }

    fn intents_root(&self) -> PathBuf {
        self.root.join("apply-intents")
    }

    fn results_root(&self) -> PathBuf {
        self.root.join("apply-results")
    }
}

fn record_path(root: &Path, key: &str) -> PathBuf {
    root.join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
}

async fn run_store<T: Send + 'static>(
    operation: impl FnOnce() -> PluginManagerResult<T> + Send + 'static,
) -> PluginManagerResult<T> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "reviewed enablement store task failed: {error}"
            ))
        })?
}
