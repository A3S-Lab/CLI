//! Global coding-agent "dynamic island" projection.
//!
//! The island is a presentation adapter over [`crate::system_agents`]. It
//! merges the current in-memory A3S lifecycle on every render with the latest
//! cross-process snapshot, so the local row never waits for the background
//! collector while remote rows remain bounded and eventually stale.

use super::*;
use crate::system_agents::{
    activities_for_presence, epoch_ms, sanitize_display_text, sort_activities, AgentActivityState,
    AgentChildPresence, AgentPresence, AgentPresencePublisher, SystemAgentActivity,
    SystemAgentSnapshot,
};

pub(super) const SYSTEM_AGENT_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const SYSTEM_AGENT_STALE_AFTER: Duration = Duration::from_secs(6);
const EXPANDED_MAX_ROWS: usize = 6;
const EXPANDED_MIN_WIDTH: usize = 34;
const EXPANDED_MAX_WIDTH: usize = 78;
const TERMINAL_STATE_RETENTION: Duration = Duration::from_secs(8);

#[derive(Debug)]
struct RecentTerminalState {
    session_id: String,
    state: AgentActivityState,
    task: Option<String>,
    started_at_ms: Option<u64>,
    recorded_at: Instant,
}

#[derive(Debug)]
pub(super) struct SystemAgentIsland {
    pub(super) publisher: AgentPresencePublisher,
    snapshot: SystemAgentSnapshot,
    refreshed_at: Option<Instant>,
    pub(super) refreshing: bool,
    pub(super) expanded: bool,
    terminal: Option<RecentTerminalState>,
}

impl SystemAgentIsland {
    pub(super) fn new() -> Self {
        Self {
            publisher: AgentPresencePublisher::from_environment(),
            snapshot: SystemAgentSnapshot::default(),
            refreshed_at: None,
            refreshing: false,
            expanded: false,
            terminal: None,
        }
    }

    pub(super) fn apply(&mut self, snapshot: SystemAgentSnapshot) {
        self.snapshot = snapshot;
        self.refreshed_at = Some(Instant::now());
        self.refreshing = false;
    }

