use super::*;
use a3s_code_core::llm::{
    ContentBlock, LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
};
use a3s_code_core::permissions::{PermissionChecker, PermissionDecision};
use a3s_code_core::tools::{Tool, ToolContext, ToolOutput};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Mutex as StdMutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct TestSandbox;

#[async_trait::async_trait]
impl a3s_code_core::sandbox::BashSandbox for TestSandbox {
    async fn exec_command(
        &self,
        _command: &str,
        _guest_workspace: &str,
    ) -> anyhow::Result<a3s_code_core::sandbox::SandboxOutput> {
        Ok(a3s_code_core::sandbox::SandboxOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    async fn shutdown(&self) {}
}

fn checker(
    workspace: &Path,
    mode: Mode,
    sandboxed: bool,
) -> (TuiHitlPermissionChecker, TuiExecutionPolicy) {
    let gate = DeepResearchReportToolGate::default();
    gate.set_workspace(workspace);
    let sandbox =
        sandboxed.then(|| Arc::new(TestSandbox) as Arc<dyn a3s_code_core::sandbox::BashSandbox>);
    let execution = TuiExecutionPolicy::for_workspace(mode, workspace.to_path_buf(), sandbox);
    let checker = TuiHitlPermissionChecker::with_execution_policy(
        tui_permission_policy(),
        gate,
        execution.clone(),
    );
    (checker, execution)
}

#[test]
fn default_mode_is_quiet_inside_enforced_boundaries() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, _) = checker(workspace.path(), Mode::Default, true);

    for (tool, args) in [
        (
            "write",
            serde_json::json!({"file_path": "README.md", "content": "updated"}),
        ),
        ("bash", serde_json::json!({"command": "cargo test"})),
        (
            "task",
            serde_json::json!({"prompt": "inspect and implement the change"}),
        ),
        (
            "Skill",
            serde_json::json!({"skill_name": "workspace-maintenance"}),
        ),
        (
            "mcp__github__get_issue",
            serde_json::json!({"issue_number": 1}),
        ),
    ] {
        assert_eq!(
            checker.check(tool, &args),
            PermissionDecision::Allow,
            "Default should not enter HITL for bounded {tool}"
        );
    }

    assert_eq!(
        checker.check(
            "bash",
            &serde_json::json!({
                "command": "cargo test",
                "sandbox_permissions": "require_escalated",
                "justification": "Needs an approved host capability."
            })
        ),
        PermissionDecision::Ask
    );
    assert_eq!(
        checker.check(
            "git",
            &serde_json::json!({"command": "checkout", "ref": "feature"})
        ),
        PermissionDecision::Ask
    );
    for path in [
        ".git/config",
        ".a3s/permissions.acl",
        ".codex/config",
        ".claude/settings.json",
        ".mcp.json",
    ] {
        assert_eq!(
            checker.check(
                "write",
                &serde_json::json!({"file_path": path, "content": "changed"})
            ),
            PermissionDecision::Ask,
            "Default should require explicit approval for {path}"
        );
    }
    assert_eq!(
        checker.check("bash", &serde_json::json!({"command": "rm -rf /"})),
        PermissionDecision::Deny
    );
}

#[test]
fn missing_process_sandbox_prompts_in_default_and_denies_in_auto() {
    let workspace = tempfile::tempdir().unwrap();
    let (default_checker, _) = checker(workspace.path(), Mode::Default, false);
    let (auto_checker, _) = checker(workspace.path(), Mode::Auto, false);

    assert_eq!(
        default_checker.check("bash", &serde_json::json!({"command": "cargo test"})),
        PermissionDecision::Ask
    );
    assert_eq!(
        auto_checker.check("bash", &serde_json::json!({"command": "cargo test"})),
        PermissionDecision::Deny
    );
    assert_eq!(
        auto_checker.check(
            "write",
            &serde_json::json!({"file_path": "README.md", "content": "updated"})
        ),
        PermissionDecision::Allow
    );
}

