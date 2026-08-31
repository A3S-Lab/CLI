#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn a3s_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_a3s"))
}

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("reserve Harness port")
        .local_addr()
        .unwrap()
        .port()
}

fn write_fixture(root: &Path, port: u16) -> (PathBuf, PathBuf) {
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".a3s")).unwrap();
    let config = project.join("config.acl");
    std::fs::write(
        &config,
        r#"
default_model = "openai/test"
providers "openai" {
  apiKey = env("HARNESS_TEST_API_KEY")
  baseUrl = "http://127.0.0.1:1"
  models "test" { name = "test" }
}
"#,
    )
    .unwrap();
    let manifest = project.join(".a3s/asset.acl");
    std::fs::write(
        &manifest,
        format!(
            r#"
agent_release {{
  schema = "a3s.code.agent-release.v1"
  protocol = "a3s.code.agent.v1"
  artifact {{
    digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111"
    media_type = "application/vnd.oci.image.manifest.v1+json"
  }}
  entrypoint {{
    command = "/usr/bin/a3s"
    args = ["code", "harness", "--manifest", "/app/.a3s/asset.acl"]
  }}
  health {{
    transport = "http"
    port = {port}
    readiness_path = "/health/ready"
    liveness_path = "/health/live"
    shutdown_grace_seconds = 1
  }}
  storage {{
    workspace = "ephemeral"
    cache = "ephemeral"
    persistent_data = "none"
  }}
  capability "runtime.service" {{ level = 1 }}
  capability "secrets.external" {{ level = 1 }}
  capability "workspace.local" {{ level = 1 }}
  secret "provider-api-key" {{
    target = "environment"
    destination = "HARNESS_TEST_API_KEY"
  }}
  provenance "source" {{
    uri = "https://github.com/A3S-Lab/Code"
    digest = "sha256:2222222222222222222222222222222222222222222222222222222222222222"
  }}
}}
"#
        ),
    )
    .unwrap();
    (project, config)
}

fn command(project: &Path, config: &Path) -> Command {
    let mut command = Command::new(a3s_binary());
    command
        .args(["--directory"])
        .arg(project)
        .args(["--config"])
        .arg(config)
        .args([
            "--non-interactive",
            "code",
            "harness",
            "--manifest",
            ".a3s/asset.acl",
            "--listen",
            "127.0.0.1",
        ])
        .env("HOME", project.join("home"))
        .env("A3S_DATA_HOME", project.join("data"))
        .env("A3S_STATE_HOME", project.join("state"))
        .env("A3S_CACHE_HOME", project.join("cache"));
    command
}

fn http_get(address: SocketAddr, path: &str) -> std::io::Result<String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(250))?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn wait_ready(address: SocketAddr) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(response) = http_get(address, "/health/ready") {
            if response.starts_with("HTTP/1.1 200") {
                return response;
            }
        }
        assert!(Instant::now() < deadline, "Harness did not become ready");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn release_harness_is_secret_free_and_honors_sigterm_shutdown() {
    let root = tempfile::tempdir().expect("create Harness process fixture");
    let port = reserve_port();
    let (project, config) = write_fixture(root.path(), port);
    let secret = "harness-secret-value-must-never-appear";
    let child = command(&project, &config)
        .env("HARNESS_TEST_API_KEY", secret)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start Agent Harness");
    let address = SocketAddr::from(([127, 0, 0, 1], port));

    let ready = wait_ready(address);
    assert!(ready.contains(r#""status":"ready""#), "{ready}");
    assert!(!ready.contains(secret));
    let live = http_get(address, "/health/live").expect("read Harness liveness");
    assert!(live.starts_with("HTTP/1.1 200"), "{live}");
    assert!(live.contains(r#""status":"live""#), "{live}");
    assert!(!live.contains(secret));

    let shutdown_started = Instant::now();
    // SAFETY: `child.id()` is the exact owned test process and is reaped below.
    let signal_result = unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    assert_eq!(signal_result, 0, "send SIGTERM to Harness");
    let output = child.wait_with_output().expect("reap Agent Harness");
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        shutdown_started.elapsed() < Duration::from_secs(3),
        "Harness exceeded its declared shutdown window"
    );
    let process_output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!process_output.contains(secret));
    TcpListener::bind(address).expect("Harness must release its listener after shutdown");
}

#[test]
fn release_harness_fails_before_readiness_when_a_secret_is_missing() {
    let root = tempfile::tempdir().expect("create missing-secret fixture");
    let port = reserve_port();
    let (project, config) = write_fixture(root.path(), port);
    let output = command(&project, &config)
        .output()
        .expect("run Harness without required secret");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("provider-api-key"), "{stderr}");
    assert!(!stderr.contains("harness-secret-value-must-never-appear"));
    TcpListener::bind(("127.0.0.1", port))
        .expect("missing secret must fail before binding the release port");
}

#[test]
fn incompatible_release_fails_before_binding_with_its_structured_code() {
    for (name, original, replacement, expected_code) in [
        (
            "protocol",
            r#"protocol = "a3s.code.agent.v1""#,
            r#"protocol = "a3s.code.agent.v2""#,
            "a3s.code.agent_release.incompatible_protocol",
        ),
        (
            "capability",
            r#"capability "workspace.local" { level = 1 }"#,
            r#"capability "workspace.local" { level = 2 }"#,
            "a3s.code.agent_release.unsupported_capability",
        ),
    ] {
        let root = tempfile::tempdir().expect("create incompatible-release fixture");
        let port = reserve_port();
        let (project, config) = write_fixture(root.path(), port);
        let manifest = project.join(".a3s/asset.acl");
        let source = std::fs::read_to_string(&manifest).expect("read release manifest");
        assert!(source.contains(original), "missing {name} fixture marker");
        std::fs::write(&manifest, source.replacen(original, replacement, 1))
            .expect("write incompatible release manifest");

        let output = command(&project, &config)
            .arg("--json")
            .env("HARNESS_TEST_API_KEY", "injected-but-never-rendered")
            .output()
            .expect("run incompatible Agent Harness");

        assert!(
            !output.status.success(),
            "{name} incompatibility was admitted"
        );
        let document: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("decode structured CLI failure");
        assert_eq!(document["error"]["code"], expected_code, "{document}");
        let rendered = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!rendered.contains("injected-but-never-rendered"));
        TcpListener::bind(("127.0.0.1", port))
            .expect("incompatible release must fail before binding its port");
    }
}
