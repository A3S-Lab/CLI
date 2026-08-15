use a3s_code_core::sandbox::BashSandbox;
#[cfg(windows)]
use anyhow::Context;
use serde_json::json;

#[cfg(windows)]
use crate::cli::args::OutputMode;
use crate::cli::args::{CodeSandboxArgs, CodeSandboxCommand};
use crate::cli::context::InvocationContext;
use crate::cli::output::{render_value, usage_error};

pub(super) async fn run(args: CodeSandboxArgs, context: &InvocationContext) -> anyhow::Result<()> {
    match args.command {
        CodeSandboxCommand::Status => status(context).await,
        CodeSandboxCommand::Setup => setup(context).await,
    }
}

async fn status(context: &InvocationContext) -> anyhow::Result<()> {
    let resolution = a3s::components::resolve_managed_srt(
        &context.component_paths,
        &context.directory,
        false,
        context.network.offline,
        false,
    )
    .await;
    let (ready, runtime, diagnostic) = match resolution.runtime {
        Some(runtime) => match runtime.build_and_probe_sandbox(&context.directory).await {
            Ok(sandbox) => {
                sandbox.shutdown().await;
                (true, Some(runtime), None)
            }
            Err(error) => (false, Some(runtime), Some(format!("{error:#}"))),
        },
        None => {
            let diagnostic = resolution.warning.map(|warning| {
                if context.network.offline {
                    warning
                } else {
                    warning.replace(
                        "first-use setup is disabled by A3S_NO_AUTO_INSTALL; run `a3s code` once without that setting",
                        "this status command is read-only and did not perform first-use registry setup; start `a3s code` once while online to prepare it",
                    )
                }
            });
            (false, None, diagnostic)
        }
    };
    let executable = runtime
        .as_ref()
        .map(|runtime| runtime.executable().display().to_string());
    let node = runtime
        .as_ref()
        .map(|runtime| runtime.node().display().to_string());
    let human_diagnostic = diagnostic.clone();

    render_value(
        context.output_mode(),
        "code.sandbox.status",
        json!({
            "ready": ready,
            "runtimeResolved": runtime.is_some(),
            "runtimeVersion": a3s_code_core::sandbox::srt::MANAGED_SRT_VERSION,
            "executable": executable,
            "node": node,
            "diagnostic": diagnostic,
            "setupCommand": if cfg!(windows) { Some("a3s code sandbox setup") } else { None },
        }),
        move || {
            if ready {
                println!(
                    "Local command sandbox is ready (managed SRT {}).",
                    a3s_code_core::sandbox::srt::MANAGED_SRT_VERSION
                );
            } else {
                println!("Local command sandbox is unavailable.");
                if let Some(diagnostic) = human_diagnostic {
                    println!("{diagnostic}");
                }
                #[cfg(windows)]
                println!("Run `a3s code sandbox setup` from an interactive terminal to perform the one-time Windows machine setup.");
            }
        },
    )
}

async fn setup(context: &InvocationContext) -> anyhow::Result<()> {
    #[cfg(not(windows))]
    {
        let _ = context;
        Err(usage_error(
            "`a3s code sandbox setup` is only required on Windows; install the documented native prerequisites, then run `a3s code sandbox status`",
        ))
    }

    #[cfg(windows)]
    {
        if context.output_mode() != OutputMode::Human || context.interaction.non_interactive {
            return Err(usage_error(
                "`a3s code sandbox setup` requires an interactive human terminal because Windows displays a UAC prompt",
            ));
        }
        let resolution = a3s::components::resolve_managed_srt(
            &context.component_paths,
            &context.directory,
            context.network.allow_first_use_install,
            context.network.offline,
            context.output.progress,
        )
        .await;
        let runtime = resolution.runtime.with_context(|| {
            resolution.warning.unwrap_or_else(|| {
                "managed local command sandbox runtime is unavailable".to_string()
            })
        })?;

        println!("Windows will request elevation for the one-time local sandbox machine setup.");
        runtime.setup_windows_machine().await?;
        let sandbox = runtime
            .build_and_probe_sandbox(&context.directory)
            .await
            .context("Windows sandbox setup completed but its capability probe failed")?;
        sandbox.shutdown().await;
        println!("Local command sandbox setup completed and the native boundary probe passed.");
        Ok(())
    }
}
