//! Launch the external A3S Code pager (`a3s-code-tui` + `a3s-code-acp`).
//!
//! Interactive `a3s code` does not run an in-process terminal UI. Session
//! continuation is forwarded as pager flags: an id becomes `--resume`, and a
//! bare `a3s code resume` becomes `--continue`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::args::OutputMode;
use crate::cli::context::InvocationContext;
use crate::cli::output::usage_error;
use crate::git_snapshot::{GitTreeSnapshot, IsolatedWorktree};

/// How `a3s code resume` maps onto the pager's session flags.
enum PagerResume {
    Fresh,
    /// `a3s code resume` with no id continues the newest pager session.
    Continue,
    /// `a3s code resume <id>` loads that session.
    Session(String),
}

fn pager_resume_arguments(resume: &PagerResume) -> Vec<String> {
    match resume {
        PagerResume::Fresh => Vec::new(),
        PagerResume::Continue => vec!["--continue".to_string()],
        PagerResume::Session(id) => vec!["--resume".to_string(), id.clone()],
    }
}

/// Start the a3s-code 9.1.1 full-screen pager as its own process.
///
/// The CLI crate stays on its published core pin. Interactive `a3s code`
/// runs the 9.1.1 pager and `a3s-code-acp`, so the two cores are not linked.
fn code_tui_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("A3S_CODE_TUI_BIN") {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("a3s-code-tui");
            if sibling.is_file() {
                return sibling;
            }
        }
    }
    let forked = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../code/vendor/a3s-code-ui/target/debug/a3s-code-tui");
    if forked.is_file() {
        return forked;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../code/target/debug/a3s-code-tui")
}

