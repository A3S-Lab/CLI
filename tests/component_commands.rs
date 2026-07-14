#![cfg(unix)]

#[allow(dead_code)]
mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

use support::{a3s_bin, make_executable, sh_quote, TempWorkspace};

const INSTALL_HELP: &str = "usage: a3s install <code|box|bench>\n\n\
`code` is included with a3s; installing it verifies and repairs its companion tools.\n\
Box and Bench are downloaded only by explicit install or first real use.\n";

const UPDATE_HELP: &str = "usage: a3s update [code|box|bench]\n\n\
With no component, this updates Code for compatibility with earlier releases.\n";

const BOX_HELP: &str = "usage: a3s box <args...>\n\n\
Arguments are forwarded to a3s-box. Box is installed automatically on first use.\n";

const BENCH_HELP: &str = "usage:\n\
   a3s bench list [--all] [--json]\n\
   a3s bench info <task-id|./path> [--all] [--json]\n\
   a3s bench run <task-id|./path> --agent <asset> [--json]\n\
   a3s bench result [run-id] [--json]\n\
   a3s bench advanced <command> ...\n\n\
Bench is a private control component installed automatically on first real use;\n\
it is never added to PATH. Candidate and Judge Agent Assets are executed only\n\
by A3S OS Runtime. Local task paths must start with ./ or ../.\n";

const LIST_HELP: &str = "usage: a3s list\n\n\
Show managed Code, Box, and Bench components plus other a3s-* tools on PATH.\n";

struct OfflineEnv {
    _tmp: TempWorkspace,
    components_dir: PathBuf,
    box_install_dir: PathBuf,
    home_dir: PathBuf,
    tools_dir: PathBuf,
    curl_log: PathBuf,
}

impl OfflineEnv {
    fn new(name: &str) -> Self {
        let tmp = TempWorkspace::new(name);
        let components_dir = tmp.path("components");
        let box_install_dir = tmp.path("box-bin");
        let home_dir = tmp.path("home");
        let tools_dir = tmp.path("tools");
        let curl_log = tmp.path("curl.log");
        make_executable(
            &tools_dir.join("curl"),
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexit 97\n",
                sh_quote(&curl_log)
            ),
        );
        Self {
            _tmp: tmp,
            components_dir,
            box_install_dir,
            home_dir,
            tools_dir,
            curl_log,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(a3s_bin())
            .args(args)
            .env("A3S_COMPONENTS_DIR", &self.components_dir)
            .env("A3S_BOX_INSTALL_DIR", &self.box_install_dir)
            .env("HOME", &self.home_dir)
            .env("PATH", &self.tools_dir)
            .env("RUST_BACKTRACE", "0")
            .output()
            .unwrap_or_else(|error| panic!("failed to run a3s {args:?}: {error}"))
    }

    fn assert_no_install_or_network(&self) {
        self.assert_no_component_install();
        assert!(
            !self.curl_log.exists(),
            "curl was unexpectedly invoked: {}",
            std::fs::read_to_string(&self.curl_log).unwrap_or_default()
        );
    }

    fn assert_no_component_install(&self) {
        assert!(
            !self.components_dir.exists(),
            "component state was unexpectedly created at {}",
            self.components_dir.display()
        );
        assert!(
            !self.box_install_dir.exists(),
            "Box install directory was unexpectedly created at {}",
            self.box_install_dir.display()
        );
    }
}

#[test]
fn exact_component_help_is_offline_and_does_not_install() {
    let env = OfflineEnv::new("component-help");
    for (command, expected) in [
        ("install", INSTALL_HELP),
        ("update", UPDATE_HELP),
        ("box", BOX_HELP),
        ("bench", BENCH_HELP),
        ("list", LIST_HELP),
    ] {
        for help in ["-h", "--help", "help"] {
            let args = [command, help];
            let output = env.run(&args);
            assert!(
                output.status.success(),
                "a3s {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
            assert_eq!(String::from_utf8_lossy(&output.stderr), "");
            env.assert_no_install_or_network();
        }
    }

    for (args, expected) in [
        (&["box", "run", "--help"][..], BOX_HELP),
        (&["bench", "run", "--help"][..], BENCH_HELP),
    ] {
        let output = env.run(args);
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
        assert_eq!(String::from_utf8_lossy(&output.stderr), "");
        env.assert_no_install_or_network();
    }
}

#[test]
fn optional_component_version_flags_do_not_install() {
    let env = OfflineEnv::new("component-version");
    for (component, expected) in [
        ("box", "a3s box engine: not installed"),
        ("bench", "a3s bench control component: not installed"),
    ] {
        let output = env.run(&[component, "--version"]);
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
        assert_eq!(String::from_utf8_lossy(&output.stderr), "");
        env.assert_no_install_or_network();
    }
}

#[test]
fn install_requires_one_known_component_without_side_effects() {
    let env = OfflineEnv::new("install-errors");
    for (args, message) in [
        (
            &["install"][..],
            "missing component; choose code, box, or bench",
        ),
        (
            &["install", "unknown"][..],
            "unknown component 'unknown'; choose code, box, or bench",
        ),
    ] {
        let output = env.run(args);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!("a3s: {message}\n\n{INSTALL_HELP}\n")
        );
        env.assert_no_install_or_network();
    }
}

