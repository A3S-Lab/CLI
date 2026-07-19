use super::*;

#[test]
fn task_and_plan_start_project_non_idle_parent_lifecycle() {
    assert_eq!(
        parent_presence_state(State::Streaming, false, None),
        AgentActivityState::Working
    );
    assert_eq!(
        parent_presence_state(State::Streaming, true, None),
        AgentActivityState::Planning
    );
    assert_eq!(
        parent_presence_state(State::Awaiting, false, None),
        AgentActivityState::WaitingApproval
    );
    assert_eq!(
        parent_presence_state(State::Rebuilding, false, None),
        AgentActivityState::Working
    );
}

#[test]
fn idle_parent_uses_only_a_recent_terminal_lifecycle() {
    assert_eq!(
        parent_presence_state(State::Idle, false, None),
        AgentActivityState::Idle
    );
    assert_eq!(
        parent_presence_state(State::Idle, false, Some(AgentActivityState::Completed)),
        AgentActivityState::Completed
    );
}

#[test]
fn parent_terminal_state_uses_the_same_eight_second_retention() {
    let now = Instant::now();
    let recent = RecentTerminalState {
        session_id: "session-a".to_string(),
        state: AgentActivityState::Completed,
        task: Some("task".to_string()),
        started_at_ms: Some(10),
        finished_at_ms: 20,
        recorded_at: now.checked_sub(Duration::from_secs(7)).unwrap(),
    };
    assert!(recent.is_visible_at("session-a", now));
    assert!(!recent.is_visible_at("session-b", now));
    let expired = RecentTerminalState {
        recorded_at: now
            .checked_sub(TERMINAL_STATE_RETENTION + Duration::from_millis(1))
            .unwrap(),
        ..recent
    };

    assert!(!expired.is_visible_at("session-a", now));
}

#[test]
fn completed_child_presence_expires_after_terminal_retention() {
    let now = Instant::now();
    let old_end = now
        .checked_sub(TERMINAL_STATE_RETENTION + Duration::from_millis(1))
        .unwrap();
    let old_start = old_end.checked_sub(Duration::from_secs(2)).unwrap();
    let mut runtime = RuntimeProjection::default();
    runtime.start_subagent(
        "child".to_string(),
        "review".to_string(),
        "finished review".to_string(),
        old_start,
    );
    runtime.end_subagent(
        "child".to_string(),
        "review".to_string(),
        "done".to_string(),
        true,
        old_end,
    );

    let expired = child_presence(
        "child".to_string(),
        runtime.subagents()[0],
        AgentVendor::Other,
        now,
        epoch_ms(),
    );

    assert!(expired.is_none());
}

#[test]
fn island_launch_gate_defaults_to_desktop_only_and_allows_explicit_enablement() {
    let automatic_desktop = AgentIslandEnvironment {
        display_available: true,
        linux: true,
        ..AgentIslandEnvironment::default()
    };
    assert_eq!(automatic_desktop.skip_reason(), None);

    let headless = AgentIslandEnvironment {
        linux: true,
        ..AgentIslandEnvironment::default()
    };
    assert_eq!(
        headless.skip_reason(),
        Some("headless Linux session without a display server")
    );

    let ssh = AgentIslandEnvironment {
        ssh: true,
        display_available: true,
        ..AgentIslandEnvironment::default()
    };
    assert_eq!(
        ssh.skip_reason(),
        Some("SSH session without explicit island enablement")
    );

    for setting in ["1", "true", "ON"] {
        let explicit = AgentIslandEnvironment {
            setting: Some(OsString::from(setting)),
            ssh: true,
            linux: true,
            ..AgentIslandEnvironment::default()
        };
        assert_eq!(explicit.skip_reason(), None, "setting {setting}");
    }
}

#[test]
fn island_launch_gate_honors_explicit_disablement() {
    for setting in ["0", "false", "OFF"] {
        let environment = AgentIslandEnvironment {
            setting: Some(OsString::from(setting)),
            display_available: true,
            ..AgentIslandEnvironment::default()
        };
        assert_eq!(
            environment.skip_reason(),
            Some("disabled by A3S_AGENT_ISLAND"),
            "setting {setting}"
        );
    }
}

