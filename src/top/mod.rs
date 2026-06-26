//! `a3s top` — a ctop-style monitor for containers, processes, and coding agents.
//!
//! The first milestone intentionally keeps collectors independent from the TUI
//! layer. That lets the same snapshot model later feed `--json`, remote
//! dashboards, or the lightweight `/top` panel inside `a3s code`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use a3s_tui::cmd::{self, Cmd};
use a3s_tui::components::StatusBar;
use a3s_tui::event::KeyEvent;
use a3s_tui::keymap::{KeyBinding, Keymap};
use a3s_tui::layout::{Constraint, Layout};
use a3s_tui::style::{Color, Style};
use a3s_tui::{Event, KeyCode, KeyModifiers, Model, ProgramBuilder};
use tokio::process::Command;

const ACCENT: Color = Color::Rgb(122, 162, 247);
const GREEN: Color = Color::Rgb(158, 206, 106);
const YELLOW: Color = Color::Rgb(224, 175, 104);
const RED: Color = Color::Rgb(247, 118, 142);
const CYAN: Color = Color::Rgb(125, 207, 255);
const ORANGE: Color = Color::Rgb(255, 158, 100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Agents,
    Containers,
    Processes,
    Events,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Agents, Tab::Containers, Tab::Processes, Tab::Events];

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Agents => "Agents",
            Tab::Containers => "Containers",
            Tab::Processes => "Processes",
            Tab::Events => "Events",
        }
    }

    fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Cpu,
    Mem,
    Name,
}

impl SortBy {
    fn next(self) -> Self {
        match self {
            SortBy::Cpu => SortBy::Mem,
            SortBy::Mem => SortBy::Name,
            SortBy::Name => SortBy::Cpu,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortBy::Cpu => "cpu",
            SortBy::Mem => "mem",
            SortBy::Name => "name",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    fn label(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "med",
            Risk::High => "high",
        }
    }

    fn color(self) -> Color {
        match self {
            Risk::Low => GREEN,
            Risk::Medium => YELLOW,
            Risk::High => RED,
        }
    }
}

#[derive(Debug, Clone)]
struct ProcessRow {
    pid: u32,
    ppid: u32,
    cpu_pct: f32,
    mem_pct: f32,
    elapsed: String,
    command: String,
    agent: Option<AgentKind>,
    risk: Risk,
}

#[derive(Debug, Clone)]
struct ContainerRow {
    id: String,
    name: String,
    image: String,
    status: String,
    cpu_pct: Option<f32>,
    mem_usage: String,
    net_io: String,
    block_io: String,
    pids: String,
}

#[derive(Debug, Clone)]
struct EventRow {
    ts: String,
    source: String,
    kind: String,
    message: String,
    risk: Risk,
}

#[derive(Debug, Clone, Default)]
struct TopSnapshot {
    processes: Vec<ProcessRow>,
    containers: Vec<ContainerRow>,
    events: Vec<EventRow>,
    errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentKind {
    A3sCode,
    ClaudeCode,
    Codex,
    Cursor,
    Gemini,
}

impl AgentKind {
    fn label(self) -> &'static str {
        match self {
            AgentKind::A3sCode => "a3s-code",
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Cursor => "cursor",
            AgentKind::Gemini => "gemini",
        }
    }

    fn color(self) -> Color {
        match self {
            AgentKind::A3sCode => ACCENT,
            AgentKind::ClaudeCode => ORANGE,
            AgentKind::Codex => Color::Rgb(16, 163, 127),
            AgentKind::Cursor => Color::Rgb(180, 182, 200),
            AgentKind::Gemini => Color::Rgb(124, 137, 245),
        }
    }
}

#[derive(Debug, Clone)]
enum Action {
    KillProcess(u32, String),
    StopContainer(String, String),
    RestartContainer(String, String),
}

#[derive(Debug, Clone)]
enum Msg {
    Term(Event),
    Snapshot(TopSnapshot),
    Tick,
    ActionDone(String),
}

impl From<Event> for Msg {
    fn from(event: Event) -> Self {
        Msg::Term(event)
    }
}

#[derive(Clone)]
enum TopKey {
    Quit,
    Up,
    Down,
    PageUp,
    PageDown,
    NextTab,
    PrevTab,
    Filter,
    Sort,
    TogglePause,
    Detail,
    Kill,
    Restart,
}

struct TopApp {
    snapshot: TopSnapshot,
    tab: Tab,
    sort_by: SortBy,
    selected: usize,
    scroll: usize,
    filter: String,
    editing_filter: bool,
    detail: bool,
    paused: bool,
    confirm: Option<Action>,
    note: Option<String>,
    interval: Duration,
    width: u16,
    height: u16,
    last_refresh: Option<Instant>,
    keymap: Keymap<TopKey>,
}

impl TopApp {
    fn new(options: TopOptions) -> Self {
        let (width, height) = a3s_tui::terminal::Terminal::size().unwrap_or((100, 30));
        Self {
            snapshot: TopSnapshot::default(),
            tab: options.tab,
            sort_by: SortBy::Cpu,
            selected: 0,
            scroll: 0,
            filter: String::new(),
            editing_filter: false,
            detail: false,
            paused: false,
            confirm: None,
            note: None,
            interval: options.interval,
            width,
            height,
            last_refresh: None,
            keymap: top_keymap(),
        }
    }

