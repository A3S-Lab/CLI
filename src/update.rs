//! Self-update, shared by the TUI `/update` and the `a3s update` CLI command.
//!
//! Tries Homebrew (how a3s is usually installed) and **falls back to a direct
//! binary download** if brew or the tap is in any bad state — so an update can
//! never be blocked again by a stale tap clone or a broken `brew upgrade`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use a3s_updater::{extract_tar_gz_archive, verify_sha256};

struct CommandOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

trait CommandRunner {
    fn output(&self, program: &OsStr, args: &[OsString]) -> Option<CommandOutput>;
    fn status(&self, program: &OsStr, args: &[OsString]) -> bool;
}

struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn output(&self, program: &OsStr, args: &[OsString]) -> Option<CommandOutput> {
        let out = Command::new(program).args(args).output().ok()?;
        Some(CommandOutput {
            success: out.status.success(),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }

    fn status(&self, program: &OsStr, args: &[OsString]) -> bool {
        Command::new(program)
            .args(args)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn args(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}

const BREW_TAP: &str = "a3s-lab/tap";
const BREW_TAP_URL: &str = "https://github.com/A3S-Lab/homebrew-tap";
const BREW_FORMULA: &str = "a3s-lab/tap/a3s";
const BREW_SHORT_FORMULA: &str = "a3s";
const WEBVIEW_FORMULA: &str = "a3s-lab/tap/a3s-webview";
const MAX_SELF_UPDATE_ARCHIVE_BYTES: usize = 512 * 1024 * 1024;
const WEBVIEW_BINARY: &str = if cfg!(windows) {
    "a3s-webview.exe"
} else {
    "a3s-webview"
};
const LATEST_RELEASE_REDIRECT_ARGS: &[&str] = &[
    "-fsSL",
    "--connect-timeout",
    "5",
    "--max-time",
    "12",
    "-o",
    "/dev/null",
    "-w",
    "%{url_effective}",
    "https://github.com/A3S-Lab/Cli/releases/latest",
];
const LATEST_RELEASE_API_ARGS: &[&str] = &[
    "-fsSL",
    "--connect-timeout",
    "5",
    "--max-time",
    "12",
    "https://api.github.com/repos/A3S-Lab/Cli/releases/latest",
];

fn numeric_version_parts(s: &str) -> Vec<u32> {
    let trimmed = s.trim().trim_start_matches('v');
    let core = trimmed.split(['-', '+']).next().unwrap_or(trimmed);
    let mut parts = Vec::new();
    for part in core.split('.') {
        let digits = part
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        if digits.is_empty() {
            break;
        }
        match digits.parse::<u32>() {
            Ok(n) => parts.push(n),
            Err(_) => break,
        }
    }
    parts
}

/// Compare stable numeric version components with optional `v` prefixes.
pub(crate) fn version_ge(a: &str, b: &str) -> bool {
    let mut av = numeric_version_parts(a);
    let mut bv = numeric_version_parts(b);
    if av.is_empty() || bv.is_empty() {
        return false;
    }
    let len = av.len().max(bv.len());
    av.resize(len, 0);
    bv.resize(len, 0);
    av >= bv
}

/// Latest release version tag from GitHub (no leading `v`), or `None` if the
/// release server is unreachable. Blocking — use [`fetch_latest_async`] from
/// cancellation-sensitive async flows.
///
/// Uses the `releases/latest` REDIRECT on github.com (which 302s to
/// `…/releases/tag/vX.Y.Z`) first because it avoids unauthenticated REST API
/// rate limits, then falls back to the GitHub API when the redirect is
/// unavailable.
pub(crate) fn fetch_latest() -> Option<String> {
    fetch_latest_from_redirect().or_else(fetch_latest_from_api)
}

fn fetch_latest_from_redirect() -> Option<String> {
    let out = Command::new("curl")
        .args(LATEST_RELEASE_REDIRECT_ARGS)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    version_from_release_url(&String::from_utf8_lossy(&out.stdout))
}

fn fetch_latest_from_api() -> Option<String> {
    let out = Command::new("curl")
        .args(LATEST_RELEASE_API_ARGS)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    version_from_api_response(&out.stdout)
}

/// Async latest-release lookup whose `curl` process is terminated if the
/// caller is cancelled (for example, while the TUI is shutting down).
pub(crate) async fn fetch_latest_async() -> Option<String> {
    if let Some(version) = fetch_latest_from_redirect_async().await {
        return Some(version);
    }
    fetch_latest_from_api_async().await
}

async fn fetch_latest_from_redirect_async() -> Option<String> {
    let out = cancellable_curl_output(LATEST_RELEASE_REDIRECT_ARGS).await?;
    if !out.status.success() {
        return None;
    }
    version_from_release_url(&String::from_utf8_lossy(&out.stdout))
}

async fn fetch_latest_from_api_async() -> Option<String> {
    let out = cancellable_curl_output(LATEST_RELEASE_API_ARGS).await?;
    if !out.status.success() {
        return None;
    }
    version_from_api_response(&out.stdout)
}

async fn cancellable_curl_output(args: &[&str]) -> Option<std::process::Output> {
    let mut command = tokio::process::Command::new("curl");
    command.args(args);
    cancellable_command_output(command).await
}

async fn cancellable_command_output(
    mut command: tokio::process::Command,
) -> Option<std::process::Output> {
    command.kill_on_drop(true);
    command.output().await.ok()
}

/// Extract `X.Y.Z` from a `…/releases/tag/vX.Y.Z` URL.
fn version_from_release_url(url: &str) -> Option<String> {
    url.trim()
        .rsplit_once("/tag/")
        .map(|(_, v)| {
            v.trim()
                .split(['?', '#'])
                .next()
                .unwrap_or(v)
                .trim_start_matches('v')
                .to_string()
        })
        .filter(|v| numeric_version_parts(v).len() >= 2)
}

fn version_from_api_response(bytes: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()?
        .get("tag_name")?
        .as_str()
        .map(|s| {
            s.trim()
                .trim_start_matches('v')
                .split(['-', '+'])
                .next()
                .unwrap_or(s)
                .to_string()
        })
        .filter(|v| numeric_version_parts(v).len() >= 2)
}

fn version_from_output(text: &str) -> Option<String> {
    for token in text.split(|c: char| {
        !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+' || c == 'v')
    }) {
        let token = token.trim().trim_start_matches('v');
        let core = token.split(['-', '+']).next().unwrap_or(token);
        if numeric_version_parts(core).len() >= 2 {
            return Some(core.to_string());
        }
    }
    None
}

/// Version reported by the running executable. Falls back to the package
/// version only if the self-probe fails.
pub(crate) fn current_version() -> String {
    let runner = RealCommandRunner;
    std::env::current_exe()
        .ok()
        .and_then(|exe| binary_version(&runner, exe.as_os_str()))
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// GitHub release target triple for this platform, or `None` if unsupported
/// (e.g. Windows) — those fall back to a manual download.
pub(crate) fn release_target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        _ => return None,
    })
}

/// Whether an in-place self-update is possible on this platform.
pub(crate) fn can_self_update() -> bool {
    release_target().is_some()
}

fn brew_manages_formula(runner: &impl CommandRunner, formula: &str) -> bool {
    runner
        .output(OsStr::new("brew"), &args(&["list", "--versions", formula]))
        .map(|o| o.success && !o.stdout.is_empty())
        .unwrap_or(false)
}

fn managed_brew_formula(runner: &impl CommandRunner) -> Option<&'static str> {
    if brew_manages_formula(runner, BREW_SHORT_FORMULA)
        || brew_manages_formula(runner, BREW_FORMULA)
    {
        Some(BREW_FORMULA)
    } else {
        None
    }
}

