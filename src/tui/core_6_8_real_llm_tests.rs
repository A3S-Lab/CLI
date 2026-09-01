use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use a3s_code_core::hitl::{ConfirmationPolicy, TimeoutAction};
use a3s_code_core::run::RunStatus;
use a3s_code_core::store::{
    ContextUsage, FileSessionStore, MemorySessionStore, SessionConfig, SessionData, SessionState,
    SessionStore,
};
use a3s_code_core::{
    Agent, AgentEvent, AgentRunSpawn, AgentSession, CodeConfig, CodeError, LlmClient, Message,
    PlanningMode, SessionOptions, TokenUsage,
};
use serde_json::json;

const OPERATION_TIMEOUT: Duration = Duration::from_secs(300);
const COMPACTION_THRESHOLD: f32 = 0.01;

fn real_model_config() -> PathBuf {
    if let Some(path) = std::env::var_os("A3S_REAL_LLM_CONFIG") {
        return PathBuf::from(path);
    }

    crate::config::default_config_path().expect("resolve ~/.a3s/config.acl")
}

fn real_model_name() -> String {
    std::env::var("A3S_REAL_LLM_MODEL").unwrap_or_else(|_| "codex/gpt-5.6-terra".to_string())
}

fn resolve_real_model(config: &CodeConfig, model: &str, session_id: &str) -> Arc<dyn LlmClient> {
    let options = SessionOptions::new().with_model(model.to_string());
    crate::session_llm::resolve_session_llm_client(config, &options, session_id)
        .unwrap_or_else(|error| panic!("resolve ./a3s model {model}: {error}"))
}

fn governed_options(client: Arc<dyn LlmClient>) -> SessionOptions {
    SessionOptions::new()
        .with_llm_client(client)
        .with_planning_mode(PlanningMode::Disabled)
        .with_llm_api_timeout(180_000)
        .with_continuation(false)
        .with_max_tool_rounds(1)
        .with_confirmation_policy(
            ConfirmationPolicy::enabled().with_timeout(500, TimeoutAction::Reject),
        )
}

async fn turn(session: &AgentSession, prompt: &str) -> (String, usize, Vec<(usize, usize)>) {
    let operation = async {
        let (mut receiver, worker) = session.stream(prompt, None).await.expect("start real turn");
        let mut text = String::new();
        let mut prompt_tokens = 0usize;
        let mut compactions = Vec::new();
        while let Some(event) = receiver.recv().await {
            match event {
                AgentEvent::TextDelta { text: delta } => text.push_str(&delta),
                AgentEvent::TurnEnd { usage, .. } if usage.prompt_tokens > 0 => {
                    prompt_tokens = usage.prompt_tokens;
                }
                AgentEvent::ContextCompacted {
                    before_messages,
                    after_messages,
                    ..
                } => compactions.push((before_messages, after_messages)),
                AgentEvent::End {
                    text: final_text, ..
                } => {
                    if text.trim().is_empty() {
                        text = final_text;
                    }
                    break;
                }
                AgentEvent::Error { message } => panic!("real-model turn failed: {message}"),
                _ => {}
            }
        }
        drop(receiver);
        worker.await.expect("join real-model turn");
        (text, prompt_tokens, compactions)
    };

    tokio::time::timeout(OPERATION_TIMEOUT, operation)
        .await
        .expect("real-model turn timed out")
}

