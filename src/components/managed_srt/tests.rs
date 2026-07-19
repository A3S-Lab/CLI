use a3s_code_core::sandbox::BashSandbox;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

fn runtime(name: &str) -> ManagedSrtRuntime {
    ManagedSrtRuntime {
        executable: PathBuf::from(format!("/managed/{name}/cli.js")),
        node: PathBuf::from("/host/node"),
    }
}

#[tokio::test]
async fn unavailable_runtime_preserves_the_resolution_warning() {
    let workspace = tempfile::tempdir().unwrap();
    let resolution = ManagedSrtResolution {
        runtime: None,
        warning: Some("managed sandbox unavailable".to_string()),
    }
    .into_probed_bash_sandbox(workspace.path())
    .await;

    assert!(resolution.sandbox.is_none());
    assert_eq!(
        resolution.warning.as_deref(),
        Some("managed sandbox unavailable")
    );
}

#[tokio::test]
async fn failed_runtime_probe_never_returns_an_unprobed_sandbox() {
    let workspace = tempfile::tempdir().unwrap();
    let resolution = ManagedSrtResolution {
        runtime: Some(runtime("missing")),
        warning: None,
    }
    .into_probed_bash_sandbox(workspace.path())
    .await;

    assert!(resolution.sandbox.is_none());
    let warning = resolution
        .warning
        .expect("probe failure must be actionable");
    assert!(warning.contains("failed its bounded OS capability probe"));
    assert!(warning.contains("Auto mode will deny Bash"));
}

