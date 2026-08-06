use std::path::Path;
use std::sync::Arc;

use a3s_code_core::llm::{
    ContentBlock, LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
};
use a3s_code_core::{Agent, AgentRunSpawn, AgentSession, CodeError, PlanningMode, SessionOptions};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct StaticLlmClient;

#[async_trait]
impl LlmClient for StaticLlmClient {
    async fn complete(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        Ok(done_response())
    }

    async fn complete_streaming(
        &self,
        _messages: &[Message],
        _system: Option<&str>,
        _tools: &[ToolDefinition],
        _cancel_token: CancellationToken,
    ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = tx.send(StreamEvent::Done(done_response())).await;
        });
        Ok(rx)
    }
}

fn done_response() -> LlmResponse {
    LlmResponse {
        message: Message {
            role: "assistant".to_string(),
            content: vec![ContentBlock::Text {
                text: "deterministic child result".to_string(),
            }],
            reasoning_content: None,
        },
        usage: TokenUsage::default(),
        stop_reason: Some("stop".to_string()),
        token_logprobs: Vec::new(),
        meta: None,
    }
}

fn write_config(workspace: &Path) -> std::path::PathBuf {
    let config = workspace.join("config.acl");
    std::fs::write(
        &config,
        "default_model = \"openai/test\"\n\
         providers \"openai\" {\n\
           apiKey = \"test\"\n\
           baseUrl = \"http://127.0.0.1:1\"\n\
           models \"test\" { name = \"test\" }\n\
         }\n",
    )
    .expect("write deterministic Code config");
    config
}

async fn session(workspace: &Path, options: SessionOptions) -> AgentSession {
    let config = write_config(workspace);
    let agent = Agent::new(config.to_string_lossy().to_string())
        .await
        .expect("load Code 6.8 config");
    agent
        .session_async(
            workspace.to_string_lossy().to_string(),
            Some(options.with_llm_client(Arc::new(StaticLlmClient))),
        )
        .await
        .expect("create TUI-compatible Code session")
}

#[tokio::test]
async fn tui_session_exposes_and_executes_all_unified_search_modes() {
    let workspace = tempfile::tempdir().expect("create search workspace");
    std::fs::create_dir_all(workspace.path().join("src")).expect("create source directory");
    std::fs::write(
        workspace.path().join("src/session.rs"),
        "pub fn invalidate_cache() {\n    // deterministic session cache invalidation policy\n}\n",
    )
    .expect("write ranked source fixture");
    std::fs::write(
        workspace.path().join("README.md"),
        "unrelated documentation\n",
    )
    .expect("write documentation fixture");

    let session = session(
        workspace.path(),
        SessionOptions::new().with_planning_mode(PlanningMode::Disabled),
    )
    .await;
    let definitions = session.tool_definitions();
    let search = definitions
        .iter()
        .find(|tool| tool.name == "search")
        .expect("unified search definition");
    assert_eq!(
        search.parameters["properties"]["mode"]["enum"],
        json!(["grep", "glob", "bm25"])
    );
    for obsolete in ["grep", "glob", "bm25"] {
        assert!(
            !definitions.iter().any(|tool| tool.name == obsolete),
            "{obsolete} must not consume a separate model-visible schema"
        );
    }

    let cases = [
        (
            "grep",
            json!({
                "mode": "grep",
                "query": "SESSION CACHE",
                "include": "*.rs",
                "case_sensitive": false
            }),
        ),
        (
            "glob",
            json!({"mode": "glob", "query": "**/*.rs", "sort": "path"}),
        ),
        (
            "bm25",
            json!({
                "mode": "bm25",
                "query": "session cache invalidation",
                "include": "*.rs",
                "limit": 5
            }),
        ),
    ];

    for (mode, arguments) in cases {
        let result = session
            .tool("search", arguments)
            .await
            .unwrap_or_else(|error| panic!("execute {mode} through search: {error}"));
        assert_eq!(result.exit_code, 0, "{mode}: {}", result.output);
        assert!(
            result.output.contains("src/session.rs"),
            "{mode}: {}",
            result.output
        );
        assert_eq!(
            result.metadata.as_ref().expect("search metadata")["mode"],
            mode
        );
    }

    session.close().await;
}

