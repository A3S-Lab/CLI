use super::*;

#[cfg(unix)]
// Keep process-backed fixtures from competing for spawn and stdio scheduling;
// startup budget tests must measure the product path, not test-harness load.
static PROCESS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_config() -> a3s_code_core::CodeConfig {
    a3s_code_core::CodeConfig::from_acl(
        r#"
                default_model = "openai/gpt-4o"

                providers "openai" {
                  api_key = "sk-test"

                  models "gpt-4o" {
                    name = "GPT-4o"
                  }
                }
            "#,
    )
    .expect("valid test config")
}

fn fixture_skill() -> &'static str {
    r#"---
name: fixture-report
description: Build fixture reports
allowed-tools: Read(*)
kind: instruction
---
# Fixture Report

Build a concise report.
"#
}

fn fixture_skill_digest() -> String {
    use sha2::{Digest, Sha256};

    format!("{:x}", Sha256::digest(fixture_skill().as_bytes()))
}

#[derive(Clone, Default)]
struct UseCallingLlm {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

impl UseCallingLlm {
    fn response(
        &self,
        tools: &[a3s_code_core::llm::ToolDefinition],
    ) -> anyhow::Result<a3s_code_core::LlmResponse> {
        let tool_names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        if !tool_names.contains(&"mcp__use_report__fixture_tool") {
            anyhow::bail!("Use MCP fixture tool was not inherited by the child: {tool_names:?}");
        }
        if let Some(disallowed) = tool_names
            .iter()
            .find(|name| !name.starts_with("mcp__use_"))
        {
            anyhow::bail!("Use child was exposed to disallowed tool '{disallowed}'");
        }

        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (content, stop_reason) = if call == 0 {
            (
                vec![a3s_code_core::ContentBlock::ToolUse {
                    id: "use-fixture-call".to_string(),
                    name: "mcp__use_report__fixture_tool".to_string(),
                    input: serde_json::json!({}),
                }],
                "tool_use",
            )
        } else {
            (
                vec![a3s_code_core::ContentBlock::Text {
                    text: "Use observed fixture-ok through the report capability.".to_string(),
                }],
                "end_turn",
            )
        };
        Ok(a3s_code_core::LlmResponse {
            message: a3s_code_core::Message {
                role: "assistant".to_string(),
                content,
                reasoning_content: None,
            },
            usage: a3s_code_core::TokenUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            stop_reason: Some(stop_reason.to_string()),
            token_logprobs: Vec::new(),
            meta: None,
        })
    }
}

#[async_trait::async_trait]
impl a3s_code_core::LlmClient for UseCallingLlm {
    async fn complete(
        &self,
        _messages: &[a3s_code_core::Message],
        _system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
    ) -> anyhow::Result<a3s_code_core::LlmResponse> {
        self.response(tools)
    }

