use super::*;

#[test]
fn parent_terminal_state_uses_the_same_eight_second_retention() {
    let now = Instant::now();
    let recent = RecentTerminalState {
        session_id: "session-a".to_string(),
        state: AgentActivityState::Completed,
        task: Some("task".to_string()),
        started_at_ms: Some(10),
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
fn failed_parent_terminal_state_is_retained_for_export() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = AgentPresenceRuntime {
        publisher: AgentPresencePublisher::for_directory(temp.path().to_path_buf()),
        refreshing: false,
        terminal: None,
        island: AgentIslandSupervisor::default(),
        last_warnings: Vec::new(),
    };

    runtime.record_terminal(
        "session-a".to_string(),
        AgentActivityState::Failed,
        Some("failed task".to_string()),
        Some(10),
    );

    let terminal = runtime.recent_terminal("session-a").unwrap();
    assert_eq!(terminal.state, AgentActivityState::Failed);
    assert_eq!(terminal.task.as_deref(), Some("failed task"));
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
    let recent_end = now.checked_sub(Duration::from_secs(1)).unwrap();
    let recent_start = recent_end.checked_sub(Duration::from_secs(2)).unwrap();
    runtime.start_subagent(
        "recent".to_string(),
        "review".to_string(),
        "recent review".to_string(),
        recent_start,
    );
    runtime.end_subagent(
        "recent".to_string(),
        "review".to_string(),
        "done".to_string(),
        true,
        recent_end,
    );

    let expired = child_presence("child".to_string(), runtime.subagents()[0], now, epoch_ms());
    let recent = child_presence(
        "recent".to_string(),
        runtime.subagents()[1],
        now,
        epoch_ms(),
    )
    .unwrap();

    assert!(expired.is_none());
    assert_eq!(recent.state, AgentActivityState::Completed);
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
fn island_binary_override_is_honored_without_silent_fallback() {
    let override_path = PathBuf::from("/explicit/a3s-webview");
    let environment = AgentIslandEnvironment {
        binary_override: Some(override_path.clone()),
        ..AgentIslandEnvironment::default()
    };

    assert_eq!(
        resolve_agent_island_binary(&environment).unwrap(),
        override_path
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
        last_warnings: Vec::new(),
    };
    let result = SystemAgentRefreshResult {
        snapshot_path: Some(temp.path().join("system-snapshot.json")),
        lock_path: Some(temp.path().join("island.lock")),
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
fn failed_export_leaves_the_island_waiting_for_a_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = AgentPresenceRuntime {
        publisher: AgentPresencePublisher::for_directory(temp.path().to_path_buf()),
        refreshing: true,
        terminal: None,
        island: AgentIslandSupervisor::default(),
        last_warnings: Vec::new(),
    };

    assert!(runtime
        .apply_refresh(SystemAgentRefreshResult {
            warnings: vec!["snapshot: unavailable".to_string()],
            ..SystemAgentRefreshResult::default()
        })
        .is_none());
    assert!(matches!(
        runtime.island.lifecycle,
        AgentIslandLifecycle::AwaitingSnapshot
    ));
}

#[test]
fn retry_backoff_is_tick_driven_and_stops_after_four_consecutive_failures() {
    let request = AgentIslandLaunchRequest {
        snapshot_path: PathBuf::from("/private/state/system-snapshot.json"),
        lock_path: PathBuf::from("/private/state/island.lock"),
    };
    let mut supervisor = AgentIslandSupervisor::default();
    assert_eq!(
        supervisor.observe_snapshot(request.clone()),
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
    assert!(matches!(
        supervisor.lifecycle,
        AgentIslandLifecycle::Stopped
    ));
    assert!(supervisor.poll(now + Duration::from_secs(60)).is_none());
}

#[test]
fn obsolete_helper_is_a_permanent_stop_instead_of_a_crash_loop() {
    let request = AgentIslandLaunchRequest {
        snapshot_path: PathBuf::from("/private/state/system-snapshot.json"),
        lock_path: PathBuf::from("/private/state/island.lock"),
    };
    let mut supervisor = AgentIslandSupervisor::default();
    assert!(supervisor.observe_snapshot(request).is_some());

    supervisor.apply_launch_result(
        Ok(AgentIslandLaunchOutcome::Unsupported(
            "RemoteUI-only helper".to_string(),
        )),
        Instant::now(),
    );

    assert!(matches!(
        supervisor.lifecycle,
        AgentIslandLifecycle::Stopped
    ));
    assert_eq!(supervisor.consecutive_failures, 0);
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

    assert!(matches!(
        supervisor.lifecycle,
        AgentIslandLifecycle::Backoff { .. }
    ));
    assert_eq!(supervisor.consecutive_failures, 0);
    assert!(supervisor
        .poll(now + AGENT_ISLAND_CONTENTION_RECHECK - Duration::from_millis(1))
        .is_none());
    assert_eq!(
        supervisor.poll(now + AGENT_ISLAND_CONTENTION_RECHECK),
        Some(request)
    );
}

#[test]
fn watchdog_length_success_is_relaunched_with_bounded_backoff() {
    let request = AgentIslandLaunchRequest {
        snapshot_path: PathBuf::from("/private/state/system-snapshot.json"),
        lock_path: PathBuf::from("/private/state/island.lock"),
    };
    let mut supervisor = AgentIslandSupervisor::default();
    supervisor.request = Some(request.clone());
    let now = Instant::now();

    supervisor.apply_exit(
        AgentIslandExit {
            success: true,
            status: "exit status: 0".to_string(),
            detail: String::new(),
            ran_for: Duration::from_secs(20),
        },
        now,
    );

    assert!(matches!(
        supervisor.lifecycle,
        AgentIslandLifecycle::Backoff { .. }
    ));
    assert_eq!(supervisor.consecutive_failures, 1);
    assert_eq!(
        supervisor.poll(now + agent_island_retry_delay(1)),
        Some(request)
    );
}

#[tokio::test]
async fn helper_monitor_reaps_a_failed_child_and_reports_its_exit() {
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
