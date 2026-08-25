use super::*;

#[derive(Clone, Copy)]
enum ExecCapabilityRuntime {
    InstalledOnly,
    Required,
}

fn run_code_exec(
    workspace: &TempWorkspace,
    release: &FakeReleaseServer,
    policy: FirstUsePolicy,
    capability_runtime: ExecCapabilityRuntime,
    request_skill_search: bool,
) -> (std::process::Output, FakeOpenAi) {
    let project = workspace.path("project");
    std::fs::create_dir_all(&project).expect("create test project");
    let llm = FakeOpenAi::start(request_skill_search);
    let config = write_config(&project, &llm.base_url);
    let mut command = Command::new(a3s_bin());
    command
        .args(["--output", "jsonl", "--non-interactive", "--directory"])
        .arg(&project)
        .arg("--config")
        .arg(&config)
        .args(["code", "exec"]);
    if matches!(capability_runtime, ExecCapabilityRuntime::Required) {
        command.args(["--capability-runtime", "scoped-v1"]);
    }
    command
        .args([
            "--mode",
            "auto",
            "Report whether the verified OCR Skill supplied by A3S Use is available.",
        ])
        .env("HOME", workspace.path("home"))
        .env("A3S_DATA_HOME", workspace.path("data"))
        .env("A3S_STATE_HOME", workspace.path("state"))
        .env("A3S_CACHE_HOME", workspace.path("cache"))
        .env("A3S_RUNTIME_HOME", workspace.path("runtime"))
        .env("A3S_USE_E2E_MCP_MARKER", workspace.path("mcp-started"))
        .env("A3S_USE_E2E_WATCH_PID", workspace.path("watch.pid"))
        .env("A3S_UPDATER_GITHUB_API_BASE", release.api_base())
        .env_remove("A3S_OFFLINE")
        .env_remove("A3S_NO_AUTO_INSTALL")
        .env("PATH", "/usr/bin:/bin");
    match policy {
        FirstUsePolicy::Online => {}
        FirstUsePolicy::Offline => {
            command.arg("--offline");
        }
        FirstUsePolicy::NoAutoInstall => {
            command.env("A3S_NO_AUTO_INSTALL", "1");
        }
    }
    let output =
        command_output_with_timeout(&mut command, TUI_SMOKE_TIMEOUT, "run scoped Code Exec");
    (output, llm)
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn terminal_result(output: &std::process::Output) -> serde_json::Value {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|document| document["type"] == "result")
        .expect("terminal Code Exec result")
}

fn install_use(workspace: &TempWorkspace, release: &FakeReleaseServer) {
    let (output, _) = run_code_exec(
        workspace,
        release,
        FirstUsePolicy::Online,
        ExecCapabilityRuntime::Required,
        true,
    );
    assert_success(&output);
}

