//! Self-update version check.

/// The latest published version from GitHub releases (stripped of the `v`), or
/// `None` if offline / the lookup fails. Short timeout so startup never hangs.
pub(crate) async fn check_latest_version() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        std::process::Command::new("curl")
            .args([
                "-fsSL",
                "-m",
                "4",
                "https://api.github.com/repos/A3S-Lab/Cli/releases/latest",
            ])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
            .and_then(|v| {
                v.get("tag_name")?
                    .as_str()
                    .map(|s| s.trim_start_matches('v').to_string())
            })
    })
    .await
    .ok()
    .flatten()
}