    async fn complete_streaming(
        &self,
        _messages: &[a3s_code_core::Message],
        _system: Option<&str>,
        tools: &[a3s_code_core::llm::ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<tokio::sync::mpsc::Receiver<a3s_code_core::llm::StreamEvent>> {
        let response = self.response(tools)?;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            if let Some(text) = response
                .message
                .content
                .iter()
                .find_map(|block| match block {
                    a3s_code_core::ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            {
                let _ = tx
                    .send(a3s_code_core::llm::StreamEvent::TextDelta(text))
                    .await;
            }
            let _ = tx
                .send(a3s_code_core::llm::StreamEvent::Done(response))
                .await;
        });
        Ok(rx)
    }
}

#[test]
fn dedicated_use_worker_allows_only_use_mcp_tools() {
    let worker = use_worker_spec(&DesiredCapabilities::default()).into_agent_definition();
    assert_eq!(
        worker.confirmation_inheritance,
        Some(ConfirmationInheritance::InheritParent),
        "Use Ask decisions must reach the parent TUI instead of auto-approval"
    );
    assert_eq!(
        worker
            .permissions
            .check("mcp__use_browser__browser_open", &serde_json::json!({})),
        PermissionDecision::Allow
    );
    assert_eq!(
        worker
            .permissions
            .check("mcp__github__search", &serde_json::json!({})),
        PermissionDecision::Deny
    );
    assert_eq!(
        worker
            .permissions
            .check("read", &serde_json::json!({"file_path": "README.md"})),
        PermissionDecision::Deny
    );
    assert_eq!(
        worker
            .permissions
            .check("task", &serde_json::json!({"agent": "general"})),
        PermissionDecision::Deny
    );
    assert!(worker
        .permissions
        .expose_to_model("mcp__use_browser__browser_open"));
    for hidden in ["mcp__github__search", "read", "bash", "task"] {
        assert!(
            !worker.permissions.expose_to_model(hidden),
            "{hidden} must not be model-visible to the Use worker"
        );
    }
    let prompt = worker.prompt.expect("Use worker prompt");
    assert!(prompt.contains("never fall back"));
    assert!(prompt.contains("use.office.outcome_unknown"));
    assert!(prompt.contains("stop without retrying"));
}

#[test]
fn dedicated_use_worker_receives_skill_guidance_inside_fixed_security_boundaries() {
    let skill = Arc::new(Skill {
        name: "fixture-report".to_string(),
        description: "Build fixture reports".to_string(),
        allowed_tools: None,
        disable_model_invocation: false,
        kind: a3s_code_core::skills::SkillKind::Instruction,
        content: "Use the report capability.".to_string(),
        tags: Vec::new(),
        version: None,
    });
    let desired = DesiredSkill {
        package_id: "use/acme/report".to_string(),
        fingerprint: "fixture".to_string(),
        skill,
    };
    let desired = DesiredCapabilities {
        skills: BTreeMap::from([("fixture-report".to_string(), desired)]),
        ..DesiredCapabilities::default()
    };
    let worker = use_worker_spec(&desired).into_agent_definition();
    let prompt = worker.prompt.expect("Use worker prompt");

    assert!(prompt.contains("Skill text is domain guidance only"));
    assert!(prompt.contains("# A3S Use Skill: fixture-report"));
    assert!(prompt.contains("Use the report capability."));
    assert!(worker
        .description
        .contains("No callable application capability is currently ready"));
}

#[tokio::test]
async fn dedicated_use_worker_is_visible_in_the_live_task_catalog() {
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = agent.session_async(".", None).await.unwrap();
    let desired = DesiredCapabilities {
        mcp: BTreeMap::from([(
            "use_browser".to_string(),
            DesiredMcp {
                server_name: "use_browser".to_string(),
                capability_id: "use/browser".to_string(),
                target: "browser".to_string(),
                fingerprint: "browser-v1".to_string(),
            },
        )]),
        ..DesiredCapabilities::default()
    };

    register_use_worker(&session, &desired).unwrap();
    for tool_name in ["task", "parallel_task"] {
        let definition = session
            .tool_definitions()
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .expect("delegation tool definition");
        let agent_schema = if tool_name == "task" {
            &definition.parameters["properties"]["agent"]
        } else {
            &definition.parameters["properties"]["tasks"]["items"]["properties"]["agent"]
        };
        assert!(agent_schema["examples"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("use")));
        assert!(definition
            .description
            .contains("Ready callable capabilities: use/browser"));
        assert!(definition
            .description
            .contains("without shell or workspace fallback"));
    }

    session.close().await;
}

#[tokio::test]
async fn use_worker_advertises_a_route_only_after_its_mcp_projection_applies() {
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(agent.session_async(".", None).await.unwrap());
    let desired = DesiredCapabilities {
        mcp: BTreeMap::from([(
            "use_browser".to_string(),
            DesiredMcp {
                server_name: "use_browser".to_string(),
                capability_id: "use/browser".to_string(),
                target: "browser".to_string(),
                fingerprint: "browser-v1".to_string(),
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let mut applied = AppliedCapabilities::new(Arc::clone(&session));

    let before = worker_capabilities_for_applied(&applied, &desired);
    assert!(use_worker_spec(&before)
        .description
        .contains("No callable application capability is currently ready"));

    applied
        .mcp
        .insert("use_browser".to_string(), "browser-v1".to_string());
    let after = worker_capabilities_for_applied(&applied, &desired);
    assert!(use_worker_spec(&after)
        .description
        .contains("Ready callable capabilities: use/browser"));

    session.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn process_client_resolves_unified_snapshot_and_managed_skill() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("package");
    std::fs::create_dir_all(package.join("skills/fixture-report")).unwrap();
    std::fs::write(
        package.join("skills/fixture-report/SKILL.md"),
        fixture_skill(),
    )
    .unwrap();

    let binding = serde_json::json!({
        "id": "use/acme/report",
        "route": "report",
        "version": "1.0.0",
        "origin": "extension",
        "packageRoot": package,
        "enabled": true,
        "surfaces": ["skill"],
        "skills": [{
            "path": package.join("skills/fixture-report/SKILL.md"),
            "sha256": fixture_skill_digest()
        }]
    });
    let snapshot = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 1,
            "generation": 7,
            "revision": "1111111111111111111111111111111111111111111111111111111111111111",
            "capabilities": [binding]
        }}
    });
    let executable = temp.path().join("a3s-use-fixture");
    let script = format!(
        "#!/bin/sh\ncase \"$1 $2\" in\n  \"capability snapshot\") printf '%s\\n' '{}' ;;\n  *) exit 2 ;;\nesac\n",
        shell_single_quote(&snapshot.to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let client = UseRegistryClient::for_test(executable, temp.path().to_path_buf());
    let snapshot = client.snapshot().await.unwrap();
    let desired = client.stable_desired(snapshot).await.unwrap();

    assert_eq!(desired.generation, 7);
    assert!(desired.mcp.is_empty());
    assert_eq!(desired.skills.len(), 1);
    assert_eq!(
        desired.skills["fixture-report"].skill.description,
        "Build fixture reports"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn generation_watch_hot_plugs_and_disables_skill_and_mcp() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("package");
    std::fs::create_dir_all(package.join("skills/fixture-report")).unwrap();
    std::fs::write(
        package.join("skills/fixture-report/SKILL.md"),
        fixture_skill(),
    )
    .unwrap();
    let state = temp.path().join("generation");
    let mcp_log = temp.path().join("mcp-args.log");
    std::fs::write(&state, "1\n").unwrap();

    let route = serde_json::json!({
        "id": "use/acme/report",
        "route": "report",
        "version": "1.0.0",
        "origin": "extension",
        "packageRoot": package,
        "enabled": true,
        "surfaces": ["mcp", "skill"],
        "mcp": {"target": "acme/report", "transport": "stdio"},
        "skills": [{
            "path": package.join("skills/fixture-report/SKILL.md"),
            "sha256": fixture_skill_digest()
        }]
    });
    let mut disabled_route = route.clone();
    disabled_route["enabled"] = serde_json::Value::Bool(false);
    let snapshot_one = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 1,
            "generation": 1,
            "revision": "1111111111111111111111111111111111111111111111111111111111111111",
            "capabilities": [route]
        }}
    });
    let snapshot_two = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 1,
            "generation": 2,
            "revision": "2222222222222222222222222222222222222222222222222222222222222222",
            "capabilities": [disabled_route]
        }}
    });
    let watch_two = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {
            "changed": true,
            "registry": snapshot_two["data"]["registry"]
        }
    });
    let watch_one = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {
            "changed": true,
            "registry": snapshot_one["data"]["registry"]
        }
    });
    let executable = temp.path().join("a3s-use-fixture");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "mcp" ] && [ "$2" = "serve" ]; then
  printf '%s\n' "$*" > '{}'
  while IFS= read -r line; do
    case "$line" in
      *'"method":"initialize"'*)
        printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2024-11-05","capabilities":{{}},"serverInfo":{{"name":"fixture","version":"1.0.0"}}}}}}'
        ;;
      *'"method":"notifications/initialized"'*) ;;
      *'"method":"tools/list"'*)
        printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"fixture_tool","description":"Fixture tool","inputSchema":{{"type":"object"}}}}]}}}}'
        ;;
      *'"method":"tools/call"'*)
        printf '%s\n' '{{"jsonrpc":"2.0","id":3,"result":{{"content":[{{"type":"text","text":"fixture-ok"}}],"isError":false}}}}'
        ;;
    esac
  done
  exit 0
