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

/// Live suites pin the config file's `default_model` unless the caller
/// overrides `A3S_REAL_LLM_MODEL`. They do not fall back to Codex.
fn review_live_config() -> (CodeConfig, String, PathBuf) {
    let path = std::env::var_os("A3S_CONFIG_FILE")
        .or_else(|| std::env::var_os("A3S_REAL_LLM_CONFIG"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../.a3s/config.acl")
        });
    assert!(
        path.is_file(),
        "review live config is missing at {}",
        path.display()
    );
    let config = CodeConfig::from_file(&path)
        .unwrap_or_else(|error| panic!("load {}: {error}", path.display()));
    let model = std::env::var("A3S_REAL_LLM_MODEL").unwrap_or_else(|_| {
        config
            .default_model
            .clone()
            .unwrap_or_else(|| panic!("default_model missing in {}", path.display()))
    });
    (config, model, path)
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
    try_turn(session, prompt)
        .await
        .unwrap_or_else(|message| panic!("real-model turn failed: {message}"))
}

async fn try_turn(
    session: &AgentSession,
    prompt: &str,
) -> Result<(String, usize, Vec<(usize, usize)>), String> {
    let operation = async {
        let (mut receiver, worker) = session
            .stream(prompt, None)
            .await
            .map_err(|error| error.to_string())?;
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
                AgentEvent::Error { message } => return Err(message),
                _ => {}
            }
        }
        drop(receiver);
        worker
            .await
            .map_err(|error| format!("join real-model turn: {error}"))?;
        Ok((text, prompt_tokens, compactions))
    };

    tokio::time::timeout(OPERATION_TIMEOUT, operation)
        .await
        .map_err(|_| "real-model turn timed out".to_string())?
}

fn is_transient_provider_block(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("network protection")
        || lower.contains("http 403")
        || lower.contains("vpn/proxy")
        || lower.contains("blocked by chatgpt")
        || lower.contains("connection refused")
        || lower.contains("timed out")
        || lower.contains("timeout")
}

#[cfg(test)]
mod soft_skip_tests {
    use super::is_transient_provider_block;

    #[test]
    fn transient_provider_block_detects_codex_network_protection() {
        assert!(is_transient_provider_block(
            "Codex WebSocket and HTTPS fallback were blocked by ChatGPT network protection (HTTP 403). Check the VPN/proxy route or use the official Codex transport."
        ));
        assert!(is_transient_provider_block("request timed out"));
        assert!(!is_transient_provider_block(
            "live model did not emit a parseable a3s-review fence"
        ));
    }
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
        durable_memory_binding: None,
        correlation_id: None,
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
    let (config, model, config_path) = review_live_config();
    let resolver_id = format!("core-6-8-real-resolver-{}", std::process::id());
    let client = resolve_real_model(&config, &model, &resolver_id);
    let agent = Agent::from_config(config)
        .await
        .expect("build agent from real A3S configuration");
    let workspace = tempfile::Builder::new()
        .prefix("a3s-core-6-8-real-")
        .tempdir()
        .expect("create real-model workspace");

    eprintln!(
        "Testing Core 6.8 through ./a3s model {model} from {}",
        config_path.display()
    );
    verify_task_and_detached_run(&agent, workspace.path(), Arc::clone(&client)).await;
    verify_fork(&agent, workspace.path(), Arc::clone(&client)).await;
    verify_compaction(&agent, workspace.path(), client).await;
}

