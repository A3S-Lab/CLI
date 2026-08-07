use a3s_use_core::{
    FilesystemScope, PlanActor, PlanEnforcementProfile, PlanPackageChangeKind, PlanPolicyDecision,
    PlanScopeKind, PlannedPackageTransition, PluginOperationPlan, PluginPlanSource,
    PluginSurfaceKind, SurfacePermissionCeiling,
};

use super::{
    PluginAuthorizationPolicy, PluginPolicyEvaluation, PluginPolicyViolation,
    PluginPolicyViolationCode, PluginPolicyWorkspaceCeiling,
};
use crate::plugin_manager::{PluginManagerError, PluginManagerResult};

impl PluginAuthorizationPolicy {
    /// Evaluate a structurally valid immutable plan without trusting its
    /// embedded policy decision.
    pub fn evaluate_plan(
        &self,
        plan: &PluginOperationPlan,
    ) -> PluginManagerResult<PluginPolicyEvaluation> {
        plan.validate().map_err(|error| {
            PluginManagerError::InvalidRequest(format!(
                "invalid plugin operation plan for policy evaluation: {error}"
            ))
        })?;
        let configured_decision = self.configured_decision(plan.authority.actor, plan.action);
        let mut violations = self.ceiling_violations(plan);
        violations.sort();
        violations.dedup();
        let hard_denied =
            configured_decision == PlanPolicyDecision::Deny || agent_hard_denial(plan, &violations);
        let decision = if hard_denied {
            PlanPolicyDecision::Deny
        } else if configured_decision == PlanPolicyDecision::Allow && violations.is_empty() {
            PlanPolicyDecision::Allow
        } else {
            PlanPolicyDecision::Ask
        };
        Ok(PluginPolicyEvaluation {
            actor: plan.authority.actor,
            configured_decision,
            decision,
            policy_digest: self.descriptor_digest()?,
            confirmation_required: decision == PlanPolicyDecision::Ask,
            violations,
        })
    }

    /// Re-evaluate policy at apply and require the stored authority to remain
    /// byte-for-byte equivalent to the current host decision.
    pub fn verify_plan_authority(
        &self,
        plan: &PluginOperationPlan,
    ) -> PluginManagerResult<PluginPolicyEvaluation> {
        let evaluation = self.evaluate_plan(plan)?;
        if plan.authority != evaluation.authority() {
            return Err(PluginManagerError::OperationFailed(
                "the plugin policy or authorization decision changed after review; create and review a new plan"
                    .to_string(),
            ));
        }
        Ok(evaluation)
    }

    fn ceiling_violations(&self, plan: &PluginOperationPlan) -> Vec<PluginPolicyViolation> {
        let mut violations = Vec::new();
        self.evaluate_operation_impact(plan, &mut violations);
        self.evaluate_workspace(plan, &mut violations);

        let activated = plan
            .packages
            .iter()
            .filter(|package| {
                matches!(
                    package.change,
                    PlanPackageChangeKind::Add | PlanPackageChangeKind::Replace
                ) || (plan.action == a3s_use_core::PluginOperationAction::Enable
                    && package.change == PlanPackageChangeKind::Retain)
            })
            .collect::<Vec<_>>();
        if activated.len() as u32 > self.max_packages {
            push(
                &mut violations,
                PluginPolicyViolationCode::PackageCountExceeded,
                "operation",
            );
        }
        let surface_count = activated
            .iter()
            .filter_map(|package| package.after.as_ref())
            .map(|state| state.release.surfaces.len())
            .sum::<usize>();
        if surface_count as u32 > self.max_surfaces {
            push(
                &mut violations,
                PluginPolicyViolationCode::SurfaceCountExceeded,
                "operation",
            );
        }
        for package in activated {
            self.evaluate_package(package, &mut violations);
        }
        for provider in &plan.providers {
            if provider.enforcement == PlanEnforcementProfile::NativeUnconfined {
                push(
                    &mut violations,
                    PluginPolicyViolationCode::NativeUnconfined,
                    qualified_surface(
                        &provider.surface.package_id,
                        provider.surface.surface.kind,
                        &provider.surface.surface.id,
                    ),
                );
            }
        }
        violations
    }

