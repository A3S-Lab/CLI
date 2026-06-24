//! Codex-style terminal UI for the A3S Code agent.
//!
//! Built on the `a3s-tui` TEA framework: it drives an [`AgentSession`] via
//! `session.stream()` and renders the resulting [`AgentEvent`] stream as a live
//! chat transcript, with an inline (y/n/a) approval prompt for tool calls.
//!
//! Streaming bridge: `session.stream()` yields a `tokio::mpsc` receiver. A
//! self-re-issuing "pump" command reads one event, turns it into a `Msg`, and
//! the update handler issues the next pump — feeding the async event stream into
//! the synchronous TEA update loop one event at a time.

use std::collections::BinaryHeap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_code_core::hitl::TimeoutAction;
use a3s_code_core::{Agent, AgentEvent, AgentSession, SessionOptions};
use a3s_tui::cmd::{self, Cmd};
use a3s_tui::components::textarea::TextareaMsg;
use a3s_tui::components::viewport::ViewportMsg;
use a3s_tui::components::{Spinner, Textarea, Viewport};
use a3s_tui::event::KeyEvent;
use a3s_tui::keymap::{KeyBinding, Keymap};
use a3s_tui::layout::{Constraint, Layout};
use a3s_tui::streaming::StreamingMarkdown;
use a3s_tui::style::{Color, Style};
use a3s_tui::{Event, KeyCode, KeyModifiers, Model, ProgramBuilder};
use tokio::sync::{mpsc, Mutex};

/// Theme accent — ShuAn OS blue. Single source of truth for the UI accent color.
const ACCENT: Color = Color::Rgb(37, 99, 235);

/// Built-in slash commands shown in the `/` menu.
const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/model", "switch provider / model"),
    ("/config", "edit .a3s/config.acl in your editor"),
    ("/btw", "ask a background side-question (/btw <prompt>)"),
    ("/help", "show commands and shortcuts"),
    ("/clear", "reset the conversation"),
    ("/auto", "switch to auto-approve mode"),
    ("/exit", "quit a3s code"),
];

/// Open `path` in the user's editor without taking over the TUI terminal: a
/// known GUI editor from $VISUAL/$EDITOR, else the OS default text-editor opener.
fn open_in_editor(path: &str) -> bool {
    // Detached spawn with the TUI's terminal hidden from the child, so a GUI
    // launcher can't print to (and corrupt) the alt-screen.
    let spawn = |bin: &str, pre: &[&str]| -> bool {
        std::process::Command::new(bin)
            .args(pre)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    };
    const GUI: &[&str] = &[
        "code", "cursor", "zed", "subl", "codium", "atom", "mate", "gedit", "windsurf",
    ];
    // 1. An explicit GUI editor from $VISUAL/$EDITOR.
    if let Ok(ed) = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")) {
        let bin = ed.split_whitespace().next().unwrap_or("");
        let base = bin.rsplit('/').next().unwrap_or(bin);
        if GUI.contains(&base) && spawn(bin, &[]) {
            return true;
        }
    }
    // 2. Common GUI editor CLIs on PATH (VS Code first).
    for ed in ["code", "cursor", "zed", "subl", "codium", "windsurf"] {
        if spawn(ed, &[]) {
            return true;
        }
    }
    // 3. The OS default app for the file, then a plain text editor as last resort.
    #[cfg(target_os = "macos")]
    {
        spawn("open", &[]) || spawn("open", &["-t"])
    }
    #[cfg(not(target_os = "macos"))]
    {
        spawn("xdg-open", &[])
    }
}

/// Slash commands whose name starts with `input` (input begins with `/`).
fn slash_candidates(input: &str) -> Vec<(&'static str, &'static str)> {
    SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(input))
        .copied()
        .collect()
}

/// A dim bottom status line with `left` and `right` justified to `width`.
fn status_line(left: &str, right: &str, width: usize) -> String {
    let used = a3s_tui::style::visible_len(left) + a3s_tui::style::visible_len(right);
    let gap = width.saturating_sub(used);
    Style::new()
        .fg(Color::BrightBlack)
        .render(&format!("{left}{}{right}", " ".repeat(gap)))
}

/// Pad a (possibly styled) string with spaces to `width` display columns.
fn pad_to(s: &str, width: usize) -> String {
    let vis = a3s_tui::style::visible_len(s);
    if vis >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - vis))
    }
}

/// Prefix a message block with a colored ● gutter on its first line and align
/// the rest under the text — marks user (blue) vs assistant (green) messages.
fn gutter(color: Color, content: &str) -> String {
    let dot = Style::new().fg(color).bold().render("●");
    let margin = " ".repeat(PAD);
    content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                format!("{margin}{dot} {line}")
            } else {
                format!("{margin}  {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Current git branch of `dir` (cheap: parse `.git/HEAD`), if any.
fn git_branch(dir: &str) -> Option<String> {
    let head = std::fs::read_to_string(format!("{dir}/.git/HEAD")).ok()?;
    head.strip_prefix("ref: refs/heads/")
        .map(|b| b.trim().to_string())
}

/// Left margin for the whole UI (inner padding).
const PAD: usize = 2;

/// Fixed session id so relaunching in the same directory continues the chat.
const SESSION_ID: &str = "tui-default";

/// Run mode, cycled with Shift+Tab.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Approve every tool call.
    Default,
    /// Read-only tools auto-approved; writes still prompt (exploration/planning).
    Plan,
    /// Auto-approve every tool call.
    Auto,
}

impl Mode {
    fn next(self) -> Self {
        match self {
            Mode::Default => Mode::Plan,
            Mode::Plan => Mode::Auto,
            Mode::Auto => Mode::Default,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Mode::Default => "default · approve each",
            Mode::Plan => "plan · auto-read",
            Mode::Auto => "auto · approve all",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Mode::Default => "⏵",
            Mode::Plan => "✎",
            Mode::Auto => "⏵⏵",
        }
    }

    fn color(self) -> Color {
        match self {
            Mode::Default => Color::BrightWhite,
            Mode::Plan => Color::Cyan,
            Mode::Auto => Color::Green,
        }
    }

    /// Whether a tool call is auto-approved in this mode.
    fn auto_approves(self, tool: &str) -> bool {
        match self {
            Mode::Auto => true,
            Mode::Plan => is_readonly_tool(tool),
            Mode::Default => false,
        }
    }
}

/// "1m 05s" / "42s".
fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// "79.9k" / "512".
fn humanize(n: usize) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// Render `text` with a 3-char bright band sweeping left→right (loading shimmer).
/// `phase` advances each frame; the band cycles across the text with a gap.
fn shimmer(text: &str, phase: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let head = phase % (chars.len() + 6);
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let lit = head >= i && head - i < 3;
        let s = if lit {
            Style::new().fg(Color::BrightWhite).bold()
        } else {
            Style::new().fg(ACCENT)
        };
        out.push_str(&s.render(&c.to_string()));
    }
    out
}

fn is_readonly_tool(name: &str) -> bool {
    matches!(
        name,
        "read" | "grep" | "ls" | "glob" | "find" | "search" | "web_search" | "web_fetch"
    )
}

/// A user message queued while the agent is busy. Priority queue: lower `prio`
/// runs first, FIFO within a priority.
struct Queued {
    prio: u8,
    seq: u64,
    text: String,
}

