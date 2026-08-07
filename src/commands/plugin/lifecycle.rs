use std::fmt::Write as _;
use std::process::ExitCode;

use a3s::plugin_manager::{
    PluginApplyRequest, PluginEnablementPlanRequest, PluginLifecycleAction, PluginManager,
    PluginPlanRequest,
};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::cli::args::{
    OutputMode, PluginApplyArgs, PluginInstallArgs, PluginMutationArgs, PluginReviewArgs,
    PluginToggleArgs,
};
use crate::cli::context::InvocationContext;
use crate::cli::output;

pub(super) async fn install(
    manager: &PluginManager,
    args: PluginInstallArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let package_id = super::normalize_package_id(&args.package_id)?;
    let request = PluginPlanRequest {
        action: PluginLifecycleAction::Install,
        component_id: component_id(&package_id),
        version: args.version,
        channel: args.channel.map(|channel| channel.as_str().to_string()),
    };
    plan_and_apply(manager, request, package_id, args.review, context).await
}

pub(super) async fn upgrade(
    manager: &PluginManager,
    args: PluginMutationArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    lifecycle_mutation(manager, PluginLifecycleAction::Upgrade, args, context).await
}

pub(super) async fn uninstall(
    manager: &PluginManager,
    args: PluginMutationArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    lifecycle_mutation(manager, PluginLifecycleAction::Uninstall, args, context).await
}

pub(super) async fn apply(
    manager: &PluginManager,
    args: PluginApplyArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    ensure_confirmation_available(
        args.yes,
        context,
        "plugin apply requires `--yes` in non-interactive mode",
    )?;
    if !args.yes {
        confirm(
            &format!(
                "Apply reviewed plugin operation {}?",
                super::single_line(&args.operation_id, 256)
            ),
            context,
            "plugin apply cancelled",
            json!({"operationId": args.operation_id}),
        )
        .await?;
    }
    let result = manager
        .apply_confirmed_operation(&PluginApplyRequest {
            operation_id: args.operation_id,
            plan_digest: args.plan_digest,
        })
        .await
        .map_err(super::manager_error)?;
    render_result("plugin.apply", "Plugin operation result", result, context)
}

pub(super) async fn set_enabled(
    manager: &PluginManager,
    args: PluginToggleArgs,
    enabled: bool,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let package_id = super::normalize_package_id(&args.package_id)?;
    let action = if enabled { "enable" } else { "disable" };
    let command = if enabled {
        "plugin.enable"
    } else {
        "plugin.disable"
    };
    if !args.review.dry_run {
        ensure_confirmation_available(
            args.review.yes,
            context,
            &format!("plugin {action} requires `--yes` or `--dry-run` in non-interactive mode"),
        )?;
    }

    let plan = manager
        .plan_package_enablement(&PluginEnablementPlanRequest {
            component_id: component_id(&package_id),
            enabled,
            expected_package_generation: None,
        })
        .await
        .map_err(super::manager_error)?;
    let human_plan = format_reviewed_plan(action, &package_id, &plan)?;
    if args.review.dry_run || plan_status(&plan) == Some("no-change") {
        output::render_value(context.output_mode(), command, plan, || {
            println!("{human_plan}");
        })?;
        return Ok(ExitCode::SUCCESS);
    }

    if context.output_mode() == OutputMode::Human {
        println!("{human_plan}");
    }
    let (operation_id, plan_digest) = plan_identity(&plan)?;
    if !args.review.yes {
        confirm(
            &format!("Apply this exact {action} plan for {package_id}?"),
            context,
            &format!("plugin {action} cancelled"),
            json!({
                "operationId": operation_id,
                "planDigest": plan_digest,
                "packageId": package_id,
            }),
        )
        .await?;
    }
    let result = manager
        .apply_confirmed_operation(&PluginApplyRequest {
            operation_id,
            plan_digest,
        })
        .await
        .map_err(super::manager_error)?;
    render_result(command, "Plugin desired-state result", result, context)
}

async fn lifecycle_mutation(
    manager: &PluginManager,
    action: PluginLifecycleAction,
    args: PluginMutationArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let package_id = super::normalize_package_id(&args.package_id)?;
    let request = PluginPlanRequest {
        action,
        component_id: component_id(&package_id),
        version: None,
        channel: None,
    };
    plan_and_apply(manager, request, package_id, args.review, context).await
}