fi

case "$1 $2" in
  "capability snapshot")
    if [ "$(tr -d '\n' < '{}')" = "1" ]; then
      printf '%s\n' '{}'
    else
      printf '%s\n' '{}'
    fi
    ;;
  "capability watch")
    if [ "$4" = "0" ]; then
      printf '%s\n' '{}'
    else
      while [ "$(tr -d '\n' < '{}')" = "1" ]; do sleep 0.05; done
      printf '%s\n' '{}'
    fi
    ;;
  *) exit 2 ;;
esac
"#,
        mcp_log.display(),
        state.display(),
        shell_single_quote(&snapshot_one.to_string()),
        shell_single_quote(&snapshot_two.to_string()),
        shell_single_quote(&watch_one.to_string()),
        state.display(),
        shell_single_quote(&watch_two.to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let workspace = tempfile::tempdir().unwrap();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(workspace.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let cancellation = CancellationToken::new();
    let (handle, warning) = start(
        executable,
        workspace.path().to_path_buf(),
        cancellation,
        Arc::clone(&session),
    )
    .await;
    if let Some(warning) = warning {
        assert!(
            warning.contains("startup discovery exceeded"),
            "unexpected startup warning: {warning}"
        );
    }
    wait_for_capabilities(&session, true).await;
    assert_eq!(
        std::fs::read_to_string(&mcp_log).unwrap(),
        "mcp serve acme/report\n"
    );

    let replacement_workspace = tempfile::tempdir().unwrap();
    let use_client = Arc::new(UseCallingLlm::default());
    let replacement = Arc::new(
        agent
            .session_async(
                replacement_workspace.path().display().to_string(),
                Some(a3s_code_core::SessionOptions::new().with_llm_client(use_client.clone())),
            )
            .await
            .unwrap(),
    );
    handle.replace_session(Arc::clone(&replacement));
    assert!(
        replacement
            .skill_names()
            .iter()
            .any(|name| name == "fixture-report"),
        "replacement must receive live skills synchronously"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if replacement
                .tool_names()
                .iter()
                .any(|name| name == "mcp__use_report__fixture_tool")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("replacement session must reconnect live MCP");
    session.close().await;

    let web_workspace = tempfile::tempdir().unwrap();
    let web_session = Arc::new(
        agent
            .session_async(web_workspace.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    handle.attach_session(Arc::clone(&web_session));
    wait_for_capabilities(&web_session, true).await;
    assert_eq!(
        handle.inner.projections.lock().unwrap().len(),
        2,
        "one coordinator must project into the TUI and Web sessions"
    );

    let called = replacement
        .tool("mcp__use_report__fixture_tool", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(called.exit_code, 0, "{}", called.output);
    assert!(called.output.contains("fixture-ok"));

    let delegated = replacement
        .tool(
            "task",
            serde_json::json!({
                "agent": "use",
                "description": "Call the report capability",
                "prompt": "Call the report fixture and return the observed result.",
                "max_steps": 3
            }),
        )
        .await
        .unwrap();
    assert_eq!(delegated.exit_code, 0, "{}", delegated.output);
    assert!(delegated.output.contains("Use observed fixture-ok"));
    assert_eq!(
        use_client.calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the Use child should call MCP once and then return its observation"
    );

    std::fs::write(&state, "2\n").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let skill_gone = !replacement
                .skill_names()
                .iter()
                .any(|name| name == "fixture-report");
            let mcp_gone = !replacement
                .tool_names()
                .iter()
                .any(|name| name == "mcp__use_report__fixture_tool");
            if skill_gone && mcp_gone {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("generation 2 must remove live capabilities");
    wait_for_capabilities(&web_session, false).await;
    let task_definition = replacement
        .tool_definitions()
        .into_iter()
        .find(|tool| tool.name == "task")
        .expect("task definition after capability removal");
    assert!(task_definition
        .description
        .contains("No callable application capability is currently ready"));
    assert!(!task_definition.description.contains("use/acme/report"));

    handle.detach_session(web_session.session_id()).await;
    handle.shutdown().await;
    web_session.close().await;
    replacement.close().await;
}

/// Crosses the real `a3s-use` process boundary instead of using the shell
/// contract fixture above. The monorepo orchestration recipe builds Use and
/// supplies `A3S_USE_E2E_BIN`; the test stays ignored for standalone CLI
/// checkouts where that independently released component is unavailable.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires A3S_USE_E2E_BIN pointing to a real a3s-use binary"]
async fn real_use_process_converges_install_upgrade_rebuild_disable_and_enable() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let binary = std::env::var_os("A3S_USE_E2E_BIN")
        .map(PathBuf::from)
        .expect("A3S_USE_E2E_BIN must point to the real a3s-use binary");
    let binary = std::fs::canonicalize(&binary)
        .unwrap_or_else(|error| panic!("failed to resolve {}: {error}", binary.display()));
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let package = temp.path().join("package");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(package.join("bin")).unwrap();
    std::fs::create_dir_all(package.join("skills/fixture-report")).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();

    write_real_extension_fixture(&package, "1.0.0", "fixture-v1");
    let executable = temp.path().join("a3s-use-e2e");
    let script = format!(
        "#!/bin/sh\nexport A3S_USE_HOME='{}'\nexec '{}' \"$@\"\n",
        shell_single_quote(&home.display().to_string()),
        shell_single_quote(&binary.display().to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    run_real_use(
        &executable,
        vec![
            "component".into(),
            "install".into(),
            "acme/report".into(),
            "--from".into(),
            package.display().to_string(),
            "--allow-unsigned".into(),
            "--json".into(),
        ],
    )
    .await;

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    let cancellation = CancellationToken::new();
    let (handle, warning) = start(
        executable.clone(),
        workspace.clone(),
        cancellation.clone(),
        Arc::clone(&session),
    )
    .await;
    assert!(warning.is_none(), "{warning:?}");
    wait_for_capabilities(&session, true).await;
    assert_fixture_tool(&session, "fixture-v1").await;

    write_real_extension_fixture(&package, "2.0.0", "fixture-v2");
    run_real_use(
        &executable,
        vec![
            "component".into(),
            "install".into(),
            "acme/report".into(),
            "--from".into(),
            package.display().to_string(),
            "--allow-unsigned".into(),
            "--force".into(),
            "--json".into(),
        ],
    )
    .await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if fixture_tool_output(&session)
                .await
                .is_some_and(|output| output.contains("fixture-v2"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("the real generation upgrade must replace the MCP process");

    let replacement_workspace = temp.path().join("replacement-workspace");
    std::fs::create_dir_all(&replacement_workspace).unwrap();
    let replacement = Arc::new(
        agent
            .session_async(replacement_workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    handle.replace_session(Arc::clone(&replacement));
    assert!(replacement
        .skill_names()
        .iter()
        .any(|name| name == "fixture-report"));
    wait_for_capabilities(&replacement, true).await;
    assert_fixture_tool(&replacement, "fixture-v2").await;
    session.close().await;

    run_real_use(
        &executable,
        vec![
            "extension".into(),
            "disable".into(),
            "acme/report".into(),
            "--json".into(),
        ],
    )
    .await;
    wait_for_capabilities(&replacement, false).await;

    run_real_use(
        &executable,
        vec![
            "extension".into(),
            "enable".into(),
            "acme/report".into(),
            "--json".into(),
        ],
    )
    .await;
    wait_for_capabilities(&replacement, true).await;
    assert_fixture_tool(&replacement, "fixture-v2").await;

    cancellation.cancel();
    drop(handle);
    replacement.close().await;
}

#[cfg(unix)]
fn write_real_extension_fixture(package: &Path, version: &str, response: &str) {
    use std::os::unix::fs::PermissionsExt;

    let manifest = format!(
        r#"extension "acme/report" {{
  schema_version = 1
  version = "{version}"
  route = "report"
  actions = ["read"]

  mcp {{
    executable = "bin/report-mcp"
    args = []
    transport = "stdio"
  }}

  skill {{
    path = "skills/fixture-report/SKILL.md"
  }}
}}
"#,
    );
    std::fs::write(package.join("a3s-use-extension.acl"), manifest).unwrap();
    std::fs::write(
        package.join("skills/fixture-report/SKILL.md"),
        fixture_skill(),
    )
    .unwrap();
    let server = package.join("bin/report-mcp");
    let script = format!(
        r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([^,}}]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{}},\"serverInfo\":{{\"name\":\"real-use-fixture\",\"version\":\"{version}\"}}}}}}"
      ;;
    *'"method":"notifications/initialized"'*) ;;
    *'"method":"tools/list"'*)
      printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"tools\":[{{\"name\":\"fixture_tool\",\"description\":\"Real Use fixture\",\"inputSchema\":{{\"type\":\"object\"}}}}]}}}}"
      ;;
    *'"method":"tools/call"'*)
      printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{response}\"}}],\"isError\":false}}}}"
      ;;
  esac
done
"#,
    );
    std::fs::write(&server, script).unwrap();
    let mut permissions = std::fs::metadata(&server).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&server, permissions).unwrap();
}

#[cfg(unix)]
async fn run_real_use(executable: &Path, args: Vec<String>) -> serde_json::Value {
    let output = tokio::process::Command::new(executable)
        .args(&args)
        .output()
        .await
        .unwrap_or_else(|error| panic!("failed to run {:?}: {error}", args));
    assert!(
        output.status.success(),
        "a3s-use {:?} failed with {}: {}",
        args,
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "a3s-use {:?} returned invalid JSON: {error}: {}",
            args,
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[cfg(unix)]
async fn wait_for_capabilities(session: &AgentSession, present: bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let skill_present = session
                .skill_names()
                .iter()
                .any(|name| name == "fixture-report");
            let tool_present = session
                .tool_names()
                .iter()
                .any(|name| name == "mcp__use_report__fixture_tool");
            if skill_present == present && tool_present == present {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("capabilities did not converge to present={present}"));
}

#[cfg(unix)]
async fn fixture_tool_output(session: &AgentSession) -> Option<String> {
    session
        .tool("mcp__use_report__fixture_tool", serde_json::json!({}))
        .await
        .ok()
        .map(|result| result.output)
}

#[cfg(unix)]
async fn assert_fixture_tool(session: &AgentSession, expected: &str) {
    let output = fixture_tool_output(session)
        .await
        .expect("fixture tool must be callable");
    assert!(output.contains(expected), "{output}");
}

#[cfg(unix)]
#[tokio::test]
async fn startup_discovery_respects_its_budget() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("slow-a3s-use");
    std::fs::write(&executable, "#!/bin/sh\nsleep 5\n").unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temp.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let started = std::time::Instant::now();
    let (handle, warning) = start_with_budget(
        executable,
        temp.path().to_path_buf(),
        CancellationToken::new(),
        Arc::clone(&session),
        Duration::from_millis(50),
    )
    .await;

    assert!(
        started.elapsed() < Duration::from_millis(500),
        "startup blocked for {:?}",
        started.elapsed()
    );
    assert!(
        warning
            .as_deref()
            .is_some_and(|message| message.contains("exceeded 50 ms")),
        "{warning:?}"
    );

    drop(handle);
    session.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn startup_gives_initial_mcp_more_time_than_registry_discovery() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join("slow-initial-mcp");
    let snapshot = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": {
            "schemaVersion": 1,
            "generation": 1,
            "revision": "1111111111111111111111111111111111111111111111111111111111111111",
            "capabilities": [{
                "id": "use/acme/report",
                "route": "report",
                "version": "1.0.0",
                "origin": "extension",
                "enabled": true,
                "packageRoot": temp.path(),
                "surfaces": ["mcp"],
                "mcp": {"target": "acme/report", "transport": "stdio"},
                "skills": []
            }]
        }}
    });
    let unchanged = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"changed": false, "registry": null}
    });
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "mcp" ] && [ "$2" = "serve" ]; then
  while IFS= read -r line; do
    case "$line" in
      *'"method":"initialize"'*)
        sleep 1.25
        printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"2024-11-05","capabilities":{{}},"serverInfo":{{"name":"slow-fixture","version":"1.0.0"}}}}}}'
        ;;
      *'"method":"notifications/initialized"'*) ;;
      *'"method":"tools/list"'*)
        printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"fixture_tool","description":"Fixture tool","inputSchema":{{"type":"object"}},"annotations":{{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}}}}]}}}}'
        ;;
    esac
  done
  exit 0