async fn verify_task_and_detached_run(
    agent: &Agent,
    workspace: &std::path::Path,
    client: Arc<dyn LlmClient>,
) {
    let session = agent
        .session_async(
            workspace.to_string_lossy().to_string(),
            Some(
                governed_options(client)
                    .with_manual_delegation_enabled(true)
                    .with_auto_delegation_enabled(false)
                    .with_max_parallel_tasks(2),
            ),
        )
        .await
        .expect("create real-model Core 6.8 session");

    let task = tokio::time::timeout(
        OPERATION_TIMEOUT,
        session.tool(
            "task",
            json!({
                "tasks": [
                    {
                        "agent": "explore",
                        "description": "real alpha branch",
                        "prompt": "Do not call tools. Reply with only CORE_6_8_TASK_ALPHA."
                    },
                    {
                        "agent": "explore",
                        "description": "real beta branch",
                        "prompt": "Do not call tools. Reply with only CORE_6_8_TASK_BETA."
                    }
                ]
            }),
        ),
    )
    .await
    .expect("real unified task timed out")
    .expect("execute real unified task");
    assert_eq!(task.exit_code, 0, "{}", task.output);
    assert!(
        task.output.contains("CORE_6_8_TASK_ALPHA"),
        "{}",
        task.output
    );
    assert!(
        task.output.contains("CORE_6_8_TASK_BETA"),
        "{}",
        task.output
    );
    let task_metadata = task.metadata.as_ref().expect("real task metadata");
    assert_eq!(task_metadata["task_count"], 2);
    assert_eq!(task_metadata["success_count"], 2);

    let run_id = format!("core-6-8-real-detached-{}", std::process::id());
    let prompt = "Do not call tools. Reply with only CORE_6_8_DETACHED_OK.";
    let first = tokio::time::timeout(
        OPERATION_TIMEOUT,
        session.spawn_run_with_id(&run_id, prompt),
    )
    .await
    .expect("real detached-run admission timed out")
    .expect("admit real detached run");
    assert!(!first.replayed());
    let AgentRunSpawn::Started { worker, .. } = first else {
        panic!("the first real detached run must start a worker");
    };
    tokio::time::timeout(OPERATION_TIMEOUT, worker)
        .await
        .expect("real detached-run worker timed out")
        .expect("join real detached-run worker");

    let snapshot = session
        .run_snapshot(&run_id)
        .await
        .expect("real detached-run snapshot");
    assert_eq!(snapshot.status, RunStatus::Completed, "{snapshot:?}");
    assert!(
        snapshot
            .result_text
            .as_deref()
            .is_some_and(|value| value.contains("CORE_6_8_DETACHED_OK")),
        "{snapshot:?}"
    );

    let replay = session
        .spawn_run_with_id(&run_id, prompt)
        .await
        .expect("replay completed real detached run");
    assert!(replay.replayed());

    let conflict = match session
        .spawn_run_with_id(&run_id, "Reply with different immutable input.")
        .await
    {
        Ok(_) => panic!("different input must not reuse a real run identity"),
        Err(error) => error,
    };
    assert!(matches!(
        conflict,
        CodeError::RunIdentityConflict { run_id: ref conflicting } if conflicting == &run_id
    ));

    session.close().await;
}

async fn verify_fork(agent: &Agent, workspace: &std::path::Path, client: Arc<dyn LlmClient>) {
    let store: Arc<dyn SessionStore> = Arc::new(
        FileSessionStore::new(workspace.join("fork-store"))
            .await
            .expect("create fork store"),
    );
    let workspace_path = workspace.to_string_lossy().to_string();
    let prefix = format!("core-6-8-fork-{}", std::process::id());
    let original_id = format!("{prefix}-a");
    let fork_id = format!("{prefix}-b");
    let options = |session_id: &str| {
        governed_options(Arc::clone(&client))
            .with_session_store(Arc::clone(&store))
            .with_session_id(session_id)
            .with_auto_save(true)
    };

    let original = agent
        .session_async(workspace_path.clone(), Some(options(original_id.as_str())))
        .await
        .expect("create original real-model session");
    let (planted, _, _) = turn(
        &original,
        "Remember this secret code exactly: BANANA-42. Reply with only OK.",
    )
    .await;
    assert!(!planted.trim().is_empty());
    original.save().await.expect("save original before fork");
    drop(original);

    let mut fork_data = store
        .load(&original_id)
        .await
        .expect("load original session")
        .expect("original session must be persisted");
    fork_data.id.clone_from(&fork_id);
    store.save(&fork_data).await.expect("save forked session");

    let fork = agent
        .resume_session_async(&fork_id, options(fork_id.as_str()))
        .await
        .expect("resume forked session");
    let (recalled, _, _) = turn(
        &fork,
        "What secret code did I tell you? Reply with only the code.",
    )
    .await;
    assert!(
        recalled.to_uppercase().contains("BANANA-42") || recalled.contains("42"),
        "fork did not retain context: {recalled:?}"
    );
    let _ = turn(
        &fork,
        "Replace the secret code with CHERRY-99. Reply with only OK.",
    )
    .await;
    let (fork_value, _, _) = turn(
        &fork,
        "What is the secret code now? Reply with only the code.",
    )
    .await;
    assert!(
        fork_value.to_uppercase().contains("CHERRY-99") || fork_value.contains("99"),
        "fork did not diverge: {fork_value:?}"
    );

    let original_again = agent
        .resume_session_async(&original_id, options(original_id.as_str()))
        .await
        .expect("resume original session");
    let (original_value, _, _) = turn(
        &original_again,
        "What is the secret code? Reply with only the code.",
    )
    .await;
    assert!(
        original_value.to_uppercase().contains("BANANA-42") || original_value.contains("42"),
        "original session lost its context: {original_value:?}"
    );
    assert!(
        !original_value.to_uppercase().contains("CHERRY") && !original_value.contains("99"),
        "original session observed fork-only state: {original_value:?}"
    );

    fork.close().await;
    original_again.close().await;
}

fn seeded_history() -> Vec<Message> {
    (0..40)
        .map(|index| {
            let text = format!(
                "Seeded compaction fixture message {index}. This is inert historical context. {}",
                format!(
                    "Ledger row {index} reconciles to invoice batch {index} and archived note {index}. "
                )
                .repeat(6)
            );
            if index % 2 == 0 {
                Message::user(&text)
            } else {
                Message::assistant(&text)
            }
        })
        .collect()
}

