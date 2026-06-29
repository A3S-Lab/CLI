//! `a3s box` proxy command.
//!
//! Runs `a3s-box ...` when it is available. If it is missing, bootstrap the
//! Box runtime first so `a3s top` and `a3s box ...` share the same happy path.

use std::path::{Path, PathBuf};
use std::process::Command;

const BOX_BINARY: &str = "a3s-box";
const PRIMARY_BOX_RELEASE_BASE: &str = "https://github.com/A3S-Lab/Box";
const BOX_RELEASE_BASES: &[&str] = &[PRIMARY_BOX_RELEASE_BASE, "https://github.com/AI45Lab/Box"];

pub async fn run(args: Vec<String>) -> anyhow::Result<()> {
    let binary = ensure_a3s_box()?;
    let status = Command::new(&binary).args(args).status().map_err(|err| {
        anyhow::anyhow!(
            "failed to run {} at {}: {err}",
            BOX_BINARY,
            binary.display()
        )
    })?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

pub(crate) fn ensure_a3s_box() -> anyhow::Result<PathBuf> {
    if let Some(path) = find_existing_a3s_box() {
        return Ok(path);
    }

    eprintln!("a3s: {BOX_BINARY} is not installed; installing it now...");
    if let Some(path) = install_with_homebrew() {
        return Ok(path);
    }

    install_standalone()
}

fn find_existing_a3s_box() -> Option<PathBuf> {
    preferred_box_paths()
        .into_iter()
        .find_map(|path| executable_path(&path))
        .or_else(|| find_on_path(BOX_BINARY))
        .or_else(|| {
            fallback_box_paths()
                .into_iter()
                .find_map(|path| executable_path(&path))
        })
}

fn preferred_box_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(value) = std::env::var_os("A3S_BOX_INSTALL_DIR") {
        paths.push(PathBuf::from(value).join(BOX_BINARY));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            paths.push(parent.join(BOX_BINARY));
        }
    }

    paths
}

fn fallback_box_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(home) = std::env::var_os("HOME") {
        paths.push(
            PathBuf::from(home)
                .join(".local")
                .join("bin")
                .join(BOX_BINARY),
        );
    }

    paths
}