fn recorded_process_exited(path: &Path) -> bool {
    let Ok(pid) = std::fs::read_to_string(path) else {
        return true;
    };
    let Ok(pid) = pid.trim().parse::<libc::pid_t>() else {
        return false;
    };
    for _ in 0..100 {
        let result = unsafe { libc::kill(pid, 0) };
        if result != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn installs_use_and_freezes_atomic_evidence_before_llm_egress() {
    let workspace = TempWorkspace::new("code-exec-scoped-use");
    let release = start_fake_use_release(&workspace);
    let (output, llm) = run_code_exec(
        &workspace,
        &release,
        FirstUsePolicy::Online,
        ExecCapabilityRuntime::Required,
        true,
    );

    assert_success(&output);
    assert!(llm.request_count() > 0, "Code Exec did not call the model");
    assert!(
        llm.saw_atomic_ocr_skill(),
        "the first model Run did not receive the atomic OCR Skill"
    );
    let result = terminal_result(&output);
    let evidence = &result["data"]["capabilityRuntime"];
    assert_eq!(evidence["schema"], "a3s.code.scoped-capability-runtime.v1");
    assert_eq!(evidence["ready"], true);
    assert_eq!(evidence["codeCatalog"]["generation"], 1);
    assert!(evidence["codeCatalog"]["digest"]
        .as_str()
        .is_some_and(|digest| digest.starts_with("sha256:")));
    assert_eq!(
        evidence["useSnapshot"]["schema"],
        "a3s.use.capability-snapshot-cursor.v1"
    );
    assert!(evidence["useSnapshot"]["generation"].is_u64());
    assert!(evidence["useSnapshot"]["revision"].is_string());
    assert!(evidence["useSnapshot"]["registryRevision"].is_string());
    assert!(evidence["skillCount"]
        .as_u64()
        .is_some_and(|count| count > 0));
    assert_eq!(evidence["runtimeTasks"]["count"], 0);
    assert!(evidence["runtimeTasks"]["digest"]
        .as_str()
        .is_some_and(|digest| digest.starts_with("sha256:")));
    assert!(
        !workspace.path("mcp-started").exists(),
        "HOST-CAP1 must not start asynchronous MCP compatibility surfaces"
    );
    assert!(
        recorded_process_exited(&workspace.path("watch.pid")),
        "the frozen one-shot runtime left its A3S Use watcher running"
    );
}

#[test]
fn installed_only_mode_does_not_install_a_missing_use_runtime() {
    let workspace = TempWorkspace::new("code-exec-installed-use-missing");
    let release = start_fake_use_release(&workspace);
    let (output, llm) = run_code_exec(
        &workspace,
        &release,
        FirstUsePolicy::Online,
        ExecCapabilityRuntime::InstalledOnly,
        false,
    );

    assert_success(&output);
    assert!(llm.request_count() > 0, "Code Exec did not call the model");
    assert!(!workspace.path("state/components/use.json").exists());
    assert!(
        release.requests().is_empty(),
        "ordinary Code Exec contacted the component release service"
    );
    assert!(terminal_result(&output)["data"]["capabilityRuntime"].is_null());
}

#[test]
fn installed_only_mode_reuses_use_without_network_mutation() {
    let workspace = TempWorkspace::new("code-exec-installed-use-ready");
    let release = start_fake_use_release(&workspace);
    install_use(&workspace, &release);
    let release_requests = release.requests().len();

    let (output, llm) = run_code_exec(
        &workspace,
        &release,
        FirstUsePolicy::Offline,
        ExecCapabilityRuntime::InstalledOnly,
        true,
    );
    assert_success(&output);
    assert!(llm.saw_atomic_ocr_skill());
    assert_eq!(
        release.requests().len(),
        release_requests,
        "installed-only Code Exec performed release network I/O"
    );
    assert_eq!(
        terminal_result(&output)["data"]["capabilityRuntime"]["schema"],
        "a3s.code.scoped-capability-runtime.v1"
    );
}

#[test]
fn installed_only_mode_skips_incompatible_use_before_run_admission() {
    let workspace = TempWorkspace::new("code-exec-installed-use-incompatible");
    let release = start_fake_use_release(&workspace);
    install_use(&workspace, &release);
    let receipt: serde_json::Value = serde_json::from_slice(
        &std::fs::read(workspace.path("state/components/use.json")).unwrap(),
    )
    .unwrap();
    let executable = PathBuf::from(receipt["executablePath"].as_str().unwrap());
    let compatible = std::fs::read_to_string(&executable).unwrap();
    let incompatible = compatible.replace(
        "\\\"registry\\\":{\\\"schemaVersion\\\":2",
        "\\\"registry\\\":{\\\"schemaVersion\\\":1",
    );
    assert_ne!(incompatible, compatible, "fake Use schema was not changed");
    std::fs::write(&executable, incompatible).unwrap();
    let release_requests = release.requests().len();

    let (output, llm) = run_code_exec(
        &workspace,
        &release,
        FirstUsePolicy::Offline,
        ExecCapabilityRuntime::InstalledOnly,
        false,
    );
    assert_success(&output);
    assert!(
        llm.request_count() > 0,
        "fallback execution did not reach the model"
    );
    assert!(!llm.saw_atomic_ocr_skill());
    assert_eq!(release.requests().len(), release_requests);
    assert!(terminal_result(&output)["data"]["capabilityRuntime"].is_null());
    assert!(!workspace.path("mcp-started").exists());
    assert!(recorded_process_exited(&workspace.path("watch.pid")));
}

#[test]
fn required_mode_fails_before_llm_when_first_use_is_forbidden() {
    for (name, policy) in [
        ("offline", FirstUsePolicy::Offline),
        ("no-auto-install", FirstUsePolicy::NoAutoInstall),
    ] {
        let workspace = TempWorkspace::new(&format!("code-exec-scoped-{name}"));
        let release = start_fake_use_release(&workspace);
        let (output, llm) = run_code_exec(
            &workspace,
            &release,
            policy,
            ExecCapabilityRuntime::Required,
            true,
        );

        assert!(!output.status.success(), "{name}: runtime must fail closed");
        assert_eq!(llm.request_count(), 0, "{name}: model egress occurred");
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["type"], "error");
        assert_eq!(error["error"]["code"], "capability-runtime.unavailable");
        assert_eq!(error["error"]["details"]["ready"], false);
        assert!(
            release.requests().is_empty(),
            "{name}: release network was used"
        );
        assert!(!workspace.path("state/components/use.json").exists());
        assert!(!workspace.path("mcp-started").exists());
    }
}
