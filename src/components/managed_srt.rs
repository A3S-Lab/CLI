use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use a3s_code_core::sandbox::srt::{SrtBashSandbox, MANAGED_SRT_VERSION, SRT_NPM_PACKAGE_NAME};
use a3s_code_core::sandbox::{BashSandbox, SandboxCommandRequest};
use a3s_updater::{
    ComponentReceipt, DirectoryActivation, InstallProvenance, RECEIPT_SCHEMA_VERSION,
};
use anyhow::{bail, Context};
use tokio::process::{Child, Command};

use super::id::ComponentId;
use super::lock::ComponentOperationLock;
use super::paths::ComponentPaths;
use tree::{hash_install_tree, hash_packaged_install_tree};

mod tree;

const SRT_COMPONENT_ID: &str = "code/srt";
const SRT_PACKAGE_INTEGRITY: &str =
    "sha512-OE7QiGZJXe7ZshP47U2vk2z9FGSyiSN4ca9krVrE28LS2Qj0AHRWZz+gAce6FzG3gx/4OjNFwIhDuHXnI0WWwA==";
const LOCKED_NPM_PACKAGES: &[LockedNpmPackage] = &[
    LockedNpmPackage {
        path: "node_modules/@anthropic-ai/sandbox-runtime",
        version: "0.0.66",
        integrity: SRT_PACKAGE_INTEGRITY,
        resolved: "https://registry.npmjs.org/@anthropic-ai/sandbox-runtime/-/sandbox-runtime-0.0.66.tgz",
    },
    LockedNpmPackage {
        path: "node_modules/@pondwader/socks5-server",
        version: "1.0.10",
        integrity: "sha512-bQY06wzzR8D2+vVCUoBsr5QS2U6UgPUQRmErNwtsuI6vLcyRKkafjkr3KxbtGFf9aBBIV2mcvlsKD1UYaIV+sg==",
        resolved: "https://registry.npmjs.org/@pondwader/socks5-server/-/socks5-server-1.0.10.tgz",
    },
    LockedNpmPackage {
        path: "node_modules/commander",
        version: "12.1.0",
        integrity: "sha512-Vw8qHK3bZM9y/P10u3Vib8o/DdkvA2OtPtZvD871QKjy74Wj1WSKFILMPRPSdUSx5RFK1arlJzEtA4PkFgnbuA==",
        resolved: "https://registry.npmjs.org/commander/-/commander-12.1.0.tgz",
    },
    LockedNpmPackage {
        path: "node_modules/node-forge",
        version: "1.4.0",
        integrity: "sha512-LarFH0+6VfriEhqMMcLX2F7SwSXeWwnEAJEsYm5QKWchiVYVvJyV9v7UDvUv+w5HO23ZpQTXDv/GxdDdMyOuoQ==",
        resolved: "https://registry.npmjs.org/node-forge/-/node-forge-1.4.0.tgz",
    },
    LockedNpmPackage {
        path: "node_modules/zod",
        version: "3.25.76",
        integrity: "sha512-gzUt/qt81nXsFGKIFcC3YnfEAx5NkunCfnDlvuBSSFS02bcXu4Lmea0AFIUwbLWxWPx3d9p8S5QoaujKcNQxcQ==",
        resolved: "https://registry.npmjs.org/zod/-/zod-3.25.76.tgz",
    },
];
const SRT_SOURCE: &str = "npm:@anthropic-ai/sandbox-runtime@0.0.66";
const NPM_REGISTRY: &str = "https://registry.npmjs.org/";
const PACKAGE_INTEGRITY_KEY: &str = "npm-package-integrity";
const INSTALL_TREE_KEY: &str = "install-tree-sha256";
pub const MANAGED_SRT_PAYLOAD_RELATIVE_ROOT: &str = "support/managed-srt";
const PACKAGED_SRT_TREE_SHA256: &str = include_str!("../../support/managed-srt.tree-sha256");
const MANAGED_PACKAGE_JSON: &[u8] = include_bytes!("../../support/managed-srt/package.json");
const MANAGED_PACKAGE_LOCK: &[u8] = include_bytes!("../../support/managed-srt/package-lock.json");
const INSTALL_TIMEOUT: Duration = Duration::from_secs(180);
const INSTALL_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(2);
const NODE_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const SANDBOX_PROBE_TIMEOUT_MS: u64 = 15_000;
const MINIMUM_NODE_VERSION: (u64, u64, u64) = (20, 11, 0);

