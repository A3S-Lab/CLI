use super::*;
use crate::top::Risk;

#[path = "system_agents_tests/control.rs"]
mod control;

fn process(pid: u32, ppid: u32, agent: Option<AgentKind>) -> ProcessRow {
    ProcessRow {
        pid,
        ppid,
        cpu_pct: 0.0,
        mem_pct: 0.0,
        elapsed: "00:01".to_string(),
        cwd: Some(format!("/workspace/{pid}")),
        command: "agent --secret prompt".to_string(),
        agent,
        risk: Risk::Low,
    }
}

fn presence(instance: &str, pid: u32, state: AgentActivityState) -> AgentPresence {
    AgentPresence::new(
        instance,
        pid,
        format!("/workspace/{pid}"),
        Some("implement the system island".to_string()),
        state,
        Vec::new(),
        epoch_ms(),
    )
}

#[test]
fn exact_presence_replaces_same_process_and_keeps_inferred_agents() {
    let now = epoch_ms();
    let mut exact = presence("local", 10, AgentActivityState::Working);
    exact.updated_at_ms = now.saturating_sub(9_000);
    let rows = aggregate_activities(
        &[exact],
        &[
            process(10, 1, Some(AgentKind::A3sCode)),
            process(20, 1, Some(AgentKind::Codex)),
        ],
        "local",
        now,
    );

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].confidence, AgentActivityConfidence::Exact);
    assert!(rows[0].local);
    assert_eq!(rows[0].expires_at_ms, now.saturating_add(1_000));
    assert_eq!(rows[1].confidence, AgentActivityConfidence::Process);
    assert_eq!(rows[1].expires_at_ms, now.saturating_add(PRESENCE_TTL_MS));
    assert_eq!(rows[1].started_at_ms, Some(now.saturating_sub(1_000)));
    assert_eq!(rows[1].task.as_deref(), Some("active process"));
    assert!(!rows[1]
        .task
        .as_deref()
        .unwrap_or_default()
        .contains("secret"));
}

#[test]
fn nested_same_agent_process_is_collapsed_to_its_root() {
    let processes = [
        process(20, 1, Some(AgentKind::Codex)),
        process(21, 20, Some(AgentKind::Codex)),
        process(22, 21, None),
    ];

    let roots = root_agent_processes(&processes);

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].pid, 20);
}

#[test]
fn stale_presence_cannot_resurrect_a_finished_process() {
    let now = epoch_ms();
    let mut stale = presence("old", 44, AgentActivityState::Working);
    stale.updated_at_ms = now.saturating_sub(PRESENCE_TTL_MS + 1);

    assert!(aggregate_activities(&[stale], &[], "new", now).is_empty());
}

#[test]
fn island_launch_accepts_exact_lifecycle_or_a_detected_external_agent() {
    let now = epoch_ms();
    let process_rows = [process(20, 1, Some(AgentKind::Codex))];
    let idle = SystemAgentSnapshot {
        activities: aggregate_activities(
            &[presence("local", 1, AgentActivityState::Idle)],
            &[],
            "local",
            now,
        ),
        warnings: Vec::new(),
    };
    assert!(!snapshot_requests_island_launch(&idle));

    for state in [
        AgentActivityState::Planning,
        AgentActivityState::Working,
        AgentActivityState::WaitingApproval,
        AgentActivityState::Completed,
        AgentActivityState::Failed,
        AgentActivityState::Cancelled,
    ] {
        let snapshot = SystemAgentSnapshot {
            activities: aggregate_activities(
                &[presence("local", 1, state)],
                &process_rows,
                "local",
                now,
            ),
            warnings: Vec::new(),
        };
        assert!(
            snapshot_requests_island_launch(&snapshot),
            "state {state:?} should request the island"
        );
    }

    let process_only = SystemAgentSnapshot {
        activities: aggregate_activities(&[], &process_rows, "local", now),
        warnings: Vec::new(),
    };
    assert!(snapshot_requests_island_launch(&process_only));
}

#[tokio::test]
async fn publisher_round_trips_private_heartbeat_and_removes_it() {
    let temp = tempfile::tempdir().unwrap();
    let publisher = AgentPresencePublisher::for_directory(temp.path().to_path_buf());
    let local = presence(
        publisher.instance_id(),
        std::process::id(),
        AgentActivityState::Idle,
    );

    publisher.write_presence(&local).await.unwrap();
    let stored = publisher.read_presences(epoch_ms()).await.unwrap();

    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].instance_id, publisher.instance_id());
    assert_eq!(stored[0].workspace, std::process::id().to_string());
    assert!(stored[0].task.is_none());

    publisher.remove().await;
    assert!(!publisher.path.as_ref().unwrap().exists());
}

