#![cfg(unix)]

mod support;

use std::process::{Command, Output, Stdio};
use std::{io::Read, io::Write, net::TcpListener};

use support::{a3s_bin, make_executable, TempWorkspace};

fn run(home: &std::path::Path, config: &std::path::Path, args: &[&str]) -> Output {
    command(home, config, args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run a3s {args:?}: {error}"))
}

fn run_with_stdin(
    home: &std::path::Path,
    config: &std::path::Path,
    args: &[&str],
    input: &str,
) -> Output {
    let mut command = command(home, config, args);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|error| panic!("failed to run a3s {args:?}: {error}"));
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.as_bytes())
        .expect("write protected token input");
    child.wait_with_output().expect("collect a3s output")
}

fn command(home: &std::path::Path, config: &std::path::Path, args: &[&str]) -> Command {
    let mut command = Command::new(a3s_bin());
    command
        .args(args)
        .env("HOME", home)
        .env("A3S_CONFIG_FILE", config)
        .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .env_remove("CODEX_HOME")
        .env_remove("A3S_KIMI_HOME")
        .env_remove("A3S_KIMI_DESKTOP_HOME")
        .env_remove("A3S_KIMI_BASE_URL")
        .env_remove("A3S_KIMI_OAUTH_HOST")
        .env_remove("KIMI_CODE_HOME")
        .env_remove("KIMI_SHARE_DIR")
        .env_remove("KIMI_DESKTOP_HOME")
        .env_remove("KIMI_CODE_BASE_URL")
        .env_remove("KIMI_CODE_OAUTH_HOST")
        .env("A3S_CODEBUDDY_CLI", home.join("bin/codebuddy"))
        .env("PATH", home.join("bin"))
        .env("RUST_BACKTRACE", "0");
    command
}

fn run_json(home: &std::path::Path, config: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let mut json_args = vec!["--json"];
    json_args.extend_from_slice(args);
    let output = run(home, config, &json_args);
    assert!(
        output.status.success(),
        "a3s {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "a3s {args:?} returned invalid JSON: {error}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn model_by_id<'a>(response: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    response["data"]["models"]
        .as_array()
        .expect("model list data")
        .iter()
        .find(|model| model["id"] == id)
        .unwrap_or_else(|| panic!("model {id:?} missing from {response}"))
}

fn fixture() -> (TempWorkspace, std::path::PathBuf, std::path::PathBuf) {
    let workspace = TempWorkspace::new("model-commands");
    let home = workspace.path("home");
    let config = workspace.path("config.acl");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        &config,
        r#"
default_model = "openai/gpt-test"
providers "openai" {
  apiKey = "not-used"
  baseUrl = "https://example.invalid/v1"
  models "gpt-test" {
    name = "GPT Test"
    reasoning = true
    toolCall = true
    limit = { context = 32000, output = 1024 }
  }
}
"#,
    )
    .unwrap();
    (workspace, home, config)
}