#[test]
fn auto_mode_never_returns_ask() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, _) = checker(workspace.path(), Mode::Auto, true);

    for (tool, args, expected) in [
        (
            "write",
            serde_json::json!({"file_path": "README.md", "content": "updated"}),
            PermissionDecision::Allow,
        ),
        (
            "bash",
            serde_json::json!({"command": "cargo test"}),
            PermissionDecision::Allow,
        ),
        (
            "task",
            serde_json::json!({"prompt": "implement the change"}),
            PermissionDecision::Allow,
        ),
        (
            "mcp__github__get_issue",
            serde_json::json!({"issue_number": 1}),
            PermissionDecision::Allow,
        ),
        (
            "bash",
            serde_json::json!({
                "command": "cargo test",
                "sandbox_permissions": "require_escalated",
                "justification": "Needs the host."
            }),
            PermissionDecision::Deny,
        ),
        (
            "write",
            serde_json::json!({
                "file_path": ".a3s/permissions.acl",
                "content": "changed"
            }),
            PermissionDecision::Deny,
        ),
        (
            "git",
            serde_json::json!({"command": "checkout", "ref": "feature"}),
            PermissionDecision::Deny,
        ),
        ("runtime", serde_json::json!({}), PermissionDecision::Deny),
    ] {
        assert_eq!(checker.check(tool, &args), expected, "{tool}");
        assert_ne!(
            checker.check(tool, &args),
            PermissionDecision::Ask,
            "{tool}"
        );
    }
    assert_eq!(
        checker.check("git", &serde_json::json!({"command": "status"})),
        PermissionDecision::Allow
    );
}

#[test]
fn plan_mode_is_read_only() {
    let workspace = tempfile::tempdir().unwrap();
    let (checker, _) = checker(workspace.path(), Mode::Plan, true);

    assert_eq!(
        checker.check("read", &serde_json::json!({"file_path": "README.md"})),
        PermissionDecision::Allow
    );
    assert_eq!(
        checker.check(
            "write",
            &serde_json::json!({"file_path": "README.md", "content": "new"})
        ),
        PermissionDecision::Deny
    );
    assert_eq!(
        checker.check("bash", &serde_json::json!({"command": "pwd"})),
        PermissionDecision::Deny
    );
}

#[tokio::test]
async fn admitted_run_snapshots_do_not_follow_the_next_mode() {
    use a3s_code_core::hitl::ConfirmationProvider;

    let workspace = tempfile::tempdir().unwrap();
    let execution = TuiExecutionPolicy::for_workspace(
        Mode::Auto,
        workspace.path().to_path_buf(),
        Some(Arc::new(TestSandbox)),
    );
    let checker = TuiHitlPermissionChecker::with_execution_policy(
        tui_permission_policy(),
        DeepResearchReportToolGate::default(),
        execution.clone(),
    );
    let provider = TuiModeConfirmationProvider::new(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        execution.clone(),
    );
    let run_checker = checker.snapshot_for_run().unwrap();
    let run_provider = provider.snapshot_for_run().unwrap();
    let escalation = serde_json::json!({
        "command": "cargo test",
        "sandbox_permissions": "require_escalated",
        "justification": "Needs the host."
    });

    execution.set_mode(Mode::Default);

    assert_eq!(checker.check("bash", &escalation), PermissionDecision::Ask);
    assert_eq!(
        run_checker.check("bash", &escalation),
        PermissionDecision::Deny
    );
    assert!(
        provider
            .confirmation_available_for("bash", &escalation)
            .await
    );
    assert!(
        !run_provider
            .confirmation_available_for("bash", &escalation)
            .await
    );
}

