use std::path::{Path, PathBuf};

use a3s_use::cognitive_package::CognitivePackageEnablementRequest;
use a3s_use_core::{
    PluginDesiredState, PluginHostCapabilities, PluginHostEnablementRequest,
    PluginHostEnablementResult, PluginHostPackageState, UseError, UseResult,
    PLUGIN_HOST_ENABLEMENT_RESULT_SCHEMA,
};
use olpc_cjson::CanonicalFormatter;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::components::managed_cognitive_package_manager;
use crate::plugin_manager::operation::store::io::{
    ensure_real_directory, read_optional_record, write_new_record, WriteDisposition,
};
use crate::plugin_manager::{PluginManager, PluginManagerError, PluginManagerResult};

const MANAGED_ENABLEMENT_INTENT_SCHEMA: &str = "a3s.cli.managed-plugin-enablement-intent.v1";
const MANAGED_ENABLEMENT_RESULT_SCHEMA: &str = "a3s.cli.managed-plugin-enablement-result.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredManagedHostEnablementIntent {
    schema: String,
    request: PluginHostEnablementRequest,
}

impl StoredManagedHostEnablementIntent {
    fn new(request: PluginHostEnablementRequest) -> UseResult<Self> {
        let intent = Self {
            schema: MANAGED_ENABLEMENT_INTENT_SCHEMA.to_string(),
            request,
        };
        intent.validate()?;
        Ok(intent)
    }

