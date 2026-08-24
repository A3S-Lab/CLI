use std::fmt::Write as _;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use a3s_use::cognitive_package::CognitiveRegistryAccess;
use a3s_use::plugin_manager::PluginManagerService;
use a3s_use_core::{
    PlanActor, PlanPolicyDecision, PlanScopeKind, PluginHostEnablementPlanResult,
    PluginHostEnablementPlanStatus, PluginHostPlanResult, PluginManagerApplyPlanInput,
    PluginManagerInstallPlanInput, PluginManagerPackageScopeInput, PluginManagerUpgradePlanInput,
    PluginOperationAction, PluginOperationConfirmation, PluginOperationPlanEnvelope,
    PluginPackageId, PLUGIN_OPERATION_CONFIRMATION_SCHEMA,
};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::cli::args::{
    OutputMode, PluginApplyArgs, PluginInstallArgs, PluginMutationArgs, PluginReviewArgs,
    PluginToggleArgs,
};
use crate::cli::context::InvocationContext;
use crate::cli::output;
use a3s::plugin_manager::review::plan_review_fields;

pub(super) async fn install(
    service: &PluginManagerService,
    args: PluginInstallArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    require_review_authority(
        &args.review,
        context,
        "plugin install requires `--yes` or `--dry-run` in non-interactive mode",
    )?;
    let package_id = package_id(&args.package_id)?;
    let plan = service
        .plan_install(
            PluginManagerInstallPlanInput {
                package_id: package_id.clone(),
                registry_name: args.registry_name,
                version_requirement: args.version,
                channel: args.channel.map(super::release_channel),
                surfaces: None,
                scope_kind: PlanScopeKind::User,
                scope_id: managed_scope_id(),
            },
            registry_access(context),
        )
        .await
        .map_err(super::service_error)?;
    review_graph_plan(
        service,
        plan,
        PluginOperationAction::Install,
        package_id.as_str(),
        args.review,
        context,
    )
    .await
}

pub(super) async fn upgrade(
    service: &PluginManagerService,
    args: PluginMutationArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    require_review_authority(
        &args.review,
        context,
        "plugin upgrade requires `--yes` or `--dry-run` in non-interactive mode",
    )?;
    let package_id = package_id(&args.package_id)?;
    let plan = service
        .plan_upgrade(
            PluginManagerUpgradePlanInput {
                package_id: package_id.clone(),
                version_requirement: None,
                channel: None,
                surfaces: None,
                scope_kind: PlanScopeKind::User,
                scope_id: managed_scope_id(),
            },
            registry_access(context),
        )
        .await
        .map_err(super::service_error)?;
    review_graph_plan(
        service,
        plan,
        PluginOperationAction::Upgrade,
        package_id.as_str(),
        args.review,
        context,
    )
    .await
}

pub(super) async fn uninstall(
    service: &PluginManagerService,
    args: PluginMutationArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    require_review_authority(
        &args.review,
        context,
        "plugin uninstall requires `--yes` or `--dry-run` in non-interactive mode",
    )?;
    let package_id = package_id(&args.package_id)?;
    let plan = service
        .plan_uninstall(package_scope(package_id.clone()))
        .await
        .map_err(super::service_error)?;
    review_graph_plan(
        service,
        plan,
        PluginOperationAction::Uninstall,
        package_id.as_str(),
        args.review,
        context,
    )
    .await
}

pub(super) async fn apply(
    service: &PluginManagerService,
    args: PluginApplyArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    ensure_confirmation_available(
        args.yes,
        context,
        "plugin apply requires `--yes` in non-interactive mode",
    )?;
    let input = PluginManagerApplyPlanInput {
        operation_id: args.operation_id,
        plan_digest: args.plan_digest,
    };
    let reviewed = service
        .reviewed_plan(&input)
        .await
        .map_err(super::service_error)?;
    if context.output_mode() == OutputMode::Human {
        println!(
            "{}",
            format_graph_plan(
                action_name(reviewed.plan.plan.action),
                reviewed.package_id.as_str(),
                &reviewed,
            )?
        );
    }
    if !args.yes {
        confirm(
            &format!(
                "Apply exact reviewed operation {} with digest {}?",
                super::single_line(&input.operation_id, 256),
                super::single_line(&input.plan_digest, 80),
            ),
            context,
            "plugin apply cancelled",
            json!({
                "operationId": input.operation_id,
                "planDigest": input.plan_digest,
            }),
        )
        .await?;
    }
    let confirmation = confirmation_for(&reviewed)?;
    let result = service
        .apply_plan(input, confirmation)
        .await
        .map_err(super::service_error)?;
    render_result("plugin.apply", "Plugin operation result", &result, context)
}

