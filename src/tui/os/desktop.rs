//! TUI handoff into the installed A3S Desktop workbench.
//!
//! Desktop owns the visual product loop. This module does not embed a webview,
//! install a second Desktop, or start a model turn. It selects the newest
//! runnable Desktop and either focuses the one already open on this workspace
//! or launches that binary with `A3S_DESKTOP_WORKSPACE`.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

const BUNDLE_ID: &str = "dev.a3s.desktop";
const WORKSPACE_ENV: &str = "A3S_DESKTOP_WORKSPACE";
const PIN_ENV: &str = "A3S_DESKTOP_BIN";
/// Same-version tie stays with an installed app (preference 30/20).
/// Checkout artifacts share one preference so a fresh binary beats a stale
/// bundle of the same version.
const CHECKOUT_BUNDLE_PREFERENCE: u8 = 8;
const CHECKOUT_BINARY_PREFERENCE: u8 = 8;
const CHECKOUT_WALK_LIMIT: usize = 8;
const CHECKOUT_BUNDLE_DIRS: &[&str] = &[
    "apps/desktop/src-tauri/target/release/bundle/macos",
    "apps/desktop/src-tauri/target/debug/bundle/macos",
];
const CHECKOUT_BINARIES: &[&str] = &[
    "apps/desktop/src-tauri/target/release/A3S",
    "apps/desktop/src-tauri/target/debug/A3S",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DesktopKind {
    Installed,
    Built,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DesktopCandidate {
    pub(crate) executable: PathBuf,
    pub(crate) version: Option<(u64, u64, u64)>,
    pub(crate) version_label: String,
    pub(crate) kind: DesktopKind,
    /// Tie-break only. A newer version always beats a higher preference.
    pub(crate) preference: u8,
    /// Last tie-break, so a fresh checkout build beats an older build of the
    /// same version without beating an installed app of that version.
    pub(crate) modified: SystemTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DesktopQuery {
    /// Operator pin. When set, discovery must not substitute another binary.
    pub(crate) pin: Option<PathBuf>,
    pub(crate) application_roots: Vec<(PathBuf, u8)>,
    pub(crate) bare_binaries: Vec<(PathBuf, u8)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DesktopSelection {
    pub(crate) executable: PathBuf,
    pub(crate) version_label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunningDesktop {
    pub(crate) pid: u32,
    pub(crate) executable: PathBuf,
    pub(crate) workspace: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DesktopOpen {
    Launch {
        executable: PathBuf,
        workspace: PathBuf,
    },
    Focus {
        pid: u32,
    },
}

#[derive(Debug)]
pub(crate) enum DesktopOpenError {
    PinnedBinaryMissing(PathBuf),
    NotInstalled { searched: String },
    Launch(std::io::Error),
    Focus(std::io::Error),
}

impl std::fmt::Display for DesktopOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PinnedBinaryMissing(path) => write!(
                f,
                "pinned Desktop binary is not runnable: {}",
                path.display()
            ),
            Self::NotInstalled { searched } => write!(
                f,
                "A3S Desktop is not installed (searched {searched}). Install the latest desktop release or set {PIN_ENV}."
            ),
            Self::Launch(error) => write!(f, "could not launch A3S Desktop: {error}"),
            Self::Focus(error) => write!(f, "could not focus the running A3S Desktop: {error}"),
        }
    }
}

pub(crate) fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let mut parts = raw.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch_token = parts.next().unwrap_or("0");
    let patch = patch_token
        .split(['-', '+'])
        .next()
        .unwrap_or("0")
        .parse()
        .ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub(crate) fn select_latest(candidates: &[DesktopCandidate]) -> Option<&DesktopCandidate> {
    candidates.iter().max_by(|left, right| {
        match (&left.version, &right.version) {
            (Some(left_version), Some(right_version)) => left_version.cmp(right_version),
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            (None, None) => Ordering::Equal,
        }
        .then(left.preference.cmp(&right.preference))
        .then(left.modified.cmp(&right.modified))
    })
}

pub(crate) fn resolve_desktop(query: &DesktopQuery) -> Result<DesktopSelection, DesktopOpenError> {
    if let Some(pin) = &query.pin {
        return runnable_file(pin)
            .map(|executable| DesktopSelection {
                executable,
                version_label: version_label_for_executable(pin),
            })
            .ok_or_else(|| DesktopOpenError::PinnedBinaryMissing(pin.clone()));
    }

    let mut candidates = Vec::new();
    for (root, preference) in &query.application_roots {
        if let Some(candidate) = candidate_from_app(root, *preference) {
            candidates.push(candidate);
        }
    }
    for (path, preference) in &query.bare_binaries {
        if let Some(executable) = runnable_file(path) {
            candidates.push(candidate_from_binary(executable, *preference));
        }
    }
    select_latest(&candidates)
        .map(|candidate| DesktopSelection {
            executable: candidate.executable.clone(),
            version_label: candidate.version_label.clone(),
        })
        .ok_or_else(|| DesktopOpenError::NotInstalled {
            searched: describe_search(query),
        })
}

fn describe_search(query: &DesktopQuery) -> String {
    let mut parts = Vec::new();
    for (root, _) in &query.application_roots {
        parts.push(root.display().to_string());
    }
    for (path, _) in &query.bare_binaries {
        parts.push(path.display().to_string());
    }
    if parts.is_empty() {
        "no application roots or checkout binaries".to_string()
    } else {
        parts.join(", ")
    }
}

pub(crate) fn plan_desktop_open(
    selection: &DesktopSelection,
    workspace: &Path,
    running: &[RunningDesktop],
) -> DesktopOpen {
    let workspace = canonical_dir(workspace);
    let selected = canonical_file(&selection.executable);
    if let Some(existing) = running.iter().find(|process| {
        canonical_file(&process.executable) == selected
            && process
                .workspace
                .as_ref()
                .is_some_and(|open| canonical_dir(open) == workspace)
    }) {
        return DesktopOpen::Focus { pid: existing.pid };
    }
    DesktopOpen::Launch {
        executable: selection.executable.clone(),
        workspace,
    }
}

pub(crate) fn format_desktop_report(selection: &DesktopSelection, action: &DesktopOpen) -> String {
    match action {
        DesktopOpen::Focus { pid } => format!(
            "desktop · focused existing pid {pid} · {} · same workspace, no second process",
            selection.version_label
        ),
        DesktopOpen::Launch {
            executable,
            workspace,
        } => format!(
            "desktop · launched {} · {} · workspace {}",
            selection.version_label,
            executable.display(),
            workspace.display()
        ),
    }
}

fn format_launched_report(selection: &DesktopSelection, pid: u32, workspace: &Path) -> String {
    format!(
        "desktop · launched pid {pid} · {} · workspace {}",
        selection.version_label,
        workspace.display()
    )
}

pub(crate) fn open_workspace(workspace: &Path) -> Result<String, DesktopOpenError> {
    let selection = resolve_desktop(&production_query())?;
    let running = running_desktops();
    let action = plan_desktop_open(&selection, workspace, &running);
    match &action {
        DesktopOpen::Launch {
            executable,
            workspace,
        } => {
            let pid = launch_detached(executable, workspace).map_err(DesktopOpenError::Launch)?;
            Ok(format_launched_report(&selection, pid, workspace))
        }
        DesktopOpen::Focus { .. } => {
            execute_desktop_open(&action)?;
            Ok(format_desktop_report(&selection, &action))
        }
    }
}

fn production_query() -> DesktopQuery {
    let mut application_roots = default_application_roots();
    let mut bare_binaries = Vec::new();
    for start in discovery_starts() {
        let (roots, binaries) = checkout_desktop_candidates(&start);
        application_roots.extend(roots);
        bare_binaries.extend(binaries);
    }
    DesktopQuery {
        pin: std::env::var_os(PIN_ENV).map(PathBuf::from),
        application_roots,
        bare_binaries,
    }
}

fn discovery_starts() -> Vec<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        starts.push(executable);
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    starts
}

/// Newest Desktop built in this checkout, if the running CLI or the workspace
/// sits inside that checkout. A shipped binary outside the checkout finds nothing
/// here and falls through to the installed app.
pub(crate) fn checkout_desktop_candidates(
    start: &Path,
) -> (Vec<(PathBuf, u8)>, Vec<(PathBuf, u8)>) {
    let mut roots = Vec::new();
    let mut binaries = Vec::new();
    let mut seen = Vec::new();
    let mut dir = if start.is_file() {
        start.parent().unwrap_or(start).to_path_buf()
    } else {
        start.to_path_buf()
    };
    for _ in 0..CHECKOUT_WALK_LIMIT {
        if seen.iter().any(|seen: &PathBuf| seen == &dir) {
            break;
        }
        seen.push(dir.clone());
        for relative in CHECKOUT_BUNDLE_DIRS {
            let root = dir.join(relative);
            if runnable_file(&root.join("A3S.app/Contents/MacOS/A3S")).is_some() {
                roots.push((root, CHECKOUT_BUNDLE_PREFERENCE));
            }
        }
        for relative in CHECKOUT_BINARIES {
            let executable = dir.join(relative);
            if runnable_file(&executable).is_some() {
                binaries.push((executable, CHECKOUT_BINARY_PREFERENCE));
            }
        }
        let Some(parent) = dir.parent() else {
            break;
        };
        if parent == dir {
            break;
        }
        dir = parent.to_path_buf();
    }
    (roots, binaries)
}

fn default_application_roots() -> Vec<(PathBuf, u8)> {
    let mut roots = vec![(PathBuf::from("/Applications"), 30)];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push((PathBuf::from(home).join("Applications"), 20));
    }
    roots
}

fn candidate_from_app(root: &Path, preference: u8) -> Option<DesktopCandidate> {
    let app = root.join("A3S.app");
    let executable = runnable_file(&app.join("Contents/MacOS/A3S"))?;
    let plist = fs::read_to_string(app.join("Contents/Info.plist")).ok()?;
    if let Some(bundle_id) = plist_string(&plist, "CFBundleIdentifier") {
        if bundle_id != BUNDLE_ID {
            return None;
        }
    }
    let version_label = plist_string(&plist, "CFBundleShortVersionString").unwrap_or_default();
    let modified = modified_at(&executable);
    Some(DesktopCandidate {
        executable,
        version: parse_version(&version_label),
        version_label: if version_label.is_empty() {
            "unversioned".to_string()
        } else {
            version_label
        },
        kind: DesktopKind::Installed,
        preference,
        modified,
    })
}

fn candidate_from_binary(executable: PathBuf, preference: u8) -> DesktopCandidate {
    let version_label = version_label_for_executable(&executable);
    let version = parse_version(&version_label);
    let modified = modified_at(&executable);
    DesktopCandidate {
        executable,
        version,
        version_label: if version.is_some() {
            version_label
        } else {
            "unversioned".to_string()
        },
        kind: DesktopKind::Built,
        preference,
        modified,
    }
}

fn version_label_for_executable(executable: &Path) -> String {
    executable
        .parent()
        .and_then(|dir| fs::read_to_string(dir.join("VERSION")).ok())
        .map(|label| label.trim().to_string())
        .filter(|label| !label.is_empty())
        .or_else(|| version_from_tauri_conf(executable))
        .unwrap_or_else(|| "unversioned".to_string())
}

fn version_from_tauri_conf(executable: &Path) -> Option<String> {
    let target_dir = executable.parent()?;
    let profile = target_dir.file_name()?.to_str()?;
    if profile != "debug" && profile != "release" {
        return None;
    }
    let src_tauri = target_dir.parent()?.parent()?;
    if src_tauri.file_name()? != "src-tauri" {
        return None;
    }
    let text = fs::read_to_string(src_tauri.join("tauri.conf.json")).ok()?;
    json_string_after_key(&text, "version")
}

fn json_string_after_key(text: &str, key: &str) -> Option<String> {
    let after_key = text.split_once(&format!("\"{key}\""))?.1;
    let after_colon = after_key.split_once(':')?.1;
    let value = after_colon.split('"').nth(1)?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn modified_at(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn plist_string(plist: &str, key: &str) -> Option<String> {
    let key_tag = format!("<key>{key}</key>");
    let after_key = plist.split_once(&key_tag)?.1;
    let after_string = after_key.split("<string>").nth(1)?;
    Some(after_string.split("</string>").next()?.trim().to_string())
}

fn runnable_file(path: &Path) -> Option<PathBuf> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    Some(path.to_path_buf())
}

fn canonical_dir(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn canonical_file(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn launch_detached(executable: &Path, workspace: &Path) -> std::io::Result<u32> {
    // Do not `pre_exec` inside the TUI. That forces `fork` of this process and
    // stalls the handoff. A short-lived helper creates the session and exits,
    // so the workbench is reparented and keeps the loader path.
    // `python3 -c` places the first extra argument in sys.argv[1].
    let log = std::env::temp_dir().join(format!("a3s-desktop-launch-{}.log", std::process::id()));
    let library_path = launch_library_path(
        executable,
        &std::env::var("DYLD_LIBRARY_PATH").unwrap_or_default(),
    );
    let fallback_path = launch_library_path(
        executable,
        &std::env::var("DYLD_FALLBACK_LIBRARY_PATH").unwrap_or_default(),
    );
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import os, subprocess, sys
log = open(sys.argv[4], "ab")
env = os.environ.copy()
if sys.argv[2]:
    env["DYLD_LIBRARY_PATH"] = sys.argv[2]
if sys.argv[3]:
    env["DYLD_FALLBACK_LIBRARY_PATH"] = sys.argv[3]
proc = subprocess.Popen([sys.argv[1]], env=env, cwd=sys.argv[5], stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
print(proc.pid)
"#,
        )
        .arg(executable)
        .arg(library_path)
        .arg(fallback_path)
        .arg(&log)
        .arg(workspace)
        .env(WORKSPACE_ENV, workspace)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "detached Desktop launch exited {}",
            output.status
        )));
    }
    let pid = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|_| std::io::Error::other("detached Desktop did not report a pid"))?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    if process_alive(pid) {
        let _ = fs::remove_file(&log);
        return Ok(pid);
    }
    let detail = fs::read_to_string(&log)
        .unwrap_or_default()
        .chars()
        .take(240)
        .collect::<String>();
    let _ = fs::remove_file(&log);
    Err(std::io::Error::other(if detail.is_empty() {
        format!("A3S Desktop pid {pid} exited during launch")
    } else {
        format!("A3S Desktop pid {pid} exited during launch: {detail}")
    }))
}