/// R-live: real model must emit a parseable claim-vs-record fail (or warn)
/// when the sticky prompt shows a “tests passed” claim against a failed tool.
///
/// Hermetic coverage is `mock_detect_false_tests_passed_claim_pipeline`.
/// Run with:
///   cargo test --bin a3s reviewer_claim_vs_record_detects_false -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "uses the real account/config model; R-live claim-vs-record detect"]
async fn reviewer_claim_vs_record_detects_false_tests_passed_claim() {
    use crate::tui::panels::review::{
        open_reply_findings_from_issues, parse_review_report, ReviewReportKind,
    };
    use crate::tui::panels::workspace_review::{
        mock_sticky_reply_review_report, sticky_reply_review_prompt_and_display,
        TurnEvidenceBundle, TurnEvidenceTool,
    };

    let (config, model, config_path) = review_live_config();
    eprintln!(
        "R-live reviewer model {model} from {}",
        config_path.display()
    );
    let resolver_id = format!("reviewer-r-live-resolver-{}", std::process::id());
    let client = resolve_real_model(&config, &model, &resolver_id);
    let agent = Agent::from_config(config)
        .await
        .expect("build agent from real A3S configuration");
    let workspace = tempfile::Builder::new()
        .prefix("a3s-reviewer-r-live-")
        .tempdir()
        .expect("create real-model workspace");

    let bundle = TurnEvidenceBundle {
        user: "Did the tests pass?".into(),
        assistant: "Yes — all tests passed.".into(),
        tools: vec![TurnEvidenceTool {
            index: 1,
            name: "bash".into(),
            state: "failed".into(),
            args: "{\"command\":\"cargo test\"}".into(),
            output: "test result: FAILED. 2 failed".into(),
            exit_code: Some(1),
            truncated: false,
        }],
        complete: true,
    };
    // Sanity: hermetic mock still detects before spending a live turn.
    let mock = mock_sticky_reply_review_report(workspace.path(), &bundle);
    let (_, mock_kind, mock_issues) = parse_review_report(&mock).expect("mock fence");
    assert_eq!(mock_kind, ReviewReportKind::Reply);
    assert_eq!(mock_issues[0].verdict, "fail");

    let (prompt, _) = sticky_reply_review_prompt_and_display(workspace.path(), &bundle);
    let session = agent
        .session_async(
            workspace.path().to_string_lossy().to_string(),
            Some(
                governed_options(client)
                    .with_max_tool_rounds(1)
                    .with_continuation(false),
            ),
        )
        .await
        .expect("create reviewer R-live session");

    eprintln!("R-live reviewer claim-vs-record through config model {model}");
    let (text, prompt_tokens, _) = match try_turn(&session, &prompt).await {
        Ok(outcome) => outcome,
        Err(message) => {
            session.close().await;
            panic!("R-live must fail closed, not soft-skip: {message}");
        }
    };
    session.close().await;
    assert!(prompt_tokens > 0, "provider did not report prompt usage");
    eprintln!("R-live reviewer reply ({} chars):\n{text}", text.len());

    let (_, kind, issues) = parse_review_report(&text)
        .unwrap_or_else(|| panic!("live model did not emit a parseable a3s-review fence:\n{text}"));
    assert_eq!(kind, ReviewReportKind::Reply);
    assert!(
        !issues.is_empty(),
        "expected at least one finding for false pass claim:\n{text}"
    );
    assert!(
        issues.iter().any(|issue| {
            matches!(issue.verdict.as_str(), "fail" | "warn")
                && issue
                    .evidence_refs
                    .iter()
                    .any(|reference| reference.contains("tool:"))
        }),
        "expected fail/warn with tool evidence_refs:\n{text}"
    );
    let open = open_reply_findings_from_issues(&issues);
    assert!(!open.is_empty(), "open findings should remain injectable");
}

/// Git `/review` must inspect a planted working-tree defect with the product
/// CodeReview side-session, emit a parseable `kind: code` report that names
/// that defect, and leave the tree unchanged. A soft-skip is not a pass.
///
///   A3S_CONFIG_FILE=/abs/path/.a3s/config.acl \
///     cargo test --bin a3s reviewer_git_review_names_planted_length_compare -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "uses the config default model; git /review planted-defect detect"]
async fn reviewer_git_review_names_planted_length_compare() {
    use a3s_code_core::PlanningMode;

    use crate::tui::panels::review::{parse_review_report, ReviewReportKind};
    use crate::tui::panels::workspace_review::{workspace_review_prompt, WorkspaceReviewTarget};

    let (config, model, config_path) = review_live_config();
    eprintln!(
        "git /review planted defect through model {model} from {}",
        config_path.display()
    );
    let resolver_id = format!("reviewer-git-live-{}", std::process::id());
    let agent = Agent::from_config(config.clone())
        .await
        .expect("build agent from review config");
    let workspace = tempfile::Builder::new()
        .prefix("a3s-reviewer-git-live-")
        .tempdir()
        .expect("create review workspace");
    plant_length_only_auth_bug(workspace.path());
    let planted =
        std::fs::read_to_string(workspace.path().join("src/auth.rs")).expect("read plant");
    let before = source_porcelain(workspace.path());
    assert!(
        before.contains("src/auth.rs"),
        "planted change missing from git status:\n{before}"
    );

    let prompt = workspace_review_prompt(workspace.path(), &WorkspaceReviewTarget::WorkingTree);
    let execution = super::TuiExecutionPolicy::for_workspace(
        super::Mode::Reviewer,
        workspace.path().to_path_buf(),
    );
    let opts = super::apply_launch_model_options(
        super::tui_session_options_with_gate_grants_and_execution(
            a3s_code_core::hitl::ConfirmationPolicy::enabled()
                .with_timeout(500, a3s_code_core::hitl::TimeoutAction::Reject),
            super::DeepResearchReportToolGate::default(),
            super::TuiPermissionGrants::default(),
            execution,
        )
        .with_prompt_slots(super::git_review_side_session_prompt_slots())
        .with_auto_compact(false)
        .with_session_id(&resolver_id)
        .with_planning(false)
        .with_planning_mode(PlanningMode::Disabled)
        .with_auto_delegation_enabled(false)
        .with_max_tool_rounds(8)
        .with_llm_api_timeout(120_000),
        Some(&model),
        None,
        "medium",
        &config,
        &resolver_id,
    );
    let session = agent
        .session_async(workspace.path().to_string_lossy().to_string(), Some(opts))
        .await
        .expect("create git review side-session");

    let (text, tools) = match review_turn(&session, &prompt).await {
        Ok(outcome) => outcome,
        Err(message) => {
            session.close().await;
            panic!("git /review must fail closed, not soft-skip: {message}");
        }
    };
    session.close().await;
    eprintln!(
        "git /review tools: {tools:?}\nreply ({} chars):\n{text}",
        text.len()
    );

    let after = source_porcelain(workspace.path());
    assert_eq!(
        before, after,
        "review mutated source outside host session dirs\nbefore:\n{before}\nafter:\n{after}"
    );
    let after_auth = std::fs::read_to_string(workspace.path().join("src/auth.rs")).expect("reread");
    assert_eq!(planted, after_auth, "review rewrote the planted source");
    assert!(
        tools.iter().any(|call| {
            (call.name == "git" && call.args.contains("diff"))
                || (call.name == "read" && call.args.contains("auth.rs"))
        }),
        "review did not inspect the planted diff or auth.rs: {tools:?}"
    );
    assert!(
        tools
            .iter()
            .all(|call| !matches!(call.name.as_str(), "write" | "edit" | "patch")),
        "review used a mutating tool: {tools:?}"
    );

    let (_, kind, issues) = parse_review_report(&text)
        .unwrap_or_else(|| panic!("live model did not emit a parseable a3s-review fence:\n{text}"));
    assert_eq!(kind, ReviewReportKind::Code);
    assert!(
        issues.iter().any(names_length_only_token_compare),
        "expected a code finding naming the length-only token compare in src/auth.rs:\n{text}"
    );
}