fn brew_has_version(runner: &impl CommandRunner, formula: &str, v: &str) -> bool {
    runner
        .output(OsStr::new("brew"), &args(&["list", "--versions", formula]))
        .map(|o| o.success && String::from_utf8_lossy(&o.stdout).contains(v))
        .unwrap_or(false)
}

fn brew_prefix_bin(runner: &impl CommandRunner, formula: &str) -> Option<PathBuf> {
    let out = runner.output(OsStr::new("brew"), &args(&["--prefix", formula]))?;
    if !out.success {
        return None;
    }
    let prefix = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!prefix.is_empty()).then(|| PathBuf::from(prefix).join("bin").join("a3s"))
}

fn binary_version(runner: &impl CommandRunner, bin: impl AsRef<OsStr>) -> Option<String> {
    let out = runner.output(bin.as_ref(), &[OsString::from("--version")])?;
    if !out.success {
        return None;
    }
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    version_from_output(&text)
}

fn verify_binary_version(
    runner: &impl CommandRunner,
    bin: impl AsRef<OsStr>,
    latest: &str,
) -> Option<String> {
    let version = binary_version(runner, bin)?;
    version_ge(&version, latest).then_some(version)
}

fn verify_brew_binary(
    runner: &impl CommandRunner,
    formula: &str,
    current_exe: &Path,
    latest: &str,
) -> Option<PathBuf> {
    let path_bin = PathBuf::from("a3s");
    if verify_binary_version(runner, path_bin.as_os_str(), latest).is_some() {
        return Some(path_bin);
    }
    let prefix_bin = brew_prefix_bin(runner, formula)?;
    verify_binary_version(runner, prefix_bin.as_os_str(), latest)?;

    eprintln!("\n⚠  Homebrew has a3s {latest}, but `a3s` on PATH is still older — relinking…");
    let _ = runner.status(OsStr::new("brew"), &args(&["link", "--overwrite", formula]));
    if verify_binary_version(runner, path_bin.as_os_str(), latest).is_some() {
        return Some(path_bin);
    }

    if current_exe != prefix_bin {
        eprintln!(
            "⚠  Homebrew link is still shadowed — repairing {} from {}…",
            current_exe.display(),
            prefix_bin.display()
        );
        if swap_binary_and_verify(runner, &prefix_bin, current_exe, latest).is_ok() {
            if verify_binary_version(runner, path_bin.as_os_str(), latest).is_some() {
                return Some(path_bin);
            }
            return Some(current_exe.to_path_buf());
        }
    }

    eprintln!(
        "⚠  Homebrew binary is current at {}, but the active `a3s` command is still shadowed",
        prefix_bin.display()
    );
    None
}

fn sibling_webview_helper(current_exe: &Path) -> Option<PathBuf> {
    let sibling = current_exe.parent()?.join(WEBVIEW_BINARY);
    sibling.is_file().then_some(sibling)
}

fn path_webview_helper(runner: &impl CommandRunner) -> Option<PathBuf> {
    runner
        .output(OsStr::new(WEBVIEW_BINARY), &[OsString::from("--help")])
        .filter(|out| out.success)
        .map(|_| PathBuf::from(WEBVIEW_BINARY))
}

fn webview_helper_path(runner: &impl CommandRunner, current_exe: &Path) -> Option<PathBuf> {
    sibling_webview_helper(current_exe).or_else(|| path_webview_helper(runner))
}