#[derive(Debug, Clone, Copy)]
struct LockedNpmPackage {
    path: &'static str,
    version: &'static str,
    integrity: &'static str,
    resolved: &'static str,
}

fn configure_managed_command(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

struct ManagedCommandProcessGroup {
    #[cfg(unix)]
    process_group: Option<libc::pid_t>,
}

impl ManagedCommandProcessGroup {
    fn attach(child: &Child) -> Self {
        Self {
            #[cfg(unix)]
            process_group: child.id().and_then(|pid| libc::pid_t::try_from(pid).ok()),
        }
    }

    fn terminate(&mut self) {
        #[cfg(unix)]
        if let Some(process_group) = self.process_group.take() {
            // SAFETY: the managed helper was spawned as the leader of a new
            // process group. A negative PID terminates it and all descendants.
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
    }
}

impl Drop for ManagedCommandProcessGroup {
    fn drop(&mut self) {
        self.terminate();
    }
}

struct ManagedInstallChild {
    child: Child,
    process_group: ManagedCommandProcessGroup,
}

impl ManagedInstallChild {
    fn attach(child: Child) -> Self {
        let process_group = ManagedCommandProcessGroup::attach(&child);
        Self {
            child,
            process_group,
        }
    }

    fn terminate(&mut self) {
        self.process_group.terminate();
        let _ = self.child.start_kill();
    }
}

impl Drop for ManagedInstallChild {
    fn drop(&mut self) {
        self.terminate();
    }
}

async fn wait_for_managed_install(
    child: &mut ManagedInstallChild,
    timeout: Duration,
) -> anyhow::Result<std::process::ExitStatus> {
    match tokio::time::timeout(timeout, child.child.wait()).await {
        Ok(status) => {
            let status = status.context("failed to wait for npm")?;
            // npm setup owns no background service. Closing the group after
            // the direct child exits prevents an orphaned installer helper.
            child.process_group.terminate();
            Ok(status)
        }
        Err(_) => {
            child.terminate();
            let _ = tokio::time::timeout(INSTALL_SETTLEMENT_TIMEOUT, child.child.wait()).await;
            bail!(
                "npm did not finish managed SRT setup within {} seconds",
                timeout.as_secs()
            )
        }
    }
}

/// Exact managed runtime selected for one Code process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedSrtRuntime {
    executable: PathBuf,
    node: PathBuf,
}

impl ManagedSrtRuntime {
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn node(&self) -> &Path {
        &self.node
    }

    pub fn build_sandbox(&self, workspace: &Path) -> anyhow::Result<SrtBashSandbox> {
        SrtBashSandbox::from_verified_npm_with_node(&self.executable, &self.node, workspace)
            .context("managed SRT failed its Core capability handshake")
    }

    /// Build the exact sandbox and prove that its native OS boundary can start.
    ///
    /// Package verification alone cannot establish that platform facilities
    /// such as Seatbelt, Linux user namespaces, or Windows machine setup are
    /// usable. The TUI runs this bounded probe before exposing sandboxed Bash.
    pub async fn build_and_probe_sandbox(
        &self,
        workspace: &Path,
    ) -> anyhow::Result<SrtBashSandbox> {
        const MARKER: &str = "a3s-managed-srt-ready";
        #[cfg(windows)]
        const COMMAND: &str = "[Console]::Out.Write('a3s-managed-srt-ready')";
        #[cfg(not(windows))]
        const COMMAND: &str = "printf %s a3s-managed-srt-ready";

        let sandbox = self.build_sandbox(workspace)?;
        let output = sandbox
            .exec(SandboxCommandRequest {
                command: COMMAND.to_string(),
                guest_workspace: "/workspace".to_string(),
                timeout_ms: SANDBOX_PROBE_TIMEOUT_MS,
                output_observer: None,
                env: None,
            })
            .await
            .with_context(|| {
                format!(
                    "managed SRT could not start its OS capability probe; {}",
                    platform_sandbox_prerequisite_hint()
                )
            })?;
        if output.timed_out {
            bail!(
                "managed SRT OS capability probe exceeded {} seconds; {}",
                SANDBOX_PROBE_TIMEOUT_MS / 1_000,
                platform_sandbox_prerequisite_hint()
            );
        }
        if output.exit_code != 0 {
            let diagnostic = output.stderr.trim();
            bail!(
                "managed SRT OS capability probe exited with code {}{}; {}",
                output.exit_code,
                if diagnostic.is_empty() {
                    String::new()
                } else {
                    format!(": {diagnostic}")
                },
                platform_sandbox_prerequisite_hint()
            );
        }
        if output.stdout != MARKER {
            bail!(
                "managed SRT OS capability probe returned unexpected output; {}",
                platform_sandbox_prerequisite_hint()
            );
        }
        Ok(sandbox)
    }
}

fn platform_sandbox_prerequisite_hint() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "Linux requires bubblewrap, socat, ripgrep, and permitted unprivileged user namespaces"
    }
    #[cfg(target_os = "macos")]
    {
        "macOS requires sandbox-exec and ripgrep"
    }
    #[cfg(windows)]
    {
        "Windows requires the one-time elevated managed-sandbox machine setup"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        "this operating system has no release-qualified local sandbox provider"
    }
}