#[test]
fn model_list_use_current_and_reset_share_one_selection() {
    let (_workspace, home, config) = fixture();

    let list = run_json(&home, &config, &["model", "list"]);
    let configured = model_by_id(&list, "openai/gpt-test");
    assert_eq!(configured["source"], "config.acl");
    assert_eq!(configured["contextWindow"], 32_000);

    let config_path = run(&home, &config, &["config", "path"]);
    assert_eq!(
        String::from_utf8_lossy(&config_path.stdout).trim(),
        config.display().to_string()
    );

    let selected = run_json(&home, &config, &["model", "use", "openai/gpt-test"]);
    assert_eq!(selected["data"]["model"], "openai/gpt-test");
    assert_eq!(selected["data"]["effectiveModel"], "openai/gpt-test");
    assert_eq!(selected["data"]["effective"], true);
    let acl = std::fs::read_to_string(&config).unwrap();
    assert!(acl.contains(r#"default_model = "openai/gpt-test""#));
    assert!(!home.join(".a3s/tui/model-selection.json").exists());

    let current = run_json(&home, &config, &["model", "current"]);
    assert_eq!(current["data"]["model"], "openai/gpt-test");
    assert_eq!(current["data"]["source"], "config.acl");

    let reset = run_json(&home, &config, &["model", "reset"]);
    assert_eq!(reset["data"]["previous"], "openai/gpt-test");
    assert!(reset["data"]["effectiveModel"].is_null());
    assert!(!home.join(".a3s/tui/model-selection.json").exists());

    let current = run_json(&home, &config, &["model", "current"]);
    assert!(current["data"]["model"].is_null());
}

#[test]
fn model_use_rejects_routes_missing_from_the_catalog() {
    let (_workspace, home, config) = fixture();
    let before = std::fs::read_to_string(&config).unwrap();
    let output = run(&home, &config, &["model", "use", "openai/not-in-catalog"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not available"));
    assert_eq!(std::fs::read_to_string(&config).unwrap(), before);
}

#[test]
fn borrowed_login_routes_are_rejected_without_reading_account_files() {
    let (_workspace, home, config) = fixture();
    let before = std::fs::read_to_string(&config).unwrap();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude/.credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"claude-secret"}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex/auth.json"),
        r#"{"tokens":{"access_token":"codex-secret"}}"#,
    )
    .unwrap();
    let daimon = home.join(".config/kimi-desktop/daimon-share/daimon");
    std::fs::create_dir_all(&daimon).unwrap();
    std::fs::write(
        daimon.join("kimi-code-key.json"),
        r#"{"apiKey":"kimi-desktop-secret"}"#,
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".workbuddy")).unwrap();
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(
        home.join(".workbuddy/settings.json"),
        r#"{"privateAccountState":"workbuddy-secret"}"#,
    )
    .unwrap();
    let marker = home.join("codebuddy-was-started");
    make_executable(
        &home.join("bin/codebuddy"),
        &format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    );

    let list = run_json(&home, &config, &["model", "list"]);
    let rendered = list.to_string();
    for secret in [
        "claude-secret",
        "codex-secret",
        "kimi-desktop-secret",
        "workbuddy-secret",
    ] {
        assert!(!rendered.contains(secret), "model list leaked {secret}");
    }
    for id in [
        "claude-code/",
        "codex/",
        "kimi/",
        "workbuddy/",
        "codebuddy/",
    ] {
        assert!(
            !rendered.contains(id),
            "model list still advertised borrowed route {id}"
        );
    }
    assert!(!marker.exists(), "model list started the CodeBuddy CLI");

    for route in [
        "claude-code/claude-opus-4-6",
        "codex/gpt-5.2-codex",
        "kimi/k3-agent",
        "workbuddy/glm-5.1",
        "codebuddy/glm-5.1",
    ] {
        let selected = run(&home, &config, &["model", "use", route]);
        assert!(
            !selected.status.success(),
            "accepted borrowed route {route}"
        );
        let stderr = String::from_utf8_lossy(&selected.stderr);
        assert!(
            stderr.contains("no longer a model route"),
            "route {route} stderr: {stderr}"
        );
        assert_eq!(std::fs::read_to_string(&config).unwrap(), before);
    }
    assert!(!marker.exists(), "model use started the CodeBuddy CLI");
}

#[test]
fn offline_model_list_never_starts_account_discovery_processes() {
    let (_workspace, home, config) = fixture();
    std::fs::create_dir_all(home.join(".workbuddy")).unwrap();
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(home.join(".workbuddy/settings.json"), "{}").unwrap();
    let marker = home.join("codebuddy-was-started");
    make_executable(
        &home.join("bin/codebuddy"),
        &format!(
            "#!/bin/sh\ntouch '{}'\nprintf '%s\\n' '  - remote-only-model'\n",
            marker.display()
        ),
    );

    let response = run_json(&home, &config, &["--offline", "model", "list"]);
    assert_eq!(response["command"], "model.list");
    assert!(
        !marker.exists(),
        "offline model discovery must not start account provider processes"
    );
}

#[test]
fn a3s_os_gateway_models_are_selectable_without_storing_the_os_token() {
    let (_workspace, home, config) = fixture();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let size = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer os-secret"));
            let body = if request.starts_with("GET /api/v1/users/me ") {
                r#"{"data":{"displayName":"OS Test User"}}"#
            } else {
                assert!(request.starts_with("GET /api/v1/llm/models "));
                r#"{"data":[{"id":"gateway-model","context_length":128000}]}"#
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
    });
    let mut body = std::fs::read_to_string(&config).unwrap();
    body.push_str(&format!("\nos = \"http://{address}\"\n"));
    std::fs::write(&config, body).unwrap();

    let login = run_with_stdin(
        &home,
        &config,
        &["auth", "login", "os", "--token-stdin"],
        "os-secret",
    );
    assert!(login.status.success());
    let list = run_json(&home, &config, &["model", "list"]);
    let model = model_by_id(&list, "a3s-os/gateway-model");
    assert_eq!(model["source"], "A3S OS");
    assert_eq!(model["contextWindow"], 128_000);
    assert!(!list.to_string().contains("os-secret"));

    let selected = run(&home, &config, &["model", "use", "a3s-os/gateway-model"]);
    assert!(selected.status.success());
    let acl = std::fs::read_to_string(&config).unwrap();
    assert!(acl.contains(r#"default_model = "a3s-os/gateway-model""#));
    assert!(!acl.contains("os-secret"));
    server.join().unwrap();
}

#[test]
fn selecting_a_config_model_does_not_refresh_unrelated_accounts() {
    let (_workspace, home, config) = fixture();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(
        home.join(".codex/auth.json"),
        r#"{"tokens":{"access_token":"codex-secret"}}"#,
    )
    .unwrap();
    let probe = home.join("codex-probed");
    make_executable(
        &home.join("bin/codex"),
        &format!(
            "#!/bin/sh\nprintf probed > '{}'\n/bin/sleep 3\nprintf '%s\\n' '{{\"models\":[]}}'\n",
            probe.display()
        ),
    );

    let output = run(&home, &config, &["model", "use", "openai/gpt-test"]);
    assert!(output.status.success());
    assert!(!probe.exists(), "Codex was probed for a config-only route");
}

#[test]
fn model_list_discovers_os_models_without_starting_borrowed_clis() {
    let (_workspace, home, config) = fixture();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(
        home.join(".codex/auth.json"),
        r#"{"tokens":{"access_token":"codex-secret"}}"#,
    )
    .unwrap();
    let codex_started = home.join("codex-started");
    make_executable(
        &home.join("bin/codex"),
        &format!("#!/bin/sh\ntouch '{}'\n", codex_started.display()),
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let amount = stream.read(&mut request).unwrap();
            assert!(amount > 0, "expected an HTTP request");
            let request = String::from_utf8_lossy(&request[..amount]);
            let body = if request.starts_with("GET /api/v1/users/me ") {
                r#"{"data":{"displayName":"OS Slow User"}}"#
            } else {
                assert!(request.starts_with("GET /api/v1/llm/models "));
                r#"{"data":[{"id":"os-slow"}]}"#
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
    });
    let mut body = std::fs::read_to_string(&config).unwrap();
    body.push_str(&format!("\nos = \"http://{address}\"\n"));
    std::fs::write(&config, body).unwrap();
    assert!(run_with_stdin(
        &home,
        &config,
        &["auth", "login", "os", "--token-stdin"],
        "os-secret",
    )
    .status
    .success());

    let output = run(&home, &config, &["model", "list"]);
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("a3s-os/os-slow"));
    assert!(!stdout.contains("codex-secret"));
    assert!(!stdout.contains("codex/"));
    assert!(!codex_started.exists(), "model list started the Codex CLI");
}

#[test]
fn model_config_show_redacts_inline_and_environment_secrets() {
    let (_workspace, home, config) = fixture();
    std::fs::write(
        &config,
        r#"
default_model = "openai/gpt-test"
providers "openai" {
  apiKey = env("A3S_TEST_MISSING_MODEL_KEY")
  baseUrl = "https://example.invalid/v1"
  headers = { Authorization = "Bearer inline-secret" }
  models "gpt-test" {
    name = "GPT Test"
    reasoning = true
    toolCall = true
    limit = { context = 32000, output = 1024 }
  }
}
"#,
    )
    .unwrap();

    let shown = run_json(&home, &config, &["model", "config", "show"]);
    let provider = &shown["data"]["providers"][0];
    assert_eq!(provider["id"], "openai");
    assert_eq!(provider["credential"]["source"], "environment");
    assert_eq!(
        provider["credential"]["reference"],
        "A3S_TEST_MISSING_MODEL_KEY"
    );
    assert_eq!(provider["credential"]["available"], false);
    assert_eq!(provider["headers"][0]["source"], "inline");
    assert!(!shown.to_string().contains("inline-secret"));
}

#[test]
fn model_config_show_initializes_missing_storage() {
    let (_workspace, home, config) = fixture();
    std::fs::remove_file(&config).expect("remove seeded config");
    assert!(!config.exists(), "fixture must begin without model storage");

    let shown = run_json(&home, &config, &["model", "config", "show"]);

    assert_eq!(shown["data"]["providers"], serde_json::json!([]));
    assert!(
        config.is_file(),
        "model configuration storage must be initialized"
    );
    assert_eq!(
        std::fs::read_to_string(config).expect("initialized config"),
        ""
    );
}

#[test]
fn model_config_apply_preserves_environment_references_and_model_metadata() {
    let (_workspace, home, config) = fixture();
    let provider = serde_json::json!({
        "operation": "upsertProvider",
        "provider": {
            "id": "openai",
            "baseUrl": "https://api.example.test/v1",
            "credential": {
                "mode": "environment",
                "variable": "A3S_TEST_NEW_MODEL_KEY"
            },
            "headers": []
        }
    });
    let output = run_with_stdin(
        &home,
        &config,
        &["--json", "model", "config", "apply", "--input-stdin"],
        &provider.to_string(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!response.to_string().contains("__A3S_MODEL_CONFIG_ENV_"));

    let model = serde_json::json!({
        "operation": "upsertModel",
        "providerId": "openai",
        "model": {
            "id": "gpt-test",
            "name": "GPT Test Edited",
            "family": "gpt",
            "attachment": true,
            "reasoning": true,
            "toolCall": true,
            "temperature": false,
            "modalities": { "input": ["text", "image"], "output": ["text"] },
            "cost": { "input": 1.25, "output": 5.0 },
            "limit": { "context": 64000, "output": 4096 },
            "headers": []
        }
    });
    let output = run_with_stdin(
        &home,
        &config,
        &["--json", "model", "config", "apply", "--input-stdin"],
        &model.to_string(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let acl = std::fs::read_to_string(&config).unwrap();
    assert!(acl.contains(r#"api_key = env("A3S_TEST_NEW_MODEL_KEY")"#));
    assert!(acl.contains("GPT Test Edited"));
    assert!(acl.contains("context = 64000"));
    assert!(!acl.contains("__A3S_MODEL_CONFIG_ENV_"));

    let shown = run_json(&home, &config, &["model", "config", "show"]);
    let configured_model = &shown["data"]["providers"][0]["models"][0];
    assert_eq!(configured_model["name"], "GPT Test Edited");
    assert_eq!(configured_model["modalities"]["input"][1], "image");
    assert_eq!(configured_model["cost"]["input"], 1.25);
    assert_eq!(configured_model["limit"]["output"], 4096);
}

#[test]
fn model_config_cannot_remove_the_selected_default() {
    let (_workspace, home, config) = fixture();
    let before = std::fs::read_to_string(&config).unwrap();
    let mutation = serde_json::json!({
        "operation": "removeModel",
        "providerId": "openai",
        "modelId": "gpt-test"
    });
    let output = run_with_stdin(
        &home,
        &config,
        &["model", "config", "apply", "--input-stdin"],
        &mutation.to_string(),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("different default model"));
    assert_eq!(std::fs::read_to_string(&config).unwrap(), before);
}

#[test]
fn model_config_tests_a_provider_draft_without_echoing_credentials() {
    let (_workspace, home, config) = fixture();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let size = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..size]);
        assert!(request.starts_with("GET /v1/models "));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer provider-test-secret"));
        let body = r#"{"data":[]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let input = serde_json::json!({
        "provider": {
            "id": "custom",
            "baseUrl": format!("http://{address}/v1"),
            "credential": { "mode": "inline", "value": "provider-test-secret" },
            "headers": []
        }
    });
    let output = run_with_stdin(
        &home,
        &config,
        &["--json", "model", "config", "test", "--input-stdin"],
        &input.to_string(),
    );
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("provider-test-secret"));
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"]["providerId"], "custom");
    assert_eq!(response["data"]["status"], 200);
}