fn ensure_remoteui_helper_with(
    runner: &impl CommandRunner,
    current_exe: &Path,
    macos: bool,
) -> Result<Option<PathBuf>, String> {
    if !macos {
        return Ok(None);
    }
    if let Some(path) = webview_helper_path(runner, current_exe) {
        return Ok(Some(path));
    }

    let _ = runner.status(OsStr::new("brew"), &args(&["tap", BREW_TAP, BREW_TAP_URL]));
    let installed = runner.status(OsStr::new("brew"), &args(&["install", WEBVIEW_FORMULA]));
    if let Some(path) = webview_helper_path(runner, current_exe) {
        return Ok(Some(path));
    }
    if installed {
        Err("Homebrew installed a3s-webview, but the helper is still not on PATH".to_string())
    } else {
        Err("a3s-webview is missing and Homebrew could not install it".to_string())
    }
}

fn ensure_remoteui_helper_best_effort(runner: &impl CommandRunner, current_exe: &Path) {
    if let Err(error) = ensure_remoteui_helper_with(runner, current_exe, cfg!(target_os = "macos"))
    {
        eprintln!("\n⚠  RemoteUI helper repair skipped: {error}");
    }
}

/// Repair install-time companion tools. Today this means the macOS RemoteUI
/// helper, which old Homebrew installs did not depend on.
pub(crate) fn repair_installation() -> Result<Vec<String>, String> {
    let runner = RealCommandRunner;
    let exe =
        std::env::current_exe().map_err(|e| format!("could not locate current binary: {e}"))?;
    let mut repaired = Vec::new();
    if let Some(formula) = managed_brew_formula(&runner) {
        let current = current_version();
        if let Some(bin) = verify_brew_binary(&runner, formula, &exe, &current) {
            repaired.push(format!("Homebrew command ready: {}", bin.display()));
        }
    }
    if let Some(path) = ensure_remoteui_helper_with(&runner, &exe, cfg!(target_os = "macos"))? {
        repaired.push(format!("RemoteUI helper ready: {}", path.display()));
    }
    Ok(repaired)
}

/// Upgrade to `latest` in place. Returns the binary to exec on success —
/// Homebrew repoints `a3s` on PATH (exec by name); a direct download swaps
/// `current_exe` (exec that path) — or an error explaining why every path failed.
///
/// Run after the TUI has exited (terminal restored) so child stdio shows real
/// download/upgrade progress.
pub(crate) fn perform_upgrade(latest: &str) -> Result<PathBuf, String> {
    let runner = RealCommandRunner;
    let exe =
        std::env::current_exe().map_err(|e| format!("could not locate current binary: {e}"))?;
    perform_upgrade_with(latest, &runner, exe)
}

fn perform_upgrade_with(
    latest: &str,
    runner: &impl CommandRunner,
    current_exe: PathBuf,
) -> Result<PathBuf, String> {
    if latest.trim().is_empty() {
        return Err("latest version is empty".to_string());
    }

    let mut failures = Vec::new();
    if let Some(formula) = managed_brew_formula(runner) {
        // `brew upgrade` reads a *cached* formula — refresh the tap first, else
        // it no-ops with "already installed". Prefer a fast targeted git pull,
        // fall back to a full `brew update`.
        let _ = runner.status(OsStr::new("brew"), &args(&["tap", BREW_TAP, BREW_TAP_URL]));
        let tap = runner
            .output(OsStr::new("brew"), &args(&["--repo", BREW_TAP]))
            .filter(|o| o.success)
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());
        let pulled = tap
            .as_ref()
            .map(|r| {
                runner.status(
                    OsStr::new("git"),
                    &[
                        OsString::from("-C"),
                        OsString::from(r),
                        OsString::from("pull"),
                        OsString::from("--quiet"),
                        OsString::from("--ff-only"),
                    ],
                )
            })
            .unwrap_or(false);
        if !pulled {
            let _ = runner.status(OsStr::new("brew"), &args(&["update"]));
        }
        println!("\n⬇  upgrading a3s {latest} via Homebrew…\n");
        let upgrade_ok = runner.status(OsStr::new("brew"), &args(&["upgrade", formula]));
        if let Some(bin) = verify_brew_binary(runner, formula, &current_exe, latest) {
            ensure_remoteui_helper_best_effort(runner, &current_exe);
            return Ok(bin);
        }

        // Homebrew metadata can claim the new version while PATH still runs an
        // older binary (stale link, failed pour, or partial tap refresh). Reinstall
        // once before falling back to the standalone updater.
        let metadata_has_latest = brew_has_version(runner, formula, latest);
        let reason = if metadata_has_latest {
            format!("Homebrew metadata says {latest}, but `a3s --version` did not")
        } else if upgrade_ok {
            "Homebrew upgrade finished, but the installed binary is still old".to_string()
        } else {
            "Homebrew upgrade failed".to_string()
        };
        eprintln!("\n⚠  {reason} — reinstalling…");
        let _ = runner.status(OsStr::new("brew"), &args(&["reinstall", formula]));
        if let Some(bin) = verify_brew_binary(runner, formula, &current_exe, latest) {
            ensure_remoteui_helper_best_effort(runner, &current_exe);
            return Ok(bin);
        }

        let failure = format!("Homebrew formula {formula} did not install a3s {latest}");
        failures.push(failure);
        eprintln!("\n⚠  Homebrew didn't install a3s {latest} — falling back to a direct download…");
    }
    let result = standalone_upgrade_with(latest, runner, current_exe).map_err(|e| {
        failures.push(e);
        failures.join("; ")
    });
    if let Ok(bin) = &result {
        ensure_remoteui_helper_best_effort(runner, bin);
    }
    result
}