/// Non-fatal first-use result consumed by the Code TUI.
#[derive(Debug)]
pub struct ManagedSrtResolution {
    pub runtime: Option<ManagedSrtRuntime>,
    pub warning: Option<String>,
}

/// Resolve or prepare the exact local command sandbox used by A3S Code.
///
/// Existing verified state is always reusable. Network mutation occurs only
/// when first-use installation is enabled by the immutable invocation policy.
pub async fn resolve_managed_srt(
    paths: &ComponentPaths,
    workspace: &Path,
    allow_first_use_install: bool,
    offline: bool,
    progress: bool,
) -> ManagedSrtResolution {
    resolve_managed_srt_with(
        allow_first_use_install,
        offline,
        || discover_managed_srt(paths, workspace),
        || discover_packaged_srt(paths, workspace),
        || install_managed_srt(paths, workspace, progress),
    )
    .await
}

async fn resolve_managed_srt_with<D, DiscoverFuture, P, PackagedFuture, I, InstallFuture>(
    allow_first_use_install: bool,
    offline: bool,
    discover: D,
    discover_packaged: P,
    install: I,
) -> ManagedSrtResolution
where
    D: FnOnce() -> DiscoverFuture,
    DiscoverFuture: std::future::Future<Output = anyhow::Result<Option<ManagedSrtRuntime>>>,
    P: FnOnce() -> PackagedFuture,
    PackagedFuture: std::future::Future<Output = anyhow::Result<Option<ManagedSrtRuntime>>>,
    I: FnOnce() -> InstallFuture,
    InstallFuture: std::future::Future<Output = anyhow::Result<ManagedSrtRuntime>>,
{
    let discovery = match discover().await {
        Ok(Some(runtime)) => {
            return ManagedSrtResolution {
                runtime: Some(runtime),
                warning: None,
            };
        }
        other => other,
    };
    let packaged_discovery = match discover_packaged().await {
        Ok(Some(runtime)) => {
            return ManagedSrtResolution {
                runtime: Some(runtime),
                warning: None,
            };
        }
        other => other,
    };

    if allow_first_use_install {
        return match install().await {
            Ok(runtime) => ManagedSrtResolution {
                runtime: Some(runtime),
                warning: None,
            },
            Err(install_error) => {
                let mut failures = Vec::new();
                if let Err(error) = discovery {
                    failures.push(format!("existing managed state was invalid ({error})"));
                }
                if let Err(error) = packaged_discovery {
                    failures.push(format!("packaged support payload was invalid ({error})"));
                }
                failures.push(format!("registry fallback also failed ({install_error})"));
                let detail = failures.join("; ");
                ManagedSrtResolution {
                    runtime: None,
                    warning: Some(format!(
                        "Local command sandbox setup failed: {detail}. Automatic setup requires Node.js >= 20.11.0 and npm on the trusted host PATH; a global `srt` installation is neither required nor selected. Default mode will ask before exact host Bash execution; Auto mode will deny Bash. Retry `a3s code` while online with automatic installation enabled"
                    )),
                }
            }
        };
    }

    let policy = if offline {
        "first-use setup is disabled in offline mode; start `a3s code` once while online"
    } else {
        "first-use setup is disabled by A3S_NO_AUTO_INSTALL; run `a3s code` once without that setting"
    };
    let mut states = Vec::new();
    match discovery {
        Ok(None) => states.push("managed state is not prepared".to_string()),
        Err(error) => states.push(format!("managed state failed verification ({error})")),
        Ok(Some(_)) => unreachable!("ready managed discovery returned above"),
    }
    match packaged_discovery {
        Ok(None) => states.push("no packaged support payload was found".to_string()),
        Err(error) => states.push(format!(
            "packaged support payload failed verification ({error})"
        )),
        Ok(Some(_)) => unreachable!("ready packaged discovery returned above"),
    }
    ManagedSrtResolution {
        runtime: None,
        warning: Some(format!(
            "Local command sandbox is unavailable because {}; {policy}. First-use registry fallback requires Node.js >= 20.11.0 and npm on the trusted host PATH; a global `srt` installation is neither required nor selected. Default mode will ask before exact host Bash execution; Auto mode will deny Bash",
            states.join(" and ")
        )),
    }
}