#[tokio::test]
async fn auto_rejects_tool_owned_escalation_without_pending_hitl() {
    use a3s_code_core::hitl::ConfirmationProvider;

    let workspace = tempfile::tempdir().unwrap();
    let execution =
        TuiExecutionPolicy::for_workspace(Mode::Auto, workspace.path().to_path_buf(), None);
    let provider = TuiModeConfirmationProvider::new(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        execution,
    );

    assert!(
        !provider
            .confirmation_available_for("mcp__server__destructive", &serde_json::json!({}))
            .await
    );
    let response = provider
        .request_confirmation("tool-1", "mcp__server__destructive", &serde_json::json!({}))
        .await
        .await
        .unwrap();
    assert!(!response.approved);
    assert!(provider.pending_confirmations().await.is_empty());
}

#[tokio::test]
async fn terminal_confirmation_timeouts_are_always_rejections() {
    use a3s_code_core::hitl::{ConfirmationProvider, TimeoutAction};

    for mode in [Mode::Default, Mode::Plan, Mode::Auto] {
        let workspace = tempfile::tempdir().unwrap();
        let execution =
            TuiExecutionPolicy::for_workspace(mode, workspace.path().to_path_buf(), None);
        let provider = TuiModeConfirmationProvider::new(
            a3s_code_core::hitl::ConfirmationPolicy::enabled()
                .with_timeout(321, TimeoutAction::AutoApprove),
            execution,
        );
        let policy = provider.policy().await;
        assert_eq!(policy.default_timeout_ms, 321);
        assert_eq!(policy.timeout_action, TimeoutAction::Reject);
    }
}

#[tokio::test]
async fn session_options_share_execution_policy_across_both_layers() {
    let workspace = tempfile::tempdir().unwrap();
    let execution = TuiExecutionPolicy::for_workspace(
        Mode::Default,
        workspace.path().to_path_buf(),
        Some(Arc::new(TestSandbox)),
    );
    let options = tui_session_options_with_gate_and_execution(
        a3s_code_core::hitl::ConfirmationPolicy::enabled(),
        DeepResearchReportToolGate::default(),
        execution.clone(),
    );
    assert!(options.sandbox_handle.is_some());
    let checker = options.permission_checker.unwrap();
    let confirmation = options.confirmation_manager.unwrap();
    let args = serde_json::json!({"command": "cargo test"});

    assert_eq!(checker.check("bash", &args), PermissionDecision::Allow);
    assert!(confirmation.requires_confirmation("bash").await);

    execution.set_mode(Mode::Auto);
    assert_eq!(checker.check("bash", &args), PermissionDecision::Allow);
    assert!(!confirmation.confirmation_available_for("bash", &args).await);
}

struct ToolTurnClient {
    tool_name: String,
    responses: StdMutex<VecDeque<LlmResponse>>,
}

impl ToolTurnClient {
    fn new(tool_name: impl Into<String>, input: serde_json::Value) -> Self {
        let tool_name = tool_name.into();
        Self {
            tool_name: tool_name.clone(),
            responses: StdMutex::new(
                [tool_response(&tool_name, input), text_response("done")].into(),
            ),
        }
    }

    fn next_response(&self) -> LlmResponse {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| text_response("done"))
    }
}

#[async_trait::async_trait]
impl LlmClient for ToolTurnClient {
    async fn complete(
        &self,
        messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        let prompt = messages.last().map(Message::text).unwrap_or_default();
        Ok(text_response(
            &serde_json::json!({
                "intent": "GeneralPurpose",
                "requires_planning": false,
                "goal": {
                    "description": prompt,
                    "success_criteria": ["the Bash call reaches its governed boundary"]
                },
                "execution_plan": {
                    "complexity": "Simple",
                    "steps": [{
                        "id": "step-1",
                        "description": "Run the requested command",
                        "tool": self.tool_name.as_str(),
                        "dependencies": [],
                        "success_criteria": "the command reaches its governed boundary"
                    }],
                    "required_tools": [self.tool_name.as_str()]
                },
                "optimized_input": prompt
            })
            .to_string(),
        ))
    }

