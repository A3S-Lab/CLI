#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("a3s-web-interrupt-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path).expect("create Web interrupt test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
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

#[test]
fn foreground_web_exits_on_ctrl_c_from_raw_terminal() {
    let directory = TestDirectory::new();
    let config_path = directory.join("config.acl");
    fs::write(&config_path, test_config()).expect("write Web interrupt test config");

    let expect_script = r#"
log_user 0
set timeout 45
spawn /bin/sh -c {stty raw -echo -isig; exec "$A3S_WEB_INTERRUPT_BIN" web --api-only --host 127.0.0.1 --port 0 --workspace "$A3S_WEB_INTERRUPT_ROOT" --config "$A3S_WEB_INTERRUPT_CONFIG"}
expect {
    -exact "Press Ctrl+C to stop.\n" {}
    eof {
        set result [wait]
        puts "a3s web exited before becoming ready: [lindex $result 3]"
        exit 120
    }
    timeout {
        catch {exec kill -TERM [exp_pid]}
        catch {wait}
        puts "a3s web did not become ready"
        exit 121
    }
}

send -- "\003"
set timeout 12
expect {
    eof {
        set result [wait]
        set status [lindex $result 3]
        if {$status != 0} {
            puts "a3s web exited with status $status"
            exit 122
        }
        exit 0
    }
    timeout {
        catch {exec kill -TERM [exp_pid]}
        after 500
        catch {exec kill -KILL [exp_pid]}
        catch {wait}
        puts "Ctrl+C did not stop a3s web"
        exit 123
    }
}
"#;
    let output = Command::new("/usr/bin/expect")
        .args(["-c", expect_script])
        .env("HOME", directory.path())
        .env("A3S_NO_AUTO_INSTALL", "1")
        .env("A3S_WEB_INTERRUPT_BIN", env!("CARGO_BIN_EXE_a3s"))
        .env("A3S_WEB_INTERRUPT_ROOT", directory.path())
        .env("A3S_WEB_INTERRUPT_CONFIG", &config_path)
        .output()
        .expect("run foreground Web PTY probe");

    assert!(
        output.status.success(),
        "foreground Web PTY probe failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn test_config() -> &'static str {
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
"#
}
