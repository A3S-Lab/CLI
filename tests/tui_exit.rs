#![cfg(target_os = "macos")]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

fn process_exists(pid: &str) -> bool {
    Command::new("/bin/kill")
        .args(["-0", pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn process_group_exists(pid: &str) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn kill_process_group(pid: &str) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn kill_process(pid: &str) {
    let _ = Command::new("/bin/kill")
        .args(["-KILL", pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn wait_for_process_exit(pid: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(1);
    while process_exists(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    !process_exists(pid)
}

fn metric_ms(output: &str, name: &str) -> Option<u64> {
    output
        .split_whitespace()
        .find_map(|field| field.strip_prefix(&format!("{name}=")))?
        .parse()
        .ok()
}

fn startup_trace_total_ms(trace: &str, phase: &str) -> Option<u64> {
    trace
        .lines()
        .find(|line| line.contains(&format!("phase={phase} ")))
        .and_then(|line| metric_ms(line, "total_ms"))
}

#[test]
fn code_startup_reaches_first_frame_before_external_capability_setup() {
    use std::os::unix::fs::OpenOptionsExt;

    const RELEASE_MEMORY_AFTER_READER: Duration = Duration::from_secs(5);
    const STARTUP_DEADLINE_MS: u64 = 1_500;

    let directory = TestDirectory::new();
    let workspace = directory.join("workspace");
    let home = directory.join("home");
    let memory = directory.join("memory");
    let items = memory.join("items");
    let config = directory.join("config.acl");
    let trace = directory.join("startup-trace.log");
    let release_marker = directory.join("memory-payload-released");
    let mcp_started = directory.join("blocked-mcp-started");
    let mcp_server = directory.join("blocked-mcp");
    let fifo = items.join("blocked-memory.json");
    fs::create_dir_all(&workspace).expect("create startup workspace");
    fs::create_dir_all(&home).expect("create startup home");
    fs::create_dir_all(&items).expect("create startup memory directory");

    let timestamp = "2026-08-17T00:00:00Z";
    let content = "A blocked memory item proves that startup does not await evolution scanning.";
    let item = serde_json::json!({
        "id": "blocked-memory",
        "content": content,
        "timestamp": timestamp,
        "importance": 0.5,
        "tags": [],
        "memory_type": "semantic",
        "metadata": {},
        "access_count": 0,
        "last_accessed": null
    });
    fs::write(
        memory.join("index.json"),
        serde_json::to_vec(&serde_json::json!([{
            "id": "blocked-memory",
            "content_lower": content.to_ascii_lowercase(),
            "tags": [],
            "importance": 0.5,
            "timestamp": timestamp,
            "memory_type": "semantic"
        }]))
        .expect("encode startup memory index"),
    )
    .expect("write startup memory index");
    let status = Command::new("/usr/bin/mkfifo")
        .arg(&fifo)
        .status()
        .expect("create blocked memory FIFO");
    assert!(status.success(), "mkfifo exited with {status}");

    write_executable(
        &mcp_server,
        "#!/bin/sh\n: > \"$A3S_BLOCKED_MCP_STARTED\"\ntrap 'exit 0' TERM INT\nsleep 300 &\nwait\n",
    );

    let memory_acl = memory.to_string_lossy().replace('"', "\\\"");
    let mcp_server_acl = mcp_server.to_string_lossy().replace('"', "\\\"");
    let mcp_started_acl = mcp_started.to_string_lossy().replace('"', "\\\"");
    fs::write(
        &config,
        format!(
            r#"default_model = "openai/test"
memory_dir = "{memory_acl}"
providers "openai" {{
  apiKey = "test"
  baseUrl = "http://127.0.0.1:1"
  models "test" {{
    name = "Test"
    toolCall = true
  }}
}}
memory {{ llmExtraction = false }}
workspace_retrieval {{
  enabled = true
  local_cpu {{}}
}}
mcp_servers "startup-blocker" {{
  transport = "stdio"
  command = "{mcp_server_acl}"
  enabled = true
  env = {{ A3S_BLOCKED_MCP_STARTED = "{mcp_started_acl}" }}
}}
"#,
        ),
    )
    .expect("write startup config");

    let fifo_payload = serde_json::to_vec(&item).expect("encode blocked memory item");
    let fifo_for_writer = fifo.clone();
    let release_marker_for_writer = release_marker.clone();
    let writer = std::thread::spawn(move || -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo_for_writer)
            {
                Ok(mut file) => {
                    // Hold the Evolution read after it has opened the FIFO. A
                    // correct launch has already rendered its first frame;
                    // the pre-optimization path cannot reach terminal handoff
                    // until this payload is released.
                    std::thread::sleep(RELEASE_MEMORY_AFTER_READER);
                    fs::write(&release_marker_for_writer, b"released")
                        .map_err(|error| format!("mark memory payload release: {error}"))?;
                    file.write_all(&fifo_payload)
                        .map_err(|error| format!("write blocked memory item: {error}"))?;
                    return Ok(());
                }
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => {
                    return Err(format!(
                        "evolution never opened the blocked memory item: {error}"
                    ));
                }
            }
        }
    });

    let expect_script = r#"
log_user 0
set timeout 20
set started [clock milliseconds]
spawn -noecho /bin/sh -c {exec "$A3S_STARTUP_TEST_BIN" code -C "$A3S_STARTUP_TEST_WORKSPACE" --config "$A3S_STARTUP_TEST_CONFIG" 2>"$A3S_STARTUP_TEST_TRACE"}
expect {
    -exact "\033\[?1049h" { set takeover [clock milliseconds] }
    eof { puts "a3s exited before terminal takeover"; exit 130 }
    timeout { catch {exec kill -TERM [exp_pid]}; catch {wait}; puts "terminal takeover timed out"; exit 131 }
}
expect {
    -exact "\033\[?u\033\[c" {
        send -- "\033\[?1u\033\[?1c"
        exp_continue
    }
    -exact "\033\[2J" {
        set frame [clock milliseconds]
        set released_before_frame [file exists $env(A3S_STARTUP_TEST_RELEASE_MARKER)]
        set mcp_started_before_frame [file exists $env(A3S_STARTUP_TEST_MCP_MARKER)]
    }
    eof { puts "a3s exited before its first frame"; exit 132 }
    timeout { catch {exec kill -TERM [exp_pid]}; catch {wait}; puts "first frame timed out"; exit 133 }
}
# Keep the process alive long enough for the post-frame Evolution reader to
# connect and receive the deliberately delayed payload.
after 6500
send -- "/exit\r"
set timeout 15
expect {
    eof {
        set result [wait]
        set status [lindex $result 3]
        puts "takeover_ms=[expr {$takeover - $started}] frame_ms=[expr {$frame - $started}] released_before_frame=$released_before_frame mcp_started_before_frame=$mcp_started_before_frame exit_status=$status"
        exit $status
    }
    timeout {
        catch {exec kill -TERM [exp_pid]}
        after 500
        catch {exec kill -KILL [exp_pid]}
        catch {wait}
        puts "startup probe did not exit"
        exit 134
    }
}
"#;
    let output = Command::new("/usr/bin/expect")
        .args(["-c", expect_script])
        .env("HOME", &home)
        .env("A3S_DATA_HOME", directory.join("data"))
        .env("A3S_STATE_HOME", directory.join("state"))
        .env("A3S_CACHE_HOME", directory.join("cache"))
        .env("A3S_RUNTIME_HOME", directory.join("runtime"))
        .env("A3S_NO_AUTO_INSTALL", "1")
        .env("A3S_OFFLINE", "1")
        .env("A3S_CODE_STARTUP_TRACE", "1")
        .env("A3S_STARTUP_TEST_BIN", env!("CARGO_BIN_EXE_a3s"))
        .env("A3S_STARTUP_TEST_WORKSPACE", &workspace)
        .env("A3S_STARTUP_TEST_CONFIG", &config)
        .env("A3S_STARTUP_TEST_TRACE", &trace)
        .env("A3S_STARTUP_TEST_RELEASE_MARKER", &release_marker)
        .env("A3S_STARTUP_TEST_MCP_MARKER", &mcp_started)
        .env_remove("CODEX_HOME")
        .output()
        .expect("run first-frame startup probe");
    writer
        .join()
        .expect("blocked memory writer panicked")
        .expect("release blocked memory item");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "first-frame startup probe failed:\nstdout: {stdout}\nstderr: {stderr}"
    );
    let takeover_ms = metric_ms(&stdout, "takeover_ms").expect("terminal takeover metric");
    let frame_ms = metric_ms(&stdout, "frame_ms").expect("first-frame metric");
    let released_before_frame =
        metric_ms(&stdout, "released_before_frame").expect("memory release ordering metric");
    let mcp_started_before_frame =
        metric_ms(&stdout, "mcp_started_before_frame").expect("MCP start ordering metric");
    let trace = fs::read_to_string(&trace).expect("read startup trace");
    assert!(
        released_before_frame == 0,
        "first frame waited for the blocked Evolution payload: {stdout}\n{trace}"
    );
    assert_eq!(
        mcp_started_before_frame, 0,
        "configured MCP started before the first frame: {stdout}\n{trace}"
    );
    assert!(
        mcp_started.is_file(),
        "deferred configured MCP never started after the first frame: {stdout}\n{trace}"
    );
    assert!(
        frame_ms.saturating_sub(takeover_ms) < STARTUP_DEADLINE_MS,
        "first render stalled after terminal takeover: {stdout}\n{trace}"
    );
    let handoff_ms =
        startup_trace_total_ms(&trace, "terminal_handoff").expect("terminal handoff trace metric");
    assert!(
        handoff_ms < STARTUP_DEADLINE_MS,
        "pre-render startup exceeded {STARTUP_DEADLINE_MS} ms: {trace}"
    );
}

#[test]
fn code_exit_completes_after_session_saved_with_a_blocked_workspace_scan() {
    let directory = TestDirectory::new();
    let workspace = directory.join("workspace");
    let home = directory.join("home");
    let bin = directory.join("bin");
    let config = directory.join("config.acl");
    let block_git = directory.join("block-git");
    let git_started = directory.join("git-started");
    let sleep_started = directory.join("sleep-started");
    let trigger = workspace.join("trigger.txt");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&home).expect("create home");
    fs::write(workspace.join("README.md"), "# Exit test\n").expect("write workspace file");
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
memory { llmExtraction = false }
"#,
    )
    .expect("write test config");
    write_executable(
        &bin.join("curl"),
        "#!/bin/sh\nprintf 'https://github.com/A3S-Lab/Cli/releases/tag/v0.8.3'\n",
    );
    write_executable(
        &bin.join("git"),
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'git version test\\n'\n  exit 0\nfi\nif [ -f '{}' ]; then\n  printf '%s\\n' \"$$\" > '{}'\n  /bin/sleep 30 &\n  sleep_pid=$!\n  printf '%s\\n' \"$sleep_pid\" > '{}'\n  wait \"$sleep_pid\"\nfi\n",
            block_git.display(),
            git_started.display(),
            sleep_started.display()
        ),
    );

    let expect_script = r#"
log_user 0
set timeout 60
spawn $env(A3S_EXIT_TEST_BIN) code -C $env(A3S_EXIT_TEST_WORKSPACE) --config $env(A3S_EXIT_TEST_CONFIG)
expect {
    -exact "\033\[?1049h" {}
    eof {
        set result [wait]
        puts "a3s exited before the TUI became ready: [lindex $result 3]"
        exit 120
    }
    timeout {
        catch {exec kill -TERM [exp_pid]}
        catch {wait}
        puts "TUI event loop did not become ready"
        exit 121
    }
}

set block [open $env(A3S_EXIT_TEST_BLOCK_GIT) w]
close $block
set trigger [open $env(A3S_EXIT_TEST_TRIGGER) w]
puts $trigger "trigger"
close $trigger

set scan_deadline [expr {[clock milliseconds] + 5000}]
while {(![file exists $env(A3S_EXIT_TEST_GIT_STARTED)] || ![file exists $env(A3S_EXIT_TEST_SLEEP_STARTED)]) && [clock milliseconds] < $scan_deadline} {
    after 50
}
if {![file exists $env(A3S_EXIT_TEST_GIT_STARTED)] || ![file exists $env(A3S_EXIT_TEST_SLEEP_STARTED)]} {
    catch {exec kill -TERM [exp_pid]}
    catch {wait}
    puts "blocked Git scan was not observed"
    exit 122
}

set started [clock milliseconds]
send -- "/exit\r"
set timeout 12
expect {
    -glob "*session saved*" {
        set saved [clock milliseconds]
        set timeout 5
        expect {
            eof {
                set finished [clock milliseconds]
                set elapsed [expr {$finished - $started}]
                set after_saved [expr {$finished - $saved}]
                set result [wait]
                set status [lindex $result 3]
                puts "exit_ms=$elapsed after_session_saved_ms=$after_saved exit_status=$status"
                if {$status != 0 || $elapsed >= 10000 || $after_saved >= 4000} {
                    exit 123
                }
                exit 0
            }
            timeout {
                catch {exec kill -TERM [exp_pid]}
                after 500
                catch {exec kill -KILL [exp_pid]}
                catch {wait}
                puts "process remained alive after the session-saved message"
                exit 127
            }
        }
    }
    eof {
        set result [wait]
        puts "a3s exited without the session-saved message: [lindex $result 3]"
        exit 128
    }
    timeout {
        catch {exec kill -TERM [exp_pid]}
        after 500
        catch {exec kill -KILL [exp_pid]}
        catch {wait}
        puts "TUI exit exceeded its deadline"
        exit 124
    }
}
"#;
    let path = format!("{}:/usr/local/bin:/usr/bin:/bin", bin.to_string_lossy());
    let output = Command::new("/usr/bin/expect")
        .args(["-c", expect_script])
        .env("HOME", &home)
        .env("PATH", path)
        .env("A3S_NO_AUTO_INSTALL", "1")
        .env("A3S_EXIT_TEST_BIN", env!("CARGO_BIN_EXE_a3s"))
        .env("A3S_EXIT_TEST_WORKSPACE", &workspace)
        .env("A3S_EXIT_TEST_CONFIG", &config)
        .env("A3S_EXIT_TEST_BLOCK_GIT", &block_git)
        .env("A3S_EXIT_TEST_GIT_STARTED", &git_started)
        .env("A3S_EXIT_TEST_SLEEP_STARTED", &sleep_started)
        .env("A3S_EXIT_TEST_TRIGGER", &trigger)
        .output()
        .expect("run PTY exit probe");

    let git_pid = fs::read_to_string(&git_started)
        .ok()
        .map(|value| value.trim().to_owned());
    let sleep_pid = fs::read_to_string(&sleep_started)
        .ok()
        .map(|value| value.trim().to_owned());
    let git_exited = git_pid.as_deref().is_some_and(wait_for_process_exit);
    let sleep_exited = sleep_pid.as_deref().is_some_and(wait_for_process_exit);
    let git_group_still_running = git_pid.as_deref().is_some_and(process_group_exists);
    if let Some(pid) = git_pid.as_deref().filter(|_| git_group_still_running) {
        kill_process_group(pid);
    }
    if let Some(pid) = git_pid.as_deref().filter(|_| !git_exited) {
        kill_process(pid);
    }
    if let Some(pid) = sleep_pid.as_deref().filter(|_| !sleep_exited) {
        kill_process(pid);
    }

    assert!(
        output.status.success(),
        "PTY exit probe failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        git_exited && sleep_exited && !git_group_still_running,
        "workspace scan processes survived TUI shutdown: git={git_pid:?}, sleep={sleep_pid:?}, \
         git_alive={}, sleep_alive={}, group_alive={git_group_still_running}",
        !git_exited,
        !sleep_exited
    );
}