impl PartialEq for Queued {
    fn eq(&self, o: &Self) -> bool {
        self.prio == o.prio && self.seq == o.seq
    }
}
impl Eq for Queued {}
impl Ord for Queued {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        // BinaryHeap is a max-heap; invert so lowest prio, then lowest seq, pops first.
        o.prio.cmp(&self.prio).then(o.seq.cmp(&self.seq))
    }
}
impl PartialOrd for Queued {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// Shared, single-consumer receiver for the active agent run. Wrapped so the
/// pump command can own a clone; pumps run sequentially, so the mutex never
/// actually contends.
type SharedRx = Arc<Mutex<mpsc::Receiver<AgentEvent>>>;

#[derive(PartialEq)]
enum State {
    Idle,
    Streaming,
    Awaiting,
}

#[derive(Clone)]
#[allow(clippy::enum_variant_names)]
enum Action {
    ScrollUp,
    ScrollDown,
    ScrollTop,
    ScrollBottom,
}

enum Msg {
    Term(Event),
    // Boxed: AgentEvent is large; keeps the Msg enum small.
    Agent(Box<AgentEvent>),
    Submit(String),
    StreamStarted(SharedRx),
    StreamEnded,
    StreamError(String),
    SpinnerTick,
    ModalConfirm(usize),
    Resume,
    Interrupted,
    /// Output of a `!`-prefixed shell command.
    ShellOutput(String),
    /// Answer from a `/btw` background side-thread.
    SideNote(String),
}

impl From<Event> for Msg {
    fn from(event: Event) -> Self {
        // Ctrl+C is handled in the key loop (double-press to quit), not here.
        Msg::Term(event)
    }
}

/// Read one event from the active run and turn it into a `Msg`.
fn pump(rx: SharedRx) -> Cmd<Msg> {
    cmd::cmd(move || async move {
        let mut guard = rx.lock().await;
        match guard.recv().await {
            Some(event) => Msg::Agent(Box::new(event)),
            None => Msg::StreamEnded,
        }
    })
}

fn spinner_tick() -> Cmd<Msg> {
    cmd::tick(Duration::from_millis(80), Msg::SpinnerTick)
}

struct App {
    session: Arc<AgentSession>,
    /// Agent + session-rebuild bits, kept so `/model` can switch models by
    /// resuming the session under a new model (no in-place model setter exists).
    agent: Arc<Agent>,
    store: Arc<dyn a3s_code_core::store::SessionStore>,
    confirmation: a3s_code_core::hitl::ConfirmationPolicy,
    /// This session's id (for model-switch resume + the exit hint).
    session_id: String,
    /// "provider/model" ids from the config, for the /model picker.
    models: Vec<String>,
    /// Context-window size per model id, for the ctx% indicator.
    model_ctx: std::collections::HashMap<String, u32>,
    /// Context window of the active model (0 = unknown).
    context_limit: u32,
    /// Prompt tokens of the last turn = current context fill.
    last_prompt_tokens: usize,
    /// Selected index in the /model panel; `Some` means the panel is open.
    model_menu: Option<usize>,
    /// First Ctrl+C arms quit; a second within the window exits.
    quit_armed: Option<Instant>,
    viewport: Viewport,
    textarea: Textarea,
    spinner: Spinner,
    streaming: StreamingMarkdown,
    /// Live reasoning ("thinking") text for the current turn, shown dimmed above
    /// the answer and cleared when the answer is finalized.
    thinking: String,
    state: State,
    messages: Vec<String>,
    rx: Option<SharedRx>,
    pending_tool: Option<(String, String)>,
    /// Submitted prompts, oldest first, for ↑/↓ recall.
    history: Vec<String>,
    /// Cursor into `history` while browsing; `None` means "fresh input".
    history_pos: Option<usize>,
    /// Model name reported by the provider (captured from the first turn).
    model: Option<String>,
    /// Cumulative tokens used this session.
    total_tokens: usize,
    /// Accumulated streamed JSON args of the in-progress tool call, so the
    /// result line can show what the tool actually did (command/path/pattern).
    tool_args: String,
    /// Live stdout of the in-progress tool (e.g. a running command), shown
    /// dimmed under the action and cleared when the tool completes.
    tool_output: String,
    /// When the current run started, for the live elapsed-time indicator.
    stream_started: Option<Instant>,
    /// Name of the tool currently executing (shown live with a blinking dot).
    running_tool: Option<String>,
    /// Animation counter for the blinking running-tool dot (advances per tick).
    blink_tick: u8,
    /// Run mode (Shift+Tab cycles default → plan → auto).
    mode: Mode,
    /// User messages submitted while the agent is busy, run when it frees up.
    queue: BinaryHeap<Queued>,
    /// Monotonic counter for FIFO ordering within a queue priority.
    seq: u64,
    /// Turns completed this session, for the status-bar task counter.
    completed: usize,
    /// Working directory shown for context.
    cwd: String,
    /// Git branch of the workspace (if any), shown in the bottom status bar.
    branch: Option<String>,
    /// Selected index in the `/` command menu.
    slash_sel: usize,
    width: u16,
    height: u16,
    keymap: Keymap<Action>,
}

impl Model for App {
    type Msg = Msg;

    fn init(&mut self) -> Option<Cmd<Msg>> {
        if self.messages.is_empty() {
            self.viewport.set_content(&self.banner());
        } else {
            // Resumed session — show the prior conversation, scrolled to the end.
            self.rebuild_viewport();
            self.viewport.update(ViewportMsg::Bottom);
        }
        None
    }