fi

case "$1 $2" in
  "capability snapshot") printf '%s\n' '{}' ;;
  "capability watch") printf '%s\n' '{}' ;;
  *) exit 2 ;;
esac
"#,
        shell_single_quote(&snapshot.to_string()),
        shell_single_quote(&unchanged.to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temp.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    assert!(
        STARTUP_PROJECTION_BUDGET > STARTUP_DISCOVERY_BUDGET,
        "production startup must reserve more time for initial MCP projection"
    );
    let started = std::time::Instant::now();
    let (handle, warning) = start_with_budgets(
        executable,
        temp.path().to_path_buf(),
        CancellationToken::new(),
        Arc::clone(&session),
        Duration::from_secs(10),
        Duration::from_secs(10),
    )
    .await;

    assert!(warning.is_none(), "{warning:?}");
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "fixture did not exercise the longer projection budget"
    );
    assert!(
        session
            .tool_names()
            .iter()
            .any(|name| name == "mcp__use_report__fixture_tool"),
        "initial MCP route was not ready when startup returned"
    );

    handle.shutdown().await;
    session.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn timed_out_startup_discovery_converges_from_the_watch_generation() {
    use std::os::unix::fs::PermissionsExt;

    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("package");
    std::fs::create_dir_all(package.join("skills/fixture-report")).unwrap();
    std::fs::write(
        package.join("skills/fixture-report/SKILL.md"),
        fixture_skill(),
    )
    .unwrap();

    let registry = serde_json::json!({
        "schemaVersion": 1,
        "generation": 1,
        "revision": "1111111111111111111111111111111111111111111111111111111111111111",
        "capabilities": [{
            "id": "use/acme/report",
            "route": "report",
            "version": "1.0.0",
            "origin": "extension",
            "packageRoot": package,
            "enabled": true,
            "surfaces": ["skill"],
            "skills": [{
                "path": package.join("skills/fixture-report/SKILL.md"),
                "sha256": fixture_skill_digest()
            }]
        }]
    });
    let snapshot = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"registry": registry}
    });
    let changed = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"changed": true, "registry": registry}
    });
    let unchanged = serde_json::json!({
        "schemaVersion": 1,
        "ok": true,
        "data": {"changed": false}
    });
    let executable = temp.path().join("slow-first-snapshot");
    let script = format!(
        r#"#!/bin/sh
case "$1 $2" in
  "capability snapshot")
    sleep 0.1
    printf '%s\n' '{}'
    ;;
  "capability watch")
    if [ "$4" = "0" ]; then
      printf '%s\n' '{}'
    else
      printf '%s\n' '{}'
    fi
    ;;
  *) exit 2 ;;