    fn reset_position(&mut self) {
        self.selected = 0;
        self.scroll = 0;
        self.detail = false;
    }

    fn visible_len(&self) -> usize {
        match self.tab {
            Tab::Agents => self.filtered_agents().len(),
            Tab::Containers => self.filtered_containers().len(),
            Tab::Processes => self.filtered_processes().len(),
            Tab::Events => self.filtered_events().len(),
        }
    }

    fn visible_height(&self) -> usize {
        let reserved = if self.detail { 12 } else { 5 };
        (self.height as usize).saturating_sub(reserved).max(3)
    }

    fn clamp_selection(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        self.selected = self.selected.min(len - 1);
        let body = self.visible_height();
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + body {
            self.scroll = self.selected + 1 - body;
        }
    }

    fn selected_action(&self, restart: bool) -> Option<Action> {
        match self.tab {
            Tab::Agents | Tab::Processes => self
                .current_process()
                .map(|p| Action::KillProcess(p.pid, display_cmd(&p.command))),
            Tab::Containers if restart => self.current_container().map(|c| {
                Action::RestartContainer(c.id.clone(), format!("{} ({})", c.name, short_id(&c.id)))
            }),
            Tab::Containers => self.current_container().map(|c| {
                Action::StopContainer(c.id.clone(), format!("{} ({})", c.name, short_id(&c.id)))
            }),
            Tab::Events => None,
        }
    }

    fn current_process(&self) -> Option<ProcessRow> {
        match self.tab {
            Tab::Agents => self.filtered_agents().get(self.selected).cloned(),
            Tab::Processes => self.filtered_processes().get(self.selected).cloned(),
            _ => None,
        }
    }

    fn current_container(&self) -> Option<ContainerRow> {
        self.filtered_containers().get(self.selected).cloned()
    }

    fn filtered_agents(&self) -> Vec<ProcessRow> {
        let mut rows: Vec<_> = self
            .snapshot
            .processes
            .iter()
            .filter(|p| p.agent.is_some())
            .filter(|p| self.matches_filter(&p.command))
            .cloned()
            .collect();
        sort_processes(&mut rows, self.sort_by);
        rows
    }

    fn filtered_processes(&self) -> Vec<ProcessRow> {
        let mut rows: Vec<_> = self
            .snapshot
            .processes
            .iter()
            .filter(|p| self.matches_filter(&p.command) || self.matches_filter(&p.pid.to_string()))
            .cloned()
            .collect();
        sort_processes(&mut rows, self.sort_by);
        rows
    }