fn install_with_homebrew() -> Option<PathBuf> {
    find_on_path("brew")?;

    eprintln!("a3s: trying Homebrew formula a3s-lab/tap/a3s-box...");
    let installed = Command::new("brew")
        .args(["install", "a3s-lab/tap/a3s-box"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    if !installed {
        eprintln!("a3s: Homebrew install failed; falling back to direct download");
        return None;
    }

    find_on_path(BOX_BINARY).or_else(|| {
        let output = Command::new("brew")
            .args(["--prefix", "a3s-box"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if prefix.is_empty() {
            return None;
        }
        executable_path(&PathBuf::from(prefix).join("bin").join(BOX_BINARY))
    })
}

fn install_standalone() -> anyhow::Result<PathBuf> {
    let target = box_asset_target().ok_or_else(|| {
        anyhow::anyhow!(
            "automatic a3s-box install is not supported on {}-{}; install it manually from {PRIMARY_BOX_RELEASE_BASE}/releases/latest",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let (release_base, latest) = fetch_latest_box_release().ok_or_else(|| {
        anyhow::anyhow!(
            "could not reach {PRIMARY_BOX_RELEASE_BASE}/releases/latest to install a3s-box"
        )
    })?;
    let url = box_release_url(release_base, &latest, target);
    let bin_dir = install_bin_dir()?;
    let tmp = std::env::temp_dir().join(format!("a3s-box-install-{}", std::process::id()));
    let tarball = tmp.join("a3s-box.tar.gz");

    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    std::fs::create_dir_all(&bin_dir)?;

    eprintln!("a3s: downloading a3s-box {latest} for {target}...");
    let downloaded = Command::new("curl")
        .args(["-fL", "--show-error", "--progress-bar", "-o"])
        .arg(&tarball)
        .arg(&url)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !downloaded {
        return Err(anyhow::anyhow!("failed to download {url}"));
    }

    eprintln!("a3s: extracting a3s-box {latest}...");
    let extracted = Command::new("tar")
        .arg("xzf")
        .arg(&tarball)
        .arg("-C")
        .arg(&tmp)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !extracted {
        return Err(anyhow::anyhow!("failed to extract {}", tarball.display()));
    }

    let package_dir = extracted_box_package_dir(&tmp, &latest, target)?;

    eprintln!("a3s: installing a3s-box into {}...", bin_dir.display());
    for binary in [
        "a3s-box",
        "a3s-box-shim",
        "a3s-box-guest-init",
        "a3s-box-cri",
    ] {
        let src = package_dir.join(binary);
        if src.exists() {
            install_executable(&src, &bin_dir.join(binary))?;
        }
    }

    let extracted_lib = package_dir.join("lib");
    if extracted_lib.is_dir() {
        for lib_dir in standalone_lib_dirs(&bin_dir) {
            std::fs::create_dir_all(&lib_dir)?;
            copy_dir_contents(&extracted_lib, &lib_dir)?;
        }
    }

    let installed = bin_dir.join(BOX_BINARY);
    if !installed.exists() {
        return Err(anyhow::anyhow!(
            "downloaded archive did not contain {BOX_BINARY}"
        ));
    }

    if !path_contains_dir(&bin_dir) {
        eprintln!(
            "a3s: installed {} to {}; add this directory to PATH for direct use",
            BOX_BINARY,
            bin_dir.display()
        );
    } else {
        eprintln!("a3s: installed {} to {}", BOX_BINARY, bin_dir.display());
    }

    let _ = std::fs::remove_dir_all(&tmp);
    Ok(installed)
}

fn extracted_box_package_dir(tmp: &Path, version: &str, target: &str) -> anyhow::Result<PathBuf> {
    let expected = tmp.join(format!("a3s-box-v{version}-{target}"));
    if expected.join(BOX_BINARY).is_file() {
        return Ok(expected);
    }

    if tmp.join(BOX_BINARY).is_file() {
        return Ok(tmp.to_path_buf());
    }

    for entry in std::fs::read_dir(tmp)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join(BOX_BINARY).is_file() {
            return Ok(path);
        }
    }

    Err(anyhow::anyhow!(
        "downloaded archive did not contain {BOX_BINARY}"
    ))
}

fn install_executable(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::copy(src, dest)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn copy_dir_contents(src: &Path, dest: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if src_path.is_dir() {
            std::fs::create_dir_all(&dest_path)?;
            copy_dir_contents(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)?;
        }
    }
    Ok(())
}

fn fetch_latest_box_release() -> Option<(&'static str, String)> {
    BOX_RELEASE_BASES.iter().find_map(|release_base| {
        fetch_latest_box_version(release_base).map(|version| (*release_base, version))
    })
}

fn fetch_latest_box_version(release_base: &str) -> Option<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "-o",
            "/dev/null",
            "-w",
            "%{url_effective}",
            &format!("{release_base}/releases/latest"),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    version_from_release_url(&String::from_utf8_lossy(&out.stdout))
}

fn version_from_release_url(url: &str) -> Option<String> {
    url.trim()
        .rsplit_once("/tag/v")
        .map(|(_, version)| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

fn box_release_url(release_base: &str, version: &str, target: &str) -> String {
    format!("{release_base}/releases/download/v{version}/a3s-box-v{version}-{target}.tar.gz")
}

fn box_asset_target() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x86_64",
        _ => return None,
    })
}

fn install_bin_dir() -> anyhow::Result<PathBuf> {
    if let Some(value) = std::env::var_os("A3S_BOX_INSTALL_DIR") {
        return Ok(PathBuf::from(value));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if dir_is_writable(parent) {
                return Ok(parent.to_path_buf());
            }
        }
    }

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set; set A3S_BOX_INSTALL_DIR"))?;
    Ok(home.join(".local").join("bin"))
}

fn install_prefix_for_bin_dir(bin_dir: &Path) -> PathBuf {
    if bin_dir.file_name().and_then(|name| name.to_str()) == Some("bin") {
        bin_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| bin_dir.to_path_buf())
    } else {
        bin_dir.to_path_buf()
    }
}

fn standalone_lib_dirs(bin_dir: &Path) -> Vec<PathBuf> {
    let runtime_lib_dir = bin_dir.join("lib");
    let prefix_lib_dir = install_prefix_for_bin_dir(bin_dir).join("lib");
    if prefix_lib_dir == runtime_lib_dir {
        vec![runtime_lib_dir]
    } else {
        vec![runtime_lib_dir, prefix_lib_dir]
    }
}

fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".a3s-box-install-check-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

fn path_contains_dir(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|path| path == dir))
        .unwrap_or(false)
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
    let path = Path::new(binary);
    if path.components().count() > 1 {
        return executable_path(path);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| executable_path(&dir.join(binary)))
    })
}