pub(super) async fn set_enabled(
    service: &PluginManagerService,
    args: PluginToggleArgs,
    enabled: bool,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let action = if enabled { "enable" } else { "disable" };
    let command = if enabled {
        "plugin.enable"
    } else {
        "plugin.disable"
    };
    require_review_authority(
        &args.review,
        context,
        &format!("plugin {action} requires `--yes` or `--dry-run` in non-interactive mode"),
    )?;
    let package_id = package_id(&args.package_id)?;
    let input = package_scope(package_id.clone());
    let plan = if enabled {
        service.plan_enable(input).await
    } else {
        service.plan_disable(input).await
    }
    .map_err(super::service_error)?;
    let human_plan = format_enablement_plan(action, package_id.as_str(), &plan)?;
    if args.review.dry_run || plan.status == PluginHostEnablementPlanStatus::NoChange {
        return render_plan(command, &plan, &human_plan, context);
    }

    if context.output_mode() == OutputMode::Human {
        println!("{human_plan}");
    }
    let reviewed = plan.reviewed_plan().map_err(super::service_error)?;
    let apply_input = apply_input(&reviewed);
    if !args.review.yes {
        confirm(
            &format!(
                "Apply this exact {action} plan for {}?",
                package_id.as_str()
            ),
            context,
            &format!("plugin {action} cancelled"),
            json!({
                "operationId": apply_input.operation_id,
                "planDigest": apply_input.plan_digest,
                "packageId": package_id.as_str(),
            }),
        )
        .await?;
    }
    let confirmation = confirmation_for(&reviewed)?;
    let result = service
        .apply_plan(apply_input, confirmation)
        .await
        .map_err(super::service_error)?;
    render_result(command, "Plugin desired-state result", &result, context)
}

async fn review_graph_plan(
    service: &PluginManagerService,
    plan: PluginHostPlanResult,
    expected_action: PluginOperationAction,
    package_id: &str,
    review: PluginReviewArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    if plan.plan.plan.action != expected_action || plan.package_id.as_str() != package_id {
        return Err(plan_invalid(
            "the shared Plugin Manager changed the requested action or package identity",
        ));
    }
    let command = command_name(expected_action);
    let human_plan = format_graph_plan(action_name(expected_action), package_id, &plan)?;
    if review.dry_run {
        return render_plan(command, &plan, &human_plan, context);
    }

    if context.output_mode() == OutputMode::Human {
        println!("{human_plan}");
    }
    let input = apply_input(&plan);
    if !review.yes {
        confirm(
            &format!(
                "Apply this exact {} plan for {package_id}?",
                action_name(expected_action)
            ),
            context,
            &format!("plugin {} cancelled", action_name(expected_action)),
            json!({
                "operationId": input.operation_id,
                "planDigest": input.plan_digest,
                "packageId": package_id,
            }),
        )
        .await?;
    }
    let confirmation = confirmation_for(&plan)?;
    let result = service
        .apply_plan(input, confirmation)
        .await
        .map_err(super::service_error)?;
    render_result(command, "Plugin lifecycle result", &result, context)
}

fn apply_input(plan: &PluginHostPlanResult) -> PluginManagerApplyPlanInput {
    PluginManagerApplyPlanInput {
        operation_id: plan.plan.plan.operation_id.clone(),
        plan_digest: plan.plan.plan_digest.clone(),
    }
}

fn confirmation_for(
    plan: &PluginHostPlanResult,
) -> anyhow::Result<Option<PluginOperationConfirmation>> {
    if plan.plan.plan.authority.actor != PlanActor::User {
        return Err(plan_invalid(
            "the reviewed Plugin Manager plan does not belong to the user actor",
        ));
    }
    let confirmation = match plan.plan.plan.authority.decision {
        PlanPolicyDecision::Ask => Some(PluginOperationConfirmation {
            schema: PLUGIN_OPERATION_CONFIRMATION_SCHEMA.to_string(),
            operation_id: plan.plan.plan.operation_id.clone(),
            plan_digest: plan.plan.plan_digest.clone(),
            confirmed_by: PlanActor::User,
            confirmed_at_ms: unix_time_millis()?,
        }),
        PlanPolicyDecision::Allow | PlanPolicyDecision::Deny => None,
    };
    if let Some(confirmation) = &confirmation {
        confirmation.validate().map_err(super::service_error)?;
    }
    Ok(confirmation)
}

fn render_result<T: Serialize>(
    command: &'static str,
    title: &'static str,
    result: &T,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let value = serde_json::to_value(result)?;
    let human = terminal_safe_pretty_json(&value)?;
    output::render_value(context.output_mode(), command, value, || {
        println!("{title}:");
        println!("{human}");
    })?;
    Ok(ExitCode::SUCCESS)
}