fn standalone_upgrade_with(
    latest: &str,
    runner: &impl CommandRunner,
    exe: PathBuf,
) -> Result<PathBuf, String> {
    let target = release_target().ok_or_else(|| {
        format!(
            "automatic self-update is not supported on {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let url = format!(
        "https://github.com/A3S-Lab/Cli/releases/download/v{latest}/a3s-v{latest}-{target}.tar.gz"
    );
    let asset_name = format!("a3s-v{latest}-{target}.tar.gz");
    let digest = release_asset_digest(runner, latest, &asset_name)?;
    let tmp = unique_update_dir();
    if std::fs::create_dir_all(&tmp).is_err() {
        return Err(format!(
            "could not create temporary directory {}",
            tmp.display()
        ));
    }
    println!("\n⬇  downloading a3s {latest}…\n");
    let download = runner.output(
        OsStr::new("curl"),
        &[
            OsString::from("-fL"),
            OsString::from("--silent"),
            OsString::from("--show-error"),
            OsString::from("--retry"),
            OsString::from("3"),
            OsString::from("--connect-timeout"),
            OsString::from("10"),
            OsString::from("--max-time"),
            OsString::from("180"),
            OsString::from("--max-filesize"),
            OsString::from(MAX_SELF_UPDATE_ARCHIVE_BYTES.to_string()),
            OsString::from("--proto"),
            OsString::from("=https"),
            OsString::from("--proto-redir"),
            OsString::from("=https"),
            OsString::from(&url),
        ],
    );
    let Some(download) = download else {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("download failed: {url}"));
    };
    if !download.success {
        let _ = std::fs::remove_dir_all(&tmp);
        let detail = String::from_utf8_lossy(&download.stderr);
        return Err(format!("download failed: {url}: {}", detail.trim()));
    }
    if download.stdout.len() > MAX_SELF_UPDATE_ARCHIVE_BYTES {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!(
            "downloaded archive exceeds the {} byte limit",
            MAX_SELF_UPDATE_ARCHIVE_BYTES
        ));
    }
    verify_sha256(&download.stdout, &digest).map_err(|error| {
        let _ = std::fs::remove_dir_all(&tmp);
        format!("self-update checksum verification failed: {error:#}")
    })?;
    extract_tar_gz_archive(&download.stdout, &tmp).map_err(|error| {
        let _ = std::fs::remove_dir_all(&tmp);
        format!("failed to safely extract release archive: {error:#}")
    })?;
    let new_bin = find_downloaded_binary(&tmp);
    if new_bin.is_none() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("release archive did not contain an a3s binary".to_string());
    }
    let new_bin = new_bin.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new_bin, std::fs::Permissions::from_mode(0o755));
    }
    if verify_binary_version(runner, new_bin.as_os_str(), latest).is_none() {
        eprintln!("\n✗ downloaded a3s did not report version {latest}");
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!(
            "downloaded binary {} did not report version {latest}",
            new_bin.display()
        ));
    }
    let result = match swap_binary_and_verify(runner, &new_bin, &exe, latest) {
        Ok(()) => Ok(exe),
        Err(err) => {
            eprintln!("\n✗ failed to install downloaded a3s: {err}");
            Err(err)
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn release_asset_digest(
    runner: &impl CommandRunner,
    latest: &str,
    asset_name: &str,
) -> Result<String, String> {
    let url = format!(
        "https://api.github.com/repos/A3S-Lab/Cli/releases/tags/v{latest}"
    );
    let output = runner
        .output(
            OsStr::new("curl"),
            &[
                OsString::from("-fsSL"),
                OsString::from("--retry"),
                OsString::from("3"),
                OsString::from("--connect-timeout"),
                OsString::from("10"),
                OsString::from("--max-time"),
                OsString::from("30"),
                OsString::from("--max-filesize"),
                OsString::from((4 * 1024 * 1024).to_string()),
                OsString::from("--proto"),
                OsString::from("=https"),
                OsString::from("--proto-redir"),
                OsString::from("=https"),
                OsString::from("-H"),
                OsString::from("Accept: application/vnd.github+json"),
                OsString::from("-H"),
                OsString::from("User-Agent: a3s-self-update/1.0"),
                OsString::from(&url),
            ],
        )
        .ok_or_else(|| format!("failed to query release metadata from {url}"))?;
    if !output.success {
        return Err(format!(
            "release metadata request failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("release metadata is invalid JSON: {error}"))?;
    let digest = value
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset.get("name").and_then(serde_json::Value::as_str) == Some(asset_name)
            })
        })
        .and_then(|asset| asset.get("digest"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            format!("release asset '{asset_name}' has no GitHub SHA-256 digest; refusing an unverified self-update")
        })?;
    let digest = digest.strip_prefix("sha256:").unwrap_or(digest);
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "release asset '{asset_name}' has an invalid SHA-256 digest"
        ));
    }
    Ok(digest.to_ascii_lowercase())
}

fn find_downloaded_binary(root: &Path) -> Option<PathBuf> {
    let direct = root.join("a3s");
    if direct.is_file() {
        return Some(direct);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .is_some_and(|name| name == OsStr::new("a3s"))
            {
                return Some(path);
            }
        }
    }
    None
}

fn unique_update_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("a3s-update-{}-{nanos}", std::process::id()))
}

fn sibling_temp_path(target: &Path, suffix: &str) -> Option<PathBuf> {
    let dir = target.parent()?;
    let name = target.file_name()?.to_string_lossy();
    Some(dir.join(format!(
        ".{name}.a3s-update-{}.{suffix}",
        std::process::id()
    )))
}

