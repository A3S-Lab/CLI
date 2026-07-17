use super::*;
use crate::top::Risk;

fn process(pid: u32, ppid: u32, agent: Option<AgentKind>) -> ProcessRow {
    ProcessRow {
        pid,
        ppid,
        cpu_pct: 0.0,
        mem_pct: 0.0,
        elapsed: "1s".to_string(),
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

async fn write_raw_presence(directory: &Path, presence: &AgentPresence) -> PathBuf {
    let path = directory.join(format!("{}.json", presence.instance_id));
    tokio::fs::write(&path, serde_json::to_vec(presence).unwrap())
        .await
        .unwrap();
    path
}

#[test]
fn exact_presence_replaces_same_process_and_keeps_inferred_agents() {
    let now = epoch_ms();
    let exact = presence("local", 10, AgentActivityState::Working);
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
    assert_eq!(rows[1].confidence, AgentActivityConfidence::Process);
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

    let rows = aggregate_activities(&[stale], &[], "new", now);

    assert!(rows.is_empty());
}

#[test]
fn activity_order_puts_attention_and_live_local_state_first() {
    let now = epoch_ms();
    let local = presence("local", 1, AgentActivityState::Working);
    let failed = presence("remote", 2, AgentActivityState::Failed);
    let idle = presence("idle", 3, AgentActivityState::Idle);

    let rows = aggregate_activities(&[idle, local, failed], &[], "local", now);

    assert_eq!(rows[0].state, AgentActivityState::Failed);
    assert_eq!(rows[1].state, AgentActivityState::Working);
    assert!(rows[1].local);
    assert_eq!(rows[2].state, AgentActivityState::Idle);
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(publisher.path.as_ref().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let directory_mode = std::fs::metadata(temp.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(directory_mode, 0o700);
    }

    publisher.remove().await;
    assert!(!publisher.path.as_ref().unwrap().exists());
    assert!(publisher.write_presence(&local).await.is_err());
    assert!(!publisher.path.as_ref().unwrap().exists());
}

#[tokio::test]
async fn registry_discovers_multiple_live_publishers() {
    let temp = tempfile::tempdir().unwrap();
    let left = AgentPresencePublisher::for_directory(temp.path().to_path_buf());
    let right = AgentPresencePublisher::for_directory(temp.path().to_path_buf());
    let left_presence = presence(
        left.instance_id(),
        std::process::id(),
        AgentActivityState::Working,
    );
    let right_presence = presence(
        right.instance_id(),
        std::process::id(),
        AgentActivityState::Idle,
    );

    left.write_presence(&left_presence).await.unwrap();
    right.write_presence(&right_presence).await.unwrap();
    let mut stored = left.read_presences(epoch_ms()).await.unwrap();
    stored.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));

    assert_eq!(stored.len(), 2);
    assert!(stored
        .iter()
        .any(|item| item.instance_id == left.instance_id()));
    assert!(stored
        .iter()
        .any(|item| item.instance_id == right.instance_id()));

    left.remove().await;
    right.remove().await;
}

#[test]
fn registry_file_names_are_canonical_pid_and_hex_instances() {
    let valid = parse_presence_file_name(OsStr::new("42-0123456789abcdef.json")).unwrap();
    assert_eq!(valid.pid, 42);
    assert_eq!(valid.instance_id, "42-0123456789abcdef");

    for invalid in [
        "notes.json",
        "42-0123456789abcde.json",
        "42-0123456789abcdef.json.bak",
        "42-0123456789abcdeg.json",
        "042-0123456789abcdef.json",
        "0-0123456789abcdef.json",
        "42-0123456789abcdef-extra.json",
    ] {
        assert!(
            parse_presence_file_name(OsStr::new(invalid)).is_none(),
            "accepted {invalid}"
        );
    }
}

#[tokio::test]
async fn registry_preserves_non_protocol_and_identity_mismatched_files() {
    let temp = tempfile::tempdir().unwrap();
    let reader = AgentPresencePublisher::for_directory(temp.path().to_path_buf());
    let unrelated = temp.path().join("settings.json");
    tokio::fs::write(&unrelated, b"{\"keep\":true}")
        .await
        .unwrap();
    let malformed = temp.path().join("77-0000000000000001.json");
    tokio::fs::write(&malformed, b"not a heartbeat")
        .await
        .unwrap();

    let mismatched = presence("77-0000000000000002", 77, AgentActivityState::Working);
    let mismatched_path = temp.path().join("78-0000000000000002.json");
    tokio::fs::write(&mismatched_path, serde_json::to_vec(&mismatched).unwrap())
        .await
        .unwrap();
    let valid = presence("79-0000000000000003", 79, AgentActivityState::Working);
    let valid_path = write_raw_presence(temp.path(), &valid).await;

    let stored = reader.read_presences(epoch_ms()).await.unwrap();

    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].instance_id, valid.instance_id);
    assert!(unrelated.exists());
    assert!(malformed.exists());
    assert!(mismatched_path.exists());
    assert!(valid_path.exists());
}