#[derive(Debug)]
struct ReviewToolCall {
    name: String,
    args: String,
}

fn names_length_only_token_compare(issue: &crate::tui::panels::review::ReviewIssue) -> bool {
    let file = issue.file.to_ascii_lowercase();
    let text = format!("{} {}", issue.title, issue.detail).to_ascii_lowercase();
    let anchored = file.contains("auth.rs") || text.contains("tokens_match");
    let length = text.contains("len") || text.contains("length");
    let contents = text.contains("content")
        || text.contains("character")
        || text.contains("value")
        || text.contains("ident")
        || text.contains("compar")
        || text.contains("equal");
    anchored && length && contents
}

fn plant_length_only_auth_bug(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("src")).expect("src dir");
    std::fs::write(
        dir.join("src/ok.rs"),
        "pub fn add(left: i32, right: i32) -> i32 {\n    left + right\n}\n",
    )
    .expect("write ok.rs");
    std::fs::write(
        dir.join("src/auth.rs"),
        "/// Returns true only when `presented` and `expected` contain the same characters.\n\
         pub fn tokens_match(presented: &str, expected: &str) -> bool {\n\
             presented == expected\n\
         }\n",
    )
    .expect("write correct auth.rs");
    git(dir, &["init"]);
    git(dir, &["add", "src/ok.rs", "src/auth.rs"]);
    git(dir, &["commit", "-m", "baseline"]);
    std::fs::write(
        dir.join("src/auth.rs"),
        "/// Returns true only when `presented` and `expected` contain the same characters.\n\
         pub fn tokens_match(presented: &str, expected: &str) -> bool {\n\
             presented.len() == expected.len()\n\
         }\n",
    )
    .expect("plant length-only compare");
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "review-test")
        .env("GIT_AUTHOR_EMAIL", "review-test@example.com")
        .env("GIT_COMMITTER_NAME", "review-test")
        .env("GIT_COMMITTER_EMAIL", "review-test@example.com")
        .status()
        .unwrap_or_else(|error| panic!("spawn git {args:?}: {error}"));
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

fn source_porcelain(dir: &std::path::Path) -> String {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .expect("git status");
    assert!(output.status.success(), "git status failed");
    String::from_utf8(output.stdout)
        .expect("git status utf-8")
        .lines()
        .filter(|line| {
            let path = line
                .trim_start()
                .trim_start_matches(['?', 'M', 'A', 'D', ' ']);
            let path = path.trim_start();
            !path.starts_with(".a3s/") && !path.starts_with(".a3s-code/")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn review_turn(
    session: &a3s_code_core::AgentSession,
    prompt: &str,
) -> Result<(String, Vec<ReviewToolCall>), String> {
    use a3s_code_core::AgentEvent;

    let operation = async {
        let (mut receiver, worker) = session
            .stream(prompt, None)
            .await
            .map_err(|error| error.to_string())?;
        let mut text = String::new();
        let mut tools = Vec::new();
        while let Some(event) = receiver.recv().await {
            match event {
                AgentEvent::TextDelta { text: delta } => text.push_str(&delta),
                AgentEvent::ToolExecutionStart { name, args, .. } => {
                    tools.push(ReviewToolCall {
                        name,
                        args: args.to_string(),
                    });
                }
                AgentEvent::End {
                    text: final_text, ..
                } => {
                    if text.trim().is_empty() {
                        text = final_text;
                    }
                    break;
                }
                AgentEvent::Error { message } => return Err(message),
                _ => {}
            }
        }
        drop(receiver);
        worker
            .await
            .map_err(|error| format!("join git review turn: {error}"))?;
        Ok((text, tools))
    };
    tokio::time::timeout(OPERATION_TIMEOUT, operation)
        .await
        .map_err(|_| "git review turn timed out".to_string())?
}
