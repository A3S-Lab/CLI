#![cfg(unix)]

mod support;

use std::process::Command;

use support::{a3s_bin, configure_component_env, make_executable, sh_quote, TempWorkspace};

#[test]
fn list_json_separates_catalog_components_from_external_tools() {
    let temp = TempWorkspace::new("component-list");
    let bin = temp.path("bin");
    let marker = temp.path("unknown-ran");
    make_executable(
        &bin.join("a3s-use"),
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'a3s-use 0.1.0\\n'; exit 0; fi\nif [ \"$1\" = \"component\" ] && [ \"$2\" = \"status\" ]; then printf '{\"component\":{\"id\":\"%s\",\"presence\":\"missing\",\"health\":\"unknown\"}}\\n' \"$3\"; exit 0; fi\nexit 2\n",
    );
    make_executable(
        &bin.join("a3s-local-tool"),
        &format!("#!/bin/sh\nprintf ran > {}\n", sh_quote(&marker)),
    );
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args(["list", "--json"])
        .env("PATH", &bin)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["command"], "component.list");
    let components = report["data"]["components"].as_array().unwrap();
    let use_component = components
        .iter()
        .find(|component| component["id"] == "use")
        .unwrap();
    assert_eq!(use_component["presence"], "external");
    assert_eq!(use_component["health"], "ready");
    assert!(components
        .iter()
        .any(|component| component["id"] == "use/browser"));
    assert_eq!(report["data"]["externalTools"][0]["command"], "local-tool");
    assert!(
        !marker.exists(),
        "listing must not execute unknown PATH tools"
    );
}

