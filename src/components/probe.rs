use std::ffi::{OsStr, OsString};
#[cfg(windows)]
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};

use super::catalog::{ReleaseProbe, ReleaseSpec};

// A freshly extracted, checksummed component can incur one-time executable
// scanning on macOS and Windows. Keep the probe bounded, but allow enough time
// for that cold-start path before declaring a valid managed release broken.
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_PROBE_OUTPUT: u64 = 1024 * 1024;

pub fn probe_release(release: ReleaseSpec, path: &Path) -> anyhow::Result<Option<String>> {
    match release.probe {
        ReleaseProbe::Version => probe_version(path).map(Some),
        ReleaseProbe::WebViewRemoteUi => probe_webview_remoteui(path),
    }
}
#[cfg(unix)]
fn configure_probe_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_probe_process_group(_command: &mut Command) {}

struct ProbeProcessGroup {
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
}

impl ProbeProcessGroup {
    fn attach(_child: &std::process::Child) -> Self {
        Self {
            #[cfg(unix)]
            process_group: libc::pid_t::try_from(_child.id()).ok(),
        }
    }

    fn terminate(&mut self) {
        #[cfg(unix)]
        if let Some(process_group) = self.process_group.take() {
            // SAFETY: the probe is the leader of a dedicated process group.
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
    }
}

impl Drop for ProbeProcessGroup {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub fn probe_version(path: &Path) -> anyhow::Result<String> {
    if !is_executable(path) {
        bail!("component executable is missing or not executable");
    }
    let output = run_bounded(path.as_os_str(), &[OsString::from("--version")])?;
    if !output.success {
        bail!("component version probe exited unsuccessfully");
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    parse_version_output(&text).context("component version probe returned no version")
}

/// Published WebView helpers before 0.2.0 reject `--version` but still own
/// RemoteUI via `--help` / `--url`. Prefer a parseable version when present.
pub fn probe_webview_remoteui(path: &Path) -> anyhow::Result<Option<String>> {
    if !is_executable(path) {
        bail!("component executable is missing or not executable");
    }
    match probe_version(path) {
        Ok(version) => return Ok(Some(version)),
        Err(_) => {}
    }
    let output = run_bounded(path.as_os_str(), &[OsString::from("--help")])?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if webview_exposes_remoteui_usage(&text) {
        return Ok(None);
    }
    bail!("webview helper does not expose the RemoteUI contract")
}

pub fn webview_exposes_remoteui_usage(output: &str) -> bool {
    let lowered = output.to_ascii_lowercase();
    lowered.contains("usage: a3s-webview") && lowered.contains("--url")
}
pub fn run_bounded(program: &OsStr, args: &[OsString]) -> anyhow::Result<BoundedOutput> {
    let stdout_file = tempfile::NamedTempFile::new()?;
    let stderr_file = tempfile::NamedTempFile::new()?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file.reopen()?))
        .stderr(Stdio::from(stderr_file.reopen()?));
    configure_probe_process_group(&mut command);
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to run {}", Path::new(program).display()))?;
    let mut process_group = ProbeProcessGroup::attach(&child);
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let now = Instant::now();
        if now >= deadline {
            process_group.terminate();
            let _ = child.kill();
            let _ = child.wait();
            bail!("component probe timed out after {:?}", PROBE_TIMEOUT);
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(10)));
    };
    // Version probes own no persistent background service.
    process_group.terminate();
    for file in [&stdout_file, &stderr_file] {
        if file.as_file().metadata()?.len() > MAX_PROBE_OUTPUT {
            bail!("component probe output exceeded {} bytes", MAX_PROBE_OUTPUT);
        }
    }
    Ok(BoundedOutput {
        success: status.success(),
        stdout: std::fs::read(stdout_file.path())?,
        stderr: std::fs::read(stderr_file.path())?,
    })
}

pub struct BoundedOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

fn parse_version_output(output: &str) -> Option<String> {
    output
        .split(|character: char| {
            !(character.is_ascii_alphanumeric()
                || character == '.'
                || character == '-'
                || character == '+')
        })
        .map(|token| token.trim_start_matches('v'))
        .find(|token| {
            token.contains('.')
                && token
                    .split('.')
                    .take(2)
                    .all(|part| !part.is_empty() && part.chars().all(|char| char.is_ascii_digit()))
        })
        .map(str::to_string)
}

pub fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_version_output() {
        assert_eq!(
            parse_version_output("a3s-use v1.2.3\n"),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            parse_version_output("a3s-search 1.4.1-beta.1"),
            Some("1.4.1-beta.1".to_string())
        );
        assert_eq!(parse_version_output("unknown"), None);
    }

    #[test]
    fn webview_help_usage_counts_as_remoteui_ready() {
        assert!(webview_exposes_remoteui_usage(
            "usage: a3s-webview --url <http(s)://…|file://…> [--width N]\n"
        ));
        assert!(!webview_exposes_remoteui_usage(
            "a3s-webview: unknown argument: --version\n"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn webview_probe_accepts_help_only_helpers() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("a3s-webview");
        std::fs::write(
            &executable,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf '%s\\n' 'a3s-webview: unknown argument: --version' >&2\n  exit 2\nfi\nprintf '%s\\n' 'usage: a3s-webview --url <http(s)://example>'\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();

        let version = probe_webview_remoteui(&executable).unwrap();
        assert_eq!(version, None);
    }

    #[cfg(unix)]
    #[test]
    fn webview_probe_accepts_published_homebrew_helper_when_present() {
        let path = Path::new("/opt/homebrew/bin/a3s-webview");
        if !path.exists() {
            return;
        }
        let version =
            probe_webview_remoteui(path).expect("published helper must be RemoteUI-ready");
        // 0.1.5 has no --version; newer builds may return Some.
        let _ = version;
    }

    #[cfg(unix)]
    #[test]
    fn completed_probe_kills_surviving_descendants() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("component");
        let leaked = directory.path().join("probe-leak");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\n(sleep 0.30; : > '{}') &\nprintf 'component 1.2.3\\n'\n",
                leaked.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();

        let version = probe_version(&executable).unwrap();

        assert_eq!(version, "1.2.3");
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            !leaked.exists(),
            "a completed component probe must kill helper descendants"
        );
    }
}
