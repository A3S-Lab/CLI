use std::ffi::{OsStr, OsString};
#[cfg(windows)]
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};

use super::catalog::{ReleaseProbe, ReleaseSpec};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_OUTPUT: u64 = 1024 * 1024;
#[cfg(windows)]
const AGENT_ISLAND_HELPER_USAGE: &[u8] =
    b"usage: a3s-webview --agent-island --snapshot <absolute-path> --lock-file <absolute-path>";
#[cfg(windows)]
const SYSTEM_AGENT_SNAPSHOT_MARKER: &[u8] = b"a3s.system_agent_snapshot.v1";
#[cfg(windows)]
const MAX_AGENT_ISLAND_HELPER_BINARY_BYTES: u64 = 128 * 1024 * 1024;
#[cfg(windows)]
const MIN_WINDOWS_PE_HEADER_OFFSET: usize = 0x40;
#[cfg(windows)]
const MAX_WINDOWS_PE_HEADER_OFFSET: usize = 1024 * 1024;
#[cfg(all(windows, target_arch = "x86_64"))]
const WINDOWS_PE_MACHINE_AMD64: u16 = 0x8664;
#[cfg(all(windows, target_arch = "aarch64"))]
const WINDOWS_PE_MACHINE_ARM64: u16 = 0xaa64;

pub fn probe_release(release: ReleaseSpec, path: &Path) -> anyhow::Result<Option<String>> {
    match release.probe {
        ReleaseProbe::Version => probe_version(path).map(Some),
        ReleaseProbe::AgentIslandContract => {
            if webview_binary_supports_agent_island(path)? {
                Ok(None)
            } else {
                bail!("component executable does not support the Agent Island contract")
            }
        }
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

pub fn webview_supports_agent_island_output(stdout: &[u8], stderr: &[u8]) -> bool {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let contract = format!("{stdout}\n{stderr}");
    contract.contains("usage: a3s-webview --agent-island")
        && contract.contains("--snapshot")
        && contract.contains("--lock-file")
}

#[cfg(not(windows))]
pub fn webview_binary_supports_agent_island(binary: &Path) -> std::io::Result<bool> {
    if !is_executable(binary) {
        return Ok(false);
    }
    let output = run_bounded(
        binary.as_os_str(),
        &[OsString::from("--agent-island"), OsString::from("--help")],
    )
    .map_err(std::io::Error::other)?;
    Ok(webview_supports_agent_island_output(
        &output.stdout,
        &output.stderr,
    ))
}

/// Validate the Windows helper contract without executing an untrusted
/// candidate. The released helper does not expose `--version`, so readiness is
/// established from its target PE header and embedded Agent Island protocol.
#[cfg(windows)]
pub fn webview_binary_supports_agent_island(binary: &Path) -> std::io::Result<bool> {
    let file = std::fs::File::open(binary)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_AGENT_ISLAND_HELPER_BINARY_BYTES {
        return Ok(false);
    }
    let mut bytes = Vec::new();
    file.take(MAX_AGENT_ISLAND_HELPER_BINARY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_AGENT_ISLAND_HELPER_BINARY_BYTES {
        return Ok(false);
    }
    Ok(webview_binary_contains_agent_island_contract(&bytes))
}

#[cfg(windows)]
fn webview_binary_contains_agent_island_contract(bytes: &[u8]) -> bool {
    if !webview_binary_has_target_pe_header(bytes) {
        return false;
    }
    [AGENT_ISLAND_HELPER_USAGE, SYSTEM_AGENT_SNAPSHOT_MARKER]
        .into_iter()
        .all(|needle| {
            bytes
                .windows(needle.len())
                .any(|candidate| candidate == needle)
        })
}

#[cfg(windows)]
fn webview_binary_has_target_pe_header(bytes: &[u8]) -> bool {
    if bytes.get(..2) != Some(b"MZ") {
        return false;
    }
    let Some(pe_offset_bytes) = bytes.get(0x3c..0x40) else {
        return false;
    };
    let pe_offset = u32::from_le_bytes([
        pe_offset_bytes[0],
        pe_offset_bytes[1],
        pe_offset_bytes[2],
        pe_offset_bytes[3],
    ]);
    let Ok(pe_offset) = usize::try_from(pe_offset) else {
        return false;
    };
    if !(MIN_WINDOWS_PE_HEADER_OFFSET..=MAX_WINDOWS_PE_HEADER_OFFSET).contains(&pe_offset) {
        return false;
    }
    let Some(machine_offset) = pe_offset.checked_add(4) else {
        return false;
    };
    if bytes.get(pe_offset..machine_offset) != Some(b"PE\0\0") {
        return false;
    }
    let Some(machine_end) = machine_offset.checked_add(2) else {
        return false;
    };
    let Some(machine_bytes) = bytes.get(machine_offset..machine_end) else {
        return false;
    };
    let machine = u16::from_le_bytes([machine_bytes[0], machine_bytes[1]]);
    target_windows_pe_machine().is_some_and(|target| machine == target)
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn target_windows_pe_machine() -> Option<u16> {
    Some(WINDOWS_PE_MACHINE_AMD64)
}

#[cfg(all(windows, target_arch = "aarch64"))]
fn target_windows_pe_machine() -> Option<u16> {
    Some(WINDOWS_PE_MACHINE_ARM64)
}

#[cfg(all(windows, not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
fn target_windows_pe_machine() -> Option<u16> {
    None
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

    #[test]
    fn recognizes_the_agent_island_help_contract_across_output_streams() {
        assert!(webview_supports_agent_island_output(
            b"usage: a3s-webview --agent-island --snapshot <path>",
            b"options: --lock-file <path>"
        ));
        assert!(!webview_supports_agent_island_output(
            b"usage: a3s-webview --url <url>",
            b""
        ));
    }

    #[test]
    #[cfg(windows)]
    fn validates_the_target_pe_and_embedded_agent_island_contract() {
        let mut binary = vec![0_u8; 0x80];
        binary[..2].copy_from_slice(b"MZ");
        binary[0x3c..0x40].copy_from_slice(&0x40_u32.to_le_bytes());
        binary[0x40..0x44].copy_from_slice(b"PE\0\0");
        binary[0x44..0x46].copy_from_slice(
            &target_windows_pe_machine()
                .expect("supported Windows test target")
                .to_le_bytes(),
        );
        binary.extend_from_slice(AGENT_ISLAND_HELPER_USAGE);
        binary.extend_from_slice(SYSTEM_AGENT_SNAPSHOT_MARKER);

        assert!(webview_binary_contains_agent_island_contract(&binary));
        let marker = binary
            .windows(SYSTEM_AGENT_SNAPSHOT_MARKER.len())
            .position(|candidate| candidate == SYSTEM_AGENT_SNAPSHOT_MARKER)
            .unwrap();
        binary[marker] ^= 0xff;
        assert!(!webview_binary_contains_agent_island_contract(&binary));
    }
}
