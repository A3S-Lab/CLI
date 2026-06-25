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
use a3s_code_core::{Agent, AgentEvent, AgentSession, SessionOptions, SystemPromptSlots};
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

mod config;
mod gitutil;
mod image;
mod panels;
mod render;
mod skills;
mod syntax;
mod update;
mod util;
use config::*;
use gitutil::*;
use image::*;
use render::*;
use skills::*;
use syntax::*;
use update::*;
use util::*;

/// Theme accent — ShuAn OS blue. Single source of truth for the UI accent color.
// Tokyo Night palette — muted, cohesive accents used across the whole UI.
const ACCENT: Color = Color::Rgb(122, 162, 247); // soft blue (primary)
const TN_GREEN: Color = Color::Rgb(158, 206, 106);
const TN_YELLOW: Color = Color::Rgb(224, 175, 104);
const TN_RED: Color = Color::Rgb(247, 118, 142);
const TN_CYAN: Color = Color::Rgb(125, 207, 255);
const TN_ORANGE: Color = Color::Rgb(255, 158, 100);
const TN_FG: Color = Color::Rgb(192, 202, 245); // body text

/// Built-in slash commands shown in the `/` menu.
const SLASH_COMMANDS: &[(&str, &str)] = &[
    (
        "/model",
        "switch model (←/→ for Claude/GPT accounts if signed in)",
    ),
    ("/init", "analyze the project and generate AGENTS.md"),
    ("/config", "edit .a3s/config.acl in your editor"),
    ("/theme", "cycle the code-highlight theme (Atom One Dark …)"),
    ("/plugin", "enable/disable Claude skills & plugins"),
    ("/reload", "re-scan skills/plugins (hot-reload the / menu)"),
    ("/update", "upgrade a3s to the latest release"),
    ("/btw", "ask a background side-question (/btw <prompt>)"),
    ("/top", "live process monitor (highlights coding agents)"),
    ("/ide", "file tree + code viewer for the workspace"),
    ("/git", "git status / diff / stage / commit (gitui-style)"),
    ("/effort", "adjust model effort (low … max)"),
    ("/compact", "summarize + compact the conversation context"),
    ("/goal", "set a north-star goal the agent keeps in mind"),
    (
        "/loop",
        "run a task, auto-continuing until done (Esc stops)",
    ),
    ("/relay", "continue an unfinished task from another agent"),
    ("/help", "show commands and shortcuts"),
    ("/clear", "reset the conversation"),
    ("/auto", "switch to auto-approve mode"),
    ("/exit", "quit a3s code"),
];

/// Slash commands that mutate the session / conversation and so must NOT run
/// mid-stream — hidden from the menu and rejected while a turn is in flight.
const IDLE_ONLY: &[&str] = &[
    "/clear", "/compact", "/model", "/effort", "/goal", "/loop", "/relay", "/update", "/init",
];

/// Workspace files for the `@` picker (git-tracked, gitignore-respected).
fn workspace_files(dir: &str) -> Vec<String> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Slash commands whose name starts with `input` (input begins with `/`).
fn slash_candidates(input: &str) -> Vec<(&'static str, &'static str)> {
    SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(input))
        .copied()
        .collect()
}

/// One row of the `/top` process panel.
struct ProcRow {
    pid: String,
    cpu: f32,
    mem: f32,
    cmd: String,
    agent: Option<&'static str>,
}

/// Detect a coding-agent process from its command line.
fn detect_agent(cmd: &str) -> Option<&'static str> {
    let l = cmd.to_lowercase();
    if l.contains("a3s-code")
        || l.contains("a3s code")
        || l.contains("/a3s ")
        || l.ends_with("/a3s")
    {
        Some("a3s-code")
    } else if l.contains("claude") {
        Some("claude code")
    } else if l.contains("codex") {
        Some("codex")
    } else if l.contains("cursor-agent") {
        Some("cursor")
    } else if l.contains("gemini") {
        Some("gemini")
    } else {
        None
    }
}

/// Glyph + colour for a plan task's status.
fn task_status_style(status: a3s_code_core::planning::TaskStatus) -> (char, Color) {
    use a3s_code_core::planning::TaskStatus;
    match status {
        TaskStatus::Completed => ('✔', Color::Green),
        TaskStatus::InProgress => ('▶', Color::Yellow),
        TaskStatus::Failed => ('✗', Color::Red),
        TaskStatus::Skipped | TaskStatus::Cancelled => ('⊘', Color::BrightBlack),
        _ => ('□', Color::BrightBlack), // Pending
    }
}

/// Brand/theme colour for a coding agent, used to tag its rows and tabs.
fn agent_color(agent: &str) -> Color {
    match agent {
        "a3s-code" => ACCENT,
        "claude code" => Color::Rgb(217, 119, 87), // Claude clay
        "codex" => Color::Rgb(16, 163, 127),       // OpenAI green
        "cursor" => Color::Rgb(180, 182, 200),
        "gemini" => Color::Rgb(124, 137, 245),
        _ => Color::BrightBlack,
    }
}

/// Snapshot the process table via `ps`, sorted by CPU, agents first.
async fn fetch_top() -> Vec<ProcRow> {
    let out = tokio::process::Command::new("ps")
        .args(["-axo", "pid=,pcpu=,pmem=,args="])
        .output()
        .await;
    let Ok(out) = out else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows: Vec<ProcRow> = text
        .lines()
        .filter_map(|line| {
            // ps right-aligns columns with runs of spaces, so collapse them.
            let mut it = line.split_whitespace();
            let pid = it.next()?.to_string();
            let cpu: f32 = it.next()?.parse().ok()?;
            let mem: f32 = it.next()?.parse().ok()?;
            let cmd = it.collect::<Vec<_>>().join(" ");
            if cmd.is_empty() {
                return None;
            }
            let agent = detect_agent(&cmd);
            Some(ProcRow {
                pid,
                cpu,
                mem,
                cmd,
                agent,
            })
        })
        .collect();
    // Agents first, then by CPU descending.
    rows.sort_by(|a, b| {
        b.agent.is_some().cmp(&a.agent.is_some()).then(
            b.cpu
                .partial_cmp(&a.cpu)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    rows.truncate(200);
    rows
}

/// One visible row of the `/ide` file tree (a flattened, expandable tree).
struct IdeEntry {
    path: std::path::PathBuf,
    name: String,
    depth: usize,
    is_dir: bool,
    expanded: bool,
}

/// An open, editable file in the `/ide` panel.
struct IdeFile {
    path: std::path::PathBuf,
    lines: Vec<String>, // text rows, or pre-rendered half-block rows if `image`
    scroll: usize,
    row: usize, // cursor line
    col: usize, // cursor column (char index)
    dirty: bool,
    image: bool, // read-only image preview
}

/// State of the `/ide` panel: the file tree, selection, and the open file.
struct Ide {
    entries: Vec<IdeEntry>,
    sel: usize,
    tree_scroll: usize,
    file: Option<IdeFile>,
    focus_editor: bool,
}

/// Directory children for the tree, dirs first then files, noise skipped.
fn ide_children(dir: &std::path::Path, depth: usize) -> Vec<IdeEntry> {
    let mut v: Vec<IdeEntry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if matches!(
                name.as_str(),
                ".git" | "node_modules" | "target" | ".DS_Store" | ".next" | "dist"
            ) {
                return None;
            }
            let is_dir = e.path().is_dir();
            Some(IdeEntry {
                path: e.path(),
                name,
                depth,
                is_dir,
                expanded: false,
            })
        })
        .collect();
    v.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    v
}

/// Project instructions for the agent's system prompt. a3s-code already
/// auto-loads `AGENTS.md`; this adds Claude Code's `CLAUDE.md` (preferred), so
/// existing projects work unchanged. Returns the content wrapped with a header.
fn project_instructions(workspace: &str) -> Option<String> {
    for name in ["CLAUDE.md", "AGENT.md"] {
        let p = std::path::Path::new(workspace).join(name);
        if let Ok(c) = std::fs::read_to_string(&p) {
            if !c.trim().is_empty() {
                return Some(format!("# Project Instructions ({name})\n\n{c}"));
            }
        }
    }
    None
}

/// Left margin for the whole UI (inner padding).
const PAD: usize = 2;

/// Model effort levels (label, thinking-token budget) — `/effort` slider. The
/// last, `ultracode`, additionally plans a dynamic workflow and dispatches work
/// to parallel subagents (a3s-code PTC).
const EFFORT_LEVELS: &[(&str, usize)] = &[
    ("low", 1024),
    ("medium", 4096),
    ("high", 8192),
    ("xhigh", 16384),
    ("max", 32768),
    ("ultracode", 32768),
];
/// Index of the `ultracode` level (special: planning + parallel subagents).
const ULTRACODE: usize = 5;

/// A resumable/relayable session from this or another coding agent.
struct RelaySession {
    agent: &'static str,
    /// Native a3s-code session id (resume in place), if ours.
    native_id: Option<String>,
    /// Extracted last task, to continue here (foreign agents).
    seed: Option<String>,
    label: String,
    mtime: std::time::SystemTime,
}

/// Last user message in a Claude Code / Codex `.jsonl` transcript.
/// Extract a user message's text from one transcript line, across formats —
/// Claude `{message:{role,content}}` / `{role,content}` and Codex
/// `{payload:{role,content}}` with `input_text` parts. None if not a user line.
fn parse_user_line(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let role = v
        .get("message")
        .and_then(|m| m.get("role"))
        .or_else(|| v.get("payload").and_then(|p| p.get("role")))
        .or_else(|| v.get("role"))
        .and_then(|r| r.as_str());
    if role != Some("user") {
        return None;
    }
    let content = v
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| v.get("payload").and_then(|p| p.get("content")))
        .or_else(|| v.get("content"))?;
    let txt = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let txt = txt.trim();
    if txt.is_empty() || txt.starts_with('<') {
        return None;
    }
    Some(txt.to_string())
}

/// Most recent user message — read only the file tail (transcripts are big).
fn last_user_msg_jsonl(path: &std::path::Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(128 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // drop the partial first line
    }
    lines.iter().rev().find_map(|l| parse_user_line(l))
}

/// First user message — the initial task. Read the file head (cheap). Used as a
/// fallback for Codex, whose huge rollouts keep the prompt far from the tail.
fn first_user_msg_jsonl(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 96 * 1024];
    let n = f.read(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf[..n]);
    text.lines().find_map(parse_user_line)
}

/// Last user message from an a3s-code session JSON (for a task description).
fn last_user_msg_a3s(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    for m in v.get("messages")?.as_array()?.iter().rev() {
        if m.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        let txt = match m.get("content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(a)) => a
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(" "),
            _ => continue,
        };
        if !txt.trim().is_empty() {
            return Some(txt.trim().to_string());
        }
    }
    None
}