    async fn complete_streaming(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let response = self.next_response();
        let (tx, rx) = mpsc::channel(2);
        tokio::spawn(async move {
            let _ = tx.send(StreamEvent::Done(response)).await;
        });
        Ok(rx)
    }
}

fn text_response(text: &str) -> LlmResponse {
    LlmResponse {
        message: Message {
            role: "assistant".to_string(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            reasoning_content: None,
        },
        usage: TokenUsage::default(),
        stop_reason: Some("stop".to_string()),
        token_logprobs: Vec::new(),
        meta: None,
    }
}

fn tool_response(name: &str, input: serde_json::Value) -> LlmResponse {
    LlmResponse {
        message: Message {
            role: "assistant".to_string(),
            content: vec![ContentBlock::ToolUse {
                id: "bash-event-tool".to_string(),
                name: name.to_string(),
                input,
            }],
            reasoning_content: None,
        },
        usage: TokenUsage::default(),
        stop_reason: Some("tool_use".to_string()),
        token_logprobs: Vec::new(),
        meta: None,
    }
}

async fn governed_session_events(
    mode: Mode,
    sandboxed: bool,
    tool_name: &str,
    tool_args: serde_json::Value,
    dynamic_tool: Option<Arc<dyn Tool>>,
) -> Vec<a3s_code_core::AgentEvent> {
    let workspace = tempfile::tempdir().unwrap();
    let config_path = workspace.path().join("config.acl");
    std::fs::write(
        &config_path,
        "default_model = \"openai/test\"\n\
         providers \"openai\" {\n\
           apiKey = \"test\"\n\
           baseUrl = \"http://127.0.0.1:1\"\n\
           models \"test\" { name = \"Test\" }\n\
         }\n",
    )
    .unwrap();
    let agent = a3s_code_core::Agent::new(config_path.to_string_lossy().to_string())
        .await
        .unwrap();
    let sandbox =
        sandboxed.then(|| Arc::new(TestSandbox) as Arc<dyn a3s_code_core::sandbox::BashSandbox>);
    let execution =
        TuiExecutionPolicy::for_workspace(mode, workspace.path().to_path_buf(), sandbox);
    let options = tui_session_options_with_gate_and_execution(
        a3s_code_core::hitl::ConfirmationPolicy::enabled()
            .with_timeout(5_000, a3s_code_core::hitl::TimeoutAction::Reject),
        DeepResearchReportToolGate::default(),
        execution,
    )
    .with_llm_client(Arc::new(ToolTurnClient::new(tool_name, tool_args)))
    .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
    .with_auto_delegation_enabled(false)
    .with_auto_parallel_delegation(false);
    let session = agent
        .session_async(
            workspace.path().to_string_lossy().to_string(),
            Some(options),
        )
        .await
        .unwrap();
    if let Some(tool) = dynamic_tool {
        session.register_dynamic_tool(tool).unwrap();
    }

    tokio::time::timeout(Duration::from_secs(10), async {
        let (mut receiver, worker) = session.stream("Run the test command.", None).await.unwrap();
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            if let a3s_code_core::AgentEvent::ConfirmationRequired { tool_id, .. } = &event {
                session
                    .confirm_tool_use(tool_id, false, Some("test rejection".to_string()))
                    .await
                    .unwrap();
            }
            let terminal = matches!(
                event,
                a3s_code_core::AgentEvent::End { .. } | a3s_code_core::AgentEvent::Error { .. }
            );
            events.push(event);
            if terminal {
                break;
            }
        }
        worker.await.unwrap();
        events
    })
    .await
    .expect("governed tool session did not settle")
}

async fn bash_session_events(mode: Mode, sandboxed: bool) -> Vec<a3s_code_core::AgentEvent> {
    governed_session_events(
        mode,
        sandboxed,
        "bash",
        serde_json::json!({"command": "printf sandbox-event"}),
        None,
    )
    .await
}

fn confirmation_count(events: &[a3s_code_core::AgentEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            matches!(
                event,
                a3s_code_core::AgentEvent::ConfirmationRequired { .. }
            )
        })
        .count()
}