#[test]
fn install_without_components_lists_the_typed_catalog_without_mutation() {
    let temp = TempWorkspace::new("component-available");
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command.args(["install", "--json"]).output().unwrap();

    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let ids = report["data"]["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|component| component["id"].as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"use"));
    assert!(ids.contains(&"use/browser"));
    assert!(ids.contains(&"use/office"));
    assert!(!temp.path("state/components").exists());
}

#[test]
fn unsafe_component_ids_fail_at_the_parser_boundary() {
    let temp = TempWorkspace::new("unsafe-component-id");
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args(["--output", "json", "install", "use/../box"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["command"], "a3s");
    assert_eq!(error["error"]["code"], "usage.invalid");
    assert!(!temp.path("state/components").exists());
    assert!(!temp.path("data/components").exists());
}

#[test]
fn install_dry_run_plans_without_network_or_mutation() {
    let temp = TempWorkspace::new("component-install-dry-run");
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args([
            "install",
            "box",
            "--source",
            "release",
            "--dry-run",
            "--json",
        ])
        .env("A3S_UPDATER_GITHUB_API_BASE", "http://127.0.0.1:1")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["command"], "component.install");
    assert_eq!(result["data"]["dryRun"], true);
    assert_eq!(result["data"]["plans"][0]["component"], "box");
    assert!(!temp.path("state/components").exists());
    assert!(!temp.path("data/components").exists());
}

#[test]
fn offline_install_fails_before_any_network_or_mutation() {
    if support::box_release_target().is_none() {
        eprintln!("skipping offline network test on unsupported host target");
        return;
    }
    let temp = TempWorkspace::new("component-install-offline");
    let server = support::start_fake_box_release(&temp, "2.5.2", None);
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args([
            "--offline",
            "install",
            "box",
            "--source",
            "release",
            "--json",
        ])
        .env_remove("A3S_OFFLINE")
        .env_remove("A3S_NO_AUTO_INSTALL")
        .env("A3S_UPDATER_GITHUB_API_BASE", server.api_base())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(server.requests().is_empty(), "offline install used network");
    assert!(!temp.path("state/components").exists());
    assert!(!temp.path("data/components").exists());
}

#[test]
fn info_and_doctor_have_machine_readable_results() {
    let temp = TempWorkspace::new("component-inspection");

    let mut info = Command::new(a3s_bin());
    configure_component_env(&mut info, &temp);
    let output = info.args(["info", "code", "--json"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schemaVersion"], 1);
    assert_eq!(result["command"], "component.info");
    assert_eq!(result["data"]["component"]["id"], "code");

    let mut doctor = Command::new(a3s_bin());
    configure_component_env(&mut doctor, &temp);
    let output = doctor.args(["doctor", "code", "--json"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schemaVersion"], 1);
    assert_eq!(result["command"], "component.doctor");
    assert_eq!(result["ok"], true);
    assert_eq!(result["data"]["healthy"], true);
    assert_eq!(result["data"]["checks"][0]["id"], "code");
}

#[test]
fn multi_component_partial_failure_preserves_every_outcome() {
    let temp = TempWorkspace::new("component-partial");
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args(["install", "code", "use/acme/slack", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["command"], "component.install");
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["code"], "component.partial");
    assert_eq!(
        result["error"]["details"]["operations"][0]["component"],
        "code"
    );
    assert_eq!(
        result["error"]["details"]["failures"][0]["component"],
        "use/acme/slack"
    );
}

#[test]
fn use_proxy_preserves_arguments_and_child_status() {
    let temp = TempWorkspace::new("use-proxy");
    let bin = temp.path("use-bin");
    let args_log = temp.path("use-args.log");
    make_executable(
        &bin.join("a3s-use"),
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'a3s-use 0.1.0\\n'; exit 0; fi\nprintf '%s\\n' \"$@\" > {}\nprintf 'use:%s\\n' \"$*\"\nif [ \"$1\" = \"fail\" ]; then exit 9; fi\nexit 0\n",
            sh_quote(&args_log)
        ),
    );
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args([
            "use",
            "browser",
            "open",
            "https://example.com",
            "--session",
            "research",
            "--json",
        ])
        .env("A3S_USE_INSTALL_DIR", &bin)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "use:browser open https://example.com --session research --json\n"
    );
    assert_eq!(
        std::fs::read_to_string(&args_log).unwrap(),
        "browser\nopen\nhttps://example.com\n--session\nresearch\n--json\n"
    );

    let mut failing = Command::new(a3s_bin());
    configure_component_env(&mut failing, &temp);
    let output = failing
        .args(["use", "fail"])
        .env("A3S_USE_INSTALL_DIR", &bin)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(9));
}

#[test]
fn use_box_receives_only_the_registered_box_executable() {
    let temp = TempWorkspace::new("use-box-route");
    let use_bin = temp.path("use-bin");
    let box_bin = temp.path("box-bin");
    let route_log = temp.path("use-box-route.log");
    let box_executable = box_bin.join("a3s-box");
    make_executable(
        &box_executable,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'a3s-box 2.5.2\\n'; exit 0; fi\nexit 0\n",
    );
    make_executable(
        &use_bin.join("a3s-use"),
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'a3s-use 0.1.0\\n'; exit 0; fi\nprintf '%s\\n' \"${{A3S_USE_BOX_EXECUTABLE-unset}}\" \"$@\" > {}\nexit 0\n",
            sh_quote(&route_log)
        ),
    );

    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args(["use", "box", "ps", "--json"])
        .env("A3S_USE_INSTALL_DIR", &use_bin)
        .env("A3S_BOX_INSTALL_DIR", &box_bin)
        .env("A3S_USE_BOX_EXECUTABLE", "/untrusted/parent/value")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let lines = std::fs::read_to_string(&route_log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        lines[0],
        box_executable.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(&lines[1..], ["box", "ps", "--json"]);
}

#[test]
fn proxy_receives_the_versioned_invocation_context() {
    let temp = TempWorkspace::new("proxy-context");
    let workspace = temp.path("workspace");
    let launch_directory = temp.path("launch");
    let bin = temp.path("use-bin");
    let context_log = temp.path("proxy-context.log");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&launch_directory).unwrap();
    let canonical_workspace = workspace.canonicalize().unwrap();
    make_executable(
        &bin.join("a3s-use"),
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'a3s-use 0.1.0\\n'; exit 0; fi\nprintf '%s\\n' \"$PWD\" \"$A3S_CLI_CONTEXT_VERSION\" \"$A3S_CLI_DIRECTORY\" \"$A3S_CLI_OUTPUT\" \"$A3S_CLI_OFFLINE\" \"$A3S_CLI_NON_INTERACTIVE\" \"$A3S_CLI_NO_PROGRESS\" \"$A3S_CONFIG_FILE\" \"$A3S_OFFLINE\" \"$A3S_NON_INTERACTIVE\" \"$A3S_NO_PROGRESS\" > {}\nprintf '%s\\n' \"$@\" >> {}\nexit 17\n",
            sh_quote(&context_log),
            sh_quote(&context_log),
        ),
    );

    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .current_dir(&launch_directory)
        .arg("-C")
        .arg(&workspace)
        .args([
            "--config",
            "child.acl",
            "--offline",
            "use",
            "browser",
            "open",
            "value with spaces",
            "--native-json",
        ])
        .env("A3S_USE_INSTALL_DIR", &bin)
        .env_remove("A3S_OFFLINE")
        .env_remove("A3S_NON_INTERACTIVE")
        .env_remove("A3S_NO_PROGRESS")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(17));
    let lines = std::fs::read_to_string(&context_log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(lines[0], canonical_workspace.display().to_string());
    assert_eq!(lines[1], "1");
    assert_eq!(lines[2], canonical_workspace.display().to_string());
    assert_eq!(lines[3], "human");
    assert_eq!(lines[4], "true");
    assert_eq!(lines[5], "true");
    assert_eq!(lines[6], "true");
    assert_eq!(
        lines[7],
        canonical_workspace.join("child.acl").display().to_string()
    );
    assert_eq!(&lines[8..11], ["1", "1", "1"]);
    assert_eq!(
        &lines[11..],
        ["browser", "open", "value with spaces", "--native-json"]
    );
}