    fn validate(&self) -> UseResult<()> {
        self.request.validate()?;
        if self.schema != MANAGED_ENABLEMENT_INTENT_SCHEMA {
            return Err(store_invalid());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredManagedHostEnablementResult {
    schema: String,
    request: PluginHostEnablementRequest,
    result: PluginHostEnablementResult,
}

impl StoredManagedHostEnablementResult {
    fn new(
        request: PluginHostEnablementRequest,
        result: PluginHostEnablementResult,
    ) -> UseResult<Self> {
        let record = Self {
            schema: MANAGED_ENABLEMENT_RESULT_SCHEMA.to_string(),
            request,
            result,
        };
        record.validate()?;
        Ok(record)
    }

    fn validate(&self) -> UseResult<()> {
        self.request.validate()?;
        self.result.validate()?;
        let expected_desired = if self.request.enabled {
            PluginDesiredState::Enabled
        } else {
            PluginDesiredState::InstalledDisabled
        };
        let generation = self
            .result
            .state
            .package_generation
            .ok_or_else(store_invalid)?;
        let generation_matches = if self.result.changed {
            generation > self.request.expected_package_generation
        } else {
            generation == self.request.expected_package_generation
        };
        if self.schema != MANAGED_ENABLEMENT_RESULT_SCHEMA
            || self.result.replayed
            || self.result.request_id != self.request.request_id
            || self.result.operation_id != self.request.operation_id
            || self.result.assignment_generation != self.request.assignment_generation
            || self.result.capabilities_digest != self.request.capabilities_digest
            || self.result.scope != self.request.scope
            || self.result.package_id != self.request.package_id
            || self.result.state.desired != expected_desired
            || !generation_matches
            || self.result.operation_result_digest != result_digest(&self.request, &self.result)?
        {
            return Err(store_invalid());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) struct ManagedHostEnablementStore {
    root: PathBuf,
}

impl ManagedHostEnablementStore {
    pub(super) fn from_state_root(state_root: &Path) -> Self {
        Self {
            root: state_root.join("plugin-manager/managed-host/enablement"),
        }
    }

    async fn begin(
        &self,
        request: &PluginHostEnablementRequest,
    ) -> UseResult<Option<PluginHostEnablementResult>> {
        self.persist_intent(StoredManagedHostEnablementIntent::new(request.clone())?)
            .await?;
        if let Some(record) = self.load_result(&request.operation_id).await? {
            if record.request != *request {
                return Err(operation_conflict());
            }
            let mut result = record.result;
            result.replayed = true;
            return Ok(Some(result));
        }
        Ok(None)
    }

    async fn persist_intent(&self, intent: StoredManagedHostEnablementIntent) -> UseResult<()> {
        intent.validate()?;
        if let Some(current) = self.load_intent(&intent.request.operation_id).await? {
            if current == intent {
                return Ok(());
            }
            return Err(operation_conflict());
        }

        let root = self.intents_root();
        let path = self.intent_path(&intent.request.operation_id);
        let candidate = intent.clone();
        let disposition = run_store(move || {
            ensure_real_directory(&root)?;
            write_new_record(&path, &candidate)
        })
        .await
        .map_err(store_unavailable)?;
        if disposition == WriteDisposition::Created {
            return Ok(());
        }

        let current = self
            .load_intent(&intent.request.operation_id)
            .await?
            .ok_or_else(store_unavailable_missing)?;
        if current == intent {
            Ok(())
        } else {
            Err(operation_conflict())
        }
    }

    async fn persist_result(
        &self,
        record: StoredManagedHostEnablementResult,
    ) -> UseResult<(PluginHostEnablementResult, bool)> {
        record.validate()?;
        let intent = self
            .load_intent(&record.request.operation_id)
            .await?
            .ok_or_else(store_unavailable_missing)?;
        if intent.request != record.request {
            return Err(operation_conflict());
        }
        if let Some(current) = self.load_result(&record.request.operation_id).await? {
            if current == record {
                return Ok((current.result, false));
            }
            return Err(operation_conflict());
        }

        let root = self.results_root();
        let path = self.result_path(&record.request.operation_id);
        let candidate = record.clone();
        let disposition = run_store(move || {
            ensure_real_directory(&root)?;
            write_new_record(&path, &candidate)
        })
        .await
        .map_err(store_unavailable)?;
        if disposition == WriteDisposition::Created {
            return Ok((record.result, true));
        }

        let current = self
            .load_result(&record.request.operation_id)
            .await?
            .ok_or_else(store_unavailable_missing)?;
        if current == record {
            Ok((current.result, false))
        } else {
            Err(operation_conflict())
        }
    }

    async fn load_intent(
        &self,
        operation_id: &str,
    ) -> UseResult<Option<StoredManagedHostEnablementIntent>> {
        let root = self.intents_root();
        let path = self.intent_path(operation_id);
        let intent = run_store(move || {
            ensure_real_directory(&root)?;
            read_optional_record::<StoredManagedHostEnablementIntent>(&path)
        })
        .await
        .map_err(store_unavailable)?;
        if let Some(intent) = &intent {
            intent.validate()?;
        }
        Ok(intent)
    }

    async fn load_result(
        &self,
        operation_id: &str,
    ) -> UseResult<Option<StoredManagedHostEnablementResult>> {
        let root = self.results_root();
        let path = self.result_path(operation_id);
        let record = run_store(move || {
            ensure_real_directory(&root)?;
            read_optional_record::<StoredManagedHostEnablementResult>(&path)
        })
        .await
        .map_err(store_unavailable)?;
        if let Some(record) = &record {
            record.validate()?;
        }
        Ok(record)
    }

    fn intents_root(&self) -> PathBuf {
        self.root.join("intents")
    }

    fn results_root(&self) -> PathBuf {
        self.root.join("results")
    }

    fn intent_path(&self, operation_id: &str) -> PathBuf {
        self.intents_root()
            .join(Self::record_file_name(operation_id))
    }

    fn result_path(&self, operation_id: &str) -> PathBuf {
        self.results_root()
            .join(Self::record_file_name(operation_id))
    }

    fn record_file_name(operation_id: &str) -> String {
        format!("{:x}.json", Sha256::digest(operation_id.as_bytes()))
    }
}

pub(super) async fn set_enablement(
    manager: &PluginManager,
    store: &ManagedHostEnablementStore,
    capabilities: &PluginHostCapabilities,
    request: &PluginHostEnablementRequest,
) -> UseResult<PluginHostEnablementResult> {
    if let Some(result) = store.begin(request).await? {
        result.validate_for(request, capabilities)?;
        return Ok(result);
    }

    let package_manager =
        managed_cognitive_package_manager(&manager.component_paths, request.scope.plan_scope())?;
    let cognitive_request = CognitivePackageEnablementRequest::new(
        request.operation_id.clone(),
        request.package_id.to_string(),
        request.expected_package_generation,
        request.enabled,
    )?;
    let cognitive_result = package_manager.set_enablement(&cognitive_request).await?;
    let resumed = cognitive_result.replayed;
    let mut result = PluginHostEnablementResult {
        schema: PLUGIN_HOST_ENABLEMENT_RESULT_SCHEMA.to_string(),
        request_id: request.request_id.clone(),
        operation_id: request.operation_id.clone(),
        assignment_generation: request.assignment_generation,
        capabilities_digest: request.capabilities_digest.clone(),
        scope: request.scope.clone(),
        package_id: request.package_id.clone(),
        completed_at_ms: cognitive_result.completed_at_ms,
        operation_result_digest: String::new(),
        changed: cognitive_result.changed,
        state: cognitive_result.state,
        replayed: false,
    };
    result.operation_result_digest = result_digest(request, &result)?;
    result.validate_for(request, capabilities)?;
    let record = StoredManagedHostEnablementResult::new(request.clone(), result)?;
    let (mut durable, created) = store.persist_result(record).await?;
    durable.replayed = resumed || !created;
    durable.validate_for(request, capabilities)?;
    Ok(durable)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedHostEnablementOutcome<'a> {
    schema: &'a str,
    request: &'a PluginHostEnablementRequest,
    completed_at_ms: u64,
    changed: bool,
    state: &'a PluginHostPackageState,
}

fn result_digest(
    request: &PluginHostEnablementRequest,
    result: &PluginHostEnablementResult,
) -> UseResult<String> {
    let outcome = ManagedHostEnablementOutcome {
        schema: &result.schema,
        request,
        completed_at_ms: result.completed_at_ms,
        changed: result.changed,
        state: &result.state,
    };
    let mut bytes = Vec::new();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut bytes, CanonicalFormatter::new());
    outcome.serialize(&mut serializer).map_err(|_| {
        UseError::new(
            "use.plugin.host_enablement_result_invalid",
            "The managed enablement result could not be canonicalized.",
        )
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

async fn run_store<T: Send + 'static>(
    operation: impl FnOnce() -> PluginManagerResult<T> + Send + 'static,
) -> PluginManagerResult<T> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| {
            PluginManagerError::Infrastructure(format!(
                "managed enablement store task failed: {error}"
            ))
        })?
}

fn operation_conflict() -> UseError {
    UseError::new(
        "use.plugin.host_enablement_operation_conflict",
        "The managed enablement operation ID is already bound to a different complete request or result.",
    )
}

fn store_invalid() -> UseError {
    UseError::new(
        "use.plugin.host_enablement_store_invalid",
        "The durable managed enablement record is invalid.",
    )
}

fn store_unavailable(_error: PluginManagerError) -> UseError {
    UseError::new(
        "use.plugin.host_enablement_store_unavailable",
        "The durable managed enablement store is unavailable.",
    )
}

fn store_unavailable_missing() -> UseError {
    UseError::new(
        "use.plugin.host_enablement_store_unavailable",
        "The durable managed enablement result appeared without a readable record.",
    )
}