esac
"#,
        shell_single_quote(&snapshot.to_string()),
        shell_single_quote(&changed.to_string()),
        shell_single_quote(&unchanged.to_string()),
    );
    std::fs::write(&executable, script).unwrap();
    let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executable, permissions).unwrap();

    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temp.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let (handle, warning) = start_with_budget(
        executable,
        temp.path().to_path_buf(),
        CancellationToken::new(),
        Arc::clone(&session),
        Duration::from_millis(20),
    )
    .await;

    assert!(
        warning
            .as_deref()
            .is_some_and(|message| message.contains("exceeded 20 ms")),
        "{warning:?}"
    );
    // This bounds eventual background convergence, not the 20 ms startup SLA
    // asserted above. The full test binary starts many external processes in
    // parallel, so leave enough scheduling headroom without changing any
    // production command or retry timeout.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !session
            .skill_names()
            .iter()
            .any(|name| name == "fixture-report")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the watch loop must recover the generation skipped at startup");

    drop(handle);
    session.close().await;
}

#[tokio::test]
async fn replacement_session_receives_live_skills_without_waiting_for_projection() {
    let temp = tempfile::tempdir().unwrap();
    let first_workspace = temp.path().join("first");
    let second_workspace = temp.path().join("second");
    std::fs::create_dir_all(&first_workspace).unwrap();
    std::fs::create_dir_all(&second_workspace).unwrap();
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let first = Arc::new(
        agent
            .session_async(first_workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    let second = Arc::new(
        agent
            .session_async(second_workspace.display().to_string(), None)
            .await
            .unwrap(),
    );
    let skill_path = temp.path().join("SKILL.md");
    std::fs::write(&skill_path, fixture_skill()).unwrap();
    let skill = Arc::new(Skill::from_file(&skill_path).unwrap());
    let desired = DesiredCapabilities {
        generation: 2,
        revision: "2222222222222222222222222222222222222222222222222222222222222222".to_string(),
        skills: BTreeMap::from([(
            "fixture-report".to_string(),
            DesiredSkill {
                package_id: "use/acme/report".to_string(),
                fingerprint: "v2".to_string(),
                skill,
            },
        )]),
        ..DesiredCapabilities::default()
    };
    let (desired_tx, _) = watch::channel(Arc::new(desired));
    let handle = UseRegistryHandle {
        inner: Arc::new(UseRegistryInner {
            executable: temp.path().join("unused-a3s-use"),
            desired_tx,
            cancellation: CancellationToken::new(),
            projections: Mutex::new(BTreeMap::new()),
            registry_task: Mutex::new(None),
        }),
    };
    handle.replace_session(Arc::clone(&first));

    let started = std::time::Instant::now();
    handle.replace_session(Arc::clone(&second));
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(second
        .skill_names()
        .iter()
        .any(|name| name == "fixture-report"));
    assert_eq!(
        handle
            .inner
            .projections
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![PRIMARY_ATTACHMENT.to_string()]
    );

    handle.shutdown().await;
    first.close().await;
    second.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn partial_reconciliation_never_advances_the_generation() {
    let _process_test_guard = PROCESS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let skill_path = temp.path().join("SKILL.md");
    std::fs::write(&skill_path, fixture_skill()).unwrap();
    let skill = Arc::new(Skill::from_file(&skill_path).unwrap());
    let agent = a3s_code_core::Agent::from_config(test_config())
        .await
        .unwrap();
    let session = Arc::new(
        agent
            .session_async(temp.path().display().to_string(), None)
            .await
            .unwrap(),
    );
    let mut applied = AppliedCapabilities::new(Arc::clone(&session));
    let desired = DesiredCapabilities {
        generation: 9,
        revision: "9999999999999999999999999999999999999999999999999999999999999999".to_string(),
        mcp: BTreeMap::from([(
            "use_broken".to_string(),
            DesiredMcp {
                server_name: "use_broken".to_string(),
                capability_id: "use/acme/broken".to_string(),
                target: "acme/broken".to_string(),
                fingerprint: "mcp-v1".to_string(),
            },
        )]),
        skills: BTreeMap::from([(
            "fixture-report".to_string(),
            DesiredSkill {
                package_id: "acme/broken".to_string(),
                fingerprint: "skill-v1".to_string(),
                skill,
            },
        )]),
        warnings: Vec::new(),
    };

    let error = reconcile(Path::new("/usr/bin/false"), &mut applied, &desired)
        .await
        .expect_err("a process that exits immediately cannot become an MCP server");
    assert!(error.to_string().contains("failed to attach"), "{error:#}");
    assert_eq!(applied.generation, 0);
    assert!(applied.revision.is_empty());
    assert_eq!(applied.skills["fixture-report"], "skill-v1");
    assert!(session
        .skill_names()
        .iter()
        .any(|name| name == "fixture-report"));

    session.close().await;
}

#[test]
fn retry_delay_is_bounded() {
    assert_eq!(next_retry_delay(Duration::from_secs(20)), MAX_RETRY_DELAY);
    assert_eq!(next_retry_delay(MAX_RETRY_DELAY), MAX_RETRY_DELAY);
}

#[test]
fn response_envelope_requires_the_supported_schema() {
    validate_envelope_schema(&serde_json::json!({"schemaVersion": 1})).unwrap();

    let future = validate_envelope_schema(&serde_json::json!({"schemaVersion": 2}))
        .unwrap_err()
        .to_string();
    assert!(future.contains("schema version 2"), "{future}");

    let missing = validate_envelope_schema(&serde_json::json!({}))
        .unwrap_err()
        .to_string();
    assert!(missing.contains("schema version missing"), "{missing}");
}

#[test]
fn capability_snapshot_rejects_an_invalid_skill_digest() {
    let temp = tempfile::tempdir().unwrap();
    let snapshot: RegistrySnapshot = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "generation": 1,
        "revision": "1111111111111111111111111111111111111111111111111111111111111111",
        "capabilities": [{
            "id": "use/acme/report",
            "route": "report",
            "version": "1.0.0",
            "origin": "extension",
            "packageRoot": temp.path(),
            "enabled": true,
            "surfaces": ["skill"],
            "skills": [{
                "path": temp.path().join("SKILL.md"),
                "sha256": "not-a-sha256"
            }]
        }]
    }))
    .unwrap();

    let error = validate_snapshot(&snapshot)
        .expect_err("Skill content identities must be lowercase SHA-256 digests");
    assert!(error.to_string().contains("Skill digest"), "{error:#}");
}

