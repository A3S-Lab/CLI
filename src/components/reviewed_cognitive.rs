//! Trusted umbrella-host adapter for reviewed cognitive-package operations.

use std::sync::Arc;

use a3s_updater::InstallProvenance;
use a3s_use::cognitive_package::{
    CognitivePackageManager, ReviewedCognitivePackageAuthorizationProvider,
};
use a3s_use_core::{
    PluginOperationAction, PluginOperationConfirmation, PluginOperationPlanEnvelope,
    PluginPackageLock,
};
use a3s_use_extension::{ExtensionPaths, ExtensionRegistry};
use anyhow::{bail, Context};
use serde::Serialize;

use super::cognitive_lifecycle::CodeCognitivePackageLifecycleFactory;
use super::id::ComponentId;
use super::lifecycle::OperationRecord;
use super::lock::ComponentOperationLock;
use super::paths::ComponentPaths;
use crate::registry::RegistryStore;

/// Apply exactly the cognitive-package operation reviewed by the umbrella
/// CLI/Web host without creating a second operation identity or authority.
pub(crate) async fn apply_reviewed_cognitive_package(
    envelope: &PluginOperationPlanEnvelope,
    confirmation: Option<&PluginOperationConfirmation>,
    paths: &ComponentPaths,
    registries: &RegistryStore,
) -> anyhow::Result<OperationRecord> {
    envelope.validate().map_err(anyhow::Error::new)?;
    let component = reviewed_component(envelope)?;
    let authorization =
        ReviewedCognitivePackageAuthorizationProvider::new(envelope.clone(), confirmation.cloned())
            .map_err(anyhow::Error::new)?;
    let manager = CognitivePackageManager::with_plan_scope_lifecycle_and_authorization(
        ExtensionRegistry::new(ExtensionPaths::new(
            paths.data_root.join("use"),
            paths.state_root.join("use"),
        )),
        envelope.plan.scope.clone(),
        Arc::new(CodeCognitivePackageLifecycleFactory::default()),
        Arc::new(authorization),
    )
    .map_err(anyhow::Error::new)?;
    let _lock =
        ComponentOperationLock::acquire(paths.operation_lock_path(&component), &component).await?;

    match envelope.plan.action {
        PluginOperationAction::Install => {
            apply_install(envelope, &component, &manager, paths, registries).await
        }
        PluginOperationAction::Upgrade => {
            apply_upgrade(envelope, &component, &manager, paths, registries).await
        }
        PluginOperationAction::Uninstall => apply_uninstall(envelope, &component, &manager).await,
    }
}

async fn apply_install(
    envelope: &PluginOperationPlanEnvelope,
    component: &ComponentId,
    manager: &CognitivePackageManager,
    paths: &ComponentPaths,
    registries: &RegistryStore,
) -> anyhow::Result<OperationRecord> {
    let lock = reviewed_candidate_lock(envelope)?;
    let root = reviewed_root(lock, envelope)?;
    let trusted = registries.trusted_registries_for_lock(&paths.state_root, lock)?;
    let expected_lock_digest = lock.descriptor_digest().map_err(anyhow::Error::new)?;
    let result = manager
        .install_remote(
            &trusted.root,
            &trusted.dependencies,
            &envelope.plan.package_id,
            Some(&root.catalog.record.version),
            root.catalog.record.channel,
            Some(&expected_lock_digest),
        )
        .await
        .map_err(anyhow::Error::new)?;
    let package_graph = reviewed_graph(&result, envelope)?;

    Ok(OperationRecord {
        component: component.clone(),
        action: "install",
        changed: result.changed,
        recovered: false,
        version: Some(result.root.receipt.version.clone()),
        provenance: Some(InstallProvenance::Delegated),
        path: Some(result.root.receipt.package_root.clone()),
        package_graph: Some(package_graph),
        message: format!(
            "A3S Use installed cognitive package '{}' and its reviewed dependency closure.",
            component
        ),
    })
}