fn launch_library_path(executable: &Path, inherited: &str) -> String {
    let mut dirs = Vec::new();
    if let Some(parent) = executable.parent() {
        let checkout_binary = executable
            .components()
            .any(|component| component.as_os_str() == "src-tauri");
        if checkout_binary && parent.join("libzvec_c_api.dylib").is_file() {
            dirs.push(parent.display().to_string());
        }
    }
    if !inherited.is_empty() {
        dirs.push(inherited.to_string());
    }
    dirs.join(":")
}

fn process_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn execute_desktop_open(action: &DesktopOpen) -> Result<(), DesktopOpenError> {
    match action {
        DesktopOpen::Launch {
            executable,
            workspace,
        } => launch_detached(executable, workspace)
            .map(|_| ())
            .map_err(DesktopOpenError::Launch),
        DesktopOpen::Focus { pid } => focus_pid(*pid).map_err(DesktopOpenError::Focus),
    }
}

#[cfg(target_os = "macos")]
fn focus_pid(pid: u32) -> std::io::Result<()> {
    let script = format!(
        "tell application \"System Events\" to set frontmost of the first process whose unix id is {pid} to true"
    );
    let status = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "System Events did not focus pid {pid}"
        )))
    }
}

#[cfg(not(target_os = "macos"))]
fn focus_pid(pid: u32) -> std::io::Result<()> {
    let _ = pid;
    Err(std::io::Error::other(
        "focusing a running Desktop is implemented on macOS only",
    ))
}