#[tokio::test]
async fn heartbeat_redacts_tasks_by_default_and_shares_only_with_opt_in() {
    let temp = tempfile::tempdir().unwrap();
    let private = AgentPresencePublisher::for_directory(temp.path().to_path_buf());
    let private_task = "parent-private\nfull prompt\u{1b}[2J".to_string();
    let mut local = AgentPresence::new(
        private.instance_id(),
        std::process::id(),
        "/Users/example/secret-workspace/repository",
        Some(private_task.clone()),
        AgentActivityState::Working,
        vec![AgentChildPresence {
            id: "child-1".to_string(),
            agent: "codex".to_string(),
            task: Some("child-private description".to_string()),
            state: AgentActivityState::Working,
            vendor: AgentVendor::OpenAi,
            started_at_ms: None,
            finished_at_ms: None,
            actions: Vec::new(),
        }],
        epoch_ms(),
    );

    private.write_presence(&local).await.unwrap();
    let serialized = tokio::fs::read_to_string(private.path.as_ref().unwrap())
        .await
        .unwrap();
    let stored = private.read_presences(epoch_ms()).await.unwrap();

    assert_eq!(local.task.as_deref(), Some(private_task.as_str()));
    assert_eq!(stored[0].workspace, "repository");
    assert!(stored[0].task.is_none());
    assert!(stored[0].children[0].task.is_none());
    assert!(!serialized.contains("parent-private"));
    assert!(!serialized.contains("child-private"));
    assert!(!serialized.contains("secret-workspace"));
    private.remove().await;

    let shared = AgentPresencePublisher::for_directory_with_task_sharing(temp.path().to_path_buf());
    local.instance_id = shared.instance_id().to_string();
    local.pid = std::process::id();
    shared.write_presence(&local).await.unwrap();
    let stored = shared.read_presences(epoch_ms()).await.unwrap();

    assert_eq!(
        stored[0].task.as_deref(),
        Some("parent-private full prompt")
    );
    assert_eq!(
        stored[0].children[0].task.as_deref(),
        Some("child-private description")
    );
    shared.remove().await;
}

#[test]
fn agent_island_preference_defaults_on_and_round_trips() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("agent-presence");

    assert!(preference::is_enabled(Some(&directory)));
    preference::set_enabled(Some(&directory), false).unwrap();
    assert!(!preference::is_enabled(Some(&directory)));

    let disabled_publisher = AgentPresencePublisher::for_directory(directory.clone());
    assert!(!disabled_publisher.island_preference_enabled());

    preference::set_enabled(Some(&directory), true).unwrap();
    assert!(preference::is_enabled(Some(&directory)));
    let enabled_publisher = AgentPresencePublisher::for_directory(directory);
    assert!(enabled_publisher.island_preference_enabled());
}

#[tokio::test]
async fn island_opt_out_does_not_disable_the_shared_presence_protocol() {
    let temp = tempfile::tempdir().unwrap();
    preference::set_enabled(Some(temp.path()), false).unwrap();
    let publisher = AgentPresencePublisher::for_directory(temp.path().to_path_buf());
    let local = presence(
        publisher.instance_id(),
        std::process::id(),
        AgentActivityState::Working,
    );

    publisher.write_presence(&local).await.unwrap();
    assert!(publisher.path.as_ref().unwrap().is_file());
    assert!(!publisher.island_preference_enabled());
}

#[test]
fn terminal_text_sanitizer_removes_controls_and_bounds_unicode() {
    let hostile = format!(
        "first\n\u{1b}[2Jhidden\u{1b}]52;c;Y2xpcGJvYXJk\u{7} \u{9b}31mred \u{202e}实现 e\u{301} {}",
        "x".repeat(MAX_TASK_CHARS + 20)
    );

    let value = sanitize_display_text(&hostile, MAX_TASK_CHARS);

    assert!(!value.chars().any(char::is_control), "{value:?}");
    assert!(!value.contains("[2J"), "{value:?}");
    assert!(!value.contains("]52"), "{value:?}");
    assert!(!value.contains('\u{202e}'), "{value:?}");
    assert!(value.contains("实现"), "{value:?}");
    assert!(value.contains("e\u{301}"), "{value:?}");
    assert!(value.chars().count() <= MAX_TASK_CHARS);
}
