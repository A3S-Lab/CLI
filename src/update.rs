//! Self-update, shared by the TUI `/update` and the `a3s update` CLI command.
//!
//! Tries Homebrew (how a3s is usually installed) and **falls back to a direct
//! binary download** if brew or the tap is in any bad state — so an update can
//! never be blocked again by a stale tap clone or a broken `brew upgrade`.

use std::path::PathBuf;
use std::process::Command;

/// `[0,2,6] >= [0,2,5]` — `Vec<u32>` compares lexicographically = semver order.
pub(crate) fn version_ge(a: &str, b: &str) -> bool {
    let parse = |s: &str| {
        s.split('.')
            .filter_map(|x| x.parse::<u32>().ok())
            .collect::<Vec<_>>()
    };
    parse(a) >= parse(b)
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
        .rsplit_once("/tag/v")
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
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

fn brew_manages_a3s() -> bool {
    Command::new("brew")
        .args(["list", "--versions", "a3s"])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false)
}

fn brew_has_version(v: &str) -> bool {
    Command::new("brew")
        .args(["list", "--versions", "a3s"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(v))
        .unwrap_or(false)
}

/// Upgrade to `latest` in place. Returns the binary to exec on success —
/// Homebrew repoints `a3s` on PATH (exec by name); a direct download swaps
/// `current_exe` (exec that path) — or `None` if every path failed.
///
/// Run after the TUI has exited (terminal restored) so child stdio shows real
/// download/upgrade progress.
pub(crate) fn perform_upgrade(latest: &str) -> Option<PathBuf> {
    if brew_manages_a3s() {
        // `brew upgrade` reads a *cached* formula — refresh the tap first, else
        // it no-ops with "already installed". Prefer a fast targeted git pull,
        // fall back to a full `brew update`.
        let tap = Command::new("brew")
            .args(["--repo", "a3s-lab/tap"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty());
        let pulled = tap
            .as_ref()
            .map(|r| {
                Command::new("git")
                    .args(["-C", r, "pull", "--quiet", "--ff-only"])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !pulled {
            let _ = Command::new("brew").arg("update").status();
        }
        println!("\n⬇  upgrading a3s {latest} via Homebrew…\n");
        let _ = Command::new("brew").args(["upgrade", "a3s"]).status();
        // Verify (brew exits 0 on a no-op). If it didn't take, don't give up —
        // fall through to a direct binary download.
        if brew_has_version(latest) {
            return Some(PathBuf::from("a3s"));
        }
        eprintln!("\n⚠  Homebrew didn't pick up {latest} — falling back to a direct download…");
    }
    standalone_upgrade(latest)
}

/// Download the release tarball for this platform and swap it over the running
/// binary (works for curl/manual installs, and as the brew fallback).
fn standalone_upgrade(latest: &str) -> Option<PathBuf> {
    let target = release_target()?;
    let exe = std::env::current_exe().ok()?;
    let url = format!(
        "https://github.com/A3S-Lab/Cli/releases/download/v{latest}/a3s-v{latest}-{target}.tar.gz"
    );
    let tmp = std::env::temp_dir().join(format!("a3s-update-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let tarball = tmp.join("a3s.tar.gz");
    println!("\n⬇  downloading a3s {latest}…\n");
    let dl = Command::new("curl")
        .args(["-fL", "--progress-bar", "-o"])
        .arg(&tarball)
        .arg(&url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !dl {
        return None;
    }
    let extracted = Command::new("tar")
        .arg("xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&tmp)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let new_bin = tmp.join("a3s");
    if !extracted || !new_bin.exists() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new_bin, std::fs::Permissions::from_mode(0o755));
    }
    // Rename works over a running binary on unix; fall back to copy across FS.
    std::fs::rename(&new_bin, &exe)
        .or_else(|_| std::fs::copy(&new_bin, &exe).map(|_| ()))
        .is_ok()
        .then_some(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(version_ge("0.5.6", "0.5.5"));
        assert!(version_ge("0.5.5", "0.5.5"));
        assert!(!version_ge("0.5.4", "0.5.5"));
        assert!(version_ge("1.0.0", "0.9.9"));
    }

    #[test]
    fn parse_version_from_redirect() {
        let v = version_from_release_url("https://github.com/A3S-Lab/Cli/releases/tag/v0.5.6");
        assert_eq!(v.as_deref(), Some("0.5.6"));
        let v = version_from_release_url("https://github.com/A3S-Lab/Cli/releases/tag/v1.2.30\n");
        assert_eq!(v.as_deref(), Some("1.2.30"));
        // No redirect to a tag (e.g. the bare releases page) → None, not garbage.
        assert_eq!(
            version_from_release_url("https://github.com/A3S-Lab/Cli/releases"),
            None
        );
    }

    #[test]
    fn target_is_known_on_this_host() {
        // CI runs on macOS + Linux, both supported.
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(release_target().is_some());
            assert!(can_self_update());
        }
    }
}