#[tokio::test]
async fn unrelated_json_does_not_consume_the_protocol_scan_budget() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..(MAX_PRESENCE_FILES + 20) {
        tokio::fs::write(temp.path().join(format!("unrelated-{index}.json")), b"{}")
            .await
            .unwrap();
    }
    let valid = presence("81-0000000000000004", 81, AgentActivityState::Working);
    write_raw_presence(temp.path(), &valid).await;
    let reader = AgentPresencePublisher::for_directory(temp.path().to_path_buf());

    let stored = reader.read_presences(epoch_ms()).await.unwrap();

    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].instance_id, valid.instance_id);
}

#[tokio::test]
async fn registry_directory_overflow_is_reported_instead_of_returning_partial_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let now = epoch_ms();
    // Every entry is valid so an implementation that stops at the 256-record
    // evidence cap would miss the independent hard directory limit.
    for index in 1..=MAX_REGISTRY_ENTRIES + 1 {
        let pid = (30_000 + index) as u32;
        let mut item = presence(
            &format!("{pid}-{index:016x}"),
            pid,
            AgentActivityState::Working,
        );
        item.updated_at_ms = now;
        write_raw_presence(temp.path(), &item).await;
    }
    let reader = AgentPresencePublisher::for_directory(temp.path().to_path_buf());

    let error = reader.read_presences(now).await.unwrap_err();

    assert!(error.to_string().contains("directory entry limit"));
}

#[tokio::test]
async fn newer_live_presence_is_not_hidden_by_crash_leftovers() {
    let temp = tempfile::tempdir().unwrap();
    let now = epoch_ms();
    for index in 1..=(MAX_PRESENCE_FILES + 20) {
        let instance = format!("{}-{index:016x}", 10_000 + index);
        let mut stale = presence(
            &instance,
            (10_000 + index) as u32,
            AgentActivityState::Working,
        );
        stale.updated_at_ms = now.saturating_sub(PRESENCE_TTL_MS + 1);
        write_raw_presence(temp.path(), &stale).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let live = presence("99-0000000000000007", 99, AgentActivityState::Working);
    write_raw_presence(temp.path(), &live).await;
    let reader = AgentPresencePublisher::for_directory(temp.path().to_path_buf());

    let stored = reader.read_presences(epoch_ms()).await.unwrap();

    assert!(stored
        .iter()
        .any(|presence| presence.instance_id == live.instance_id));
}

#[tokio::test]
async fn canonical_malformed_flood_does_not_consume_the_valid_presence_limit() {
    let temp = tempfile::tempdir().unwrap();
    let live = presence("98-0000000000000008", 98, AgentActivityState::Working);
    write_raw_presence(temp.path(), &live).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    for index in 1..=300usize {
        let pid = 20_000 + index;
        let path = temp.path().join(format!("{pid}-{index:016x}.json"));
        tokio::fs::write(path, b"{}" as &[u8]).await.unwrap();
    }
    let reader = AgentPresencePublisher::for_directory(temp.path().to_path_buf());

    let stored = reader.read_presences(epoch_ms()).await.unwrap();

    assert!(stored
        .iter()
        .any(|presence| presence.instance_id == live.instance_id));
}

#[cfg(unix)]
#[tokio::test]
async fn registry_ignores_symlinks_even_with_protocol_file_names() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    let link = temp.path().join("82-0000000000000005.json");
    symlink(outside.path(), &link).unwrap();
    let reader = AgentPresencePublisher::for_directory(temp.path().to_path_buf());

    let stored = reader.read_presences(epoch_ms()).await.unwrap();

    assert!(stored.is_empty());
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(outside.path().exists());
}