/// Moli installed beside the real `a3s` binary (`bin/moli/moli`).
///
/// Homebrew links `/opt/homebrew/bin/a3s` into the Cellar. The sidecar lives
/// next to the canonical executable, not next to the symlink.
fn bundled_moli_executable(executable: &Path) -> Option<PathBuf> {
    let mut seen = Vec::new();
    let mut candidates = vec![executable.to_path_buf()];
    if let Ok(canonical) = std::fs::canonicalize(executable) {
        if canonical != executable {
            candidates.push(canonical);
        }
    }
    for executable in candidates {
        let Some(parent) = executable.parent() else {
            continue;
        };
        let name = if cfg!(windows) { "moli.exe" } else { "moli" };
        for candidate in [parent.join("moli").join(name), parent.join(name)] {
            if seen.iter().any(|current: &PathBuf| current == &candidate) {
                continue;
            }
            seen.push(candidate.clone());
            if is_sidecar_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// `a3s-code-acp` shipped beside the pager, or the local Code debug build.
fn bundled_acp_executable(tui: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "a3s-code-acp.exe"
    } else {
        "a3s-code-acp"
    };
    if let Some(dir) = tui.parent() {
        let sibling = dir.join(name);
        if is_sidecar_executable(&sibling) {
            return Some(sibling);
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../code/target/debug")
        .join(name);
    if is_sidecar_executable(&dev) {
        return Some(dev);
    }
    None
}

fn is_sidecar_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn spawn_code_tui(
    workspace: &Path,
    home: Option<&Path>,
    model_id: &str,
    config_path: &Path,
    resume: &PagerResume,
) -> anyhow::Result<()> {
    let binary = code_tui_binary();
    let mut command = std::process::Command::new(&binary);
    command.arg("--cwd").arg(workspace);
    if !model_id.is_empty() {
        command.arg("--model").arg(model_id);
    }
    for argument in pager_resume_arguments(resume) {
        command.arg(argument);
    }
    if let Some(path) = home {
        command.env("A3S_HOME", path);
    }
    command.env("A3S_CONFIG", config_path);
    if std::env::var_os("A3S_ACP_AGENT_BIN").is_none() {
        if let Some(acp) = bundled_acp_executable(&binary) {
            command.env("A3S_ACP_AGENT_BIN", acp);
        }
    }
    if std::env::var_os("A3S_CODE_MOLI_EXECUTABLE").is_none() {
        if let Ok(executable) = std::env::current_exe() {
            if let Some(moli) = bundled_moli_executable(&executable) {
                command.env("A3S_CODE_MOLI_EXECUTABLE", moli);
            }
        }
    }
    if !model_id.is_empty() {
        command.env("A3S_DEFAULT_MODEL", model_id);
    }
    let status = command.status().map_err(|error| {
        anyhow::anyhow!(
            "failed to start A3S Code TUI at {}: {error}",
            binary.display()
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("A3S Code TUI exited with {status}"))
    }
}

fn resume_from_args(args: &[String]) -> PagerResume {
    let resuming = args.first().map(String::as_str) == Some("resume");
    if !resuming {
        PagerResume::Fresh
    } else if let Some(id) = args.get(1).cloned() {
        PagerResume::Session(id)
    } else {
        PagerResume::Continue
    }
}

/// Launch Code using the directory and configuration resolved at the CLI boundary.
///
/// `workspace` is the pager working directory. It can differ from
/// `context.directory` when `--worktree` created an isolated checkout.
pub(crate) async fn run_in(
    args: Vec<String>,
    workspace: &Path,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    if context.explicit_config.is_none()
        && crate::commands::config_resolver::workspace_config_path(workspace).is_none()
        && context
            .user_config_path()
            .is_none_or(|path| !path.is_file())
    {
        let path = context
            .user_config_path()
            .ok_or_else(|| anyhow::anyhow!("no user home found for ~/.a3s/config.acl"))?;
        crate::config::write_template_config(&path)
            .map_err(|error| anyhow::anyhow!("failed to write starter config {path:?}: {error}"))?;
    }
    let runtime_configuration =
        crate::commands::config::resolve_code_runtime_configuration(context)?;
    let model_id = runtime_configuration
        .config
        .default_model
        .clone()
        .unwrap_or_default();
    spawn_code_tui(
        workspace,
        context.home.as_deref(),
        &model_id,
        &runtime_configuration.config_path,
        &resume_from_args(&args),
    )
}

/// `a3s code --worktree [NAME]`: create an isolated checkout, then start the pager there.
pub(crate) async fn run_in_isolated_worktree(
    name: String,
    context: &InvocationContext,
) -> anyhow::Result<()> {
    if context.output_mode() != OutputMode::Human {
        return Err(usage_error(
            "interactive `a3s code --worktree` requires human output",
        ));
    }
    let source = context.directory.clone();
    let isolated = tokio::task::spawn_blocking(move || create_launch_worktree(&source, &name))
        .await
        .map_err(|error| anyhow::anyhow!("worktree task failed: {error}"))?
        .map_err(anyhow::Error::msg)?;

    eprintln!(
        "a3s: isolated worktree {} (branch {})",
        isolated.workspace.display(),
        isolated.branch
    );
    eprintln!(
        "  remove later with: git -C {} worktree remove {}",
        isolated.source_repository.display(),
        isolated.root.display()
    );

    run_in(Vec::new(), &isolated.workspace, context).await
}

fn create_launch_worktree(
    source_workspace: &Path,
    identity: &str,
) -> Result<IsolatedWorktree, String> {
    let identity = identity.trim();
    let identity = if identity.is_empty() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        format!("launch-{stamp}")
    } else {
        identity.to_string()
    };
    GitTreeSnapshot::capture(source_workspace)
        .and_then(|snapshot| snapshot.fork_worktree(&identity))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_forwards_pager_session_flags() {
        assert!(pager_resume_arguments(&PagerResume::Fresh).is_empty());
        assert_eq!(
            pager_resume_arguments(&PagerResume::Continue),
            vec!["--continue".to_string()]
        );
        assert_eq!(
            pager_resume_arguments(&PagerResume::Session("session-42".to_string())),
            vec!["--resume".to_string(), "session-42".to_string()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn bundled_moli_follows_the_cli_symlink_into_the_cellar() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("tempdir");
        let cellar_bin = root.path().join("Cellar/a3s/0.16.0/bin");
        let moli_dir = cellar_bin.join("moli");
        std::fs::create_dir_all(&moli_dir).expect("cellar moli dir");
        let a3s = cellar_bin.join("a3s");
        let moli = moli_dir.join("moli");
        std::fs::write(&a3s, b"#!/bin/sh\n").expect("a3s fixture");
        std::fs::write(&moli, b"#!/bin/sh\n").expect("moli fixture");
        std::fs::set_permissions(&a3s, std::fs::Permissions::from_mode(0o755)).expect("a3s mode");
        std::fs::set_permissions(&moli, std::fs::Permissions::from_mode(0o755)).expect("moli mode");
        let opt_bin = root.path().join("opt/bin");
        std::fs::create_dir_all(&opt_bin).expect("opt bin");
        let link = opt_bin.join("a3s");
        std::os::unix::fs::symlink(&a3s, &link).expect("homebrew symlink");

        let found = bundled_moli_executable(&link).expect("sidecar");
        assert_eq!(
            std::fs::canonicalize(&found).expect("canonical sidecar"),
            std::fs::canonicalize(&moli).expect("canonical moli")
        );
    }

    #[cfg(unix)]
    #[test]
    fn bundled_acp_uses_the_pager_sibling() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("tempdir");
        let tui = root.path().join("a3s-code-tui");
        let acp = root.path().join("a3s-code-acp");
        std::fs::write(&tui, b"#!/bin/sh\n").expect("tui fixture");
        std::fs::write(&acp, b"#!/bin/sh\n").expect("acp fixture");
        std::fs::set_permissions(&tui, std::fs::Permissions::from_mode(0o755)).expect("tui mode");
        std::fs::set_permissions(&acp, std::fs::Permissions::from_mode(0o755)).expect("acp mode");

        let found = bundled_acp_executable(&tui).expect("acp sibling");
        assert_eq!(found, acp);
    }
}
