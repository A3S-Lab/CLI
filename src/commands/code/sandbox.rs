use anyhow::Context;
use serde_json::json;

/// Host-facing diagnosis for a failed native sandbox probe.
///
/// Bash stays denied. This does not change sysctl and does not offer an
/// unsandboxed fallback. Only a known platform refusal gets a repair; other
/// failures keep the probe text.
pub(crate) fn explain_sandbox_probe_failure(error: &anyhow::Error) -> String {
    let raw = format!("{error:#}");
    let repair = sandbox_prerequisite_repair(&raw);
    if repair.is_empty() {
        format!(
            "The native local command sandbox failed its bounded OS capability probe: {raw}. \
             Bash will remain denied in every mode. Repair the reported platform prerequisite \
             and restart `a3s code`"
        )
    } else {
        format!(
            "The native local command sandbox failed its bounded OS capability probe: {raw}. \
             Bash will remain denied in every mode. {repair}"
        )
    }
}

fn sandbox_prerequisite_repair(raw: &str) -> &'static str {
    if raw.contains("setting up uid map") && raw.contains("Permission denied") {
        "Linux denied bubblewrap's unprivileged user-namespace uid map. On that host, as root, set kernel.apparmor_restrict_unprivileged_userns=0 if that key exists and kernel.unprivileged_userns_clone=1 if that key exists. /proc/sys/user/max_user_namespaces must be greater than 0. A container cannot enable this from inside. a3s will not change sysctl and will not run Bash unsandboxed. Restart `a3s code` after the host change."
    } else {
        ""
    }
}

use crate::cli::args::{CodeSandboxArgs, CodeSandboxCommand};
use crate::cli::context::InvocationContext;
use crate::cli::output::render_value;

pub(super) async fn run(args: CodeSandboxArgs, context: &InvocationContext) -> anyhow::Result<()> {
    match args.command {
        CodeSandboxCommand::Status => status(context).await,
        CodeSandboxCommand::Setup => setup(context).await,
    }
}

async fn status(context: &InvocationContext) -> anyhow::Result<()> {
    let result = build_and_probe(&context.directory).await;
    let diagnostic = result.as_ref().err().map(|error| format!("{error:#}"));
    let ready = result.is_ok();
    let human_diagnostic = diagnostic.clone();

    render_value(
        context.output_mode(),
        "code.sandbox.status",
        json!({
            "ready": ready,
            "backend": a3s_code_core::sandbox::native::NATIVE_SANDBOX_BACKEND,
            "workspace": context.directory,
            "setupRequired": false,
            "diagnostic": diagnostic,
        }),
        move || {
            if ready {
                println!(
                    "Native local command sandbox is ready ({}).",
                    a3s_code_core::sandbox::native::NATIVE_SANDBOX_BACKEND
                );
            } else {
                println!("Native local command sandbox is unavailable; Bash is denied.");
                if let Some(diagnostic) = human_diagnostic {
                    println!(
                        "{}",
                        explain_sandbox_probe_failure(&anyhow::anyhow!(diagnostic))
                    );
                }
            }
        },
    )
}

async fn setup(context: &InvocationContext) -> anyhow::Result<()> {
    let sandbox = build_and_probe(&context.directory).await?;
    let backend = sandbox.backend();
    render_value(
        context.output_mode(),
        "code.sandbox.setup",
        json!({
            "ready": true,
            "backend": backend,
            "workspace": sandbox.workspace(),
            "changed": false,
        }),
        move || {
            println!(
                "Native local command sandbox needs no managed runtime setup; the {backend} boundary probe passed."
            );
        },
    )
}

async fn build_and_probe(
    workspace: &std::path::Path,
) -> anyhow::Result<a3s_code_core::sandbox::native::NativeBashSandbox> {
    let sandbox = a3s_code_core::sandbox::native::NativeBashSandbox::new(workspace)
        .context("failed to initialize the native local command sandbox")?;
    sandbox
        .probe()
        .await
        .context("native local command sandbox capability probe failed")?;
    Ok(sandbox)
}

#[cfg(test)]
mod tests {
    use a3s_code_core::sandbox::BashSandbox;

    #[test]
    fn uid_map_denial_names_the_user_namespace_prerequisite_and_keeps_bash_denied() {
        let error = anyhow::anyhow!(
            "native sandbox capability probe returned exit code 1 with stdout \"\" and stderr \"bwrap: setting up uid map: Permission denied\\n\""
        );
        let warning = super::explain_sandbox_probe_failure(&error);
        assert!(warning.contains("Bash will remain denied"));
        assert!(warning.contains("unprivileged user-namespace"));
        assert!(warning.contains("will not run Bash unsandboxed"));
        assert!(!warning.contains("Repair the reported platform prerequisite"));
    }

    #[test]
    fn unrelated_probe_failure_does_not_invent_a_user_namespace_repair() {
        let error = anyhow::anyhow!("native sandbox workspace exceeds the 1000000 entry scan limit");
        let warning = super::explain_sandbox_probe_failure(&error);
        assert!(warning.contains("1000000 entry scan limit"));
        assert!(!warning.contains("unprivileged user-namespace"));
    }

    #[tokio::test]
    #[ignore = "requires the native sandbox prerequisite for the host platform"]
    async fn real_native_sandbox_enforces_local_policy() {
        let workspace = tempfile::tempdir().expect("temporary workspace");
        std::fs::create_dir_all(workspace.path().join(".git")).expect("create .git fixture");
        std::fs::create_dir_all(workspace.path().join(".a3s")).expect("create .a3s fixture");
        std::fs::write(workspace.path().join(".git/config"), "original")
            .expect("write protected fixture");
        let sandbox = a3s_code_core::sandbox::native::NativeBashSandbox::new(workspace.path())
            .expect("initialize native sandbox");

        sandbox.probe().await.expect("probe native sandbox");

        #[cfg(windows)]
        let ordinary_command =
            "[IO.File]::WriteAllText((Join-Path (Get-Location) 'ordinary.txt'), 'changed')";
        #[cfg(not(windows))]
        let ordinary_command = "printf changed > ordinary.txt";
        let ordinary = sandbox
            .exec_command(ordinary_command, "/workspace")
            .await
            .expect("run ordinary write");
        assert_eq!(ordinary.exit_code, 0, "{}", ordinary.stderr);
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("ordinary.txt")).unwrap(),
            "changed"
        );

        #[cfg(windows)]
        let protected_command =
            "[IO.File]::WriteAllText((Join-Path (Get-Location) '.git\\config'), 'changed')";
        #[cfg(not(windows))]
        let protected_command = "printf changed > .git/config";
        let protected = sandbox
            .exec_command(protected_command, "/workspace")
            .await
            .expect("run protected write probe");
        assert_ne!(
            protected.exit_code, 0,
            "protected write unexpectedly passed"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.path().join(".git/config")).unwrap(),
            "original"
        );
    }
}