async fn discover_managed_srt(
    paths: &ComponentPaths,
    workspace: &Path,
) -> anyhow::Result<Option<ManagedSrtRuntime>> {
    let id = ComponentId::parse(SRT_COMPONENT_ID)?;
    let Some(receipt) = paths.receipt_store().read(id.as_str())? else {
        return Ok(None);
    };
    let executable = validate_managed_receipt(paths, &id, &receipt)?;
    let expected_tree = receipt
        .artifact_checksums
        .get(INSTALL_TREE_KEY)
        .context("managed SRT receipt has no installation-tree digest")?;
    let actual_tree = hash_install_tree(&receipt.install_root)?;
    if &actual_tree != expected_tree {
        bail!(
            "managed SRT installation tree failed integrity verification (expected {}, found {})",
            expected_tree,
            actual_tree
        );
    }
    validate_npm_install(&receipt.install_root)?;
    let node = resolve_and_probe_node(paths, workspace).await?;
    Ok(Some(ManagedSrtRuntime { executable, node }))
}

async fn discover_packaged_srt(
    paths: &ComponentPaths,
    workspace: &Path,
) -> anyhow::Result<Option<ManagedSrtRuntime>> {
    let workspace = workspace
        .canonicalize()
        .context("failed to canonicalize the Code workspace")?;
    let mut invalid = Vec::new();
    for candidate in packaged_srt_candidates(&paths.current_exe) {
        if !candidate.exists() {
            continue;
        }
        let root = match candidate.canonicalize() {
            Ok(root) => root,
            Err(error) => {
                invalid.push(format!(
                    "{} could not be resolved ({error})",
                    candidate.display()
                ));
                continue;
            }
        };
        if root.starts_with(&workspace) {
            invalid.push(format!(
                "{} is inside the active workspace",
                candidate.display()
            ));
            continue;
        }
        match validate_managed_srt_payload(&root) {
            Ok(executable) => {
                let node = resolve_and_probe_node(paths, &workspace).await?;
                return Ok(Some(ManagedSrtRuntime { executable, node }));
            }
            Err(error) => invalid.push(format!("{}: {error}", candidate.display())),
        }
    }
    if invalid.is_empty() {
        Ok(None)
    } else {
        bail!("{}", invalid.join("; "))
    }
}

