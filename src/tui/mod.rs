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

use std::collections::{BinaryHeap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use a3s_code_core::config::OsConfig;
use a3s_code_core::context::RecentWorkspaceFilesContextProvider;
use a3s_code_core::hitl::TimeoutAction;
use a3s_code_core::workspace::{
    LocalWorkspaceManifest, LocalWorkspaceManifestSnapshot, ManifestWorkspaceBackend,
    WorkspaceServices,
};
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

use crate::top::{collect_processes, render_process_table, ProcessRow, ProcessTableView};

mod config;
mod gitutil;
mod image;
mod kbutil;
mod memutil;
mod panels;
mod remote_ui;
mod render;
pub(crate) mod skills;
mod syntax;
mod update;
mod util;
use config::*;
use gitutil::*;
use image::*;
use memutil::*;
use render::*;
use skills::*;
use syntax::*;
use update::*;
use util::*;

/// Theme accent — OS blue. Single source of truth for the UI accent color.
// Tokyo Night palette — muted, cohesive accents used across the whole UI.
const ACCENT: Color = Color::Rgb(122, 162, 247); // soft blue (primary)
const TN_GREEN: Color = Color::Rgb(158, 206, 106);
const TN_YELLOW: Color = Color::Rgb(224, 175, 104);
const TN_RED: Color = Color::Rgb(247, 118, 142);
const TN_CYAN: Color = Color::Rgb(125, 207, 255);
const TN_ORANGE: Color = Color::Rgb(255, 158, 100);
const TN_PURPLE: Color = Color::Rgb(187, 154, 247); // magenta / purple accent
const TN_FG: Color = Color::Rgb(192, 202, 245); // body text
const TN_GRAY: Color = Color::Rgb(122, 132, 168); // completed / muted tasks

/// Self-contained system-prompt directive injected ONLY when signed in to the OS
/// platform. It disambiguates "OS" (the user means the signed-in 书安OS open
/// platform, not this machine's operating system) AND inlines exactly how to call
/// the progressive API, so the model can act immediately — without first
/// discovering/loading the `a3s-os-capabilities` skill (that extra hop is why a
/// passive catalog entry rarely triggered: the model fell back to `whoami`).
/// `base_url` is the signed-in address so the endpoint is concrete.
fn os_platform_guide(base_url: &str) -> String {
    format!(
        "[OS platform] You are signed in to the 书安OS open platform at {base_url} (via /login). \
DEFAULT RULE: while signed in, \"OS\" in the user's questions ALWAYS means THIS 书安OS platform — \
never this machine's operating system. So \"what's my OS account\", \"what modules does OS have\", \
etc. are about the platform. Answer them via the platform's progressive API; do NOT answer from \
this machine (whoami / hostname / paths / working directory describe the local box, not the \
platform — they are the WRONG answer). The endpoint and auth token are ALREADY in your shell \
environment (exported at login) — use them directly; do NOT read ~/.a3s/os-auth.json or any config \
file on each call:\n\
  curl -s -X POST \"$A3S_OS_BASE_URL/api/v1/kernel/capabilities\" \
-H \"Authorization: Bearer $A3S_OS_TOKEN\" -H 'Content-Type: application/json' \
-d '{{\"action\":\"list\"}}'\n\
Body fields: `action` = list|search|describe|execute, plus `module` / `operation` / `params`. \
Go broad→narrow: `list` (modules) → `describe`/`search` for the one operation → `execute`. \
For `list`/`search`/`describe`, pipe through `jq` to extract only the fields you need so output \
stays a few lines (e.g. `| jq -r '.data.modules[].name'`). \
For `execute`, ALWAYS add `\"shaped\":true` to the request body — that is what makes the response \
carry the `.view` popup deep-link — and do NOT jq-narrow an execute response: pipe it whole (it is \
already compact), so `.view` survives. If you strip `.view` (or omit `\"shaped\":true`), the user \
loses the 查看视图 link. \
Summarize the result for the user in a few lines; do NOT paste the whole raw JSON back. \
After your summary, ALWAYS output the trace on its own line, exactly \
`↳ requestId <requestId> · <timestamp>`. \
You do NOT print the view link yourself: whenever the execute output carries a `.view`, the host \
automatically shows a one-click `🔗 查看视图` line that opens the authenticated 渐进式UI popup \
(the user's OS login is injected, no re-login). Never print the raw URL. The \
`a3s-os-capabilities` skill has full examples."
    )
}

/// Built-in slash commands shown in the `/` menu.
const SLASH_COMMANDS: &[(&str, &str)] = &[
    (
        "/model",
        "switch model (←/→ for Claude/GPT accounts if signed in)",
    ),
    ("/init", "analyze the project and generate AGENTS.md"),
    ("/config", "edit .a3s/config.acl in your editor"),
    ("/theme", "cycle the code-highlight theme (Atom One Dark …)"),
    (
        "/workflow",
        "view the latest ultracode dynamic workflow (read-only)",
    ),
    (
        "/output",
        "view every tool call this session (name · args · result)",
    ),
    ("/login", "sign in to the configured OS account"),
    ("/logout", "sign out from the configured OS account"),
    ("/view", "open the last OS view in a native window"),
    ("/plugin", "enable/disable Claude skills & plugins"),
    ("/reload", "re-scan skills/plugins (hot-reload the / menu)"),
    ("/update", "upgrade a3s to the latest release"),
    ("/btw", "ask a background side-question (/btw <prompt>)"),
    ("/top", "live process monitor (highlights coding agents)"),
    ("/ide", "file tree + code viewer for the workspace"),
    ("/git", "git status / diff / stage / commit (gitui-style)"),
    (
        "/memory",
        "browse the agent's long-term memory (GitLens-style timeline)",
    ),
    (
        "/kb",
        "add text / a file / a folder to the knowledge base (.a3s/kb)",
    ),
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
    "/clear", "/compact", "/model", "/effort", "/goal", "/loop", "/relay", "/reload", "/update",
    "/init",
];

/// Slash commands whose name starts with `input` (input begins with `/`).
fn slash_candidates(input: &str) -> Vec<(&'static str, &'static str)> {
    SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(input))
        .copied()
        .collect()
}

/// Glyph + colour for a plan task's status.
fn task_status_style(status: a3s_code_core::planning::TaskStatus) -> (char, Color) {
    use a3s_code_core::planning::TaskStatus;
    match status {
        TaskStatus::Completed => ('✔', TN_GRAY),
        TaskStatus::InProgress => ('▶', TN_YELLOW),
        TaskStatus::Failed => ('✗', TN_RED),
        TaskStatus::Skipped | TaskStatus::Cancelled => ('⊘', TN_GRAY),
        _ => ('□', TN_GRAY), // Pending
    }
}

/// A turn that delegated work (tools / subagents / planning) but stopped without
/// a final user-facing answer should auto-synthesize one. This applies in EVERY
/// mode, not just ultracode: parallel fan-out and planning run at all efforts, so
/// the "did work, produced no answer" gap can happen anywhere (e.g. a high-effort
/// plan that fans out to subagents which return artifacts-only). Fires at most
/// once per turn (`synthesis_used`).
fn needs_synthesis(
    synthesis_inflight: bool,
    synthesis_used: bool,
    had_agent_activity: bool,
    text_after_activity: bool,
) -> bool {
    !synthesis_inflight && !synthesis_used && had_agent_activity && !text_after_activity
}

/// Rough in-flight token estimate for text that's still streaming, before the
/// provider's exact `usage` arrives on End. ASCII text averages ~4 chars/token,
/// but CJK and other wide scripts are closer to ~1 token/char — so a flat
/// `chars / 4` under-counts Chinese by 3-4× and makes the live counter lurch
/// upward when it snaps to the real number. Count the two classes separately.
fn estimate_tokens(s: &str) -> usize {
    let (ascii, wide) = s.chars().fold((0usize, 0usize), |(a, w), c| {
        if c.is_ascii() {
            (a + 1, w)
        } else {
            (a, w + 1)
        }
    });
    ascii / 4 + wide
}

/// Conservative context window assumed for models that declare no `limit.context`
/// in config. Most modern models are >= this, so the ctx% indicator and
/// auto-compaction keep working instead of silently disabling (a 0 window turns
/// both off). Declared limits always override this.
const DEFAULT_CONTEXT_LIMIT: u32 = 128_000;

/// The fixed `max_context_tokens` the core uses for its auto-compaction trigger.
/// The core compares each turn's prompt tokens against this constant and has no
/// setter for the real model window, so we compensate by scaling the threshold.
const CORE_MAX_CONTEXT_TOKENS: f32 = 200_000.0;

/// Resolve a model's usable context window: the declared limit, or a sane
/// default when it's missing/zero so context management never silently no-ops.
fn resolve_ctx_limit(raw: Option<u32>) -> u32 {
    match raw {
        Some(c) if c > 0 => c,
        _ => DEFAULT_CONTEXT_LIMIT,
    }
}

/// Scale the core's auto-compact threshold so it fires at ~85% of `window` (the
/// model's REAL context window) rather than 85% of the core's fixed 200k. For a
/// 128k model: `0.85 * 128k / 200k = 0.544` → triggers at ~108.8k (= 85% of
/// 128k). Windows above ~235k clamp to 1.0 (trigger at 200k): a touch early, but
/// it never lets the window overflow.
fn auto_compact_threshold_for(window: u32) -> f32 {
    let window = if window > 0 {
        window as f32
    } else {
        CORE_MAX_CONTEXT_TOKENS
    };
    (0.85 * window / CORE_MAX_CONTEXT_TOKENS).clamp(0.05, 1.0)
}

