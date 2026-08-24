//! Exact lifecycle, Runtime evidence, and Gateway target binding.

use a3s_gateway::config::ManagedTargetConfig;
use a3s_gateway::managed_service::ManagedServiceHealthCheck;
use a3s_runtime::contract::{
    HealthProbe, RuntimeObservation, RuntimeServiceEndpoint, TransportProtocol,
};
use a3s_use::plugin_lifecycle::PluginLifecycleIntent;
use a3s_use::plugin_runtime::{
    RuntimeBindingReceipt, RuntimeServiceBindingReceipt, RuntimeSurfacePlan,
};
use a3s_use_core::{PlanQualifiedSurfaceRef, PlanScope, PluginSurfaceKind, UseError, UseResult};
use uuid::Uuid;

use super::gateway_error;

const TARGET_ID_SCHEMA: &str = "a3s.code.gateway-runtime-target.v1";
const TARGET_ID_NAMESPACE: Uuid = Uuid::from_u128(0xf6a591c9_0af3_50d7_9c4e_10589abc676d);

pub(super) fn validate_binding_context(
    intent: &PluginLifecycleIntent,
    expected_kind: PluginSurfaceKind,
    surface_id: &str,
    plan: &RuntimeSurfacePlan,
    observation: &RuntimeObservation,
    runtime_endpoint: &RuntimeServiceEndpoint,
) -> UseResult<()> {
    validate_intent(intent)?;
    let planned = plan.surface();
    if planned.package_id != intent.package_id
        || planned.surface.kind != expected_kind
        || planned.surface.id != surface_id
        || !intent
            .surfaces
            .iter()
            .any(|candidate| candidate.surface == planned.surface)
        || plan.context().scope() != &intent.scope
        || plan.context().generation() != intent.generation
        || observation.unit_id != plan.spec().unit_id
        || observation.generation == 0
        || runtime_endpoint.protocol != TransportProtocol::Tcp
    {
        return Err(binding_error(
            "The private Gateway binding does not match the reviewed lifecycle and Runtime generation.",
        ));
    }
    observation.validate_against(plan.spec()).map_err(|_| {
        binding_error("The Runtime Service observation no longer validates against its plan.")
    })?;
    runtime_endpoint.validate().map_err(|_| {
        binding_error("The Runtime provider returned an invalid private Service endpoint.")
    })?;
    let observed_endpoint =
        RuntimeServiceEndpoint::from_observation(observation, &runtime_endpoint.port_name)
            .map_err(|_| {
                binding_error(
                    "The Runtime Service endpoint is not bound to its exact observation evidence.",
                )
            })?;
    if &observed_endpoint != runtime_endpoint {
        return Err(binding_error(
            "The Runtime Service endpoint is not bound to its exact observation evidence.",
        ));
    }
    Ok(())
}

pub(super) fn validate_retirement_context(
    intent: &PluginLifecycleIntent,
    receipt: &RuntimeServiceBindingReceipt,
) -> UseResult<()> {
    validate_intent(intent)?;
    RuntimeBindingReceipt::Service(receipt.clone())
        .validate()
        .map_err(|_| binding_error("The Runtime Service receipt is not canonical."))?;
    if receipt.surface.package_id != intent.package_id
        || receipt.scope != intent.scope
        || receipt.package_digest != intent.package_digest
        || receipt.generation != intent.generation
        || !intent
            .surfaces
            .iter()
            .any(|candidate| candidate.surface == receipt.surface.surface)
    {
        return Err(binding_error(
            "The Runtime Service receipt is not owned by the reviewed lifecycle generation.",
        ));
    }
    Ok(())
}

fn validate_intent(intent: &PluginLifecycleIntent) -> UseResult<()> {
    intent
        .validate()
        .map_err(|_| binding_error("The Runtime Service lifecycle intent is not canonical."))
}

pub(super) fn managed_health(
    plan: &RuntimeSurfacePlan,
    runtime_endpoint: &RuntimeServiceEndpoint,
) -> UseResult<ManagedServiceHealthCheck> {
    let health = plan.spec().health.as_ref().ok_or_else(|| {
        binding_error("A persistent Runtime Service requires HTTP health evidence.")
    })?;
    let HealthProbe::Http { port, path, .. } = &health.probe else {
        return Err(binding_error(
            "A Gateway-backed Runtime Service requires an HTTP health probe.",
        ));
    };
    if port != &runtime_endpoint.port_name {
        return Err(binding_error(
            "The Runtime health probe does not reference the published Service endpoint.",
        ));
    }
    ManagedServiceHealthCheck::new(
        path,
        health.interval_ms,
        health.timeout_ms,
        health.success_threshold,
        health.failure_threshold,
    )
    .map_err(|error| {
        gateway_error(
            "use.plugin.gateway_bind_failed",
            "validate health for",
            error,
        )
    })
}

pub(super) fn managed_target(
    surface: &PlanQualifiedSurfaceRef,
    scope: &PlanScope,
    unit_id: &str,
    generation: u64,
) -> UseResult<ManagedTargetConfig> {
    if unit_id.is_empty() || generation == 0 {
        return Err(binding_error(
            "The Runtime generation cannot produce a stable Gateway target identity.",
        ));
    }
    let kind = match surface.surface.kind {
        PluginSurfaceKind::Tool => "tool",
        PluginSurfaceKind::Mcp => "mcp",
        _ => {
            return Err(binding_error(
                "Only Tool and MCP Runtime Services can own Gateway targets.",
            ))
        }
    };
    let mut name = Vec::new();
    for value in [
        TARGET_ID_SCHEMA,
        scope.kind.as_str(),
        scope.id.as_str(),
        surface.package_id.as_str(),
        kind,
        surface.surface.id.as_str(),
        unit_id,
    ] {
        let length = u64::try_from(value.len()).map_err(|_| {
            binding_error("The Gateway target identity exceeds its portable bound.")
        })?;
        name.extend_from_slice(&length.to_be_bytes());
        name.extend_from_slice(value.as_bytes());
    }
    name.extend_from_slice(&generation.to_be_bytes());
    Ok(ManagedTargetConfig {
        target_id: Uuid::new_v5(&TARGET_ID_NAMESPACE, &name),
        unit_id: unit_id.to_string(),
        generation,
    })
}

pub(super) fn binding_error(message: impl Into<String>) -> UseError {
    UseError::new("use.plugin.gateway_binding_invalid", message)
}