#[test]
fn island_preference_command_accepts_status_on_and_off_only() {
    assert_eq!(
        preference::parse_agent_island_preference_command(""),
        Ok(preference::AgentIslandPreferenceCommand::Status)
    );
    assert_eq!(
        preference::parse_agent_island_preference_command("ON"),
        Ok(preference::AgentIslandPreferenceCommand::Enable)
    );
    assert_eq!(
        preference::parse_agent_island_preference_command("off"),
        Ok(preference::AgentIslandPreferenceCommand::Disable)
    );
    assert!(preference::parse_agent_island_preference_command("toggle").is_err());
}

#[tokio::test]
async fn disabling_supervisor_stops_a_running_helper_and_blocks_launches() {
    let request = AgentIslandLaunchRequest {
        snapshot_path: PathBuf::from("/private/state/system-snapshot.json"),
        lock_path: PathBuf::from("/private/state/island.lock"),
    };
    let (_exit_tx, exit) = oneshot::channel();
    let (shutdown, shutdown_rx) = oneshot::channel();
    let mut supervisor = AgentIslandSupervisor {
        enabled: true,
        lifecycle: AgentIslandLifecycle::Running(AgentIslandMonitor {
            exit,
            started_at: Instant::now(),
            shutdown: Some(shutdown),
        }),
        request: Some(request.clone()),
        consecutive_failures: 0,
    };

    supervisor.set_enabled(false);
    tokio::time::timeout(Duration::from_secs(1), shutdown_rx)
        .await
        .expect("helper stop signal timed out")
        .expect("helper stop sender was dropped");
    assert!(!supervisor.enabled);
    assert!(supervisor.request.is_none());
    assert!(matches!(
        supervisor.lifecycle,
        AgentIslandLifecycle::Stopped
    ));
    assert!(supervisor.observe_snapshot(request.clone(), true).is_none());

    supervisor.set_enabled(true);
    assert_eq!(
        supervisor.observe_snapshot(request.clone(), true),
        Some(request)
    );
}

#[test]
fn island_helper_arguments_match_the_native_mode_contract() {
    let request = AgentIslandLaunchRequest {
        snapshot_path: PathBuf::from("/private/state/system-snapshot.json"),
        lock_path: PathBuf::from("/private/state/island.lock"),
    };

    assert_eq!(
        agent_island_args(&request),
        vec![
            OsString::from("--agent-island"),
            OsString::from("--snapshot"),
            OsString::from("/private/state/system-snapshot.json"),
            OsString::from("--lock-file"),
            OsString::from("/private/state/island.lock"),
        ]
    );
}

#[test]
fn successful_export_starts_one_launch_while_the_first_is_in_flight() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = AgentPresenceRuntime {
        publisher: AgentPresencePublisher::for_directory(temp.path().to_path_buf()),
        refreshing: true,
        terminal: None,
        island: AgentIslandSupervisor::default(),
        cancel_requested: std::collections::HashSet::new(),
        last_warnings: Vec::new(),
    };
    let result = SystemAgentRefreshResult {
        snapshot_path: Some(temp.path().join("system-snapshot.json")),
        lock_path: Some(temp.path().join("island.lock")),
        launch_requested: true,
        control_requests: Vec::new(),
        warnings: Vec::new(),
    };

    assert!(runtime.apply_refresh(result.clone()).is_some());
    assert!(!runtime.refreshing);
    assert!(runtime.apply_refresh(result).is_none());
    assert!(matches!(
        runtime.island.lifecycle,
        AgentIslandLifecycle::Launching
    ));
}

