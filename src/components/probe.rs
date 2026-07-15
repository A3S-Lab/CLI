use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_PROBE_OUTPUT: u64 = 1024 * 1024;

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

pub fn run_bounded(program: &OsStr, args: &[OsString]) -> anyhow::Result<BoundedOutput> {
    let stdout_file = tempfile::NamedTempFile::new()?;
    let stderr_file = tempfile::NamedTempFile::new()?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file.reopen()?))
        .stderr(Stdio::from(stderr_file.reopen()?))
        .spawn()
        .with_context(|| format!("failed to run {}", Path::new(program).display()))?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("component probe timed out after {:?}", PROBE_TIMEOUT);
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(10)));
    };
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
}
