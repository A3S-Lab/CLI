#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let directory_id = NEXT_TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "a3s-tui-exit-{}-{stamp}-{directory_id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create TUI exit test directory");
        Self { path }
    }

    fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.path.join(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_executable(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create executable parent");
    }
    fs::write(path, contents).expect("write executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make executable");
}

/// Launch `a3s` through an unrestricted wrapper so macOS SIP cannot strip the
/// native zvec library path when `/usr/bin/expect` is the parent process.
fn sip_safe_a3s_launcher(directory: &Path) -> PathBuf {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_a3s"));
    let launcher = directory.join("a3s-sip-safe");
    let script = match discover_zvec_lib_dir(&bin) {
        Some(zvec_dir) => format!(
            "#!/bin/sh\n\
export DYLD_LIBRARY_PATH=\"{zvec}:${{DYLD_LIBRARY_PATH:-}}\"\n\
export LD_LIBRARY_PATH=\"{zvec}:${{LD_LIBRARY_PATH:-}}\"\n\
exec \"{bin}\" \"$@\"\n",
            zvec = zvec_dir.display(),
            bin = bin.display(),
        ),
        None => format!(
            "#!/bin/sh\n\
exec \"{bin}\" \"$@\"\n",
            bin = bin.display(),
        ),
    };
    write_executable(&launcher, &script);
    launcher
}

fn discover_zvec_lib_dir(bin: &Path) -> Option<PathBuf> {
    let build_dir = bin.parent()?.join("build");
    let mut candidates = Vec::new();
    for entry in fs::read_dir(build_dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("zvec-rust-sys-") {
            continue;
        }
        let prebuilt = entry.path().join("out").join("zvec-prebuilt");
        let dylib = prebuilt.join("libzvec_c_api.dylib");
        let so = prebuilt.join("libzvec_c_api.so");
        if dylib.is_file() || so.is_file() {
            candidates.push(prebuilt);
        }
    }
    candidates.sort();
    candidates.pop()
}

fn kill_process_group(pid: &str) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn command_output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> std::io::Result<(Output, bool)> {
    command
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let child_pid = child.id().to_string();
    let deadline = Instant::now() + timeout;

    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map(|output| (output, false));
        }
        if Instant::now() >= deadline {
            kill_process_group(&child_pid);
            let _ = child.kill();
            return child.wait_with_output().map(|output| (output, true));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn code_tui_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../code/target/debug/a3s-code-tui")
}

fn write_launch_fixture(directory: &TestDirectory) -> (PathBuf, PathBuf, PathBuf) {
    let workspace = directory.join("workspace");
    let home = directory.join("home");
    let config = directory.join("config.acl");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&home).expect("create home");
    fs::write(workspace.join("README.md"), "# TUI fixture\n").expect("write readme");
    fs::write(
        &config,
        r#"default_model = "openai/test"
providers "openai" {
  apiKey = "test"
  baseUrl = "http://127.0.0.1:1"
  models "test" {
    name = "Test"
    toolCall = true
  }
}
"#,
    )
    .expect("write config");
    (workspace, home, config)
}

fn expect_a3s_code_shows_product_and_model(directory: &TestDirectory) {
    let tui = code_tui_binary();
    assert!(
        tui.is_file(),
        "build a3s-code-tui before this probe: {}",
        tui.display()
    );
    let (workspace, home, config) = write_launch_fixture(directory);
    let expect_script = r#"
log_user 1
set timeout 20
set stty_init "rows 32 cols 100"
spawn -noecho /bin/sh -c {exec "$A3S_STARTUP_TEST_BIN" code -C "$A3S_STARTUP_TEST_WORKSPACE" --config "$A3S_STARTUP_TEST_CONFIG"}
expect {
    -exact "\033\[?1049h" {}
    eof { puts "a3s exited before the alternate screen"; exit 130 }
    timeout { puts "alternate screen timed out"; exit 131 }
}
expect {
    -exact "A3S" {}
    timeout { puts "product name missing"; exit 132 }
}
expect {
    -exact "openai/test" {}
    timeout { puts "ACL model missing"; exit 133 }
}
send -- "/exit\r"
set timeout 15
expect {
    eof {
        set result [wait]
        exit [lindex $result 3]
    }
    timeout { puts "TUI did not exit"; exit 134 }
}
"#;
    let launcher = sip_safe_a3s_launcher(&directory.path);
    let mut command = Command::new("/usr/bin/expect");
    command
        .args(["-c", expect_script])
        .env("HOME", &home)
        .env("A3S_DATA_HOME", directory.join("data"))
        .env("A3S_STATE_HOME", directory.join("state"))
        .env("A3S_CACHE_HOME", directory.join("cache"))
        .env("A3S_RUNTIME_HOME", directory.join("runtime"))
        .env("A3S_NO_AUTO_INSTALL", "1")
        .env("A3S_OFFLINE", "1")
        .env("A3S_CODE_TUI_BIN", &tui)
        .env("A3S_STARTUP_TEST_BIN", &launcher)
        .env("A3S_STARTUP_TEST_WORKSPACE", &workspace)
        .env("A3S_STARTUP_TEST_CONFIG", &config)
        .env_remove("A3S_CODE_TUI_SMOKE")
        .env_remove("A3S_CODE_TUI_PROMPT")
        .env_remove("A3S_DEFAULT_MODEL");
    let (output, timed_out) =
        command_output_with_timeout(&mut command, Duration::from_secs(50)).expect("run TUI probe");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!timed_out, "TUI probe timed out:\n{stdout}\n{stderr}");
    assert!(
        output.status.success(),
        "TUI probe failed:\n{stdout}\n{stderr}"
    );
    assert!(stdout.contains("A3S"), "stdout missing A3S:\n{stdout}");
    assert!(
        stdout.contains("openai/test"),
        "stdout missing ACL model:\n{stdout}"
    );
}

#[test]
fn code_tui_shows_a3s_and_acl_model() {
    let directory = TestDirectory::new();
    expect_a3s_code_shows_product_and_model(&directory);
}

#[test]
fn code_tui_exits_on_slash_exit() {
    let directory = TestDirectory::new();
    expect_a3s_code_shows_product_and_model(&directory);
}