/// A readable session name from a transcript filename (Codex/Claude fallback).
fn jsonl_session_name(p: &std::path::Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| {
            let s = s.strip_prefix("rollout-").unwrap_or(s);
            s.chars().take(19).collect::<String>().replace('T', " ")
        })
        .unwrap_or_else(|| "session".into())
}

/// Scan a3s-code (native), Claude Code, and Codex session stores for this dir.
fn scan_relay(cwd: &str) -> Vec<RelaySession> {
    let mut out: Vec<RelaySession> = Vec::new();

    // The cwd plus its ancestors — so launching from a subdirectory still finds
    // the project root's sessions (Claude/Codex usually run at the root).
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    let mut p = std::path::Path::new(cwd);
    loop {
        dirs.push(p.to_path_buf());
        match p.parent() {
            Some(par) if par != p && dirs.len() < 6 => p = par,
            _ => break,
        }
    }

    // a3s-code: our own session store under cwd/ancestors (resume natively).
    for d in &dirs {
        if let Ok(entries) = std::fs::read_dir(d.join(".a3s/tui-sessions")) {
            for e in entries.flatten() {
                let f = e.path();
                if let Some(id) = f.is_file().then(|| f.file_stem()?.to_str()).flatten() {
                    let mtime = std::fs::metadata(&f)
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    // Show the last task as the description, like Claude/Codex.
                    let label = match last_user_msg_a3s(&f) {
                        Some(m) => format!("a3s-code · {}", truncate(&m, 56)),
                        None => format!("a3s-code · session {id}"),
                    };
                    out.push(RelaySession {
                        agent: "a3s-code",
                        native_id: Some(id.to_string()),
                        seed: None,
                        label,
                        mtime,
                    });
                }
            }
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        // Claude Code: ~/.claude/projects/<encoded path>/**.jsonl for cwd+ancestors.
        for d in &dirs {
            let encoded = format!(
                "-{}",
                d.to_string_lossy()
                    .trim_start_matches('/')
                    .replace('/', "-")
            );
            collect_jsonl(
                &home.join(".claude/projects").join(&encoded),
                "claude code",
                &mut out,
            );
        }
        // Codex stores all sessions under one tree.
        collect_jsonl(&home.join(".codex/sessions"), "codex", &mut out);
    }

    // Newest first, then keep only the most recent few per agent — users care
    // about recent sessions, not the whole history.
    out.sort_by_key(|e| std::cmp::Reverse(e.mtime));
    const PER_AGENT: usize = 8;
    let mut kept: std::collections::HashMap<&'static str, usize> = std::collections::HashMap::new();
    out.retain(|s| {
        let n = kept.entry(s.agent).or_insert(0);
        *n += 1;
        *n <= PER_AGENT
    });
    out
}

/// Recursively gather `.jsonl` paths (+ mtime) under `dir` — Claude nests them
/// one level (`<id>/…`), Codex several (`sessions/YYYY/MM/DD/…`).
fn gather_jsonl(
    dir: &std::path::Path,
    depth: usize,
    max: usize,
    out: &mut Vec<(std::path::PathBuf, std::time::SystemTime)>,
) {
    if depth > max {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            gather_jsonl(&p, depth + 1, max, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            let mtime = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            out.push((p, mtime));
        }
    }
}

/// Add relay sessions for the most recent transcripts under `dir`. Only the
/// newest dozen are read for a description (cheap), the rest are stat-only.
fn collect_jsonl(dir: &std::path::Path, agent: &'static str, out: &mut Vec<RelaySession>) {
    let mut paths: Vec<(std::path::PathBuf, std::time::SystemTime)> = Vec::new();
    gather_jsonl(dir, 0, 6, &mut paths);
    paths.sort_by_key(|e| std::cmp::Reverse(e.1)); // newest first
    paths.truncate(12);
    for (p, mtime) in paths {
        // Most-recent task (tail); fall back to the initial prompt (head).
        let desc = last_user_msg_jsonl(&p).or_else(|| first_user_msg_jsonl(&p));
        let label = match &desc {
            Some(m) => format!("{agent} · {}", truncate(m, 56)),
            None => format!("{agent} · {}", jsonl_session_name(&p)),
        };
        out.push(RelaySession {
            agent,
            native_id: None,
            seed: desc,
            label,
            mtime,
        });
    }
}

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

    fn glyph(self) -> &'static str {
        match self {
            Mode::Default => "⏵",
            Mode::Plan => "✎",
            Mode::Auto => "⏵⏵",
        }
    }

    /// Short one-word name for the status line ("auto mode on").
    fn name(self) -> &'static str {
        match self {
            Mode::Default => "default",
            Mode::Plan => "plan",
            Mode::Auto => "auto",
        }
    }

    fn color(self) -> Color {
        match self {
            Mode::Default => TN_FG,
            Mode::Plan => TN_CYAN,
            Mode::Auto => TN_GREEN,
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
    /// Advance the welcome-mascot animation frame.
    BannerTick,
    ModalConfirm(usize),
    Resume,
    Interrupted,
    /// Output of a `!`-prefixed shell command.
    ShellOutput(String),
    /// Answer from a `/btw` background side-thread.
    SideNote(String),
    /// Refreshed process snapshot for the `/top` panel.
    TopData(Vec<ProcRow>),
    /// Tick to re-fetch the `/top` snapshot.
    TopRefresh,
    /// Result of the async `/relay` session scan.
    RelayData(Vec<RelaySession>),
    /// `/git` status + recent log snapshot.
    GitStatus(Vec<GitFile>, Vec<String>),
    /// `/git` diff for the selected file.
    GitDiff(Vec<String>),
    /// Inactivity auto-review summary text.
    AutoReview(String),
    /// `/compact` produced this conversation summary; reseed a fresh session.
    Compacted(String),
    /// Startup update check completed with the latest published version (if any).
    UpdateCheck(Option<String>),
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

/// Drives the welcome-mascot animation while the banner is on screen.
fn banner_tick() -> Cmd<Msg> {
    cmd::tick(Duration::from_millis(280), Msg::BannerTick)
}

/// A running (or just-finished) parallel subagent task, for the bottom tracker.
struct SubAgent {
    task_id: String,
    agent: String,
    description: String,
    started: Instant,
    tokens: u64,
    done: bool,
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
    /// Active tab in the /model panel (0 = config; account tabs when signed in).
    model_tab: usize,
    /// Custom LLM client to inject (Codex account); None uses config.acl creds.
    llm_override: Option<Arc<dyn a3s_code_core::llm::LlmClient>>,
    /// Current model effort (index into EFFORT_LEVELS).
    effort: usize,
    /// `/effort` slider panel: temp selection while open.
    effort_panel: Option<usize>,
    /// `/theme` picker: temp theme index while open.
    theme_panel: Option<usize>,
    /// /relay panel: resumable/relayable sessions, the active agent tab, and the
    /// selected index within that tab (when open).
    relay: Vec<RelaySession>,
    relay_menu: Option<usize>,
    relay_tab: usize,
    /// First Ctrl+C arms quit; a second within the window exits.
    quit_armed: Option<Instant>,
    /// Last user activity; drives the inactivity auto-review.
    last_activity: Instant,
    /// True once the idle conversation has been auto-reviewed (until next input).
    auto_reviewed: bool,
    /// Shell mode: a leading `!` becomes the prompt, the rest is the command.
    shell_mode: bool,
    /// Clipboard images pasted (Ctrl+V), sent with the next message.
    pending_images: Vec<a3s_code_core::llm::Attachment>,
    /// Persistent north-star goal (`/goal`), prepended to each prompt.
    goal: Option<String>,
    /// Remaining auto-continue turns for `/loop` (0 = off).
    loop_remaining: usize,
    /// Live parallelism for the status bar: running tools + running subagents.
    active_tools: usize,
    active_agents: usize,
    /// Parallel subagent tasks shown in the bottom tracker panel.
    subagents: Vec<SubAgent>,
    /// Project instructions (CLAUDE.md/AGENT.md), injected into the system prompt.
    instructions: Option<String>,
    /// Summary of earlier conversation after a manual `/compact` (reseed).
    compact_summary: Option<String>,
    /// Brief rainbow-ribbon flourish on the input border when ultracode is picked.
    rainbow_until: Option<Instant>,
    rainbow_frame: usize,
    /// Ultracode confirm animation playing in the /effort panel before it closes.
    effort_anim: Option<Instant>,
    /// Active `/btw` side-chat shown as a panel: (question, answer-once-ready).
    btw: Option<(String, Option<String>)>,
    viewport: Viewport,
    textarea: Textarea,
    spinner: Spinner,
    streaming: StreamingMarkdown,
    /// Whether the current turn streamed any text deltas (vs. text only at End).
    got_delta: bool,
    /// Set while `/compact` is summarizing — drives the progress bar + blocks input.
    compacting: Option<Instant>,
    /// Last time the streaming viewport was rebuilt — throttles the O(n) rebuild
    /// to ~30fps so a flood of deltas doesn't starve animation on the 1 loop.
    last_paint: Option<Instant>,
    /// Live reasoning ("thinking") text for the current turn, shown dimmed above
    /// the answer and cleared when the answer is finalized.
    thinking: String,
    state: State,
    messages: Vec<String>,
    rx: Option<SharedRx>,
    pending_tool: Option<(String, String)>,
    /// Selected row in the tool-approval options panel (0 yes · 1 always · 2 no).
    approval_sel: usize,
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
    /// Frame counter for the welcome-mascot animation.
    anim: u8,
    /// Run mode (Shift+Tab cycles default → plan → auto).
    mode: Mode,
    /// User messages submitted while the agent is busy, run when it frees up.
    queue: BinaryHeap<Queued>,
    /// Monotonic counter for FIFO ordering within a queue priority.
    seq: u64,
    /// Text of the message currently being processed (the running task).
    running_task: Option<String>,
    /// Live plan/TODO from planning mode: (task text, status glyph, colour),
    /// pinned above the input. Updated from PlanningEnd/TaskUpdated events.
    plan: Vec<(String, String, char, Color)>, // (id, content, glyph, colour)
    /// `/top` process panel: `Some(rows)` when open; `top_scroll` is the scroll
    /// offset and `top_sel` the highlighted (absolute) row index.
    top: Option<Vec<ProcRow>>,
    top_scroll: usize,
    top_sel: usize,
    /// Pending force-kill confirmation in `/top`: (pid, command label).
    top_kill: Option<(String, String)>,
    /// `/ide` file-tree + viewer panel (Some when open).
    ide: Option<Ide>,
    /// `/git` full-screen panel (Some when open).
    git: Option<Git>,
    /// `/help` overlay panel is showing.
    help_open: bool,
    /// Turns completed this session, for the status-bar task counter.
    completed: usize,
    /// Working directory shown for context.
    cwd: String,
    /// Git branch of the workspace (if any), shown in the bottom status bar.
    branch: Option<String>,
    /// Selected index in the `/` command menu.
    slash_sel: usize,
    /// Workspace files (for the `@` file picker) + its selected index.
    files: Vec<String>,
    file_sel: usize,
    /// Expanded directories in the `@` picker tree (collapsed by default).
    at_expanded: std::collections::HashSet<String>,
    /// Count of discoverable Claude skills (incl. plugin-bundled) for the banner.
    skill_count: usize,
    /// Loaded skills (name, description) for the slash menu + `/plugin`.
    skills: Vec<(String, String)>,
    /// Skill names the user disabled via `/plugins` (persisted, hidden from `/`).
    disabled_skills: std::collections::HashSet<String>,
    /// `/plugins` panel: selected row while open.
    plugins_panel: Option<usize>,
    /// Newer release found at startup (latest version), if any.
    update_available: Option<String>,
    width: u16,
    height: u16,
    keymap: Keymap<Action>,
}

impl Model for App {
    type Msg = Msg;

    fn init(&mut self) -> Option<Cmd<Msg>> {
        // Auto-check for a newer release on every launch (non-blocking).
        let mut cmds = vec![cmd::cmd(|| async {
            Msg::UpdateCheck(check_latest_version().await)
        })];
        if self.messages.is_empty() {
            self.viewport.set_content(&self.banner());
            cmds.push(banner_tick()); // start the mascot animation
        } else {
            // Resumed session — show the prior conversation, scrolled to the end.
            self.rebuild_viewport();
            self.viewport.update(ViewportMsg::Bottom);
        }
        Some(cmd::batch(cmds))
    }

    fn update(&mut self, msg: Msg) -> Option<Cmd<Msg>> {
        match msg {
            Msg::Term(Event::Resize { width, height }) => {
                self.width = width;
                self.height = height;
                self.relayout();
                self.textarea
                    .set_width(width.saturating_sub((PAD + 2) as u16));
                self.streaming = StreamingMarkdown::new((width as usize).saturating_sub(PAD + 2));
                self.rebuild_viewport();
            }

            Msg::Term(Event::Key(key)) => {
                self.last_activity = Instant::now();
                self.auto_reviewed = false;
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
                // Esc closes the /btw side-chat panel.
                if self.btw.is_some() && key.code == KeyCode::Esc {
                    self.btw = None;
                    return None;
                }
                // The /help overlay closes on any key.
                if self.help_open {
                    self.help_open = false;
                    return None;
                }
                // /git panel takes all keys while open.
                if self.git.is_some() {
                    return self.git_key(&key);
                }
                // /ide panel takes all keys while open.
                if self.ide.is_some() {
                    self.ide_key(&key);
                    return None;
                }
                // /top panel takes keys while open.
                if self.top.is_some() {
                    // A force-kill confirmation grabs keys first.
                    if self.top_kill.is_some() {
                        match key.code {
                            KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                                let pid = self.top_kill.take().unwrap().0;
                                return Some(cmd::cmd(move || async move {
                                    let _ = tokio::process::Command::new("kill")
                                        .arg("-9")
                                        .arg(&pid)
                                        .output()
                                        .await;
                                    Msg::TopData(fetch_top().await) // refresh after kill
                                }));
                            }
                            KeyCode::Char('n' | 'N') | KeyCode::Esc => self.top_kill = None,
                            _ => {}
                        }
                        return None;
                    }
                    let last = self.top.as_ref().map_or(0, |r| r.len()).saturating_sub(1);
                    match key.code {
                        KeyCode::Esc => self.top = None,
                        KeyCode::Up | KeyCode::Char('k') => {
                            self.top_sel = self.top_sel.saturating_sub(1)
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            self.top_sel = (self.top_sel + 1).min(last)
                        }
                        KeyCode::PageUp => self.top_sel = self.top_sel.saturating_sub(10),
                        KeyCode::PageDown => self.top_sel = (self.top_sel + 10).min(last),
                        // Enter asks to force-kill the highlighted process.
                        KeyCode::Enter => {
                            let info = self
                                .top
                                .as_ref()
                                .and_then(|rs| rs.get(self.top_sel))
                                .map(|r| (r.pid.clone(), r.cmd.clone()));
                            self.top_kill = info;
                        }
                        _ => {}
                    }
                    // Keep the selection within the visible window.
                    let body = (self.height as usize).saturating_sub(3);
                    if self.top_sel < self.top_scroll {
                        self.top_scroll = self.top_sel;
                    } else if self.top_sel >= self.top_scroll + body {
                        self.top_scroll = self.top_sel + 1 - body;
                    }
                    return None;
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
                // /effort slider takes keys while open.
                if let Some(sel) = self.effort_panel {
                    match key.code {
                        KeyCode::Left => self.effort_panel = Some(sel.saturating_sub(1)),
                        KeyCode::Right => {
                            self.effort_panel = Some((sel + 1).min(EFFORT_LEVELS.len() - 1))
                        }
                        KeyCode::Enter => {
                            self.effort = sel;
                            if sel == ULTRACODE {
                                // Play a flourish in the panel, then close + apply
                                // (handled on the banner tick).
                                self.effort_anim = Some(Instant::now());
                                self.rainbow_frame = 0;
                            } else {
                                self.effort_panel = None;
                                self.apply_effort();
                            }
                        }
                        KeyCode::Esc => {
                            self.effort_panel = None;
                            self.effort_anim = None;
                        }
                        _ => {}
                    }
                    return None;
                }
                // /theme picker: ↑/↓ preview, Enter apply, Esc cancel.
                if let Some(sel) = self.theme_panel {
                    match key.code {
                        KeyCode::Up => self.theme_panel = Some(sel.saturating_sub(1)),
                        KeyCode::Down => self.theme_panel = Some((sel + 1).min(THEMES.len() - 1)),
                        KeyCode::Enter => {
                            SYNTAX_THEME.store(sel, std::sync::atomic::Ordering::Relaxed);
                            self.theme_panel = None;
                            self.rebuild_viewport();
                            self.push_line(
                                &Style::new()
                                    .fg(Color::Green)
                                    .render(&format!("  ◆ code theme: {}", THEMES[sel].name)),
                            );
                        }
                        KeyCode::Esc => self.theme_panel = None,
                        _ => {}
                    }
                    return None;
                }
                // /plugins panel: ↑/↓ select, Space enable/disable, Esc close.
                if let Some(sel) = self.plugins_panel {
                    let last = self.skills.len().saturating_sub(1);
                    match key.code {
                        KeyCode::Up => self.plugins_panel = Some(sel.saturating_sub(1)),
                        KeyCode::Down => self.plugins_panel = Some((sel + 1).min(last)),
                        KeyCode::Char(' ') => {
                            if let Some((name, _)) = self.skills.get(sel.min(last)) {
                                let name = name.clone();
                                if !self.disabled_skills.remove(&name) {
                                    self.disabled_skills.insert(name);
                                }
                                save_disabled_skills(&self.disabled_skills);
                            }
                        }
                        KeyCode::Esc => self.plugins_panel = None,
                        _ => {}
                    }
                    return None;
                }
                // /relay picker takes keys while open.
                if self.relay_menu.is_some() {
                    if let Some(result) = self.handle_relay_key(&key) {
                        return result;
                    }
                }
                // Shift+End jumps to the latest output and resumes auto-follow.
                if key.code == KeyCode::End && key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.viewport.update(ViewportMsg::Bottom);
                    self.viewport.set_auto_scroll(true);
                    return None;
                }
                if let Some(action) = self.keymap.resolve(&key) {
                    let m = match action {
                        Action::ScrollUp => ViewportMsg::PageUp,
                        Action::ScrollDown => ViewportMsg::PageDown,
                        Action::ScrollTop => ViewportMsg::Top,
                        Action::ScrollBottom => ViewportMsg::Bottom,
                    };
                    self.viewport.update(m);
                    // Pause auto-follow while scrolled up; resume once back at the
                    // bottom — so streaming output doesn't yank the view down.
                    self.viewport.set_auto_scroll(self.viewport.at_bottom());
                    return None;
                }
                // Esc leaves shell mode first (discarding the partial command),
                // taking priority over the streaming interrupt below.
                if self.shell_mode && key.code == KeyCode::Esc {
                    self.shell_mode = false;
                    self.textarea.clear();
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
                // `@` file picker takes nav keys while open.
                if self.file_menu_open() {
                    if let Some(result) = self.handle_file_key(&key) {
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
                // Ctrl+V pastes a clipboard image (macOS Cmd+V is swallowed by the
                // terminal, so the app can't see it) to attach to the next message.
                if key.code == KeyCode::Char('v') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.paste_clipboard_image();
                    return None;
                }
                // Input is always live (you can keep typing while the agent works);
                // a submit while busy is queued and run when the current turn ends.
                if let Some(TextareaMsg::Submit(text)) = self.textarea.handle_key(&key) {
                    return Some(cmd::msg(Msg::Submit(text)));
                }
                // Shell mode: a leading `!` becomes the prompt (stripped from the
                // text). It stays on until Esc or a submit (handled elsewhere).
                let val = self.textarea.value();
                if !self.shell_mode && val.starts_with('!') {
                    self.shell_mode = true;
                    self.textarea.set_value(val.strip_prefix('!').unwrap_or(""));
                }
            }

            Msg::Term(Event::Mouse(m)) => {
                use a3s_tui::event::MouseEventKind;
                match m.kind {
                    MouseEventKind::ScrollUp => self.viewport.update(ViewportMsg::ScrollUp(3)),
                    MouseEventKind::ScrollDown => self.viewport.update(ViewportMsg::ScrollDown(3)),
                    _ => {}
                }
                // Pause auto-follow while scrolled up (so streaming output won't
                // yank the view down); resume once back at the bottom.
                self.viewport.set_auto_scroll(self.viewport.at_bottom());
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

            Msg::Interrupted => {
                // Esc force-aborted the turn: keep partial output, drop the
                // stream (finish() clears rx so late events are ignored), idle.
                self.finalize_streaming();
                self.push_line(&Style::new().fg(Color::Yellow).render("  ⎋ interrupted"));
                self.loop_remaining = 0; // Esc also stops a /loop
                self.finish();
                return self.drain_queue();
            }

            Msg::Agent(event) => return self.on_agent_event(*event),

            Msg::StreamEnded => {
                if self.state == State::Streaming {
                    self.finalize_streaming();
                    self.completed += 1;
                }
                self.finish();
                // /loop: auto-continue until the agent says DONE, the cap is hit,
                // or Esc. Queued user messages take priority.
                if self.loop_remaining > 0 && self.queue.is_empty() {
                    self.loop_remaining -= 1;
                    let n = self.loop_remaining;
                    self.push_line(
                        &Style::new()
                            .fg(Color::BrightBlack)
                            .render(&format!("  ↻ loop ({n} left · Esc to stop)")),
                    );
                    return Some(cmd::msg(Msg::Submit(
                        "Continue. If the task is fully complete, reply DONE and stop.".to_string(),
                    )));
                }
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

            Msg::BannerTick => {
                // Re-render the animated mascot only while the banner is shown
                // (start screen / after /clear); the heartbeat keeps running so
                // the animation resumes whenever the banner reappears.
                if self.messages.is_empty()
                    && self.state == State::Idle
                    && self.top.is_none()
                    && self.ide.is_none()
                    && self.git.is_none()
                    && !self.help_open
                {
                    self.anim = self.anim.wrapping_add(1);
                    self.viewport.set_content(&self.banner());
                }
                // Advance the ultracode rainbow flourish (re-renders via the view).
                if self.rainbow_until.is_some() || self.effort_anim.is_some() {
                    self.rainbow_frame = self.rainbow_frame.wrapping_add(1);
                }
                // Ultracode confirm flourish: play in the /effort panel ~1.1s,
                // then close the panel and apply (which lights the input borders).
                if let Some(t) = self.effort_anim {
                    if t.elapsed() > Duration::from_millis(1100) {
                        self.effort_anim = None;
                        self.effort_panel = None;
                        self.apply_effort();
                    }
                }
                // Inactivity auto-review: after a quiet stretch with a real
                // conversation, summarise it once as a side note (Claude-style).
                if !self.auto_reviewed
                    && self.state == State::Idle
                    && !self.messages.is_empty()
                    && self.last_activity.elapsed() > Duration::from_secs(300)
                {
                    self.auto_reviewed = true;
                    let agent = self.agent.clone();
                    let workspace = self.cwd.clone();
                    let history = self.session.history();
                    let review = cmd::cmd(move || async move {
                        let conf = a3s_code_core::hitl::ConfirmationPolicy::enabled()
                            .with_timeout(500, TimeoutAction::Reject);
                        let prompt = "Briefly review this conversation so far: summarise the \
                             key decisions and what's done, then list any open threads or next \
                             steps. Keep it to a few lines.";
                        let mut answer = String::new();
                        if let Ok(sess) = agent.session(
                            workspace,
                            Some(SessionOptions::new().with_confirmation_policy(conf)),
                        ) {
                            if let Ok((mut rx, _j)) = sess.stream(prompt, Some(&history)).await {
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
                        }
                        Msg::AutoReview(answer)
                    });
                    return Some(cmd::batch(vec![banner_tick(), review]));
                }
                return Some(banner_tick());
            }

            Msg::AutoReview(text) => {
                if !text.trim().is_empty() {
                    // Dim + unobtrusive — it's a passive side note, not output.
                    let dim = |s: &str| {
                        format!(
                            "  {}",
                            Style::new().fg(Color::BrightBlack).italic().render(s)
                        )
                    };
                    let mut lines = vec![dim("⟳ inactivity review")];
                    lines.extend(text.trim().lines().map(dim));
                    self.push_line(&lines.join("\n"));
                }
            }

            Msg::Compacted(summary) => {
                self.compacting = None;
                if summary.trim().is_empty() {
                    self.push_line(
                        &Style::new()
                            .fg(Color::Red)
                            .render("  compaction failed (empty summary)"),
                    );
                    return None;
                }
                // Reseed a FRESH session (new id, no history) carrying just the
                // summary in its system prompt — that's the actual compaction.
                self.compact_summary = Some(summary.trim().to_string());
                self.session_id = new_session_id();
                let model = self.model.clone();
                match self.rebuild_session(model.as_deref()) {
                    Ok((s, _)) => {
                        self.session = Arc::new(s);
                        self.messages.clear();
                        self.total_tokens = 0;
                        self.last_prompt_tokens = 0;
                        self.push_line(
                            &Style::new()
                                .fg(Color::Green)
                                .bold()
                                .render("  ✦ context compacted — continuing from this summary:"),
                        );
                        self.push_line(&gutter(
                            Color::Cyan,
                            self.compact_summary.as_deref().unwrap_or(""),
                        ));
                        self.rebuild_viewport();
                    }
                    Err(e) => self.push_line(
                        &Style::new()
                            .fg(Color::Red)
                            .render(&format!("  compaction failed: {e}")),
                    ),
                }
            }

            Msg::UpdateCheck(latest) => {
                let newer = latest
                    .as_deref()
                    .is_some_and(|l| !crate::version_ge(env!("CARGO_PKG_VERSION"), l));
                if newer {
                    self.update_available = latest;
                    // Refresh the start screen so the notice shows in the banner
                    // without clobbering it with a transcript line.
                    if self.messages.is_empty() {
                        self.viewport.set_content(&self.banner());
                    }
                }
            }

            Msg::ModalConfirm(idx) => {
                let approved = idx == 0;
                self.state = State::Streaming;
                if let Some((tool_id, label)) = self.pending_tool.take() {
                    // Approved → silent (the tool runs, ToolEnd shows the result);
                    // denied → a brief note since no result will follow.
                    if !approved {
                        self.push_line(
                            &Style::new()
                                .fg(Color::Red)
                                .render(&format!("  ⎿ denied {label}")),
                        );
                    }
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
                if let Some((q, _)) = self.btw.take() {
                    self.btw = Some((q, Some(text.trim().to_string())));
                }
            }

            Msg::TopData(rows) => {
                if self.top.is_some() {
                    self.top = Some(rows);
                    return Some(cmd::tick(Duration::from_millis(1500), Msg::TopRefresh));
                }
            }
            Msg::TopRefresh => {
                if self.top.is_some() {
                    return Some(cmd::cmd(|| async { Msg::TopData(fetch_top().await) }));
                }
            }
            Msg::RelayData(sessions) => {
                if self.relay_menu.is_some() {
                    self.relay = sessions;
                }
            }

            Msg::GitStatus(files, log) => {
                if let Some(g) = &mut self.git {
                    g.files = files;
                    g.log = log;
                    g.sel = g.sel.min(g.files.len().saturating_sub(1));
                    g.log_sel = g.log_sel.min(g.log.len().saturating_sub(1));
                    g.note.clear();
                    return self.git_load_diff();
                }
            }
            Msg::GitDiff(lines) => {
                if let Some(g) = &mut self.git {
                    g.diff = lines;
                    g.diff_scroll = 0;
                }
            }

            _ => {}
        }
        None
    }

    fn view(&self) -> String {
        if self.help_open {
            return self.render_help();
        }
        if let Some(g) = &self.git {
            return self.render_git(g);
        }
        if let Some(ide) = &self.ide {
            return self.render_ide(ide);
        }
        if let Some(rows) = &self.top {
            return self.render_top_panel(rows);
        }
        let width = self.width as usize;
        let viewport_view = self.viewport.view();
        // Input mode hint: `!` = shell command (pink), `/btw` = side-channel
        // (yellow), otherwise the normal prompt (accent blue).
        let inp = self.textarea.value();
        let (sym, icolor, border): (&str, Color, Color) = if self.shell_mode {
            ("!", Color::Rgb(255, 105, 180), Color::Rgb(255, 105, 180))
        } else if inp.starts_with("/btw") {
            ("❯", Color::Yellow, Color::Yellow)
        } else {
            ("❯", ACCENT, Color::BrightBlack)
        };
        // Brief rainbow ribbon on BOTH input borders right after picking
        // ultracode; otherwise plain bottom + effort-chip top.
        let bar = width.saturating_sub(2 * PAD);
        let rainbow = self
            .rainbow_until
            .is_some_and(|t| t.elapsed() < Duration::from_millis(1600));
        const PALETTE: [Color; 7] = [
            Color::Rgb(255, 0, 0),
            Color::Rgb(255, 127, 0),
            Color::Rgb(255, 255, 0),
            Color::Rgb(0, 220, 0),
            Color::Rgb(0, 150, 255),
            Color::Rgb(75, 0, 200),
            Color::Rgb(160, 0, 230),
        ];
        let ribbon = |offset: usize| {
            let mut s = " ".repeat(PAD);
            for i in 0..bar {
                let c = PALETTE[(i + self.rainbow_frame + offset) % PALETTE.len()];
                s.push_str(&Style::new().fg(c).bold().render("━"));
            }
            s
        };
        let separator = if rainbow {
            ribbon(3)
        } else {
            Style::new()
                .fg(border)
                .render(&format!("{}{}", " ".repeat(PAD), "─".repeat(bar)))
        };
        let top_separator = if rainbow {
            ribbon(0)
        } else {
            let elabel = format!("◇ {}", EFFORT_LEVELS[self.effort].0);
            let left = bar.saturating_sub(elabel.chars().count() + 4);
            format!(
                "{}{} {} {}",
                " ".repeat(PAD),
                Style::new().fg(border).render(&"─".repeat(left)),
                Style::new().fg(ACCENT).bold().render(&elabel),
                Style::new().fg(border).render("──"),
            )
        };

        // Activity line directly above the input: spinner while the agent works,
        // an inline approval prompt while awaiting, empty when idle.
        let activity = if let Some(t0) = self.compacting {
            // Time-estimated progress bar (compaction has no real % to report).
            let secs = t0.elapsed().as_secs();
            let pct = ((secs as f64 / 30.0) * 100.0).min(95.0) as usize;
            let filled = pct * 24 / 100;
            let bar = format!("{}{}", "▰".repeat(filled), "▱".repeat(24 - filled));
            Style::new().fg(ACCENT).render(&format!(
                "  ✦ Compacting context… ({}) {bar} {pct}%",
                fmt_elapsed(t0.elapsed())
            ))
        } else {
            match self.state {
                State::Streaming => {
                    // Pulsing sparkle + "Thinking…" with live elapsed + token count.
                    let g = ['✶', '✸', '✹', '✺', '✹', '✷'][(self.blink_tick as usize / 2) % 6];
                    let spark = Style::new().fg(ACCENT).render(&g.to_string());
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
                            tail.push_str(&format!(" · ↓ {} tokens", humanize(est)));
                        }
                        tail.push(')');
                    }
                    let tail = Style::new().fg(ACCENT).render(&tail);
                    format!("  {spark} {working}{tail}")
                }
                // The approval options panel (overlay_approval) is the UI now.
                State::Awaiting => String::new(),
                State::Idle => String::new(),
            }
        };

        let prompt = Style::new().fg(icolor).bold().render(&format!("{sym} "));
        let typed = self.textarea.view();
        let typed = if sym == "!" || inp.starts_with("/btw") {
            Style::new().fg(icolor).render(&typed)
        } else {
            typed
        };
        // First line carries the prompt; continuation lines (multi-line input)
        // are indented to align under the prompt (PAD margin + "{sym} " = PAD+2).
        let input_view = {
            let cont = " ".repeat(PAD + 2);
            let mut parts = typed.split('\n');
            let first = parts.next().unwrap_or("");
            let mut s = format!("{}{}{}", " ".repeat(PAD), prompt, first);
            for line in parts {
                s.push('\n');
                s.push_str(&cont);
                s.push_str(line);
            }
            s
        };

        // Bottom status bar (Claude-style, two lines):
        //   dir git:(branch) <model> (<window> context) ctx:N%   [+ live chips]
        //   ⏵⏵ <mode> mode on (shift+tab to cycle) · …
        let dim = |s: &str| Style::new().fg(Color::BrightBlack).render(s);
        let dir = self.cwd.rsplit('/').next().unwrap_or(&self.cwd);
        let mut line1 = format!("  {}", Style::new().fg(ACCENT).bold().render(dir));
        if let Some(b) = &self.branch {
            line1.push_str(&format!(
                " {}{}{}",
                dim("git:("),
                Style::new().fg(TN_YELLOW).render(b),
                dim(")")
            ));
        }
        if let Some(m) = &self.model {
            let name = m.rsplit('/').next().unwrap_or(m);
            line1.push_str(&format!("  {}", Style::new().fg(TN_FG).render(name)));
            if self.context_limit > 0 {
                let win = if self.context_limit >= 1_000_000 {
                    format!("{}M", self.context_limit / 1_000_000)
                } else {
                    format!("{}k", self.context_limit / 1000)
                };
                line1.push_str(&format!(" {}", dim(&format!("({win} context)"))));
            }
        }
        if self.context_limit > 0 {
            let pct = (self.last_prompt_tokens * 100 / self.context_limit as usize).min(100);
            line1.push_str(&format!(" {}", dim(&format!("ctx:{pct}%"))));
        } else if self.total_tokens > 0 {
            line1.push_str(&format!(" {}", dim(&format!("{} tok", self.total_tokens))));
        }
        // Live chips, only when active.
        if let Some(g) = &self.goal {
            let short: String = g.chars().take(24).collect();
            line1.push_str(&format!("  🎯 {short}"));
        }
        if self.loop_remaining > 0 {
            line1.push_str(&format!("  ↻{}", self.loop_remaining));
        }
        if self.active_agents > 0 {
            line1.push_str(&format!("  ⇉ {} agents", self.active_agents));
        }
        if self.active_tools > 0 {
            line1.push_str(&format!("  ⚙ {} running", self.active_tools));
        }
        if let Some(v) = &self.update_available {
            line1.push_str(&format!("  ⬆ {v}"));
        }
        let status1 = pad_to(&line1, width);
        let mode_part = Style::new().fg(self.mode.color()).bold().render(&format!(
            "  {} {} mode on",
            self.mode.glyph(),
            self.mode.name()
        ));
        let hints = dim(" (shift+tab to cycle) · /help · ↑↓ history · esc");
        let status2 = pad_to(&format!("{mode_part}{hints}"), width);

        // Gap line between transcript and loading — or a floating "jump to
        // latest" hint when the user has scrolled up away from the bottom.
        let spacer = if self.viewport.at_bottom() {
            String::new()
        } else {
            let label = " ↓ more below · Shift+End to jump to latest ";
            let pad = width.saturating_sub(a3s_tui::style::visible_len(label)) / 2;
            format!(
                "{}{}",
                " ".repeat(pad),
                Style::new().fg(Color::Black).bg(ACCENT).render(label)
            )
        };
        let tasks = self.task_lines();
        let task_block = tasks.join("\n");
        // Plan/TODO panel + parallel-subagent tracker pinned above the input.
        let plan = self.plan_lines();
        let plan_block = plan.join("\n");
        let subs = self.subagent_lines();
        let sub_block = subs.join("\n");
        let composed = Layout::vertical()
            .item(&viewport_view, Constraint::Fill)
            .item(&spacer, Constraint::Fixed(1))
            .item(&activity, Constraint::Fixed(1))
            .item(&plan_block, Constraint::Fixed(plan.len() as u16))
            .item(&sub_block, Constraint::Fixed(subs.len() as u16))
            .item(&top_separator, Constraint::Fixed(1))
            .item(&input_view, Constraint::Fixed(self.input_height()))
            .item(&separator, Constraint::Fixed(1))
            .item(&status1, Constraint::Fixed(1))
            .item(&status2, Constraint::Fixed(1))
            .item(&task_block, Constraint::Fixed(tasks.len() as u16))
            .render(self.height);

        let composed = self.overlay_slash_menu(composed);
        let composed = self.overlay_file_menu(composed);
        let composed = self.overlay_model_menu(composed);
        let composed = self.overlay_relay_menu(composed);
        let composed = self.overlay_effort(composed);
        let composed = self.overlay_theme(composed);
        let composed = self.overlay_plugins(composed);
        let composed = self.overlay_approval(composed);
        self.overlay_btw(composed)
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        // In the /ide editor, place the cursor at the edit position.
        if let Some(ide) = &self.ide {
            if ide.focus_editor {
                if let Some(f) = &ide.file {
                    let width = self.width as usize;
                    let tw = (width / 3).clamp(16, 38);
                    let col = (tw + 8 + f.col).min(width.saturating_sub(1)) as u16;
                    let row = (2 + f.row.saturating_sub(f.scroll)) as u16;
                    return Some((col, row));
                }
            }
            return None;
        }
        // Real cursor at the input insertion point whenever the input is live —
        // idle OR streaming (you can keep typing while the agent works). Hidden
        // only during an approval prompt.
        if self.state == State::Awaiting
            || self.top.is_some()
            || self.git.is_some()
            || self.help_open
        {
            return None;
        }
        // Below the input: separator + 2 status lines + the bottom task panel.
        // The input itself spans `input_height` rows; the cursor sits on its row.
        let below = 3 + self.task_lines().len() as u16;
        let row = self.height.saturating_sub(below + self.input_height())
            + self.textarea.cursor_row() as u16;
        let col = (PAD + 2) as u16 + self.textarea.cursor_display_col() as u16; // PAD + "❯ "
        Some((col, row))
    }
}

impl App {
    fn on_submit(&mut self, text: String) -> Option<Cmd<Msg>> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        // No input while compacting — the summary is reseeding the session.
        if self.compacting.is_some() {
            self.textarea.clear();
            return None;
        }
        // Shell mode (`!`) runs a shell command directly (not through the agent).
        if self.shell_mode {
            self.shell_mode = false;
            let cmd = trimmed.trim_start_matches('!').trim().to_string();
            if cmd.is_empty() {
                return None;
            }
            self.messages.push(gutter(
                Color::Rgb(255, 105, 180),
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
        // Block session-mutating commands while a turn is streaming.
        if self.state != State::Idle {
            let cmd0 = trimmed.split_whitespace().next().unwrap_or("");
            if IDLE_ONLY.contains(&cmd0) {
                self.textarea.clear();
                self.push_line(&Style::new().fg(Color::Yellow).render(&format!(
                    "  {cmd0} is unavailable while a turn is running — press Esc to stop first"
                )));
                return None;
            }
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
            self.btw = Some((q.clone(), None));
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
        // `/goal [text|clear]` — a persistent goal prepended to every prompt.
        if let Some(rest) = trimmed.strip_prefix("/goal") {
            let g = rest.trim();
            self.textarea.clear();
            if g.is_empty() {
                match &self.goal {
                    Some(cur) => self.push_line(&gutter(
                        Color::Cyan,
                        &format!("🎯 goal: {cur}   (/goal clear to remove)"),
                    )),
                    None => self.push_line(
                        &Style::new()
                            .fg(Color::BrightBlack)
                            .render("  usage: /goal <what you're working toward>"),
                    ),
                }
            } else if g == "clear" {
                self.goal = None;
                self.push_line(&Style::new().fg(Color::BrightBlack).render("  goal cleared"));
                return None;
            } else {
                // Set the persistent goal AND start working toward it now (the
                // goal is prepended to this and every later prompt).
                self.goal = Some(g.to_string());
                self.push_line(&gutter(Color::Cyan, &format!("🎯 goal set: {g}")));
                return Some(cmd::msg(Msg::Submit(g.to_string())));
            }
            return None;
        }
        // `/loop <task>` — run the task, then auto-continue until done / Esc.
        if let Some(rest) = trimmed.strip_prefix("/loop") {
            let task = rest.trim().to_string();
            self.textarea.clear();
            if task.is_empty() {
                self.push_line(
                    &Style::new().fg(Color::BrightBlack).render(
                        "  usage: /loop <task>   (auto-continues up to 8 turns; Esc stops)",
                    ),
                );
                return None;
            }
            self.loop_remaining = 8;
            return Some(cmd::msg(Msg::Submit(task)));
        }
        // Slash commands run inline in any state.
        match trimmed {
            "/exit" | "/quit" => return Some(cmd::quit()),
            "/clear" => {
                self.messages.clear();
                self.plan.clear();
                self.subagents.clear();
                self.queue.clear();
                self.completed = 0;
                self.textarea.clear();
                self.relayout();
                self.rebuild_viewport();
                return None;
            }
            "/init" => {
                // Agent-driven: analyze the repo and write AGENTS.md (auto-loaded
                // by the core, like CLAUDE.md). Guarded idle by IDLE_ONLY above.
                self.textarea.clear();
                self.messages.push(user_bubble(
                    "/init — generate AGENTS.md",
                    self.width as usize,
                ));
                self.rebuild_viewport();
                return self.start_stream(
                    "Analyze this codebase and create (or update) an AGENTS.md file at the \
                     project root. Include: a concise project overview, the exact build / test / \
                     lint / run commands, the high-level architecture and key directories, and \
                     the conventions an AI coding agent should follow. Base everything on what's \
                     actually in the repo, and write the file with your file-writing tool."
                        .to_string(),
                );
            }
            "/compact" => {
                self.textarea.clear();
                if self.state != State::Idle {
                    self.push_line(
                        &Style::new()
                            .fg(Color::Yellow)
                            .render("  finish the current turn before compacting"),
                    );
                    return None;
                }
                let history = self.session.history();
                if history.is_empty() {
                    self.push_line(
                        &Style::new()
                            .fg(Color::BrightBlack)
                            .render("  nothing to compact yet"),
                    );
                    return None;
                }
                self.compacting = Some(Instant::now()); // progress bar + input lock
                let agent = self.agent.clone();
                let workspace = self.cwd.clone();
                return Some(cmd::cmd(move || async move {
                    let conf = a3s_code_core::hitl::ConfirmationPolicy::enabled()
                        .with_timeout(500, TimeoutAction::Reject);
                    let prompt = "Summarize this conversation so a fresh session can continue \
                         seamlessly: the goal, key decisions, files/commands touched, current \
                         state, and the immediate next steps. Be thorough but compact.";
                    let mut summary = String::new();
                    if let Ok(sess) = agent.session(
                        workspace,
                        Some(SessionOptions::new().with_confirmation_policy(conf)),
                    ) {
                        if let Ok((mut rx, _j)) = sess.stream(prompt, Some(&history)).await {
                            while let Some(ev) = rx.recv().await {
                                match ev {
                                    AgentEvent::TextDelta { text } => summary.push_str(&text),
                                    AgentEvent::End { text, .. } => {
                                        if summary.trim().is_empty() {
                                            summary = text;
                                        }
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    Msg::Compacted(summary)
                }));
            }
            "/help" => {
                self.textarea.clear();
                self.help_open = true;
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
                // Resolve the config; if there's none, generate a starter so the
                // user always lands in the editor with something to edit.
                let path = find_config().map(std::path::PathBuf::from).or_else(|| {
                    let p = default_config_path()?;
                    let _ = write_template_config(&p);
                    Some(p)
                });
                match path {
                    Some(p) => self.open_config_in_ide(&p),
                    None => self.push_line(
                        &Style::new()
                            .fg(Color::Yellow)
                            .render("  could not locate a home directory for ~/.a3s/config.acl"),
                    ),
                }
                return None;
            }
            "/model" => {
                self.textarea.clear();
                self.open_model_menu();
                return None;
            }
            "/effort" => {
                self.textarea.clear();
                self.effort_panel = Some(self.effort);
                return None;
            }
            "/top" => {
                self.textarea.clear();
                self.top = Some(Vec::new());
                self.top_scroll = 0;
                self.top_sel = 0;
                return Some(cmd::cmd(|| async { Msg::TopData(fetch_top().await) }));
            }
            "/ide" => {
                self.textarea.clear();
                let entries = ide_children(std::path::Path::new(&self.cwd), 0);
                self.ide = Some(Ide {
                    entries,
                    sel: 0,
                    tree_scroll: 0,
                    file: None,
                    focus_editor: false,
                });
                return None;
            }
            "/plugin" | "/plugins" => {
                self.textarea.clear();
                if self.skills.is_empty() {
                    self.push_line(&Style::new().fg(Color::BrightBlack).render(
                        "  no Claude skills/plugins found (~/.claude/skills, ~/.claude/plugins)",
                    ));
                } else {
                    self.plugins_panel = Some(0);
                }
                return None;
            }
            "/theme" => {
                self.textarea.clear();
                let cur = SYNTAX_THEME.load(std::sync::atomic::Ordering::Relaxed);
                self.theme_panel = Some(cur.min(THEMES.len() - 1));
                return None;
            }
            "/reload" => {
                self.textarea.clear();
                // Hot-reload: re-discover skill dirs + re-parse (new plugins show up).
                let dirs = claude_skill_dirs(&self.cwd);
                self.skills = load_skills(&dirs);
                self.skill_count = count_skill_files(&dirs);
                self.push_line(&Style::new().fg(Color::Green).render(&format!(
                    "  ↻ reloaded — {} skills available in the / menu",
                    self.skills.len()
                )));
                return None;
            }
            "/update" => {
                self.textarea.clear();
                self.push_line(
                    &Style::new()
                        .fg(Color::BrightBlack)
                        .render("  upgrading a3s…"),
                );
                let exe = std::env::current_exe().ok();
                return Some(cmd::cmd(move || async move {
                    let out = match &exe {
                        Some(p) => tokio::process::Command::new(p).arg("update").output().await,
                        None => {
                            tokio::process::Command::new("a3s")
                                .arg("update")
                                .output()
                                .await
                        }
                    };
                    let text = match out {
                        Ok(o) => {
                            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                            s.push_str(&String::from_utf8_lossy(&o.stderr));
                            s
                        }
                        Err(e) => format!("update failed: {e}"),
                    };
                    Msg::ShellOutput(format!(
                        "{}\n(restart a3s code to use the new version)",
                        text.trim_end()
                    ))
                }));
            }
            "/git" => {
                self.textarea.clear();
                self.git = Some(Git {
                    files: Vec::new(),
                    sel: 0,
                    diff: Vec::new(),
                    diff_scroll: 0,
                    log: Vec::new(),
                    log_sel: 0,
                    view: GitView::Status,
                    commit_input: None,
                    note: "loading…".into(),
                });
                let repo = self.cwd.clone();
                return Some(cmd::cmd(move || async move {
                    let (files, log) = git_status_log(repo).await;
                    Msg::GitStatus(files, log)
                }));
            }
            "/relay" => {
                self.textarea.clear();
                // Open immediately (tabs show right away); scan off the UI thread
                // so reading large transcripts never freezes the panel.
                self.relay.clear();
                self.relay_menu = Some(0);
                self.relay_tab = 0;
                let cwd = self.cwd.clone();
                return Some(cmd::cmd(move || async move {
                    let sessions = tokio::task::spawn_blocking(move || scan_relay(&cwd))
                        .await
                        .unwrap_or_default();
                    Msg::RelayData(sessions)
                }));
            }
            _ => {}
        }

        self.history.push(trimmed.to_string());
        self.history_pos = None;
        // Show the user message in a background bubble, then run now (if idle)
        // or queue it (if the agent is busy).
        self.messages
            .push(user_bubble(trimmed, self.width as usize));
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
            self.relayout();
            None
        }
    }

    /// Begin streaming a prompt (the user message must already be on screen).
    /// Grab a clipboard image, preview it inline, and queue it for the next send.
    fn paste_clipboard_image(&mut self) {
        let dest =
            std::env::temp_dir().join(format!("a3s-paste-{}.png", self.pending_images.len()));
        if !clipboard_image_to(&dest) {
            self.push_line(
                &Style::new()
                    .fg(Color::Yellow)
                    .render("  no image in clipboard (Ctrl+V pastes a copied/screenshot image)"),
            );
            return;
        }
        let Ok(bytes) = std::fs::read(&dest) else {
            return;
        };
        self.messages.push(gutter(
            ACCENT,
            "📎 pasted image (sends with your next message):",
        ));
        // Render narrower than the viewport so half-block rows never wrap (a
        // wrapped row splits the picture and garbles it). Indent to align.
        let cols = (self.width as usize).saturating_sub(PAD + 2).min(72);
        if let Some(lines) = render_image_file(&dest, cols, 16) {
            for l in lines {
                self.messages.push(format!("{}{l}", " ".repeat(PAD)));
            }
        }
        self.rebuild_viewport();
        self.pending_images
            .push(a3s_code_core::llm::Attachment::png(bytes));
    }

    fn start_stream(&mut self, prompt: String) -> Option<Cmd<Msg>> {
        self.streaming.clear();
        self.got_delta = false; // track if this turn streamed any text deltas
        self.last_paint = None; // first delta of the turn paints immediately
        self.viewport.set_auto_scroll(true); // sending a message jumps to latest
        self.plan.clear(); // fresh plan per turn; planning events refill it
        self.running_task = Some(prompt.clone());
        self.state = State::Streaming;
        self.relayout();
        self.stream_started = Some(Instant::now());
        self.spinner.start();
        self.rebuild_viewport();
        let session = self.session.clone();
        let atts = std::mem::take(&mut self.pending_images);
        // Keep the agent aligned with the standing goal (display stays clean).
        let prompt = match &self.goal {
            Some(g) => format!("[Ongoing goal: {g}]\n\n{prompt}"),
            None => prompt,
        };
        // ultracode: drive the work through PTC — write + show a JS workflow
        // program, then run it dispatching steps to parallel subagents.
        let prompt = if self.effort == ULTRACODE {
            format!(
                "[ultracode] First, using the `program` tool, write a short JavaScript \
                 workflow program that decomposes this task into independent steps and \
                 dispatches them to parallel subagents (call parallel_task inside the \
                 program). Show the program, then execute it. Prefer inline program \
                 source; if you must write a script file, put it under the system temp \
                 directory (never the project workspace) and delete it when done.\n\n{prompt}"
            )
        } else {
            prompt
        };
        Some(cmd::batch(vec![
            cmd::cmd(move || async move {
                let res = if atts.is_empty() {
                    session.stream(prompt.as_str(), None).await
                } else {
                    session
                        .stream_with_attachments(prompt.as_str(), &atts, None)
                        .await
                };
                match res {
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
        // After an interrupt, rx is cleared — ignore any late buffered events.
        self.rx.as_ref()?;
        match event {
            AgentEvent::TextDelta { text } => {
                self.got_delta = true;
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
                self.active_tools += 1;
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
                self.active_tools = self.active_tools.saturating_sub(1);
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
            // Parallel/child task lifecycle (parallel_task, task) — show each
            // sub-task starting, its progress, and how it finished.
            AgentEvent::SubagentStart {
                task_id,
                agent,
                description,
                ..
            } => {
                self.finalize_streaming();
                self.active_agents += 1;
                // Track it in the live bottom panel instead of a transcript line.
                self.subagents.push(SubAgent {
                    task_id,
                    agent,
                    description,
                    started: Instant::now(),
                    tokens: 0,
                    done: false,
                });
                self.relayout();
            }
            AgentEvent::SubagentProgress {
                task_id, metadata, ..
            } => {
                // Pull a token count from the progress metadata, if present.
                let toks = metadata
                    .get("tokens")
                    .or_else(|| metadata.get("total_tokens"))
                    .or_else(|| metadata.pointer("/usage/total_tokens"))
                    .and_then(|v| v.as_u64());
                if let Some(s) = self.subagents.iter_mut().find(|s| s.task_id == task_id) {
                    if let Some(t) = toks {
                        s.tokens = s.tokens.max(t);
                    }
                }
            }
            AgentEvent::SubagentEnd {
                task_id,
                agent,
                output,
                success,
                ..
            } => {
                self.active_agents = self.active_agents.saturating_sub(1);
                // Drop it from the live panel; record the result in the transcript.
                self.subagents.retain(|s| s.task_id != task_id);
                self.relayout();
                let (mark, color) = if success {
                    ("✓", Color::Green)
                } else {
                    ("✗", Color::Red)
                };
                let snippet = output.lines().next().unwrap_or("").trim();
                let snippet = truncate(snippet, self.width.saturating_sub(20) as usize);
                self.push_line(&Style::new().fg(color).render(&format!(
                    "  ⇉ {mark} {agent}{}",
                    if snippet.is_empty() {
                        String::new()
                    } else {
                        format!(" · {snippet}")
                    }
                )));
            }
            AgentEvent::ConfirmationRequired {
                tool_id,
                tool_name,
                args,
                ..
            } => {
                if self.mode.auto_approves(&tool_name) {
                    // Silent: the mode indicator already shows auto-approve is on;
                    // a line per tool is just noise.
                    let session = self.session.clone();
                    return Some(cmd::batch(vec![
                        cmd::cmd(move || async move {
                            let _ = session.confirm_tool_use(&tool_id, true, None).await;
                            Msg::Resume
                        }),
                        spinner_tick(),
                    ]));
                }
                // Claude-style: no "requests:" transcript line — the prompt on
                // the activity line shows the tool; after approval the tool just
                // runs and its result lands via ToolEnd.
                self.state = State::Awaiting;
                self.approval_sel = 0;
                let label = tool_label(&tool_name, Some(&args));
                self.pending_tool = Some((tool_id, label));
                return None; // wait for the user; do not pump
            }
            AgentEvent::End {
                text, usage, meta, ..
            } => {
                // /loop: stop once the agent signals completion (the word DONE).
                if self.loop_remaining > 0 {
                    let r = if text.is_empty() {
                        self.streaming.raw_content().to_string()
                    } else {
                        text.clone()
                    };
                    if r.split(|c: char| !c.is_alphabetic())
                        .any(|w| w.eq_ignore_ascii_case("done"))
                    {
                        self.loop_remaining = 0;
                    }
                }
                // Only fall back to End.text when the provider never streamed
                // deltas this turn. Using the live buffer's emptiness here dups
                // text: a mid-turn finalize (e.g. a tool call) empties the buffer,
                // so End.text (the full message) would be appended a second time.
                if !self.got_delta && !text.is_empty() {
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
            // Planning mode: capture the plan and live task-status updates for
            // the pinned TODO panel above the input.
            AgentEvent::PlanningEnd { plan, .. } => {
                self.set_plan(&plan.steps);
            }
            AgentEvent::TaskUpdated { tasks, .. } => {
                self.set_plan(&tasks);
            }
            // Per-step lifecycle also drives the panel, in case TaskUpdated is
            // sparse: a step turns ▶ on start and ✔/✗/⊘ on completion.
            AgentEvent::StepStart { step_id, .. } => {
                self.set_task_status(&step_id, '▶', Color::Yellow);
            }
            AgentEvent::StepEnd {
                step_id, status, ..
            } => {
                let (g, c) = task_status_style(status);
                self.set_task_status(&step_id, g, c);
            }
            // TurnStart/TurnEnd, ToolInputDelta, memory, confirmation echoes,
            // etc. — not surfaced in this MVP.
            _ => {}
        }
        // Keep draining the stream.
        self.rx.clone().map(pump)
    }

    fn finalize_streaming(&mut self) {
        let rendered = self.streaming.view();
        if !rendered.trim().is_empty() {
            let block = gutter(Color::Green, &rendered);
            // Safety net against duplicate output: skip if this exact block
            // already appeared in the last few messages (a re-finalize, or an
            // agent that re-emits earlier text — e.g. its preamble after a tool).
            let recent_dup = self.messages.iter().rev().take(4).any(|m| m == &block);
            if !recent_dup {
                self.messages.push(block);
            }
        }
        self.streaming.clear();
        self.thinking.clear();
        self.rebuild_viewport();
    }

    fn finish(&mut self) {
        self.state = State::Idle;
        self.running_task = None;
        self.active_tools = 0;
        self.active_agents = 0;
        self.subagents.clear();
        self.relayout();
        self.stream_started = None;
        self.spinner.stop();
        self.rx = None;
        self.rebuild_viewport();
    }

    fn push_line(&mut self, line: &str) {
        self.messages.push(line.to_string());
        self.rebuild_viewport();
    }

    /// Open `path` directly in the built-in IDE editor (tree rooted at its
    /// directory, file loaded, editor focused). Used by `/config` + first launch.
    fn open_config_in_ide(&mut self, path: &std::path::Path) {
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        let lines: Vec<String> = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace('\t', "    ")
            .lines()
            .map(String::from)
            .collect();
        self.ide = Some(Ide {
            entries: ide_children(dir, 0),
            sel: 0,
            tree_scroll: 0,
            file: Some(IdeFile {
                path: path.to_path_buf(),
                lines: if lines.is_empty() {
                    vec![String::new()]
                } else {
                    lines
                },
                scroll: 0,
                row: 0,
                col: 0,
                dirty: false,
                image: false,
            }),
            focus_editor: true,
        });
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
        // Throttle this O(n) rebuild to ~30fps. A fast stream emits deltas far
        // faster than that; rebuilding the whole transcript each time starves
        // the animation ticks on the single-threaded loop (the "时快时慢" jitter).
        if let Some(t) = self.last_paint {
            if t.elapsed() < Duration::from_millis(33) {
                return;
            }
        }
        self.last_paint = Some(Instant::now());
        let mut blocks: Vec<String> = self.messages.clone();
        if !self.thinking.trim().is_empty() {
            let body = indent(&format!("💭 {}", self.thinking.trim()), PAD);
            blocks.push(Style::new().fg(Color::BrightBlack).italic().render(&body));
        }
        let rendered = self.streaming.view();
        if !rendered.is_empty() {
            blocks.push(gutter(Color::Green, &rendered));
        }
        // Currently-executing tool: "• Running <cmd>…" with a blinking bullet.
        if let Some(name) = &self.running_tool {
            let args: Option<serde_json::Value> = serde_json::from_str(&self.tool_args).ok();
            let verb = match name.as_str() {
                "bash" | "shell" | "run" | "exec" => "Running",
                _ => tool_verb(name),
            };
            let arg = args.as_ref().and_then(arg_summary).unwrap_or_default();
            let on = self.blink_tick % 8 < 4; // ~320ms on / 320ms off
            let dot = Style::new()
                .fg(if on {
                    Color::Yellow
                } else {
                    Color::BrightBlack
                })
                .bold()
                .render("•");
            let m = " ".repeat(PAD);
            blocks.push(if arg.is_empty() {
                format!("{m}{dot} {verb}…")
            } else {
                format!("{m}{dot} {verb} {arg}…")
            });
        }
        // Live stdout of the running tool — tail prefixed with "│" like Codex.
        if !self.tool_output.trim().is_empty() {
            let m = " ".repeat(PAD + 2);
            let bar = Style::new().fg(Color::BrightBlack).render("│");
            let tail: Vec<&str> = self.tool_output.lines().rev().take(12).collect();
            let body = tail
                .into_iter()
                .rev()
                .map(|l| format!("{m}{bar} {}", Style::new().fg(Color::BrightBlack).render(l)))
                .collect::<Vec<_>>()
                .join("\n");
            blocks.push(body);
        }
        // Same "\n…\n" framing as rebuild_viewport so the transcript doesn't
        // jump a line when streaming starts/ends.
        self.viewport
            .set_content(&format!("\n{}\n", blocks.join("\n\n")));
    }

    fn rebuild_viewport(&mut self) {
        let full = self.messages.join("\n\n");
        self.viewport.set_content(&format!("\n{full}\n")); // top padding
    }

    /// Rows the input box needs — the textarea auto-grows its own height with
    /// embedded newlines (Shift+Enter), so the layout just mirrors it.
    pub(crate) fn input_height(&self) -> u16 {
        self.textarea.height()
    }

    /// Inline tool-approval keys (Codex-style): y/Enter allow, n/Esc deny,
    /// a = allow + enable auto-approve for the rest of the session.
    fn handle_approval_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        match key.code {
            KeyCode::Up => {
                self.approval_sel = self.approval_sel.saturating_sub(1);
                None
            }
            KeyCode::Down => {
                self.approval_sel = (self.approval_sel + 1).min(2);
                None
            }
            // Enter selects the highlighted option (0 yes · 1 always · 2 no).
            KeyCode::Enter => Some(cmd::msg(self.apply_approval(self.approval_sel))),
            KeyCode::Char('y' | 'Y') => Some(cmd::msg(self.apply_approval(0))),
            KeyCode::Char('a' | 'A') => Some(cmd::msg(self.apply_approval(1))),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Some(cmd::msg(self.apply_approval(2))),
            _ => None,
        }
    }

    fn apply_approval(&mut self, choice: usize) -> Msg {
        match choice {
            0 => Msg::ModalConfirm(0), // yes, once
            1 => {
                self.mode = Mode::Auto; // yes, and stop asking
                Msg::ModalConfirm(0)
            }
            _ => Msg::ModalConfirm(1), // no
        }
    }

    /// Tool-approval options panel (Claude-style numbered choices).
    fn overlay_approval(&self, composed: String) -> String {
        if self.state != State::Awaiting {
            return composed;
        }
        let Some((_, label)) = &self.pending_tool else {
            return composed;
        };
        let width = self.width as usize;
        let opts = ["Yes", "Yes, and don't ask again", "No"];
        let mut menu = vec![pad_to(
            &Style::new()
                .fg(Color::Yellow)
                .bold()
                .render(&format!("  ⏵ Allow {label}?")),
            width,
        )];
        for (i, o) in opts.iter().enumerate() {
            let marker = if i == self.approval_sel { "❯" } else { " " };
            let raw = pad_to(&format!("  {marker} {}. {o}", i + 1), width);
            menu.push(if i == self.approval_sel {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(Color::White).render(&raw)
            });
        }
        menu.push(pad_to(
            &Style::new()
                .fg(Color::BrightBlack)
                .render("  Enter select · ↑/↓ · Esc"),
            width,
        ));
        self.overlay_list(composed, &menu)
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

pub async fn run(args: Vec<String>) -> anyhow::Result<()> {
    // `a3s code resume [id]` continues a saved session (newest if no id given);
    // otherwise a fresh id. Existence is verified against the store below.
    let resuming = args.first().map(String::as_str) == Some("resume");
    let explicit_id = if resuming { args.get(1).cloned() } else { None };
    let mut session_id = explicit_id.clone().unwrap_or_else(new_session_id);
    // First launch: if there's no config, generate a starter template at
    // ~/.a3s/config.acl and open it in the built-in IDE (see `created_config`).
    let (config_path, created_config) = match find_config() {
        Some(p) => (p, false),
        None => {
            let p = default_config_path()
                .ok_or_else(|| anyhow::anyhow!("no HOME directory found for ~/.a3s/config.acl"))?;
            write_template_config(&p)
                .map_err(|e| anyhow::anyhow!("failed to write starter config {p:?}: {e}"))?;
            (p.to_string_lossy().into_owned(), true)
        }
    };
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

    // Resolve `resume`: verify the id exists (else show what's available), or
    // pick the most recent session when no id was given.
    if resuming {
        let mut saved: Vec<(String, std::time::SystemTime)> = std::fs::read_dir(&store_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("json") {
                    return None;
                }
                let id = p.file_stem()?.to_str()?.to_string();
                let mtime = e.metadata().ok()?.modified().ok()?;
                Some((id, mtime))
            })
            .collect();
        saved.sort_by_key(|e| std::cmp::Reverse(e.1)); // newest first
        match &explicit_id {
            Some(id) if !saved.iter().any(|(s, _)| s == id) => {
                eprintln!("a3s: session '{id}' not found in {}", store_dir.display());
                if saved.is_empty() {
                    eprintln!("  (no saved sessions in this directory)");
                } else {
                    eprintln!("  available sessions (newest first):");
                    for (s, _) in saved.iter().take(10) {
                        eprintln!("    a3s code resume {s}");
                    }
                }
                return Ok(());
            }
            None => match saved.first() {
                Some((s, _)) => session_id = s.clone(),
                None => {
                    eprintln!(
                        "a3s: no saved sessions to resume in {}",
                        store_dir.display()
                    );
                    return Ok(());
                }
            },
            _ => {}
        }
    }

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
    // Claude Code compatibility: load Claude/plugin SKILL.md skills alongside
    // a3s's own (they share the markdown + YAML-frontmatter format).
    let claude_dirs = claude_skill_dirs(&workspace);
    // Claude Code compatibility: inject CLAUDE.md (AGENTS.md is auto-loaded by
    // the core) into the system prompt via prompt slots.
    let instructions = project_instructions(&workspace);
    let with_instr = |o: SessionOptions| match &instructions {
        Some(i) => o.with_prompt_slots(SystemPromptSlots::default().with_extra(i.clone())),
        None => o,
    };
    let session = match agent.resume_session(
        session_id.as_str(),
        with_instr(
            SessionOptions::new()
                .with_session_store(store.clone())
                .with_confirmation_policy(confirmation.clone())
                .with_skill_dirs(claude_dirs.clone())
                .with_auto_save(true)
                .with_auto_compact(true)
                .with_auto_compact_threshold(0.85)
                .with_file_memory(memory_dir())
                .with_max_parallel_tasks(8)
                .with_auto_delegation_enabled(true)
                .with_auto_parallel_delegation(true),
        ),
    ) {
        Ok(s) => s,
        Err(_) => agent.session(
            workspace.clone(),
            Some(with_instr(
                SessionOptions::new()
                    .with_session_store(store.clone())
                    .with_session_id(session_id.as_str())
                    .with_confirmation_policy(confirmation.clone())
                    .with_skill_dirs(claude_dirs.clone())
                    .with_auto_save(true)
                    .with_auto_compact(true)
                    .with_auto_compact_threshold(0.85)
                    .with_file_memory(memory_dir())
                    .with_max_parallel_tasks(8)
                    .with_auto_delegation_enabled(true)
                    .with_auto_parallel_delegation(true),
            )),
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
        // Mac-friendly half-page scroll (no fn key needed).
        .bind(
            KeyBinding::ctrl(KeyCode::Char('u')),
            Action::ScrollUp,
            "Scroll up",
        )
        .bind(
            KeyBinding::ctrl(KeyCode::Char('d')),
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

    let mut app = App {
        session,
        agent: agent.clone(),
        store: store.clone(),
        confirmation,
        session_id: session_id.clone(),
        models,
        relay: Vec::new(),
        relay_menu: None,
        relay_tab: 0,
        model_ctx,
        context_limit,
        last_prompt_tokens: 0,
        model_menu: None,
        model_tab: 0,
        llm_override: None,
        effort: 2, // high
        effort_panel: None,
        theme_panel: None,
        quit_armed: None,
        last_activity: Instant::now(),
        auto_reviewed: false,
        shell_mode: false,
        pending_images: Vec::new(),
        goal: None,
        loop_remaining: 0,
        active_tools: 0,
        active_agents: 0,
        subagents: Vec::new(),
        instructions,
        rainbow_until: None,
        rainbow_frame: 0,
        effort_anim: None,
        compact_summary: None,
        btw: None,
        viewport: Viewport::new(width, height.saturating_sub(7)),
        textarea: Textarea::new()
            .with_height(1)
            .with_auto_grow(8) // box grows with Shift+Enter newlines (no scroll)
            .with_width(width.saturating_sub((PAD + 2) as u16)) // PAD margin + "❯ "
            .with_submit_on_enter(true),
        spinner: Spinner::new().with_title(""),
        streaming: StreamingMarkdown::new((width as usize).saturating_sub(PAD + 2)),
        got_delta: false,
        compacting: None,
        last_paint: None,
        thinking: String::new(),
        state: State::Idle,
        messages: initial_messages,
        rx: None,
        pending_tool: None,
        approval_sel: 0,
        history: Vec::new(),
        history_pos: None,
        model: default_model,
        total_tokens: 0,
        tool_args: String::new(),
        tool_output: String::new(),
        stream_started: None,
        running_tool: None,
        blink_tick: 0,
        anim: 0,
        mode: Mode::Default,
        queue: BinaryHeap::new(),
        seq: 0,
        running_task: None,
        plan: Vec::new(),
        top: None,
        top_scroll: 0,
        top_sel: 0,
        top_kill: None,
        ide: None,
        git: None,
        help_open: false,
        completed: 0,
        branch: git_branch(&workspace),
        slash_sel: 0,
        files: workspace_files(&workspace),
        at_expanded: std::collections::HashSet::new(),
        file_sel: 0,
        skill_count: count_skill_files(&claude_dirs),
        skills: load_skills(&claude_dirs),
        disabled_skills: load_disabled_skills(),
        plugins_panel: None,
        update_available: None,
        cwd: workspace.clone(),
        width,
        height,
        keymap,
    };

    // First launch: drop the user straight into the editor on the new config.
    if created_config {
        app.messages.push(gutter(
            ACCENT,
            "Welcome to a3s code! Generated a starter ~/.a3s/config.acl — fill in your \
             provider apiKey/baseUrl + model, Ctrl+S to save, Esc to close, then restart \
             `a3s code` to load it.",
        ));
        app.open_config_in_ide(std::path::Path::new(&config_path));
        app.rebuild_viewport();
    }

    ProgramBuilder::new(app)
        .with_alt_screen()
        // Mouse capture so the wheel/trackpad scrolls the transcript (alt-screen
        // has no native scrollback). Native text selection still works while
        // holding Option (macOS/iTerm) or Shift (most terminals).
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
        // The diff code is syntax-highlighted (ANSI between tokens), so compare
        // against the ANSI-stripped text.
        let plain = strip_ansi(&out);
        assert!(plain.contains("src/x.rs"), "header has path");
        assert!(
            plain.contains("+1") && plain.contains("-1"),
            "add/del counts"
        );
        assert!(plain.contains("let a = 2;"), "shows inserted line");
        assert!(plain.contains("let a = 1;"), "shows deleted line");
        assert!(
            plain.contains("keep;"),
            "context lines are shown (unified diff)"
        );
        assert!(plain.contains("Edited src/x.rs"), "edit header with path");
    }

    /// Strip ANSI SGR sequences so tests can match the underlying text.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn non_edit_tool_renders_status_line() {
        let out = render_tool_end("bash", 0, "hello\nworld", None, None, 80);
        // Action-verb header ("Ran") + the output; no diff marker.
        assert!(out.contains("Ran") && out.contains("hello"));
        assert!(!out.contains('✎'), "no diff marker for non-edit tools");
    }

    #[test]
    fn tool_end_shows_primary_arg_summary() {
        let args = serde_json::json!({ "command": "npm test", "timeout": 60 });
        let out = render_tool_end("bash", 0, "ok\n", None, Some(&args), 80);
        assert!(out.contains("Ran"), "action verb for bash");
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

    // ---- image preview (/ide + paste) ----

    #[test]
    fn image_path_detection() {
        assert!(is_image_path(std::path::Path::new("a.PNG")));
        assert!(is_image_path(std::path::Path::new("x/y.jpeg")));
        assert!(!is_image_path(std::path::Path::new("main.rs")));
        assert!(!is_image_path(std::path::Path::new("noext")));
    }

    #[test]
    fn half_block_render_packs_two_rows_and_colors() {
        // 6px tall image -> 3 half-block rows; each row is colored ▀ cells.
        let img = ::image::DynamicImage::ImageRgba8(::image::RgbaImage::from_pixel(
            4,
            6,
            ::image::Rgba([10, 20, 30, 255]),
        ));
        let lines = render_image_blocks(&img, 80, 40);
        assert_eq!(lines.len(), 3, "6px / 2 = 3 rows");
        assert!(lines[0].contains('▀'), "uses upper half-block");
        assert!(lines[0].contains("\x1b["), "carries ANSI color");
    }

    #[test]
    fn half_block_render_fits_within_bounds() {
        let img = ::image::DynamicImage::ImageRgba8(::image::RgbaImage::new(400, 400));
        let lines = render_image_blocks(&img, 20, 10);
        assert!(lines.len() <= 10, "never exceeds max_rows");
    }

    #[test]
    fn clipboard_helper_cleans_up_on_no_image() {
        // No way to guarantee an empty clipboard, but the helper must never
        // leave a stray empty file behind when it fails.
        let dest = std::env::temp_dir().join("a3s-test-noimg.png");
        let _ = std::fs::remove_file(&dest);
        let ok = clipboard_image_to(&dest);
        if !ok {
            assert!(!dest.exists(), "failed paste leaves no file");
        } else {
            let _ = std::fs::remove_file(&dest);
        }
    }

    // ---- /ide editor cursor math (multi-byte safe) ----

    #[test]
    fn char_byte_handles_ascii_and_cjk() {
        assert_eq!(char_byte("hello", 0), 0);
        assert_eq!(char_byte("hello", 3), 3);
        assert_eq!(char_byte("hello", 5), 5); // past end clamps to len
                                              // CJK chars are 3 bytes each in UTF-8; cursor index 1 -> byte 3.
        assert_eq!(char_byte("你好", 1), 3);
        assert_eq!(char_byte("你好", 2), 6);
    }

    #[test]
    fn char_byte_supports_inplace_edits() {
        // Mirrors the /ide insert path: insert a CJK char mid-string by char idx.
        let mut s = String::from("ab");
        let b = char_byte(&s, 1);
        s.insert(b, '中');
        assert_eq!(s, "a中b");
    }

    // ---- config + skills ----

    #[test]
    fn starter_config_template_parses() {
        // First-launch generates this — it must be valid ACL with a usable model.
        let p = std::env::temp_dir().join("a3s-template-test.acl");
        std::fs::write(&p, config_template()).unwrap();
        let cfg = a3s_code_core::config::CodeConfig::from_file(&p)
            .expect("starter template must parse as valid ACL");
        let models: Vec<_> = cfg.list_models().into_iter().collect();
        assert!(!models.is_empty(), "template defines at least one model");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn counts_skill_dirs_and_flat_md() {
        let base = std::env::temp_dir().join("a3s-skillcount-test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("myskill")).unwrap();
        std::fs::write(base.join("myskill/SKILL.md"), "# skill").unwrap();
        std::fs::write(base.join("flat.md"), "# flat skill").unwrap();
        std::fs::write(base.join("notes.txt"), "ignored").unwrap();
        assert_eq!(count_skill_files(std::slice::from_ref(&base)), 2);
        let _ = std::fs::remove_dir_all(&base);
    }
}
