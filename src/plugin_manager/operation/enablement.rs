use a3s_use_core::UseError;
use serde_json::{Map, Value};

use crate::components::code_cognitive_package_manager;

use super::super::capability::observe;
use super::super::process::{normalize_component_id, PluginPackageToggleRequest};
use super::super::{PluginManager, PluginManagerError, PluginManagerResult};

const COGNITIVE_PACKAGE_RECEIPT_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnablementRoute {
    UseOwned,
    Legacy,
}

pub(in crate::plugin_manager) async fn set_enabled(
    manager: &PluginManager,
    request: &PluginPackageToggleRequest,
) -> PluginManagerResult<Value> {
    let _mutation_guard = manager.operation_store.acquire_mutation_lock().await?;
    validate_replay_identity(request)?;
    let component_id = normalize_component_id(&request.component_id)?;
    let package_id = component_id
        .strip_prefix("use/")
        .ok_or_else(|| PluginManagerError::InvalidRequest("invalid Use package ID".to_string()))?;
    let capability_before = observe(manager).await;
    let package_manager = code_cognitive_package_manager(
        &manager.component_paths,
        super::super::default_plan_scope(),
    )
    .map_err(use_infrastructure_error)?;
    let installed = package_manager
        .registry()
        .get(package_id)
        .await
        .map_err(use_infrastructure_error)?;

    let extension = installed.ok_or_else(|| not_installed(package_id))?;
    let (mut result, durable) = match enablement_route(extension.receipt.schema_version)? {
        EnablementRoute::UseOwned => {
            return Err(PluginManagerError::InvalidRequest(
                "schema-v3 cognitive packages require reviewed enablement plan/apply; refusing the compatibility toggle"
                    .to_string(),
            ));
        }
        EnablementRoute::Legacy => {
            if request.operation_id.is_some() || request.expected_package_generation.is_some() {
                return Err(PluginManagerError::InvalidRequest(
                    "operationId and expectedPackageGeneration are supported only for schema-v3 cognitive packages"
                        .to_string(),
                ));
            }
            (manager.process.set_enabled(request).await?, false)
        }
    };

    let capability_after = observe(manager).await;
    let object = result_object(&mut result)?;
    object.insert("componentId".to_string(), Value::String(component_id));
    object.insert(
        "capabilityBefore".to_string(),
        serde_json::to_value(capability_before).map_err(json_error)?,
    );
    object.insert(
        "capabilityAfter".to_string(),
        serde_json::to_value(capability_after).map_err(json_error)?,
    );
    object.insert("durableEnablement".to_string(), Value::Bool(durable));
    if !durable {
        object.insert("replayed".to_string(), Value::Bool(false));
    }
    Ok(result)
}

fn enablement_route(schema_version: u32) -> PluginManagerResult<EnablementRoute> {
    match schema_version {
        COGNITIVE_PACKAGE_RECEIPT_SCHEMA_VERSION => Ok(EnablementRoute::UseOwned),
        1 | 2 => Ok(EnablementRoute::Legacy),
        _ => Err(PluginManagerError::InvalidRequest(format!(
            "unsupported cognitive-package receipt schema version {schema_version}; refusing legacy enablement fallback"
        ))),
    }
}

fn validate_replay_identity(request: &PluginPackageToggleRequest) -> PluginManagerResult<()> {
    if request.operation_id.is_some() != request.expected_package_generation.is_some() {
        return Err(PluginManagerError::InvalidRequest(
            "operationId and expectedPackageGeneration must be supplied together".to_string(),
        ));
    }
    if request.expected_package_generation == Some(0) {
        return Err(PluginManagerError::InvalidRequest(
            "expectedPackageGeneration must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

fn result_object(value: &mut Value) -> PluginManagerResult<&mut Map<String, Value>> {
    value.as_object_mut().ok_or_else(|| {
        PluginManagerError::Upstream(
            "A3S Use enablement response must be a JSON object".to_string(),
        )
    })
}

fn not_installed(package_id: &str) -> PluginManagerError {
    PluginManagerError::OperationFailed(format!(
        "cognitive package '{package_id}' is not installed"
    ))
}

fn use_infrastructure_error(error: UseError) -> PluginManagerError {
    PluginManagerError::Infrastructure(format!("{}: {}", error.code, error.message))
}

fn json_error(error: serde_json::Error) -> PluginManagerError {
    PluginManagerError::Infrastructure(format!(
        "failed to encode cognitive-package enablement evidence: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        operation_id: Option<&str>,
        expected_package_generation: Option<u64>,
    ) -> PluginPackageToggleRequest {
        PluginPackageToggleRequest {
            component_id: "use/acme/guide".to_string(),
            enabled: false,
            operation_id: operation_id.map(str::to_string),
            expected_package_generation,
        }
    }

    #[test]
    fn exact_replay_identity_requires_operation_and_generation_together() {
        let missing_generation = request(Some("plugin-enablement-guide-disable"), None);
        let missing_operation = request(None, Some(7));

        assert!(validate_replay_identity(&missing_generation)
            .unwrap_err()
            .to_string()
            .contains("must be supplied together"));
        assert!(validate_replay_identity(&missing_operation)
            .unwrap_err()
            .to_string()
            .contains("must be supplied together"));
    }

    #[test]
    fn exact_replay_identity_rejects_zero_generation() {
        let request = request(Some("plugin-enablement-guide-disable"), Some(0));

        assert!(validate_replay_identity(&request)
            .unwrap_err()
            .to_string()
            .contains("must be greater than zero"));
    }

    #[test]
    fn exact_replay_identity_accepts_absent_or_complete_identity() {
        assert!(validate_replay_identity(&request(None, None)).is_ok());
        assert!(validate_replay_identity(&request(
            Some("plugin-enablement-guide-disable"),
            Some(7),
        ))
        .is_ok());
    }

    #[test]
    fn only_known_legacy_receipts_can_use_the_subprocess_route() {
        assert_eq!(enablement_route(1).unwrap(), EnablementRoute::Legacy);
        assert_eq!(enablement_route(2).unwrap(), EnablementRoute::Legacy);
        assert_eq!(enablement_route(3).unwrap(), EnablementRoute::UseOwned);
        assert!(enablement_route(0)
            .unwrap_err()
            .to_string()
            .contains("refusing legacy enablement fallback"));
        assert!(enablement_route(4)
            .unwrap_err()
            .to_string()
            .contains("refusing legacy enablement fallback"));
    }
}