#[tokio::test]
async fn auto_agent_session_emits_no_confirmation_for_sandboxed_bash() {
    let events = bash_session_events(Mode::Auto, true).await;
    assert_eq!(confirmation_count(&events), 0, "{events:?}");
    assert!(events.iter().any(|event| matches!(
        event,
        a3s_code_core::AgentEvent::ToolEnd {
            name,
            exit_code: 0,
            ..
        } if name == "bash"
    )));
}

#[tokio::test]
async fn auto_agent_session_denies_unsandboxed_bash_without_confirmation() {
    let events = bash_session_events(Mode::Auto, false).await;
    assert_eq!(confirmation_count(&events), 0, "{events:?}");
    assert!(events.iter().any(|event| matches!(
        event,
        a3s_code_core::AgentEvent::PermissionDenied { tool_name, .. }
            if tool_name == "bash"
    )));
}

#[tokio::test]
async fn default_agent_session_emits_no_confirmation_for_sandboxed_bash() {
    let events = bash_session_events(Mode::Default, true).await;
    assert_eq!(confirmation_count(&events), 0, "{events:?}");
    assert!(events.iter().any(|event| matches!(
        event,
        a3s_code_core::AgentEvent::ToolEnd {
            name,
            exit_code: 0,
            ..
        } if name == "bash"
    )));
}

#[tokio::test]
async fn default_agent_session_emits_exactly_one_confirmation_for_host_bash() {
    let events = bash_session_events(Mode::Default, false).await;
    assert_eq!(confirmation_count(&events), 1, "{events:?}");
}

struct ExternalTool {
    name: &'static str,
    requires_confirmation: bool,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for ExternalTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "Test-only external capability"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false
        })
    }

    fn requires_confirmation(&self, _args: &serde_json::Value) -> bool {
        self.requires_confirmation
    }

    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> anyhow::Result<ToolOutput> {
        self.calls.fetch_add(1, AtomicOrdering::SeqCst);
        Ok(ToolOutput::success("external tool executed"))
    }
}

fn external_tool(
    name: &'static str,
    requires_confirmation: bool,
) -> (Arc<dyn Tool>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(ExternalTool {
            name,
            requires_confirmation,
            calls: Arc::clone(&calls),
        }),
        calls,
    )
}

#[tokio::test]
async fn auto_agent_session_denies_side_effecting_external_tool_without_confirmation() {
    let name = "mcp__fixture__mutate";
    let (tool, calls) = external_tool(name, true);
    let events =
        governed_session_events(Mode::Auto, true, name, serde_json::json!({}), Some(tool)).await;

    assert_eq!(confirmation_count(&events), 0, "{events:?}");
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
    assert!(events.iter().any(|event| matches!(
        event,
        a3s_code_core::AgentEvent::PermissionDenied { tool_name, .. }
            if tool_name == name
    )));
}

#[tokio::test]
async fn default_agent_session_asks_once_for_side_effecting_external_tool() {
    let name = "mcp__fixture__mutate";
    let (tool, calls) = external_tool(name, true);
    let events =
        governed_session_events(Mode::Default, true, name, serde_json::json!({}), Some(tool)).await;

    assert_eq!(confirmation_count(&events), 1, "{events:?}");
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
}

#[tokio::test]
async fn auto_agent_session_runs_closed_world_read_only_external_tool() {
    let name = "mcp__fixture__inspect";
    let (tool, calls) = external_tool(name, false);
    let events =
        governed_session_events(Mode::Auto, true, name, serde_json::json!({}), Some(tool)).await;

    assert_eq!(confirmation_count(&events), 0, "{events:?}");
    assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
    assert!(events.iter().any(|event| matches!(
        event,
        a3s_code_core::AgentEvent::ToolEnd {
            name: tool_name,
            exit_code: 0,
            ..
        } if tool_name == name
    )));
}