fn executable_path(path: &Path) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).ok()?.permissions().mode();
        (mode & 0o111 != 0).then_some(path.to_path_buf())
    }
    #[cfg(not(unix))]
    {
        Some(path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_latest_release_redirect() {
        assert_eq!(
            version_from_release_url("https://github.com/A3S-Lab/Box/releases/tag/v2.5.2"),
            Some("2.5.2".to_string())
        );
        assert_eq!(
            version_from_release_url("https://github.com/A3S-Lab/Box/releases"),
            None
        );
    }

    #[test]
    fn builds_box_release_url() {
        assert_eq!(
            box_release_url(PRIMARY_BOX_RELEASE_BASE, "2.5.2", "linux-x86_64"),
            "https://github.com/A3S-Lab/Box/releases/download/v2.5.2/a3s-box-v2.5.2-linux-x86_64.tar.gz"
        );
    }

    #[test]
    fn target_is_known_on_supported_hosts() {
        if cfg!(all(target_os = "macos", target_arch = "aarch64"))
            || cfg!(all(target_os = "linux", target_arch = "aarch64"))
            || cfg!(all(target_os = "linux", target_arch = "x86_64"))
        {
            assert!(box_asset_target().is_some());
        }
    }

    #[test]
    fn finds_existing_box_in_configured_install_dir() {
        let _guard = env_guard();
        let root =
            std::env::temp_dir().join(format!("a3s-box-install-dir-test-{}", std::process::id()));
        let bin = root.join(BOX_BINARY);
        make_executable(&bin);

        let old_install_dir = std::env::var_os("A3S_BOX_INSTALL_DIR");
        let old_path = std::env::var_os("PATH").unwrap_or_default();
        std::env::set_var("A3S_BOX_INSTALL_DIR", &root);
        std::env::set_var("PATH", "");

        let found = find_existing_a3s_box();

        restore_var("A3S_BOX_INSTALL_DIR", old_install_dir);
        std::env::set_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(found, Some(bin));
    }

    #[test]
    fn configured_install_dir_wins_over_path() {
        let _guard = env_guard();
        let root = std::env::temp_dir().join(format!(
            "a3s-box-install-dir-priority-test-{}",
            std::process::id()
        ));
        let configured = root.join("configured");
        let path_dir = root.join("path");
        let configured_bin = configured.join(BOX_BINARY);
        let path_bin = path_dir.join(BOX_BINARY);
        make_executable(&configured_bin);
        make_executable(&path_bin);

        let old_install_dir = std::env::var_os("A3S_BOX_INSTALL_DIR");
        let old_path = std::env::var_os("PATH").unwrap_or_default();
        std::env::set_var("A3S_BOX_INSTALL_DIR", &configured);
        std::env::set_var("PATH", &path_dir);

        let found = find_existing_a3s_box();

        restore_var("A3S_BOX_INSTALL_DIR", old_install_dir);
        std::env::set_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(found, Some(configured_bin));
    }

    #[test]
    fn install_prefix_for_bin_parent() {
        assert_eq!(
            install_prefix_for_bin_dir(Path::new("/tmp/a3s/bin")),
            PathBuf::from("/tmp/a3s")
        );
        assert_eq!(
            install_prefix_for_bin_dir(Path::new("/tmp/a3s-tools")),
            PathBuf::from("/tmp/a3s-tools")
        );
    }

    #[test]
    fn standalone_lib_dirs_cover_runtime_rpath_and_prefix_layout() {
        assert_eq!(
            standalone_lib_dirs(Path::new("/tmp/a3s/bin")),
            vec![
                PathBuf::from("/tmp/a3s/bin/lib"),
                PathBuf::from("/tmp/a3s/lib")
            ]
        );
        assert_eq!(
            standalone_lib_dirs(Path::new("/tmp/a3s-tools")),
            vec![PathBuf::from("/tmp/a3s-tools/lib")]
        );
    }

    #[test]
    fn finds_nested_box_release_package_dir() {
        let root =
            std::env::temp_dir().join(format!("a3s-box-package-dir-test-{}", std::process::id()));
        let package = root.join("a3s-box-v2.5.2-linux-x86_64");
        make_executable(&package.join(BOX_BINARY));

        let found = extracted_box_package_dir(&root, "2.5.2", "linux-x86_64").unwrap();

        let _ = std::fs::remove_dir_all(root);
        assert_eq!(found, package);
    }

    #[test]
    fn finds_flat_box_release_package_dir() {
        let root =
            std::env::temp_dir().join(format!("a3s-box-flat-dir-test-{}", std::process::id()));
        make_executable(&root.join(BOX_BINARY));

        let found = extracted_box_package_dir(&root, "2.5.2", "linux-x86_64").unwrap();

        assert_eq!(found, root);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finds_executable_on_path() {
        let _guard = env_guard();
        let root = std::env::temp_dir().join(format!("a3s-box-test-{}", std::process::id()));
        let bin = root.join("a3s-box-test-bin");
        make_executable(&bin);

        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let joined = std::env::join_paths(
            std::iter::once(root.clone()).chain(std::env::split_paths(&old_path)),
        )
        .unwrap();
        std::env::set_var("PATH", joined);

        let found = find_on_path("a3s-box-test-bin");

        std::env::set_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(found, Some(bin));
    }

    fn make_executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn restore_var(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }
}