    fn activities(&self, local: &AgentPresence) -> Vec<SystemAgentActivity> {
        let local_id = self.publisher.instance_id();
        let mut activities = self
            .snapshot
            .activities
            .iter()
            .filter(|activity| {
                activity.id != local_id && activity.parent_id.as_deref() != Some(local_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        activities.extend(activities_for_presence(local, true));
        sort_activities(&mut activities);
        activities
    }

    fn stale(&self) -> bool {
        self.refreshed_at
            .is_some_and(|refreshed| refreshed.elapsed() > SYSTEM_AGENT_STALE_AFTER)
    }

    fn recent_terminal(&self, session_id: &str) -> Option<&RecentTerminalState> {
        self.terminal.as_ref().filter(|terminal| {
            terminal.session_id == session_id
                && terminal.recorded_at.elapsed() <= TERMINAL_STATE_RETENTION
        })
    }

    pub(super) fn record_terminal(
        &mut self,
        session_id: String,
        state: AgentActivityState,
        task: Option<String>,
        started_at_ms: Option<u64>,
    ) {
        self.terminal = Some(RecentTerminalState {
            session_id,
            state,
            task,
            started_at_ms,
            recorded_at: Instant::now(),
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentIslandFrame {
    pub(super) rows: Vec<String>,
    pub(super) width: usize,
    pub(super) start_col: usize,
    pub(super) expanded: bool,
}

impl AgentIslandFrame {
    pub(super) fn hit_test(&self, row: u16, col: u16) -> bool {
        usize::from(row) < self.rows.len()
            && usize::from(col) >= self.start_col
            && usize::from(col) < self.start_col.saturating_add(self.width)
    }
}

impl App {
    pub(super) fn local_agent_presence(&self) -> AgentPresence {
        let now_ms = epoch_ms();
        let now = Instant::now();
        let children = self
            .runtime
            .subagent_ids()
            .into_iter()
            .zip(self.runtime.subagents())
            .filter_map(|(id, child)| child_presence(id, child, now, now_ms))
            .collect();

        let recent_terminal = self.system_agent_island.recent_terminal(&self.session_id);
        let state =
            match self.state {
                State::Awaiting => AgentActivityState::WaitingApproval,
                State::Rebuilding => AgentActivityState::Working,
                State::Streaming
                    if self.plan.tasks().iter().any(|task| {
                        task.status == a3s_code_core::planning::TaskStatus::InProgress
                    }) =>
                {
                    AgentActivityState::Planning
                }
                State::Streaming => AgentActivityState::Working,
                State::Idle => recent_terminal
                    .map(|terminal| terminal.state)
                    .unwrap_or(AgentActivityState::Idle),
            };
        let task = self
            .running_task
            .clone()
            .or_else(|| recent_terminal.and_then(|terminal| terminal.task.clone()));
        let started_at_ms = self
            .stream_started
            .map(|started| {
                now_ms
                    .saturating_sub(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
            })
            .or_else(|| recent_terminal.and_then(|terminal| terminal.started_at_ms))
            .unwrap_or_else(|| self.system_agent_island.publisher.started_at_ms());

        AgentPresence::new(
            self.system_agent_island.publisher.instance_id(),
            std::process::id(),
            &self.cwd,
            task,
            state,
            children,
            started_at_ms,
        )
    }

    pub(super) fn refresh_system_agents(&mut self) -> Cmd<Msg> {
        self.system_agent_island.refreshing = true;
        let publisher = self.system_agent_island.publisher.clone();
        let local = self.local_agent_presence();
        cmd::cmd(move || async move {
            Msg::SystemAgentsRefreshed(publisher.publish_and_collect(local).await)
        })
    }

    pub(super) fn system_agent_island_frame(&self) -> AgentIslandFrame {
        let local = self.local_agent_presence();
        let activities = self.system_agent_island.activities(&local);
        render_agent_island(
            &activities,
            self.system_agent_island.expanded,
            self.system_agent_island.stale(),
            !self.system_agent_island.snapshot.warnings.is_empty(),
            self.blink_tick,
            self.width as usize,
            self.height as usize,
        )
    }

    pub(super) fn overlay_system_agent_island(&self, frame: String) -> String {
        let island = self.system_agent_island_frame();
        TextOverlay::new(island.rows)
            .top()
            .width(self.width as usize)
            .centered()
            .apply(&frame)
    }

    pub(super) fn toggle_system_agent_island(&mut self) {
        self.system_agent_island.expanded = !self.system_agent_island.expanded;
    }

    pub(super) fn record_local_agent_terminal(&mut self, state: AgentActivityState) {
        let started_at_ms = self.stream_started.map(|started| {
            epoch_ms()
                .saturating_sub(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
        });
        self.system_agent_island.record_terminal(
            self.session_id.clone(),
            state,
            self.running_task.clone(),
            started_at_ms,
        );
    }
}

fn child_presence(
    id: String,
    child: &runtime_projection::SubagentRun,
    now: Instant,
    now_ms: u64,
) -> Option<AgentChildPresence> {
    if child
        .ended
        .is_some_and(|ended| now.saturating_duration_since(ended) > TERMINAL_STATE_RETENTION)
    {
        return None;
    }

    let state = match child.outcome {
        Some(runtime_projection::SubagentOutcome::Succeeded) => AgentActivityState::Completed,
        Some(runtime_projection::SubagentOutcome::Failed) => AgentActivityState::Failed,
        Some(runtime_projection::SubagentOutcome::Cancelled) => AgentActivityState::Cancelled,
        Some(runtime_projection::SubagentOutcome::TrackingLost) => AgentActivityState::Unknown,
        None => AgentActivityState::Working,
    };
    Some(AgentChildPresence {
        id,
        agent: child.display_agent(),
        task: nonempty_island_text(&child.description),
        state,
        started_at_ms: Some(
            now_ms.saturating_sub(
                now.saturating_duration_since(child.started)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            ),
        ),
    })
}

pub(super) fn system_agent_tick() -> Cmd<Msg> {
    cmd::tick(SYSTEM_AGENT_REFRESH_INTERVAL, Msg::SystemAgentsTick)
}

pub(super) fn is_system_agent_island_key(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('g') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn render_agent_island(
    activities: &[SystemAgentActivity],
    expanded: bool,
    stale: bool,
    degraded: bool,
    blink_tick: u8,
    screen_width: usize,
    screen_height: usize,
) -> AgentIslandFrame {
    if screen_width == 0 || screen_height == 0 {
        return AgentIslandFrame {
            rows: Vec::new(),
            width: 0,
            start_col: 0,
            expanded: false,
        };
    }

    let can_expand = expanded && screen_width >= EXPANDED_MIN_WIDTH && screen_height >= 5;
    let rows = if can_expand {
        expanded_rows(activities, stale, degraded, screen_width, screen_height)
    } else {
        vec![collapsed_row(
            activities,
            stale,
            degraded,
            blink_tick,
            screen_width,
        )]
    };
    let width = rows
        .iter()
        .map(|row| a3s_tui::style::visible_len(row))
        .max()
        .unwrap_or_default()
        .min(screen_width);

    AgentIslandFrame {
        rows,
        width,
        start_col: screen_width.saturating_sub(width) / 2,
        expanded: can_expand,
    }
}

fn collapsed_row(
    activities: &[SystemAgentActivity],
    stale: bool,
    degraded: bool,
    blink_tick: u8,
    screen_width: usize,
) -> String {
    let failures = activities
        .iter()
        .filter(|activity| activity.state == AgentActivityState::Failed)
        .count();
    let waiting = activities
        .iter()
        .filter(|activity| {
            matches!(
                activity.state,
                AgentActivityState::WaitingApproval | AgentActivityState::WaitingInput
            )
        })
        .count();
    let active = activities
        .iter()
        .filter(|activity| activity.state.is_active())
        .count();
    let cancelled = activities
        .iter()
        .filter(|activity| activity.state == AgentActivityState::Cancelled)
        .count();
    let inferred = activities
        .iter()
        .filter(|activity| activity.state == AgentActivityState::Unknown)
        .count();
    let completed = activities
        .iter()
        .filter(|activity| activity.state == AgentActivityState::Completed)
        .count();
    let total = activities.len();

    let (glyph, color, summary) = if stale {
        (
            "◌",
            COMPOSER_CHROME.warning,
            "agent status stale".to_string(),
        )
    } else if failures > 0 {
        (
            "×",
            COMPOSER_CHROME.error,
            format!("{failures} agent{} failed", plural(failures)),
        )
    } else if waiting > 0 {
        (
            "!",
            COMPOSER_CHROME.warning,
            format!("{waiting} agent{} waiting", plural(waiting)),
        )
    } else if active > 0 {
        let pulse = ["●", "◉", "●", "•"][(blink_tick as usize / 2) % 4];
        (
            pulse,
            COMPOSER_CHROME.active,
            format!("{active}/{total} agents working"),
        )
    } else if cancelled > 0 {
        (
            "–",
            COMPOSER_CHROME.faint,
            format!("{cancelled} agent{} cancelled", plural(cancelled)),
        )
    } else if inferred > 0 {
        (
            "◌",
            COMPOSER_CHROME.faint,
            format!("{total} agent{} detected", plural(total)),
        )
    } else if completed > 0 {
        (
            "✓",
            COMPOSER_CHROME.success,
            format!("{completed} agent{} completed", plural(completed)),
        )
    } else {
        (
            "○",
            COMPOSER_CHROME.faint,
            format!("{total} agent{} idle", plural(total)),
        )
    };
    let degraded = if degraded { " · degraded" } else { "" };
    let shortcut = if screen_width >= 44 { " · Ctrl+G" } else { "" };
    let body = format!(
        "{} {}{}{}",
        Style::new().fg(color).bold().render(glyph),
        Style::new().fg(COMPOSER_CHROME.primary).render(&summary),
        Style::new().fg(COMPOSER_CHROME.faint).render(degraded),
        Style::new().fg(COMPOSER_CHROME.faint).render(shortcut),
    );
    let row = format!(
        "{}{}{}",
        Style::new().fg(COMPOSER_CHROME.faint).render("╭ "),
        body,
        Style::new().fg(COMPOSER_CHROME.faint).render(" ╮")
    );
    a3s_tui::style::truncate_visible(&row, screen_width)
}

fn expanded_rows(
    activities: &[SystemAgentActivity],
    stale: bool,
    degraded: bool,
    screen_width: usize,
    screen_height: usize,
) -> Vec<String> {
    let panel_width = screen_width
        .saturating_sub(2)
        .min(EXPANDED_MAX_WIDTH)
        .max(EXPANDED_MIN_WIDTH.min(screen_width));
    let inner = panel_width.saturating_sub(2);
    let active = activities
        .iter()
        .filter(|activity| activity.state.is_active())
        .count();
    let status = if stale {
        "stale".to_string()
    } else if degraded {
        "degraded".to_string()
    } else {
        format!("{active}/{} active", activities.len())
    };
    let title = a3s_tui::style::truncate_visible(&format!("─ System agents · {status} "), inner);
    let top_fill = inner.saturating_sub(a3s_tui::style::visible_len(&title));
    let mut rows = vec![format!(
        "{}{}{}{}",
        Style::new().fg(COMPOSER_CHROME.faint).render("╭"),
        Style::new()
            .fg(COMPOSER_CHROME.primary)
            .bold()
            .render(&title),
        Style::new()
            .fg(COMPOSER_CHROME.faint)
            .render(&"─".repeat(top_fill)),
        Style::new().fg(COMPOSER_CHROME.faint).render("╮")
    )];

    let max_rows = EXPANDED_MAX_ROWS
        .min(screen_height.saturating_sub(3))
        .max(1);
    if activities.is_empty() {
        rows.push(panel_content_row("No coding agents detected", inner));
    } else {
        rows.extend(
            activities
                .iter()
                .take(max_rows)
                .map(|activity| activity_row(activity, inner)),
        );
        if activities.len() > max_rows {
            rows.push(panel_content_row(
                &format!("… {} more agents", activities.len() - max_rows),
                inner,
            ));
        }
    }

    let hint = a3s_tui::style::truncate_visible("─ Ctrl+G / Esc close · click to collapse ", inner);
    let bottom_fill = inner.saturating_sub(a3s_tui::style::visible_len(&hint));
    rows.push(format!(
        "{}{}{}{}",
        Style::new().fg(COMPOSER_CHROME.faint).render("╰"),
        Style::new().fg(COMPOSER_CHROME.faint).render(&hint),
        Style::new()
            .fg(COMPOSER_CHROME.faint)
            .render(&"─".repeat(bottom_fill)),
        Style::new().fg(COMPOSER_CHROME.faint).render("╯")
    ));
    rows
}

fn activity_row(activity: &SystemAgentActivity, inner: usize) -> String {
    let (glyph, color) = state_glyph(activity.state);
    let child = if activity.parent_id.is_some() {
        "↳"
    } else {
        " "
    };
    let local = if activity.local { " this" } else { "" };
    let agent_name =
        visible_island_text(&activity.agent, 64).unwrap_or_else(|| "agent".to_string());
    let agent = format!("{child} {agent_name}{local}");
    let elapsed = activity
        .started_at_ms
        .map(|started| {
            format!(
                " · {}",
                format_island_elapsed(epoch_ms().saturating_sub(started))
            )
        })
        .unwrap_or_default();
    let state = format!(
        "{} {}{}",
        activity.state.label(),
        activity.confidence.label(),
        elapsed
    );
    let right_width = a3s_tui::style::visible_len(&state).min(inner);
    let left_width = inner.saturating_sub(right_width + usize::from(right_width > 0));
    let task = activity
        .task
        .as_deref()
        .and_then(|task| visible_island_text(task, 240));
    let workspace = activity
        .workspace
        .as_deref()
        .and_then(|workspace| visible_island_text(workspace, 128));
    let detail = task
        .as_deref()
        .or(workspace.as_deref())
        .unwrap_or("no task detail");
    let prefix = format!(
        "{} {}  {}",
        Style::new().fg(color).render(glyph),
        Style::new().fg(COMPOSER_CHROME.primary).render(&agent),
        Style::new().fg(COMPOSER_CHROME.secondary).render(detail),
    );
    let left = a3s_tui::style::fit_visible(&prefix, left_width);
    let right = Style::new().fg(color).render(&state);
    let content = if right_width == 0 {
        left
    } else {
        format!("{left} {right}")
    };
    bordered_content_row(&content, inner)
}

fn panel_content_row(text: &str, inner: usize) -> String {
    let content = Style::new()
        .fg(COMPOSER_CHROME.secondary)
        .render(&a3s_tui::style::fit_visible(text, inner));
    bordered_content_row(&content, inner)
}

fn bordered_content_row(content: &str, inner: usize) -> String {
    format!(
        "{}{}{}",
        Style::new().fg(COMPOSER_CHROME.faint).render("│"),
        a3s_tui::style::fit_visible(content, inner),
        Style::new().fg(COMPOSER_CHROME.faint).render("│")
    )
}

fn state_glyph(state: AgentActivityState) -> (&'static str, Color) {
    match state {
        AgentActivityState::Planning => ("◐", COMPOSER_CHROME.active),
        AgentActivityState::Working => ("●", COMPOSER_CHROME.active),
        AgentActivityState::WaitingApproval | AgentActivityState::WaitingInput => {
            ("!", COMPOSER_CHROME.warning)
        }
        AgentActivityState::Idle => ("○", COMPOSER_CHROME.faint),
        AgentActivityState::Completed => ("✓", COMPOSER_CHROME.success),
        AgentActivityState::Failed => ("×", COMPOSER_CHROME.error),
        AgentActivityState::Cancelled => ("–", COMPOSER_CHROME.faint),
        AgentActivityState::Unknown => ("◌", COMPOSER_CHROME.faint),
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn format_island_elapsed(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    if seconds >= 3_600 {
        format!("{}h {:02}m", seconds / 3_600, (seconds % 3_600) / 60)
    } else if seconds >= 60 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn nonempty_island_text(value: &str) -> Option<String> {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!value.is_empty()).then_some(value)
}

fn visible_island_text(value: &str, max_chars: usize) -> Option<String> {
    let value = sanitize_display_text(value, max_chars);
    (!value.is_empty() && a3s_tui::style::visible_len(&value) > 0).then_some(value)
}

#[cfg(test)]
#[path = "agent_island_tests.rs"]
mod tests;