async fn apply_upgrade(
    envelope: &PluginOperationPlanEnvelope,
    component: &ComponentId,
    manager: &CognitivePackageManager,
    paths: &ComponentPaths,
    registries: &RegistryStore,
) -> anyhow::Result<OperationRecord> {
    let lock = reviewed_candidate_lock(envelope)?;
    let root = reviewed_root(lock, envelope)?;
    let trusted = registries.trusted_registries_for_lock(&paths.state_root, lock)?;
    let expected_lock_digest = lock.descriptor_digest().map_err(anyhow::Error::new)?;
    let result = manager
        .upgrade_remote(
            &trusted.root,
            &trusted.dependencies,
            &envelope.plan.package_id,
            Some(&root.catalog.record.version),
            root.catalog.record.channel,
            Some(&expected_lock_digest),
        )
        .await
        .map_err(anyhow::Error::new)?;
    let package_graph = reviewed_graph(&result, envelope)?;

    Ok(OperationRecord {
        component: component.clone(),
        action: "upgrade",
        changed: result.changed,
        recovered: false,
        version: Some(result.root.receipt.version.clone()),
        provenance: Some(InstallProvenance::Delegated),
        path: Some(result.root.receipt.package_root.clone()),
        package_graph: Some(package_graph),
        message: format!(
            "A3S Use upgraded cognitive package '{}' and its reviewed dependency closure.",
            component
        ),
    })
}

async fn apply_uninstall(
    envelope: &PluginOperationPlanEnvelope,
    component: &ComponentId,
    manager: &CognitivePackageManager,
) -> anyhow::Result<OperationRecord> {
    let lock = reviewed_candidate_lock(envelope)?;
    let root = reviewed_root(lock, envelope)?;
    let version = root.catalog.record.version.clone();
    let result = manager
        .uninstall(&envelope.plan.package_id)
        .await
        .map_err(anyhow::Error::new)?;
    let package_graph = reviewed_graph(&result, envelope)?;

    Ok(OperationRecord {
        component: component.clone(),
        action: "uninstall",
        changed: result.changed,
        recovered: false,
        version: Some(version),
        provenance: Some(InstallProvenance::Delegated),
        path: None,
        package_graph: Some(package_graph),
        message: format!(
            "A3S Use uninstalled cognitive package '{}' and its unreferenced dependency closure.",
            component
        ),
    })
}

fn reviewed_component(envelope: &PluginOperationPlanEnvelope) -> anyhow::Result<ComponentId> {
    let component = ComponentId::parse(&envelope.plan.component_id)?;
    let expected = format!("use/{}", envelope.plan.package_id);
    if component.as_str() != expected {
        bail!(
            "reviewed cognitive-package component '{}' does not match package '{}'",
            component,
            envelope.plan.package_id
        );
    }
    Ok(component)
}

fn reviewed_candidate_lock(
    envelope: &PluginOperationPlanEnvelope,
) -> anyhow::Result<&PluginPackageLock> {
    envelope
        .package_lock
        .as_ref()
        .context("reviewed cognitive-package operation omitted its exact package lock")
}

fn reviewed_root<'a>(
    lock: &'a PluginPackageLock,
    envelope: &PluginOperationPlanEnvelope,
) -> anyhow::Result<&'a a3s_use_core::LockedPluginPackage> {
    if lock.root_package_id != envelope.plan.package_id {
        bail!(
            "reviewed cognitive-package lock root '{}' does not match package '{}'",
            lock.root_package_id,
            envelope.plan.package_id
        );
    }
    lock.package(&lock.root_package_id)
        .context("reviewed cognitive-package lock omitted its root package")
}

fn reviewed_graph(
    result: &impl Serialize,
    envelope: &PluginOperationPlanEnvelope,
) -> anyhow::Result<serde_json::Value> {
    let mut graph = serde_json::to_value(result)
        .context("failed to encode reviewed cognitive-package graph evidence")?;
    let object = graph
        .as_object_mut()
        .context("reviewed cognitive-package graph evidence is not an object")?;
    let expected = serde_json::to_value(envelope)
        .context("failed to encode the reviewed cognitive-package plan")?;
    match object.get("plan") {
        Some(actual) if !actual.is_null() && actual != &expected => {
            bail!("A3S Use returned a cognitive-package plan different from the reviewed host plan")
        }
        Some(_) => {
            object.insert("plan".to_string(), expected);
        }
        None => {
            object.insert("plan".to_string(), expected);
        }
    }
    Ok(graph)
}