#[test]
fn root_machine_output_is_not_silently_applied_to_native_proxies() {
    let temp = TempWorkspace::new("proxy-root-output");
    let marker = temp.path("proxy-ran");
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args(["--output", "json", "use", "browser", "status"])
        .env("A3S_USE_BIN", &marker)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        !marker.exists(),
        "proxy resolution or execution should not run"
    );
    let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["command"], "use");
    assert_eq!(error["error"]["code"], "usage.invalid");
}

#[test]
fn external_use_install_uses_cli_json_not_custom_rpc() {
    let temp = TempWorkspace::new("extension-install");
    let bin = temp.path("use-bin");
    let args_log = temp.path("extension-args.log");
    make_executable(
        &bin.join("a3s-use"),
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'a3s-use 0.1.0\\n'; exit 0; fi\nif [ \"$1\" = \"component\" ] && [ \"$2\" = \"status\" ]; then printf '{{\"component\":{{\"id\":\"%s\",\"presence\":\"missing\",\"health\":\"unknown\"}}}}\\n' \"$3\"; exit 0; fi\nif [ \"$1\" = \"component\" ] && [ \"$2\" = \"install\" ]; then printf '%s\\n' \"$@\" > {}; printf '{{\"schemaVersion\":1,\"ok\":true}}\\n'; exit 0; fi\nexit 2\n",
            sh_quote(&args_log)
        ),
    );
    let package = temp.path("package");
    std::fs::create_dir_all(&package).unwrap();
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .args([
            "install",
            "use/acme/slack",
            "--from",
            package.to_str().unwrap(),
            "--allow-unsigned",
            "--json",
        ])
        .env("A3S_USE_INSTALL_DIR", &bin)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(
        result["data"]["operations"][0]["component"],
        "use/acme/slack"
    );
    let arguments = std::fs::read_to_string(args_log).unwrap();
    assert!(arguments.contains("component\ninstall\nacme/slack\n--json\n"));
    assert!(!arguments.to_ascii_lowercase().contains("jsonrpc"));
}

#[test]
fn built_in_use_runtime_lifecycle_delegates_native_component_commands() {
    let temp = TempWorkspace::new("use-runtime-lifecycle");
    let bin = temp.path("use-bin");
    let args_log = temp.path("runtime-args.log");
    make_executable(
        &bin.join("a3s-use"),
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then printf 'a3s-use 0.1.0\\n'; exit 0; fi\nprintf '%s\\n' \"$@\" >> {}\nif [ \"$1\" = \"component\" ] && [ \"$2\" = \"install\" ]; then printf '{{\"schemaVersion\":1,\"ok\":true,\"data\":{{\"changed\":true,\"component\":{{\"id\":\"%s\",\"version\":\"1.0.136\"}}}}}}\\n' \"$3\"; exit 0; fi\nif [ \"$1\" = \"component\" ] && [ \"$2\" = \"uninstall\" ]; then printf '{{\"schemaVersion\":1,\"ok\":true,\"data\":{{\"changed\":true}}}}\\n'; exit 0; fi\nexit 2\n",
            sh_quote(&args_log)
        ),
    );

    let mut install = Command::new(a3s_bin());
    configure_component_env(&mut install, &temp);
    let output = install
        .args(["install", "use/office", "--json"])
        .env("A3S_USE_INSTALL_DIR", &bin)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut uninstall = Command::new(a3s_bin());
    configure_component_env(&mut uninstall, &temp);
    let output = uninstall
        .args(["uninstall", "use/office", "--json"])
        .env("A3S_USE_INSTALL_DIR", &bin)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let arguments = std::fs::read_to_string(args_log).unwrap();
    assert!(arguments.contains("component\ninstall\noffice\n--json\n"));
    assert!(arguments.contains("component\nuninstall\noffice\n--json\n"));
    assert!(!arguments.to_ascii_lowercase().contains("jsonrpc"));
}