#[tokio::test]
async fn tui_session_runs_unified_task_shapes_and_keeps_the_legacy_alias_hidden() {
    let workspace = tempfile::tempdir().expect("create task workspace");
    let session = session(
        workspace.path(),
        SessionOptions::new()
            .with_manual_delegation_enabled(true)
            .with_auto_delegation_enabled(false)
            .with_max_parallel_tasks(4)
            .with_planning_mode(PlanningMode::Disabled),
    )
    .await;
    let definitions = session.tool_definitions();
    let task = definitions
        .iter()
        .find(|tool| tool.name == "task")
        .expect("unified task definition");
    assert_eq!(task.parameters["required"], json!(["tasks"]));
    assert_eq!(task.parameters["properties"]["tasks"]["minItems"], 1);
    assert_eq!(task.parameters["properties"]["tasks"]["maxItems"], 32);
    assert!(!definitions.iter().any(|tool| tool.name == "parallel_task"));

    let child = |description: &str| {
        json!({
            "agent": "explore",
            "description": description,
            "prompt": "Return the deterministic result without tools."
        })
    };
    let single = session
        .tool("task", json!({"tasks": [child("single child")]}))
        .await
        .expect("execute one unified task");
    assert_eq!(single.exit_code, 0, "{}", single.output);
    assert!(single.output.contains("deterministic child result"));
    assert!(single
        .metadata
        .as_ref()
        .is_some_and(|metadata| metadata["task_id"].is_string()));

    let fan_out = session
        .tool(
            "task",
            json!({"tasks": [child("first branch"), child("second branch")]}),
        )
        .await
        .expect("execute unified task fan-out");
    assert_eq!(fan_out.exit_code, 0, "{}", fan_out.output);
    let metadata = fan_out.metadata.as_ref().expect("fan-out metadata");
    assert_eq!(metadata["task_count"], 2);
    assert_eq!(metadata["success_count"], 2);

    let legacy = session
        .tool(
            "parallel_task",
            json!({"tasks": [child("legacy first"), child("legacy second")]}),
        )
        .await
        .expect("execute hidden compatibility alias");
    assert_eq!(legacy.exit_code, 0, "{}", legacy.output);
    assert_eq!(
        legacy.metadata.as_ref().expect("legacy alias metadata")["task_count"],
        2
    );

    session.close().await;
}

#[tokio::test]
async fn tui_linked_core_supports_exact_detached_run_replay_and_conflict_detection() {
    let workspace = tempfile::tempdir().expect("create detached-run workspace");
    let session = session(
        workspace.path(),
        SessionOptions::new().with_planning_mode(PlanningMode::Disabled),
    )
    .await;

    let first = session
        .spawn_run_with_id("tui-run-001", "Return the deterministic result.")
        .await
        .expect("admit exact detached run");
    assert!(!first.replayed());
    let AgentRunSpawn::Started { worker, .. } = first else {
        panic!("the first exact run must start a worker");
    };
    worker.await.expect("join exact detached run worker");

    let replay = session
        .spawn_run_with_id("tui-run-001", "Return the deterministic result.")
        .await
        .expect("replay exact detached run");
    assert!(replay.replayed());
    assert_eq!(replay.snapshot().id, "tui-run-001");

    let conflict = match session
        .spawn_run_with_id("tui-run-001", "Use different immutable input.")
        .await
    {
        Ok(_) => panic!("different input must not reuse an exact run identity"),
        Err(error) => error,
    };
    assert!(matches!(
        conflict,
        CodeError::RunIdentityConflict { ref run_id } if run_id == "tui-run-001"
    ));

    session.close().await;
}