#[tokio::test]
async fn existing_verified_install_is_reused_without_preparation() {
    let install_called = AtomicBool::new(false);
    let expected = runtime("existing");

    let resolution = resolve_managed_srt_with(
        true,
        false,
        || async { Ok(Some(expected.clone())) },
        || async { anyhow::bail!("packaged discovery must not run") },
        || async {
            install_called.store(true, Ordering::SeqCst);
            anyhow::bail!("installer must not run")
        },
    )
    .await;

    assert_eq!(resolution.runtime, Some(expected));
    assert!(resolution.warning.is_none());
    assert!(!install_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn missing_install_is_prepared_once_when_policy_allows_it() {
    let install_called = AtomicBool::new(false);
    let expected = runtime("prepared");

    let resolution = resolve_managed_srt_with(
        true,
        false,
        || async { Ok(None) },
        || async { Ok(None) },
        || async {
            assert!(!install_called.swap(true, Ordering::SeqCst));
            Ok(expected.clone())
        },
    )
    .await;

    assert_eq!(resolution.runtime, Some(expected));
    assert!(resolution.warning.is_none());
    assert!(install_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn packaged_runtime_is_available_offline_without_mutation() {
    let install_called = AtomicBool::new(false);
    let expected = runtime("packaged");

    let resolution = resolve_managed_srt_with(
        false,
        true,
        || async { Ok(None) },
        || async { Ok(Some(expected.clone())) },
        || async {
            install_called.store(true, Ordering::SeqCst);
            anyhow::bail!("installer must not run")
        },
    )
    .await;

    assert_eq!(resolution.runtime, Some(expected));
    assert!(resolution.warning.is_none());
    assert!(!install_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn invalid_managed_state_falls_back_to_the_packaged_runtime() {
    let expected = runtime("packaged");

    let resolution = resolve_managed_srt_with(
        false,
        true,
        || async { anyhow::bail!("receipt was tampered") },
        || async { Ok(Some(expected.clone())) },
        || async { anyhow::bail!("installer must not run") },
    )
    .await;

    assert_eq!(resolution.runtime, Some(expected));
    assert!(resolution.warning.is_none());
}

#[tokio::test]
async fn offline_and_no_auto_install_are_strict_no_mutation_boundaries() {
    for (offline, expected) in [(true, "offline mode"), (false, "A3S_NO_AUTO_INSTALL")] {
        let install_called = AtomicBool::new(false);
        let resolution = resolve_managed_srt_with(
            false,
            offline,
            || async { Ok(None) },
            || async { Ok(None) },
            || async {
                install_called.store(true, Ordering::SeqCst);
                anyhow::bail!("installer must not run")
            },
        )
        .await;

        assert!(resolution.runtime.is_none());
        assert!(!install_called.load(Ordering::SeqCst));
        let warning = resolution.warning.unwrap();
        assert!(warning.contains(expected), "{warning}");
        assert!(warning.contains("Auto mode will deny Bash"), "{warning}");
    }
}

#[tokio::test]
async fn failed_preparation_is_non_fatal_and_actionable() {
    let resolution = resolve_managed_srt_with(
        true,
        false,
        || async { Ok(None) },
        || async { Ok(None) },
        || async { anyhow::bail!("registry unavailable") },
    )
    .await;

    assert!(resolution.runtime.is_none());
    let warning = resolution.warning.unwrap();
    assert!(warning.contains("registry unavailable"), "{warning}");
    assert!(warning.contains("Node.js >= 20.11.0"), "{warning}");
    assert!(warning.contains("npm"), "{warning}");
    assert!(
        warning.contains("global `srt` installation is neither required nor selected"),
        "{warning}"
    );
    assert!(warning.contains("Retry `a3s code`"), "{warning}");
    assert!(warning.contains("Auto mode will deny Bash"), "{warning}");
}

#[test]
fn package_identity_version_and_integrity_are_all_required() {
    let root = tempfile::tempdir().unwrap();
    write_install_fixture(root.path(), SRT_NPM_PACKAGE_NAME, MANAGED_SRT_VERSION);
    validate_npm_install(root.path()).unwrap();

    write_install_fixture(root.path(), "@example/not-srt", MANAGED_SRT_VERSION);
    assert!(validate_npm_install(root.path())
        .unwrap_err()
        .to_string()
        .contains("identity"));

    write_install_fixture(root.path(), SRT_NPM_PACKAGE_NAME, "0.0.65");
    assert!(validate_npm_install(root.path())
        .unwrap_err()
        .to_string()
        .contains("version"));

    write_install_fixture(root.path(), SRT_NPM_PACKAGE_NAME, MANAGED_SRT_VERSION);
    let lock_path = root.path().join("package-lock.json");
    let mut lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock["packages"]["node_modules/node-forge"]["integrity"] =
        serde_json::Value::String("sha512-unexpected".to_string());
    std::fs::write(&lock_path, serde_json::to_vec(&lock).unwrap()).unwrap();
    let error = validate_npm_install(root.path()).unwrap_err().to_string();
    assert!(error.contains("node-forge"), "{error}");
    assert!(error.contains("integrity"), "{error}");
}

#[test]
fn installation_tree_digest_detects_content_changes() {
    let root = tempfile::tempdir().unwrap();
    write_install_fixture(root.path(), SRT_NPM_PACKAGE_NAME, MANAGED_SRT_VERSION);
    let before = hash_install_tree(root.path()).unwrap();
    std::fs::write(managed_srt_executable(root.path()), "tampered").unwrap();
    let after = hash_install_tree(root.path()).unwrap();
    assert_ne!(before, after);
}

#[test]
fn bootstrap_files_are_the_release_payload_source_of_truth() {
    let root = tempfile::tempdir().unwrap();
    write_bootstrap_manifest(root.path()).unwrap();

    assert_eq!(
        std::fs::read(root.path().join("package.json")).unwrap(),
        MANAGED_PACKAGE_JSON
    );
    assert_eq!(
        std::fs::read(root.path().join("package-lock.json")).unwrap(),
        MANAGED_PACKAGE_LOCK
    );
}

#[cfg(unix)]
#[test]
fn packaged_tree_rejects_symbolic_and_hard_links() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    write_install_fixture(root.path(), SRT_NPM_PACKAGE_NAME, MANAGED_SRT_VERSION);
    symlink(
        "../@anthropic-ai/sandbox-runtime/dist/cli.js",
        root.path().join("node_modules/srt-link"),
    )
    .unwrap();

    let error = hash_packaged_install_tree(root.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("symbolic link"), "{error}");

    std::fs::remove_file(root.path().join("node_modules/srt-link")).unwrap();
    std::fs::hard_link(
        managed_srt_executable(root.path()),
        root.path().join("node_modules/srt-hardlink"),
    )
    .unwrap();
    let error = hash_packaged_install_tree(root.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("hard-linked"), "{error}");
}

#[test]
fn node_version_boundary_is_explicit() {
    assert_eq!(parse_semver_triplet("v20.11.0"), Some((20, 11, 0)));
    assert!(parse_semver_triplet("v20.10.9").unwrap() < MINIMUM_NODE_VERSION);
    assert!(parse_semver_triplet("v24.0.0").unwrap() >= MINIMUM_NODE_VERSION);
    assert_eq!(parse_semver_triplet("unknown"), None);
}

#[test]
fn managed_component_root_cannot_resolve_inside_the_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let inside = workspace.path().join("data/components/code/srt");
    assert!(ensure_managed_root_outside_workspace(&inside, workspace.path()).is_err());

    let outside = tempfile::tempdir().unwrap();
    ensure_managed_root_outside_workspace(
        &outside.path().join("data/components/code/srt"),
        workspace.path(),
    )
    .unwrap();
}

#[test]
fn packaged_payload_discovery_covers_archive_and_prefix_layouts() {
    let candidates = packaged_srt_candidates(Path::new("release-prefix/bin/a3s"));

    assert_eq!(
        candidates,
        vec![
            PathBuf::from("release-prefix/bin/support/managed-srt"),
            PathBuf::from("release-prefix/share/a3s/support/managed-srt"),
        ]
    );
}

/// Run explicitly to verify registry bootstrap, activation, reuse, and the
/// OS-enforced command path.
#[tokio::test]
#[ignore = "requires network access, npm, Node.js, and the host SRT boundary"]
async fn real_managed_first_use_installs_once_and_runs_without_host_fallback() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut paths = ComponentPaths::for_test(root.path());
    paths.path_env = std::env::var_os("PATH");

    let first = resolve_managed_srt(&paths, &workspace, true, false, false).await;
    assert!(first.warning.is_none(), "{:?}", first.warning);
    let first = first.runtime.expect("prepared managed SRT");
    let receipt = paths
        .receipt_store()
        .read(SRT_COMPONENT_ID)
        .unwrap()
        .expect("managed SRT receipt");
    assert_eq!(receipt.version, MANAGED_SRT_VERSION);

    let sandbox = first.build_sandbox(&workspace).unwrap();
    let output = sandbox
        .exec_command("printf managed-srt", "/workspace")
        .await
        .unwrap();
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, "managed-srt");

    let second = resolve_managed_srt(&paths, &workspace, false, true, false).await;
    assert!(second.warning.is_none(), "{:?}", second.warning);
    assert_eq!(
        second.runtime.unwrap().executable(),
        first.executable(),
        "offline startup must reuse the exact verified installation"
    );
}

/// Run explicitly after preparing `support/managed-srt/node_modules`.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires the prepared release payload, Node.js, and the host SRT boundary"]
async fn real_packaged_payload_enforces_complete_local_command_policy() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    for directory in [
        workspace.as_path(),
        &workspace.join(".git"),
        &workspace.join(".a3s"),
        &workspace.join(".claude"),
        &workspace.join(".codex"),
        &workspace.join("services/api"),
        outside.as_path(),
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }
    std::fs::write(workspace.join("visible.txt"), "workspace-visible").unwrap();
    std::fs::write(workspace.join(".env"), "WORKSPACE_SECRET=hidden").unwrap();
    std::fs::write(
        workspace.join("services/api/.env"),
        "NESTED_WORKSPACE_SECRET=hidden",
    )
    .unwrap();
    std::fs::write(workspace.join(".codex/auth.json"), "workspace-auth").unwrap();
    std::fs::write(workspace.join(".git/config"), "original-git").unwrap();
    std::fs::write(workspace.join(".a3s/policy.acl"), "original-policy").unwrap();
    std::fs::write(
        workspace.join(".claude/settings.json"),
        "original-agent-settings",
    )
    .unwrap();
    std::fs::write(outside.join("secret.txt"), "outside-hidden").unwrap();
    std::fs::hard_link(
        outside.join("secret.txt"),
        workspace.join("outside-hardlink"),
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, workspace.join("outside-link")).unwrap();

    let mut paths = ComponentPaths::for_test(root.path());
    paths.path_env = std::env::var_os("PATH");
    paths.current_exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("a3s-release-fixture");
    let resolution = resolve_managed_srt(&paths, &workspace, false, true, false).await;
    assert!(resolution.warning.is_none(), "{:?}", resolution.warning);
    let runtime = resolution.runtime.expect("packaged managed SRT");
    assert!(paths
        .receipt_store()
        .read(SRT_COMPONENT_ID)
        .unwrap()
        .is_none());
    let sandbox = runtime.build_and_probe_sandbox(&workspace).await.unwrap();

    let allowed = sandbox
        .exec_command(
            "printf packaged > allowed.txt && cat visible.txt",
            "/workspace",
        )
        .await
        .unwrap();
    assert_eq!(
        allowed.exit_code, 0,
        "ordinary workspace access failed: {}{}",
        allowed.stdout, allowed.stderr
    );
    assert_eq!(allowed.stdout, "workspace-visible");
    assert_eq!(
        std::fs::read_to_string(workspace.join("allowed.txt")).unwrap(),
        "packaged"
    );

    assert_sandbox_denies(
        &sandbox,
        "printf forbidden > ../outside.txt",
        "outside write",
    )
    .await;
    assert_sandbox_denies(
        &sandbox,
        "printf forbidden > outside-link/symlink-escaped.txt",
        "symlink write escape",
    )
    .await;

    for (command, path, original) in [
        (
            "printf changed > .git/config",
            ".git/config",
            "original-git",
        ),
        (
            "printf changed > .a3s/policy.acl",
            ".a3s/policy.acl",
            "original-policy",
        ),
        (
            "printf changed > .claude/settings.json",
            ".claude/settings.json",
            "original-agent-settings",
        ),
    ] {
        assert_sandbox_denies(&sandbox, command, path).await;
        assert_eq!(
            std::fs::read_to_string(workspace.join(path)).unwrap(),
            original
        );
    }

    for (command, secret) in [
        ("cat .env", "WORKSPACE_SECRET=hidden"),
        ("cat services/api/.env", "NESTED_WORKSPACE_SECRET=hidden"),
        ("cat .codex/auth.json", "workspace-auth"),
        ("cat outside-hardlink", "outside-hidden"),
        ("cat ../outside/secret.txt", "outside-hidden"),
    ] {
        let denied = assert_sandbox_denies(&sandbox, command, "credential read").await;
        assert!(!denied.stdout.contains(secret));
    }

    assert_sandbox_denies(
        &sandbox,
        "node -e 'const net=require(\"net\");\
         const socket=net.connect({host:\"1.1.1.1\",port:443});\
         const done=code=>{socket.destroy();process.exit(code)};\
         socket.once(\"connect\",()=>done(0));\
         socket.once(\"error\",()=>done(7));\
         setTimeout(()=>done(8),2000)'",
        "network egress",
    )
    .await;
    assert_sandbox_denies(
        &sandbox,
        "node -e 'const net=require(\"net\");\
         const server=net.createServer();\
         server.once(\"error\",()=>process.exit(7));\
         server.listen(0,\"127.0.0.1\",()=>server.close(()=>process.exit(0)));\
         setTimeout(()=>process.exit(8),2000)'",
        "local listener",
    )
    .await;
}

#[cfg(unix)]
async fn assert_sandbox_denies(
    sandbox: &impl BashSandbox,
    command: &str,
    boundary: &str,
) -> a3s_code_core::sandbox::SandboxOutput {
    let output = sandbox.exec_command(command, "/workspace").await.unwrap();
    assert_ne!(
        output.exit_code, 0,
        "{boundary} unexpectedly succeeded for `{command}`: {}{}",
        output.stdout, output.stderr
    );
    output
}

#[cfg(unix)]
fn spawn_installer_fixture(directory: &Path, leak_name: &str) -> ManagedInstallChild {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!(
            "touch started; (sleep 0.30; touch {leak_name}) & wait"
        ))
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    configure_managed_command(&mut command);
    ManagedInstallChild::attach(command.spawn().unwrap())
}