fn copy_executable(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::copy(src, dst).map(|_| ())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn swap_binary_and_verify(
    runner: &impl CommandRunner,
    new_bin: &Path,
    target: &Path,
    latest: &str,
) -> Result<(), String> {
    let staging = sibling_temp_path(target, "new")
        .ok_or_else(|| format!("cannot derive staging path for {}", target.display()))?;
    let backup = sibling_temp_path(target, "bak")
        .ok_or_else(|| format!("cannot derive backup path for {}", target.display()))?;

    let _ = std::fs::remove_file(&staging);
    let _ = std::fs::remove_file(&backup);

    copy_executable(new_bin, &staging)
        .map_err(|e| format!("copy {} to {}: {e}", new_bin.display(), staging.display()))?;

    std::fs::hard_link(target, &backup)
        .or_else(|_| std::fs::copy(target, &backup).map(|_| ()))
        .map_err(|e| format!("backup {} to {}: {e}", target.display(), backup.display()))?;

    if let Err(err) = std::fs::rename(&staging, target) {
        let _ = std::fs::remove_file(&staging);
        let _ = std::fs::remove_file(&backup);
        return Err(format!(
            "rename {} over {}: {err}",
            staging.display(),
            target.display()
        ));
    }

    if verify_binary_version(runner, target.as_os_str(), latest).is_some() {
        let _ = std::fs::remove_file(&backup);
        return Ok(());
    }

    std::fs::rename(&backup, target).map_err(|e| {
        format!(
            "restore {} from {}: {e}",
            target.display(),
            backup.display()
        )
    })?;
    Err(format!(
        "{} did not report version {latest} after replacement",
        target.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[test]
    fn version_ordering() {
        assert!(version_ge("0.5.6", "0.5.5"));
        assert!(version_ge("0.5.5", "0.5.5"));
        assert!(!version_ge("0.5.4", "0.5.5"));
        assert!(version_ge("1.0.0", "0.9.9"));
        assert!(version_ge("v0.5.11", "0.5.9"));
        assert!(version_ge("1.0", "1.0.0"));
        assert!(!version_ge("1.0.0", "1.0.1"));
    }

    #[test]
    fn parse_version_from_redirect() {
        let v = version_from_release_url("https://github.com/A3S-Lab/Cli/releases/tag/v0.5.6");
        assert_eq!(v.as_deref(), Some("0.5.6"));
        let v = version_from_release_url("https://github.com/A3S-Lab/Cli/releases/tag/v1.2.30\n");
        assert_eq!(v.as_deref(), Some("1.2.30"));
        let v = version_from_release_url("https://github.com/A3S-Lab/Cli/releases/tag/1.2.31");
        assert_eq!(v.as_deref(), Some("1.2.31"));
        let v = version_from_release_url(
            "https://github.com/A3S-Lab/Cli/releases/tag/v1.2.32?expanded=true",
        );
        assert_eq!(v.as_deref(), Some("1.2.32"));
        // No redirect to a tag (e.g. the bare releases page) → None, not garbage.
        assert_eq!(
            version_from_release_url("https://github.com/A3S-Lab/Cli/releases"),
            None
        );
    }

    #[test]
    fn parse_version_from_api_json() {
        let json = br#"{"tag_name":"v2.3.4"}"#;
        assert_eq!(version_from_api_response(json).as_deref(), Some("2.3.4"));
        assert_eq!(version_from_api_response(br#"{"name":"v2.3.4"}"#), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_async_output_terminates_the_child() {
        use std::time::Duration;

        let temp = tempfile::tempdir().unwrap();
        let started = temp.path().join("started");
        let finished = temp.path().join("finished");
        let mut command = tokio::process::Command::new("sh");
        command
            .args([
                "-c",
                r#"printf started > "$1"; sleep 1; printf finished > "$2""#,
                "a3s-update-cancellation-test",
            ])
            .arg(&started)
            .arg(&finished);

        let task = tokio::spawn(cancellable_command_output(command));
        tokio::time::timeout(Duration::from_secs(2), async {
            while !started.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("test child did not start");

        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(1_500)).await;

        assert!(
            !finished.exists(),
            "cancelled child continued running after its future was dropped"
        );
    }

    #[test]
    fn parse_version_from_binary_output() {
        assert_eq!(
            version_from_output("a3s 0.5.11\n").as_deref(),
            Some("0.5.11")
        );
        assert_eq!(
            version_from_output("a3s-code v0.6.0 (release)\n").as_deref(),
            Some("0.6.0")
        );
        assert_eq!(version_from_output("not a version\n"), None);
    }

    #[test]
    fn target_is_known_on_this_host() {
        // CI runs on macOS + Linux, both supported.
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(release_target().is_some());
            assert!(can_self_update());
        }
    }

    #[derive(Default)]
    struct FakeRunner {
        commands: Mutex<Vec<String>>,
        version_checks: AtomicUsize,
    }

    impl FakeRunner {
        fn commands(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }

        fn record(&self, program: &OsStr, args: &[OsString]) -> String {
            let mut line = program.to_string_lossy().to_string();
            for arg in args {
                line.push(' ');
                line.push_str(&arg.to_string_lossy());
            }
            self.commands.lock().unwrap().push(line.clone());
            line
        }
    }

    impl CommandRunner for FakeRunner {
        fn output(&self, program: &OsStr, args: &[OsString]) -> Option<CommandOutput> {
            let line = self.record(program, args);
            let stdout = match line.as_str() {
                "brew list --versions a3s" => b"a3s 9.9.9\n".to_vec(),
                "brew --repo a3s-lab/tap" => b"/tmp/a3s-tap\n".to_vec(),
                "a3s --version" => {
                    if self.version_checks.fetch_add(1, Ordering::SeqCst) == 0 {
                        b"a3s 0.1.0\n".to_vec()
                    } else {
                        b"a3s 9.9.9\n".to_vec()
                    }
                }
                _ => return None,
            };
            Some(CommandOutput {
                success: true,
                stdout,
                stderr: Vec::new(),
            })
        }

        fn status(&self, program: &OsStr, args: &[OsString]) -> bool {
            self.record(program, args);
            true
        }
    }

    #[test]
    fn brew_upgrade_reinstalls_when_metadata_is_new_but_binary_is_old() {
        let runner = FakeRunner::default();
        let result = perform_upgrade_with("9.9.9", &runner, PathBuf::from("/unused/a3s"));

        assert_eq!(result.as_deref(), Ok(Path::new("a3s")));
        let commands = runner.commands();
        assert!(commands
            .iter()
            .any(|c| c == "brew tap a3s-lab/tap https://github.com/A3S-Lab/homebrew-tap"));
        assert!(commands.iter().any(|c| c == "brew upgrade a3s-lab/tap/a3s"));
        assert!(commands
            .iter()
            .any(|c| c == "brew reinstall a3s-lab/tap/a3s"));
        assert_eq!(runner.version_checks.load(Ordering::SeqCst), 2);
    }

    struct ShadowedBrewRunner {
        commands: Mutex<Vec<String>>,
        linked: AtomicBool,
        prefix: PathBuf,
    }

    impl ShadowedBrewRunner {
        fn new(prefix: PathBuf) -> Self {
            Self {
                commands: Mutex::new(Vec::new()),
                linked: AtomicBool::new(false),
                prefix,
            }
        }

        fn commands(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }

        fn record(&self, program: &OsStr, args: &[OsString]) -> String {
            let mut line = program.to_string_lossy().to_string();
            for arg in args {
                line.push(' ');
                line.push_str(&arg.to_string_lossy());
            }
            self.commands.lock().unwrap().push(line.clone());
            line
        }

        fn prefix_bin(&self) -> PathBuf {
            self.prefix.join("bin").join("a3s")
        }
    }

    impl CommandRunner for ShadowedBrewRunner {
        fn output(&self, program: &OsStr, args: &[OsString]) -> Option<CommandOutput> {
            let line = self.record(program, args);
            let prefix_line = format!("brew --prefix {BREW_FORMULA}");
            let prefix_bin = self.prefix_bin();
            let stdout = if line == "brew list --versions a3s" {
                b"a3s 9.9.9\n".to_vec()
            } else if line == "brew --repo a3s-lab/tap" {
                b"/tmp/a3s-tap\n".to_vec()
            } else if line == prefix_line {
                format!("{}\n", self.prefix.display()).into_bytes()
            } else if line == "a3s --version" {
                if self.linked.load(Ordering::SeqCst) {
                    b"a3s 9.9.9\n".to_vec()
                } else {
                    b"a3s 0.1.0\n".to_vec()
                }
            } else if program == prefix_bin.as_os_str() && args == [OsString::from("--version")] {
                b"a3s 9.9.9\n".to_vec()
            } else {
                return None;
            };
            Some(CommandOutput {
                success: true,
                stdout,
                stderr: Vec::new(),
            })
        }

        fn status(&self, program: &OsStr, args: &[OsString]) -> bool {
            let line = self.record(program, args);
            if line == format!("brew link --overwrite {BREW_FORMULA}") {
                self.linked.store(true, Ordering::SeqCst);
                return true;
            }
            line == format!("brew tap {BREW_TAP} {BREW_TAP_URL}")
                || line == "git -C /tmp/a3s-tap pull --quiet --ff-only"
                || line == format!("brew upgrade {BREW_FORMULA}")
        }
    }

    #[test]
    fn brew_upgrade_relinks_when_keg_is_latest_but_path_is_old() {
        let runner = ShadowedBrewRunner::new(PathBuf::from("/tmp/a3s-shadowed-prefix"));
        let result = perform_upgrade_with("9.9.9", &runner, PathBuf::from("/unused/a3s"));

        assert_eq!(result.as_deref(), Ok(Path::new("a3s")));
        assert!(runner
            .commands()
            .iter()
            .any(|c| c == &format!("brew link --overwrite {BREW_FORMULA}")));
    }

    #[cfg(unix)]
    struct TempDir {
        root: PathBuf,
    }

    #[cfg(unix)]
    impl TempDir {
        fn new(name: &str) -> Self {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "a3s-update-test-{name}-{}-{id}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.root.join(name)
        }
    }

    #[cfg(unix)]
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, version: &str) {
        use std::os::unix::fs::PermissionsExt;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, format!("#!/bin/sh\nprintf 'a3s {version}\\n'\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    struct LinkFailingBrewRunner {
        commands: Mutex<Vec<String>>,
        prefix: PathBuf,
    }

    #[cfg(unix)]
    impl LinkFailingBrewRunner {
        fn new(prefix: PathBuf) -> Self {
            Self {
                commands: Mutex::new(Vec::new()),
                prefix,
            }
        }

        fn commands(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }

        fn record(&self, program: &OsStr, args: &[OsString]) -> String {
            let mut line = program.to_string_lossy().to_string();
            for arg in args {
                line.push(' ');
                line.push_str(&arg.to_string_lossy());
            }
            self.commands.lock().unwrap().push(line.clone());
            line
        }
    }

    #[cfg(unix)]
    impl CommandRunner for LinkFailingBrewRunner {
        fn output(&self, program: &OsStr, args: &[OsString]) -> Option<CommandOutput> {
            let line = self.record(program, args);
            if line == "brew list --versions a3s" {
                return Some(CommandOutput {
                    success: true,
                    stdout: b"a3s 9.9.9\n".to_vec(),
                    stderr: Vec::new(),
                });
            }
            if line == "brew --repo a3s-lab/tap" {
                return Some(CommandOutput {
                    success: true,
                    stdout: b"/tmp/a3s-tap\n".to_vec(),
                    stderr: Vec::new(),
                });
            }
            if line == format!("brew --prefix {BREW_FORMULA}") {
                return Some(CommandOutput {
                    success: true,
                    stdout: format!("{}\n", self.prefix.display()).into_bytes(),
                    stderr: Vec::new(),
                });
            }
            if line == "a3s --version" {
                return Some(CommandOutput {
                    success: true,
                    stdout: b"a3s 0.1.0\n".to_vec(),
                    stderr: Vec::new(),
                });
            }
            let path = Path::new(program);
            if path.is_absolute() && args == [OsString::from("--version")] {
                let out = Command::new(path).arg("--version").output().ok()?;
                return Some(CommandOutput {
                    success: out.status.success(),
                    stdout: out.stdout,
                    stderr: out.stderr,
                });
            }
            None
        }

        fn status(&self, program: &OsStr, args: &[OsString]) -> bool {
            let line = self.record(program, args);
            line == format!("brew tap {BREW_TAP} {BREW_TAP_URL}")
                || line == "git -C /tmp/a3s-tap pull --quiet --ff-only"
                || line == format!("brew upgrade {BREW_FORMULA}")
        }
    }

    #[test]
    #[cfg(unix)]
    fn brew_upgrade_repairs_current_exe_when_link_stays_shadowed() {
        let tmp = TempDir::new("brew-shadowed-current");
        let current_exe = tmp.path("shadowed-a3s");
        let prefix = tmp.path("prefix");
        let prefix_bin = prefix.join("bin").join("a3s");
        write_executable(&current_exe, "0.1.0");
        write_executable(&prefix_bin, "9.9.9");
        let runner = LinkFailingBrewRunner::new(prefix);

        let result = perform_upgrade_with("9.9.9", &runner, current_exe.clone());

        assert_eq!(result.as_deref(), Ok(current_exe.as_path()));
        let out = Command::new(&current_exe)
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a3s 9.9.9\n");
        assert!(runner
            .commands()
            .iter()
            .any(|c| c == &format!("brew link --overwrite {BREW_FORMULA}")));
    }

    #[derive(Default)]
    struct HelperRunner {
        commands: Mutex<Vec<String>>,
        helper_available: AtomicBool,
    }

    impl HelperRunner {
        fn with_helper_available() -> Self {
            Self {
                helper_available: AtomicBool::new(true),
                ..Self::default()
            }
        }

        fn commands(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }

        fn record(&self, program: &OsStr, args: &[OsString]) -> String {
            let mut line = program.to_string_lossy().to_string();
            for arg in args {
                line.push(' ');
                line.push_str(&arg.to_string_lossy());
            }
            self.commands.lock().unwrap().push(line.clone());
            line
        }
    }

    impl CommandRunner for HelperRunner {
        fn output(&self, program: &OsStr, args: &[OsString]) -> Option<CommandOutput> {
            let line = self.record(program, args);
            if line == format!("{WEBVIEW_BINARY} --help") {
                return Some(CommandOutput {
                    success: self.helper_available.load(Ordering::SeqCst),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                });
            }
            None
        }

        fn status(&self, program: &OsStr, args: &[OsString]) -> bool {
            let line = self.record(program, args);
            if line == format!("brew install {WEBVIEW_FORMULA}") {
                self.helper_available.store(true, Ordering::SeqCst);
                return true;
            }
            line == format!("brew tap {BREW_TAP} {BREW_TAP_URL}")
        }
    }

    #[test]
    #[cfg(unix)]
    fn remoteui_helper_uses_existing_path_helper_without_brew() {
        let tmp = TempDir::new("helper-path");
        let runner = HelperRunner::with_helper_available();

        let result = ensure_remoteui_helper_with(&runner, &tmp.path("a3s"), true).unwrap();

        assert_eq!(result.as_deref(), Some(Path::new(WEBVIEW_BINARY)));
        assert_eq!(runner.commands(), vec![format!("{WEBVIEW_BINARY} --help")]);
    }

    #[test]
    #[cfg(unix)]
    fn remoteui_helper_installs_missing_homebrew_helper() {
        let tmp = TempDir::new("helper-install");
        let runner = HelperRunner::default();

        let result = ensure_remoteui_helper_with(&runner, &tmp.path("a3s"), true).unwrap();

        assert_eq!(result.as_deref(), Some(Path::new(WEBVIEW_BINARY)));
        let commands = runner.commands();
        assert!(commands
            .iter()
            .any(|c| c == &format!("brew tap {BREW_TAP} {BREW_TAP_URL}")));
        assert!(commands
            .iter()
            .any(|c| c == &format!("brew install {WEBVIEW_FORMULA}")));
        assert_eq!(
            commands
                .iter()
                .filter(|c| c.as_str() == format!("{WEBVIEW_BINARY} --help"))
                .count(),
            2
        );
    }

    #[test]
    #[cfg(unix)]
    fn standalone_swap_replaces_target_and_verifies_new_version() {
        let tmp = TempDir::new("swap-success");
        let target = tmp.path("a3s");
        let new_bin = tmp.path("downloaded-a3s");
        write_executable(&target, "0.1.0");
        write_executable(&new_bin, "9.9.9");

        let runner = RealCommandRunner;
        swap_binary_and_verify(&runner, &new_bin, &target, "9.9.9").unwrap();

        let out = Command::new(&target).arg("--version").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a3s 9.9.9\n");
        assert!(!sibling_temp_path(&target, "new").unwrap().exists());
        assert!(!sibling_temp_path(&target, "bak").unwrap().exists());
    }

    #[test]
    #[cfg(unix)]
    fn standalone_swap_restores_target_when_new_binary_reports_wrong_version() {
        let tmp = TempDir::new("swap-restore");
        let target = tmp.path("a3s");
        let new_bin = tmp.path("downloaded-a3s");
        write_executable(&target, "0.1.0");
        write_executable(&new_bin, "0.2.0");

        let runner = RealCommandRunner;
        let err = swap_binary_and_verify(&runner, &new_bin, &target, "9.9.9").unwrap_err();

        assert!(err.contains("did not report version 9.9.9"));
        let out = Command::new(&target).arg("--version").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a3s 0.1.0\n");
    }

    #[cfg(unix)]
    struct FakeStandaloneRunner {
        commands: Mutex<Vec<String>>,
        archive: Vec<u8>,
        digest: Option<String>,
    }

    #[cfg(unix)]
    impl FakeStandaloneRunner {
        fn new(binary_path: &str) -> Self {
            let archive = standalone_archive(binary_path, "9.9.9");
            let digest = a3s_updater::sha256_hex(&archive);
            Self {
                commands: Mutex::new(Vec::new()),
                archive,
                digest: Some(digest),
            }
        }

        fn commands(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }

        fn record(&self, program: &OsStr, args: &[OsString]) -> String {
            let mut line = program.to_string_lossy().to_string();
            for arg in args {
                line.push(' ');
                line.push_str(&arg.to_string_lossy());
            }
            self.commands.lock().unwrap().push(line.clone());
            line
        }
    }

    #[cfg(unix)]
    impl CommandRunner for FakeStandaloneRunner {
        fn output(&self, program: &OsStr, args: &[OsString]) -> Option<CommandOutput> {
            let line = self.record(program, args);
            if line == "brew list --versions a3s" || line == "brew list --versions a3s-lab/tap/a3s"
            {
                return Some(CommandOutput {
                    success: false,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                });
            }
            if program == OsStr::new("curl") {
                let url = args.last()?.to_string_lossy();
                if url.contains("api.github.com/repos/A3S-Lab/Cli/releases/tags/v9.9.9") {
                    let target = release_target()?;
                    let metadata = serde_json::to_vec(&serde_json::json!({
                        "tag_name": "v9.9.9",
                        "assets": [{
                            "name": format!("a3s-v9.9.9-{target}.tar.gz"),
                            "digest": self.digest,
                        }]
                    }))
                    .ok()?;
                    return Some(CommandOutput {
                        success: true,
                        stdout: metadata,
                        stderr: Vec::new(),
                    });
                }
                if url.contains("github.com/A3S-Lab/Cli/releases/download/v9.9.9/") {
                    return Some(CommandOutput {
                        success: true,
                        stdout: self.archive.clone(),
                        stderr: Vec::new(),
                    });
                }
            }
            RealCommandRunner.output(program, args)
        }

        fn status(&self, program: &OsStr, args: &[OsString]) -> bool {
            self.record(program, args);
            false
        }
    }

    #[cfg(unix)]
    fn standalone_archive(binary_path: &str, version: &str) -> Vec<u8> {
        let tmp = TempDir::new("standalone-archive");
        let root = tmp.path("root");
        let archive = tmp.path("release.tar.gz");
        write_executable(&root.join(binary_path), version);
        let top_level = Path::new(binary_path)
            .components()
            .next()
            .unwrap()
            .as_os_str();
        let status = Command::new("tar")
            .arg("czf")
            .arg(&archive)
            .arg("-C")
            .arg(&root)
            .arg(top_level)
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::read(archive).unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn standalone_upgrade_fallback_downloads_installs_and_verifies() {
        let Some(target) = release_target() else {
            return;
        };

        let tmp = TempDir::new("standalone-upgrade");
        let current = tmp.path("a3s");
        write_executable(&current, "0.1.0");

        let runner = FakeStandaloneRunner::new("a3s");
        let result = standalone_upgrade_with("9.9.9", &runner, current.clone());

        assert_eq!(result.as_deref(), Ok(current.as_path()));
        let out = Command::new(&current).arg("--version").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a3s 9.9.9\n");

        let commands = runner.commands();
        assert!(commands
            .iter()
            .any(|c| c.contains(&format!("a3s-v9.9.9-{target}.tar.gz"))));
        assert!(commands.iter().any(|c| c.contains("api.github.com/repos/A3S-Lab/Cli/releases/tags/v9.9.9")));
        assert!(!commands.iter().any(|c| c.starts_with("tar ")));
    }

    #[test]
    #[cfg(unix)]
    fn standalone_upgrade_accepts_nested_archive_binary() {
        let tmp = TempDir::new("standalone-nested");
        let current = tmp.path("a3s");
        write_executable(&current, "0.1.0");

        let runner = FakeStandaloneRunner::new("pkg/bin/a3s");
        let result = standalone_upgrade_with("9.9.9", &runner, current.clone());

        assert_eq!(result.as_deref(), Ok(current.as_path()));
        let out = Command::new(&current).arg("--version").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a3s 9.9.9\n");
    }

    #[test]
    #[cfg(unix)]
    fn standalone_upgrade_rejects_a_checksum_mismatch_without_replacing_the_binary() {
        let tmp = TempDir::new("standalone-checksum-mismatch");
        let current = tmp.path("a3s");
        write_executable(&current, "0.1.0");
        let mut runner = FakeStandaloneRunner::new("a3s");
        runner.digest = Some("0".repeat(64));

        let error = standalone_upgrade_with("9.9.9", &runner, current.clone()).unwrap_err();

        assert!(error.contains("checksum verification failed"), "{error}");
        let out = Command::new(&current).arg("--version").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a3s 0.1.0\n");
    }

    #[test]
    #[cfg(unix)]
    fn standalone_upgrade_refuses_release_metadata_without_a_digest() {
        let tmp = TempDir::new("standalone-missing-digest");
        let current = tmp.path("a3s");
        write_executable(&current, "0.1.0");
        let mut runner = FakeStandaloneRunner::new("a3s");
        runner.digest = None;

        let error = standalone_upgrade_with("9.9.9", &runner, current.clone()).unwrap_err();

        assert!(error.contains("no GitHub SHA-256 digest"), "{error}");
        let commands = runner.commands();
        assert_eq!(
            commands
                .iter()
                .filter(|command| command.starts_with("curl "))
                .count(),
            1,
            "the archive must not download before metadata is trusted: {commands:?}"
        );
    }
}
