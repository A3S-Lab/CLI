use super::*;
use crate::system_agents::AgentActivityConfidence;

fn activity(id: &str, agent: &str, task: &str, state: AgentActivityState) -> SystemAgentActivity {
    SystemAgentActivity {
        id: id.to_string(),
        parent_id: None,
        agent: agent.to_string(),
        workspace: Some("/workspace/a3s".to_string()),
        task: Some(task.to_string()),
        state,
        confidence: AgentActivityConfidence::Exact,
        started_at_ms: Some(epoch_ms().saturating_sub(2_000)),
        local: id == "local",
    }
}

#[test]
fn collapsed_island_prioritizes_waiting_and_failure_states() {
    let waiting = vec![activity(
        "local",
        "a3s-code",
        "approve shell",
        AgentActivityState::WaitingApproval,
    )];
    let failed = vec![activity(
        "remote",
        "codex",
        "verify",
        AgentActivityState::Failed,
    )];

    let waiting = render_agent_island(&waiting, false, false, false, 0, 80, 24);
    let failed = render_agent_island(&failed, false, false, false, 0, 80, 24);
    let waiting = a3s_tui::style::strip_ansi(&waiting.rows[0]);
    let failed = a3s_tui::style::strip_ansi(&failed.rows[0]);

    assert!(waiting.contains("waiting"), "{waiting}");
    assert!(failed.contains("failed"), "{failed}");
}

#[test]
fn collapsed_island_reports_recent_terminal_states() {
    for (state, expected) in [
        (AgentActivityState::Completed, "completed"),
        (AgentActivityState::Cancelled, "cancelled"),
    ] {
        let activities = vec![activity("local", "a3s-code", "task", state)];
        let frame = render_agent_island(&activities, false, false, false, 0, 80, 24);
        let row = a3s_tui::style::strip_ansi(&frame.rows[0]);

        assert!(row.contains(expected), "{row}");
    }
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
fn expanded_island_is_bounded_and_keeps_cjk_task_text() {
    let activities = vec![activity(
        "local",
        "a3s-code",
        "实现跨平台灵动岛并验证 very long detail",
        AgentActivityState::Working,
    )];

    let frame = render_agent_island(&activities, true, false, false, 0, 52, 12);
    let plain = frame
        .rows
        .iter()
        .map(|row| a3s_tui::style::strip_ansi(row))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(frame.expanded);
    assert!(plain.contains("实现跨平台"), "{plain}");
    assert!(frame
        .rows
        .iter()
        .all(|row| { a3s_tui::style::visible_len(row) == frame.width && frame.width <= 52 }));
}

#[test]
fn narrow_or_short_terminals_fall_back_to_one_row() {
    let activities = vec![activity(
        "local",
        "a3s-code",
        "task",
        AgentActivityState::Working,
    )];

    for (width, height) in [(20, 20), (80, 3)] {
        let frame = render_agent_island(&activities, true, false, false, 0, width, height);
        assert!(!frame.expanded);
        assert_eq!(frame.rows.len(), 1);
        assert!(frame.width <= width);
    }
}

#[test]
fn expanded_island_fits_every_supported_narrow_width() {
    let activities = vec![activity(
        "local",
        "a3s-code",
        "task",
        AgentActivityState::Working,
    )];

    for width in EXPANDED_MIN_WIDTH..=44 {
        let frame = render_agent_island(&activities, true, false, false, 0, width, 12);

        assert!(frame.expanded, "width {width}");
        assert!(frame.width <= width, "width {width}: {frame:?}");
        assert!(
            frame.rows.iter().all(|row| {
                a3s_tui::style::visible_len(row) == frame.width && frame.width <= width
            }),
            "width {width}: {frame:?}"
        );
    }
}

#[test]
fn frame_hitbox_matches_centered_visible_bounds() {
    let frame = render_agent_island(&[], false, false, false, 0, 80, 24);

    assert!(frame.hit_test(0, frame.start_col as u16));
    assert!(!frame.hit_test(1, frame.start_col as u16));
    assert!(!frame.hit_test(0, frame.start_col.saturating_add(frame.width) as u16));
}

#[test]
fn expanded_island_sanitizes_hostile_agent_and_task_text_before_styling() {
    let activities = vec![activity(
        "remote",
        "co\u{1b}[2Jdex\nspoof",
        "safe task \u{1b}]52;c;c2VjcmV0\u{7}\u{202e}still safe",
        AgentActivityState::Working,
    )];

    let frame = render_agent_island(&activities, true, false, false, 0, 72, 12);
    let rendered = frame.rows.join("\n");
    let plain = frame
        .rows
        .iter()
        .map(|row| a3s_tui::style::strip_ansi(row))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!rendered.contains("\u{1b}[2J"), "{rendered:?}");
    assert!(!rendered.contains("]52;"), "{rendered:?}");
    assert!(!rendered.contains('\u{202e}'), "{rendered:?}");
    assert!(plain.contains("codex spoof"), "{plain:?}");
    assert!(plain.contains("safe task still safe"), "{plain:?}");
    assert!(frame.rows.iter().all(|row| !row.contains(['\n', '\r'])));
}

#[test]
fn expanded_island_sanitizes_workspace_fallback() {
    let mut item = activity(
        "remote",
        "\u{301}\u{200d}",
        "\u{301}\u{200d}",
        AgentActivityState::Working,
    );
    item.workspace = Some("repo\u{1b}[2J\nworkspace".to_string());

    let frame = render_agent_island(&[item], true, false, false, 0, 72, 12);
    let rendered = frame.rows.join("\n");
    let plain = frame
        .rows
        .iter()
        .map(|row| a3s_tui::style::strip_ansi(row))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(!rendered.contains("\u{1b}[2J"), "{rendered:?}");
    assert!(plain.contains("agent"), "{plain:?}");
    assert!(plain.contains("repo workspace"), "{plain:?}");
}