#[test]
fn installing_code_is_offline_and_does_not_install_box_or_bench() {
    let env = OfflineEnv::new("install-code");
    let output = env.run(&["install", "code"]);

    assert!(
        output.status.success(),
        "a3s install code failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("is installed (included with a3s)"),
        "Code inclusion was not explained: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    if cfg!(target_os = "macos") && !output.stderr.is_empty() {
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("companion repair failed"),
            "unexpected macOS warning: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    } else {
        assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    }
    env.assert_no_install_or_network();
}

#[test]
fn missing_bench_lookup_happens_on_explicit_install_or_first_real_use() {
    if !matches!(
        (std::env::consts::OS, std::env::consts::ARCH),
        ("macos" | "linux", "aarch64" | "x86_64")
    ) {
        return;
    }

    for (name, args, delayed) in [
        ("bench-explicit-install", &["install", "bench"][..], false),
        ("bench-first-real-use", &["bench", "run", "smoke"][..], true),
    ] {
        let env = OfflineEnv::new(name);
        let output = env.run(args);
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("Bench control component may not be published yet"),
            "unexpected a3s {args:?} diagnostic: {stderr}"
        );
        assert_eq!(
            stderr.contains("Bench control component is not installed; installing it now"),
            delayed,
            "only a delayed first real use should report the missing component: {stderr}"
        );

        let curl_log = std::fs::read_to_string(&env.curl_log)
            .expect("explicit install and first real use should query the Bench release");
        assert!(
            curl_log.contains("A3S-Lab/a3s-bench/releases/latest"),
            "unexpected release lookup: {curl_log}"
        );
        env.assert_no_component_install();
    }
}

#[test]
fn bare_and_explicit_code_update_share_the_code_updater() {
    let env = OfflineEnv::new("update-code-aliases");
    make_executable(
        &env.tools_dir.join("curl"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nprintf 'https://github.com/A3S-Lab/Cli/releases/tag/v{}\\n'\nexit 0\n",
            sh_quote(&env.curl_log),
            env!("CARGO_PKG_VERSION")
        ),
    );

    for args in [&["update"][..], &["update", "code"][..]] {
        let output = env.run(args);
        assert!(
            output.status.success(),
            "a3s {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("already up to date"),
            "a3s {args:?} did not use the Code updater: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        if cfg!(target_os = "macos") && !output.stderr.is_empty() {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("install repair failed"),
                "unexpected macOS warning: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        } else {
            assert_eq!(String::from_utf8_lossy(&output.stderr), "");
        }
        env.assert_no_component_install();
    }

    let curl_log = std::fs::read_to_string(&env.curl_log)
        .expect("both Code updates should query the release redirect");
    let calls = curl_log.lines().collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "unexpected curl calls: {curl_log}");
    assert!(
        calls
            .iter()
            .all(|call| call.contains("A3S-Lab/Cli/releases/latest")),
        "Code update touched a non-Code release endpoint: {curl_log}"
    );
}