    fn update(&mut self, msg: Msg) -> Option<Cmd<Msg>> {
        match msg {
            Msg::Term(Event::Resize { width, height }) => {
                self.width = width;
                self.height = height;
                self.viewport.resize(width, height.saturating_sub(7));
                self.textarea
                    .set_width(width.saturating_sub((PAD + 2) as u16));
                self.streaming = StreamingMarkdown::new((width as usize).saturating_sub(PAD + 2));
                self.rebuild_viewport();
            }

            Msg::Term(Event::Key(key)) => {
                // Ctrl+C: arm on the first press, exit on a second within 2s.
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    match self.quit_armed {
                        Some(t) if t.elapsed() < Duration::from_secs(2) => return Some(cmd::quit()),
                        _ => {
                            self.quit_armed = Some(Instant::now());
                            self.push_line(
                                &Style::new()
                                    .fg(Color::Yellow)
                                    .render("  press Ctrl+C again to exit"),
                            );
                            return None;
                        }
                    }
                }
                // Shift+Tab cycles run mode in any state.
                if key.code == KeyCode::BackTab {
                    self.mode = self.mode.next();
                    return None;
                }
                if self.state == State::Awaiting {
                    return self.handle_approval_key(&key);
                }
                // /model picker takes keys while open.
                if self.model_menu.is_some() {
                    if let Some(result) = self.handle_model_key(&key) {
                        return result;
                    }
                }
                if let Some(action) = self.keymap.resolve(&key) {
                    let m = match action {
                        Action::ScrollUp => ViewportMsg::PageUp,
                        Action::ScrollDown => ViewportMsg::PageDown,
                        Action::ScrollTop => ViewportMsg::Top,
                        Action::ScrollBottom => ViewportMsg::Bottom,
                    };
                    self.viewport.update(m);
                    return None;
                }
                // Esc interrupts the in-progress run (input stays usable otherwise).
                if self.state == State::Streaming && key.code == KeyCode::Esc {
                    self.push_line(&Style::new().fg(Color::Yellow).render("  ⎋ interrupting…"));
                    let session = self.session.clone();
                    return Some(cmd::cmd(move || async move {
                        session.cancel().await;
                        Msg::Interrupted
                    }));
                }
                // Slash-command menu: ↑/↓ select, Enter run, Tab complete, Esc
                // dismiss — takes priority over history recall while open.
                if self.slash_menu_open() {
                    if let Some(result) = self.handle_slash_key(&key) {
                        return result;
                    }
                }
                // ↑/↓ recall prompt history (single-line input only, so multi-line
                // editing keeps normal cursor movement).
                if matches!(key.code, KeyCode::Up | KeyCode::Down)
                    && !self.textarea.value().contains('\n')
                    && !self.history.is_empty()
                {
                    self.history_recall(key.code == KeyCode::Up);
                    return None;
                }
                // Input is always live (you can keep typing while the agent works);
                // a submit while busy is queued and run when the current turn ends.
                if let Some(TextareaMsg::Submit(text)) = self.textarea.handle_key(&key) {
                    return Some(cmd::msg(Msg::Submit(text)));
                }
            }

            Msg::Term(Event::Mouse(m)) => {
                use a3s_tui::event::MouseEventKind;
                match m.kind {
                    MouseEventKind::ScrollUp => self.viewport.update(ViewportMsg::ScrollUp(3)),
                    MouseEventKind::ScrollDown => self.viewport.update(ViewportMsg::ScrollDown(3)),
                    _ => {}
                }
            }

            Msg::Submit(text) => return self.on_submit(text),

            Msg::StreamStarted(rx) => {
                self.rx = Some(rx.clone());
                return Some(pump(rx));
            }

            Msg::StreamError(e) => {
                self.push_line(&Style::new().fg(Color::Red).render(&format!("  error: {e}")));
                self.finish();
            }

            Msg::Agent(event) => return self.on_agent_event(*event),

            Msg::StreamEnded => {
                if self.state == State::Streaming {
                    self.finalize_streaming();
                    self.completed += 1;
                }
                self.finish();
                // Run the next queued message (submitted while busy), if any.
                return self.drain_queue();
            }

            Msg::SpinnerTick => {
                self.spinner.tick();
                self.blink_tick = self.blink_tick.wrapping_add(1);
                if self.state == State::Streaming {
                    self.update_viewport_with_stream();
                    return Some(spinner_tick());
                }
            }

            Msg::ModalConfirm(idx) => {
                let approved = idx == 0;
                self.state = State::Streaming;
                if let Some((tool_id, name)) = self.pending_tool.take() {
                    let verdict = if approved { "allowed" } else { "denied" };
                    let color = if approved { Color::Yellow } else { Color::Red };
                    self.push_line(
                        &Style::new()
                            .fg(color)
                            .render(&format!("  [{verdict}] {name}")),
                    );
                    let session = self.session.clone();
                    return Some(cmd::batch(vec![
                        cmd::cmd(move || async move {
                            let _ = session.confirm_tool_use(&tool_id, approved, None).await;
                            Msg::Resume
                        }),
                        spinner_tick(),
                    ]));
                }
            }

            Msg::Resume => {
                if let Some(rx) = self.rx.clone() {
                    return Some(pump(rx));
                }
            }

            Msg::ShellOutput(text) => {
                let body = text.lines().take(40).collect::<Vec<_>>().join("\n");
                self.push_line(&gutter(Color::BrightBlack, body.trim_end()));
            }

            Msg::SideNote(text) => {
                self.push_line(&gutter(
                    Color::Magenta,
                    &format!("↘ by the way\n{}", text.trim()),
                ));
            }

            _ => {}
        }
        None
    }

    fn view(&self) -> String {
        let width = self.width as usize;
        let viewport_view = self.viewport.view();
        let separator = Style::new().fg(Color::BrightBlack).render(&format!(
            "{}{}",
            " ".repeat(PAD),
            "─".repeat(width.saturating_sub(2 * PAD))
        ));

        // Activity line directly above the input: spinner while the agent works,
        // an inline approval prompt while awaiting, empty when idle.
        let activity = match self.state {
            State::Streaming => {
                let spin = Style::new().fg(ACCENT).render(&self.spinner.view());
                let working = shimmer("Working…", self.blink_tick as usize);
                let mut tail = String::new();
                if let Some(t0) = self.stream_started {
                    // Live token estimate: finalized total + ~chars/4 for the
                    // in-flight reasoning + answer (snaps to exact usage on End).
                    let est = self.total_tokens
                        + self.streaming.raw_content().chars().count() / 4
                        + self.thinking.chars().count() / 4;
                    tail.push_str(&format!(" ({}", fmt_elapsed(t0.elapsed())));
                    if est > 0 {
                        tail.push_str(&format!(" · ↑ ~{} tokens", humanize(est)));
                    }
                    tail.push(')');
                }
                let tail = Style::new().fg(ACCENT).render(&tail);
                format!("  {spin} {working}{tail}")
            }
            State::Awaiting => {
                let tool = self
                    .pending_tool
                    .as_ref()
                    .map(|(_, n)| n.as_str())
                    .unwrap_or("tool");
                Style::new().fg(Color::Yellow).bold().render(&format!(
                    "  ⏵ Allow {tool}?   (y)es   (n)o   (a)lways   ·   Esc denies"
                ))
            }
            State::Idle => String::new(),
        };

        let prompt = Style::new().fg(ACCENT).bold().render("❯ ");
        let input_view = format!("{}{}{}", " ".repeat(PAD), prompt, self.textarea.view());

        // Bottom status bar (two lines): cwd/branch + model/tokens, then mode + hints.
        let dir = self.cwd.rsplit('/').next().unwrap_or(&self.cwd);
        let mut ctx = format!("  {dir}");
        if let Some(b) = &self.branch {
            ctx.push_str(&format!("  ⎇ {b}"));
        }
        // Live task bar: completed turns + currently running/queued (the input queue).
        let pending = self.queue.len();
        let active = usize::from(self.state == State::Streaming);
        if self.completed > 0 || pending + active > 0 {
            ctx.push_str(&format!("  ·  ✓ {} done", self.completed));
            if pending + active > 0 {
                ctx.push_str(&format!("  ⏳ {} running", pending + active));
            }
        }
        let mut info = String::new();
        if let Some(m) = &self.model {
            info.push_str(m); // provider/model
        }
        if self.context_limit > 0 {
            let pct = (self.last_prompt_tokens * 100 / self.context_limit as usize).min(100);
            info.push_str(&format!("  ·  ctx: {pct}%"));
        } else if self.total_tokens > 0 {
            info.push_str(&format!("  ·  {} tok", self.total_tokens));
        }
        info.push_str("  ");
        let status1 = status_line(&ctx, &info, width);
        let mode_part = Style::new().fg(self.mode.color()).bold().render(&format!(
            "  {} {}",
            self.mode.glyph(),
            self.mode.label()
        ));
        let hints = Style::new()
            .fg(Color::BrightBlack)
            .render("Shift+Tab mode · /help · ↑↓ history · Esc · Ctrl+C quit  ");
        let used = a3s_tui::style::visible_len(&mode_part) + a3s_tui::style::visible_len(&hints);
        let status2 = format!(
            "{mode_part}{}{hints}",
            " ".repeat(width.saturating_sub(used))
        );

        let spacer = String::new(); // gap between the transcript and the loading line
        let composed = Layout::vertical()
            .item(&viewport_view, Constraint::Fill)
            .item(&spacer, Constraint::Fixed(1))
            .item(&activity, Constraint::Fixed(1))
            .item(&separator, Constraint::Fixed(1))
            .item(&input_view, Constraint::Fixed(1))
            .item(&separator, Constraint::Fixed(1))
            .item(&status1, Constraint::Fixed(1))
            .item(&status2, Constraint::Fixed(1))
            .render(self.height);

        let composed = self.overlay_slash_menu(composed);
        self.overlay_model_menu(composed)
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        // Real cursor at the input insertion point whenever the input is live —
        // idle OR streaming (you can keep typing while the agent works). Hidden
        // only during an approval prompt.
        if self.state == State::Awaiting {
            return None;
        }
        let row = self.height.saturating_sub(4); // input line: …input, border, status×2
        let col = (PAD + 2) as u16 + self.textarea.cursor_display_col() as u16; // PAD + "❯ "
        Some((col, row))
    }
}

