//! `a3s box` proxy command.
//!
//! Runs `a3s-box ...` when it is available. If it is missing, bootstrap the
//! Box runtime first so `a3s top` and `a3s box ...` share the same happy path.

use std::path::{Path, PathBuf};
use std::process::Command;

const BOX_BINARY: &str = "a3s-box";
const PRIMARY_BOX_RELEASE_BASE: &str = "https://github.com/A3S-Lab/Box";
const BOX_RELEASE_BASES: &[&str] = &[PRIMARY_BOX_RELEASE_BASE];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoxSource {
    ConfiguredInstallDir,
    Sibling,
    Path,
    LegacyUserBin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledBox {
    pub(crate) version: Option<String>,
    pub(crate) path: PathBuf,
    pub(crate) source: BoxSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BoxState {
    Missing,
    Installed(InstalledBox),
    Broken(String),
}

pub async fn run(args: Vec<String>) -> anyhow::Result<()> {
    let binary = ensure_a3s_box()?;
    run_binary(&binary, args)
}

/// Run Box only when it is already present. Used for help/version probes so a
/// read-only question never becomes an implicit installation.
pub(crate) fn run_installed(args: Vec<String>) -> anyhow::Result<bool> {
    let Some(binary) = find_existing_a3s_box() else {
        return Ok(false);
    };
    run_binary(&binary, args)?;
    Ok(true)
}

fn run_binary(binary: &Path, args: Vec<String>) -> anyhow::Result<()> {
    let status = Command::new(binary).args(args).status().map_err(|err| {
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

/// Inspect the locally selected Box executable without installing or fetching
/// anything. Version probing is best-effort so existing third-party wrappers
/// remain usable by `a3s box` even when they do not expose a version command.
pub(crate) fn inspect() -> BoxState {
    inspect_with_version_probe(true)
}

/// Inspect Box for `a3s list` using filesystem metadata only. In particular,
/// do not execute an arbitrary `a3s-box` discovered on PATH just to obtain its
/// version: list must remain bounded and side-effect free.
pub(crate) fn inspect_read_only() -> BoxState {
    inspect_with_version_probe(false)
}

fn inspect_with_version_probe(probe_version: bool) -> BoxState {
    if let Some(located) = locate_existing_a3s_box() {
        return BoxState::Installed(InstalledBox {
            version: probe_version
                .then(|| probe_box_version(&located.path))
                .flatten(),
            path: located.path,
            source: located.source,
        });
    }

    if let Some(configured) = configured_box_path() {
        if configured.exists() {
            return BoxState::Broken(format!(
                "configured a3s-box is not executable: {}",
                configured.display()
            ));
        }
    }

    BoxState::Missing
}

/// Explicit `a3s install box` operation. The operation is idempotent and uses
/// the same bootstrap path as first-use installation.
pub(crate) fn install() -> anyhow::Result<InstalledBox> {
    match inspect() {
        BoxState::Installed(installed) => {
            print_already_installed(&installed);
            Ok(installed)
        }
        BoxState::Broken(error) => {
            eprintln!("a3s: repairing invalid Box component state: {error}");
            let installed_path = bootstrap_a3s_box()?;
            installed_box_at(installed_path)
        }
        BoxState::Missing => {
            eprintln!("a3s: {BOX_BINARY} is not installed; installing it now...");
            let installed_path = bootstrap_a3s_box()?;
            installed_box_at(installed_path)
        }
    }
}

/// Explicit `a3s update box` operation. Updating a missing component is an
/// error rather than an implicit install so scripts can distinguish the two
/// package-manager operations.
pub(crate) fn update() -> anyhow::Result<InstalledBox> {
    let current = match inspect() {
        BoxState::Installed(installed) => installed,
        BoxState::Missing => {
            return Err(anyhow::anyhow!(
                "a3s box is not installed; run `a3s install box` first"
            ));
        }
        BoxState::Broken(error) => {
            return Err(anyhow::anyhow!(
                "Box component state is invalid: {error}; run `a3s install box` to repair it before updating"
            ));
        }
    };

    let target = box_asset_target().ok_or_else(|| {
        anyhow::anyhow!(
            "automatic a3s-box update is not supported on {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let (release_base, latest) = fetch_latest_box_release().ok_or_else(|| {
        anyhow::anyhow!(
            "could not reach {PRIMARY_BOX_RELEASE_BASE}/releases/latest to update a3s-box"
        )
    })?;

    if current
        .version
        .as_deref()
        .is_some_and(|version| crate::update::version_ge(version, &latest))
    {
        println!("✓ a3s Box {latest} is already up to date");
        return Ok(current);
    }

    let installed_path = if homebrew_manages_box(&current.path) {
        update_with_homebrew()?
    } else {
        let bin_dir = direct_update_bin_dir(&current.path)?;
        install_standalone_release(release_base, &latest, target, &bin_dir)?
    };
    let updated = installed_box_at(installed_path)?;
    let updated_version = updated.version.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "updated a3s-box at {} did not report a version",
            updated.path.display()
        )
    })?;
    if !crate::update::version_ge(updated_version, &latest) {
        return Err(anyhow::anyhow!(
            "updated a3s-box at {} reports {updated_version}, expected at least {latest}",
            updated.path.display()
        ));
    }

    println!(
        "✓ updated a3s Box to {} at {}",
        updated_version,
        updated.path.display()
    );
    Ok(updated)
}

pub(crate) fn ensure_a3s_box() -> anyhow::Result<PathBuf> {
    if let Some(path) = find_existing_a3s_box() {
        return Ok(path);
    }

    eprintln!("a3s: {BOX_BINARY} is not installed; installing it now...");
    bootstrap_a3s_box()
}

fn bootstrap_a3s_box() -> anyhow::Result<PathBuf> {
    if let Some(path) = install_with_homebrew() {
        return Ok(path);
    }

    install_standalone()
}

fn find_existing_a3s_box() -> Option<PathBuf> {
    locate_existing_a3s_box().map(|located| located.path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocatedBox {
    path: PathBuf,
    source: BoxSource,
}

fn locate_existing_a3s_box() -> Option<LocatedBox> {
    if let Some(path) = configured_box_path().and_then(|path| executable_path(&path)) {
        return Some(LocatedBox {
            path,
            source: BoxSource::ConfiguredInstallDir,
        });
    }

    if let Some(path) = sibling_box_path().and_then(|path| executable_path(&path)) {
        return Some(LocatedBox {
            path,
            source: BoxSource::Sibling,
        });
    }

    if let Some(path) = find_on_path(BOX_BINARY) {
        return Some(LocatedBox {
            path,
            source: BoxSource::Path,
        });
    }

    fallback_box_paths()
        .into_iter()
        .find_map(|path| executable_path(&path))
        .map(|path| LocatedBox {
            path,
            source: BoxSource::LegacyUserBin,
        })
}

fn configured_box_path() -> Option<PathBuf> {
    std::env::var_os("A3S_BOX_INSTALL_DIR")
        .map(PathBuf::from)
        .map(|dir| dir.join(BOX_BINARY))
}

fn sibling_box_path() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|parent| parent.join(BOX_BINARY))
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

fn homebrew_manages_box(current: &Path) -> bool {
    if find_on_path("brew").is_none() {
        return false;
    }
    let installed = Command::new("brew")
        .args(["list", "--versions", "a3s-box"])
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false);
    installed
        && homebrew_box_binary()
            .as_deref()
            .is_some_and(|brew_binary| paths_refer_to_same_file(current, brew_binary))
}

fn update_with_homebrew() -> anyhow::Result<PathBuf> {
    eprintln!("a3s: updating a3s-box with Homebrew...");
    let _ = Command::new("brew").arg("update").status();
    let upgraded = Command::new("brew")
        .args(["upgrade", "a3s-lab/tap/a3s-box"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !upgraded {
        return Err(anyhow::anyhow!("Homebrew failed to update a3s-box"));
    }

    find_on_path(BOX_BINARY)
        .or_else(homebrew_box_binary)
        .ok_or_else(|| anyhow::anyhow!("Homebrew updated a3s-box but its binary was not found"))
}

fn homebrew_box_binary() -> Option<PathBuf> {
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
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
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
    let bin_dir = install_bin_dir()?;
    install_standalone_release(release_base, &latest, target, &bin_dir)
}

fn install_standalone_release(
    release_base: &str,
    version: &str,
    target: &str,
    bin_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let url = box_release_url(release_base, version, target);
    let tmp = std::env::temp_dir().join(format!("a3s-box-install-{}", std::process::id()));
    let tarball = tmp.join("a3s-box.tar.gz");

    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    std::fs::create_dir_all(bin_dir)?;

    eprintln!("a3s: downloading a3s-box {version} for {target}...");
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

    eprintln!("a3s: extracting a3s-box {version}...");
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

    let package_dir = extracted_box_package_dir(&tmp, version, target)?;

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
        for lib_dir in standalone_lib_dirs(bin_dir) {
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

    if !path_contains_dir(bin_dir) {
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

fn direct_update_bin_dir(current: &Path) -> anyhow::Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(current)?;
    if metadata.file_type().is_symlink() {
        return Err(anyhow::anyhow!(
            "a3s-box at {} is managed through a symbolic link; update it with its package manager or run `a3s install box` into A3S_BOX_INSTALL_DIR",
            current.display()
        ));
    }
    current
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("a3s-box path has no parent: {}", current.display()))
}

fn installed_box_at(path: PathBuf) -> anyhow::Result<InstalledBox> {
    let located = locate_existing_a3s_box()
        .filter(|located| located.path == path)
        .unwrap_or(LocatedBox {
            source: source_for_path(&path),
            path,
        });
    Ok(InstalledBox {
        version: probe_box_version(&located.path),
        path: located.path,
        source: located.source,
    })
}

fn source_for_path(path: &Path) -> BoxSource {
    if configured_box_path().as_deref() == Some(path) {
        BoxSource::ConfiguredInstallDir
    } else if sibling_box_path().as_deref() == Some(path) {
        BoxSource::Sibling
    } else if fallback_box_paths()
        .iter()
        .any(|candidate| candidate == path)
    {
        BoxSource::LegacyUserBin
    } else {
        BoxSource::Path
    }
}

fn print_already_installed(installed: &InstalledBox) {
    match installed.version.as_deref() {
        Some(version) => println!(
            "✓ a3s Box {version} is already installed at {}",
            installed.path.display()
        ),
        None => println!(
            "✓ a3s Box is already installed at {}",
            installed.path.display()
        ),
    }
}

fn probe_box_version(path: &Path) -> Option<String> {
    for argument in ["version", "--version"] {
        let output = Command::new(path).arg(argument).output().ok()?;
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if let Some(version) = parse_box_version(&stdout).or_else(|| parse_box_version(&stderr)) {
            return Some(version);
        }
    }
    None
}

fn parse_box_version(output: &str) -> Option<String> {
    output
        .split(|character: char| {
            !(character.is_ascii_alphanumeric()
                || character == '.'
                || character == '-'
                || character == '+')
        })
        .filter_map(|token| {
            let token = token.trim_start_matches('v');
            let stable = token.split(['-', '+']).next().unwrap_or(token);
            let parts = stable.split('.').collect::<Vec<_>>();
            (parts.len() >= 2
                && parts
                    .iter()
                    .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())))
            .then(|| stable.to_string())
        })
        .next()
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
        .filter(|version| is_stable_release_version(version))
}

fn is_stable_release_version(version: &str) -> bool {
    let parts = version.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == &"0" || !part.starts_with('0'))
                && part.parse::<u32>().is_ok()
        })
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
        assert_eq!(
            version_from_release_url("https://github.com/A3S-Lab/Box/releases/tag/v../../escape"),
            None
        );
        assert_eq!(
            version_from_release_url("https://github.com/A3S-Lab/Box/releases/tag/v1.2.3-beta"),
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
    fn parses_box_version_output() {
        assert_eq!(
            parse_box_version("a3s-box version 3.0.5\n"),
            Some("3.0.5".to_string())
        );
        assert_eq!(
            parse_box_version("a3s-box v2.6.0-beta.1"),
            Some("2.6.0".to_string())
        );
        assert_eq!(parse_box_version("a3s-box version unknown"), None);
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
    fn inspect_reports_configured_box_and_version() {
        let _guard = env_guard();
        let root = std::env::temp_dir().join(format!("a3s-box-status-test-{}", std::process::id()));
        let bin_dir = root.join("bin");
        let home = root.join("home");
        let bin = bin_dir.join(BOX_BINARY);
        make_executable_with_body(
            &bin,
            "#!/bin/sh\nprintf 'a3s-box version 3.0.5\\n'\nexit 0\n",
        );

        let old_install_dir = std::env::var_os("A3S_BOX_INSTALL_DIR");
        let old_home = std::env::var_os("HOME");
        let old_path = std::env::var_os("PATH");
        std::env::set_var("A3S_BOX_INSTALL_DIR", &bin_dir);
        std::env::set_var("HOME", &home);
        std::env::set_var("PATH", "");

        let state = inspect();

        restore_var("A3S_BOX_INSTALL_DIR", old_install_dir);
        restore_var("HOME", old_home);
        restore_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(
            state,
            BoxState::Installed(InstalledBox {
                version: Some("3.0.5".to_string()),
                path: bin,
                source: BoxSource::ConfiguredInstallDir,
            })
        );
    }

    #[test]
    fn read_only_inspection_never_executes_box() {
        let _guard = env_guard();
        let root = std::env::temp_dir().join(format!(
            "a3s-box-read-only-status-test-{}",
            std::process::id()
        ));
        let bin_dir = root.join("bin");
        let marker = root.join("executed");
        let bin = bin_dir.join(BOX_BINARY);
        make_executable_with_body(
            &bin,
            &format!(
                "#!/bin/sh\nprintf invoked > '{}'\nprintf 'a3s-box version 3.0.5\\n'\n",
                marker.display()
            ),
        );

        let old_install_dir = std::env::var_os("A3S_BOX_INSTALL_DIR");
        let old_home = std::env::var_os("HOME");
        let old_path = std::env::var_os("PATH");
        std::env::set_var("A3S_BOX_INSTALL_DIR", &bin_dir);
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("PATH", "");

        let state = inspect_read_only();

        restore_var("A3S_BOX_INSTALL_DIR", old_install_dir);
        restore_var("HOME", old_home);
        restore_var("PATH", old_path);
        assert!(!marker.exists(), "read-only inspection executed a3s-box");
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(
            state,
            BoxState::Installed(InstalledBox {
                version: None,
                path: bin,
                source: BoxSource::ConfiguredInstallDir,
            })
        );
    }

    #[test]
    fn explicit_install_is_idempotent_for_existing_box() {
        let _guard = env_guard();
        let root =
            std::env::temp_dir().join(format!("a3s-box-install-test-{}", std::process::id()));
        let bin_dir = root.join("bin");
        let bin = bin_dir.join(BOX_BINARY);
        make_executable_with_body(
            &bin,
            "#!/bin/sh\nprintf 'a3s-box version 2.6.0\\n'\nexit 0\n",
        );

        let old_install_dir = std::env::var_os("A3S_BOX_INSTALL_DIR");
        let old_home = std::env::var_os("HOME");
        let old_path = std::env::var_os("PATH");
        std::env::set_var("A3S_BOX_INSTALL_DIR", &bin_dir);
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("PATH", "");

        let installed = install().unwrap();

        restore_var("A3S_BOX_INSTALL_DIR", old_install_dir);
        restore_var("HOME", old_home);
        restore_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(installed.path, bin);
        assert_eq!(installed.version.as_deref(), Some("2.6.0"));
    }

    #[test]
    fn update_missing_box_points_to_explicit_install() {
        let _guard = env_guard();
        let root =
            std::env::temp_dir().join(format!("a3s-box-update-missing-{}", std::process::id()));

        let old_install_dir = std::env::var_os("A3S_BOX_INSTALL_DIR");
        let old_home = std::env::var_os("HOME");
        let old_path = std::env::var_os("PATH");
        std::env::set_var("A3S_BOX_INSTALL_DIR", root.join("bin"));
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("PATH", "");

        let error = update().unwrap_err().to_string();

        restore_var("A3S_BOX_INSTALL_DIR", old_install_dir);
        restore_var("HOME", old_home);
        restore_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(
            error,
            "a3s box is not installed; run `a3s install box` first"
        );
    }

    #[test]
    fn update_is_noop_when_installed_box_is_current() {
        let _guard = env_guard();
        let root =
            std::env::temp_dir().join(format!("a3s-box-update-current-{}", std::process::id()));
        let bin_dir = root.join("bin");
        let tools = root.join("tools");
        let box_bin = bin_dir.join(BOX_BINARY);
        make_executable_with_body(
            &box_bin,
            "#!/bin/sh\nprintf 'a3s-box version 3.0.5\\n'\nexit 0\n",
        );
        make_executable_with_body(
            &tools.join("curl"),
            "#!/bin/sh\nprintf 'https://github.com/A3S-Lab/Box/releases/tag/v3.0.5\\n'\nexit 0\n",
        );

        let old_install_dir = std::env::var_os("A3S_BOX_INSTALL_DIR");
        let old_home = std::env::var_os("HOME");
        let old_path = std::env::var_os("PATH");
        std::env::set_var("A3S_BOX_INSTALL_DIR", &bin_dir);
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("PATH", &tools);

        let updated = update().unwrap();

        restore_var("A3S_BOX_INSTALL_DIR", old_install_dir);
        restore_var("HOME", old_home);
        restore_var("PATH", old_path);
        let _ = std::fs::remove_dir_all(root);

        assert_eq!(updated.path, box_bin);
        assert_eq!(updated.version.as_deref(), Some("3.0.5"));
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
        make_executable_with_body(path, "#!/bin/sh\nexit 0\n");
    }

    fn make_executable_with_body(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
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