#[test]
fn updating_missing_box_and_bench_points_to_install_without_network() {
    let env = OfflineEnv::new("missing-update");
    for (component, expected) in [
        ("box", "run `a3s install box` first"),
        ("bench", "run `a3s install bench` first"),
    ] {
        let output = env.run(&["update", component]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "unexpected status for update {component}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected),
            "missing install guidance for {component}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        env.assert_no_install_or_network();
    }
}

#[test]
fn list_always_shows_code_box_bench_without_creating_components() {
    let env = OfflineEnv::new("component-list");
    let output = env.run(&["list"]);

    assert!(
        output.status.success(),
        "a3s list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let component_rows = stdout
        .lines()
        .filter(|line| matches!(*line, "  code" | "  box" | "  bench"))
        .collect::<Vec<_>>();
    assert_eq!(component_rows, vec!["  code", "  box", "  bench"]);
    assert!(stdout.contains("managed components\n"));
    assert!(stdout.contains("other a3s-* tools on PATH\n  none found\n"));
    env.assert_no_install_or_network();
}

#[test]
fn managed_bench_forwards_arguments_and_exit_status() {
    let env = OfflineEnv::new("bench-proxy");
    let args_log = env._tmp.path("bench-args.log");
    install_fake_bench_bundle(&env.components_dir, &args_log, 23);

    let output = env.run(&["bench", "run", "./smoke task", "--agent", "asset name"]);

    assert_eq!(output.status.code(), Some(23));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "fake-bench:run ./smoke task --agent asset name\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert_eq!(
        std::fs::read_to_string(args_log).expect("fake Bench should record arguments"),
        "run\n./smoke task\n--agent\nasset name\n"
    );
    assert!(
        !env.curl_log.exists(),
        "an installed Bench control component must not trigger a release lookup"
    );
}

fn install_fake_bench_bundle(components_dir: &Path, args_log: &Path, exit_code: i32) {
    let version = "1.2.3";
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x86_64",
        _ => "test-target",
    };
    let relative_root = format!("versions/{version}/{target}");
    let bench_root = components_dir.join("bench");
    let package_root = bench_root.join(&relative_root);
    let entrypoint = package_root.join("bin/a3s-bench");
    make_executable(
        &entrypoint,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\nprintf 'fake-bench:%s\\n' \"$*\"\nexit {exit_code}\n",
            sh_quote(args_log)
        ),
    );

    write_json(
        &package_root.join("component.json"),
        &format!(
            r#"{{
  "schema": "a3s.component.v1",
  "component": "bench",
  "version": "{version}",
  "target": "{target}",
  "cli_protocol": "a3s-bench-cli/v1",
  "entrypoint": "bin/a3s-bench",
  "required_files": []
}}"#
        ),
    );
    let payload_sha256 = bench_payload_sha256(&package_root);
    write_json(
        &package_root.join("receipt.json"),
        &format!(
            r#"{{
  "schema": "a3s.component-receipt.v1",
  "component": "bench",
  "version": "{version}",
  "target": "{target}",
  "source_url": "https://github.com/A3S-Lab/a3s-bench/releases/download/v{version}/a3s-bench-{version}-{target}.tar.gz",
  "archive_sha256": "{}",
  "payload_sha256": "{payload_sha256}",
  "entrypoint": "bin/a3s-bench",
  "cli_protocol": "a3s-bench-cli/v1"
}}"#,
            "a".repeat(64)
        ),
    );
    write_json(
        &bench_root.join("current.json"),
        &format!(
            r#"{{
  "schema": "a3s.component-current.v1",
  "component": "bench",
  "version": "{version}",
  "target": "{target}",
  "path": "{relative_root}",
  "archive_sha256": "{}"
}}"#,
            "a".repeat(64)
        ),
    );
}

fn bench_payload_sha256(root: &Path) -> String {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).expect("Bench fixture directory should be readable") {
            let path = entry
                .expect("Bench fixture entry should be readable")
                .path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .strip_prefix(root)
                .expect("Bench fixture path should stay under root")
                != Path::new("receipt.json")
            {
                files.push(path);
            }
        }
    }
    files.sort_by(|left, right| {
        left.strip_prefix(root)
            .unwrap()
            .cmp(right.strip_prefix(root).unwrap())
    });

    let mut hasher = Sha256::new();
    for path in files {
        let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
        let contents = std::fs::read(&path).expect("Bench fixture file should be readable");
        hasher.update((relative.len() as u64).to_le_bytes());
        hasher.update(relative.as_bytes());
        hasher.update((contents.len() as u64).to_le_bytes());
        hasher.update(contents);
    }
    format!("{:x}", hasher.finalize())
}

fn write_json(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("JSON fixture parent should be created");
    }
    std::fs::write(path, contents).expect("JSON fixture should be written");
}