#[test]
fn idle_export_does_not_start_the_native_island() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = AgentPresenceRuntime {
        publisher: AgentPresencePublisher::for_directory(temp.path().to_path_buf()),
        refreshing: true,
        terminal: None,
        island: AgentIslandSupervisor::default(),
        cancel_requested: std::collections::HashSet::new(),
        last_warnings: Vec::new(),
    };

    assert!(runtime
        .apply_refresh(SystemAgentRefreshResult {
            snapshot_path: Some(temp.path().join("system-snapshot.json")),
            lock_path: Some(temp.path().join("island.lock")),
            launch_requested: false,
            control_requests: Vec::new(),
            warnings: Vec::new(),
        })
        .is_none());
    assert!(matches!(
        runtime.island.lifecycle,
        AgentIslandLifecycle::AwaitingSnapshot
    ));
    assert!(runtime.island.request.is_none());
}

#[test]
fn retry_backoff_is_tick_driven_and_cools_down_after_four_consecutive_failures() {
    let request = AgentIslandLaunchRequest {
        snapshot_path: PathBuf::from("/private/state/system-snapshot.json"),
        lock_path: PathBuf::from("/private/state/island.lock"),
    };
    let mut supervisor = AgentIslandSupervisor::default();
    assert_eq!(
        supervisor.observe_snapshot(request.clone(), true),
        Some(request.clone())
    );

    let mut now = Instant::now();
    for failure in 1..AGENT_ISLAND_MAX_CONSECUTIVE_FAILURES {
        supervisor.apply_launch_result(Err(format!("failure {failure}")), now);
        let delay = agent_island_retry_delay(failure);
        assert!(supervisor
            .poll(now + delay.saturating_sub(Duration::from_millis(1)))
            .is_none());
        now += delay;
        assert_eq!(supervisor.poll(now), Some(request.clone()));
    }

    supervisor.apply_launch_result(Err("final failure".to_string()), now);
    assert_eq!(supervisor.consecutive_failures, 0);
    assert!(supervisor
        .poll(now + AGENT_ISLAND_RECOVERY_RECHECK - Duration::from_millis(1))
        .is_none());
    assert_eq!(
        supervisor.poll(now + AGENT_ISLAND_RECOVERY_RECHECK),
        Some(request)
    );
}

#[test]
fn singleton_contention_rechecks_infrequently_for_eventual_takeover() {
    let mut supervisor = AgentIslandSupervisor::default();
    let request = AgentIslandLaunchRequest {
        snapshot_path: PathBuf::from("/private/state/system-snapshot.json"),
        lock_path: PathBuf::from("/private/state/island.lock"),
    };
    supervisor.request = Some(request.clone());
    let now = Instant::now();

    supervisor.apply_exit(
        AgentIslandExit {
            success: true,
            status: "exit status: 0".to_string(),
            detail: String::new(),
            ran_for: Duration::from_millis(100),
        },
        now,
    );

    assert!(supervisor
        .poll(now + AGENT_ISLAND_CONTENTION_RECHECK - Duration::from_millis(1))
        .is_none());
    assert_eq!(
        supervisor.poll(now + AGENT_ISLAND_CONTENTION_RECHECK),
        Some(request)
    );
}

#[tokio::test]
async fn helper_monitor_reaps_a_failed_child_and_reports_its_exit() {
    let _process_probe_guard = crate::system_agents::agent_island_process_test_lock()
        .lock()
        .await;
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .arg("--definitely-not-a-valid-libtest-option")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut monitor = monitor_agent_island_child(child);

    let exit = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(exit) = monitor.try_take_exit() {
                return exit;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child monitor did not report the reaped process");

    assert!(!exit.success);
    assert!(!exit.status.is_empty());
}

#[tokio::test]
async fn bounded_reader_retains_the_prefix_while_draining_to_eof() {
    use tokio::io::AsyncWriteExt;

    let payload = vec![b'x'; 64 * 1024];
    let (mut writer, reader) = tokio::io::duplex(16);
    let write = tokio::spawn(async move {
        writer.write_all(&payload).await.unwrap();
    });

    let retained = tokio::time::timeout(Duration::from_secs(1), read_bounded(Some(reader), 17))
        .await
        .expect("bounded reader stopped draining after reaching its retention limit");
    write.await.unwrap();

    assert_eq!(retained, vec![b'x'; 17]);
}