async fn plan_and_apply(
    manager: &PluginManager,
    request: PluginPlanRequest,
    package_id: String,
    review: PluginReviewArgs,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let command = command_name(request.action);
    if !review.dry_run {
        ensure_confirmation_available(
            review.yes,
            context,
            &format!("{command} requires `--yes` or `--dry-run` in non-interactive mode"),
        )?;
    }

    let plan = manager
        .plan_operation(&request)
        .await
        .map_err(super::manager_error)?;
    let human_plan = format_plan(request.action, &package_id, &plan)?;
    if review.dry_run {
        output::render_value(context.output_mode(), command, plan, || {
            println!("{human_plan}");
        })?;
        return Ok(ExitCode::SUCCESS);
    }

    if context.output_mode() == OutputMode::Human {
        println!("{human_plan}");
    }
    let (operation_id, plan_digest) = plan_identity(&plan)?;
    if !review.yes {
        confirm(
            &format!(
                "Apply this exact {} plan for {}?",
                action_name(request.action),
                package_id
            ),
            context,
            &format!("plugin {} cancelled", action_name(request.action)),
            json!({
                "operationId": operation_id,
                "planDigest": plan_digest,
                "packageId": package_id,
            }),
        )
        .await?;
    }

    let result = manager
        .apply_confirmed_operation(&PluginApplyRequest {
            operation_id,
            plan_digest,
        })
        .await
        .map_err(super::manager_error)?;
    render_result(command, "Plugin lifecycle result", result, context)
}

fn render_result(
    command: &'static str,
    title: &'static str,
    result: Value,
    context: &InvocationContext,
) -> anyhow::Result<ExitCode> {
    let human = terminal_safe_pretty_json(&result)?;
    output::render_value(context.output_mode(), command, result, || {
        println!("{title}:");
        println!("{human}");
    })?;
    Ok(ExitCode::SUCCESS)
}

fn format_plan(
    action: PluginLifecycleAction,
    package_id: &str,
    plan: &Value,
) -> anyhow::Result<String> {
    format_reviewed_plan(action_name(action), package_id, plan)
}

fn format_reviewed_plan(action: &str, package_id: &str, plan: &Value) -> anyhow::Result<String> {
    let operation_id = plan
        .get("operationId")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    let plan_digest = plan
        .get("canonicalPlanDigest")
        .and_then(Value::as_str)
        .or_else(|| plan.get("planDigest").and_then(Value::as_str))
        .unwrap_or("unavailable");
    let expires_at_ms = plan
        .pointer("/plan/plan/expiresAtMs")
        .or_else(|| plan.get("expiresAtMs"))
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
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
        super::single_line(operation_id, 256)
    )?;
    writeln!(
        &mut output,
        "digest: {}",
        super::single_line(plan_digest, 80)
    )?;
    writeln!(&mut output, "expiresAtMs: {expires_at_ms}")?;
    if let Some(status) = plan_status(plan) {
        writeln!(&mut output, "status: {}", super::single_line(status, 32))?;
    }
    writeln!(&mut output, "review:")?;
    output.push_str(&terminal_safe_pretty_json(plan)?);
    Ok(output)
}

fn plan_status(plan: &Value) -> Option<&str> {
    plan.get("status").and_then(Value::as_str)
}

fn plan_identity(plan: &Value) -> anyhow::Result<(String, String)> {
    let operation_id = plan
        .get("operationId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            output::coded_error(
                "plugin.plan_invalid",
                "the shared Plugin Manager returned a plan without an operation ID",
                output::ExitClass::Failure,
            )
        })?;
    let plan_digest = plan
        .get("canonicalPlanDigest")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            output::coded_error(
                "plugin.plan_invalid",
                "the shared Plugin Manager returned a plan without a canonical digest",
                output::ExitClass::Failure,
            )
        })?;
    Ok((operation_id.to_string(), plan_digest.to_string()))
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

fn affirmative(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn component_id(package_id: &str) -> String {
    format!("use/{package_id}")
}

const fn command_name(action: PluginLifecycleAction) -> &'static str {
    match action {
        PluginLifecycleAction::Install => "plugin.install",
        PluginLifecycleAction::Upgrade => "plugin.upgrade",
        PluginLifecycleAction::Uninstall => "plugin.uninstall",
    }
}

const fn action_name(action: PluginLifecycleAction) -> &'static str {
    match action {
        PluginLifecycleAction::Install => "install",
        PluginLifecycleAction::Upgrade => "upgrade",
        PluginLifecycleAction::Uninstall => "uninstall",
    }
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
    fn reviewed_plan_identity_requires_manager_owned_fields() {
        let identity = plan_identity(&json!({
            "operationId": "plugin-install-abc",
            "canonicalPlanDigest": format!("sha256:{}", "a".repeat(64)),
        }))
        .unwrap();
        assert_eq!(identity.0, "plugin-install-abc");
        assert_eq!(identity.1, format!("sha256:{}", "a".repeat(64)));
        assert!(plan_identity(&json!({"operationId": "plugin-install-abc"})).is_err());
    }

    #[test]
    fn enablement_plan_rendering_exposes_nested_lifetime_and_no_change_status() {
        let plan = json!({
            "componentId": "use/acme/guide",
            "status": "planned",
            "operationId": "plugin-enablement:guide",
            "canonicalPlanDigest": format!("sha256:{}", "a".repeat(64)),
            "plan": {"plan": {"expiresAtMs": 42}},
        });
        let rendered = format_reviewed_plan("disable", "acme/guide", &plan).unwrap();
        assert!(rendered.contains("Plugin disable plan for acme/guide"));
        assert!(rendered.contains("expiresAtMs: 42"));
        assert!(rendered.contains("status: planned"));

        assert_eq!(
            plan_status(&json!({"status": "no-change"})),
            Some("no-change")
        );
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
}
