use anyhow::Context;
use serde_json::json;

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
                    println!("{diagnostic}");
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