    fn evaluate_operation_impact(
        &self,
        plan: &PluginOperationPlan,
        output: &mut Vec<PluginPolicyViolation>,
    ) {
        if plan.impact.download_bytes > self.max_download_bytes {
            push(
                output,
                PluginPolicyViolationCode::DownloadSizeExceeded,
                "operation",
            );
        }
        if plan.impact.installed_bytes_after > self.max_installed_bytes {
            push(
                output,
                PluginPolicyViolationCode::InstalledSizeExceeded,
                "operation",
            );
        }
    }

    fn evaluate_workspace(
        &self,
        plan: &PluginOperationPlan,
        output: &mut Vec<PluginPolicyViolation>,
    ) {
        match plan.scope.kind {
            PlanScopeKind::User if !self.allow_user_scope => push(
                output,
                PluginPolicyViolationCode::UserScopeNotAllowed,
                &plan.scope.id,
            ),
            PlanScopeKind::Workspace if !self.workspace_ids.contains(&plan.scope.id) => push(
                output,
                PluginPolicyViolationCode::WorkspaceNotAllowed,
                &plan.scope.id,
            ),
            PlanScopeKind::User | PlanScopeKind::Workspace => {}
        }
        if plan.workspace_impacts.len() as u32 > self.max_workspaces {
            push(
                output,
                PluginPolicyViolationCode::WorkspaceCountExceeded,
                "operation",
            );
        }
        for impact in &plan.workspace_impacts {
            if plan.scope.kind == PlanScopeKind::User && impact.scope_id == plan.scope.id {
                continue;
            }
            if !self.workspace_ids.contains(&impact.scope_id) {
                push(
                    output,
                    PluginPolicyViolationCode::WorkspaceNotAllowed,
                    &impact.scope_id,
                );
            }
        }
    }

    fn evaluate_package(
        &self,
        package: &PlannedPackageTransition,
        output: &mut Vec<PluginPolicyViolation>,
    ) {
        let publisher = package.package_id.split('/').next().unwrap_or_default();
        if !self
            .trusted_publishers
            .iter()
            .any(|trusted| trusted == publisher)
        {
            push(
                output,
                PluginPolicyViolationCode::UntrustedPublisher,
                &package.package_id,
            );
        }
        match package.source.as_ref() {
            Some(PluginPlanSource::Registry { provenance, .. }) => {
                if !self.trusted_registries.contains(&provenance.registry_name) {
                    push(
                        output,
                        PluginPolicyViolationCode::UntrustedRegistry,
                        format!("{}@{}", package.package_id, provenance.registry_name),
                    );
                }
            }
            Some(_) => push(
                output,
                PluginPolicyViolationCode::UnsupportedSource,
                &package.package_id,
            ),
            None => {}
        }

        let Some(after) = &package.after else {
            return;
        };
        for surface in &after.release.surfaces {
            let subject = qualified_surface(&package.package_id, surface.kind, &surface.id);
            if !self.allowed_surfaces.contains(&surface.kind) {
                push(
                    output,
                    PluginPolicyViolationCode::SurfaceKindNotAllowed,
                    subject,
                );
            }
        }
        for permission in &after.permissions.surfaces {
            self.evaluate_permission(&package.package_id, permission, output);
        }
    }