fn seeded_session(
    id: &str,
    workspace: &str,
    messages: Vec<Message>,
    auto_compact: bool,
) -> SessionData {
    SessionData {
        id: id.to_string(),
        config: SessionConfig {
            workspace: workspace.to_string(),
            auto_compact,
            auto_compact_threshold: COMPACTION_THRESHOLD,
            max_context_length: 200_000,
            ..Default::default()
        },
        state: SessionState::Active,
        messages,
        context_usage: ContextUsage::default(),
        total_usage: TokenUsage::default(),
        total_cost: 0.0,
        model_name: None,
        cost_records: Vec::new(),
        tool_names: Vec::new(),
        thinking_enabled: false,
        thinking_budget: None,
        created_at: 0,
        updated_at: 0,
        llm_config: None,
        tasks: Vec::new(),
        parent_id: None,
        tenant_id: None,
        principal: None,
        agent_template_id: None,
        correlation_id: None,
        durable_memory_binding: None,
        cognitive_package_binding: None,
        immutable_content_adapter_binding: None,
    }
}

async fn verify_compaction(agent: &Agent, workspace: &std::path::Path, client: Arc<dyn LlmClient>) {
    let store: Arc<dyn SessionStore> = Arc::new(MemorySessionStore::new());
    let workspace_path = workspace.to_string_lossy().to_string();
    let prefix = format!("core-6-8-compact-{}", std::process::id());
    let baseline_id = format!("{prefix}-baseline");
    let compact_id = format!("{prefix}-active");
    store
        .save(&seeded_session(
            &baseline_id,
            &workspace_path,
            seeded_history(),
            false,
        ))
        .await
        .expect("seed baseline session");
    store
        .save(&seeded_session(
            &compact_id,
            &workspace_path,
            seeded_history(),
            true,
        ))
        .await
        .expect("seed compacting session");

    let options = |auto_compact: bool| {
        governed_options(Arc::clone(&client))
            .with_session_store(Arc::clone(&store))
            .with_auto_compact(auto_compact)
            .with_auto_compact_threshold(COMPACTION_THRESHOLD)
            .with_auto_save(auto_compact)
            .with_temperature(0.0)
    };
    let prompt = "Do not use tools. Reply with only OK.";

    let baseline = agent
        .resume_session_async(&baseline_id, options(false))
        .await
        .expect("resume baseline session");
    let (_, baseline_tokens, baseline_compactions) = turn(&baseline, prompt).await;
    assert!(baseline_tokens > 0, "provider did not report prompt usage");
    assert!(baseline_compactions.is_empty());
    baseline.close().await;

    let compacting = agent
        .resume_session_async(&compact_id, options(true))
        .await
        .expect("resume compacting session");
    let (_, compacted_tokens, compactions) = turn(&compacting, prompt).await;
    let (before, after) = compactions
        .iter()
        .copied()
        .find(|(before, after)| after < before)
        .expect("auto-compaction did not shrink the seeded history");
    assert!(compacted_tokens > 0, "compacted turn reported no usage");
    assert!(
        compacted_tokens < baseline_tokens,
        "compacted prompt {compacted_tokens} was not below baseline {baseline_tokens}"
    );
    compacting.close().await;

    let persisted = agent
        .resume_session_async(&compact_id, options(false))
        .await
        .expect("resume persisted compacted session");
    let (_, persisted_tokens, _) = turn(&persisted, prompt).await;
    assert!(
        persisted_tokens < baseline_tokens,
        "persisted compacted prompt {persisted_tokens} was not below baseline {baseline_tokens}"
    );
    persisted.close().await;

    eprintln!(
        "Core 6.8 compaction: {before} -> {after} messages; prompt tokens {baseline_tokens} -> {compacted_tokens} -> {persisted_tokens}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "uses the real account/config model exposed by ./a3s"]
async fn core_6_8_real_model_end_to_end() {
    let config_path = real_model_config();
    assert!(
        config_path.is_file(),
        "real A3S model configuration is missing at {}",
        config_path.display()
    );
    let config = CodeConfig::from_file(&config_path).expect("load real A3S configuration");
    let model = real_model_name();
    let resolver_id = format!("core-6-8-real-resolver-{}", std::process::id());
    let client = resolve_real_model(&config, &model, &resolver_id);
    let agent = Agent::from_config(config)
        .await
        .expect("build agent from real A3S configuration");
    let workspace = tempfile::Builder::new()
        .prefix("a3s-core-6-8-real-")
        .tempdir()
        .expect("create real-model workspace");

    eprintln!("Testing Core 6.8 through ./a3s model {model}");
    verify_task_and_detached_run(&agent, workspace.path(), Arc::clone(&client)).await;
    verify_fork(&agent, workspace.path(), Arc::clone(&client)).await;
    verify_compaction(&agent, workspace.path(), client).await;
}