impl App {
    /// True when the `/` command menu should be shown (idle, single-line input
    /// starting with `/` that matches at least one command).
    fn slash_menu_open(&self) -> bool {
        let input = self.textarea.value();
        self.state == State::Idle
            && input.starts_with('/')
            && !input.contains('\n')
            && !slash_candidates(&input).is_empty()
    }

    /// Keys while the slash menu is open: ↑/↓ select, Enter run, Tab complete,
    /// Esc dismiss. Returns `Some(handled)` to consume the key.
    fn handle_slash_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let cands = slash_candidates(&self.textarea.value());
        if cands.is_empty() {
            return None;
        }
        let last = cands.len() - 1;
        self.slash_sel = self.slash_sel.min(last);
        match key.code {
            KeyCode::Up => {
                self.slash_sel = self.slash_sel.saturating_sub(1);
                Some(None)
            }
            KeyCode::Down => {
                self.slash_sel = (self.slash_sel + 1).min(last);
                Some(None)
            }
            KeyCode::Enter => {
                let cmd = cands[self.slash_sel].0;
                self.slash_sel = 0;
                self.textarea.clear();
                // /model opens its picker directly (stays open); others just run.
                if cmd == "/model" {
                    self.open_model_menu();
                    return Some(None);
                }
                Some(Some(cmd::msg(Msg::Submit(cmd.to_string()))))
            }
            KeyCode::Tab => {
                // Fill into the input (trailing space closes the menu) to add args.
                self.textarea
                    .set_value(&format!("{} ", cands[self.slash_sel].0));
                self.slash_sel = 0;
                Some(None)
            }
            KeyCode::Esc => {
                self.textarea.clear();
                self.slash_sel = 0;
                Some(None)
            }
            _ => None,
        }
    }

    /// Overlay the `/` command menu just above the input box.
    /// Overlay `menu` rows just above the input box (last row on the activity line).
    fn overlay_list(&self, composed: String, menu: &[String]) -> String {
        if menu.is_empty() {
            return composed;
        }
        let mut rows: Vec<String> = composed.lines().map(str::to_string).collect();
        let bottom = (self.height as usize).saturating_sub(6);
        let start = bottom.saturating_sub(menu.len().saturating_sub(1));
        for (i, ml) in menu.iter().enumerate() {
            let row = start + i;
            if row < rows.len() {
                rows[row] = ml.clone();
            }
        }
        rows.join("\n")
    }

    fn overlay_slash_menu(&self, composed: String) -> String {
        if !self.slash_menu_open() {
            return composed;
        }
        let cands = slash_candidates(&self.textarea.value());
        let sel = self.slash_sel.min(cands.len() - 1);
        let width = self.width as usize;
        let menu: Vec<String> = cands
            .iter()
            .enumerate()
            .map(|(i, (cmd, desc))| {
                let raw = pad_to(&format!("  {cmd:<9} {desc}"), width);
                if i == sel {
                    Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
                } else {
                    Style::new().fg(Color::BrightBlack).render(&raw)
                }
            })
            .collect();
        self.overlay_list(composed, &menu)
    }

    /// Open the /model picker on the current model (no-op if none configured).
    fn open_model_menu(&mut self) {
        if self.models.is_empty() {
            self.push_line(
                &Style::new()
                    .fg(Color::Red)
                    .render("  no models configured in config.acl"),
            );
            return;
        }
        let cur = self.model.as_deref();
        let idx = self
            .models
            .iter()
            .position(|m| Some(m.as_str()) == cur)
            .unwrap_or(0);
        self.model_menu = Some(idx);
    }

    /// Keys while the /model panel is open: ↑/↓ select, Enter switch, Esc close.
    fn handle_model_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let sel = self.model_menu?;
        let last = self.models.len().saturating_sub(1);
        match key.code {
            KeyCode::Up => {
                self.model_menu = Some(sel.saturating_sub(1));
                Some(None)
            }
            KeyCode::Down => {
                self.model_menu = Some((sel + 1).min(last));
                Some(None)
            }
            KeyCode::Enter => {
                let model = self.models[sel.min(last)].clone();
                self.model_menu = None;
                self.switch_model(&model);
                Some(None)
            }
            KeyCode::Esc => {
                self.model_menu = None;
                Some(None)
            }
            _ => None,
        }
    }

    /// Switch the active model by resuming the session under it (history kept).
    fn switch_model(&mut self, model: &str) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(Color::Yellow)
                    .render("  finish the current turn before switching models"),
            );
            return;
        }
        let opts = SessionOptions::new()
            .with_session_store(self.store.clone())
            .with_session_id(self.session_id.as_str())
            .with_confirmation_policy(self.confirmation.clone())
            .with_auto_save(true)
            .with_model(model);
        match self.agent.resume_session(self.session_id.as_str(), opts) {
            Ok(s) => {
                self.session = Arc::new(s);
                self.model = Some(model.to_string());
                self.context_limit = self.model_ctx.get(model).copied().unwrap_or(0);
                self.push_line(
                    &Style::new()
                        .fg(Color::Green)
                        .render(&format!("  ⇄ switched to {model}")),
                );
            }
            Err(e) => self.push_line(
                &Style::new()
                    .fg(Color::Red)
                    .render(&format!("  failed to switch model: {e}")),
            ),
        }
    }

    fn overlay_model_menu(&self, composed: String) -> String {
        let Some(sel) = self.model_menu else {
            return composed;
        };
        if self.models.is_empty() {
            return composed;
        }
        let width = self.width as usize;
        let mut menu = vec![pad_to(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  Select model — ↑/↓ · Enter · Esc"),
            width,
        )];
        for (i, m) in self.models.iter().enumerate().take(12) {
            let cur = Some(m.as_str()) == self.model.as_deref();
            let raw = pad_to(&format!("  {} {m}", if cur { "●" } else { " " }), width);
            menu.push(if i == sel.min(self.models.len() - 1) {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(Color::BrightBlack).render(&raw)
            });
        }
        self.overlay_list(composed, &menu)
    }

    fn on_submit(&mut self, text: String) -> Option<Cmd<Msg>> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        // `!cmd` runs a shell command directly (not through the agent).
        if let Some(rest) = trimmed.strip_prefix('!') {
            let cmd = rest.trim().to_string();
            if cmd.is_empty() {
                return None;
            }
            self.messages.push(gutter(
                ACCENT,
                &Style::new().bold().render(&format!("! {cmd}")),
            ));
            self.textarea.clear();
            self.rebuild_viewport();
            return Some(cmd::cmd(move || async move {
                let out = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .output()
                    .await;
                let text = match out {
                    Ok(o) => {
                        let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                        s.push_str(&String::from_utf8_lossy(&o.stderr));
                        if s.trim().is_empty() {
                            format!("(exit {})", o.status.code().unwrap_or(-1))
                        } else {
                            s
                        }
                    }
                    Err(e) => format!("failed to run: {e}"),
                };
                Msg::ShellOutput(text)
            }));
        }
        // `/btw <prompt>` runs a background side-thread (separate ephemeral
        // session, the main conversation as context) without disturbing the
        // current turn; its answer arrives as a side note.
        if let Some(rest) = trimmed.strip_prefix("/btw") {
            let q = rest.trim().to_string();
            self.textarea.clear();
            if q.is_empty() {
                self.push_line(
                    &Style::new()
                        .fg(Color::BrightBlack)
                        .render("  usage: /btw <question>"),
                );
                return None;
            }
            self.messages
                .push(gutter(Color::Magenta, &format!("↘ by the way: {q}")));
            self.rebuild_viewport();
            let agent = self.agent.clone();
            let workspace = self.cwd.clone();
            let history = self.session.history();
            return Some(cmd::cmd(move || async move {
                // Side-thread is a quick Q&A; auto-reject tool prompts (no UI).
                let conf = a3s_code_core::hitl::ConfirmationPolicy::enabled()
                    .with_timeout(500, TimeoutAction::Reject);
                let sess = match agent.session(
                    workspace,
                    Some(SessionOptions::new().with_confirmation_policy(conf)),
                ) {
                    Ok(s) => s,
                    Err(e) => return Msg::SideNote(format!("(/btw failed: {e})")),
                };
                let mut answer = String::new();
                if let Ok((mut rx, _join)) = sess.stream(&q, Some(&history)).await {
                    while let Some(ev) = rx.recv().await {
                        match ev {
                            AgentEvent::TextDelta { text } => answer.push_str(&text),
                            AgentEvent::End { text, .. } => {
                                if answer.trim().is_empty() {
                                    answer = text;
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Msg::SideNote(answer)
            }));
        }
        // Slash commands run inline in any state.
        match trimmed {
            "/exit" | "/quit" => return Some(cmd::quit()),
            "/clear" => {
                self.messages.clear();
                self.textarea.clear();
                self.rebuild_viewport();
                return None;
            }
            "/help" => {
                self.messages.push(gutter(
                    Color::BrightBlack,
                    "commands: /clear reset · /auto auto-approve · /exit quit\n\
                     Enter send · Shift+Tab cycle mode · ↑/↓ history · Esc interrupt · PgUp/PgDn scroll",
                ));
                self.textarea.clear();
                self.rebuild_viewport();
                return None;
            }
            "/auto" => {
                self.mode = Mode::Auto;
                self.textarea.clear();
                self.rebuild_viewport();
                return None;
            }
            "/config" => {
                self.textarea.clear();
                let line = match find_config() {
                    Some(path) if open_in_editor(&path) => {
                        format!("opened {path} in your editor")
                    }
                    Some(path) => {
                        format!("config: {path} (couldn't launch an editor — open it manually)")
                    }
                    None => "no config found — create ~/.a3s/config.acl".to_string(),
                };
                self.messages.push(gutter(Color::BrightBlack, &line));
                self.rebuild_viewport();
                return None;
            }
            "/model" => {
                self.textarea.clear();
                self.open_model_menu();
                return None;
            }
            _ => {}
        }

        self.history.push(trimmed.to_string());
        self.history_pos = None;
        // Show the user message with a blue dot gutter, then run now (if idle) or
        // queue it (if the agent is busy).
        self.messages.push(gutter(ACCENT, trimmed));
        self.textarea.clear();
        if self.state == State::Idle {
            self.start_stream(trimmed.to_string())
        } else {
            self.seq += 1;
            self.queue.push(Queued {
                prio: 1,
                seq: self.seq,
                text: trimmed.to_string(),
            });
            self.push_line(&Style::new().fg(Color::BrightBlack).render("    ⋯ queued"));
            None
        }
    }

    /// Begin streaming a prompt (the user message must already be on screen).
    fn start_stream(&mut self, prompt: String) -> Option<Cmd<Msg>> {
        self.streaming.clear();
        self.state = State::Streaming;
        self.stream_started = Some(Instant::now());
        self.spinner.start();
        self.rebuild_viewport();
        let session = self.session.clone();
        Some(cmd::batch(vec![
            cmd::cmd(move || async move {
                match session.stream(prompt.as_str(), None).await {
                    Ok((rx, _join)) => Msg::StreamStarted(Arc::new(Mutex::new(rx))),
                    Err(e) => Msg::StreamError(e.to_string()),
                }
            }),
            spinner_tick(),
        ]))
    }

    /// Pop the next queued message and start streaming it, if any.
    fn drain_queue(&mut self) -> Option<Cmd<Msg>> {
        let next = self.queue.pop()?;
        self.start_stream(next.text)
    }

    fn on_agent_event(&mut self, event: AgentEvent) -> Option<Cmd<Msg>> {
        match event {
            AgentEvent::TextDelta { text } => {
                self.streaming.push(&text);
                self.update_viewport_with_stream();
            }
            AgentEvent::ReasoningDelta { text } => {
                self.thinking.push_str(&text);
                self.update_viewport_with_stream();
            }
            AgentEvent::ToolStart { name, .. } => {
                // Finalize any assistant text; show the tool live with a blinking
                // dot. The final "• action / └ result" lands on ToolEnd.
                self.finalize_streaming();
                self.tool_args.clear();
                self.tool_output.clear();
                self.running_tool = Some(name);
            }
            AgentEvent::ToolInputDelta { delta } => {
                self.tool_args.push_str(&delta);
            }
            AgentEvent::ToolOutputDelta { delta, .. } => {
                self.tool_output.push_str(&delta);
                self.update_viewport_with_stream();
            }
            AgentEvent::ToolEnd {
                name,
                output,
                exit_code,
                metadata,
                ..
            } => {
                self.running_tool = None;
                let args: Option<serde_json::Value> = serde_json::from_str(&self.tool_args).ok();
                self.push_line(&render_tool_end(
                    &name,
                    exit_code,
                    &output,
                    metadata.as_ref(),
                    args.as_ref(),
                    self.width as usize,
                ));
                self.tool_args.clear();
                self.tool_output.clear();
            }
            AgentEvent::SubagentStart {
                agent, description, ..
            } => {
                self.finalize_streaming();
                self.push_line(
                    &Style::new()
                        .fg(Color::Magenta)
                        .render(&format!("  ↳ subagent {agent}: {description}")),
                );
            }
            AgentEvent::SubagentEnd { agent, success, .. } => {
                let mark = if success { "✓" } else { "✗" };
                self.push_line(
                    &Style::new()
                        .fg(Color::Magenta)
                        .render(&format!("  ↳ {mark} subagent {agent} done")),
                );
            }
            AgentEvent::ConfirmationRequired {
                tool_id,
                tool_name,
                args,
                ..
            } => {
                if self.mode.auto_approves(&tool_name) {
                    self.push_line(
                        &Style::new()
                            .fg(Color::BrightBlack)
                            .render(&format!("  ⚡ auto-approved {tool_name}")),
                    );
                    let session = self.session.clone();
                    return Some(cmd::batch(vec![
                        cmd::cmd(move || async move {
                            let _ = session.confirm_tool_use(&tool_id, true, None).await;
                            Msg::Resume
                        }),
                        spinner_tick(),
                    ]));
                }
                self.state = State::Awaiting;
                self.pending_tool = Some((tool_id, tool_name.clone()));
                // Show what's being approved inline in the transcript; the
                // compact y/n/a prompt lives on the activity line above input.
                let head = match arg_summary(&args) {
                    Some(summary) => format!("  ⏵ requests: {tool_name} {summary}"),
                    None => format!("  ⏵ requests: {tool_name}"),
                };
                self.push_line(&Style::new().fg(Color::Yellow).bold().render(&head));
                self.rebuild_viewport();
                self.viewport.update(ViewportMsg::Bottom);
                return None; // wait for the user; do not pump
            }
            AgentEvent::End {
                text, usage, meta, ..
            } => {
                if self.streaming.raw_content().trim().is_empty() && !text.is_empty() {
                    self.streaming.push(&text);
                }
                self.finalize_streaming();
                self.total_tokens += usage.total_tokens;
                // Latest prompt size = how full the context window is (for ctx%).
                if usage.prompt_tokens > 0 {
                    self.last_prompt_tokens = usage.prompt_tokens;
                }
                if self.model.is_none() {
                    self.model = meta.and_then(|m| m.response_model.or(m.request_model));
                }
                self.finish();
                return None;
            }
            AgentEvent::Error { message } => {
                self.push_line(
                    &Style::new()
                        .fg(Color::Red)
                        .render(&format!("  error: {message}")),
                );
                self.finish();
                return None;
            }
            // TurnStart/TurnEnd, ToolInputDelta, planning, memory, subagent,
            // confirmation echoes, etc. — not surfaced in this MVP.
            _ => {}
        }
        // Keep draining the stream.
        self.rx.clone().map(pump)
    }

    fn finalize_streaming(&mut self) {
        let rendered = self.streaming.view();
        if !rendered.trim().is_empty() {
            self.messages.push(gutter(Color::Green, &rendered));
        }
        self.streaming.clear();
        self.thinking.clear();
        self.rebuild_viewport();
    }

    fn finish(&mut self) {
        self.state = State::Idle;
        self.stream_started = None;
        self.spinner.stop();
        self.rx = None;
        self.rebuild_viewport();
    }

    fn push_line(&mut self, line: &str) {
        self.messages.push(line.to_string());
        self.rebuild_viewport();
    }

    /// Move through prompt history and load the entry into the input. Going
    /// forward past the newest entry returns to a fresh, empty input.
    fn history_recall(&mut self, up: bool) {
        let pos = match (self.history_pos, up) {
            (None, true) => self.history.len().saturating_sub(1),
            (None, false) => return,
            (Some(i), true) => i.saturating_sub(1),
            (Some(i), false) => i + 1,
        };
        if pos >= self.history.len() {
            self.history_pos = None;
            self.textarea.clear();
        } else {
            self.history_pos = Some(pos);
            self.textarea.set_value(&self.history[pos]);
        }
    }

    fn update_viewport_with_stream(&mut self) {
        let mut blocks: Vec<String> = self.messages.clone();
        if !self.thinking.trim().is_empty() {
            blocks.push(
                Style::new()
                    .fg(Color::BrightBlack)
                    .italic()
                    .render(&format!("💭 {}", self.thinking.trim())),
            );
        }
        let rendered = self.streaming.view();
        if !rendered.is_empty() {
            blocks.push(gutter(Color::Green, &rendered));
        }
        // Currently-executing tool: "● action…" with a blinking dot.
        if let Some(name) = &self.running_tool {
            let args: Option<serde_json::Value> = serde_json::from_str(&self.tool_args).ok();
            let action = tool_action_summary(name, args.as_ref());
            let on = self.blink_tick % 8 < 4; // ~320ms on / 320ms off
            let dot = Style::new()
                .fg(if on {
                    Color::Yellow
                } else {
                    Color::BrightBlack
                })
                .bold()
                .render("●");
            blocks.push(format!("{}{} {}…", " ".repeat(PAD), dot, action));
        }
        // Live stdout of the running tool — show the tail like a terminal.
        if !self.tool_output.trim().is_empty() {
            let tail: Vec<&str> = self.tool_output.lines().rev().take(12).collect();
            let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
            blocks.push(Style::new().fg(Color::BrightBlack).render(&tail));
        }
        self.viewport.set_content(&blocks.join("\n\n"));
    }

    /// First-run welcome: ASCII-art logo, version, model, and tips.
    fn banner(&self) -> String {
        let art = r#" █████╗ ██████╗ ███████╗     ██████╗ ██████╗ ██████╗ ███████╗
██╔══██╗╚════██╗██╔════╝    ██╔════╝██╔═══██╗██╔══██╗██╔════╝
███████║ █████╔╝███████╗    ██║     ██║   ██║██║  ██║█████╗
██╔══██║ ╚═══██╗╚════██║    ██║     ██║   ██║██║  ██║██╔══╝
██║  ██║██████╔╝███████║    ╚██████╗╚██████╔╝██████╔╝███████╗
╚═╝  ╚═╝╚═════╝ ╚══════╝     ╚═════╝ ╚═════╝ ╚═════╝ ╚══════╝"#;
        let margin = " ".repeat(PAD);
        let art = art
            .lines()
            .map(|l| format!("{margin}{l}"))
            .collect::<Vec<_>>()
            .join("\n");
        let logo = Style::new().fg(ACCENT).bold().render(&art);
        let model = self.model.as_deref().unwrap_or("no model configured");
        let meta = Style::new().fg(Color::BrightBlack).render(&format!(
            "{margin}a3s-code v{}  ·  {model}  ·  {}",
            env!("CARGO_PKG_VERSION"),
            self.cwd
        ));
        let tips = Style::new()
            .fg(Color::BrightBlack)
            .italic()
            .render(&format!(
            "{margin}Type a message · / for commands · Shift+Tab cycles mode · Ctrl+C twice to exit"
        ));
        format!("\n{logo}\n\n{meta}\n{tips}\n")
    }

    fn rebuild_viewport(&mut self) {
        let full = self.messages.join("\n\n");
        self.viewport.set_content(&format!("\n{full}\n")); // top padding
    }

    /// Inline tool-approval keys (Codex-style): y/Enter allow, n/Esc deny,
    /// a = allow + enable auto-approve for the rest of the session.
    fn handle_approval_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => Some(cmd::msg(Msg::ModalConfirm(0))),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(cmd::msg(Msg::ModalConfirm(1))),
            KeyCode::Char('a' | 'A') => {
                self.mode = Mode::Auto;
                Some(cmd::msg(Msg::ModalConfirm(0)))
            }
            _ => None,
        }
    }
}

/// Headless probe of the same `session.stream()` / `AgentEvent` path the TUI
/// uses, auto-approving tool calls. Drives the integration without a TTY.
async fn run_smoke(session: Arc<AgentSession>) -> anyhow::Result<()> {
    let prompt = std::env::var("A3S_CODE_TUI_PROMPT")
        .unwrap_or_else(|_| "Reply with exactly one short sentence: what is 2 + 2?".to_string());
    eprintln!("[smoke] prompt: {prompt}");
    let (mut rx, join) = session.stream(prompt.as_str(), None).await?;
    while let Some(event) = rx.recv().await {
        match event {
            AgentEvent::TextDelta { text } => print!("{text}"),
            AgentEvent::ToolStart { name, .. } => eprintln!("\n[tool start] {name}"),
            AgentEvent::ToolEnd {
                name,
                exit_code,
                output,
                ..
            } => eprintln!(
                "[tool end] {name} (exit {exit_code}): {}",
                output.lines().take(2).collect::<Vec<_>>().join(" | ")
            ),
            AgentEvent::ConfirmationRequired {
                tool_id, tool_name, ..
            } => {
                eprintln!("[confirm] auto-allowing {tool_name}");
                let _ = session.confirm_tool_use(&tool_id, true, None).await;
            }
            AgentEvent::End { .. } => eprintln!("\n[end]"),
            AgentEvent::Error { message } => eprintln!("\n[error] {message}"),
            _ => {}
        }
    }
    // Let the stream task finish (incl. auto-save/persist) before we exit.
    let _ = join.await;
    Ok(())
}

/// Render a completed tool call. File-editing tools (`write`/`edit`) carry
/// `before`/`after`/`file_path` in their metadata — show those as a colored
/// diff; everything else shows a status line + a few lines of output.
fn render_tool_end(
    name: &str,
    exit_code: i32,
    output: &str,
    meta: Option<&serde_json::Value>,
    args: Option<&serde_json::Value>,
    width: usize,
) -> String {
    if let Some(meta) = meta {
        if let (Some(before), Some(after), Some(path)) = (
            meta.get("before").and_then(|v| v.as_str()),
            meta.get("after").and_then(|v| v.as_str()),
            meta.get("file_path").and_then(|v| v.as_str()),
        ) {
            return render_diff(path, before, after);
        }
    }
    let ok = exit_code == 0;
    let margin = " ".repeat(PAD);
    // Header: "• Ran <command>" / "• Read <path>" — bullet colored by outcome.
    let bullet = Style::new()
        .fg(if ok { Color::Green } else { Color::Red })
        .bold()
        .render("•");
    let action = tool_action_summary(name, args);
    let header = format!("{margin}{bullet} {}", Style::new().render(&action));

    // For a successful file read on a known language, show the highlighted file
    // content under the action (keeps the nice read preview).
    if ok {
        if let Some(lang) = args
            .and_then(|a| {
                a.get("file_path")
                    .or_else(|| a.get("path"))
                    .and_then(|v| v.as_str())
            })
            .and_then(lang_from_path)
        {
            let head = output.lines().take(8).collect::<Vec<_>>().join("\n");
            if !head.trim().is_empty() {
                let fenced = format!("```{lang}\n{head}\n```");
                let rendered = a3s_tui::markdown::Markdown::new()
                    .with_width(width.saturating_sub(PAD + 4).max(20))
                    .render(&fenced);
                return format!("{header}\n{rendered}");
            }
        }
    }

    // Otherwise a "└ <first line>" summary with a "… +N lines" overflow marker.
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return header;
    }
    let first_color = if ok { Color::BrightBlack } else { Color::Red };
    let connector = Style::new().fg(Color::BrightBlack).render("└");
    let mut out = format!(
        "{header}\n{margin}  {connector} {}",
        Style::new().fg(first_color).render(lines[0])
    );
    if lines.len() > 1 {
        let more = lines.len() - 1;
        out.push_str(&format!(
            "\n{margin}    {}",
            Style::new()
                .fg(Color::BrightBlack)
                .render(&format!("… +{more} lines"))
        ));
    }
    out
}