#[tokio::test]
async fn inbound_protocol_strings_are_sanitized_and_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let mut hostile = presence("83-0000000000000006", 83, AgentActivityState::Working);
    hostile.workspace = "/private/source/unsafe\u{1b}[2Jrepo\nname".to_string();
    hostile.task = Some(format!(
        "task\u{1b}]52;c;c2VjcmV0\u{7}\u{202e}{}",
        "x".repeat(MAX_TASK_CHARS + 20)
    ));
    hostile.children = vec![AgentChildPresence {
        id: "child\n\u{1b}[2Jid".to_string(),
        agent: "codex\u{9b}31mred".to_string(),
        task: Some("child\r\nsecret\u{2067}".to_string()),
        state: AgentActivityState::Working,
        started_at_ms: None,
    }];
    write_raw_presence(temp.path(), &hostile).await;
    let reader = AgentPresencePublisher::for_directory(temp.path().to_path_buf());

    let stored = reader.read_presences(epoch_ms()).await.unwrap();
    let stored = &stored[0];

    for value in [
        stored.workspace.as_str(),
        stored.task.as_deref().unwrap(),
        stored.children[0].id.as_str(),
        stored.children[0].agent.as_str(),
        stored.children[0].task.as_deref().unwrap(),
    ] {
        assert!(!value.chars().any(char::is_control), "{value:?}");
        assert!(!value.contains("[2J"), "{value:?}");
        assert!(!value.contains('\u{202e}'), "{value:?}");
        assert!(!value.contains('\u{2067}'), "{value:?}");
    }
    assert!(!stored.workspace.contains("private"));
    assert!(stored.task.as_ref().unwrap().chars().count() <= MAX_TASK_CHARS);
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
            started_at_ms: None,
        }],
        epoch_ms(),
    );

    private.write_presence(&local).await.unwrap();
    let bytes = tokio::fs::read(private.path.as_ref().unwrap())
        .await
        .unwrap();
    let serialized = String::from_utf8(bytes).unwrap();
    let stored = private.read_presences(epoch_ms()).await.unwrap();

    assert_eq!(local.task.as_deref(), Some(private_task.as_str()));
    assert_eq!(stored[0].workspace, "repository");
    assert!(stored[0].task.is_none());
    assert!(stored[0].children[0].task.is_none());
    assert!(!serialized.contains("parent-private"));
    assert!(!serialized.contains("child-private"));
    assert!(!serialized.contains("secret-workspace"));
    assert!(!serialized.contains("session_id"));
    assert!(!serialized.contains("model"));
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

#[tokio::test]
async fn close_cannot_race_a_waiting_write_into_recreating_the_heartbeat() {
    let temp = tempfile::tempdir().unwrap();
    let publisher = AgentPresencePublisher::for_directory(temp.path().to_path_buf());
    let local = presence(
        publisher.instance_id(),
        std::process::id(),
        AgentActivityState::Working,
    );
    let lease = publisher.write_lock.lock().await;
    let writer_publisher = publisher.clone();
    let writer = tokio::spawn(async move { writer_publisher.write_presence(&local).await });
    tokio::task::yield_now().await;
    let remover_publisher = publisher.clone();
    let remover = tokio::spawn(async move { remover_publisher.remove().await });
    while !publisher.closed.load(Ordering::Acquire) {
        tokio::task::yield_now().await;
    }
    drop(lease);

    assert!(writer.await.unwrap().is_err());
    remover.await.unwrap();
    assert!(!publisher.path.as_ref().unwrap().exists());
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