async fn install_managed_srt(
    paths: &ComponentPaths,
    workspace: &Path,
    progress: bool,
) -> anyhow::Result<ManagedSrtRuntime> {
    let id = ComponentId::parse(SRT_COMPONENT_ID)?;
    let _lock = ComponentOperationLock::acquire(paths.operation_lock_path(&id), &id).await?;

    // A concurrent process may have completed setup while this process waited.
    if let Ok(Some(runtime)) = discover_managed_srt(paths, workspace).await {
        return Ok(runtime);
    }

    let canonical_workspace = workspace
        .canonicalize()
        .context("failed to canonicalize the Code workspace")?;
    let component_root = paths.component_root(&id);
    let active = paths.version_root(&id, MANAGED_SRT_VERSION);
    ensure_managed_root_outside_workspace(&component_root, &canonical_workspace)?;

    let node = resolve_and_probe_node(paths, &canonical_workspace).await?;
    let npm = resolve_host_executable("npm", paths.path_env.as_deref(), &canonical_workspace)
        .context("npm was not found outside the active workspace")?;
    let trusted_path = sanitized_host_path(paths.path_env.as_deref(), &canonical_workspace, &node)?;

    super::progress(
        progress,
        format!(
            "a3s: preparing managed local command sandbox {}...",
            MANAGED_SRT_VERSION
        ),
    );
    std::fs::create_dir_all(&component_root).with_context(|| {
        format!(
            "failed to create managed sandbox directory {}",
            component_root.display()
        )
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(&component_root)
        .context("failed to create managed sandbox staging directory")?;
    let staged_runtime = staging.path().join("runtime");
    std::fs::create_dir_all(&staged_runtime)?;
    write_bootstrap_manifest(&staged_runtime)?;

    let mut command = Command::new(&npm);
    command
        .args([
            "ci",
            "--ignore-scripts",
            "--omit=dev",
            "--no-audit",
            "--no-fund",
            "--engine-strict=true",
            "--registry=https://registry.npmjs.org/",
            "--replace-registry-host=never",
        ])
        .current_dir(&staged_runtime)
        .env("PATH", trusted_path)
        .env("npm_config_ignore_scripts", "true")
        .env("NPM_CONFIG_IGNORE_SCRIPTS", "true")
        .env("npm_config_registry", NPM_REGISTRY)
        .env("NPM_CONFIG_REGISTRY", NPM_REGISTRY)
        .env("npm_config_replace_registry_host", "never")
        .env("NPM_CONFIG_REPLACE_REGISTRY_HOST", "never")
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("LD_PRELOAD")
        .env_remove("DYLD_INSERT_LIBRARIES")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    configure_managed_command(&mut command);
    let child = command
        .spawn()
        .with_context(|| format!("failed to start npm at {}", npm.display()))?;
    let mut child = ManagedInstallChild::attach(child);
    let output = wait_for_managed_install(&mut child, INSTALL_TIMEOUT).await?;
    if !output.success() {
        bail!(
            "npm exited with status {} while installing the fixed SRT package",
            output
        );
    }

    validate_npm_install(&staged_runtime)?;
    let staged_executable = managed_srt_executable(&staged_runtime);
    SrtBashSandbox::from_verified_npm_with_node(&staged_executable, &node, &canonical_workspace)
        .context("staged SRT failed its Core capability handshake")?;
    let tree_digest = hash_install_tree(&staged_runtime)?;

    let activation = DirectoryActivation::activate(&staged_runtime, &active)?;
    let executable = managed_srt_executable(&active);
    let receipt = ComponentReceipt {
        schema_version: RECEIPT_SCHEMA_VERSION,
        component_id: id.to_string(),
        version: MANAGED_SRT_VERSION.to_string(),
        provenance: InstallProvenance::LocalPackage,
        install_root: active.clone(),
        executable_path: Some(executable.clone()),
        owned_paths: vec![active],
        source: Some(SRT_SOURCE.to_string()),
        artifact_checksums: BTreeMap::from([
            (
                PACKAGE_INTEGRITY_KEY.to_string(),
                SRT_PACKAGE_INTEGRITY.to_string(),
            ),
            (INSTALL_TREE_KEY.to_string(), tree_digest),
        ]),
        installed_at: chrono::Utc::now().to_rfc3339(),
    };
    paths.receipt_store().write(&receipt)?;
    activation.commit()?;

    Ok(ManagedSrtRuntime { executable, node })
}

fn validate_managed_receipt(
    paths: &ComponentPaths,
    id: &ComponentId,
    receipt: &ComponentReceipt,
) -> anyhow::Result<PathBuf> {
    let expected_root = paths.version_root(id, MANAGED_SRT_VERSION);
    let expected_executable = managed_srt_executable(&expected_root);
    if receipt.version != MANAGED_SRT_VERSION {
        bail!(
            "managed SRT receipt has version {}, expected {}",
            receipt.version,
            MANAGED_SRT_VERSION
        );
    }
    if receipt.provenance != InstallProvenance::LocalPackage {
        bail!("managed SRT receipt has unsupported provenance");
    }
    if receipt.install_root != expected_root {
        bail!(
            "managed SRT receipt points at {}, expected {}",
            receipt.install_root.display(),
            expected_root.display()
        );
    }
    if receipt.executable_path.as_deref() != Some(expected_executable.as_path()) {
        bail!("managed SRT receipt does not identify the expected CLI");
    }
    if receipt.owned_paths != [expected_root] {
        bail!("managed SRT receipt has an invalid ownership boundary");
    }
    if receipt.source.as_deref() != Some(SRT_SOURCE) {
        bail!("managed SRT receipt has an unexpected package source");
    }
    if receipt
        .artifact_checksums
        .get(PACKAGE_INTEGRITY_KEY)
        .map(String::as_str)
        != Some(SRT_PACKAGE_INTEGRITY)
    {
        bail!("managed SRT receipt has an unexpected package integrity");
    }
    Ok(expected_executable)
}

fn validate_npm_install(root: &Path) -> anyhow::Result<()> {
    let package = root
        .join("node_modules")
        .join("@anthropic-ai")
        .join("sandbox-runtime");
    let manifest_path = package.join("package.json");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    if manifest.get("name").and_then(serde_json::Value::as_str) != Some(SRT_NPM_PACKAGE_NAME) {
        bail!("managed SRT package identity is invalid");
    }
    if manifest.get("version").and_then(serde_json::Value::as_str) != Some(MANAGED_SRT_VERSION) {
        bail!(
            "managed SRT package version is invalid; expected {}",
            MANAGED_SRT_VERSION
        );
    }
    if !managed_srt_executable(root).is_file() {
        bail!("managed SRT package has no CLI");
    }

    let lock_path = root.join("package-lock.json");
    let lock: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&lock_path)
            .with_context(|| format!("failed to read {}", lock_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", lock_path.display()))?;
    if lock.get("name").and_then(serde_json::Value::as_str) != Some("a3s-code-managed-srt")
        || lock.get("version").and_then(serde_json::Value::as_str) != Some("1.0.0")
        || lock
            .get("lockfileVersion")
            .and_then(serde_json::Value::as_u64)
            != Some(3)
    {
        bail!("managed SRT lock file header is invalid");
    }
    let packages = lock
        .get("packages")
        .and_then(serde_json::Value::as_object)
        .context("managed SRT lock file has no package map")?;
    let expected_package_keys = std::iter::once("")
        .chain(LOCKED_NPM_PACKAGES.iter().map(|package| package.path))
        .collect::<BTreeSet<_>>();
    let actual_package_keys = packages.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual_package_keys != expected_package_keys {
        bail!("managed SRT lock file contains an unexpected dependency graph");
    }
    let root_dependency = packages
        .get("")
        .and_then(|root| root.get("dependencies"))
        .and_then(|dependencies| dependencies.get(SRT_NPM_PACKAGE_NAME))
        .and_then(serde_json::Value::as_str);
    if root_dependency != Some(MANAGED_SRT_VERSION) {
        bail!("managed SRT lock file does not pin the requested package exactly");
    }
    for expected in LOCKED_NPM_PACKAGES {
        let package = packages.get(expected.path).with_context(|| {
            format!("managed SRT lock file has no record for {}", expected.path)
        })?;
        if package.get("version").and_then(serde_json::Value::as_str) != Some(expected.version) {
            bail!(
                "managed SRT lock file contains an unexpected version for {}",
                expected.path
            );
        }
        if package.get("integrity").and_then(serde_json::Value::as_str) != Some(expected.integrity)
        {
            bail!(
                "managed SRT dependency {} failed registry integrity verification",
                expected.path
            );
        }
        if package.get("resolved").and_then(serde_json::Value::as_str) != Some(expected.resolved) {
            bail!(
                "managed SRT dependency {} has an unexpected registry source",
                expected.path
            );
        }
        if !root.join(expected.path).is_dir() {
            bail!(
                "managed SRT dependency {} is missing from the installed tree",
                expected.path
            );
        }
    }
    Ok(())
}

fn write_bootstrap_manifest(root: &Path) -> anyhow::Result<()> {
    std::fs::write(root.join("package.json"), MANAGED_PACKAGE_JSON)
        .context("failed to write managed SRT bootstrap manifest")?;
    std::fs::write(root.join("package-lock.json"), MANAGED_PACKAGE_LOCK)
        .context("failed to write managed SRT dependency lock")
}

fn managed_srt_executable(root: &Path) -> PathBuf {
    root.join("node_modules")
        .join("@anthropic-ai")
        .join("sandbox-runtime")
        .join("dist")
        .join("cli.js")
}

fn packaged_srt_candidates(current_exe: &Path) -> Vec<PathBuf> {
    let executable = current_exe
        .canonicalize()
        .unwrap_or_else(|_| current_exe.to_path_buf());
    let Some(binary_directory) = executable.parent() else {
        return Vec::new();
    };
    let mut candidates = vec![binary_directory.join(MANAGED_SRT_PAYLOAD_RELATIVE_ROOT)];
    if let Some(prefix) = binary_directory.parent() {
        candidates.push(
            prefix
                .join("share")
                .join("a3s")
                .join(MANAGED_SRT_PAYLOAD_RELATIVE_ROOT),
        );
    }
    candidates.dedup();
    candidates
}

/// Verify the immutable SRT support tree shipped with a CLI release.
///
/// Release installers use the same verifier before preserving the support
/// tree, and Code repeats it before every use. The payload must contain only
/// regular files and directories and must match the digest compiled into the
/// CLI.
#[doc(hidden)]
pub fn validate_managed_srt_payload(root: &Path) -> anyhow::Result<PathBuf> {
    let expected = PACKAGED_SRT_TREE_SHA256.trim();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("compiled managed SRT tree digest is invalid");
    }
    let actual = hash_packaged_install_tree(root)?;
    if actual != expected {
        bail!(
            "packaged managed SRT tree failed integrity verification (expected {}, found {})",
            expected,
            actual
        );
    }
    validate_npm_install(root)?;
    let executable = managed_srt_executable(root)
        .canonicalize()
        .context("failed to resolve the packaged managed SRT CLI")?;
    if !is_executable(&executable) {
        bail!(
            "packaged managed SRT CLI is not executable: {}",
            executable.display()
        );
    }
    Ok(executable)
}

async fn resolve_and_probe_node(
    paths: &ComponentPaths,
    workspace: &Path,
) -> anyhow::Result<PathBuf> {
    let node = resolve_host_executable("node", paths.path_env.as_deref(), workspace)
        .context("Node.js was not found outside the active workspace")?;
    let mut command = Command::new(&node);
    command
        .arg("--version")
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_managed_command(&mut command);
    let child = command
        .spawn()
        .with_context(|| format!("failed to run Node.js at {}", node.display()))?;
    let mut process_group = ManagedCommandProcessGroup::attach(&child);
    let output = match tokio::time::timeout(NODE_PROBE_TIMEOUT, child.wait_with_output()).await {
        Ok(output) => {
            process_group.terminate();
            output.with_context(|| format!("failed to run Node.js at {}", node.display()))?
        }
        Err(_) => {
            process_group.terminate();
            bail!("Node.js version probe timed out");
        }
    };
    if !output.status.success() {
        bail!("Node.js version probe exited with {}", output.status);
    }
    let version = String::from_utf8(output.stdout)
        .context("Node.js version output was not UTF-8")?
        .trim()
        .to_string();
    let parsed = parse_semver_triplet(&version)
        .with_context(|| format!("unsupported Node.js version output: {version}"))?;
    if parsed < MINIMUM_NODE_VERSION {
        bail!(
            "Node.js {version} is too old for managed SRT; expected >= {}.{}.{}",
            MINIMUM_NODE_VERSION.0,
            MINIMUM_NODE_VERSION.1,
            MINIMUM_NODE_VERSION.2
        );
    }
    Ok(node)
}

fn parse_semver_triplet(version: &str) -> Option<(u64, u64, u64)> {
    let core = version
        .trim()
        .strip_prefix('v')
        .unwrap_or(version.trim())
        .split(['-', '+'])
        .next()?;
    let mut parts = core.split('.');
    let parsed = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(parsed)
}

fn resolve_host_executable(
    binary: &str,
    path_env: Option<&OsStr>,
    excluded_root: &Path,
) -> anyhow::Result<PathBuf> {
    let path_env = path_env.context("PATH is not set")?;
    for directory in std::env::split_paths(path_env) {
        if !directory.is_absolute() {
            continue;
        }
        for name in host_executable_names(binary) {
            let candidate = directory.join(name);
            let Ok(canonical) = candidate.canonicalize() else {
                continue;
            };
            if canonical.starts_with(excluded_root)
                || !canonical.is_file()
                || !is_executable(&canonical)
            {
                continue;
            }
            return Ok(canonical);
        }
    }
    bail!("{binary} was not found on the trusted host PATH")
}

fn host_executable_names(binary: &str) -> Vec<OsString> {
    #[cfg(windows)]
    {
        [".exe", ".cmd", ".bat"]
            .into_iter()
            .map(|extension| OsString::from(format!("{binary}{extension}")))
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![OsString::from(binary)]
    }
}

fn ensure_managed_root_outside_workspace(
    component_root: &Path,
    workspace: &Path,
) -> anyhow::Result<()> {
    let workspace = workspace
        .canonicalize()
        .context("failed to canonicalize the workspace boundary")?;
    let mut existing = component_root;
    while !existing.exists() {
        existing = existing
            .parent()
            .context("managed sandbox path has no existing ancestor")?;
    }
    let resolved = existing.canonicalize().with_context(|| {
        format!(
            "failed to resolve managed sandbox parent {}",
            existing.display()
        )
    })?;
    if resolved.starts_with(&workspace) {
        bail!(
            "A3S_DATA_HOME places the managed sandbox inside the active workspace; choose a user-level data directory"
        );
    }
    Ok(())
}

fn sanitized_host_path(
    path_env: Option<&OsStr>,
    excluded_root: &Path,
    node: &Path,
) -> anyhow::Result<OsString> {
    let mut directories = vec![node
        .parent()
        .context("Node.js executable has no parent directory")?
        .to_path_buf()];
    if let Some(path_env) = path_env {
        for directory in std::env::split_paths(path_env) {
            if !directory.is_absolute() {
                continue;
            }
            let Ok(canonical) = directory.canonicalize() else {
                continue;
            };
            if canonical.starts_with(excluded_root) || directories.contains(&canonical) {
                continue;
            }
            directories.push(canonical);
        }
    }
    std::env::join_paths(directories).context("failed to compose trusted host PATH")
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests;