    fn evaluate_permission(
        &self,
        package_id: &str,
        permission: &SurfacePermissionCeiling,
        output: &mut Vec<PluginPolicyViolation>,
    ) {
        let subject =
            qualified_surface(package_id, permission.surface.kind, &permission.surface.id);
        if permission.native_execution && !self.permissions.native_execution {
            push(
                output,
                PluginPolicyViolationCode::NativeExecutionNotAllowed,
                &subject,
            );
        }
        if permission.child_process && !self.permissions.child_process {
            push(
                output,
                PluginPolicyViolationCode::ChildProcessNotAllowed,
                &subject,
            );
        }
        if permission.private_service && !self.permissions.private_service {
            push(
                output,
                PluginPolicyViolationCode::PrivateServiceNotAllowed,
                &subject,
            );
        }
        if !permission.secrets.is_empty() && !self.permissions.secrets {
            push(
                output,
                PluginPolicyViolationCode::SecretsNotAllowed,
                &subject,
            );
        }
        for filesystem in &permission.filesystem {
            let allowed = match filesystem.scope {
                FilesystemScope::PluginData => {
                    self.permissions.plugin_data.allows(filesystem.access)
                }
                FilesystemScope::Temporary => self.permissions.temporary.allows(filesystem.access),
                FilesystemScope::Workspace => {
                    self.permissions.workspace.iter().any(|rule| {
                        workspace_rule_allows(rule, &filesystem.path, filesystem.access)
                    })
                }
            };
            if !allowed {
                push(
                    output,
                    PluginPolicyViolationCode::FilesystemNotAllowed,
                    format!(
                        "{subject}:{}:{}",
                        scope_name(filesystem.scope),
                        filesystem.path
                    ),
                );
            }
        }
        for network in &permission.network_egress {
            let allowed = self
                .permissions
                .network
                .iter()
                .find(|rule| rule.host == network.host)
                .is_some_and(|rule| {
                    network
                        .ports
                        .iter()
                        .all(|port| rule.ports.binary_search(port).is_ok())
                });
            if !allowed {
                push(
                    output,
                    PluginPolicyViolationCode::NetworkEgressNotAllowed,
                    format!("{subject}:{}", network.host),
                );
            }
        }
        if permission
            .resources
            .as_ref()
            .is_some_and(|resources| !self.resources_allow(resources))
        {
            push(
                output,
                PluginPolicyViolationCode::ResourceLimitExceeded,
                &subject,
            );
        }
        for binding in &permission.ui_http {
            let allowed = self.permissions.ui_http
                && binding
                    .methods
                    .iter()
                    .all(|method| self.permissions.ui_methods.contains(method))
                && binding.path_prefixes.len() as u32 <= self.permissions.max_ui_path_prefixes;
            if !allowed {
                push(
                    output,
                    PluginPolicyViolationCode::UiHttpNotAllowed,
                    format!("{subject}:{}", binding.tool_id),
                );
            }
        }
    }

    fn resources_allow(&self, resources: &a3s_use_core::ResourcePermissionCeiling) -> bool {
        resources.cpu_millis <= self.permissions.max_cpu_millis
            && resources.memory_bytes <= self.permissions.max_memory_bytes
            && resources.pids <= self.permissions.max_pids
            && resources.ephemeral_storage_bytes <= self.permissions.max_ephemeral_storage_bytes
            && resources
                .task_timeout_ms
                .is_none_or(|value| value <= self.permissions.max_task_timeout_ms)
            && resources
                .max_stdout_bytes
                .is_none_or(|value| value <= self.permissions.max_stdout_bytes)
            && resources
                .max_stderr_bytes
                .is_none_or(|value| value <= self.permissions.max_stderr_bytes)
    }
}

fn agent_hard_denial(plan: &PluginOperationPlan, violations: &[PluginPolicyViolation]) -> bool {
    if plan.authority.actor != PlanActor::Agent {
        return false;
    }
    let unsupported_source = violations
        .iter()
        .any(|violation| violation.code == PluginPolicyViolationCode::UnsupportedSource);
    let grants_secret = plan
        .secret_changes
        .iter()
        .any(|change| change.change == a3s_use_core::PlannedSecretChangeKind::Grant);
    unsupported_source || grants_secret
}

fn workspace_rule_allows(
    rule: &PluginPolicyWorkspaceCeiling,
    path: &str,
    access: a3s_use_core::FilesystemAccess,
) -> bool {
    path_is_within(path, &rule.path) && rule.access.allows(access)
}

fn path_is_within(path: &str, ceiling: &str) -> bool {
    ceiling == "."
        || path == ceiling
        || path
            .strip_prefix(ceiling)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn push(
    output: &mut Vec<PluginPolicyViolation>,
    code: PluginPolicyViolationCode,
    subject: impl Into<String>,
) {
    output.push(PluginPolicyViolation {
        code,
        subject: subject.into(),
    });
}

fn qualified_surface(package_id: &str, kind: PluginSurfaceKind, id: &str) -> String {
    format!("{package_id}:{}:{id}", surface_kind_name(kind))
}

fn surface_kind_name(kind: PluginSurfaceKind) -> &'static str {
    match kind {
        PluginSurfaceKind::Flow => "flow",
        PluginSurfaceKind::Mcp => "mcp",
        PluginSurfaceKind::Okf => "okf",
        PluginSurfaceKind::Skill => "skill",
        PluginSurfaceKind::Tool => "tool",
        PluginSurfaceKind::Ui => "ui",
    }
}

fn scope_name(scope: FilesystemScope) -> &'static str {
    match scope {
        FilesystemScope::PluginData => "plugin-data",
        FilesystemScope::Temporary => "temporary",
        FilesystemScope::Workspace => "workspace",
    }
}