/// Verb + target for a tool call, e.g. "Ran npm test", "Read src/main.rs".
fn tool_action_summary(name: &str, args: Option<&serde_json::Value>) -> String {
    let target = args.and_then(arg_summary).unwrap_or_default();
    match name {
        "bash" | "shell" | "run" | "exec" => format!("Ran {target}"),
        "read" | "cat" => format!("Read {target}"),
        "write" | "create" => format!("Wrote {target}"),
        "edit" | "patch" | "apply_patch" => format!("Edited {target}"),
        "grep" | "search" => format!("Searched {target}"),
        "ls" | "glob" | "find" => format!("Listed {target}"),
        "web_search" => format!("Searched the web: {target}"),
        "web_fetch" => format!("Fetched {target}"),
        _ if target.is_empty() => name.to_string(),
        _ => format!("{name} {target}"),
    }
}

/// Map a file path to a syntect language token for fenced rendering.
fn lang_from_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    Some(match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" => "markdown",
        "sh" | "bash" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "java" => "java",
        "rb" => "ruby",
        "html" => "html",
        "css" => "css",
        "sql" => "sql",
        _ => return None,
    })
}

/// Extract a one-line summary of a tool's primary argument.
fn arg_summary(args: &serde_json::Value) -> Option<String> {
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "old_string",
    ] {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            let v = v.replace('\n', " ");
            return Some(truncate(v.trim(), 120));
        }
    }
    None
}

