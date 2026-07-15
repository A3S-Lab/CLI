#![cfg(unix)]

mod support;

use std::process::{Command, Output};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::{io::Read, io::Write, net::TcpListener};

use support::{a3s_bin, make_executable, TempWorkspace};

fn run(home: &std::path::Path, config: &std::path::Path, args: &[&str]) -> Output {
    Command::new(a3s_bin())
        .args(args)
        .env("HOME", home)
        .env("A3S_CONFIG_FILE", config)
        .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
        .env_remove("ANTHROPIC_AUTH_TOKEN")
        .env_remove("CODEX_HOME")
        .env("A3S_CODEBUDDY_CLI", home.join("bin/codebuddy"))
        .env("PATH", home.join("bin"))
        .env("RUST_BACKTRACE", "0")
        .output()
        .unwrap_or_else(|error| panic!("failed to run a3s {args:?}: {error}"))
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

    let list = run(&home, &config, &["model", "list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("openai/gpt-test"));
    assert!(stdout.contains("config.acl"));
    assert!(stdout.contains("context=32000"));

    let config_path = run(&home, &config, &["model", "config"]);
    assert_eq!(
        String::from_utf8_lossy(&config_path.stdout).trim(),
        config.display().to_string()
    );

    let use_model = run(&home, &config, &["model", "use", "openai/gpt-test"]);
    assert!(use_model.status.success());
    assert!(String::from_utf8_lossy(&use_model.stdout).contains("Active model: openai/gpt-test"));

    let selection = std::fs::read_to_string(home.join(".a3s/tui/model-selection.json")).unwrap();
    assert!(selection.contains(r#""source": "config""#));
    assert!(selection.contains(r#""model": "openai/gpt-test""#));

    let current = run(&home, &config, &["model", "current"]);
    assert_eq!(
        String::from_utf8_lossy(&current.stdout),
        "openai/gpt-test\n"
    );

    let reset = run(&home, &config, &["model", "reset"]);
    assert!(reset.status.success());
    assert!(!home.join(".a3s/tui/model-selection.json").exists());

    let current = run(&home, &config, &["model", "current"]);
    assert_eq!(
        String::from_utf8_lossy(&current.stdout),
        "openai/gpt-test (config.acl default)\n"
    );
}

#[test]
fn model_use_rejects_routes_missing_from_the_catalog() {
    let (_workspace, home, config) = fixture();
    let output = run(&home, &config, &["model", "use", "codex/not-entitled"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not available"));
    assert!(!home.join(".a3s/tui/model-selection.json").exists());
}

#[test]
fn claude_code_login_models_are_selectable_without_copying_credentials() {
    let (_workspace, home, config) = fixture();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude/.credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"claude-secret"}}"#,
    )
    .unwrap();
    std::fs::write(
        home.join(".claude.json"),
        r#"{"projects":{"demo":{"model":"claude-opus-4-6"}}}"#,
    )
    .unwrap();

    let list = run(&home, &config, &["model", "list"]);
    assert!(list.status.success());
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("claude-code/claude-opus-4-6"));
    assert!(stdout.contains("Claude Code"));
    assert!(!stdout.contains("claude-secret"));

    let selected = run(
        &home,
        &config,
        &["model", "use", "claude-code/claude-opus-4-6"],
    );
    assert!(selected.status.success());
    let preference = std::fs::read_to_string(home.join(".a3s/tui/model-selection.json")).unwrap();
    assert!(preference.contains(r#""source": "claude""#));
    assert!(!preference.contains("claude-secret"));
}

#[test]
fn codex_login_models_are_selectable_from_the_product_cache() {
    let (_workspace, home, config) = fixture();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::write(
        home.join(".codex/auth.json"),
        r#"{"tokens":{"id_token":"header.eyJleHAiOjF9.signature","access_token":"codex-secret","refresh_token":"codex-refresh-secret"}}"#,
    )
    .unwrap();
    std::fs::write(
        home.join(".codex/models_cache.json"),
        r#"{"models":[{"slug":"gpt-test-codex","visibility":"list","priority":1,"context_window":64000}]}"#,
    )
    .unwrap();

    let list = run(&home, &config, &["model", "list"]);
    assert!(list.status.success());
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("codex/gpt-test-codex"));
    assert!(stdout.contains("context=64000"));
    assert!(!stdout.contains("codex-secret"));

    let selected = run(&home, &config, &["model", "use", "codex/gpt-test-codex"]);
    assert!(selected.status.success());
    let preference = std::fs::read_to_string(home.join(".a3s/tui/model-selection.json")).unwrap();
    assert!(preference.contains(r#""source": "codex""#));
    assert!(!preference.contains("codex-secret"));
}

#[test]
fn workbuddy_login_models_are_discovered_without_copying_account_state() {
    let (_workspace, home, config) = fixture();
    std::fs::create_dir_all(home.join(".workbuddy")).unwrap();
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(
        home.join(".workbuddy/settings.json"),
        r#"{"privateAccountState":"workbuddy-secret"}"#,
    )
    .unwrap();
    make_executable(
        &home.join("bin/codebuddy"),
        "#!/bin/sh\nprintf '%s\\n' 'Currently supported models for your account:' '  - glm-5.1' '  - kimi-k2.7'\n",
    );

    let list = run(&home, &config, &["model", "list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("workbuddy/glm-5.1"));
    assert!(stdout.contains("workbuddy/kimi-k2.7"));
    assert!(stdout.contains("WorkBuddy"));
    assert!(!stdout.contains("workbuddy-secret"));

    let selected = run(&home, &config, &["model", "use", "workbuddy/glm-5.1"]);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let preference = std::fs::read_to_string(home.join(".a3s/tui/model-selection.json")).unwrap();
    assert!(preference.contains(r#""source": "codebuddy""#));
    assert!(!preference.contains("workbuddy-secret"));
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

    let login = run(&home, &config, &["login", "os-secret"]);
    assert!(login.status.success());
    let list = run(&home, &config, &["model", "list"]);
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        stdout.contains("a3s-os/gateway-model"),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&list.stderr)
    );
    assert!(stdout.contains("context=128000"));
    assert!(!stdout.contains("os-secret"));

    let selected = run(&home, &config, &["model", "use", "a3s-os/gateway-model"]);
    assert!(selected.status.success());
    let preference = std::fs::read_to_string(home.join(".a3s/tui/model-selection.json")).unwrap();
    assert!(preference.contains(r#""source": "os_gateway""#));
    assert!(!preference.contains("os-secret"));
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
fn model_list_discovers_slow_remote_sources_concurrently() {
    let (_workspace, home, config) = fixture();
    std::fs::create_dir_all(home.join(".codex")).unwrap();
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(
        home.join(".codex/auth.json"),
        r#"{"tokens":{"access_token":"codex-secret"}}"#,
    )
    .unwrap();
    let codex_started = home.join("codex-started");
    let os_started = home.join("os-started");
    let codex_saw_os = home.join("codex-saw-os");
    make_executable(
        &home.join("bin/codex"),
        &format!(
            "#!/bin/sh\nprintf started > '{}'\n\
             attempts=0\n\
             while [ ! -e '{}' ] && [ \"$attempts\" -lt 100 ]; do\n\
               /bin/sleep 0.05\n\
               attempts=$((attempts + 1))\n\
             done\n\
             if [ -e '{}' ]; then printf observed > '{}'; fi\n\
             printf '%s\\n' '{{\"models\":[{{\"slug\":\"codex-slow\",\"visibility\":\"list\"}}]}}'\n",
            codex_started.display(),
            os_started.display(),
            os_started.display(),
            codex_saw_os.display(),
        ),
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let os_saw_codex = Arc::new(AtomicBool::new(false));
    let os_saw_codex_from_server = Arc::clone(&os_saw_codex);
    let codex_started_from_server = codex_started.clone();
    let os_started_from_server = os_started.clone();
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
                std::fs::write(&os_started_from_server, "started").unwrap();
                for _ in 0..100 {
                    if codex_started_from_server.exists() {
                        os_saw_codex_from_server.store(true, Ordering::SeqCst);
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
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
    assert!(run(&home, &config, &["login", "os-secret"])
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
    assert!(stdout.contains("codex/codex-slow"));
    assert!(stdout.contains("a3s-os/os-slow"));
    assert!(
        codex_saw_os.exists() && os_saw_codex.load(Ordering::SeqCst),
        "remote discovery did not overlap both sources; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}