#[cfg(target_os = "macos")]
fn running_desktops() -> Vec<RunningDesktop> {
    let output = Command::new("/bin/ps")
        .args(["-axww", "-o", "pid=,command="])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();
    for line in listing.lines() {
        let Some((pid, command)) = parse_ps_line(line) else {
            continue;
        };
        let Some(executable) = executable_from_command(command) else {
            continue;
        };
        if !looks_like_desktop(&executable) {
            continue;
        }
        processes.push(RunningDesktop {
            pid,
            executable,
            workspace: process_workspace(pid),
        });
    }
    processes
}

#[cfg(not(target_os = "macos"))]
fn running_desktops() -> Vec<RunningDesktop> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn parse_ps_line(line: &str) -> Option<(u32, &str)> {
    let line = line.trim();
    let (pid, command) = line.split_once(char::is_whitespace)?;
    let pid = pid.parse().ok()?;
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    Some((pid, command))
}

#[cfg(target_os = "macos")]
fn executable_from_command(command: &str) -> Option<PathBuf> {
    if let Some(stripped) = command.strip_prefix('"') {
        let end = stripped.find('"')?;
        return Some(PathBuf::from(&stripped[..end]));
    }
    Some(PathBuf::from(command.split_whitespace().next()?))
}

#[cfg(target_os = "macos")]
fn looks_like_desktop(executable: &Path) -> bool {
    executable.ends_with("A3S")
        || executable
            .file_name()
            .is_some_and(|name| name == "A3S" || name == "a3s-desktop")
}

