//! Self-update, shared by the TUI `/update` and the `a3s update` CLI command.
//!
//! Tries Homebrew (how a3s is usually installed) and **falls back to a direct
//! binary download** if brew or the tap is in any bad state — so an update can
//! never be blocked again by a stale tap clone or a broken `brew upgrade`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

struct CommandOutput {
    success: bool,
    stdout: Vec<u8>,
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
/// release server is unreachable. Blocking — call via `spawn_blocking` in async.
///
/// Uses the `releases/latest` REDIRECT on github.com (which 302s to
/// `…/releases/tag/vX.Y.Z`), NOT the `api.github.com` REST endpoint — the API
/// is rate-limited to 60 req/hr/IP unauthenticated, which strands users behind
/// shared IPs / CI; the redirect has no such limit.
pub(crate) fn fetch_latest() -> Option<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "-o",
            "/dev/null",
            "-w",
            "%{url_effective}",
            "https://github.com/A3S-Lab/Cli/releases/latest",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    version_from_release_url(&String::from_utf8_lossy(&out.stdout))
}

/// Extract `X.Y.Z` from a `…/releases/tag/vX.Y.Z` URL.
fn version_from_release_url(url: &str) -> Option<String> {
    url.trim()
        .rsplit_once("/tag/")
        .map(|(_, v)| v.trim().trim_start_matches('v').to_string())
        .filter(|v| !v.is_empty())
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

fn brew_manages_a3s(runner: &impl CommandRunner) -> bool {
    runner
        .output(OsStr::new("brew"), &args(&["list", "--versions", "a3s"]))
        .map(|o| o.success && !o.stdout.is_empty())
        .unwrap_or(false)
}

fn brew_has_version(runner: &impl CommandRunner, v: &str) -> bool {
    runner
        .output(OsStr::new("brew"), &args(&["list", "--versions", "a3s"]))
        .map(|o| o.success && String::from_utf8_lossy(&o.stdout).contains(v))
        .unwrap_or(false)
}

fn verify_binary_version(
    runner: &impl CommandRunner,
    bin: impl AsRef<OsStr>,
    latest: &str,
) -> Option<String> {
    let out = runner.output(bin.as_ref(), &[OsString::from("--version")])?;
    if !out.success {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let version = version_from_output(&text)?;
    version_ge(&version, latest).then_some(version)
}

/// Upgrade to `latest` in place. Returns the binary to exec on success —
/// Homebrew repoints `a3s` on PATH (exec by name); a direct download swaps
/// `current_exe` (exec that path) — or `None` if every path failed.
///
/// Run after the TUI has exited (terminal restored) so child stdio shows real
/// download/upgrade progress.
pub(crate) fn perform_upgrade(latest: &str) -> Option<PathBuf> {
    let runner = RealCommandRunner;
    let exe = std::env::current_exe().ok()?;
    perform_upgrade_with(latest, &runner, exe)
}

fn perform_upgrade_with(
    latest: &str,
    runner: &impl CommandRunner,
    current_exe: PathBuf,
) -> Option<PathBuf> {
    if latest.trim().is_empty() {
        return None;
    }

    if brew_manages_a3s(runner) {
        // `brew upgrade` reads a *cached* formula — refresh the tap first, else
        // it no-ops with "already installed". Prefer a fast targeted git pull,
        // fall back to a full `brew update`.
        let tap = runner
            .output(OsStr::new("brew"), &args(&["--repo", "a3s-lab/tap"]))
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
        let _ = runner.status(OsStr::new("brew"), &args(&["upgrade", "a3s"]));
        let brew_bin = PathBuf::from("a3s");
        if verify_binary_version(runner, brew_bin.as_os_str(), latest).is_some() {
            return Some(PathBuf::from("a3s"));
        }

        // Homebrew metadata can claim the new version while PATH still runs an
        // older binary (stale link, failed pour, or partial tap refresh). Reinstall
        // once before falling back to the standalone updater.
        if brew_has_version(runner, latest) {
            eprintln!(
                "\n⚠  Homebrew metadata says {latest}, but `a3s --version` did not — reinstalling…"
            );
            let _ = runner.status(OsStr::new("brew"), &args(&["reinstall", "a3s"]));
            if verify_binary_version(runner, brew_bin.as_os_str(), latest).is_some() {
                return Some(PathBuf::from("a3s"));
            }
        }

        eprintln!("\n⚠  Homebrew didn't install a3s {latest} — falling back to a direct download…");
    }
    standalone_upgrade_with(latest, runner, current_exe)
}

fn standalone_upgrade_with(
    latest: &str,
    runner: &impl CommandRunner,
    exe: PathBuf,
) -> Option<PathBuf> {
    let target = release_target()?;
    let url = format!(
        "https://github.com/A3S-Lab/Cli/releases/download/v{latest}/a3s-v{latest}-{target}.tar.gz"
    );
    let tmp = unique_update_dir();
    if std::fs::create_dir_all(&tmp).is_err() {
        return None;
    }
    let tarball = tmp.join("a3s.tar.gz");
    println!("\n⬇  downloading a3s {latest}…\n");
    let dl = runner.status(
        OsStr::new("curl"),
        &[
            OsString::from("-fL"),
            OsString::from("--progress-bar"),
            OsString::from("-o"),
            tarball.as_os_str().to_os_string(),
            OsString::from(&url),
        ],
    );
    if !dl {
        let _ = std::fs::remove_dir_all(&tmp);
        return None;
    }
    let extracted = runner.status(
        OsStr::new("tar"),
        &[
            OsString::from("xzf"),
            tarball.as_os_str().to_os_string(),
            OsString::from("-C"),
            tmp.as_os_str().to_os_string(),
        ],
    );
    let new_bin = tmp.join("a3s");
    if !extracted || !new_bin.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new_bin, std::fs::Permissions::from_mode(0o755));
    }
    if verify_binary_version(runner, new_bin.as_os_str(), latest).is_none() {
        eprintln!("\n✗ downloaded a3s did not report version {latest}");
        let _ = std::fs::remove_dir_all(&tmp);
        return None;
    }
    let result = match swap_binary_and_verify(runner, &new_bin, &exe, latest) {
        Ok(()) => Some(exe),
        Err(err) => {
            eprintln!("\n✗ failed to install downloaded a3s: {err}");
            None
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);
    result
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
    use std::sync::atomic::{AtomicUsize, Ordering};
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
        // No redirect to a tag (e.g. the bare releases page) → None, not garbage.
        assert_eq!(
            version_from_release_url("https://github.com/A3S-Lab/Cli/releases"),
            None
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

        assert_eq!(result.as_deref(), Some(Path::new("a3s")));
        let commands = runner.commands();
        assert!(commands.iter().any(|c| c == "brew upgrade a3s"));
        assert!(commands.iter().any(|c| c == "brew reinstall a3s"));
        assert_eq!(runner.version_checks.load(Ordering::SeqCst), 2);
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
    #[derive(Default)]
    struct FakeStandaloneRunner {
        commands: Mutex<Vec<String>>,
    }

    #[cfg(unix)]
    impl FakeStandaloneRunner {
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
            if line == "brew list --versions a3s" {
                return Some(CommandOutput {
                    success: false,
                    stdout: Vec::new(),
                });
            }
            RealCommandRunner.output(program, args)
        }

        fn status(&self, program: &OsStr, args: &[OsString]) -> bool {
            self.record(program, args);
            match program.to_string_lossy().as_ref() {
                "curl" => {
                    let out = args
                        .windows(2)
                        .find(|pair| pair[0] == "-o")
                        .map(|pair| PathBuf::from(&pair[1]));
                    if let Some(out) = out {
                        std::fs::write(out, "fake tarball\n").is_ok()
                    } else {
                        false
                    }
                }
                "tar" => {
                    let dest = args
                        .windows(2)
                        .find(|pair| pair[0] == "-C")
                        .map(|pair| PathBuf::from(&pair[1]));
                    if let Some(dest) = dest {
                        write_executable(&dest.join("a3s"), "9.9.9");
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            }
        }
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

        let runner = FakeStandaloneRunner::default();
        let result = standalone_upgrade_with("9.9.9", &runner, current.clone());

        assert_eq!(result.as_deref(), Some(current.as_path()));
        let out = Command::new(&current).arg("--version").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a3s 9.9.9\n");

        let commands = runner.commands();
        assert!(commands
            .iter()
            .any(|c| c.contains(&format!("a3s-v9.9.9-{target}.tar.gz"))));
        assert!(commands.iter().any(|c| c.starts_with("tar xzf ")));
    }
}