#[tokio::test]
async fn managed_skill_rejects_content_that_does_not_match_its_digest() {
    let package = tempfile::tempdir().unwrap();
    let path = package.path().join("SKILL.md");
    tokio::fs::write(&path, fixture_skill()).await.unwrap();

    let wrong_digest = "0".repeat(64);
    let error = load_managed_skill(package.path(), &path, Some(&wrong_digest))
        .await
        .expect_err("the registry digest must bind the exact Skill bytes");
    assert!(
        error.to_string().contains("digest does not match"),
        "{error:#}"
    );
}

#[test]
fn skill_content_fingerprint_changes_without_restarting_its_mcp_surface() {
    let package = tempfile::tempdir().unwrap();
    let mcp = ProjectedMcpSurface {
        target: "acme/report".to_string(),
        transport: ProjectedMcpTransport::Stdio,
    };
    let mut skill = ProjectedSkillSurface {
        path: package.path().join("SKILL.md"),
        sha256: "1".repeat(64),
    };
    let binding = CapabilityBinding {
        id: "use/acme/report".to_string(),
        route: "report".to_string(),
        version: "1.0.0".to_string(),
        origin: CapabilityOrigin::Extension,
        enabled: true,
        package_root: package.path().to_path_buf(),
        surfaces: vec!["mcp".to_string(), "skill".to_string()],
        mcp: Some(mcp.clone()),
        skills: vec![skill.clone()],
    };

    let mcp_before = mcp_fingerprint(&binding, &mcp).unwrap();
    let skill_before = skill_fingerprint(&binding, &skill).unwrap();
    skill.sha256 = "2".repeat(64);

    assert_eq!(mcp_fingerprint(&binding, &mcp).unwrap(), mcp_before);
    assert_ne!(skill_fingerprint(&binding, &skill).unwrap(), skill_before);
}

#[tokio::test]
async fn command_output_reader_discards_bytes_beyond_its_limit() {
    use tokio::io::AsyncWriteExt;

    let (mut writer, reader) = tokio::io::duplex(128);
    let write = tokio::spawn(async move {
        writer.write_all(&[b'x'; 64]).await.unwrap();
        writer.shutdown().await.unwrap();
    });
    let output = read_limited(reader, 16).await.unwrap();
    write.await.unwrap();

    assert_eq!(output.bytes, vec![b'x'; 16]);
    assert!(output.exceeded);
}

#[cfg(unix)]
fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}