#[cfg(target_os = "macos")]
fn process_workspace(pid: u32) -> Option<PathBuf> {
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-wwE"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    env_value(&String::from_utf8_lossy(&output.stdout), WORKSPACE_ENV).map(PathBuf::from)
}

pub(crate) fn env_value(blob: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=");
    let start = blob.find(&marker)? + marker.len();
    let rest = &blob[start..];
    let mut end = rest.len();
    let mut index = 0;
    while let Some(relative) = rest[index..].find(' ') {
        let at = index + relative;
        let next = rest[at + 1..].trim_start();
        let token = next.split_whitespace().next().unwrap_or("");
        if is_env_assignment(token) {
            end = at;
            break;
        }
        index = at + 1;
    }
    let value = rest[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn is_env_assignment(token: &str) -> bool {
    let Some((key, _)) = token.split_once('=') else {
        return false;
    };
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_app(root: &Path, version: &str, bundle_id: &str) -> PathBuf {
        let macos = root.join("A3S.app/Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        let executable = macos.join("A3S");
        fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).unwrap();
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{bundle_id}</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
</dict></plist>"#
        );
        fs::write(root.join("A3S.app/Contents/Info.plist"), plist).unwrap();
        executable
    }

    fn filetime_set_older(path: &Path) {
        let status = Command::new("touch")
            .args(["-t", "200001010000"])
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn write_binary(path: &Path, version: Option<&str>) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
        if let Some(version) = version {
            fs::write(path.parent().unwrap().join("VERSION"), version).unwrap();
        }
    }

    #[test]
    fn newer_install_beats_an_older_app_in_a_preferred_root() {
        let root = tempfile::tempdir().unwrap();
        let older_root = root.path().join("Applications");
        let newer_root = root.path().join("home-Applications");
        write_app(&older_root, "0.1.0", BUNDLE_ID);
        let newer = write_app(&newer_root, "0.2.0", BUNDLE_ID);
        let selection = resolve_desktop(&DesktopQuery {
            pin: None,
            application_roots: vec![(older_root, 30), (newer_root, 10)],
            bare_binaries: Vec::new(),
        })
        .unwrap();
        assert_eq!(selection.executable, newer);
        assert_eq!(selection.version_label, "0.2.0");
    }

    #[test]
    fn same_version_prefers_the_product_install_over_a_debug_binary() {
        let root = tempfile::tempdir().unwrap();
        let installed_root = root.path().join("Applications");
        let installed = write_app(&installed_root, "0.2.0", BUNDLE_ID);
        let debug = root.path().join("target/debug/A3S");
        write_binary(&debug, Some("0.2.0"));
        let selection = resolve_desktop(&DesktopQuery {
            pin: None,
            application_roots: vec![(installed_root, 30)],
            bare_binaries: vec![(debug, 0)],
        })
        .unwrap();
        assert_eq!(selection.executable, installed);
    }

    #[test]
    fn unversioned_binary_loses_to_any_versioned_app() {
        let root = tempfile::tempdir().unwrap();
        let app_root = root.path().join("Applications");
        let installed = write_app(&app_root, "0.0.1", BUNDLE_ID);
        let debug = root.path().join("A3S");
        write_binary(&debug, None);
        let selection = resolve_desktop(&DesktopQuery {
            pin: None,
            application_roots: vec![(app_root, 1)],
            bare_binaries: vec![(debug, 100)],
        })
        .unwrap();
        assert_eq!(selection.executable, installed);
    }

    #[test]
    fn missing_pin_does_not_substitute_another_binary() {
        let root = tempfile::tempdir().unwrap();
        let app_root = root.path().join("Applications");
        write_app(&app_root, "9.0.0", BUNDLE_ID);
        let missing = root.path().join("missing-A3S");
        let error = resolve_desktop(&DesktopQuery {
            pin: Some(missing.clone()),
            application_roots: vec![(app_root, 30)],
            bare_binaries: Vec::new(),
        })
        .unwrap_err();
        assert!(matches!(error, DesktopOpenError::PinnedBinaryMissing(path) if path == missing));
    }

    #[test]
    fn foreign_bundle_id_is_not_a_desktop_candidate() {
        let root = tempfile::tempdir().unwrap();
        write_app(root.path(), "3.0.0", "com.example.other");
        let error = resolve_desktop(&DesktopQuery {
            pin: None,
            application_roots: vec![(root.path().to_path_buf(), 30)],
            bare_binaries: Vec::new(),
        })
        .unwrap_err();
        assert!(matches!(error, DesktopOpenError::NotInstalled { .. }));
    }

    #[test]
    fn matching_workspace_focuses_instead_of_spawning() {
        let executable = PathBuf::from("/tmp/A3S");
        let workspace = PathBuf::from("/tmp/workspace");
        let action = plan_desktop_open(
            &DesktopSelection {
                executable: executable.clone(),
                version_label: "0.2.0".to_string(),
            },
            &workspace,
            &[RunningDesktop {
                pid: 42,
                executable,
                workspace: Some(workspace.clone()),
            }],
        );
        assert_eq!(action, DesktopOpen::Focus { pid: 42 });
        let report = format_desktop_report(
            &DesktopSelection {
                executable: PathBuf::from("/tmp/A3S"),
                version_label: "0.2.0".to_string(),
            },
            &action,
        );
        assert!(report.contains("no second process"));
        assert!(!report.to_ascii_lowercase().contains("model"));
    }

    #[test]
    fn different_workspace_plans_a_launch_without_a_model_turn() {
        let executable = PathBuf::from("/tmp/A3S");
        let action = plan_desktop_open(
            &DesktopSelection {
                executable: executable.clone(),
                version_label: "0.2.0".to_string(),
            },
            Path::new("/tmp/other"),
            &[RunningDesktop {
                pid: 42,
                executable,
                workspace: Some(PathBuf::from("/tmp/workspace")),
            }],
        );
        match action {
            DesktopOpen::Launch { workspace, .. } => {
                assert_eq!(workspace, PathBuf::from("/tmp/other"));
            }
            DesktopOpen::Focus { .. } => panic!("different workspace must not only focus"),
        }
    }

    #[test]
    fn checkout_launch_adds_the_sibling_loader_without_touching_an_install() {
        let root = tempfile::tempdir().unwrap();
        let debug = root.path().join("apps/desktop/src-tauri/target/debug/A3S");
        write_binary(&debug, None);
        fs::write(
            debug.parent().unwrap().join("libzvec_c_api.dylib"),
            b"dylib",
        )
        .unwrap();
        let path = launch_library_path(&debug, "other");
        assert!(path.starts_with(&debug.parent().unwrap().display().to_string()));
        assert!(path.ends_with(":other"));

        let installed = root.path().join("A3S.app/Contents/MacOS/A3S");
        write_binary(&installed, None);
        fs::write(
            installed.parent().unwrap().join("libzvec_c_api.dylib"),
            b"dylib",
        )
        .unwrap();
        assert_eq!(launch_library_path(&installed, ""), "");
    }

    #[test]
    fn checkout_walk_reads_the_tauri_version_and_keeps_an_install_on_a_tie() {
        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("repo");
        let debug = checkout.join("apps/desktop/src-tauri/target/debug/A3S");
        write_binary(&debug, None);
        fs::create_dir_all(debug.parent().unwrap().parent().unwrap().parent().unwrap()).unwrap();
        fs::write(
            checkout.join("apps/desktop/src-tauri/tauri.conf.json"),
            r#"{"version":"0.1.0"}"#,
        )
        .unwrap();
        let nested = checkout.join("crates/cli/target/debug");
        fs::create_dir_all(&nested).unwrap();
        let (roots, binaries) = checkout_desktop_candidates(&nested.join("a3s"));
        assert!(roots.is_empty());
        assert_eq!(binaries, vec![(debug.clone(), CHECKOUT_BINARY_PREFERENCE)]);
        let built = resolve_desktop(&DesktopQuery {
            pin: None,
            application_roots: Vec::new(),
            bare_binaries: binaries.clone(),
        })
        .unwrap();
        assert_eq!(built.version_label, "0.1.0");

        let installed_root = root.path().join("Applications");
        let installed = write_app(&installed_root, "0.1.0", BUNDLE_ID);
        let selection = resolve_desktop(&DesktopQuery {
            pin: None,
            application_roots: vec![(installed_root, 30)],
            bare_binaries: binaries,
        })
        .unwrap();
        assert_eq!(selection.executable, installed);
        assert_eq!(selection.version_label, "0.1.0");
    }

    #[test]
    fn newer_checkout_build_beats_an_older_build_of_the_same_version() {
        let root = tempfile::tempdir().unwrap();
        let checkout = root.path().join("repo");
        let release = checkout.join("apps/desktop/src-tauri/target/release/A3S");
        let debug = checkout.join("apps/desktop/src-tauri/target/debug/A3S");
        write_binary(&release, Some("0.1.0"));
        write_binary(&debug, Some("0.1.0"));
        filetime_set_older(&release);
        let (_, binaries) = checkout_desktop_candidates(&checkout);
        let selection = resolve_desktop(&DesktopQuery {
            pin: None,
            application_roots: Vec::new(),
            bare_binaries: binaries,
        })
        .unwrap();
        assert_eq!(selection.executable, debug);
    }

    #[test]
    fn workspace_env_parser_keeps_spaces_and_stops_at_the_next_key() {
        let blob = "  9 /bin/A3S A3S_DESKTOP_WORKSPACE=/private/tmp/A3S Workspace HOME=/Users/me";
        assert_eq!(
            env_value(blob, WORKSPACE_ENV).as_deref(),
            Some("/private/tmp/A3S Workspace")
        );
    }
}