/// Render a unified-ish line diff (changed lines only) with +/- coloring.
fn render_diff(path: &str, before: &str, after: &str) -> String {
    use similar::{ChangeTag, TextDiff};
    const MAX_LINES: usize = 80;

    let diff = TextDiff::from_lines(before, after);
    let mut lines: Vec<String> = Vec::new();
    let (mut adds, mut dels) = (0usize, 0usize);
    for change in diff.iter_all_changes() {
        let raw = change.value();
        let raw = raw.strip_suffix('\n').unwrap_or(raw);
        match change.tag() {
            ChangeTag::Delete => {
                dels += 1;
                if lines.len() < MAX_LINES {
                    lines.push(Style::new().fg(Color::Red).render(&format!("  - {raw}")));
                }
            }
            ChangeTag::Insert => {
                adds += 1;
                if lines.len() < MAX_LINES {
                    lines.push(Style::new().fg(Color::Green).render(&format!("  + {raw}")));
                }
            }
            ChangeTag::Equal => {}
        }
    }
    if lines.len() >= MAX_LINES {
        lines.push(
            Style::new()
                .fg(Color::BrightBlack)
                .render("  … (diff truncated)"),
        );
    }
    let mut out = Style::new()
        .fg(Color::Cyan)
        .render(&format!("  ✎ {path}  (+{adds} -{dels})"));
    if !lines.is_empty() {
        out.push('\n');
        out.push_str(&lines.join("\n"));
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// Find the A3S config: `$A3S_CONFIG_FILE`, then `.a3s/config.acl` walking up
/// from the current directory (project-local), then `~/.a3s/config.acl`
/// (user-global) — so `a3s code` works from anywhere once a global config exists.
fn find_config() -> Option<String> {
    if let Ok(p) = std::env::var("A3S_CONFIG_FILE") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut dir: Option<&std::path::Path> = Some(cwd.as_path());
        while let Some(d) = dir {
            let candidate = d.join(".a3s/config.acl");
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
            dir = d.parent();
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = std::path::Path::new(&home).join(".a3s/config.acl");
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

pub async fn run(args: Vec<String>) -> anyhow::Result<()> {
    // `a3s code resume <id>` resumes a specific session; otherwise the default.
    let session_id = if args.first().map(String::as_str) == Some("resume") {
        args.get(1)
            .cloned()
            .unwrap_or_else(|| SESSION_ID.to_string())
    } else {
        SESSION_ID.to_string()
    };
    let config_path = find_config().ok_or_else(|| {
        anyhow::anyhow!(
            "no A3S config found.\n\nLooked for: $A3S_CONFIG_FILE, .a3s/config.acl in this \
             directory or a parent, and ~/.a3s/config.acl.\n\nCreate one, e.g.:\n  \
             mkdir -p ~/.a3s\n  $EDITOR ~/.a3s/config.acl\n\nMinimal example:\n  \
             default_model = \"openai/gpt-4o\"\n  providers \"openai\" {{\n    apiKey = \
             \"sk-...\"\n  }}\n\nOr point at an existing file: A3S_CONFIG_FILE=/path/config.acl a3s code"
        )
    })?;
    let agent = Arc::new(
        Agent::new(config_path.clone())
            .await
            .map_err(|e| anyhow::anyhow!("failed to load agent from {config_path}: {e}"))?,
    );
    let workspace = std::env::current_dir()?.to_string_lossy().to_string();

    // Configured "provider/model" ids (+ context windows) + the default model.
    let mut models: Vec<String> = Vec::new();
    let mut model_ctx: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut default_model: Option<String> = None;
    if let Ok(cfg) =
        a3s_code_core::config::CodeConfig::from_file(std::path::Path::new(&config_path))
    {
        for (p, m) in cfg.list_models() {
            let id = format!("{}/{}", p.name, m.id);
            model_ctx.insert(id.clone(), m.limit.context);
            models.push(id);
        }
        default_model = cfg.default_model.clone();
    }
    let context_limit = default_model
        .as_ref()
        .and_then(|m| model_ctx.get(m))
        .copied()
        .unwrap_or(0);

    // Persistent, resumable session: stored under <cwd>/.a3s/tui-sessions and
    // keyed by a fixed id, so relaunching in the same directory continues the
    // conversation. Falls back to a fresh session when none exists yet.
    let store_dir = std::path::Path::new(&workspace).join(".a3s/tui-sessions");
    let store: Arc<dyn a3s_code_core::store::SessionStore> = Arc::new(
        a3s_code_core::store::FileSessionStore::new(&store_dir)
            .await
            .map_err(|e| anyhow::anyhow!("failed to open session store {store_dir:?}: {e}"))?,
    );
    // Enable HITL confirmation so file-modifying tools (write/edit/patch) can
    // run — they require a confirmation manager, otherwise they fail with
    // "requires confirmation but no HITL confirmation manager is configured".
    // The TUI is that manager (approve/deny modal, or /auto). Long timeout so
    // the modal never expires while the user reads it.
    let confirmation = a3s_code_core::hitl::ConfirmationPolicy::enabled()
        .with_timeout(3_600_000, TimeoutAction::Reject);
    let session = match agent.resume_session(
        session_id.as_str(),
        SessionOptions::new()
            .with_session_store(store.clone())
            .with_confirmation_policy(confirmation.clone())
            .with_auto_save(true),
    ) {
        Ok(s) => s,
        Err(_) => agent.session(
            workspace.clone(),
            Some(
                SessionOptions::new()
                    .with_session_store(store.clone())
                    .with_session_id(session_id.as_str())
                    .with_confirmation_policy(confirmation.clone())
                    .with_auto_save(true),
            ),
        )?,
    };

    let (width, height) = a3s_tui::terminal::Terminal::size().unwrap_or((80, 24));

    // Seed the transcript with any resumed conversation (user + assistant text).
    let initial_messages: Vec<String> = session
        .history()
        .iter()
        .filter_map(|m| {
            let text = m.text();
            if text.trim().is_empty() {
                return None;
            }
            match m.role.as_str() {
                // Same gutter (● dot + indent) as live messages.
                "user" => Some(gutter(ACCENT, text.trim())),
                "assistant" => {
                    let mut md = StreamingMarkdown::new((width as usize).saturating_sub(PAD + 2));
                    md.push(&text);
                    Some(gutter(Color::Green, &md.view()))
                }
                _ => None,
            }
        })
        .collect();

    let session = Arc::new(session);

    // Headless smoke mode: exercise the agent-stream integration (the hard part
    // the TUI depends on) without taking over the terminal. Useful for CI/probes
    // and for validating a model/config end-to-end.
    if std::env::var_os("A3S_CODE_TUI_SMOKE").is_some() {
        return run_smoke(session).await;
    }

    let keymap = Keymap::new()
        .bind(
            KeyBinding::new(KeyCode::PageUp),
            Action::ScrollUp,
            "Scroll up",
        )
        .bind(
            KeyBinding::new(KeyCode::PageDown),
            Action::ScrollDown,
            "Scroll down",
        )
        .bind(
            KeyBinding::ctrl(KeyCode::Home),
            Action::ScrollTop,
            "Scroll to top",
        )
        .bind(
            KeyBinding::ctrl(KeyCode::End),
            Action::ScrollBottom,
            "Scroll to bottom",
        );

    let app = App {
        session,
        agent: agent.clone(),
        store: store.clone(),
        confirmation,
        session_id: session_id.clone(),
        models,
        model_ctx,
        context_limit,
        last_prompt_tokens: 0,
        model_menu: None,
        quit_armed: None,
        viewport: Viewport::new(width, height.saturating_sub(7)),
        textarea: Textarea::new()
            .with_height(1)
            .with_width(width.saturating_sub((PAD + 2) as u16)) // PAD margin + "❯ "
            .with_submit_on_enter(true),
        spinner: Spinner::new().with_title(""),
        streaming: StreamingMarkdown::new((width as usize).saturating_sub(PAD + 2)),
        thinking: String::new(),
        state: State::Idle,
        messages: initial_messages,
        rx: None,
        pending_tool: None,
        history: Vec::new(),
        history_pos: None,
        model: default_model,
        total_tokens: 0,
        tool_args: String::new(),
        tool_output: String::new(),
        stream_started: None,
        running_tool: None,
        blink_tick: 0,
        mode: Mode::Default,
        queue: BinaryHeap::new(),
        seq: 0,
        completed: 0,
        branch: git_branch(&workspace),
        slash_sel: 0,
        cwd: workspace.clone(),
        width,
        height,
        keymap,
    };

    ProgramBuilder::new(app)
        .with_alt_screen()
        .with_mouse_support()
        .with_fps(30)
        .run()
        .await?;

    // Session is auto-saved under this directory; show how to come back.
    println!("\n  session saved · resume it with:  a3s code resume {session_id}\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_metadata_renders_colored_diff() {
        let meta = serde_json::json!({
            "file_path": "src/x.rs",
            "before": "let a = 1;\nkeep;\n",
            "after": "let a = 2;\nkeep;\n",
        });
        let out = render_tool_end("edit", 0, "ok", Some(&meta), None, 80);
        assert!(out.contains("src/x.rs"), "header has path");
        assert!(out.contains("+1") && out.contains("-1"), "add/del counts");
        assert!(out.contains("let a = 2;"), "shows inserted line");
        assert!(out.contains("let a = 1;"), "shows deleted line");
        assert!(!out.contains("keep;"), "unchanged lines are omitted");
    }

    #[test]
    fn non_edit_tool_renders_status_line() {
        let out = render_tool_end("bash", 0, "hello\nworld", None, None, 80);
        assert!(out.contains("bash") && out.contains("hello"));
        assert!(!out.contains('✎'), "no diff marker for non-edit tools");
    }

    #[test]
    fn tool_end_shows_primary_arg_summary() {
        let args = serde_json::json!({ "command": "npm test", "timeout": 60 });
        let out = render_tool_end("bash", 0, "ok\n", None, Some(&args), 80);
        assert!(out.contains("bash"));
        assert!(out.contains("npm test"), "shows the command argument");
    }

    #[test]
    fn arg_summary_extracts_known_keys() {
        assert_eq!(
            arg_summary(&serde_json::json!({ "command": "ls -la" })),
            Some("ls -la".to_string())
        );
        assert_eq!(
            arg_summary(&serde_json::json!({ "pattern": "TODO" })),
            Some("TODO".to_string())
        );
        assert_eq!(arg_summary(&serde_json::json!({ "unknown": "x" })), None);
    }
}