fn render_plan<T: Serialize>(
    command: &'static str,
    plan: &T,
    human: &str,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let value = serde_json::to_value(plan)?;
    output::render_value(context.output_mode(), command, value, || {
        println!("{human}");
    })?;
    Ok(ExitCode::SUCCESS)
}

fn format_graph_plan(
    action: &str,
    package_id: &str,
    plan: &PluginHostPlanResult,
) -> anyhow::Result<String> {
    format_reviewed_plan(
        action,
        package_id,
        Some(&plan.plan.plan.operation_id),
        Some(&plan.plan.plan_digest),
        Some(plan.plan.plan.expires_at_ms),
        None,
        Some(&plan.plan),
        plan,
    )
}

fn format_enablement_plan(
    action: &str,
    package_id: &str,
    plan: &PluginHostEnablementPlanResult,
) -> anyhow::Result<String> {
    let identity = plan.plan.as_ref();
    format_reviewed_plan(
        action,
        package_id,
        identity.map(|plan| &plan.plan.operation_id),
        identity.map(|plan| &plan.plan_digest),
        identity.map(|plan| plan.plan.expires_at_ms),
        Some(match plan.status {
            PluginHostEnablementPlanStatus::NoChange => "no-change",
            PluginHostEnablementPlanStatus::Planned => "planned",
        }),
        identity,
        plan,
    )
}

#[allow(clippy::too_many_arguments)]
fn format_reviewed_plan<T: Serialize>(
    action: &str,
    package_id: &str,
    operation_id: Option<&String>,
    plan_digest: Option<&String>,
    expires_at_ms: Option<u64>,
    status: Option<&str>,
    envelope: Option<&PluginOperationPlanEnvelope>,
    plan: &T,
) -> anyhow::Result<String> {
    let mut output = String::new();
    writeln!(
        &mut output,
        "Plugin {} plan for {}",
        action,
        super::single_line(package_id, 256)
    )?;
    writeln!(
        &mut output,
        "operation: {}",
        operation_id.map_or("none", String::as_str)
    )?;
    writeln!(
        &mut output,
        "digest: {}",
        plan_digest.map_or("none", String::as_str)
    )?;
    writeln!(
        &mut output,
        "expiresAtMs: {}",
        expires_at_ms.map_or_else(|| "none".to_string(), |value| value.to_string())
    )?;
    if let Some(status) = status {
        writeln!(&mut output, "status: {}", super::single_line(status, 32))?;
    }
    if let Some(envelope) = envelope {
        writeln!(&mut output, "exact review:")?;
        for field in plan_review_fields(envelope).map_err(plan_invalid)? {
            writeln!(
                &mut output,
                "{}: {}",
                terminal_safe_string(&field.label),
                terminal_safe_string(&field.value)
            )?;
        }
    }
    writeln!(&mut output, "manager contract:")?;
    output.push_str(&terminal_safe_pretty_json(&serde_json::to_value(plan)?)?);
    Ok(output)
}

fn require_review_authority(
    review: &PluginReviewArgs,
    context: &InvocationContext,
    message: &str,
) -> anyhow::Result<()> {
    if review.dry_run {
        Ok(())
    } else {
        ensure_confirmation_available(review.yes, context, message)
    }
}

fn ensure_confirmation_available(
    accepted: bool,
    context: &InvocationContext,
    message: &str,
) -> anyhow::Result<()> {
    if accepted {
        return Ok(());
    }
    if context.output_mode() != OutputMode::Human
        || context.interaction.non_interactive
        || !context.terminal.stdin
        || !context.terminal.stderr
    {
        return Err(output::usage_error(message));
    }
    Ok(())
}

async fn confirm(
    prompt: &str,
    context: &InvocationContext,
    cancelled_message: &str,
    details: Value,
) -> anyhow::Result<()> {
    let mut stderr = tokio::io::stderr();
    stderr.write_all(prompt.as_bytes()).await?;
    stderr.write_all(b" [y/N] ").await?;
    stderr.flush().await?;

    let mut answer = String::new();
    let mut reader = BufReader::new(tokio::io::stdin().take(32));
    let read = tokio::select! {
        _ = context.cancellation.cancelled() => {
            return Err(cancelled_error(cancelled_message, details));
        }
        result = reader.read_line(&mut answer) => result?,
    };
    if read > 0 && affirmative(&answer) {
        Ok(())
    } else {
        Err(cancelled_error(cancelled_message, details))
    }
}

fn cancelled_error(message: &str, details: Value) -> anyhow::Error {
    output::CliError::new("operation.cancelled", message, output::ExitClass::Cancelled)
        .with_details(details)
        .into()
}