fn workflow_doc_for_tool(name: &str, args: Option<&serde_json::Value>) -> Option<(String, String)> {
    match name {
        "program" => {
            let src = args
                .and_then(|a| a.get("source"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())?;
            Some((
                format!("# Dynamic workflow script\n\n```javascript\n{src}\n```\n"),
                "dynamic workflow script · /workflow to view read-only".to_string(),
            ))
        }
        "parallel_task" => {
            let tasks = args
                .and_then(|a| a.get("tasks"))
                .and_then(|t| t.as_array())?
                .iter()
                .collect::<Vec<_>>();
            workflow_doc_for_tasks(&tasks, true)
        }
        "task" => {
            let args = args?;
            if let Some(tasks) = args.get("tasks").and_then(|t| t.as_array()) {
                let tasks = tasks.iter().collect::<Vec<_>>();
                workflow_doc_for_tasks(&tasks, tasks.len() > 1)
            } else {
                workflow_doc_for_tasks(&[args], false)
            }
        }
        _ => None,
    }
}

fn workflow_doc_for_tasks(
    tasks: &[&serde_json::Value],
    parallel: bool,
) -> Option<(String, String)> {
    if tasks.is_empty() {
        return None;
    }

    let mut doc = if parallel {
        format!(
            "# Dynamic workflow\n\nFanned out {} parallel subagent task(s):\n\n",
            tasks.len()
        )
    } else {
        "# Dynamic workflow\n\nDelegated subagent task(s):\n\n".to_string()
    };

    for (i, task) in tasks.iter().enumerate() {
        let desc = task
            .get("description")
            .or_else(|| task.get("prompt"))
            .or_else(|| task.get("task"))
            .and_then(|v| v.as_str())
            .unwrap_or("(task)");
        let agent = task
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("agent");
        let prompt = task
            .get("prompt")
            .or_else(|| task.get("task"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        doc.push_str(&format!(
            "## {}. {desc}\n\nAgent: `{agent}`\n\n{prompt}\n\n",
            i + 1
        ));
    }

    let label = if parallel {
        format!(
            "dynamic workflow · {} parallel tasks · /workflow to view read-only",
            tasks.len()
        )
    } else {
        format!(
            "dynamic workflow · {} delegated task{} · /workflow to view read-only",
            tasks.len(),
            if tasks.len() == 1 { "" } else { "s" }
        )
    };
    Some((doc, label))
}

/// Brand/theme colour for a coding agent, used to tag its rows and tabs.
fn agent_color(agent: &str) -> Color {
    match agent {
        "a3s-code" => ACCENT,
        "claude code" => Color::Rgb(217, 119, 87), // Claude clay
        "codex" => Color::Rgb(16, 163, 127),       // OpenAI green
        "cursor" => Color::Rgb(180, 182, 200),
        "gemini" => Color::Rgb(124, 137, 245),
        _ => TN_GRAY,
    }
}

/// Snapshot host processes for the `/top` panel via the shared `a3s top`
/// collector, so the panel and `a3s top` agree on rows, agent detection, risk,
/// CWD, and ordering (agents first, then CPU descending).
async fn fetch_top() -> Vec<ProcessRow> {
    collect_processes().await.unwrap_or_default()
}

/// One visible row of the `/ide` file tree (a flattened, expandable tree).
struct IdeEntry {
    path: std::path::PathBuf,
    name: String,
    depth: usize,
    is_dir: bool,
    expanded: bool,
}

/// Editor input mode — vim-aligned: Normal navigates/operates, Insert types.
/// Freshly opened buffers start in Normal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditMode {
    Normal,
    Insert,
}

/// An open, editable file in the `/ide` panel.
struct IdeFile {
    path: std::path::PathBuf,
    lines: Vec<String>, // text rows, or pre-rendered half-block rows if `image`
    scroll: usize,      // top visible row (vertical scroll)
    hscroll: usize,     // leftmost visible column (horizontal scroll; display columns)
    row: usize,         // cursor line
    col: usize,         // cursor column (char index)
    dirty: bool,
    image: bool,    // read-only image preview
    readonly: bool, // view-only (e.g. a dynamic-workflow artifact) — edits blocked
    mode: EditMode, // vim Normal/Insert (see `ide_key`)
    /// A pending operator/prefix awaiting its second keystroke (`d`, `c`, `g`, `y`).
    pending: Option<char>,
    /// Undo snapshots (lines + cursor) for `u`; bounded — configs are small.
    undo: Vec<(Vec<String>, usize, usize)>,
    clip: String,        // yank/delete register for p / P
    clip_linewise: bool, // register holds whole lines (dd/yy) vs an inline span
}

impl IdeFile {
    /// A freshly opened buffer: cursor at the top, Normal mode, empty undo.
    fn new(path: std::path::PathBuf, lines: Vec<String>, image: bool, readonly: bool) -> Self {
        IdeFile {
            path,
            lines: if lines.is_empty() {
                vec![String::new()]
            } else {
                lines
            },
            scroll: 0,
            hscroll: 0,
            row: 0,
            col: 0,
            dirty: false,
            image,
            readonly,
            mode: EditMode::Normal,
            pending: None,
            undo: Vec::new(),
            clip: String::new(),
            clip_linewise: false,
        }
    }
}

/// One completed tool call this session, retained for `/output`.
struct ToolCallRecord {
    name: String,
    args: Option<serde_json::Value>,
    output: String,
    exit_code: i32,
}

/// Render the `/output` viewer body: one `#n · name · status` header per call,
/// then its args and indented output. None when nothing has run.
fn format_tool_log_records(records: &[ToolCallRecord]) -> Option<String> {
    if records.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (i, rec) in records.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let status = if rec.exit_code == 0 {
            "ok".to_string()
        } else {
            format!("exit {}", rec.exit_code)
        };
        out.push_str(&format!("#{} · {} · {}\n", i + 1, rec.name, status));
        if let Some(args) = &rec.args {
            out.push_str(&format!(
                "  args: {}\n",
                serde_json::to_string(args).unwrap_or_default()
            ));
        }
        let trimmed = rec.output.trim_end();
        if !trimmed.is_empty() {
            out.push_str("  output:\n");
            for line in trimmed.lines() {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    Some(out)
}

/// The directive sent to the agent for a `?` deep-research turn: decompose the
/// question, search and read multiple sources, cross-check, and synthesize a
/// cited report. The user's query is appended.
fn deep_research_prompt(query: &str) -> String {
    format!(
        "Conduct deep research to answer the query below. Be thorough:\n\
         1. Break it into the key sub-questions worth investigating.\n\
         2. Use web search across those sub-questions, then read the most relevant \
         sources in full with web_fetch — don't rely on result snippets alone.\n\
         3. Cross-check claims across multiple independent sources; call out any \
         disagreement, uncertainty, or recency caveats.\n\
         4. Synthesize a comprehensive, well-structured answer with inline \
         citations and a final \"Sources\" list of the URLs you used.\n\n\
         Query: {query}"
    )
}

/// The persistent `/goal` north-star for a `?` deep-research task. Kept short
/// since it is prepended to every continuation turn of the long-horizon loop.
fn deep_research_goal(query: &str) -> String {
    format!("Deep research — deliver a comprehensive, well-cited report answering: {query}")
}

/// Append a one-column vertical scrollbar to the right of the viewport's visible
/// rows. The viewport is sized to `inner_width` (= screen width − 1, see
/// `relayout`) so the bar — a `│` track with an `█` thumb sized and positioned
/// from the scroll state — never clips content. The gutter stays blank when
/// nothing overflows the window.
fn append_scrollbar(view: &str, inner_width: usize, total: usize, scroll_percent: u8) -> String {
    let rows: Vec<&str> = view.split('\n').collect();
    let h = rows.len();
    let overflow = total > h && h > 0;
    // Thumb length proportional to the visible fraction; positioned by percent.
    let thumb_len = if overflow { (h * h / total).max(1) } else { 0 };
    let thumb_start = if overflow {
        (h - thumb_len) * scroll_percent as usize / 100
    } else {
        0
    };
    let track = Style::new().fg(TN_GRAY);
    let thumb = Style::new().fg(ACCENT);
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let bar = if !overflow {
                " ".to_string()
            } else if i >= thumb_start && i < thumb_start + thumb_len {
                thumb.render("█")
            } else {
                track.render("│")
            };
            format!("{}{}", pad_to(row, inner_width), bar)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The OSC 52 escape that asks the terminal to set the system clipboard to
/// `text` (base64). Works over SSH on terminals that support OSC 52. Capped so a
/// long reply can't blow past a terminal's OSC 52 size limit.
fn osc52_copy(text: &str) -> String {
    use base64::Engine;
    let capped: String = text.chars().take(64_000).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(capped.as_bytes());
    format!("\x1b]52;c;{b64}\x07")
}

/// Marker the agent puts inline in its reply to offer the RemoteUI popup. The
/// host recognises a mouse click on any reply line containing it and opens the
/// remembered view (`/view` does the same). The link lives in the message text —
/// the host renders no button of its own.
const VIEW_BUTTON_MARKER: &str = "查看视图";

/// Put `text` on the system clipboard: OSC 52 (portable, survives SSH on
/// supporting terminals) plus the native tool where we have one (macOS pbcopy).
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(osc52_copy(text).as_bytes());
    let _ = out.flush();
    #[cfg(target_os = "macos")]
    {
        if let Ok(mut child) = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
    }
}

/// Background of an active text selection in the transcript.
const SELECTION_BG: Color = Color::Rgb(58, 64, 88);

/// An in-progress mouse text-selection in the transcript viewport, in screen
/// cells (visible row, column). `anchor` = drag start, `head` = current point.
#[derive(Clone, Copy)]
struct Selection {
    anchor: (u16, u16),
    head: (u16, u16),
}

impl Selection {
    fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
    /// (top_row, top_col, bottom_row, bottom_col), as usize.
    fn ordered(&self) -> (usize, usize, usize, usize) {
        let (a, b) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        (a.0 as usize, a.1 as usize, b.0 as usize, b.1 as usize)
    }
}

/// Substring of `s` spanning visible columns `[from, to)` (wide chars counted by
/// display width). A char straddling the start is dropped; one straddling the
/// end is kept.
fn slice_cols(s: &str, from: usize, to: usize) -> String {
    let mut col = 0usize;
    let mut out = String::new();
    for ch in s.chars() {
        if col >= to {
            break;
        }
        if col >= from {
            out.push(ch);
        }
        col += a3s_tui::style::visible_len(&ch.to_string());
    }
    out
}

/// Plain text of a selection over the rendered viewport `view`: screen rows
/// `r1..=r2`, columns `[c1, c2)` clipped on the first/last rows. Rows are
/// ANSI-stripped and trailing padding trimmed.
fn selection_to_text(view: &str, r1: usize, c1: usize, r2: usize, c2: usize) -> String {
    let rows: Vec<&str> = view.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    for r in r1..=r2 {
        let Some(row) = rows.get(r) else { break };
        let plain = a3s_tui::style::strip_ansi(row);
        let from = if r == r1 { c1 } else { 0 };
        let to = if r == r2 { c2 } else { usize::MAX };
        out.push(slice_cols(&plain, from, to).trim_end().to_string());
    }
    out.join("\n")
}

/// Re-render the viewport `view` with the selected span highlighted: selected
/// rows render in plain text (no syntax colour, transiently) with the selected
/// columns on `SELECTION_BG`; other rows keep their styling.
fn highlight_selection(view: &str, r1: usize, c1: usize, r2: usize, c2: usize) -> String {
    let bg = Style::new().bg(SELECTION_BG).fg(TN_FG);
    view.split('\n')
        .enumerate()
        .map(|(i, row)| {
            if i < r1 || i > r2 {
                return row.to_string();
            }
            let plain = a3s_tui::style::strip_ansi(row);
            let from = if i == r1 { c1 } else { 0 };
            let to = if i == r2 { c2 } else { usize::MAX };
            let before = slice_cols(&plain, 0, from);
            let sel = slice_cols(&plain, from, to);
            let after = slice_cols(&plain, to, usize::MAX);
            format!("{before}{}{after}", bg.render(&sel))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// State of the `/ide` panel: the file tree, selection, and the open file.
struct Ide {
    entries: Vec<IdeEntry>,
    sel: usize,
    tree_scroll: usize,
    file: Option<IdeFile>,
    focus_editor: bool,
    /// Transient save status shown in the footer (set on Ctrl+S).
    flash: Option<String>,
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
/// last, `ultracode`, additionally plans, then fans independent work out to
/// parallel subagents via direct `parallel_task` calls.
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
type SharedManifestRx =
    Arc<Mutex<tokio::sync::broadcast::Receiver<LocalWorkspaceManifestSnapshot>>>;

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

/// Set by `/update` when an upgrade is available: after the TUI exits (terminal
/// restored), `run` performs the upgrade (Homebrew or standalone download) and
/// re-execs the freshly-installed binary.
static UPGRADE_ON_EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The latest version tag, stashed by `/update` for the post-exit upgrade.
static LATEST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

enum Msg {
    Term(Event),
    // Boxed: AgentEvent is large; keeps the Msg enum small.
    Agent(Box<AgentEvent>),
    Submit(String),
    StreamStarted(SharedRx),
    StreamEnded,
    StreamError(String),
    WorkspaceManifest(Box<LocalWorkspaceManifestSnapshot>),
    WorkspaceManifestStopped,
    SpinnerTick,
    /// Advance the welcome-mascot animation frame.
    BannerTick,
    ModalConfirm(usize),
    Resume,
    Interrupted,
    /// Output of a `!`-prefixed shell command.
    ShellOutput(String),
    /// `/update` version check finished: the latest version tag, if reachable.
    UpdatePlan(Option<String>),
    /// OS login completed.
    OsLogin(Result<String, String>),
    /// OS access token was refreshed (or refresh failed) in the background.
    OsRefreshed(Result<crate::a3s_os::StoredOsSession, String>),
    /// 书安OS unified-gateway model ids fetched for the `/model` picker.
    OsGatewayModels(Vec<String>),
    /// Answer from a `/btw` background side-thread.
    SideNote(String),
    /// Refreshed process snapshot for the `/top` panel.
    TopData(Vec<ProcessRow>),
    /// Tick to re-fetch the `/top` snapshot.
    TopRefresh,
    /// Result of the async `/relay` session scan.
    RelayData(Vec<RelaySession>),
    /// `/git` status + recent log snapshot.
    GitStatus(Vec<GitFile>, Vec<String>),
    /// `/git` diff for the selected file.
    GitDiff(Vec<String>),
    /// `/memory` timeline loaded (the store index, newest first).
    MemoryLoaded(Vec<MemEntry>),
    /// `/kb` ingest finished; carries the one-line summary to show.
    KbAdded(String),
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

fn pump_manifest(rx: SharedManifestRx) -> Cmd<Msg> {
    cmd::cmd(move || async move {
        let mut guard = rx.lock().await;
        loop {
            match guard.recv().await {
                Ok(snapshot) => return Msg::WorkspaceManifest(Box::new(snapshot)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Msg::WorkspaceManifestStopped;
                }
            }
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

fn with_recent_workspace_context(
    opts: SessionOptions,
    manifest: &Arc<LocalWorkspaceManifest>,
) -> SessionOptions {
    opts.with_context_provider(Arc::new(RecentWorkspaceFilesContextProvider::new(
        manifest.clone(),
    )))
}

fn touch_workspace_file_path_for_manifest(
    manifest: &LocalWorkspaceManifest,
    workspace: &str,
    path: &Path,
) {
    let root = Path::new(workspace);
    if let Ok(relative) = path.strip_prefix(root) {
        if let Some(path) = relative.to_str() {
            manifest.touch_file(path);
        }
    }
}

/// A running (or just-finished) parallel subagent task, for the bottom tracker.
struct SubAgent {
    task_id: String,
    agent: String,
    description: String,
    started: Instant,
    ended: Option<Instant>,
    tokens: u64,
    done: bool,
    success: Option<bool>,
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
    /// Custom LLM client to inject for signed-in account tabs; None uses config.acl.
    llm_override: Option<Arc<dyn a3s_code_core::llm::LlmClient>>,
    /// Optional OS endpoint from config.acl; enables /login and /logout.
    os_config: Option<OsConfig>,
    /// Restored OS login (from `~/.a3s/os-auth.json`, persisted across runs);
    /// `None` = signed out. Loaded on startup, set by /login, cleared by /logout.
    os_session: Option<crate::a3s_os::StoredOsSession>,
    /// True while an OS access-token refresh is in flight (guards the BannerTick
    /// trigger from spawning a second refresh before the first resolves).
    os_refreshing: bool,
    /// 书安OS unified-gateway models for the `/model` picker, lazily fetched on
    /// first `/model` while signed in. `None` = not fetched yet; `Some([])` = the
    /// gateway is unavailable/unconfigured.
    os_gateway_models: Option<Vec<String>>,
    /// Last 书安OS view seen in a tool result. RemoteUI is user-triggered: `/view`
    /// or clicking the agent's inline "查看视图" link opens it in the native
    /// a3s-webview window — it is never auto-opened.
    last_view: Option<remote_ui::ViewSpec>,
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
    /// Deep-research mode: a leading `?` turns the input into a deep-research
    /// query — sent to the agent with a multi-source research directive. Box
    /// turns cyan.
    research_mode: bool,
    /// Active transcript text-selection (mouse drag → highlight → copy on
    /// release); `None` when there's no selection.
    selection: Option<Selection>,
    /// Latest dynamic-workflow artifact (ultracode parallel_task dispatch),
    /// shown collapsed in the transcript and openable read-only via `/workflow`.
    last_workflow: Option<String>,
    /// Clipboard images pasted (Ctrl+V), sent with the next message.
    pending_images: Vec<a3s_code_core::llm::Attachment>,
    /// Persistent north-star goal (`/goal`), prepended to each prompt.
    goal: Option<String>,
    /// When the current `/goal` was set — drives the "Pursuing goal (1h 32m)"
    /// elapsed timer in the status bar. `None` whenever `goal` is `None`.
    goal_since: Option<Instant>,
    /// Remaining auto-continue turns for `/loop` (0 = off).
    loop_remaining: usize,
    /// Live parallelism for the status bar: running tools + running subagents.
    active_tools: usize,
    active_agents: usize,
    /// Parallel subagent tasks shown in the bottom tracker panel.
    subagents: Vec<SubAgent>,
    /// True once this turn used tools/planning/subagents that need a final
    /// user-facing synthesis if the model stops without text afterwards.
    turn_had_agent_activity: bool,
    /// True once assistant text arrived after the latest tool/planning/subagent
    /// activity in this turn.
    turn_text_after_activity: bool,
    /// Guard for the hidden ultracode continuation that turns raw workflow
    /// results into a final answer.
    ultracode_synthesis_inflight: bool,
    /// At most one hidden synthesis continuation per user turn.
    ultracode_synthesis_used: bool,
    /// Project instructions (CLAUDE.md/AGENT.md), injected into the system prompt.
    instructions: Option<String>,
    /// Summary of earlier conversation after a manual `/compact` (reseed).
    compact_summary: Option<String>,
    /// Shared in-memory workspace file manifest, refreshed by a background watcher.
    workspace_manifest: Arc<LocalWorkspaceManifest>,
    workspace_manifest_rx: SharedManifestRx,
    /// Manifest-backed workspace backend used by agent tools.
    workspace_services: Arc<WorkspaceServices>,
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
    /// Set while `/update` is upgrading — drives a progress bar + blocks input;
    /// on success the app restarts into the new binary.
    updating: Option<Instant>,
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
    /// Cumulative OUTPUT (generated) tokens this session — what `↓` reports.
    output_tokens: usize,
    /// Accumulated streamed JSON args of the in-progress tool call, so the
    /// result line can show what the tool actually did (command/path/pattern).
    tool_args: String,
    /// Live stdout of the in-progress tool (e.g. a running command), shown
    /// dimmed under the action and cleared when the tool completes.
    tool_output: String,
    /// Every completed tool call this session (name/args/output), shown by `/output`.
    tool_log: Vec<ToolCallRecord>,
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
    top: Option<Vec<ProcessRow>>,
    top_scroll: usize,
    top_sel: usize,
    /// `/top` agent drill-down: `Some(pid)` focuses one coding agent's process
    /// subtree (the agent root + its descendants). `None` shows all processes.
    top_focus: Option<u32>,
    /// Pending force-kill confirmation in `/top`: (pid, command label).
    top_kill: Option<(u32, String)>,
    /// `/ide` file-tree + viewer panel (Some when open).
    ide: Option<Ide>,
    /// `/git` full-screen panel (Some when open).
    git: Option<Git>,
    /// `/memory` full-screen timeline panel (Some when open).
    memory: Option<MemPanel>,
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

impl App {
    pub(crate) fn touch_workspace_file(&self, path: &str) {
        self.workspace_manifest.touch_file(path);
    }
}

impl Model for App {
    type Msg = Msg;

    fn init(&mut self) -> Option<Cmd<Msg>> {
        // Auto-check for a newer release on every launch (non-blocking).
        let mut cmds = vec![cmd::cmd(|| async {
            Msg::UpdateCheck(check_latest_version().await)
        })];
        cmds.push(pump_manifest(self.workspace_manifest_rx.clone()));
        // Heartbeat for EVERY session (fresh or resumed) — the BannerTick handler
        // self-gates the mascot animation, but it's also the sole driver of the
        // ultracode /effort confirm→apply and the idle auto-review. Resumed
        // sessions used to start no heartbeat, so neither ever fired.
        cmds.push(banner_tick());
        if self.messages.is_empty() {
            self.viewport.set_content(&self.banner());
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
                self.selection = None; // screen-coord selection is stale after resize
                self.width = width;
                self.height = height;
                self.relayout();
                self.textarea
                    .set_width(width.saturating_sub((PAD + 2) as u16));
                // Re-wrap the live answer at the new width instead of discarding it
                // (the old reset lost any text streamed before the resize).
                let raw = self.streaming.raw_content().to_string();
                self.streaming = StreamingMarkdown::new((width as usize).saturating_sub(PAD + 2));
                if !raw.is_empty() {
                    self.streaming.push(&raw);
                }
                self.rebuild_viewport();
            }

            // Bracketed paste: drop the whole pasted block into the input as
            // one edit (newlines become real line breaks) instead of N submitted
            // lines / a3s-lane queue spam — Claude-Code-style paste DX.
            Msg::Term(Event::Paste(text)) => {
                self.last_activity = Instant::now();
                self.textarea.insert_str(&text);
                self.relayout();
            }

            Msg::Term(Event::Key(key)) => {
                self.last_activity = Instant::now();
                self.auto_reviewed = false;
                self.selection = None; // any keypress dismisses the copy highlight
                                       // Ctrl+C: arm on the first press, exit on a second within 2s.
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    match self.quit_armed {
                        Some(t) if t.elapsed() < Duration::from_secs(2) => return Some(cmd::quit()),
                        _ => {
                            self.quit_armed = Some(Instant::now());
                            self.push_line(
                                &Style::new()
                                    .fg(TN_YELLOW)
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
                // /memory panel takes all keys while open.
                if self.memory.is_some() {
                    self.memory_key(&key);
                    return None;
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
                                        .arg(pid.to_string())
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
                    let rows = self.top_rows();
                    let last = rows.len().saturating_sub(1);
                    match key.code {
                        // Esc backs out of an agent focus first, then closes.
                        KeyCode::Esc => {
                            if self.top_focus.is_some() {
                                self.top_focus = None;
                                self.top_sel = 0;
                                self.top_scroll = 0;
                            } else {
                                self.top = None;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            self.top_sel = self.top_sel.saturating_sub(1)
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            self.top_sel = (self.top_sel + 1).min(last)
                        }
                        KeyCode::PageUp => self.top_sel = self.top_sel.saturating_sub(10),
                        KeyCode::PageDown => self.top_sel = (self.top_sel + 10).min(last),
                        // Enter / → drills into the selected coding agent's
                        // process subtree (its child processes).
                        KeyCode::Enter | KeyCode::Right => {
                            if self.top_focus.is_none() {
                                if let Some(row) = rows.get(self.top_sel) {
                                    if row.agent.is_some() {
                                        self.top_focus = Some(row.pid);
                                        self.top_sel = 0;
                                        self.top_scroll = 0;
                                    }
                                }
                            }
                        }
                        // Shift+K asks to force-kill the highlighted process.
                        KeyCode::Char('K') => {
                            self.top_kill =
                                rows.get(self.top_sel).map(|r| (r.pid, r.command.clone()));
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
                // /model picker takes keys while open — consume EVERY key so
                // nothing leaks to the hidden input box behind the overlay.
                if self.model_menu.is_some() {
                    return self.handle_model_key(&key).unwrap_or(None);
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
                                    .fg(TN_GREEN)
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
                // /relay picker: consume EVERY key so none leaks to the input.
                if self.relay_menu.is_some() {
                    return self.handle_relay_key(&key).unwrap_or(None);
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
                // Esc leaves shell/research mode first (discarding the partial
                // input), taking priority over the streaming interrupt below.
                if (self.shell_mode || self.research_mode) && key.code == KeyCode::Esc {
                    self.shell_mode = false;
                    self.research_mode = false;
                    self.textarea.clear();
                    return None;
                }
                // Esc interrupts the in-progress run (input stays usable otherwise).
                if self.state == State::Streaming && key.code == KeyCode::Esc {
                    self.push_line(&Style::new().fg(TN_YELLOW).render("  ⎋ interrupting…"));
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
                // A leading `!` enters shell mode, a leading `?` enters
                // deep-research mode (the prefix is stripped). Both stay on until
                // Esc or a submit (handled elsewhere).
                let val = self.textarea.value();
                if !self.shell_mode && !self.research_mode {
                    if let Some(rest) = val.strip_prefix('!') {
                        self.shell_mode = true;
                        self.textarea.set_value(rest);
                    } else if let Some(rest) = val.strip_prefix('?') {
                        self.research_mode = true;
                        self.textarea.set_value(rest);
                    }
                }
            }

            Msg::Term(Event::Mouse(m)) => {
                use a3s_tui::event::{MouseButton, MouseEventKind};
                let vp_rows = self.viewport_rows();
                // Content columns exclude the rightmost scrollbar column.
                let max_col = (self.width as usize).saturating_sub(2) as u16;
                match m.kind {
                    MouseEventKind::ScrollUp => {
                        self.selection = None;
                        self.viewport.update(ViewportMsg::ScrollUp(3));
                    }
                    MouseEventKind::ScrollDown => {
                        self.selection = None;
                        self.viewport.update(ViewportMsg::ScrollDown(3));
                    }
                    // Drag to select transcript text. Capture stays on so the wheel
                    // still scrolls; the app owns selection, so scroll + copy work
                    // together (no mode toggle). Release copies to the clipboard.
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.selection = if (m.row as usize) < vp_rows {
                            let p = (m.row, m.column.min(max_col));
                            Some(Selection { anchor: p, head: p })
                        } else {
                            None
                        };
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        if let Some(s) = self.selection.as_mut() {
                            let row = m.row.min(vp_rows.saturating_sub(1) as u16);
                            s.head = (row, m.column.min(max_col));
                        }
                    }
                    MouseEventKind::Up(MouseButton::Left) => {
                        if let Some(s) = self.selection {
                            if s.is_empty() {
                                // A plain click: open the OS view if it landed on
                                // the agent's inline "查看视图" link; else just clear.
                                let view = self.viewport.view();
                                let clicked = a3s_tui::style::strip_ansi(
                                    view.split('\n')
                                        .nth(s.anchor.0 as usize)
                                        .unwrap_or_default(),
                                );
                                self.selection = None;
                                if clicked.contains(VIEW_BUTTON_MARKER) {
                                    if let Some(spec) = self.last_view.clone() {
                                        self.open_remote_view(&spec);
                                    }
                                }
                            } else {
                                let (r1, c1, r2, c2) = s.ordered();
                                let text = selection_to_text(&self.viewport.view(), r1, c1, r2, c2);
                                if text.trim().is_empty() {
                                    self.selection = None;
                                } else {
                                    // Keep the highlight visible as "copied" feedback.
                                    copy_to_clipboard(&text);
                                }
                            }
                        }
                    }
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
                self.push_line(&Style::new().fg(TN_RED).render(&format!("  error: {e}")));
                self.loop_remaining = 0; // a failed turn stops the /loop
                self.finish();
                // Don't strand messages queued while this turn was starting.
                return self.drain_queue();
            }

            Msg::WorkspaceManifest(snapshot) => {
                self.files = snapshot.file_paths();
                self.file_sel = self.file_sel.min(self.files.len().saturating_sub(1));
                return Some(pump_manifest(self.workspace_manifest_rx.clone()));
            }

            Msg::WorkspaceManifestStopped => {
                let snapshot = self.workspace_manifest.snapshot();
                self.files = snapshot.file_paths();
                self.file_sel = self.file_sel.min(self.files.len().saturating_sub(1));
            }

            Msg::Interrupted => {
                // Esc force-aborted the turn: keep partial output, drop the
                // stream (finish() clears rx so late events are ignored), idle.
                self.finalize_streaming();
                self.push_line(&Style::new().fg(TN_YELLOW).render("  ⎋ interrupted"));
                self.loop_remaining = 0; // Esc also stops a /loop
                self.finish();
                return self.drain_queue();
            }

            Msg::Agent(event) => return self.on_agent_event(*event),

            Msg::StreamEnded => {
                // Channel closed without a normal End event (abnormal close).
                if self.state == State::Streaming {
                    self.finalize_streaming();
                }
                return self.complete_turn();
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
                    && self.memory.is_none()
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
                // Keep the OS access token fresh: refresh proactively before it
                // expires so the agent's $A3S_OS_TOKEN never goes stale mid-session.
                if !self.os_refreshing {
                    if let Some(s) = &self.os_session {
                        if crate::a3s_os::needs_refresh(s) {
                            self.os_refreshing = true;
                            let session = s.clone();
                            let refresh = cmd::cmd(move || async move {
                                Msg::OsRefreshed(
                                    crate::a3s_os::refresh_session(&session)
                                        .await
                                        .map_err(|e| e.to_string()),
                                )
                            });
                            return Some(cmd::batch(vec![banner_tick(), refresh]));
                        }
                    }
                }
                return Some(banner_tick());
            }

            Msg::AutoReview(text) => {
                if !text.trim().is_empty() {
                    // Dim + unobtrusive — it's a passive side note, not output.
                    let dim =
                        |s: &str| format!("  {}", Style::new().fg(TN_GRAY).italic().render(s));
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
                            .fg(TN_RED)
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
                        self.output_tokens = 0;
                        self.last_prompt_tokens = 0;
                        self.push_line(
                            &Style::new()
                                .fg(TN_GREEN)
                                .bold()
                                .render("  ✦ context compacted — continuing from this summary:"),
                        );
                        self.push_line(&gutter(
                            TN_CYAN,
                            self.compact_summary.as_deref().unwrap_or(""),
                        ));
                        self.rebuild_viewport();
                    }
                    Err(e) => self.push_line(
                        &Style::new()
                            .fg(TN_RED)
                            .render(&format!("  compaction failed: {e}")),
                    ),
                }
            }

            Msg::UpdateCheck(latest) => {
                let newer = latest
                    .as_deref()
                    .is_some_and(|l| !crate::update::version_ge(env!("CARGO_PKG_VERSION"), l));
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
                                .fg(TN_RED)
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
                self.push_line(&gutter(TN_GRAY, body.trim_end()));
            }

            Msg::UpdatePlan(latest) => {
                self.updating = None;
                self.relayout();
                let current = env!("CARGO_PKG_VERSION");
                match latest {
                    None => self.push_line(
                        &Style::new()
                            .fg(TN_YELLOW)
                            .render("  couldn't reach the release server — try again later"),
                    ),
                    Some(l) if crate::update::version_ge(current, &l) => self.push_line(
                        &Style::new()
                            .fg(TN_GREEN)
                            .render(&format!("  ✓ already up to date (a3s {current})")),
                    ),
                    Some(l) => {
                        // macOS/Linux self-update in place (Homebrew or a direct
                        // download); unsupported platforms get the download link.
                        if crate::update::can_self_update() {
                            if let Ok(mut g) = LATEST.lock() {
                                *g = Some(l.clone());
                            }
                            UPGRADE_ON_EXIT.store(true, std::sync::atomic::Ordering::Relaxed);
                            self.push_line(&Style::new().fg(TN_GREEN).render(&format!(
                                "  → a3s {l} available — closing to upgrade, then restarting…"
                            )));
                            return Some(cmd::quit());
                        }
                        self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                            "  → a3s {l} available — download: https://github.com/A3S-Lab/Cli/releases/latest"
                        )));
                    }
                }
            }

            Msg::OsLogin(result) => match result {
                Ok(label) => {
                    // The browser flow already saved to disk; load it into memory
                    // and rebuild so the login-gated skill activates this run.
                    self.os_session = self
                        .os_config
                        .as_ref()
                        .and_then(crate::a3s_os::current_session);
                    if let Some(s) = &self.os_session {
                        crate::a3s_os::export_os_env(s);
                    }
                    self.refresh_after_auth();
                    self.push_line(&Style::new().fg(TN_GREEN).render(&format!(
                        "  ✓ signed in to OS as {label} · capabilities skill active"
                    )));
                }
                Err(error) => self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  login failed: {error}")),
                ),
            },

            Msg::OsRefreshed(result) => {
                self.os_refreshing = false;
                match result {
                    Ok(session) => {
                        // Re-export the fresh token so the agent's $A3S_OS_TOKEN
                        // stays valid; no session rebuild needed (the skill reads
                        // the env var at call time). Stay quiet — it's routine.
                        crate::a3s_os::export_os_env(&session);
                        self.os_session = Some(session);
                    }
                    Err(_) => {
                        // Leave the existing session; the next BannerTick retries
                        // while it's still within the refresh window, and /login
                        // remains the fallback once it truly expires.
                    }
                }
            }

            Msg::OsGatewayModels(models) => {
                // Cache the fetched gateway models, then open the /model picker
                // (the "书安OS" tab now lists them — or shows it unavailable).
                self.os_gateway_models = Some(models);
                self.open_model_menu();
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
            Msg::MemoryLoaded(entries) => {
                if let Some(m) = &mut self.memory {
                    m.note = format!("{} entries", entries.len());
                    m.entries = entries;
                    m.sel = 0;
                    m.refresh_detail();
                }
            }
            Msg::KbAdded(summary) => {
                let color = if summary.starts_with('✗') {
                    TN_RED
                } else {
                    TN_GRAY
                };
                self.push_line(&Style::new().fg(color).render(&format!("  {summary}")));
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
        if let Some(m) = &self.memory {
            return self.render_memory(m);
        }
        if let Some(ide) = &self.ide {
            return self.render_ide(ide);
        }
        if self.top.is_some() {
            return self.render_top_panel();
        }
        let width = self.width as usize;
        let raw_view = self.viewport.view();
        // Paint an active text-selection over the visible rows, then add the bar.
        let shown = match &self.selection {
            Some(s) if !s.is_empty() => {
                let (r1, c1, r2, c2) = s.ordered();
                highlight_selection(&raw_view, r1, c1, r2, c2)
            }
            _ => raw_view,
        };
        let viewport_view = append_scrollbar(
            &shown,
            width.saturating_sub(1),
            self.viewport.total_lines(),
            self.viewport.scroll_percent(),
        );
        // Input mode hint: `!` = shell command (pink), `?` = deep research (cyan),
        // `/btw` = side-channel (yellow), otherwise the normal prompt (accent blue).
        let inp = self.textarea.value();
        let (sym, icolor, border): (&str, Color, Color) = if self.shell_mode {
            ("!", Color::Rgb(255, 105, 180), Color::Rgb(255, 105, 180))
        } else if self.research_mode {
            ("?", TN_CYAN, TN_CYAN)
        } else if inp.starts_with("/btw") {
            ("❯", TN_YELLOW, TN_YELLOW)
        } else {
            ("❯", ACCENT, TN_GRAY)
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
            // Context-window usage at the top-right of the input (Claude-style).
            let ctxlabel = if self.context_limit > 0 {
                let pct = (self.last_prompt_tokens * 100 / self.context_limit as usize).min(100);
                format!("{pct}% context used  ")
            } else {
                String::new()
            };
            let left = bar.saturating_sub(elabel.chars().count() + ctxlabel.chars().count() + 4);
            format!(
                "{}{} {}{} {}",
                " ".repeat(PAD),
                Style::new().fg(border).render(&"─".repeat(left)),
                Style::new().fg(TN_GRAY).render(&ctxlabel),
                Style::new().fg(ACCENT).bold().render(&elabel),
                Style::new().fg(border).render("──"),
            )
        };

        // Activity line directly above the input: spinner while the agent works,
        // an inline approval prompt while awaiting, empty when idle.
        let activity = if self.updating.is_some() {
            // The upgrade itself runs in the shell after exit (real brew
            // progress); in-TUI this is just the quick version check.
            Style::new()
                .fg(TN_GREEN)
                .render("  ⬇ checking for updates…")
        } else if let Some(t0) = self.compacting {
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
                        // Live output estimate: finalized output tokens + a
                        // CJK-aware estimate of the in-flight reasoning + answer
                        // (snaps to exact completion usage on End).
                        let est = self.output_tokens
                            + estimate_tokens(self.streaming.raw_content())
                            + estimate_tokens(&self.thinking);
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
        let typed = if sym == "!" || sym == "?" || inp.starts_with("/btw") {
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
        let dim = |s: &str| Style::new().fg(TN_GRAY).render(s);
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
            // Color by fill so the approach to the ~85% auto-compact point is visible.
            let c = if pct >= 85 {
                TN_RED
            } else if pct >= 70 {
                TN_YELLOW
            } else {
                TN_GRAY
            };
            line1.push_str(&format!(
                " {}",
                Style::new().fg(c).render(&format!("ctx:{pct}%"))
            ));
        } else if self.output_tokens > 0 {
            line1.push_str(&format!(" {}", dim(&format!("{} tok", self.output_tokens))));
        }
        // Live chips, only when active.
        if self.goal.is_some() {
            let elapsed = self
                .goal_since
                .map(|t| format!(" ({})", fmt_elapsed(t.elapsed())))
                .unwrap_or_default();
            line1.push_str(&format!(
                "  {}",
                Style::new().fg(TN_CYAN).render(&format!("🎯 Pursuing goal{elapsed}"))
            ));
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
        // Plan/TODO panel stays pinned above the input.
        let plan = self.plan_lines();
        let plan_block = plan.join("\n");
        // Parallel-subagent tracker is pinned at the very bottom, below the
        // status bar (not above the input) so it doesn't push the prompt around.
        let subs = self.subagent_lines();
        let sub_block = subs.join("\n");
        let composed = Layout::vertical()
            .item(&viewport_view, Constraint::Fill)
            .item(&spacer, Constraint::Fixed(1))
            .item(&activity, Constraint::Fixed(1))
            .item(&plan_block, Constraint::Fixed(plan.len() as u16))
            .item(&top_separator, Constraint::Fixed(1))
            .item(&input_view, Constraint::Fixed(self.input_height()))
            .item(&separator, Constraint::Fixed(1))
            .item(&status1, Constraint::Fixed(1))
            .item(&status2, Constraint::Fixed(1))
            .item(&sub_block, Constraint::Fixed(subs.len() as u16))
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
            || self.memory.is_some()
            || self.help_open
        {
            return None;
        }
        // Below the input: separator + 2 status lines + the subagent panel +
        // the task panel. The input spans `input_height` rows; cursor on its row.
        let below = 3 + self.subagent_lines().len() as u16 + self.task_lines().len() as u16;
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
        // No input while compacting or upgrading.
        if self.compacting.is_some() || self.updating.is_some() {
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
        // Deep-research mode (`?`) is a long-horizon task: it anchors the work
        // with the `/goal` mechanism (a persistent north-star prepended to every
        // turn) AND auto-continues via the `/loop` mechanism until the agent
        // reports completion (or Esc). The first turn carries the full
        // decompose → search + read → cross-check → synthesize directive.
        if self.research_mode {
            self.research_mode = false;
            let query = trimmed.trim_start_matches('?').trim().to_string();
            if query.is_empty() {
                self.textarea.clear();
                return None;
            }
            self.history.push(trimmed.to_string());
            self.history_pos = None;
            self.textarea.clear();
            self.goal = Some(deep_research_goal(&query));
            self.goal_since = Some(Instant::now());
            self.messages.push(gutter(
                TN_CYAN,
                &Style::new()
                    .bold()
                    .render(&format!("🔬 deep research: {query}")),
            ));
            self.push_line(&Style::new().fg(TN_GRAY).render(
                "  🎯 goal set · ↻ auto-continues until done (Esc stops · /goal clear drops it)",
            ));
            let prompt = deep_research_prompt(&query);
            let display = format!("🔬 {query}");
            // Long-horizon budget: keep researching across turns toward the goal.
            self.loop_remaining = 8;
            if self.state == State::Idle {
                return self.start_stream_inner(prompt, display, true, true, false);
            }
            self.seq += 1;
            self.queue.push(Queued {
                prio: 1,
                seq: self.seq,
                text: prompt,
            });
            self.push_line(&Style::new().fg(TN_GRAY).render("    ⋯ queued"));
            self.relayout();
            return None;
        }
        // Block session-mutating commands while a turn is streaming.
        if self.state != State::Idle {
            let cmd0 = trimmed.split_whitespace().next().unwrap_or("");
            if IDLE_ONLY.contains(&cmd0) {
                self.textarea.clear();
                self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                    "  {cmd0} is unavailable while a turn is running — press Esc to stop first"
                )));
                return None;
            }
        }
        if let Some(rest) = trimmed.strip_prefix("/login") {
            if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
                // e.g. "/login-token" is not the /login command.
            } else {
                self.textarea.clear();
                let Some(os_config) = self.os_config.clone() else {
                    self.push_line(&format!(
                        "{}\n{}\n{}\n{}",
                        Style::new()
                            .fg(TN_YELLOW)
                            .render("  /login needs an OS endpoint, but none is configured."),
                        Style::new().fg(TN_GRAY).render(
                            "  Add it to ~/.a3s/config.acl (or your project's .a3s/config.acl):"
                        ),
                        Style::new()
                            .fg(TN_CYAN)
                            .render("      os = \"https://your-os-host.example.com\""),
                        Style::new()
                            .fg(TN_GRAY)
                            .render("  then restart a3s code and run /login again."),
                    ));
                    return None;
                };
                let token = rest.trim();
                if !token.is_empty() {
                    match crate::a3s_os::login_with_token(&os_config, token) {
                        Ok(session) => {
                            let label = session.display_label();
                            crate::a3s_os::export_os_env(&session);
                            self.os_session = Some(session);
                            self.refresh_after_auth();
                            self.push_line(&Style::new().fg(TN_GREEN).render(&format!(
                                "  ✓ signed in to OS as {label} · capabilities skill active"
                            )));
                        }
                        Err(error) => self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render(&format!("  login failed: {error}")),
                        ),
                    }
                    return None;
                }

                // Already signed in (restored from a previous run) → no need to
                // re-authenticate; tell the user how to switch instead.
                if let Some(s) = &self.os_session {
                    self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                        "  already signed in to OS as {} · /logout to switch accounts",
                        s.display_label()
                    )));
                    return None;
                }

                self.push_line(
                    &Style::new()
                        .fg(TN_GRAY)
                        .render("  opening OS login in your browser…"),
                );
                return Some(cmd::cmd(move || async move {
                    let result = crate::a3s_os::login_via_browser(os_config)
                        .await
                        .map(|session| session.display_label())
                        .map_err(|error| error.to_string());
                    Msg::OsLogin(result)
                }));
            }
        }
        if trimmed == "/logout" {
            self.textarea.clear();
            let Some(os_config) = self.os_config.clone() else {
                self.push_line(&Style::new().fg(TN_YELLOW).render(
                    "  configure `os = \"https://...\"` in .a3s/config.acl to enable /logout",
                ));
                return None;
            };
            match crate::a3s_os::logout(&os_config) {
                Ok(true) => {
                    self.os_session = None;
                    crate::a3s_os::remove_capability_skill_dir();
                    crate::a3s_os::clear_os_env();
                    self.refresh_after_auth();
                    self.push_line(
                        &Style::new()
                            .fg(TN_GREEN)
                            .render("  ✓ signed out from OS · capabilities skill removed"),
                    );
                }
                Ok(false) => {
                    self.os_session = None;
                    crate::a3s_os::remove_capability_skill_dir();
                    crate::a3s_os::clear_os_env();
                    self.refresh_after_auth();
                    self.push_line(&Style::new().fg(TN_GRAY).render("  no OS login was stored"));
                }
                Err(error) => self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  logout failed: {error}")),
                ),
            }
            return None;
        }
        // `/kb <text | file | folder>` ingests raw material into the project
        // knowledge base (.a3s/kb/sources/). Deterministic file I/O off the UI
        // thread; `/okf` later compiles the sources into OKF concept pages.
        if let Some(rest) = trimmed.strip_prefix("/kb") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                let arg = rest.trim().to_string();
                self.textarea.clear();
                let cwd = self.cwd.clone();
                let now = chrono::Utc::now().to_rfc3339();
                return Some(cmd::cmd(move || async move {
                    let summary =
                        tokio::task::spawn_blocking(move || kbutil::add_to_kb(&cwd, &arg, &now))
                            .await
                            .unwrap_or_else(|e| format!("✗ /kb failed: {e}"));
                    Msg::KbAdded(summary)
                }));
            }
        }
        // `/btw <prompt>` runs a background side-thread (separate ephemeral
        // session, the main conversation as context) without disturbing the
        // current turn; its answer arrives as a side note.
        if let Some(rest) = trimmed.strip_prefix("/btw") {
            let q = rest.trim().to_string();
            self.textarea.clear();
            if q.is_empty() {
                self.push_line(&Style::new().fg(TN_GRAY).render("  usage: /btw <question>"));
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
                        TN_CYAN,
                        &format!("🎯 goal: {cur}   (/goal clear to remove)"),
                    )),
                    None => self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  usage: /goal <what you're working toward>"),
                    ),
                }
            } else if g == "clear" {
                self.goal = None;
                self.goal_since = None;
                self.push_line(&Style::new().fg(TN_GRAY).render("  goal cleared"));
                return None;
            } else {
                // Set the persistent goal AND start working toward it now (the
                // goal is prepended to this and every later prompt).
                self.goal = Some(g.to_string());
                self.goal_since = Some(Instant::now());
                self.push_line(&gutter(TN_CYAN, &format!("🎯 goal set: {g}")));
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
                    &Style::new().fg(TN_GRAY).render(
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
                // Actually reset the conversation, not just the screen: swap in a
                // fresh session (new id, no history, no carried compact summary)
                // and zero the token/ctx counters. /clear is idle-only (guarded
                // above), so replacing the session is safe. Set the id first
                // (rebuild_session keys off it) and revert it if the rebuild fails
                // so id and session never desync.
                let prev_id = std::mem::replace(&mut self.session_id, new_session_id());
                let model = self.model.clone();
                match self.rebuild_session(model.as_deref()) {
                    Ok((s, _)) => {
                        self.session = Arc::new(s);
                        self.compact_summary = None;
                        self.output_tokens = 0;
                        self.last_prompt_tokens = 0;
                    }
                    Err(_) => self.session_id = prev_id,
                }
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
                            .fg(TN_YELLOW)
                            .render("  finish the current turn before compacting"),
                    );
                    return None;
                }
                let history = self.session.history();
                if history.is_empty() {
                    self.push_line(&Style::new().fg(TN_GRAY).render("  nothing to compact yet"));
                    return None;
                }
                self.compacting = Some(Instant::now()); // progress bar + input lock
                let agent = self.agent.clone();
                let workspace = self.cwd.clone();
                // Re-compacting must subsume the PRIOR summary — it lives in the
                // system prompt, not in `history`, so without this everything
                // before the last /compact would be dropped from the new summary.
                let prompt = match &self.compact_summary {
                    Some(prev) => format!(
                        "An earlier part of this conversation was already condensed into this \
                         summary:\n\n{prev}\n\nProduce a SINGLE updated summary that fully \
                         incorporates the summary above AND the conversation history below, so a \
                         fresh session can continue seamlessly: the goal, key decisions, \
                         files/commands touched, current state, and the immediate next steps. Be \
                         thorough but compact."
                    ),
                    None => "Summarize this conversation so a fresh session can continue \
                         seamlessly: the goal, key decisions, files/commands touched, current \
                         state, and the immediate next steps. Be thorough but compact."
                        .to_string(),
                };
                return Some(cmd::cmd(move || async move {
                    let conf = a3s_code_core::hitl::ConfirmationPolicy::enabled()
                        .with_timeout(500, TimeoutAction::Reject);
                    let mut summary = String::new();
                    if let Ok(sess) = agent.session(
                        workspace,
                        Some(SessionOptions::new().with_confirmation_policy(conf)),
                    ) {
                        if let Ok((mut rx, _j)) = sess.stream(&prompt, Some(&history)).await {
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
            "/view" => {
                self.textarea.clear();
                if let Some(spec) = self.last_view.clone() {
                    self.open_remote_view(&spec);
                } else {
                    self.push_line(&Style::new().fg(TN_GRAY).render(
                        "  no OS view yet — run an OS query that returns a viewUrl, then /view",
                    ));
                }
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
                            .fg(TN_YELLOW)
                            .render("  could not locate a home directory for ~/.a3s/config.acl"),
                    ),
                }
                return None;
            }
            "/model" => {
                self.textarea.clear();
                // Signed in to 书安OS + gateway models not fetched yet → fetch them
                // once (OpenAI-compatible /v1/models) so the picker can offer the
                // unified gateway, then open. Otherwise open immediately.
                if let Some(s) = self.os_session.clone() {
                    if self.os_gateway_models.is_none() {
                        let (addr, token) = (s.address.clone(), s.access_token.clone());
                        return Some(cmd::cmd(move || async move {
                            Msg::OsGatewayModels(
                                crate::a3s_os::fetch_gateway_models(&addr, &token).await,
                            )
                        }));
                    }
                }
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
                self.top_focus = None;
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
                    flash: None,
                });
                return None;
            }
            "/plugin" | "/plugins" => {
                self.textarea.clear();
                if self.skills.is_empty() {
                    self.push_line(&Style::new().fg(TN_GRAY).render(
                        "  no skills/plugins found (~/.claude/skills, ~/.codex/skills, ~/.claude/plugins)",
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
            "/workflow" => {
                self.textarea.clear();
                match self.last_workflow.clone() {
                    Some(doc) => self.open_readonly_in_ide("dynamic-workflow.md", &doc),
                    None => self.push_line(&Style::new().fg(TN_GRAY).render(
                        "  no dynamic workflow yet — run an ultracode task that fans out via parallel_task",
                    )),
                }
                return None;
            }
            "/output" => {
                self.textarea.clear();
                match self.format_tool_log() {
                    Some(content) => self.open_readonly_in_ide("tool-calls.txt", &content),
                    None => self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  no tool calls yet this session"),
                    ),
                }
                return None;
            }
            "/reload" => {
                self.textarea.clear();
                // Hot-reload: re-discover skill dirs, refresh the UI catalog,
                // and rebuild the session so the core skill registry and
                // next Claude/system prompt see the same skills.
                let dirs = agent_skill_dirs(&self.cwd);
                self.skills = load_skills(&dirs);
                self.skill_count = count_skill_files(&dirs);
                let model = self.model.clone();
                match self.rebuild_session(model.as_deref()) {
                    Ok((session, _)) => {
                        self.session = Arc::new(session);
                        self.push_line(&Style::new().fg(TN_GREEN).render(&format!(
                            "  ↻ reloaded — {} skills available",
                            self.skills.len()
                        )));
                    }
                    Err(error) => {
                        self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render(&format!("  reload failed: {error}")),
                        );
                    }
                }
                return None;
            }
            "/update" => {
                self.textarea.clear();
                self.updating = Some(Instant::now()); // "checking…" + input lock
                self.relayout();
                return Some(cmd::cmd(|| async {
                    // Quick version check only; the actual upgrade runs in the
                    // shell after the TUI exits (run()), so brew's/curl's own
                    // progress shows and the restart picks up the new binary.
                    let latest = tokio::task::spawn_blocking(crate::update::fetch_latest)
                        .await
                        .ok()
                        .flatten();
                    Msg::UpdatePlan(latest)
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
            "/memory" => {
                self.textarea.clear();
                // Open immediately ("loading…"); parse the store index off the UI
                // thread (it can be multi-MB) so the panel never janks on open.
                let dir = memory_dir();
                self.memory = Some(MemPanel {
                    entries: Vec::new(),
                    sel: 0,
                    detail: memutil::MemDetail::default(),
                    detail_scroll: 0,
                    dir: dir.clone(),
                    note: "loading…".into(),
                });
                return Some(cmd::cmd(move || async move {
                    let entries = tokio::task::spawn_blocking(move || memutil::load_timeline(&dir))
                        .await
                        .unwrap_or_default();
                    Msg::MemoryLoaded(entries)
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
            self.push_line(&Style::new().fg(TN_GRAY).render("    ⋯ queued"));
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
                    .fg(TN_YELLOW)
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
        self.start_stream_inner(prompt.clone(), prompt, true, true, false)
    }

    fn start_ultracode_synthesis(
        &mut self,
        prompt: String,
        display_task: String,
    ) -> Option<Cmd<Msg>> {
        self.ultracode_synthesis_used = true;
        self.push_line(&Style::new().fg(TN_GRAY).render("  ⇉ synthesizing results…"));
        self.start_stream_inner(prompt, display_task, false, false, true)
    }

    fn start_stream_inner(
        &mut self,
        prompt: String,
        display_task: String,
        clear_turn_artifacts: bool,
        include_attachments: bool,
        synthesis: bool,
    ) -> Option<Cmd<Msg>> {
        self.streaming.clear();
        self.got_delta = false; // track if this turn streamed any text deltas
        self.turn_had_agent_activity = false;
        self.turn_text_after_activity = false;
        self.ultracode_synthesis_inflight = synthesis;
        if !synthesis {
            self.ultracode_synthesis_used = false;
        }
        self.last_paint = None; // first delta of the turn paints immediately
        self.viewport.set_auto_scroll(true); // sending a message jumps to latest
        if clear_turn_artifacts {
            self.plan.clear(); // fresh plan per user turn; planning events refill it
            self.subagents.clear(); // keep completed agents visible until the next user turn
        }
        self.running_task = Some(display_task);
        self.state = State::Streaming;
        self.relayout();
        self.stream_started = Some(Instant::now());
        self.spinner.start();
        self.rebuild_viewport();
        let session = self.session.clone();
        let atts = if include_attachments {
            std::mem::take(&mut self.pending_images)
        } else {
            Vec::new()
        };
        // Keep the agent aligned with the standing goal (display stays clean).
        let prompt = match &self.goal {
            Some(g) => format!("[Ongoing goal: {g}]\n\n{prompt}"),
            None => prompt,
        };
        // ultracode no longer rewrites the user turn. Whether a turn plans and
        // fans out is decided by the core's message-gated planning
        // (PlanningMode::Auto) plus the `parallel_task` tool description — not an
        // unconditional per-turn imperative, which made even "hi" trigger a plan
        // and workspace exploration.
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

    /// Shared turn-completion: count the turn, run any ultracode synthesis, go
    /// idle, then either continue a `/loop` or drain the next queued message.
    /// Called from BOTH the normal `AgentEvent::End` arm (the happy path, which
    /// returns without re-pumping so `StreamEnded` never fires) and the
    /// `StreamEnded` channel-closed arm — previously this lived only in
    /// `StreamEnded`, so on success the queue never drained and `/loop` ran once.
    fn complete_turn(&mut self) -> Option<Cmd<Msg>> {
        if self.state == State::Streaming {
            self.completed += 1;
        }
        let synthesis = self.prepare_ultracode_synthesis();
        self.finish();
        if let Some((prompt, display_task)) = synthesis {
            return self.start_ultracode_synthesis(prompt, display_task);
        }
        // /loop: auto-continue until the agent says DONE, the cap is hit, or Esc.
        // Queued user messages take priority.
        if self.loop_remaining > 0 && self.queue.is_empty() {
            self.loop_remaining -= 1;
            let n = self.loop_remaining;
            self.push_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("  ↻ loop ({n} left · Esc to stop)")),
            );
            return Some(cmd::msg(Msg::Submit(
                "Continue. If the task is fully complete, reply DONE and stop.".to_string(),
            )));
        }
        // Run the next queued message (submitted while busy), if any.
        self.drain_queue()
    }

    fn on_agent_event(&mut self, event: AgentEvent) -> Option<Cmd<Msg>> {
        // After an interrupt, rx is cleared — ignore any late buffered events.
        self.rx.as_ref()?;
        match event {
            AgentEvent::TextDelta { text } => {
                self.mark_assistant_text(&text);
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
                self.mark_agent_activity();
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
                self.mark_agent_activity();
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
                self.capture_workflow(&name, args.as_ref());
                // RemoteUI: a 书安OS viewUrl in the tool output is openable. Remember
                // it for `/view`, and if the API marked it embeddable (sized popup),
                // open it now in the native a3s-webview window (auth via $A3S_OS_TOKEN).
                // The progressive API returns a RELATIVE view url; complete it
                // against the signed-in OS origin (the TUI is "the edge").
                let os_origin = self
                    .os_session
                    .as_ref()
                    .map(|s| crate::a3s_os::os_origin(&s.address));
                if let Some(spec) = remote_ui::find_view_url(&output, os_origin.as_deref()) {
                    // RemoteUI is user-triggered — never auto-open. Remember the
                    // view for `/view`, and surface a clickable "查看视图" line
                    // ourselves (deterministic) rather than trusting the model to
                    // print the marker — weaker models often forget it or jq the
                    // `.view` object away. Only emit for a NEW view (no dupes).
                    let is_new = self.last_view.as_ref() != Some(&spec);
                    self.last_view = Some(spec);
                    if is_new {
                        self.push_line(&gutter(
                            TN_CYAN,
                            &format!("🔗 {VIEW_BUTTON_MARKER}  (click or /view to open)"),
                        ));
                    }
                }
                // Retain the call for `/output`. Cap each output so a huge build
                // log can't bloat the in-memory record.
                // ponytail: 8 KB/call cap; the transcript already holds the full text
                let logged = if output.len() > 8192 {
                    let mut s: String = output.chars().take(8000).collect();
                    s.push_str("\n… (output truncated — see transcript)");
                    s
                } else {
                    output
                };
                self.tool_log.push(ToolCallRecord {
                    name,
                    args,
                    output: logged,
                    exit_code,
                });
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
                self.mark_agent_activity();
                self.finalize_streaming();
                self.active_agents += 1;
                // Track it in the live bottom panel instead of a transcript line.
                self.subagents.push(SubAgent {
                    task_id,
                    agent,
                    description,
                    started: Instant::now(),
                    ended: None,
                    tokens: 0,
                    done: false,
                    success: None,
                });
                self.relayout();
            }
            AgentEvent::SubagentProgress {
                task_id, metadata, ..
            } => {
                self.mark_agent_activity();
                // Per-child OUTPUT tokens for the panel's `↓`. Each child turn-end
                // reports that turn's completion_tokens once, so SUM them across
                // turns (tool-event progress carries no usage, so it won't add).
                // The old code took max(total_tokens), i.e. the largest single
                // turn's prompt+completion ≈ the child's context size, not output.
                let toks = metadata
                    .get("completion_tokens")
                    .or_else(|| metadata.pointer("/usage/completion_tokens"))
                    .and_then(|v| v.as_u64());
                if let Some(s) = self.subagents.iter_mut().find(|s| s.task_id == task_id) {
                    if let Some(t) = toks {
                        s.tokens += t;
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
                self.mark_agent_activity();
                self.active_agents = self.active_agents.saturating_sub(1);
                if let Some(s) = self.subagents.iter_mut().find(|s| s.task_id == task_id) {
                    s.done = true;
                    s.success = Some(success);
                    s.ended = Some(Instant::now());
                }
                self.relayout();
                let (mark, color) = if success {
                    ("✓", TN_GREEN)
                } else {
                    ("✗", TN_RED)
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
            AgentEvent::ContextCompacted {
                before_messages,
                after_messages,
                percent_before,
                ..
            } => {
                // The core auto-compacted mid-turn (pruned tool outputs + summarized
                // old messages). The next turn's prompt reflects the smaller context,
                // so ctx% self-corrects on the following End — just surface a note.
                let pct = (percent_before * 100.0).round() as u32;
                self.push_line(&Style::new().fg(TN_GRAY).italic().render(&format!(
                    "  ✦ context auto-compacted at {pct}% · {before_messages} → {after_messages} messages"
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
                    // a line per tool is just noise. Do NOT start another
                    // spinner_tick here — the turn's tick loop is already running
                    // (state stays Streaming through auto-approval). Stacking one
                    // per auto-approved tool made the spinner advance several
                    // frames per 80ms = the "时快时慢" speed-up.
                    let session = self.session.clone();
                    return Some(cmd::cmd(move || async move {
                        let _ = session.confirm_tool_use(&tool_id, true, None).await;
                        Msg::Resume
                    }));
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
                    self.mark_assistant_text(&text);
                    self.streaming.push(&text);
                }
                self.finalize_streaming();
                // `↓` counts OUTPUT (generated) tokens. Summing total_tokens per
                // turn re-counts the whole context every turn (the prompt is
                // re-sent each round) and balloons far past what was generated.
                // completion_tokens is the output; fall back to total-prompt if a
                // provider omits it.
                self.output_tokens += if usage.completion_tokens > 0 {
                    usage.completion_tokens
                } else {
                    usage.total_tokens.saturating_sub(usage.prompt_tokens)
                };
                // Latest prompt size = how full the context window is (for ctx%).
                if usage.prompt_tokens > 0 {
                    self.last_prompt_tokens = usage.prompt_tokens;
                }
                if self.model.is_none() {
                    self.model = meta.and_then(|m| m.response_model.or(m.request_model));
                }
                // Count the turn, idle, then continue /loop or drain the queue.
                return self.complete_turn();
            }
            AgentEvent::Error { message } => {
                self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  error: {message}")),
                );
                self.loop_remaining = 0; // a failed turn stops the /loop
                self.finish();
                // Don't strand messages queued while this turn was running.
                return self.drain_queue();
            }
            // Planning mode: capture the plan and live task-status updates for
            // the pinned TODO panel above the input.
            AgentEvent::PlanningEnd { plan, .. } => {
                self.mark_agent_activity();
                self.set_plan(&plan.steps);
            }
            AgentEvent::TaskUpdated { tasks, .. } => {
                self.mark_agent_activity();
                self.set_plan(&tasks);
            }
            // Per-step lifecycle also drives the panel, in case TaskUpdated is
            // sparse: a step turns ▶ on start and ✔/✗/⊘ on completion.
            AgentEvent::StepStart { step_id, .. } => {
                self.mark_agent_activity();
                self.set_task_status(&step_id, '▶', TN_YELLOW);
            }
            AgentEvent::StepEnd {
                step_id, status, ..
            } => {
                self.mark_agent_activity();
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

    fn mark_agent_activity(&mut self) {
        self.turn_had_agent_activity = true;
        self.turn_text_after_activity = false;
    }

    fn mark_assistant_text(&mut self, text: &str) {
        if !text.trim().is_empty() {
            self.turn_text_after_activity = true;
        }
    }

    fn prepare_ultracode_synthesis(&self) -> Option<(String, String)> {
        if !needs_synthesis(
            self.ultracode_synthesis_inflight,
            self.ultracode_synthesis_used,
            self.turn_had_agent_activity,
            self.turn_text_after_activity,
        ) {
            return None;
        }

        let user_task = self
            .running_task
            .as_deref()
            .filter(|task| !task.trim().is_empty())
            .unwrap_or("the previous task");
        let mut prompt = format!(
            "[synthesis]\n\
             The previous turn completed planning/tool/subagent work \
             but stopped without a final user-facing answer.\n\n\
             Original user task:\n{user_task}\n\n\
             Write the final answer now in the user's language. Synthesize the \
             completed work into a useful response. Do not call tools or start \
             more subagents unless it is strictly necessary to avoid an incorrect \
             answer. If a child run produced no text output, summarize the \
             available plan/status instead of exposing raw task metadata.\n"
        );

        if !self.plan.is_empty() {
            prompt.push_str("\nPlan/status:\n");
            for (_, text, glyph, _) in &self.plan {
                let status = match glyph {
                    '✔' => "done",
                    '▶' => "in progress",
                    '✗' => "failed",
                    _ => "pending",
                };
                prompt.push_str(&format!("- [{status}] {text}\n"));
            }
        }

        if !self.subagents.is_empty() {
            prompt.push_str("\nSubagents:\n");
            for agent in &self.subagents {
                let status = match agent.success {
                    Some(true) => "done",
                    Some(false) => "failed",
                    None if agent.done => "done",
                    None => "unknown",
                };
                prompt.push_str(&format!(
                    "- [{status}] {}: {}\n",
                    agent.agent, agent.description
                ));
            }
        }

        if let Some(workflow) = &self.last_workflow {
            prompt.push_str("\nLatest workflow artifact excerpt:\n");
            prompt.push_str(&truncate(workflow, 4000));
            prompt.push('\n');
        }

        Some((prompt, user_task.to_string()))
    }

    fn finalize_streaming(&mut self) {
        let rendered = self.streaming.view();
        if !rendered.trim().is_empty() {
            let block = gutter(TN_GREEN, &rendered);
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
        // Clear the parallel-subagent panel when the turn ends — it's a live
        // progress tracker, so leaving completed agents pinned at the bottom once
        // the work is done just clutters the idle screen.
        self.subagents.clear();
        self.ultracode_synthesis_inflight = false;
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

    /// Open a 书安OS viewUrl in the native `a3s-webview` window. Silent on success
    /// (the window appearing is feedback enough); only a missing helper binary
    /// leaves a transcript hint.
    fn open_remote_view(&mut self, spec: &remote_ui::ViewSpec) {
        if remote_ui::open_window(spec).is_err() {
            // No helper binary (not shipped on Linux/Windows, or not installed):
            // print the url so the user can open it in a browser themselves.
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  🔗 open in your browser: {} (install a3s-webview for an in-app window, macOS)",
                spec.url
            )));
        }
    }

    /// Skill dirs for the session: the discovered Claude/Codex dirs plus the
    /// login-gated built-in OS `a3s-os-capabilities` skill when signed in.
    pub(crate) fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        let mut dirs = agent_skill_dirs(&self.cwd);
        // Always-available built-in skills (the `okf` LLM-wiki / knowledge compiler).
        if let Some(d) = ensure_builtin_skills_dir() {
            dirs.push(d);
        }
        if self.os_session.is_some() {
            if let Some(cfg) = &self.os_config {
                if let Some(d) = crate::a3s_os::ensure_capability_skill_dir(cfg) {
                    dirs.push(d);
                }
            }
        }
        dirs
    }

    /// After an OS login/logout, rebuild the session so the login-gated
    /// skill loads/unloads immediately, and refresh the start-screen skill list.
    fn refresh_after_auth(&mut self) {
        if self.state == State::Idle {
            if let Ok((s, _)) = self.rebuild_session(self.model.as_deref()) {
                self.session = Arc::new(s);
            }
        }
        let dirs = self.skill_dirs();
        self.skill_count = count_skill_files(&dirs);
        self.skills = load_skills(&dirs);
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
            file: Some(IdeFile::new(path.to_path_buf(), lines, false, false)),
            focus_editor: true,
            flash: None,
        });
    }

    /// Capture a `parallel_task`/`task` dispatch as a dynamic-workflow artifact:
    /// a readable plan of the fanned-out subtasks. Stored for `/workflow` and
    /// announced with a collapsed one-line message in the transcript.
    fn capture_workflow(&mut self, name: &str, args: Option<&serde_json::Value>) {
        let Some((doc, label)) = workflow_doc_for_tool(name, args) else {
            return;
        };
        self.last_workflow = Some(doc);
        // Collapsed indicator; the full artifact opens read-only via /workflow.
        self.push_line(&Style::new().fg(ACCENT).render(&format!("  ⊞ {label}")));
    }

    /// Open read-only text content in the built-in IDE (used by `/workflow` to
    /// show the dynamic-workflow artifact). Editor-focused for scroll/nav, but
    /// `readonly` blocks edits and Ctrl+S.
    fn open_readonly_in_ide(&mut self, title: &str, content: &str) {
        let lines: Vec<String> = content.lines().map(String::from).collect();
        self.ide = Some(Ide {
            entries: ide_children(std::path::Path::new(&self.cwd), 0),
            sel: 0,
            tree_scroll: 0,
            file: Some(IdeFile::new(
                std::path::PathBuf::from(title),
                lines,
                false,
                true,
            )),
            focus_editor: true,
            flash: Some("read-only".to_string()),
        });
    }

    /// Format every retained tool call for the `/output` viewer: a header line
    /// per call (index · name · status) followed by its args and output. Returns
    /// None when nothing has run yet.
    fn format_tool_log(&self) -> Option<String> {
        format_tool_log_records(&self.tool_log)
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
            // Lay out reasoning like every other message: pre-wrap to the content
            // width and put the margin + "💭" OUTSIDE the dim style, one styled
            // line at a time. The old `Style::render(&indent(…))` shoved the whole
            // paragraph in as one line whose leading spaces sat *inside* the ANSI
            // escape, so the viewport re-wrapped it to the screen edge (margins
            // didn't line up) with uneven spacing. "💭 " is 3 display columns;
            // continuation lines indent to match.
            let dim = Style::new().fg(TN_GRAY).italic();
            let margin = " ".repeat(PAD);
            let avail = (self.width as usize).saturating_sub(PAD + 3).max(8);
            let body = wrap_words(self.thinking.trim(), avail)
                .iter()
                .enumerate()
                .map(|(i, line)| {
                    let lead = if i == 0 { "💭 " } else { "   " };
                    format!("{margin}{}", dim.render(&format!("{lead}{line}")))
                })
                .collect::<Vec<_>>()
                .join("\n");
            blocks.push(body);
        }
        let rendered = self.streaming.view();
        if !rendered.is_empty() {
            blocks.push(gutter(TN_GREEN, &rendered));
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
                .fg(if on { TN_YELLOW } else { TN_GRAY })
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
            let bar = Style::new().fg(TN_GRAY).render("│");
            let tail: Vec<&str> = self.tool_output.lines().rev().take(12).collect();
            let body = tail
                .into_iter()
                .rev()
                .map(|l| format!("{m}{bar} {}", Style::new().fg(TN_GRAY).render(l)))
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
        self.selection = None; // content changed → screen-coord selection is stale
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
            // Digit keys pick the numbered option directly (1 Yes · 2 Always · 3 No).
            KeyCode::Char(c @ '1'..='3') => {
                Some(cmd::msg(self.apply_approval(c as usize - '1' as usize)))
            }
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
                .fg(TN_YELLOW)
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
                Style::new().fg(TN_FG).render(&raw)
            });
        }
        menu.push(pad_to(
            &Style::new()
                .fg(TN_GRAY)
                .render("  Enter select · ↑/↓ · 1–3 · Esc"),
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
    let mut os_config: Option<OsConfig> = None;
    if let Ok(cfg) =
        a3s_code_core::config::CodeConfig::from_file(std::path::Path::new(&config_path))
    {
        for (p, m) in cfg.list_models() {
            let id = format!("{}/{}", p.name, m.id);
            model_ctx.insert(id.clone(), m.limit.context);
            models.push(id);
        }
        default_model = cfg.default_model.clone();
        os_config = cfg.os.clone();
    }
    let context_limit = resolve_ctx_limit(
        default_model
            .as_ref()
            .and_then(|m| model_ctx.get(m))
            .copied(),
    );

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
    let mut claude_dirs = agent_skill_dirs(&workspace);
    // Restore the persisted OS login *before* building the session, so its
    // login-gated built-in `a3s-os-capabilities` skill is materialized and
    // loaded from the first turn (only when signed in).
    let os_session = os_config.as_ref().and_then(crate::a3s_os::current_session);
    if let Some(s) = &os_session {
        // Export endpoint + token so the agent's shell uses $A3S_OS_* directly
        // instead of re-reading ~/.a3s/os-auth.json every call.
        crate::a3s_os::export_os_env(s);
        if let Some(dir) = os_config
            .as_ref()
            .and_then(crate::a3s_os::ensure_capability_skill_dir)
        {
            claude_dirs.push(dir);
        }
    }
    // Claude Code compatibility: inject CLAUDE.md (AGENTS.md is auto-loaded by
    // the core) into the system prompt via prompt slots.
    let instructions = project_instructions(&workspace);
    // When a persisted login is restored on launch, inject the OS-platform
    // directive too (mirrors effort_session_opts) so the very first turn already
    // routes OS questions through the progressive-API skill.
    let os_address = os_session.as_ref().map(|s| s.address.clone());
    let with_instr = |o: SessionOptions| {
        let mut parts: Vec<String> = Vec::new();
        if let Some(i) = &instructions {
            parts.push(i.clone());
        }
        if let Some(addr) = &os_address {
            parts.push(os_platform_guide(addr));
        }
        if parts.is_empty() {
            o
        } else {
            o.with_prompt_slots(SystemPromptSlots::default().with_extra(parts.join("\n\n")))
        }
    };
    let manifest_backend = ManifestWorkspaceBackend::new(std::path::PathBuf::from(&workspace));
    let workspace_manifest = manifest_backend.manifest();
    let initial_manifest = workspace_manifest.snapshot();
    let initial_files = initial_manifest.file_paths();
    let workspace_manifest_rx = Arc::new(Mutex::new(workspace_manifest.subscribe()));
    let workspace_services = WorkspaceServices::local_with_manifest_backend(manifest_backend);
    let session = match agent.resume_session(
        session_id.as_str(),
        with_instr(with_recent_workspace_context(
            SessionOptions::new()
                .with_session_store(store.clone())
                .with_confirmation_policy(confirmation.clone())
                .with_workspace_backend(workspace_services.clone())
                .with_skill_dirs(claude_dirs.clone())
                .with_auto_save(true)
                .with_auto_compact(true)
                .with_auto_compact_threshold(0.85)
                .with_file_memory(memory_dir())
                .with_max_parallel_tasks(8)
                .with_auto_delegation_enabled(true)
                .with_auto_parallel_delegation(true)
                .with_manual_delegation_enabled(true),
            &workspace_manifest,
        )),
    ) {
        Ok(s) => s,
        Err(_) => agent.session(
            workspace.clone(),
            Some(with_instr(with_recent_workspace_context(
                SessionOptions::new()
                    .with_session_store(store.clone())
                    .with_session_id(session_id.as_str())
                    .with_confirmation_policy(confirmation.clone())
                    .with_workspace_backend(workspace_services.clone())
                    .with_skill_dirs(claude_dirs.clone())
                    .with_auto_save(true)
                    .with_auto_compact(true)
                    .with_auto_compact_threshold(0.85)
                    .with_file_memory(memory_dir())
                    .with_max_parallel_tasks(8)
                    .with_auto_delegation_enabled(true)
                    .with_auto_parallel_delegation(true)
                    .with_manual_delegation_enabled(true),
                &workspace_manifest,
            ))),
        )?,
    };

    let (width, height) = a3s_tui::terminal::Terminal::size().unwrap_or((80, 24));

    // Seed the transcript with any resumed conversation (user + assistant text).
    let resumed = session.history();
    let mut initial_messages: Vec<String> = resumed
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
                    Some(gutter(TN_GREEN, &md.view()))
                }
                _ => None,
            }
        })
        .collect();
    // Seed ↑/↓ input recall with the user's prior prompts so resuming a session
    // keeps its command history (tool-result `user` messages carry no text block,
    // so the non-empty filter excludes them).
    let history_seed: Vec<String> = resumed
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.text().trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    // Quiet confirmation that the persisted login was restored. Only when
    // RESUMING an existing conversation — on a fresh start, leaving the transcript
    // empty lets the welcome banner show (it notes the signed-in account itself);
    // inserting this line here is what was suppressing the banner after OS login.
    if let Some(s) = &os_session {
        if !initial_messages.is_empty() {
            initial_messages.insert(
                0,
                Style::new().fg(TN_GRAY).render(&format!(
                    "  ✓ signed in to OS as {} · capabilities skill active · /logout to sign out",
                    s.display_label()
                )),
            );
        }
    }

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
        // NB: Ctrl+U / Ctrl+D are intentionally NOT bound to scroll — they shadow
        // readline line-editing (Ctrl+U = kill-to-start) in the input. PageUp/Down
        // and Ctrl+Home/End cover scrolling.
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
        os_config,
        os_session,
        os_refreshing: false,
        os_gateway_models: None,
        last_view: None,
        effort: 2, // high
        effort_panel: None,
        theme_panel: None,
        quit_armed: None,
        last_activity: Instant::now(),
        auto_reviewed: false,
        shell_mode: false,
        research_mode: false,
        selection: None,
        last_workflow: None,
        pending_images: Vec::new(),
        goal: None,
        goal_since: None,
        loop_remaining: 0,
        active_tools: 0,
        active_agents: 0,
        subagents: Vec::new(),
        turn_had_agent_activity: false,
        turn_text_after_activity: false,
        ultracode_synthesis_inflight: false,
        ultracode_synthesis_used: false,
        instructions,
        workspace_manifest,
        workspace_manifest_rx,
        workspace_services,
        rainbow_until: None,
        rainbow_frame: 0,
        effort_anim: None,
        compact_summary: None,
        btw: None,
        viewport: Viewport::new(width.saturating_sub(1), height.saturating_sub(7)),
        textarea: Textarea::new()
            .with_height(1)
            .with_auto_grow(8) // box grows with Shift+Enter newlines (no scroll)
            .with_width(width.saturating_sub((PAD + 2) as u16)) // PAD margin + "❯ "
            .with_submit_on_enter(true),
        spinner: Spinner::new().with_title(""),
        streaming: StreamingMarkdown::new((width as usize).saturating_sub(PAD + 2)),
        got_delta: false,
        compacting: None,
        updating: None,
        last_paint: None,
        thinking: String::new(),
        state: State::Idle,
        messages: initial_messages,
        rx: None,
        pending_tool: None,
        approval_sel: 0,
        history: history_seed,
        history_pos: None,
        model: default_model,
        output_tokens: 0,
        tool_args: String::new(),
        tool_output: String::new(),
        tool_log: Vec::new(),
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
        top_focus: None,
        top_kill: None,
        ide: None,
        git: None,
        memory: None,
        help_open: false,
        completed: 0,
        branch: git_branch(&workspace),
        slash_sel: 0,
        files: initial_files,
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
        // Capture the mouse so the wheel scrolls the transcript (alt-screen has no
        // terminal scrollback, so capture is the only way to get wheel events).
        // Copy is preserved: most terminals still do native selection on
        // Shift+drag (Fn/⌥ on macOS Terminal) even with capture on, plus `/copy`
        // yanks the last reply via OSC52, and `/mouse` drops capture entirely for
        // pure native selection.
        .with_mouse_support()
        .with_fps(30)
        .run()
        .await?;

    // `/update` found a newer version → upgrade via Homebrew in the (now
    // restored) shell so brew's own download progress shows, then re-exec the
    // freshly-installed binary. Use PATH `a3s` (brew repointed its symlink to
    // the new version); current_exe() is the OLD version's path.
    if UPGRADE_ON_EXIT.load(std::sync::atomic::Ordering::Relaxed) {
        let latest = LATEST
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default();
        match crate::update::perform_upgrade(&latest) {
            Some(bin) => {
                let restart_args = ["code", "resume", session_id.as_str()];
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
                    // exec replaces this process; only returns on failure → fall back.
                    let _ = std::process::Command::new(&bin).args(restart_args).exec();
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).args(restart_args).exec();
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = std::process::Command::new(&bin).args(restart_args).status();
                }
            }
            None => eprintln!(
                "\n✗ upgrade failed — get the latest from https://github.com/A3S-Lab/Cli/releases/latest\n"
            ),
        }
        return Ok(());
    }

    // Session is auto-saved under this directory; show how to come back.
    println!("\n  session saved · resume it with:  a3s code resume {session_id}\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_code_core::llm::{
        ContentBlock, LlmClient, LlmResponse, Message, StreamEvent, TokenUsage, ToolDefinition,
    };
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone, Default)]
    struct CapturedLlmTurn {
        system: Option<String>,
        tools: Vec<String>,
    }

    struct CaptureLlmClient {
        turns: Mutex<Vec<CapturedLlmTurn>>,
        responses: Mutex<VecDeque<LlmResponse>>,
    }

    #[async_trait]
    impl LlmClient for CaptureLlmClient {
        async fn complete(
            &self,
            _messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            self.record(system, tools);
            Ok(self.next_response())
        }

        async fn complete_streaming(
            &self,
            _messages: &[Message],
            system: Option<&str>,
            tools: &[ToolDefinition],
            _cancel_token: CancellationToken,
        ) -> anyhow::Result<mpsc::Receiver<StreamEvent>> {
            self.record(system, tools);
            let response = self.next_response();
            let (tx, rx) = mpsc::channel(2);
            tokio::spawn(async move {
                let _ = tx.send(StreamEvent::Done(response)).await;
            });
            Ok(rx)
        }
    }

    impl CaptureLlmClient {
        fn new(responses: Vec<LlmResponse>) -> Self {
            Self {
                turns: Mutex::new(Vec::new()),
                responses: Mutex::new(responses.into()),
            }
        }

        fn record(&self, system: Option<&str>, tools: &[ToolDefinition]) {
            self.turns.lock().unwrap().push(CapturedLlmTurn {
                system: system.map(str::to_string),
                tools: tools.iter().map(|tool| tool.name.clone()).collect(),
            });
        }

        fn next_response(&self) -> LlmResponse {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(done_response)
        }

        fn turns(&self) -> Vec<CapturedLlmTurn> {
            self.turns.lock().unwrap().clone()
        }
    }

    fn tool_call_response(name: &str, input: serde_json::Value) -> LlmResponse {
        LlmResponse {
            message: Message {
                role: "assistant".into(),
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_test".into(),
                    name: name.into(),
                    input,
                }],
                reasoning_content: None,
            },
            usage: TokenUsage::default(),
            stop_reason: Some("tool_use".into()),
            meta: None,
        }
    }

    fn done_response() -> LlmResponse {
        LlmResponse {
            message: Message {
                role: "assistant".into(),
                content: vec![ContentBlock::Text {
                    text: "DONE".into(),
                }],
                reasoning_content: None,
            },
            usage: TokenUsage::default(),
            stop_reason: Some("stop".into()),
            meta: None,
        }
    }

    fn test_config(path: &std::path::Path) {
        std::fs::write(
            path,
            "default_model = \"openai/x\"\n\
             providers \"openai\" {\n  apiKey = \"x\"\n  baseUrl = \"http://127.0.0.1:1\"\n  \
             models \"x\" { name = \"x\" }\n}\n",
        )
        .unwrap();
    }

    /// Guard: the parallel/ultracode SessionOptions register `task` +
    /// `parallel_task` in the session tool surface (so fan-out has a tool to call).
    #[tokio::test]
    async fn parallel_opts_register_parallel_task() {
        let dir = std::env::temp_dir().join(format!("a3s-ptask-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg = dir.join("config.acl");
        test_config(&cfg);
        let agent = a3s_code_core::Agent::new(cfg.to_string_lossy().to_string())
            .await
            .unwrap();
        // The FULL ultracode config (planning + goal + parallel fan-out).
        let opts = SessionOptions::new()
            .with_max_parallel_tasks(8)
            .with_auto_delegation_enabled(true)
            .with_auto_parallel_delegation(true)
            .with_manual_delegation_enabled(true)
            .with_planning_mode(a3s_code_core::PlanningMode::Enabled)
            .with_goal_tracking(true)
            .with_max_tool_rounds(200);
        let session = agent
            .session(dir.to_string_lossy().to_string(), Some(opts))
            .unwrap();
        let names = session.tool_names();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            names.contains(&"parallel_task".to_string()) && names.contains(&"task".to_string()),
            "parallel_task/task registered under the parallel opts; got {names:?}"
        );
    }

    // ── `/output` formatting ───────────────────────────────────────────────
    #[test]
    fn format_tool_log_empty_is_none() {
        assert!(format_tool_log_records(&[]).is_none());
    }

    #[test]
    fn format_tool_log_renders_header_args_and_output() {
        let recs = vec![
            ToolCallRecord {
                name: "read".into(),
                args: Some(serde_json::json!({"file_path": "/x"})),
                output: "hello\n".into(),
                exit_code: 0,
            },
            ToolCallRecord {
                name: "bash".into(),
                args: None,
                output: String::new(),
                exit_code: 2,
            },
        ];
        let out = format_tool_log_records(&recs).unwrap();
        assert!(out.contains("#1 · read · ok"), "{out}");
        assert!(out.contains("args: {\"file_path\":\"/x\"}"), "{out}");
        assert!(
            out.contains("    hello"),
            "output should be indented: {out}"
        );
        assert!(out.contains("#2 · bash · exit 2"), "{out}");
    }

    // ── `?` deep-research mode ─────────────────────────────────────────────
    #[test]
    fn deep_research_prompt_directs_research_and_keeps_query() {
        let p = deep_research_prompt("rust async runtimes");
        assert!(p.contains("rust async runtimes"), "{p}");
        let lo = p.to_lowercase();
        assert!(lo.contains("deep research"), "{p}");
        assert!(lo.contains("web search") && lo.contains("web_fetch"), "{p}");
        assert!(lo.contains("source"), "should ask to cite sources: {p}");
    }

    #[test]
    fn deep_research_goal_is_a_research_north_star_with_query() {
        let g = deep_research_goal("rust async runtimes");
        assert!(g.contains("rust async runtimes"), "{g}");
        assert!(g.to_lowercase().contains("research"), "{g}");
    }

    // ── scroll + copy ──────────────────────────────────────────────────────
    #[test]
    fn scrollbar_blank_when_content_fits() {
        let out = append_scrollbar("a\nb\nc", 5, 3, 100);
        assert_eq!(out.lines().count(), 3);
        for line in out.lines() {
            assert!(line.ends_with(' '), "no-overflow gutter blank: {line:?}");
            assert!(!line.contains('█') && !line.contains('│'));
        }
    }

    #[test]
    fn scrollbar_thumb_tracks_position() {
        let view = "r0\nr1\nr2\nr3"; // 4 visible rows, far more total
        let top = append_scrollbar(view, 4, 40, 0);
        assert!(top.lines().next().unwrap().contains('█'), "thumb at top");
        let bottom = append_scrollbar(view, 4, 40, 100);
        assert!(
            bottom.lines().last().unwrap().contains('█'),
            "thumb at bottom"
        );
        // every row carries the bar (thumb or track) once content overflows
        assert!(top.lines().all(|l| l.contains('█') || l.contains('│')));
    }

    #[test]
    fn osc52_wraps_base64_in_envelope() {
        let s = osc52_copy("hi");
        assert!(s.starts_with("\u{1b}]52;c;") && s.ends_with('\u{7}'));
        assert!(s.contains("aGk=")); // base64("hi")
    }

    #[test]
    fn slice_cols_handles_ascii_and_wide() {
        assert_eq!(slice_cols("hello", 1, 4), "ell");
        assert_eq!(slice_cols("hello", 0, 100), "hello");
        // CJK glyphs are width-2: "你好" spans columns 0..4.
        assert_eq!(slice_cols("你好", 0, 2), "你");
        assert_eq!(slice_cols("你好", 2, 4), "好");
    }

    #[test]
    fn selection_to_text_extracts_span_across_rows() {
        let view = "  hello world\n  second line\n  third";
        // row0 col2..end, through row1 col0..8 — trailing padding trimmed.
        let t = selection_to_text(view, 0, 2, 1, 8);
        assert_eq!(t, "hello world\n  second");
    }

    #[test]
    fn highlight_selection_touches_only_selected_rows() {
        let view = "row zero\nrow one\nrow two";
        let out = highlight_selection(view, 1, 0, 1, 7);
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines[0], "row zero"); // untouched
        assert_eq!(lines[2], "row two"); // untouched
        assert!(lines[1].contains("row one")); // selected text preserved
        assert!(lines[1].contains('\u{1b}')); // wrapped in a style escape
    }

    /// `?` deep research is only meaningful if the agent actually has the web
    /// tools to call — guard that they're registered in the session surface.
    #[tokio::test]
    async fn web_tools_registered_for_q_research_mode() {
        let dir = std::env::temp_dir().join(format!(
            "a3s-research-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config.acl");
        test_config(&cfg);
        let agent = a3s_code_core::Agent::new(cfg.to_string_lossy().to_string())
            .await
            .unwrap();
        let session = agent
            .session(dir.to_string_lossy().to_string(), None)
            .unwrap();
        let names = session.tool_names();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            names.contains(&"web_search".to_string()) && names.contains(&"web_fetch".to_string()),
            "the `?` deep-research mode relies on web_search + web_fetch; got {names:?}"
        );
    }

    #[tokio::test]
    async fn claude_session_surface_passes_system_tools_and_skills_to_llm() {
        let dir = std::env::temp_dir().join(format!(
            "a3s-claude-surface-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config.acl");
        test_config(&cfg);
        std::fs::write(
            dir.join("CLAUDE.md"),
            "Project rule: claude-session-surface-marker",
        )
        .unwrap();
        let skill_dir = dir.join(".claude/skills/inspect-surface");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: inspect-surface\n\
             description: Inspect the Claude session surface\n\
             kind: instruction\n\
             allowed-tools:\n  - Read\n---\n\
             Use this skill marker: inspect-surface-skill-marker\n",
        )
        .unwrap();

        let agent = a3s_code_core::Agent::new(cfg.to_string_lossy().to_string())
            .await
            .unwrap();
        let llm = Arc::new(CaptureLlmClient::new(vec![done_response()]));
        let opts = SessionOptions::new()
            .with_llm_client(llm.clone())
            .with_prompt_slots(
                SystemPromptSlots::default()
                    .with_extra(project_instructions(dir.to_str().unwrap()).unwrap()),
            )
            .with_skill_dirs(agent_skill_dirs(dir.to_str().unwrap()))
            .with_manual_delegation_enabled(true)
            .with_auto_delegation_enabled(false)
            .with_planning_mode(a3s_code_core::PlanningMode::Disabled);
        let session = agent
            .session(dir.to_string_lossy().to_string(), Some(opts))
            .unwrap();

        let (mut rx, join) = session
            .stream("Use available skills to inspect this project.", None)
            .await
            .unwrap();
        while let Some(event) = rx.recv().await {
            if matches!(event, a3s_code_core::AgentEvent::End { .. }) {
                break;
            }
        }
        join.await.unwrap();
        let turns = llm.turns();
        let captured = turns.first().unwrap();
        let system = captured.system.as_deref().unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            system.contains("You are A3S Code"),
            "core system prompt should reach the LLM"
        );
        assert!(
            system.contains("claude-session-surface-marker"),
            "CLAUDE.md project instructions should reach the LLM"
        );
        assert!(
            system.contains("# Skills"),
            "skill catalog guidance should reach the LLM system prompt"
        );
        assert!(
            captured.tools.iter().any(|name| name == "read")
                && captured.tools.iter().any(|name| name == "Skill")
                && captured.tools.iter().any(|name| name == "search_skills")
                && captured.tools.iter().any(|name| name == "parallel_task"),
            "a3s tools and skill tools should be model-visible; got {:?}",
            captured.tools
        );
    }

    #[tokio::test]
    async fn claude_can_invoke_skill_and_child_run_receives_skill_prompt() {
        let dir = std::env::temp_dir().join(format!(
            "a3s-claude-skill-invoke-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config.acl");
        test_config(&cfg);
        let skill_dir = dir.join(".claude/skills/inspect-surface");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: inspect-surface\n\
             description: Inspect the Claude session surface\n\
             kind: instruction\n\
             allowed-tools:\n  - Read\n---\n\
             Use this skill marker: inspect-surface-skill-marker\n",
        )
        .unwrap();

        let agent = a3s_code_core::Agent::new(cfg.to_string_lossy().to_string())
            .await
            .unwrap();
        let llm = Arc::new(CaptureLlmClient::new(vec![
            tool_call_response(
                "Skill",
                serde_json::json!({
                    "skill_name": "inspect-surface",
                    "prompt": "Apply the inspect-surface skill."
                }),
            ),
            done_response(),
            done_response(),
        ]));
        let opts = SessionOptions::new()
            .with_llm_client(llm.clone())
            .with_skill_dirs(agent_skill_dirs(dir.to_str().unwrap()))
            .with_manual_delegation_enabled(true)
            .with_auto_delegation_enabled(false)
            .with_permission_policy(
                a3s_code_core::permissions::PermissionPolicy::new().allow("Skill(*)"),
            )
            .with_planning_mode(a3s_code_core::PlanningMode::Disabled)
            .with_max_tool_rounds(5);
        let session = agent
            .session(dir.to_string_lossy().to_string(), Some(opts))
            .unwrap();

        let result = session
            .send("Use the inspect-surface skill.", None)
            .await
            .unwrap();
        let turns = llm.turns();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(result.text.trim(), "DONE");
        let system_snippets = turns
            .iter()
            .enumerate()
            .map(|(index, turn)| {
                format!(
                    "#{index}: {}",
                    turn.system
                        .as_deref()
                        .unwrap_or("<none>")
                        .chars()
                        .take(220)
                        .collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            turns
                .iter()
                .any(|turn| turn.system.as_deref().is_some_and(|system| {
                    system.contains("You are executing the 'inspect-surface' skill")
                        && system.contains("inspect-surface-skill-marker")
                })),
            "Skill tool should start a child LLM run with the skill prompt; turns: {}",
            system_snippets
        );
    }

    #[test]
    fn workflow_doc_captures_single_task_dispatch() {
        let args = serde_json::json!({
            "agent": "plan",
            "description": "Design the rendering architecture",
            "prompt": "Plan a layered renderer."
        });
        let (doc, label) = workflow_doc_for_tool("task", Some(&args)).unwrap();

        assert!(label.contains("delegated task"), "{label}");
        assert!(doc.contains("Design the rendering architecture"));
        assert!(doc.contains("Agent: `plan`"));
        assert!(doc.contains("Plan a layered renderer."));
    }

    #[test]
    fn synthesis_requires_activity_without_followup_text() {
        // Fires when a turn had agent activity but produced no final text — in
        // ANY mode (no effort gate), so a high-effort fan-out that ends silently
        // still gets a synthesized answer.
        assert!(needs_synthesis(false, false, true, false));
        // No final answer needed if the turn already produced text after activity.
        assert!(!needs_synthesis(false, false, true, true));
        // At most once per turn.
        assert!(!needs_synthesis(false, true, true, false));
        // Nothing to synthesize if no work happened (e.g. a bare greeting).
        assert!(!needs_synthesis(false, false, false, false));
        // Never while a synthesis turn is itself in flight.
        assert!(!needs_synthesis(true, false, true, false));
    }

    #[test]
    fn estimate_tokens_counts_cjk_heavier_than_ascii() {
        assert_eq!(estimate_tokens("abcd"), 1); // ASCII ~4 chars/token
        assert_eq!(estimate_tokens("书安操作系统"), 6); // CJK ~1 token/char (chars/4 would say 1)
        assert_eq!(estimate_tokens("hi 书安"), 2); // mixed: 3 ASCII -> 0, 2 wide -> 2
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn ctx_limit_falls_back_when_undeclared() {
        assert_eq!(resolve_ctx_limit(Some(200_000)), 200_000); // declared wins
        assert_eq!(resolve_ctx_limit(Some(0)), DEFAULT_CONTEXT_LIMIT); // zero -> default
        assert_eq!(resolve_ctx_limit(None), DEFAULT_CONTEXT_LIMIT); // missing -> default
    }

    #[test]
    fn auto_compact_threshold_scales_to_real_window() {
        // 128k model: fire at 85% of 128k, i.e. 0.85*128/200 of the core's fixed 200k.
        assert!((auto_compact_threshold_for(128_000) - 0.544).abs() < 0.001);
        // 200k model == the core's own denominator: plain 0.85.
        assert!((auto_compact_threshold_for(200_000) - 0.85).abs() < 0.001);
        // Windows past ~235k clamp to 1.0 (trigger at the fixed 200k, never overflow).
        assert_eq!(auto_compact_threshold_for(1_000_000), 1.0);
        // Unknown window (0) falls back to the core default of 0.85.
        assert!((auto_compact_threshold_for(0) - 0.85).abs() < 0.001);
    }

    #[test]
    fn task_tool_empty_child_output_renders_useful_summary() {
        let args = serde_json::json!({
            "agent": "plan",
            "description": "Plan subsystem boundaries",
            "prompt": "Create the plan."
        });
        let meta = serde_json::json!({
            "task_id": "task-abc123",
            "session_id": "task-run-task-abc123",
            "agent": "plan",
            "success": true,
            "output_bytes": 0,
            "artifact_uri": "a3s://tasks/task-run-task-abc123/runs/task-abc123/output"
        });
        let output = "Task completed: task-abc123\n\
                      Agent: plan\n\
                      Session: task-run-task-abc123\n\
                      Task ID: task-abc123\n\
                      Artifact ID: task-output:task-abc123\n\
                      Artifact URI: a3s://tasks/task-run-task-abc123/runs/task-abc123/output\n\
                      Output:\n";
        let out = render_tool_end("task", 0, output, Some(&meta), Some(&args), 100);
        let plain = strip_ansi(&out);

        assert!(plain.contains("Explored"));
        assert!(plain.contains("Task completed · plan · task-abc123"));
        assert!(plain.contains("no child text output"));
        assert!(plain.contains("artifact: a3s://tasks/task-run-task-abc123"));
    }

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
        // Bash args are token-colored (program/flags/args), so check visible text.
        let plain = a3s_tui::style::strip_ansi(&out);
        assert!(plain.contains("Ran"), "action verb for bash");
        assert!(plain.contains("npm test"), "shows the command argument");
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

    #[test]
    fn reload_is_idle_only_because_it_rebuilds_the_session() {
        assert!(IDLE_ONLY.contains(&"/reload"));
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