    fn filtered_containers(&self) -> Vec<ContainerRow> {
        let mut rows: Vec<_> = self
            .snapshot
            .containers
            .iter()
            .filter(|c| {
                self.matches_filter(&c.name)
                    || self.matches_filter(&c.image)
                    || self.matches_filter(&c.status)
            })
            .cloned()
            .collect();
        match self.sort_by {
            SortBy::Cpu => rows.sort_by(|a, b| {
                b.cpu_pct
                    .unwrap_or_default()
                    .partial_cmp(&a.cpu_pct.unwrap_or_default())
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortBy::Mem | SortBy::Name => rows.sort_by(|a, b| a.name.cmp(&b.name)),
        }
        rows
    }

    fn filtered_events(&self) -> Vec<EventRow> {
        self.snapshot
            .events
            .iter()
            .filter(|e| {
                self.matches_filter(&e.source)
                    || self.matches_filter(&e.kind)
                    || self.matches_filter(&e.message)
            })
            .cloned()
            .collect()
    }

    fn matches_filter(&self, text: &str) -> bool {
        self.filter.is_empty() || text.to_lowercase().contains(&self.filter.to_lowercase())
    }

    fn header(&self) -> String {
        let agents = self
            .snapshot
            .processes
            .iter()
            .filter(|p| p.agent.is_some())
            .count();
        let containers = self.snapshot.containers.len();
        let processes = self.snapshot.processes.len();
        let title =
            format!(" a3s top  agents:{agents}  containers:{containers}  processes:{processes} ");
        let right = if self.paused {
            "paused".to_string()
        } else {
            self.last_refresh
                .map(|t| format!("refreshed {:.1}s ago", t.elapsed().as_secs_f32()))
                .unwrap_or_else(|| "loading".to_string())
        };
        let line = format!(
            "{}{}{}",
            title,
            " ".repeat((self.width as usize).saturating_sub(title.len() + right.len())),
            right
        );
        Style::new()
            .fg(Color::BrightWhite)
            .bg(Color::Rgb(35, 40, 60))
            .bold()
            .width(self.width)
            .render(&line)
    }

    fn tabs(&self) -> String {
        let mut parts = Vec::new();
        for tab in Tab::ALL {
            let label = format!(" {} ", tab.label());
            if tab == self.tab {
                parts.push(
                    Style::new()
                        .fg(Color::Black)
                        .bg(ACCENT)
                        .bold()
                        .render(&label),
                );
            } else {
                parts.push(Style::new().fg(Color::BrightBlack).render(&label));
            }
        }
        let mut line = parts.join(" ");
        let filter = if self.editing_filter {
            format!(" /{}_", self.filter)
        } else if self.filter.is_empty() {
            " / filter".to_string()
        } else {
            format!(" /{}", self.filter)
        };
        line.push_str(&Style::new().fg(Color::BrightBlack).render(&filter));
        pad_line(&line, self.width as usize)
    }

    fn table(&self) -> String {
        match self.tab {
            Tab::Agents => self.process_table(self.filtered_agents(), true),
            Tab::Processes => self.process_table(self.filtered_processes(), false),
            Tab::Containers => self.container_table(self.filtered_containers()),
            Tab::Events => self.events_table(self.filtered_events()),
        }
    }

    fn process_table(&self, rows: Vec<ProcessRow>, agents_only: bool) -> String {
        let title = if agents_only {
            " PID     AGENT       CPU%   MEM%   RISK  ELAPSED   COMMAND"
        } else {
            " PID     PPID     CPU%   MEM%   RISK  ELAPSED   COMMAND"
        };
        let mut out = Vec::new();
        out.push(Style::new().fg(Color::BrightBlack).render(title));
        let body = self.visible_height();
        if rows.is_empty() {
            out.push(
                Style::new()
                    .fg(Color::BrightBlack)
                    .italic()
                    .render(if agents_only {
                        " no coding-agent processes found"
                    } else {
                        " no processes match the current filter"
                    }),
            );
            return out.join("\n");
        }
        for (idx, row) in rows.iter().enumerate().skip(self.scroll).take(body) {
            let agent = row.agent.map(|a| a.label()).unwrap_or("-");
            let first_cols = if agents_only {
                format!(
                    " {:<7} {:<10} {:>5.1}  {:>5.1}   {:<4}  {:<8}  ",
                    row.pid,
                    agent,
                    row.cpu_pct,
                    row.mem_pct,
                    row.risk.label(),
                    row.elapsed
                )
            } else {
                format!(
                    " {:<7} {:<7} {:>5.1}  {:>5.1}   {:<4}  {:<8}  ",
                    row.pid,
                    row.ppid,
                    row.cpu_pct,
                    row.mem_pct,
                    row.risk.label(),
                    row.elapsed
                )
            };
            let cmd_width = (self.width as usize)
                .saturating_sub(first_cols.len())
                .max(16);
            let raw = format!("{first_cols}{}", truncate(&row.command, cmd_width));
            let color = row
                .agent
                .map(|a| a.color())
                .unwrap_or_else(|| row.risk.color());
            out.push(self.style_row(idx, &raw, color));
        }
        out.join("\n")
    }

    fn container_table(&self, rows: Vec<ContainerRow>) -> String {
        let mut out = Vec::new();
        out.push(Style::new().fg(Color::BrightBlack).render(
            " CONTAINER       CPU%    MEM              NET I/O           BLOCK I/O         PIDS  STATUS / IMAGE",
        ));
        let body = self.visible_height();
        if rows.is_empty() {
            out.push(
                Style::new()
                    .fg(Color::BrightBlack)
                    .italic()
                    .render(" no running containers found or Docker is unavailable"),
            );
            return out.join("\n");
        }
        for (idx, row) in rows.iter().enumerate().skip(self.scroll).take(body) {
            let name = truncate(&row.name, 15);
            let cpu = row
                .cpu_pct
                .map(|v| format!("{v:.1}"))
                .unwrap_or_else(|| "-".to_string());
            let prefix = format!(
                " {:<15} {:>5}  {:<16} {:<16} {:<16} {:>4}  ",
                name,
                cpu,
                truncate(&row.mem_usage, 16),
                truncate(&row.net_io, 16),
                truncate(&row.block_io, 16),
                truncate(&row.pids, 4)
            );
            let tail = format!("{} · {}", row.status, row.image);
            let raw = format!(
                "{prefix}{}",
                truncate(&tail, (self.width as usize).saturating_sub(prefix.len()))
            );
            out.push(self.style_row(idx, &raw, CYAN));
        }
        out.join("\n")
    }

    fn events_table(&self, rows: Vec<EventRow>) -> String {
        let mut out = Vec::new();
        out.push(
            Style::new()
                .fg(Color::BrightBlack)
                .render(" TIME      SOURCE        KIND          RISK  MESSAGE"),
        );
        let body = self.visible_height();
        if rows.is_empty() {
            out.push(
                Style::new()
                    .fg(Color::BrightBlack)
                    .italic()
                    .render(" no observer events yet · set A3S_TOP_OBSERVER_LOG to an NDJSON file"),
            );
            return out.join("\n");
        }
        for (idx, row) in rows.iter().enumerate().skip(self.scroll).take(body) {
            let prefix = format!(
                " {:<9} {:<13} {:<13} {:<4}  ",
                truncate(&row.ts, 9),
                truncate(&row.source, 13),
                truncate(&row.kind, 13),
                row.risk.label()
            );
            let raw = format!(
                "{prefix}{}",
                truncate(
                    &row.message,
                    (self.width as usize).saturating_sub(prefix.len())
                )
            );
            out.push(self.style_row(idx, &raw, row.risk.color()));
        }
        out.join("\n")
    }

    fn style_row(&self, idx: usize, raw: &str, color: Color) -> String {
        let line = pad_plain(raw, self.width as usize);
        if idx == self.selected {
            Style::new().fg(Color::Black).bg(color).bold().render(&line)
        } else {
            Style::new().fg(color).render(&line)
        }
    }

    fn details(&self) -> String {
        if !self.detail {
            return String::new();
        }
        let mut lines = Vec::new();
        lines.push(
            Style::new()
                .fg(Color::BrightBlack)
                .render(&"─".repeat(self.width as usize)),
        );
        match self.tab {
            Tab::Agents | Tab::Processes => {
                if let Some(row) = self.current_process() {
                    lines.push(
                        Style::new()
                            .fg(row.agent.map(|a| a.color()).unwrap_or(ACCENT))
                            .bold()
                            .render(&format!(
                                " process {} · ppid {} · risk {}",
                                row.pid,
                                row.ppid,
                                row.risk.label()
                            )),
                    );
                    lines.push(format!(
                        " cpu {:.1}% · mem {:.1}% · elapsed {}",
                        row.cpu_pct, row.mem_pct, row.elapsed
                    ));
                    lines.push(format!(
                        " agent {}",
                        row.agent.map(|a| a.label()).unwrap_or("none")
                    ));
                    lines.push(format!(" command {}", row.command));
                }
            }
            Tab::Containers => {
                if let Some(row) = self.current_container() {
                    lines.push(Style::new().fg(CYAN).bold().render(&format!(
                        " container {} ({})",
                        row.name,
                        short_id(&row.id)
                    )));
                    lines.push(format!(" image {}", row.image));
                    lines.push(format!(" status {}", row.status));
                    lines.push(format!(
                        " cpu {} · mem {} · net {} · block {} · pids {}",
                        row.cpu_pct
                            .map(|v| format!("{v:.1}%"))
                            .unwrap_or_else(|| "-".to_string()),
                        row.mem_usage,
                        row.net_io,
                        row.block_io,
                        row.pids
                    ));
                }
            }
            Tab::Events => {
                if let Some(row) = self.filtered_events().get(self.selected) {
                    lines.push(Style::new().fg(row.risk.color()).bold().render(&format!(
                        " event {} · {} · risk {}",
                        row.source,
                        row.kind,
                        row.risk.label()
                    )));
                    lines.push(format!(" time {}", row.ts));
                    lines.push(format!(" message {}", row.message));
                }
            }
        }
        lines
            .into_iter()
            .take(10)
            .map(|line| pad_line(&line, self.width as usize))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn confirm_view(&self) -> String {
        let Some(action) = &self.confirm else {
            return String::new();
        };
        let (title, target) = match action {
            Action::KillProcess(pid, label) => {
                ("Terminate process?", format!("PID {pid} · {label}"))
            }
            Action::StopContainer(_, name) => ("Stop container?", name.clone()),
            Action::RestartContainer(_, name) => ("Restart container?", name.clone()),
        };
        let width = self.width as usize;
        let inner = 58.min(width.saturating_sub(4)).max(24);
        let line = "─".repeat(inner);
        let target = truncate(&target, inner.saturating_sub(4));
        let rows = [
            format!("┌{line}┐"),
            format!("│{}│", center(title, inner)),
            format!("│{}│", center("", inner)),
            format!("│{}│", center(&target, inner)),
            format!(
                "│{}│",
                center("[ y / Enter ] confirm     [ n / Esc ] cancel", inner)
            ),
            format!("└{line}┘"),
        ];
        let styled = rows
            .iter()
            .map(|r| Style::new().fg(Color::BrightWhite).bg(RED).bold().render(r))
            .collect::<Vec<_>>()
            .join("\n");
        let pad = width.saturating_sub(inner + 2) / 2;
        styled
            .lines()
            .map(|line| format!("{}{}", " ".repeat(pad), line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Model for TopApp {
    type Msg = Msg;

    fn init(&mut self) -> Option<Cmd<Msg>> {
        Some(cmd::cmd(|| async {
            Msg::Snapshot(collect_snapshot().await)
        }))
    }

    fn update(&mut self, msg: Msg) -> Option<Cmd<Msg>> {
        match msg {
            Msg::Snapshot(snapshot) => {
                self.snapshot = snapshot;
                self.last_refresh = Some(Instant::now());
                self.clamp_selection();
                Some(cmd::tick(self.interval, Msg::Tick))
            }
            Msg::Tick => {
                if self.paused {
                    Some(cmd::tick(self.interval, Msg::Tick))
                } else {
                    Some(cmd::cmd(|| async {
                        Msg::Snapshot(collect_snapshot().await)
                    }))
                }
            }
            Msg::ActionDone(note) => {
                self.note = Some(note);
                Some(cmd::cmd(|| async {
                    Msg::Snapshot(collect_snapshot().await)
                }))
            }
            Msg::Term(Event::Resize { width, height }) => {
                self.width = width;
                self.height = height;
                self.clamp_selection();
                None
            }
            Msg::Term(Event::Key(key)) => self.handle_key(key),
            Msg::Term(_) => None,
        }
    }

    fn view(&self) -> String {
        let mut body = vec![self.header(), self.tabs(), self.table()];
        let details = self.details();
        if !details.is_empty() {
            body.push(details);
        }
        if let Some(note) = &self.note {
            body.push(Style::new().fg(YELLOW).render(&format!(" {note}")));
        } else if !self.snapshot.errors.is_empty() {
            body.push(
                Style::new()
                    .fg(YELLOW)
                    .render(&format!(" {}", self.snapshot.errors.join(" · "))),
            );
        }

        let main = body.join("\n");
        let help = if self.editing_filter {
            "type filter · Enter apply · Esc clear"
        } else {
            "Tab switch · / filter · s sort · Enter detail · K stop/kill · r restart · q quit"
        };
        let status = StatusBar::new()
            .left(format!(
                " {} · sort:{} · {} rows",
                self.tab.label(),
                self.sort_by.label(),
                self.visible_len()
            ))
            .center(help)
            .right(if self.paused { "paused" } else { "live" })
            .fg(Color::BrightWhite)
            .bg(Color::Rgb(35, 40, 60))
            .view(self.width);

        let mut screen = Layout::vertical()
            .item(&main, Constraint::Fill)
            .item(&status, Constraint::Fixed(1))
            .render(self.height);

        let confirm = self.confirm_view();
        if !confirm.is_empty() {
            screen.push('\n');
            screen.push_str(&confirm);
        }
        screen
    }
}

impl TopApp {
    fn handle_key(&mut self, key: KeyEvent) -> Option<Cmd<Msg>> {
        if self.editing_filter {
            return self.handle_filter_key(key);
        }
        if self.confirm.is_some() {
            return self.handle_confirm_key(key);
        }

        let action = self.keymap.resolve(&key);
        match action {
            Some(TopKey::Quit) => Some(cmd::quit()),
            Some(TopKey::Up) => {
                self.selected = self.selected.saturating_sub(1);
                self.clamp_selection();
                None
            }
            Some(TopKey::Down) => {
                self.selected = (self.selected + 1).min(self.visible_len().saturating_sub(1));
                self.clamp_selection();
                None
            }
            Some(TopKey::PageUp) => {
                self.selected = self.selected.saturating_sub(10);
                self.clamp_selection();
                None
            }
            Some(TopKey::PageDown) => {
                self.selected = (self.selected + 10).min(self.visible_len().saturating_sub(1));
                self.clamp_selection();
                None
            }
            Some(TopKey::NextTab) => {
                self.tab = self.tab.next();
                self.reset_position();
                None
            }
            Some(TopKey::PrevTab) => {
                self.tab = self.tab.prev();
                self.reset_position();
                None
            }
            Some(TopKey::Filter) => {
                self.editing_filter = true;
                None
            }
            Some(TopKey::Sort) => {
                self.sort_by = self.sort_by.next();
                self.clamp_selection();
                None
            }
            Some(TopKey::TogglePause) => {
                self.paused = !self.paused;
                None
            }
            Some(TopKey::Detail) => {
                self.detail = !self.detail;
                None
            }
            Some(TopKey::Kill) => {
                self.confirm = self.selected_action(false);
                None
            }
            Some(TopKey::Restart) => {
                if self.tab == Tab::Containers {
                    self.confirm = self.selected_action(true);
                }
                None
            }
            None => None,
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> Option<Cmd<Msg>> {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.editing_filter = false;
                self.reset_position();
            }
            KeyCode::Enter => {
                self.editing_filter = false;
                self.reset_position();
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.reset_position();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c);
                self.reset_position();
            }
            _ => {}
        }
        None
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Option<Cmd<Msg>> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.confirm = None;
                None
            }
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                let action = self.confirm.take()?;
                Some(cmd::cmd(move || async move {
                    Msg::ActionDone(run_action(action).await)
                }))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct TopOptions {
    tab: Tab,
    interval: Duration,
}

impl Default for TopOptions {
    fn default() -> Self {
        Self {
            tab: Tab::Agents,
            interval: Duration::from_millis(1500),
        }
    }
}

pub async fn run(args: Vec<String>) -> anyhow::Result<()> {
    let options = parse_options(args)?;
    ProgramBuilder::new(TopApp::new(options))
        .with_alt_screen()
        .with_fps(30)
        .run()
        .await?;
    Ok(())
}

fn parse_options(args: Vec<String>) -> anyhow::Result<TopOptions> {
    let mut options = TopOptions::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--agents" => options.tab = Tab::Agents,
            "--containers" => options.tab = Tab::Containers,
            "--processes" => options.tab = Tab::Processes,
            "--events" => options.tab = Tab::Events,
            "--watch" | "--interval" => {
                let value = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{arg} requires a value"))?;
                options.interval = parse_duration(&value)?;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(anyhow::anyhow!("unknown a3s top option '{other}'")),
        }
    }
    Ok(options)
}

fn print_help() {
    println!(
        "a3s top — live monitor for coding agents, containers, and processes\n\n\
         usage:\n  \
           a3s top [--agents|--containers|--processes|--events] [--watch 1500ms]\n\n\
         keys:\n  \
           Tab/Shift+Tab switch tabs · ↑/↓ select · / filter · s sort · Space pause\n  \
           Enter detail · K terminate/stop · r restart container · q quit\n\n\
         observer:\n  \
           set A3S_TOP_OBSERVER_LOG=/path/to/events.ndjson to show observer events"
    );
}

fn top_keymap() -> Keymap<TopKey> {
    let mut km = Keymap::new();
    km.register(KeyBinding::new(KeyCode::Char('q')), TopKey::Quit, "quit");
    km.register(
        KeyBinding::with_modifiers(KeyCode::Char('c'), KeyModifiers::CONTROL),
        TopKey::Quit,
        "quit",
    );
    km.register(KeyBinding::new(KeyCode::Up), TopKey::Up, "select up");
    km.register(KeyBinding::new(KeyCode::Char('k')), TopKey::Up, "select up");
    km.register(KeyBinding::new(KeyCode::Down), TopKey::Down, "select down");
    km.register(
        KeyBinding::new(KeyCode::Char('j')),
        TopKey::Down,
        "select down",
    );
    km.register(KeyBinding::new(KeyCode::PageUp), TopKey::PageUp, "page up");
    km.register(
        KeyBinding::new(KeyCode::PageDown),
        TopKey::PageDown,
        "page down",
    );
    km.register(KeyBinding::new(KeyCode::Tab), TopKey::NextTab, "next tab");
    km.register(KeyBinding::new(KeyCode::Right), TopKey::NextTab, "next tab");
    km.register(
        KeyBinding::new(KeyCode::BackTab),
        TopKey::PrevTab,
        "previous tab",
    );
    km.register(
        KeyBinding::new(KeyCode::Left),
        TopKey::PrevTab,
        "previous tab",
    );
    km.register(
        KeyBinding::new(KeyCode::Char('/')),
        TopKey::Filter,
        "filter",
    );
    km.register(KeyBinding::new(KeyCode::Char('s')), TopKey::Sort, "sort");
    km.register(
        KeyBinding::new(KeyCode::Char(' ')),
        TopKey::TogglePause,
        "pause",
    );
    km.register(KeyBinding::new(KeyCode::Enter), TopKey::Detail, "detail");
    km.register(
        KeyBinding::new(KeyCode::Char('x')),
        TopKey::Detail,
        "detail",
    );
    km.register(
        KeyBinding::new(KeyCode::Char('K')),
        TopKey::Kill,
        "terminate",
    );
    km.register(
        KeyBinding::new(KeyCode::Char('r')),
        TopKey::Restart,
        "restart",
    );
    km
}

async fn collect_snapshot() -> TopSnapshot {
    let (processes, containers, mut events) = tokio::join!(
        collect_processes(),
        collect_containers(),
        collect_observer_events(),
    );
    let (processes, process_error) = match processes {
        Ok(rows) => (rows, None),
        Err(err) => (Vec::new(), Some(format!("process collector: {err}"))),
    };
    let (containers, container_error) = match containers {
        Ok(rows) => (rows, None),
        Err(err) => (Vec::new(), Some(format!("container collector: {err}"))),
    };

    let mut errors = Vec::new();
    errors.extend(process_error);
    errors.extend(container_error);
    if events.is_empty() {
        events.extend(errors.iter().map(|err| EventRow {
            ts: "now".into(),
            source: "collector".into(),
            kind: "warning".into(),
            message: err.clone(),
            risk: Risk::Medium,
        }));
    }

    TopSnapshot {
        processes,
        containers,
        events,
        errors,
    }
}

async fn collect_processes() -> anyhow::Result<Vec<ProcessRow>> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,pcpu=,pmem=,etime=,args="])
        .output()
        .await?;
    if !output.status.success() {
        return Err(anyhow::anyhow!("ps exited with status {}", output.status));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut rows = text
        .lines()
        .filter_map(parse_process_line)
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.agent.is_some().cmp(&a.agent.is_some()).then(
            b.cpu_pct
                .partial_cmp(&a.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    Ok(rows)
}

fn parse_process_line(line: &str) -> Option<ProcessRow> {
    let mut it = line.split_whitespace();
    let pid = it.next()?.parse().ok()?;
    let ppid = it.next()?.parse().ok()?;
    let cpu_pct = it.next()?.parse().ok()?;
    let mem_pct = it.next()?.parse().ok()?;
    let elapsed = it.next()?.to_string();
    let command = it.collect::<Vec<_>>().join(" ");
    if command.is_empty() {
        return None;
    }
    let agent = detect_agent(&command);
    Some(ProcessRow {
        pid,
        ppid,
        cpu_pct,
        mem_pct,
        elapsed,
        risk: process_risk(&command, agent),
        command,
        agent,
    })
}

async fn collect_containers() -> anyhow::Result<Vec<ContainerRow>> {
    let ps = Command::new("docker")
        .args([
            "ps",
            "--no-trunc",
            "--format",
            "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}",
        ])
        .output()
        .await;
    let Ok(ps) = ps else {
        return Ok(Vec::new());
    };
    if !ps.status.success() {
        return Ok(Vec::new());
    }
    let text = String::from_utf8_lossy(&ps.stdout);
    let mut containers = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            Some(ContainerRow {
                id: parts.next()?.to_string(),
                name: parts.next()?.to_string(),
                image: parts.next()?.to_string(),
                status: parts.next().unwrap_or_default().to_string(),
                cpu_pct: None,
                mem_usage: "-".into(),
                net_io: "-".into(),
                block_io: "-".into(),
                pids: "-".into(),
            })
        })
        .collect::<Vec<_>>();
    if containers.is_empty() {
        return Ok(containers);
    }

    let stats = Command::new("docker")
        .args([
            "stats",
            "--no-stream",
            "--format",
            "{{.ID}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}\t{{.BlockIO}}\t{{.PIDs}}",
        ])
        .output()
        .await;
    let Ok(stats) = stats else {
        return Ok(containers);
    };
    if !stats.status.success() {
        return Ok(containers);
    }
    let stats_text = String::from_utf8_lossy(&stats.stdout);
    let mut by_id: HashMap<String, (Option<f32>, String, String, String, String)> = HashMap::new();
    for line in stats_text.lines() {
        let mut parts = line.split('\t');
        let Some(id) = parts.next() else { continue };
        let cpu = parts.next().and_then(parse_percent);
        let mem = parts.next().unwrap_or("-").to_string();
        let net = parts.next().unwrap_or("-").to_string();
        let block = parts.next().unwrap_or("-").to_string();
        let pids = parts.next().unwrap_or("-").to_string();
        by_id.insert(id.to_string(), (cpu, mem, net, block, pids));
    }
    for c in &mut containers {
        let short = short_id(&c.id);
        let stats = by_id.get(&c.id).or_else(|| by_id.get(short)).cloned();
        if let Some((cpu, mem, net, block, pids)) = stats {
            c.cpu_pct = cpu;
            c.mem_usage = mem;
            c.net_io = net;
            c.block_io = block;
            c.pids = pids;
        }
    }
    Ok(containers)
}

async fn collect_observer_events() -> Vec<EventRow> {
    let Some(path) = std::env::var_os("A3S_TOP_OBSERVER_LOG") else {
        return Vec::new();
    };
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => text,
        Err(err) => {
            return vec![EventRow {
                ts: "now".into(),
                source: "observer".into(),
                kind: "error".into(),
                message: format!("failed to read observer log: {err}"),
                risk: Risk::Medium,
            }];
        }
    };
    text.lines()
        .rev()
        .take(200)
        .filter_map(parse_observer_line)
        .collect::<Vec<_>>()
}

fn parse_observer_line(line: &str) -> Option<EventRow> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let identity = value.get("identity")?;
    let source = identity
        .get("agent")
        .and_then(|v| v.as_str())
        .unwrap_or("agent")
        .to_string();
    let event = value.get("event")?.as_object()?;
    let (kind, payload) = event.iter().next()?;
    let message = match payload {
        serde_json::Value::Object(map) => map
            .iter()
            .take(4)
            .map(|(k, v)| format!("{k}={}", compact_json_value(v)))
            .collect::<Vec<_>>()
            .join(" "),
        other => compact_json_value(other),
    };
    let risk = match kind.as_str() {
        "SecurityAction" | "FileDelete" => Risk::High,
        "Egress" | "FileAccess" | "ToolExec" => Risk::Medium,
        _ => Risk::Low,
    };
    Some(EventRow {
        ts: "recent".into(),
        source,
        kind: kind.clone(),
        message,
        risk,
    })
}

async fn run_action(action: Action) -> String {
    match action {
        Action::KillProcess(pid, label) => {
            let status = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status()
                .await;
            match status {
                Ok(s) if s.success() => format!("sent SIGTERM to PID {pid} · {label}"),
                Ok(s) => format!("kill failed for PID {pid}: {s}"),
                Err(err) => format!("kill failed for PID {pid}: {err}"),
            }
        }
        Action::StopContainer(id, name) => {
            let status = Command::new("docker").args(["stop", &id]).status().await;
            match status {
                Ok(s) if s.success() => format!("stopped container {name}"),
                Ok(s) => format!("docker stop failed for {name}: {s}"),
                Err(err) => format!("docker stop failed for {name}: {err}"),
            }
        }
        Action::RestartContainer(id, name) => {
            let status = Command::new("docker").args(["restart", &id]).status().await;
            match status {
                Ok(s) if s.success() => format!("restarted container {name}"),
                Ok(s) => format!("docker restart failed for {name}: {s}"),
                Err(err) => format!("docker restart failed for {name}: {err}"),
            }
        }
    }
}

fn detect_agent(command: &str) -> Option<AgentKind> {
    let l = command.to_lowercase();
    if l.contains("a3s-code") || l.contains("a3s code") || l.ends_with("/a3s") {
        Some(AgentKind::A3sCode)
    } else if l.contains("claude") {
        Some(AgentKind::ClaudeCode)
    } else if l.contains("codex") {
        Some(AgentKind::Codex)
    } else if l.contains("cursor-agent") || l.contains("cursor") {
        Some(AgentKind::Cursor)
    } else if l.contains("gemini") {
        Some(AgentKind::Gemini)
    } else {
        None
    }
}

fn process_risk(command: &str, agent: Option<AgentKind>) -> Risk {
    let lower = command.to_lowercase();
    if lower.contains("sudo ")
        || lower.contains(" rm -rf ")
        || lower.contains("ptrace")
        || lower.contains("nmap ")
    {
        Risk::High
    } else if agent.is_some()
        || lower.contains("docker ")
        || lower.contains("curl ")
        || lower.contains("bash -c")
    {
        Risk::Medium
    } else {
        Risk::Low
    }
}

fn sort_processes(rows: &mut [ProcessRow], sort_by: SortBy) {
    match sort_by {
        SortBy::Cpu => rows.sort_by(|a, b| {
            b.agent.is_some().cmp(&a.agent.is_some()).then(
                b.cpu_pct
                    .partial_cmp(&a.cpu_pct)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        }),
        SortBy::Mem => rows.sort_by(|a, b| {
            b.agent.is_some().cmp(&a.agent.is_some()).then(
                b.mem_pct
                    .partial_cmp(&a.mem_pct)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        }),
        SortBy::Name => rows.sort_by(|a, b| a.command.cmp(&b.command)),
    }
}

fn parse_percent(s: &str) -> Option<f32> {
    s.trim().trim_end_matches('%').parse().ok()
}

fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    if let Some(ms) = s.strip_suffix("ms") {
        return Ok(Duration::from_millis(ms.parse()?));
    }
    if let Some(sec) = s.strip_suffix('s') {
        let seconds: f64 = sec.parse()?;
        return Ok(Duration::from_secs_f64(seconds));
    }
    Ok(Duration::from_millis(s.parse()?))
}

fn compact_json_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => truncate(s, 80),
        _ => truncate(&v.to_string(), 80),
    }
}

fn display_cmd(command: &str) -> String {
    truncate(command, 64)
}

fn short_id(id: &str) -> &str {
    id.get(..12.min(id.len())).unwrap_or(id)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let mut out = s.chars().take(max - 1).collect::<String>();
    out.push('…');
    out
}

fn pad_plain(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        truncate(s, width)
    } else {
        format!("{s}{}", " ".repeat(width - len))
    }
}

fn pad_line(s: &str, width: usize) -> String {
    let visible = a3s_tui::style::visible_len(s);
    if visible >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - visible))
    }
}