#[cfg(unix)]
async fn wait_for_fixture_start(directory: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !directory.join("started").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("installer fixture did not start");
}

#[cfg(unix)]
#[tokio::test]
async fn managed_install_timeout_kills_every_installer_descendant() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = spawn_installer_fixture(directory.path(), "timeout-leak");
    wait_for_fixture_start(directory.path()).await;

    let error = wait_for_managed_install(&mut child, Duration::from_millis(50))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("did not finish"));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!directory.path().join("timeout-leak").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_managed_install_kills_every_installer_descendant() {
    let directory = tempfile::tempdir().unwrap();
    let mut child = spawn_installer_fixture(directory.path(), "cancellation-leak");
    let install =
        tokio::spawn(
            async move { wait_for_managed_install(&mut child, Duration::from_secs(5)).await },
        );
    wait_for_fixture_start(directory.path()).await;

    install.abort();
    assert!(install.await.unwrap_err().is_cancelled());
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!directory.path().join("cancellation-leak").exists());
}

fn write_install_fixture(root: &Path, name: &str, version: &str) {
    write_bootstrap_manifest(root).unwrap();
    for package in LOCKED_NPM_PACKAGES {
        std::fs::create_dir_all(root.join(package.path)).unwrap();
    }
    let package = root
        .join("node_modules")
        .join("@anthropic-ai")
        .join("sandbox-runtime");
    let cli = package.join("dist/cli.js");
    std::fs::create_dir_all(cli.parent().unwrap()).unwrap();
    std::fs::write(
        package.join("package.json"),
        serde_json::json!({
            "name": name,
            "version": version,
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(cli, "#!/usr/bin/env node\n").unwrap();
}