fn package_id(value: &str) -> anyhow::Result<PluginPackageId> {
    PluginPackageId::parse(super::normalize_package_id(value)?).map_err(super::service_error)
}

fn package_scope(package_id: PluginPackageId) -> PluginManagerPackageScopeInput {
    PluginManagerPackageScopeInput {
        package_id,
        scope_kind: PlanScopeKind::User,
        scope_id: managed_scope_id(),
    }
}

fn managed_scope_id() -> String {
    a3s_use::COGNITIVE_PACKAGE_DEFAULT_SCOPE.to_string()
}

fn registry_access(context: &InvocationContext) -> CognitiveRegistryAccess {
    if context.network.offline {
        CognitiveRegistryAccess::Cached
    } else {
        CognitiveRegistryAccess::Refreshed
    }
}

const fn command_name(action: PluginOperationAction) -> &'static str {
    match action {
        PluginOperationAction::Install => "plugin.install",
        PluginOperationAction::Upgrade => "plugin.upgrade",
        PluginOperationAction::Uninstall => "plugin.uninstall",
        PluginOperationAction::Enable => "plugin.enable",
        PluginOperationAction::Disable => "plugin.disable",
    }
}

const fn action_name(action: PluginOperationAction) -> &'static str {
    match action {
        PluginOperationAction::Install => "install",
        PluginOperationAction::Upgrade => "upgrade",
        PluginOperationAction::Uninstall => "uninstall",
        PluginOperationAction::Enable => "enable",
        PluginOperationAction::Disable => "disable",
    }
}

fn unix_time_millis() -> anyhow::Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| plan_invalid(format!("system clock is before the Unix epoch: {error}")))?
        .as_millis();
    u64::try_from(millis)
        .ok()
        .filter(|millis| *millis > 0)
        .ok_or_else(|| plan_invalid("system time is outside the supported millisecond range"))
}

fn plan_invalid(message: impl Into<String>) -> anyhow::Error {
    output::coded_error(
        "plugin.plan_invalid",
        message.into(),
        output::ExitClass::Failure,
    )
}

fn affirmative(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn terminal_safe_pretty_json(value: &Value) -> anyhow::Result<String> {
    serde_json::to_string_pretty(&terminal_safe_value(value)).map_err(Into::into)
}

fn terminal_safe_value(value: &Value) -> Value {
    match value {
        Value::String(value) => Value::String(terminal_safe_string(value)),
        Value::Array(items) => Value::Array(items.iter().map(terminal_safe_value).collect()),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (terminal_safe_string(key), terminal_safe_value(value)))
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
    }
}

fn terminal_safe_string(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if super::unsafe_terminal_character(character) {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_inputs_use_the_standard_user_scope() {
        let input = package_scope(PluginPackageId::parse("acme/guide").unwrap());
        assert_eq!(input.scope_kind, PlanScopeKind::User);
        assert_eq!(input.scope_id, "user/current");
        assert_eq!(input.package_id.as_str(), "acme/guide");
    }

    #[test]
    fn terminal_rendering_neutralizes_controls_and_bidi_overrides() {
        let rendered = terminal_safe_pretty_json(&json!({
            "message": "safe\u{1b}[31m\u{202e}spoof",
            "unsafe\u{202e}key": true,
        }))
        .unwrap();

        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\u{202e}'));
        assert!(rendered.contains('\u{fffd}'));
        assert!(rendered.contains("unsafe\u{fffd}key"));
    }

    #[test]
    fn confirmation_accepts_only_an_explicit_yes() {
        assert!(affirmative("y\n"));
        assert!(affirmative("YES"));
        assert!(!affirmative(""));
        assert!(!affirmative("true"));
    }

    #[test]
    fn cli_human_review_names_the_exact_graph_source_ceiling_and_confirmation() {
        let envelope = crate::plugin_plan_review_test_fixture::install_envelope();
        let rendered = format_reviewed_plan(
            "install",
            "acme/guide",
            Some(&envelope.plan.operation_id),
            Some(&envelope.plan_digest),
            Some(envelope.plan.expires_at_ms),
            None,
            Some(&envelope),
            &envelope,
        )
        .unwrap();

        for section in [
            "plan:",
            "packageGraph:",
            "transition.acme/base:",
            "source.acme/guide:",
            "permissionCeiling.acme/guide.after:",
            "confirmationBoundary:",
        ] {
            assert!(rendered.contains(section), "missing {section}:\n{rendered}");
        }
        assert!(rendered.contains(&envelope.plan_digest));
        assert!(rendered.contains("api.example.com"));
        assert!(rendered.contains("research-api"));
        assert!(rendered.contains("confirmationRequired\":true"));
    }
}