fn center(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        return truncate(s, width);
    }
    let left = (width - len) / 2;
    format!(
        "{}{}{}",
        " ".repeat(left),
        s,
        " ".repeat(width - len - left)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_agents() {
        assert_eq!(detect_agent("/usr/bin/a3s code"), Some(AgentKind::A3sCode));
        assert_eq!(
            detect_agent("node /bin/claude"),
            Some(AgentKind::ClaudeCode)
        );
        assert_eq!(detect_agent("codex exec task"), Some(AgentKind::Codex));
    }

    #[test]
    fn parses_process_rows() {
        let row = parse_process_line(" 123  1  2.5  0.7  01:02:03 codex exec hello").unwrap();
        assert_eq!(row.pid, 123);
        assert_eq!(row.ppid, 1);
        assert_eq!(row.agent, Some(AgentKind::Codex));
        assert_eq!(row.cpu_pct, 2.5);
    }

    #[test]
    fn parses_durations() {
        assert_eq!(parse_duration("250ms").unwrap(), Duration::from_millis(250));
        assert_eq!(parse_duration("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_duration("1500").unwrap(), Duration::from_millis(1500));
    }

    #[test]
    fn parses_observer_events() {
        let line = r#"{"identity":{"agent":"codex","task":"1","session":null},"provider":null,"event":{"ToolExec":{"pid":2,"argv":["git","status"],"cwd":"/tmp"}}}"#;
        let row = parse_observer_line(line).unwrap();
        assert_eq!(row.source, "codex");
        assert_eq!(row.kind, "ToolExec");
        assert_eq!(row.risk, Risk::Medium);
    }
}
