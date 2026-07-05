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
use a3s_tui::components::{
    MenuItem, MenuPanel, ModeLine, SessionStatus, SessionStatusChip, Spinner, Textarea,
    ToolLogRecord as TuiToolLogRecord, ToolLogStatus, ToolLogView, Viewport,
};
use a3s_tui::event::KeyEvent;
use a3s_tui::keymap::{KeyBinding, Keymap};
use a3s_tui::layout::{Constraint, Layout};
use a3s_tui::style::{Color, Style};
use a3s_tui::{Event, KeyCode, KeyModifiers, Model, ProgramBuilder};
use tokio::sync::{mpsc, Mutex};

use crate::top::{collect_processes, render_process_table, ProcessRow, ProcessTableView};

// Team digital assets.
#[path = "assets/clone.rs"]
mod asset_clone;
#[path = "assets/lifecycle.rs"]
mod asset_lifecycle;
#[path = "assets/naming.rs"]
mod asset_naming;

// System configuration and integrations.
#[path = "system/config.rs"]
mod config;
#[path = "system/skills.rs"]
pub(crate) mod skills;
#[path = "system/update.rs"]
mod update;

// Local workspace.
#[path = "workspace/gitutil.rs"]
mod gitutil;

// Local and shared knowledge.
#[path = "knowledge/kbutil.rs"]
mod kbutil;

// Context and memory.
#[path = "context/memutil.rs"]
mod memutil;

// OS Runtime bridge.
#[path = "os/progressive.rs"]
mod os_progressive;
#[path = "os/remote_ui.rs"]
mod remote_ui;
#[path = "os/runtime_policy.rs"]
mod runtime_policy;
mod runtime_projection;

// Terminal UI support.
#[path = "ui/design_markdown.rs"]
mod design_markdown;
#[path = "ui/image.rs"]
mod image;
#[path = "ui/render.rs"]
mod render;
#[path = "ui/syntax.rs"]
mod syntax;
#[path = "ui/util.rs"]
mod util;

mod panels;
use asset_naming::*;
use config::*;
use design_markdown::StreamingMarkdown;
use gitutil::*;
use image::*;
use memutil::*;
use render::*;
use runtime_policy::RuntimePolicy;
use runtime_projection::{RuntimeProjection, ToolCallRecord};
use skills::*;
use syntax::*;
use update::*;
use util::*;

const HITL_CONFIRM_TIMEOUT_MS: u64 = 60 * 60 * 1000;
const BACKGROUND_CONFIRM_TIMEOUT_MS: u64 = 500;
const TOOL_EXEC_TIMEOUT_MS: u64 = 30 * 60 * 1000;

/// Terminal-safe mapping of the DESIGN.md Geist/Vercel palette.
const ACCENT: Color = Color::Rgb(0, 112, 243); // link / active / success
const TN_GREEN: Color = ACCENT; // compatibility alias: success maps to blue
const TN_YELLOW: Color = Color::Rgb(245, 166, 35); // warning
const TN_RED: Color = Color::Rgb(238, 0, 0); // error / destructive
const TN_CYAN: Color = Color::Rgb(80, 227, 194); // sparse accent
const TN_ORANGE: Color = Color::Rgb(249, 203, 40); // ship gradient amber
const TN_PURPLE: Color = Color::Rgb(151, 71, 255); // lifted preview violet
const TN_FG: Color = Color::Rgb(237, 237, 237); // primary text on dark terminals
const TN_GRAY: Color = Color::Rgb(143, 143, 143); // muted text
const SURFACE_SOFT: Color = Color::Rgb(31, 31, 31);
const SURFACE_SELECTED: Color = Color::Rgb(7, 49, 108);
const GRADIENT_DEVELOP_START: Color = Color::Rgb(0, 124, 240);
const GRADIENT_DEVELOP_END: Color = Color::Rgb(0, 223, 216);
const GRADIENT_PREVIEW_START: Color = TN_PURPLE;
const GRADIENT_PREVIEW_END: Color = Color::Rgb(255, 0, 128);
const GRADIENT_SHIP_START: Color = Color::Rgb(255, 77, 77);
const GRADIENT_SHIP_END: Color = Color::Rgb(249, 203, 40);
const BRAND_GRADIENT: [Color; 6] = [
    GRADIENT_DEVELOP_START,
    GRADIENT_DEVELOP_END,
    GRADIENT_PREVIEW_START,
    GRADIENT_PREVIEW_END,
    GRADIENT_SHIP_START,
    GRADIENT_SHIP_END,
];

/// Self-contained system-prompt directive injected ONLY when signed in to the OS
/// platform. It disambiguates "OS" (the user means the signed-in OS open
/// platform, not this machine's operating system) AND inlines exactly how to call
/// the progressive API, so the model can act immediately — without first
/// discovering/loading the `a3s-os-capabilities` skill (that extra hop is why a
/// passive catalog entry rarely triggered: the model fell back to `whoami`).
/// `base_url` is the signed-in address so the endpoint is concrete.
fn os_platform_guide(base_url: &str) -> String {
    format!(
        "[OS platform] You are signed in to the OS open platform at {base_url} (via /login). \
DEFAULT RULE: while signed in, \"OS\" in the user's questions ALWAYS means THIS OS platform — \
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
loses the Open view button. \
Summarize the result for the user in a few lines; do NOT paste the whole raw JSON back. \
After your summary, ALWAYS output the trace on its own line, exactly \
`↳ requestId <requestId> · <timestamp>`. \
You do NOT print the view link yourself: whenever the execute output carries a `.view`, the host \
automatically shows a one-click `Open view` button that opens the authenticated progressive UI popup \
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
    ("/config", "edit config.acl in the built-in editor"),
    ("/theme", "cycle the code-highlight theme (Geist Dark …)"),
    (
        "/flow",
        "select a workflow asset → OS Workflow as a Service designer (needs /login) · /flow <text> drafts one",
    ),
    (
        "/agent",
        "select an agent definition → local dev · Agent as a Service or Function as a Service by kind",
    ),
    (
        "/mcp",
        "select an MCP server asset → local dev · publish/debug/test via OS Function as a Service",
    ),
    (
        "/skill",
        "select a skill asset → local dev · publish/deploy via OS Function as a Service",
    ),
    (
        "/okf",
        "select an OKF package → local dev · publish/deploy via OS Knowledge service",
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
    ("/top", "inspect local agent process activity"),
    ("/ide", "superfile-style file browser + editor"),
    (
        "/memory",
        "browse memory as an event/entity graph with tiers and forget candidates",
    ),
    (
        "/kb",
        "open the local personal knowledge base · add/import/search/vault",
    ),
    (
        "/ctx",
        "search past sessions (ctx) · /ctx <n> attach · /ctx save <n> keep as memory",
    ),
    ("/effort", "adjust model effort (low … max)"),
    ("/compact", "summarize + compact the conversation context"),
    ("/goal", "set a north-star goal the agent keeps in mind"),
    (
        "/loop",
        "engineered loop dashboard · agent-aware in /agent mode · /loop <task> quick loop",
    ),
    (
        "/sleep",
        "consolidate today's work into memory (experience · preferences · knowledge)",
    ),
    ("/help", "show commands and shortcuts"),
    (
        "/fork",
        "branch a new session from this point (original kept)",
    ),
    ("/clear", "reset the conversation"),
    ("/auto", "switch to auto-approve mode"),
    ("/exit", "quit a3s code"),
];

/// Slash commands that mutate the session / conversation and so must NOT run
/// mid-stream — hidden from the menu and rejected while a turn is in flight.
const IDLE_ONLY: &[&str] = &[
    "/clear", "/compact", "/model", "/effort", "/goal", "/loop", "/reload", "/update", "/init",
    "/fork", "/sleep", "/flow", "/agent", "/mcp", "/skill", "/okf", "/kb",
];

/// Slash commands whose name starts with `input` (input begins with `/`).
fn slash_candidates(input: &str) -> Vec<(&'static str, &'static str)> {
    SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(input))
        .copied()
        .collect()
}

fn slash_tail<'a>(input: &'a str, command: &str) -> Option<&'a str> {
    input
        .strip_prefix(command)
        .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

fn os_asset_category_query(category: &str, query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        format!("category:{category}")
    } else {
        format!("category:{category} {query}")
    }
}

fn runtime_asset_query(category: &str, asset_hint: &str, query: &str) -> String {
    let category = category.trim();
    let asset_hint = asset_hint.trim();
    let query = query.trim();
    let mut parts = Vec::new();
    if !category.is_empty() {
        parts.push(format!("category:{category}"));
    }
    if !asset_hint.is_empty() {
        parts.push(asset_hint.to_string());
    }
    if !query.is_empty() {
        parts.push(query.to_string());
    }
    parts.join(" ")
}

fn cancel_pending_picker<Panel, Pending>(
    picker: &mut Option<Panel>,
    pending: &mut Option<Pending>,
) {
    *picker = None;
    *pending = None;
}

fn os_required_message(cmd: &str, os_configured: bool) -> String {
    if os_configured {
        format!("  {cmd} needs OS — sign in with /login first")
    } else {
        format!("  {cmd} needs OS — configure `os = \"https://your-os-host\"` in config.acl, then /login")
    }
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
/// but wide scripts are closer to ~1 token/char — so a flat `chars / 4`
/// under-counts them by 3-4x and makes the live counter lurch
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

fn ctx_limit_for_model(model_ctx: &std::collections::HashMap<String, u32>, model: &str) -> u32 {
    resolve_ctx_limit(
        model_ctx
            .get(model)
            .copied()
            .or_else(|| crate::codex::codex_model_context(model))
            .or_else(|| inferred_ctx_limit(model)),
    )
}

fn inferred_ctx_limit(model: &str) -> Option<u32> {
    let m = model.trim().to_ascii_lowercase();
    if let Some(limit) = context_suffix_limit(&m) {
        return Some(limit);
    }

    // Configured/gateway-reported limits win. These are only fallbacks for
    // account-backed picker models that do not come from config.acl.
    if m.contains("claude") {
        return Some(200_000);
    }
    if m.contains("gpt-5") || m.contains("gpt-4.1") {
        return Some(1_000_000);
    }
    if m.contains("o1") || m.contains("o3") || m.contains("o4") {
        return Some(200_000);
    }
    if m.contains("gpt-4o") || m.contains("gpt-4") || m.contains("glm") {
        return Some(DEFAULT_CONTEXT_LIMIT);
    }

    None
}

fn context_suffix_limit(model: &str) -> Option<u32> {
    if !model.ends_with(']') {
        return None;
    }
    let start = model.rfind('[')?;
    let suffix = model.get(start + 1..model.len().checked_sub(1)?)?;
    if suffix.is_empty() {
        return None;
    }
    let (number, scale) = suffix.split_at(suffix.len().saturating_sub(1));
    let base = number.parse::<u32>().ok()?;
    match scale {
        "k" => base.checked_mul(1_000),
        "m" => base.checked_mul(1_000_000),
        _ => suffix.parse::<u32>().ok(),
    }
}

/// Scale the core's auto-compact threshold so it fires at ~85% of `window` (the
/// model's REAL context window) rather than 85% of the core's fixed 200k. For a
/// 128k model: `0.85 * 128k / 200k = 0.544` → triggers at ~108.8k (= 85% of
/// 128k). Windows above ~235k clamp to 1.0 (trigger at 200k): a touch early, but
/// it never lets the window overflow. The floor only guards degenerate values —
/// a real floor (like the old 0.05) would push tiny windows' trigger PAST the
/// window itself (0.05 → 10k tokens > an 8k window: compaction could never
/// precede overflow).
fn auto_compact_threshold_for(window: u32) -> f32 {
    let window = if window > 0 {
        window as f32
    } else {
        CORE_MAX_CONTEXT_TOKENS
    };
    (0.85 * window / CORE_MAX_CONTEXT_TOKENS).clamp(0.01, 1.0)
}

/// Context-fill warning latch: maps `pct` to its tier (0/70/85) and says
/// whether crossing INTO a higher tier than `warned` should warn now. The
/// returned tier becomes the new latch, so dropping back (compaction, /clear,
/// model switch) re-arms the warning. Pure for unit-testing.
fn ctx_warn_tier(pct: usize, warned: u8) -> (u8, Option<u8>) {
    let tier: u8 = if pct >= 85 {
        85
    } else if pct >= 70 {
        70
    } else {
        0
    };
    (tier, (tier > warned).then_some(tier))
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
                "dynamic workflow script · /flow workflow to view read-only".to_string(),
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
            "dynamic workflow · {} parallel tasks · /flow workflow to view read-only",
            tasks.len()
        )
    } else {
        format!(
            "dynamic workflow · {} delegated task{} · /flow workflow to view read-only",
            tasks.len(),
            if tasks.len() == 1 { "" } else { "s" }
        )
    };
    Some((doc, label))
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

/// Render the `/output` viewer body. It mirrors the main transcript's tool
/// vocabulary while staying plain-text for the read-only editor.
fn format_tool_log_records(records: &[ToolCallRecord], width: usize) -> Option<String> {
    if records.is_empty() {
        return None;
    }
    const OUTPUT_TAIL: usize = 24;

    let records = records
        .iter()
        .map(|rec| {
            let arg = rec
                .args
                .as_ref()
                .and_then(|args| arg_summary_for_tool(&rec.name, args))
                .unwrap_or_default();
            let action = if arg.is_empty() {
                tool_verb(&rec.name).to_string()
            } else {
                format!("{} {}", tool_verb(&rec.name), arg)
            };
            let status = if rec.exit_code == 0 {
                ToolLogStatus::Ok
            } else {
                ToolLogStatus::Exit(rec.exit_code)
            };
            let args = rec
                .args
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .unwrap_or_default();
            let mut record = TuiToolLogRecord::new(action, status).output(rec.output.clone());
            if let Some(args) = args {
                record = record.args(args);
            }
            record
        })
        .collect::<Vec<_>>();

    Some(
        ToolLogView::new()
            .records(records)
            .max_output_lines_per_record(OUTPUT_TAIL)
            .header_color(TN_FG)
            .status_colors(TN_GREEN, TN_RED)
            .muted_color(TN_GRAY)
            .output_color(TN_FG)
            .view(width as u16, usize::MAX),
    )
}

/// The directive sent to the agent for a `?` deep-research turn: decompose the
/// question, search and read multiple sources, cross-check, and synthesize a
/// cited report. When OS is signed in, use A3S Runtime parallelism and publish
/// the final report through RemoteUI.
fn deep_research_prompt(query: &str, os_runtime: bool) -> String {
    let runtime_directive = if os_runtime {
        format!(
            "OS runtime is available. Use it: split the query into 4-8 independent \
         research tracks and run them in parallel with the OS A3S Runtime / \
         `parallel_task` before synthesis. Do not do all source gathering serially. \
         Ask each worker to return URLs, dates, key evidence, contradictions, and \
         confidence notes. After merging the tracks, create both a Markdown report \
         and a standalone HTML page, then use the OS progressive UI/RemoteUI path \
         (shaped response with `.view`/`viewUrl` as documented by the OS capability \
         guide) so the TUI can show the user a one-click view. Runtime evidence must \
         include both fan-out (`runtime` or `parallel_task`) and the report view. Do \
         not print a raw authenticated URL; summarize and let the host surface the \
         RemoteUI view. {}",
            RuntimePolicy::Required.directive()
        )
    } else {
        "OS runtime is not available in this session. Do the research locally with \
         available web tools, still create a Markdown report and standalone HTML \
         page under `.a3s/research/<slug>/`, and tell the user the local paths. \
         Do not claim a RemoteUI view was created without an OS view response."
            .to_string()
    };
    format!(
        "Conduct deep research to answer the query below. Be thorough:\n\
         1. Break it into the key sub-questions worth investigating.\n\
         2. Use web search across those sub-questions, then read the most relevant \
         sources in full with web_fetch — don't rely on result snippets alone.\n\
         3. Cross-check claims across multiple independent sources; call out any \
         disagreement, uncertainty, or recency caveats.\n\
         4. {runtime_directive}\n\
         5. Synthesize a comprehensive, well-structured answer with inline \
         citations, a final \"Sources\" list of the URLs you used, and clear links \
         to the report artifacts/view.\n\n\
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
/// remembered view (`/view` does the same). The styled button is still transcript
/// text, so ANSI stripping keeps this marker clickable.
const VIEW_BUTTON_MARKER: &str = "Open view";

fn remote_view_button(detail: &str) -> String {
    let button = Style::new()
        .fg(Color::BrightWhite)
        .bg(ACCENT)
        .bold()
        .render(&format!(" ↗ {VIEW_BUTTON_MARKER} "));
    let detail = detail.trim();
    if detail.is_empty() {
        button
    } else {
        format!("{button} {}", Style::new().fg(TN_GRAY).render(detail))
    }
}

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
const SELECTION_BG: Color = SURFACE_SELECTED;

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
/// Also backs `/config` (rooted at the config dir) and the `/kb` browser
/// (rooted at the vault, with delete enabled) — all superfile-styled.
struct Ide {
    entries: Vec<IdeEntry>,
    sel: usize,
    tree_scroll: usize,
    file: Option<IdeFile>,
    focus_editor: bool,
    /// Transient save status shown in the footer (set on Ctrl+S).
    flash: Option<String>,
    /// Left-panel title ("workspace" / "config" / "knowledge base").
    title: String,
    /// Superfile-style hover preview of the tree-selected file, keyed by path
    /// so it reloads only when the selection actually moves.
    preview: Option<(std::path::PathBuf, Vec<String>)>,
    /// `/kb` browser: the vault root. Enables `x` delete, hard-bounded to
    /// paths inside this root. `None` for /ide and /config.
    kb_root: Option<std::path::PathBuf>,
    /// A path armed for deletion — the next `x` on the same selection deletes.
    armed_delete: Option<std::path::PathBuf>,
}

impl Ide {
    /// A fresh panel over `entries` (no file open, tree focused).
    fn browse(entries: Vec<IdeEntry>, title: &str) -> Self {
        Ide {
            entries,
            sel: 0,
            tree_scroll: 0,
            file: None,
            focus_editor: false,
            flash: None,
            title: title.to_string(),
            preview: None,
            kb_root: None,
            armed_delete: None,
        }
    }
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

/// One `/effort` level. Effort scales reasoning depth on THREE axes so it is
/// meaningful on every model, not just Anthropic:
/// - `thinking_budget`: Anthropic extended-thinking tokens (ignored elsewhere);
/// - `max_tool_rounds`: how much multi-step work a single turn may do;
/// - `guideline`: a system-prompt depth steer (the lever that works on GPT/GLM/
///   OS models, which have no thinking budget) — `None` = the model's balanced
///   default.
///
/// `ultracode` additionally plans, then fans independent work out to parallel
/// subagents via `parallel_task`.
struct EffortProfile {
    label: &'static str,
    thinking_budget: usize,
    max_tool_rounds: usize,
    /// Auto-continuation turns: how many times the loop re-prompts to keep going
    /// when the model stops before the task is done. Higher effort persists longer.
    max_continuation_turns: u32,
    guideline: Option<&'static str>,
}

const EFFORT_LEVELS: &[EffortProfile] = &[
    EffortProfile {
        label: "low",
        thinking_budget: 1024,
        max_tool_rounds: 120,
        max_continuation_turns: 2,
        guideline: Some(EFFORT_LOW),
    },
    EffortProfile {
        label: "medium",
        thinking_budget: 4096,
        max_tool_rounds: 200,
        max_continuation_turns: 3,
        guideline: None,
    },
    EffortProfile {
        label: "high",
        thinking_budget: 8192,
        max_tool_rounds: 300,
        max_continuation_turns: 4,
        guideline: Some(EFFORT_HIGH),
    },
    EffortProfile {
        label: "xhigh",
        thinking_budget: 16384,
        max_tool_rounds: 400,
        max_continuation_turns: 6,
        guideline: Some(EFFORT_XHIGH),
    },
    EffortProfile {
        label: "max",
        thinking_budget: 32768,
        max_tool_rounds: 500,
        max_continuation_turns: 8,
        guideline: Some(EFFORT_MAX),
    },
    EffortProfile {
        label: "ultracode",
        thinking_budget: 32768,
        max_tool_rounds: 600,
        max_continuation_turns: 8,
        guideline: Some(ULTRACODE_GUIDELINES),
    },
];
/// Index of the `ultracode` level (special: planning + parallel subagents).
const ULTRACODE: usize = 5;

// Model-agnostic depth steers injected into the system prompt per effort level.
// They scale reasoning + verification rigor; `medium` has none (the baseline).
// Never tell the model to skip SAFETY checks — low trims optional rigor only.
const EFFORT_LOW: &str = "\
[effort: low] Favor speed and minimalism. Answer directly, make the smallest \
change that works (reading enough surrounding code to change it safely), and \
keep verification proportionate: still run the narrowest build/test/type-check \
that covers what you touched — just don't add checks or scope the task didn't \
warrant. Don't gold-plate.";
const EFFORT_HIGH: &str = "\
[effort: high] Favor depth. Reason through the approach before acting. After \
changes, verify the narrow path you touched (build / test / type-check) and \
check the obvious edge cases, then re-read your own diff for correctness before \
finishing.";
const EFFORT_XHIGH: &str = "\
[effort: xhigh] Work rigorously. Before choosing an approach, weigh at \
least one alternative. Verify thoroughly — run the relevant tests/build, probe \
edge cases and failure modes, and confirm the change actually does what was \
asked. Do a self-review pass for correctness and simplicity before concluding.";
const EFFORT_MAX: &str = "\
[effort: max] Maximum rigor; prefer correctness and completeness over speed. \
Decompose the problem, compare alternatives, and implement the strongest \
solution. Verify exhaustively: tests, build, edge cases, and boundary / \
adversarial inputs. Finish with a self-critique pass that actively hunts for \
what you may have missed or gotten wrong, and fix it before concluding.";
/// Ultracode system-prompt steer: keep the model focused on decomposition and
/// synthesis while the core planning runtime turns independent plan waves into
/// visible `parallel_task` subagents.
const ULTRACODE_GUIDELINES: &str = "\
[ultracode] Dynamic-workflow mode is available — you decide whether a turn needs \
it. Match the effort to the task: answer trivial or conversational input (a \
greeting, a single question, a one-step edit) directly, with no plan and no \
fan-out. When a task genuinely splits into independent branches, decompose it, \
run those branches as parallel background subagents via `parallel_task` (keep \
each child prompt bounded and evidence-oriented), then synthesize their results \
before continuing dependent work.";

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
    display: String,
    runtime_expectation: Option<RuntimeExpectation>,
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
type StreamJoin = tokio::task::JoinHandle<()>;

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
    StreamStarted(SharedRx, StreamJoin),
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
    /// Post-login SSH-key sync finished (registers the local pubkey with OS).
    SshKeySynced(crate::a3s_os::SshKeyOutcome),
    /// OS access token was refreshed (or refresh failed) in the background.
    OsRefreshed(Result<crate::a3s_os::StoredOsSession, String>),
    /// OS unified-gateway model ids fetched for the `/model` picker.
    OsGatewayModels(Result<Vec<crate::a3s_os::GatewayModel>, String>),
    /// Answer from a `/btw` background side-thread.
    SideNote(String),
    /// Refreshed process snapshot for the `/top` panel.
    TopData(Vec<ProcessRow>),
    /// Tick to re-fetch the `/top` snapshot.
    TopRefresh,
    /// `/fork` copied the session under a new id (Ok) — swap the active session to
    /// it — or failed (Err with a reason).
    Forked(Result<String, String>),
    /// `/memory` graph data loaded (timeline + details + derived graph).
    MemoryLoaded(MemPanelData),
    /// A `/memory` forget-candidate deletion finished, with fresh graph data.
    MemoryForgotten(Result<(String, MemPanelData), String>),
    /// Asset-scoped OS asset list loaded.
    AssetListLoaded(Result<panels::asset_resources::AssetListFetch, String>),
    /// Runtime activity rows loaded for an asset-scoped activity panel.
    RuntimeActivityLoaded(Result<panels::asset_resources::RuntimeActivityFetch, String>),
    /// `/kb import` finished; carries the one-line summary to show.
    KbAdded(String),
    /// `/ctx <query>` finished: raw `ctx search --json` stdout (or the error).
    CtxResults(Result<String, String>),
    /// `/ctx <n>` finished: (hit title, transcript window) to stage as context.
    CtxWindow(Result<(String, String), String>),
    /// `/ctx save <n>` finished: Ok(hit title) once written to the memory store.
    CtxSaved(Result<String, String>),
    /// `/sleep` finished persisting its consolidated memories (count on Ok).
    SleepSaved(Result<usize, String>),
    /// `/flow` published/opened/inspected an OS Workflow as a Service asset.
    FlowOsCompleted(Result<panels::flow::FlowOsResult, String>),
    /// `/agent` published/opened an OS agent asset through Agent as a Service or Function as a Service.
    AgentOsCompleted(Result<panels::agent::AgentOsResult, String>),
    /// `/mcp` published/debugged/tested an OS Function as a Service MCP asset.
    McpOsCompleted(Result<panels::mcp::McpOsResult, String>),
    /// `/skill` published/deployed/inspected an OS Function as a Service skill asset.
    SkillOsCompleted(Result<panels::skill::SkillOsResult, String>),
    /// `/okf` published/deployed an OS Knowledge service package asset.
    OkfOsCompleted(Result<panels::okf::OkfOsResult, String>),
    /// Asset source was cloned into the local asset workspace.
    AssetCloned(Result<asset_clone::AssetCloneResult, String>),
    /// `/memory` → ctx back-jump finished: (ctx event id, transcript window).
    CtxMemorySource(Result<(String, String), String>),
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

fn resume_after_pending_confirmation_cmd(rx: Option<SharedRx>) -> Cmd<Msg> {
    let mut cmds = vec![spinner_tick()];
    if let Some(rx) = rx {
        cmds.push(pump(rx));
    }
    cmd::batch(cmds)
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

fn tui_session_options(confirmation: a3s_code_core::hitl::ConfirmationPolicy) -> SessionOptions {
    SessionOptions::new()
        .with_confirmation_policy(confirmation)
        .with_tool_timeout(TOOL_EXEC_TIMEOUT_MS)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeEvidenceMode {
    Any,
    ParallelReportView,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeExpectation {
    label: String,
    policy: RuntimePolicy,
    evidence_mode: RuntimeEvidenceMode,
    runtime_tool: bool,
    parallel_work: bool,
    remote_view: bool,
    warned_missing: bool,
}

impl RuntimeExpectation {
    fn required(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            policy: RuntimePolicy::Required,
            evidence_mode: RuntimeEvidenceMode::Any,
            runtime_tool: false,
            parallel_work: false,
            remote_view: false,
            warned_missing: false,
        }
    }

    fn required_report_view(label: impl Into<String>) -> Self {
        Self {
            evidence_mode: RuntimeEvidenceMode::ParallelReportView,
            ..Self::required(label)
        }
    }

    fn record_tool(&mut self, name: &str) {
        match name {
            "runtime" => self.runtime_tool = true,
            "parallel_task" | "task" => self.parallel_work = true,
            _ => {}
        }
    }

    fn record_parallel_work(&mut self) {
        self.parallel_work = true;
    }

    fn record_remote_view(&mut self) {
        self.remote_view = true;
    }

    fn has_parallel_evidence(&self) -> bool {
        self.runtime_tool || self.parallel_work
    }

    fn is_satisfied(&self) -> bool {
        match self.evidence_mode {
            RuntimeEvidenceMode::Any => self.has_parallel_evidence() || self.remote_view,
            RuntimeEvidenceMode::ParallelReportView => {
                self.has_parallel_evidence() && self.remote_view
            }
        }
    }

    fn missing_expectation(&self) -> String {
        match self.evidence_mode {
            RuntimeEvidenceMode::Any => {
                "expected `runtime`, `parallel_task`, or an OS shaped `.view`/`viewUrl` response"
                    .to_string()
            }
            RuntimeEvidenceMode::ParallelReportView => match (self.has_parallel_evidence(), self.remote_view) {
                (false, false) => {
                    "expected OS Runtime/`parallel_task` fan-out plus an OS shaped `.view`/`viewUrl` report response".to_string()
                }
                (false, true) => {
                    "expected OS Runtime/`parallel_task` fan-out before the report view".to_string()
                }
                (true, false) => {
                    "expected an OS shaped `.view`/`viewUrl` response for the report".to_string()
                }
                (true, true) => unreachable!("satisfied expectations are filtered before warning"),
            },
        }
    }

    fn missing_warning(&mut self) -> Option<String> {
        if self.policy != RuntimePolicy::Required || self.is_satisfied() || self.warned_missing {
            return None;
        }
        self.warned_missing = true;
        Some(format!(
            "  Runtime evidence missing for {} - {} before the final answer",
            self.label,
            self.missing_expectation()
        ))
    }

    fn corrective_prompt(&self) -> Option<String> {
        if self.policy != RuntimePolicy::Required || self.is_satisfied() {
            return None;
        }
        Some(format!(
            "The previous turn ended without the required OS Runtime evidence for {}: {}. \
             Continue the same task, explicitly use OS Runtime or `parallel_task` fan-out as required, \
             create or surface the shaped OS `.view`/`viewUrl` report response when required, \
             and only then give the final answer. If the OS capability is unavailable, explain exactly \
             which OS endpoint or response field is missing and provide local report artifact paths.",
            self.label,
            self.missing_expectation()
        ))
    }
}

fn is_new_remote_view(last_view: Option<&remote_ui::ViewSpec>, spec: &remote_ui::ViewSpec) -> bool {
    last_view != Some(spec)
}

fn take_pending_tool_label(
    pending_tool: &mut Option<(String, String)>,
    tool_id: &str,
) -> Option<String> {
    if pending_tool
        .as_ref()
        .is_some_and(|(pending_id, _)| pending_id == tool_id)
    {
        pending_tool.take().map(|(_, label)| label)
    } else {
        None
    }
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
    /// Highest context-fill tier already warned about (0 / 70 / 85), so each
    /// warning prints once per fill-up and re-arms when usage drops back.
    ctx_warned_tier: u8,
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
    /// OS unified-gateway models for the `/model` picker, lazily fetched on
    /// first `/model` while signed in. `None` = not fetched yet; `Some([])` = the
    /// gateway is unavailable/unconfigured.
    os_gateway_models: Option<Vec<String>>,
    /// The precise reason the last gateway-models fetch failed (e.g. `/v1` not
    /// proxied → HTML, auth error, unreachable), shown in the `/model` picker.
    os_gateway_error: Option<String>,
    /// Last OS view seen in a tool result. Generic tool views are opened by
    /// `/view` or clicking the inline "Open view" button; owned workflows like
    /// `/flow` may also open their prepared designer view directly.
    last_view: Option<remote_ui::ViewSpec>,
    /// Required Runtime use for the current autonomous workflow, plus observed
    /// evidence from tool/subagent/view events.
    runtime_expectation: Option<RuntimeExpectation>,
    /// Current model effort (index into EFFORT_LEVELS).
    effort: usize,
    /// `/effort` slider panel: temp selection while open.
    effort_panel: Option<usize>,
    /// `/theme` picker: temp theme index while open.
    theme_panel: Option<usize>,
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
    /// True from an asset-scoped review submit until its report is parsed (or
    /// the run is interrupted/fails). Gates capture_review so a turn that merely
    /// QUOTES an a3s-review block can't open a phantom checklist.
    review_pending: bool,
    /// True from a `/sleep` submit until its report is parsed (or the run is
    /// interrupted/fails). Gates capture_sleep the same way.
    sleep_pending: bool,
    /// Last parsed asset-review report (issues + checkbox state). Survives the
    /// panel closing so a follow-up asset review can reopen it.
    review: Option<panels::review::ReviewState>,
    /// `/flow` DAG picker (login-gated); open when `Some`.
    flow: Option<panels::flow::FlowPanel>,
    /// A `/flow <action>` submitted before a flow was selected; run after selection.
    pending_flow_subcommand: Option<panels::flow::FlowSubcommand>,
    /// `/agent` definition picker; open when `Some`.
    agent_picker: Option<panels::agent::AgentPanel>,
    /// A `/agent <action>` submitted before an agent was active; run after selection.
    pending_agent_subcommand: Option<panels::agent::AgentSubcommand>,
    /// The local agent currently being developed by ordinary user turns.
    agent_dev: Option<panels::agent::AgentDevSession>,
    /// `/mcp` asset selector; open when `Some`.
    mcp_picker: Option<panels::mcp::McpPanel>,
    /// A `/mcp <action>` submitted before an MCP was active; run after selection.
    pending_mcp_subcommand: Option<panels::mcp::McpSubcommand>,
    /// The local MCP asset currently being developed by ordinary user turns.
    mcp_dev: Option<panels::mcp::McpDevSession>,
    /// `/skill` picker; open when `Some`.
    skill_picker: Option<panels::skill::SkillPanel>,
    /// A `/skill <action>` submitted before a skill was active; run after selection.
    pending_skill_subcommand: Option<panels::skill::SkillSubcommand>,
    /// The local skill currently being developed by ordinary user turns.
    skill_dev: Option<panels::skill::SkillDevSession>,
    /// `/okf` OKF package picker; open when `Some`.
    okf_picker: Option<panels::okf::OkfPackagePanel>,
    /// A `/okf <action>` submitted before an OKF package was active; run after selection.
    pending_okf_subcommand: Option<panels::okf::OkfCommand>,
    /// The local OKF package currently being developed by ordinary user turns.
    okf_dev: Option<panels::okf::OkfDevSession>,
    /// Whether the review issue-checklist overlay is showing.
    review_open: bool,
    /// `ctx` CLI detected at startup (past-session history search).
    ctx_ready: bool,
    /// Last `/ctx` search hits, addressable as `/ctx <n>`.
    ctx_hits: Vec<panels::ctx::CtxHit>,
    /// A transcript window staged by `/ctx <n>`, attached (one-shot) to the
    /// next outgoing message.
    pending_ctx: Option<String>,
    /// True for the single `Msg::Submit` the `/loop` mechanism emits to
    /// auto-continue — so on_submit doesn't attach a staged `/ctx` window to
    /// this machine turn.
    loop_continuation: bool,
    /// ALL assistant text of the current turn (across mid-turn tool-call
    /// finalizes, which clear the live streaming buffer). capture_review scans
    /// this when a provider leaves `End.text` empty.
    turn_text: String,
    /// Active transcript text-selection (mouse drag → highlight → copy on
    /// release); `None` when there's no selection.
    selection: Option<Selection>,
    /// Latest dynamic-workflow artifact (ultracode parallel_task dispatch),
    /// shown collapsed in the transcript and openable read-only via `/flow workflow`.
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
    /// ECS-style projection of live runtime entities (tools/subagents) plus
    /// completed tool-call records used by `/output`.
    runtime: RuntimeProjection,
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
    /// Brief brand-gradient flourish on the input border when ultracode is picked.
    gradient_until: Option<Instant>,
    gradient_frame: usize,
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
    stream_join: Option<StreamJoin>,
    interrupting: bool,
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
    /// When the current run started, for the live elapsed-time indicator.
    stream_started: Option<Instant>,
    /// Animation counter for the blinking running-tool dot (advances per tick).
    blink_tick: u8,
    /// Frame counter for the welcome-mascot animation.
    anim: u8,
    /// Run mode (Shift+Tab cycles default → plan → auto).
    mode: Mode,
    /// The mode to restore once an autonomous directive run finishes —
    /// `Some` while such a run auto-switched to `Mode::Auto`.
    autonomy_restore: Option<Mode>,
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
    /// `/ide` file-tree + viewer panel (Some when open).
    ide: Option<Ide>,
    /// `/memory` full-screen timeline panel (Some when open).
    memory: Option<MemPanel>,
    /// Asset-scoped OS digital-asset browser.
    asset_list: Option<panels::asset_resources::AssetListPanel>,
    /// Asset-scoped OS Runtime activity panel.
    runtime_activity: Option<panels::asset_resources::RuntimeActivityPanel>,
    /// `/kb` full-screen local personal knowledge-base panel (Some when open).
    kb: Option<panels::kb::KbPanel>,
    /// `/loop` engineered loop dashboard (Some when open).
    loop_panel: Option<panels::loop_engineering::LoopPanel>,
    /// `/help` overlay panel is showing.
    help_open: bool,
    /// Scroll offset inside the `/help` overlay.
    help_scroll: usize,
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
    /// Skill names the user disabled via `/plugin` (persisted, hidden from `/`).
    disabled_skills: std::collections::HashSet<String>,
    /// `/plugin` panel: selected row while open.
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
                // An open /ide image buffer is rasterized at open-time width —
                // re-render it for the new panel size or its rows overflow the
                // frame (styled rows can't be re-truncated).
                if let Some(f) = self
                    .ide
                    .as_mut()
                    .and_then(|i| i.file.as_mut())
                    .filter(|f| f.image)
                {
                    let inner = panels::spf::ide_split(width as usize).1.saturating_sub(2);
                    let body = (height as usize).saturating_sub(5);
                    f.lines = render_image_file(&f.path, inner, body)
                        .unwrap_or_else(|| vec!["<cannot decode image>".into()]);
                    f.scroll = 0;
                }
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
                // The /help overlay owns its own close + scroll keys.
                if self.help_open {
                    return self.handle_help_key(&key);
                }
                // /memory panel takes all keys while open.
                if self.memory.is_some() {
                    return self.memory_key(&key);
                }
                // Asset resource panels take all keys while open.
                if self.asset_list.is_some() {
                    return self.handle_asset_list_key(&key);
                }
                if self.runtime_activity.is_some() {
                    return self.handle_runtime_activity_key(&key);
                }
                // /kb panel takes all keys while open; an approval prompt
                // still overlays it and must get the keys first.
                if self.kb.is_some() {
                    if self.state == State::Awaiting {
                        return self.handle_approval_key(&key);
                    }
                    return self.handle_kb_key(&key);
                }
                // /ide panel takes all keys while open — except while a tool
                // approval is pending: the prompt is overlaid on the page and
                // must get the keys, or the turn stalls invisibly.
                if self.ide.is_some() {
                    if self.state == State::Awaiting {
                        return self.handle_approval_key(&key);
                    }
                    self.ide_key(&key);
                    return None;
                }
                // /top panel takes keys while open.
                if self.top.is_some() {
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
                                self.gradient_frame = 0;
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
                // /plugin panel: ↑/↓ select, Space enable/disable, Esc close.
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
                // Asset review issue checklist: consume EVERY key while open.
                if self.review_open {
                    return self.handle_review_key(&key);
                }
                // `/flow` DAG picker: same.
                if self.flow.is_some() {
                    return self.handle_flow_key(&key);
                }
                // `/agent` definition picker: same.
                if self.agent_picker.is_some() {
                    return self.handle_agent_key(&key);
                }
                // `/mcp` asset selector: same.
                if self.mcp_picker.is_some() {
                    return self.handle_mcp_key(&key);
                }
                if self.skill_picker.is_some() {
                    return self.handle_skill_key(&key);
                }
                if self.okf_picker.is_some() {
                    return self.handle_okf_package_key(&key);
                }
                // `/loop` engineered-loop dashboard: same.
                if self.loop_panel.is_some() {
                    return self.handle_loop_key(&key);
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
                // Esc leaves shell/research mode first (discarding the
                // partial input), taking priority over the streaming interrupt
                // below.
                if (self.shell_mode || self.research_mode) && key.code == KeyCode::Esc {
                    self.shell_mode = false;
                    self.research_mode = false;
                    self.textarea.clear();
                    return None;
                }
                // Esc interrupts the in-progress run (input stays usable otherwise).
                if self.state == State::Streaming && key.code == KeyCode::Esc {
                    if self.interrupting {
                        return None;
                    }
                    self.interrupting = true;
                    self.push_line(&Style::new().fg(TN_YELLOW).render("  ⎋ interrupting…"));
                    let session = self.session.clone();
                    let join = self.stream_join.take();
                    return Some(cmd::cmd(move || async move {
                        session.cancel().await;
                        if let Some(join) = join {
                            let _ = join.await;
                        }
                        Msg::Interrupted
                    }));
                }
                if self.state == State::Idle && self.agent_dev.is_some() && key.code == KeyCode::Esc
                {
                    self.exit_agent_dev();
                    return None;
                }
                if self.state == State::Idle && self.mcp_dev.is_some() && key.code == KeyCode::Esc {
                    self.exit_mcp_dev();
                    return None;
                }
                if self.state == State::Idle && self.skill_dev.is_some() && key.code == KeyCode::Esc
                {
                    self.exit_skill_dev();
                    return None;
                }
                if self.state == State::Idle && self.okf_dev.is_some() && key.code == KeyCode::Esc {
                    self.exit_okf_dev();
                    return None;
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
                // A leading `!` enters shell mode and a leading `?` enters
                // deep-research mode. Each stays on until Esc or submit.
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
                if self.help_open {
                    match m.kind {
                        MouseEventKind::ScrollUp => self.scroll_help_by(-3),
                        MouseEventKind::ScrollDown => self.scroll_help_by(3),
                        _ => {}
                    }
                    return None;
                }
                // Full-screen /ide //config //kb page: the transcript isn't
                // visible, so transcript scroll/select must not act on it
                // (a drag would silently copy hidden text).
                if self.ide.is_some()
                    || self.kb.is_some()
                    || self.asset_list.is_some()
                    || self.runtime_activity.is_some()
                {
                    return None;
                }
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
                                // the agent's inline "Open view" button; else just clear.
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

            Msg::StreamStarted(rx, join) => {
                self.rx = Some(rx.clone());
                self.stream_join = Some(join);
                self.interrupting = false;
                return Some(pump(rx));
            }

            Msg::StreamError(e) => {
                self.push_line(&Style::new().fg(TN_RED).render(&format!("  error: {e}")));
                self.loop_remaining = 0; // a failed turn stops the /loop
                self.review_pending = false; // a turn that never started can't
                self.sleep_pending = false; // deliver a review/sleep report
                self.restore_autonomy();
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
                // Esc force-aborted the turn. The cancel command awaited the
                // stream join first, so core has committed the interrupted
                // history before any queued continuation starts.
                self.finalize_streaming();
                self.push_line(&Style::new().fg(TN_YELLOW).render("  ⎋ interrupted"));
                self.loop_remaining = 0; // Esc also stops a /loop
                self.review_pending = false; // and abandons an asset review
                self.sleep_pending = false; // and a `/sleep` consolidation
                self.restore_autonomy();
                self.finish();
                return self.drain_queue();
            }

            Msg::Agent(event) => return self.on_agent_event(*event),

            Msg::StreamEnded => {
                if self.interrupting || self.state != State::Streaming {
                    return None;
                }
                // Channel closed without a normal End event (abnormal close).
                self.finalize_streaming();
                // An asset-review report fully streamed before the drop still
                // counts — same for a `/sleep` consolidation report.
                let turn_text = self.turn_text.clone();
                self.capture_review(&turn_text);
                let sleep_save = self.capture_sleep(&turn_text);
                self.disarm_sleep_if_over(sleep_save.is_some());
                return match (sleep_save, self.complete_turn()) {
                    (Some(save), Some(next)) => Some(cmd::batch(vec![save, next])),
                    (save, next) => save.or(next),
                };
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
                    && self.memory.is_none()
                    && !self.help_open
                {
                    self.anim = self.anim.wrapping_add(1);
                    self.viewport.set_content(&self.banner());
                }
                // Advance the ultracode brand-gradient flourish.
                if self.gradient_until.is_some() || self.effort_anim.is_some() {
                    self.gradient_frame = self.gradient_frame.wrapping_add(1);
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
                            .with_timeout(BACKGROUND_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject);
                        let prompt = "Briefly review this conversation so far: summarise the \
                             key decisions and what's done, then list any open threads or next \
                             steps. Keep it to a few lines.";
                        let mut answer = String::new();
                        if let Ok(sess) = agent.session(workspace, Some(tui_session_options(conf)))
                        {
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
                        self.ctx_warned_tier = 0; // fresh window: re-arm fill warnings
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
                    // Auto-register this machine's SSH public key with OS so
                    // git-over-SSH works without manual key setup (idempotent,
                    // best-effort — never blocks the completed login).
                    if let Some(s) = self.os_session.clone() {
                        return Some(cmd::cmd(move || async move {
                            Msg::SshKeySynced(crate::a3s_os::sync_ssh_key(s).await)
                        }));
                    }
                }
                Err(error) => self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  login failed: {error}")),
                ),
            },

            Msg::SshKeySynced(outcome) => {
                use crate::a3s_os::SshKeyOutcome;
                match outcome {
                    SshKeyOutcome::Registered(fp) => self.push_line(&Style::new().fg(TN_GREEN).render(
                        &format!("  ✓ local SSH public key registered with OS ({fp}) · git clone(ssh) ready"),
                    )),
                    SshKeyOutcome::AlreadyRegistered => self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render("  · SSH public key already registered with OS; skipping"),
                    ),
                    SshKeyOutcome::NoLocalKey => self.push_line(&Style::new().fg(TN_YELLOW).render(
                        "  · no local SSH public key found; create one and run /login again to register it automatically: ssh-keygen -t ed25519",
                    )),
                    SshKeyOutcome::Failed(e) => self.push_line(
                        &Style::new()
                            .fg(TN_GRAY)
                            .render(&format!("  · SSH key sync skipped: {e}")),
                    ),
                }
            }

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

            Msg::OsGatewayModels(result) => {
                match result {
                    // Record each model's real context window (when the gateway
                    // reports one) so switching to it sizes auto-compact + the
                    // status bar correctly, then cache the ids.
                    Ok(models) => {
                        for m in &models {
                            if let Some(ctx) = m.context {
                                self.model_ctx.insert(m.id.clone(), ctx);
                            }
                        }
                        self.os_gateway_models = Some(models.into_iter().map(|m| m.id).collect());
                        self.os_gateway_error = None;
                    }
                    // Keep the precise reason so the picker + switch attempt can
                    // explain WHY the gateway is unavailable.
                    Err(e) => {
                        self.os_gateway_models = Some(Vec::new());
                        self.os_gateway_error = Some(e);
                    }
                }
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
            Msg::Forked(result) => {
                match result {
                    Ok(new_id) => {
                        // Swap the active session to the fork (which carries the copied
                        // history). Set the id first — rebuild_session keys off it — and
                        // revert on failure so id and session never desync. The
                        // transcript stays on screen: the fork continues from here.
                        let prev = std::mem::replace(&mut self.session_id, new_id);
                        let model = self.model.clone();
                        match self.rebuild_session(model.as_deref()) {
                            Ok((s, _)) => {
                                self.session = Arc::new(s);
                                let short: String = self.session_id.chars().take(8).collect();
                                self.push_line(&gutter(
                                TN_CYAN,
                                &format!("⑂ forked into a new session ({short}) — the original is kept"),
                            ));
                            }
                            Err(e) => {
                                self.session_id = prev;
                                self.push_line(
                                    &Style::new()
                                        .fg(TN_RED)
                                        .render(&format!("  fork failed: {e}")),
                                );
                            }
                        }
                    }
                    Err(e) => {
                        self.push_line(&Style::new().fg(TN_YELLOW).render(&format!("  /fork: {e}")))
                    }
                }
            }
            Msg::MemoryLoaded(data) => {
                if let Some(m) = &mut self.memory {
                    let source = if data.loaded_from_session {
                        "session fallback · "
                    } else {
                        ""
                    };
                    m.note = format!(
                        "{source}{} memories · {} entities · {} relations",
                        data.entries.len(),
                        data.graph.stats.entities,
                        data.graph.stats.relations
                    );
                    m.sel = 0;
                    m.apply_data(data);
                }
            }
            Msg::MemoryForgotten(result) => {
                if let Some(m) = &mut self.memory {
                    match result {
                        Ok((id, data)) => {
                            m.note = format!(
                                "forgot {id} · {} memories · {} candidates",
                                data.entries.len(),
                                data.graph.stats.forget_candidates
                            );
                            m.apply_data(data);
                        }
                        Err(error) => {
                            m.note = format!("forget failed: {error}");
                        }
                    }
                }
            }
            Msg::AssetListLoaded(result) => self.on_asset_list(result),
            Msg::RuntimeActivityLoaded(result) => self.on_runtime_activity(result),
            Msg::KbAdded(summary) => {
                let color = if summary.starts_with('✗') {
                    TN_RED
                } else {
                    TN_GRAY
                };
                self.push_line(&Style::new().fg(color).render(&format!("  {summary}")));
                if self.kb.is_some() {
                    self.open_kb_home(Some(summary));
                }
            }
            Msg::CtxResults(res) => self.on_ctx_results(res),
            Msg::CtxWindow(res) => self.on_ctx_window(res),
            Msg::CtxSaved(res) => self.on_ctx_saved(res),

            Msg::SleepSaved(res) => self.on_sleep_saved(res),

            Msg::FlowOsCompleted(res) => self.on_flow_os_completed(res),
            Msg::AgentOsCompleted(res) => self.on_agent_os_completed(res),
            Msg::McpOsCompleted(res) => self.on_mcp_os_completed(res),
            Msg::SkillOsCompleted(res) => self.on_skill_os_completed(res),
            Msg::OkfOsCompleted(res) => self.on_okf_os_completed(res),
            Msg::AssetCloned(res) => match res {
                Ok(result) => self.on_asset_cloned(result),
                Err(error) => self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  clone failed: {error}")),
                ),
            },
            Msg::CtxMemorySource(res) => match res {
                Ok((event_id, window)) => {
                    self.memory = None; // leave the panel to show the source
                    self.open_readonly_in_ide(&format!("ctx-source-{event_id}.txt"), &window);
                }
                Err(e) => {
                    if let Some(m) = self.memory.as_mut() {
                        m.note = format!("ctx source unavailable: {e}");
                    }
                }
            },

            _ => {}
        }
        None
    }

    fn view(&self) -> String {
        if self.help_open {
            return self.render_help();
        }
        if let Some(m) = &self.memory {
            return self.render_memory(m);
        }
        if let Some(panel) = &self.asset_list {
            return self.render_asset_list(panel);
        }
        if let Some(panel) = &self.runtime_activity {
            return self.render_runtime_activity(panel);
        }
        if let Some(kb) = &self.kb {
            let page = self.render_kb(kb);
            return self.overlay_approval(page);
        }
        if let Some(panel) = &self.loop_panel {
            return self.render_loop_panel(panel);
        }
        if let Some(ide) = &self.ide {
            // A pending tool approval overlays the full-screen page so it is
            // never invisible (its keys take priority in the key dispatch).
            let page = self.render_ide(ide);
            return self.overlay_approval(page);
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
        // Input mode hint: `!` = shell command (pink), `?` = deep research
        // (cyan), `/agent` dev = local agent development (green), `/mcp` dev =
        // local MCP development (cyan), `/btw` = side-channel (yellow),
        // otherwise the normal prompt (accent blue).
        let inp = self.textarea.value();
        let (sym, icolor, border): (&str, Color, Color) = if self.shell_mode {
            ("!", GRADIENT_PREVIEW_END, GRADIENT_PREVIEW_END)
        } else if self.research_mode {
            ("?", TN_CYAN, TN_CYAN)
        } else if self.agent_dev.is_some() {
            ("◇", TN_GREEN, TN_GREEN)
        } else if self.mcp_dev.is_some() {
            ("◆", TN_CYAN, TN_CYAN)
        } else if self.skill_dev.is_some() {
            ("✦", TN_CYAN, TN_CYAN)
        } else if self.okf_dev.is_some() {
            ("⌁", TN_CYAN, TN_CYAN)
        } else if inp.starts_with("/btw") {
            ("❯", TN_YELLOW, TN_YELLOW)
        } else {
            ("❯", ACCENT, TN_GRAY)
        };
        // Brief brand-gradient ribbon on both input borders after picking
        // ultracode; otherwise plain bottom + effort-chip top.
        let gradient = self
            .gradient_until
            .is_some_and(|t| t.elapsed() < Duration::from_millis(1600));
        let separator = if gradient {
            input_gradient_rule(width, &BRAND_GRADIENT, self.gradient_frame + 3)
        } else {
            input_rule(width, border)
        };
        let top_separator = if gradient {
            input_gradient_rule(width, &BRAND_GRADIENT, self.gradient_frame)
        } else {
            let elabel = format!("◇ {}", EFFORT_LEVELS[self.effort].label);
            // Context-window usage at the top-right of the input (Claude-style).
            let ctxlabel = if self.context_limit > 0 {
                let pct = (self.last_prompt_tokens * 100 / self.context_limit as usize).min(100);
                format!("{pct}% context used  ")
            } else {
                String::new()
            };
            input_status_rule(width, border, &ctxlabel, &elabel)
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

        let typed = self.textarea.view();
        let tint_input = sym == "!"
            || sym == "?"
            || sym == "◇"
            || sym == "◆"
            || sym == "⌁"
            || inp.starts_with("/btw");
        let input_view = input_prompt_line(sym, icolor, &typed, tint_input, width);

        // Bottom status bar (Claude-style, two lines):
        //   dir git:(branch) <model> (<window> context) ctx:N%   [+ live chips]
        //   ⏵⏵ <mode> mode on (shift+tab to cycle) · …
        let status1 = self.session_status_line(width);
        let status2 = render_mode_status_line(self.mode, width);

        // Gap line between transcript and loading — or a floating "jump to
        // latest" hint when the user has scrolled up away from the bottom.
        let spacer = if self.viewport.at_bottom() {
            String::new()
        } else {
            let label = truncate(
                &Style::new()
                    .fg(Color::BrightWhite)
                    .bg(ACCENT)
                    .render(" ↓ more below · Shift+End to jump to latest "),
                width,
            );
            let pad = width.saturating_sub(a3s_tui::style::visible_len(&label)) / 2;
            format!("{}{}", " ".repeat(pad), label)
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
        let composed = self.overlay_review_menu(composed);
        let composed = self.overlay_flow_menu(composed);
        let composed = self.overlay_agent_menu(composed);
        let composed = self.overlay_mcp_menu(composed);
        let composed = self.overlay_skill_menu(composed);
        let composed = self.overlay_okf_package_menu(composed);
        let composed = self.overlay_effort(composed);
        let composed = self.overlay_theme(composed);
        let composed = self.overlay_plugins(composed);
        let composed = self.overlay_approval(composed);
        self.overlay_btw(composed)
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        // In the /ide editor, place the cursor at the edit position — inside
        // the right panel: tree width + its left border + the `%4d ` gutter.
        if let Some(ide) = &self.ide {
            if ide.focus_editor {
                if let Some(f) = &ide.file {
                    let width = self.width as usize;
                    let (tw, _) = panels::spf::ide_split(width);
                    let gutter = if panels::spf::ide_gutter_on(width) {
                        5
                    } else {
                        0
                    };
                    let x = tw + 1 + gutter + f.display_col().saturating_sub(f.hscroll);
                    let col = x.min(width.saturating_sub(2)) as u16;
                    let row = (1 + f.row.saturating_sub(f.scroll)) as u16;
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
            || self.memory.is_some()
            || self.asset_list.is_some()
            || self.runtime_activity.is_some()
            || self.kb.is_some()
            || self.loop_panel.is_some()
            || self.flow.is_some()
            || self.agent_picker.is_some()
            || self.mcp_picker.is_some()
            || self.skill_picker.is_some()
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

fn render_session_status_line(
    cwd: &str,
    branch: Option<&str>,
    model: Option<&str>,
    context_limit: u32,
    last_prompt_tokens: usize,
    output_tokens: usize,
    chips: impl IntoIterator<Item = SessionStatusChip>,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }

    let mut status = SessionStatus::new(cwd)
        .accent_color(ACCENT)
        .branch_color(TN_YELLOW)
        .text_color(TN_FG)
        .muted_color(TN_GRAY)
        .threshold_colors(TN_YELLOW, TN_RED)
        .margin(2);

    if let Some(branch) = branch {
        status = status.branch(branch);
    }
    if let Some(model) = model {
        status = status.model(model);
    }
    if context_limit > 0 {
        status = status.context(last_prompt_tokens, context_limit as usize);
    } else if output_tokens > 0 {
        status = status.output_tokens(output_tokens);
    }
    for chip in chips {
        status = status.status_chip(chip);
    }

    status.view(width.min(u16::MAX as usize) as u16)
}

fn render_mode_status_line(mode: Mode, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    ModeLine::new(mode.name())
        .glyph(mode.glyph())
        .hints("(shift+tab to cycle) · /help · ↑↓ history · esc")
        .mode_color(mode.color())
        .hint_color(TN_GRAY)
        .view(width.min(u16::MAX as usize) as u16)
}

impl App {
    fn session_status_line(&self, width: usize) -> String {
        render_session_status_line(
            &self.cwd,
            self.branch.as_deref(),
            self.model.as_deref(),
            self.context_limit,
            self.last_prompt_tokens,
            self.output_tokens,
            self.session_status_chips(),
            width,
        )
    }

    fn session_status_chips(&self) -> Vec<SessionStatusChip> {
        let mut chips = Vec::new();

        if self.goal.is_some() {
            let elapsed = self
                .goal_since
                .map(|t| format!(" ({})", fmt_elapsed(t.elapsed())))
                .unwrap_or_default();
            chips.push(
                SessionStatusChip::new("🎯", format!("Pursuing goal{elapsed}")).color(TN_CYAN),
            );
        }
        if let Some(dev) = &self.agent_dev {
            chips.push(
                SessionStatusChip::new(
                    "◇",
                    format!("agent:{} · Esc /agent off", truncate(&dev.name, 24)),
                )
                .color(TN_GREEN),
            );
        }
        if let Some(dev) = &self.mcp_dev {
            chips.push(
                SessionStatusChip::new(
                    "◆",
                    format!("mcp:{} · Esc /mcp off", truncate(&dev.name, 24)),
                )
                .color(TN_CYAN),
            );
        }
        if let Some(dev) = &self.skill_dev {
            chips.push(
                SessionStatusChip::new(
                    "✦",
                    format!("skill:{} · Esc /skill off", truncate(&dev.name, 24)),
                )
                .color(TN_CYAN),
            );
        }
        if let Some(dev) = &self.okf_dev {
            chips.push(
                SessionStatusChip::new(
                    "⌁",
                    format!("okf:{} · Esc /okf off", truncate(&dev.name, 24)),
                )
                .color(TN_CYAN),
            );
        }
        if self.loop_remaining > 0 {
            chips.push(
                SessionStatusChip::new("↻", self.loop_remaining.to_string()).color(TN_GRAY),
            );
        }
        let active_subagents = self.runtime.active_subagent_count();
        if active_subagents > 0 {
            chips.push(
                SessionStatusChip::new("⇉", format!("{active_subagents} agents")).color(TN_GRAY),
            );
        }
        let active_tools = self.runtime.active_tool_count();
        if active_tools > 0 {
            chips.push(
                SessionStatusChip::new("⚙", format!("{active_tools} running")).color(TN_GRAY),
            );
        }
        if let Some(version) = self.update_available.as_deref() {
            chips.push(SessionStatusChip::new("⬆", version).color(TN_YELLOW));
        }

        chips
    }

    fn clone_asset_command(
        &mut self,
        family: &'static str,
        url: String,
        root: std::path::PathBuf,
    ) -> Option<Cmd<Msg>> {
        self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
            "  cloning {family} asset from {url} → {}",
            root.display()
        )));
        Some(cmd::cmd(move || async move {
            Msg::AssetCloned(asset_clone::clone_asset_source(family, url, root).await)
        }))
    }

    fn on_asset_cloned(&mut self, result: asset_clone::AssetCloneResult) {
        self.push_line(&Style::new().fg(TN_GREEN).render(&format!(
            "  cloned {} asset → {}",
            result.family,
            result.path.display()
        )));
        match result.family {
            "agent" => self.open_agent_panel_focused(&result.path),
            "mcp" => self.open_mcp_panel_focused(&result.path),
            "skill" => self.open_skill_panel_focused(&result.path),
            "okf" | "knowledge" => self.open_okf_package_panel_focused(&result.path),
            "workflow" => self.open_flow_panel_focused(&result.path),
            _ => self.push_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  run the asset command again to select or operate on it"),
            ),
        }
    }

    fn path_is_within(path: &std::path::Path, root: &std::path::Path) -> bool {
        path == root || path.starts_with(root)
    }

    fn open_agent_panel_focused(&mut self, cloned_path: &std::path::Path) {
        let root = agent_dir();
        let agents = panels::agent::list_agents(&root);
        let Some(sel) = agents
            .iter()
            .position(|agent| Self::path_is_within(&agent.path, cloned_path))
        else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  cloned source does not contain a recognized agent definition yet"),
            );
            return;
        };
        self.agent_picker = Some(panels::agent::AgentPanel { root, agents, sel });
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render("  selected cloned agent asset · Enter develops locally"),
        );
    }

    fn open_mcp_panel_focused(&mut self, cloned_path: &std::path::Path) {
        let root = mcp_dir();
        let projects = panels::mcp::list_mcp_projects(&root);
        let Some(sel) = projects
            .iter()
            .position(|project| Self::path_is_within(&project.path, cloned_path))
        else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  cloned source does not contain a recognized MCP asset yet"),
            );
            return;
        };
        self.mcp_picker = Some(panels::mcp::McpPanel {
            root,
            projects,
            sel,
        });
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render("  selected cloned MCP asset · Enter develops locally"),
        );
    }

    fn open_skill_panel_focused(&mut self, cloned_path: &std::path::Path) {
        let root = skill_dir();
        let skills = panels::skill::list_skill_assets(&root);
        let Some(sel) = skills
            .iter()
            .position(|skill| Self::path_is_within(&skill.path, cloned_path))
        else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  cloned source does not contain a recognized skill asset yet"),
            );
            return;
        };
        self.skill_picker = Some(panels::skill::SkillPanel { root, skills, sel });
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render("  selected cloned skill asset · Enter develops locally"),
        );
    }

    fn open_okf_package_panel_focused(&mut self, cloned_path: &std::path::Path) {
        let root = panels::okf::okf_package_dir(&self.cwd);
        let packages = panels::okf::list_okf_packages(&root);
        let Some(sel) = packages
            .iter()
            .position(|package| Self::path_is_within(&package.path, cloned_path))
        else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  cloned source does not contain a recognized OKF package yet"),
            );
            return;
        };
        self.okf_picker = Some(panels::okf::OkfPackagePanel {
            root,
            packages,
            sel,
        });
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render("  selected cloned OKF package · Enter develops locally"),
        );
    }

    fn open_flow_panel_focused(&mut self, cloned_path: &std::path::Path) {
        let root = flow_dir();
        let flows = panels::flow::list_flows(&root);
        let Some(sel) = flows
            .iter()
            .position(|flow| Self::path_is_within(&root.join(flow), cloned_path))
        else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  cloned source does not contain a recognized workflow design yet"),
            );
            return;
        };
        self.flow = Some(panels::flow::FlowPanel { root, flows, sel });
        self.push_line(
            &Style::new()
                .fg(TN_GRAY)
                .render("  selected cloned workflow asset · Enter opens the OS designer"),
        );
    }

    fn open_latest_dynamic_workflow(&mut self) {
        self.textarea.clear();
        match self.last_workflow.clone() {
            Some(doc) => self.open_readonly_in_ide("dynamic-workflow.md", &doc),
            None => {
                self.push_line(&Style::new().fg(TN_GRAY).render(
                    "  no dynamic workflow yet — run a task that fans out via parallel_task",
                ))
            }
        }
    }

    fn execute_agent_subcommand(
        &mut self,
        subcommand: panels::agent::AgentSubcommand,
    ) -> Option<Cmd<Msg>> {
        match subcommand {
            panels::agent::AgentSubcommand::Exit => {
                self.exit_agent_dev();
                None
            }
            panels::agent::AgentSubcommand::Clone(url) => {
                self.clone_asset_command("agent", url, agent_dir())
            }
            panels::agent::AgentSubcommand::List(query) => {
                self.open_asset_list_panel(os_asset_category_query("agent", &query))
            }
            panels::agent::AgentSubcommand::Activity(query) => {
                let Some(agent_dev) = self.agent_dev.clone() else {
                    self.pending_agent_subcommand =
                        Some(panels::agent::AgentSubcommand::Activity(query));
                    self.open_agent_panel();
                    return None;
                };
                self.open_runtime_activity_panel(runtime_asset_query(
                    "agent",
                    &agent_dev.name,
                    &query,
                ))
            }
            panels::agent::AgentSubcommand::Review => {
                let Some(agent_dev) = self.agent_dev.clone() else {
                    self.pending_agent_subcommand = Some(panels::agent::AgentSubcommand::Review);
                    self.open_agent_panel();
                    return None;
                };
                self.messages
                    .push(user_bubble("/agent review", self.width as usize));
                self.engage_autonomy(4);
                self.review_pending = true;
                let prompt = panels::agent::agent_review_prompt(&agent_dev);
                let display = format!("◇ {} review", agent_dev.name);
                self.start_stream_inner(prompt, display, true, true, false)
            }
            other => {
                let Some(agent_dev) = self.agent_dev.clone() else {
                    self.pending_agent_subcommand = Some(other);
                    self.open_agent_panel();
                    return None;
                };
                let Some(session) = self.os_session.clone() else {
                    self.push_line(
                        &Style::new()
                            .fg(TN_YELLOW)
                            .render("  /agent OS actions need /login first"),
                    );
                    return None;
                };
                let action = match other {
                    panels::agent::AgentSubcommand::Publish(kind) => {
                        panels::agent::AgentOsAction::Publish(kind)
                    }
                    panels::agent::AgentSubcommand::Run => panels::agent::AgentOsAction::Run,
                    panels::agent::AgentSubcommand::Deploy => panels::agent::AgentOsAction::Deploy,
                    panels::agent::AgentSubcommand::Open(kind) => {
                        panels::agent::AgentOsAction::Open(kind)
                    }
                    panels::agent::AgentSubcommand::Logs(kind) => {
                        panels::agent::AgentOsAction::Logs(kind)
                    }
                    panels::agent::AgentSubcommand::Status(kind) => {
                        panels::agent::AgentOsAction::Status(kind)
                    }
                    panels::agent::AgentSubcommand::Exit
                    | panels::agent::AgentSubcommand::Clone(_)
                    | panels::agent::AgentSubcommand::List(_)
                    | panels::agent::AgentSubcommand::Activity(_)
                    | panels::agent::AgentSubcommand::Review => unreachable!(),
                };
                let kind = action.target_kind();
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                    "  ◇ {} → OS {} {}…",
                    agent_dev.name,
                    kind.label(),
                    kind.service_label()
                )));
                Some(cmd::cmd(move || async move {
                    let result =
                        panels::agent::publish_agent_to_os(session, agent_dev, action).await;
                    Msg::AgentOsCompleted(result)
                }))
            }
        }
    }

    fn execute_mcp_subcommand(
        &mut self,
        subcommand: panels::mcp::McpSubcommand,
    ) -> Option<Cmd<Msg>> {
        match subcommand {
            panels::mcp::McpSubcommand::Exit => {
                self.exit_mcp_dev();
                None
            }
            panels::mcp::McpSubcommand::Clone(url) => {
                self.clone_asset_command("mcp", url, mcp_dir())
            }
            panels::mcp::McpSubcommand::List(query) => {
                self.open_asset_list_panel(os_asset_category_query("mcp", &query))
            }
            panels::mcp::McpSubcommand::Activity(query) => {
                let Some(mcp_dev) = self.mcp_dev.clone() else {
                    self.pending_mcp_subcommand = Some(panels::mcp::McpSubcommand::Activity(query));
                    self.open_mcp_panel();
                    return None;
                };
                self.open_runtime_activity_panel(runtime_asset_query("mcp", &mcp_dev.name, &query))
            }
            panels::mcp::McpSubcommand::Review => {
                let Some(mcp_dev) = self.mcp_dev.clone() else {
                    self.pending_mcp_subcommand = Some(panels::mcp::McpSubcommand::Review);
                    self.open_mcp_panel();
                    return None;
                };
                self.messages
                    .push(user_bubble("/mcp review", self.width as usize));
                self.engage_autonomy(4);
                self.review_pending = true;
                let prompt = panels::mcp::mcp_review_prompt(&mcp_dev);
                let display = format!("◆ {} review", mcp_dev.name);
                self.start_stream_inner(prompt, display, true, true, false)
            }
            other => {
                let Some(action) = other.os_action() else {
                    unreachable!("local MCP actions handled above")
                };
                let Some(mcp_dev) = self.mcp_dev.clone() else {
                    self.pending_mcp_subcommand = Some(other);
                    self.open_mcp_panel();
                    return None;
                };
                let Some(session) = self.os_session.clone() else {
                    self.push_line(
                        &Style::new()
                            .fg(TN_YELLOW)
                            .render("  /mcp OS actions need /login first"),
                    );
                    return None;
                };
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                    "  ◆ {} → OS MCP Function as a Service {}…",
                    mcp_dev.name,
                    action.label()
                )));
                Some(cmd::cmd(move || async move {
                    let result = panels::mcp::publish_mcp_to_os(session, mcp_dev, action).await;
                    Msg::McpOsCompleted(result)
                }))
            }
        }
    }

    fn execute_skill_subcommand(
        &mut self,
        subcommand: panels::skill::SkillSubcommand,
    ) -> Option<Cmd<Msg>> {
        match subcommand {
            panels::skill::SkillSubcommand::Exit => {
                self.exit_skill_dev();
                None
            }
            panels::skill::SkillSubcommand::Clone(url) => {
                self.clone_asset_command("skill", url, skill_dir())
            }
            panels::skill::SkillSubcommand::List(query) => {
                self.open_asset_list_panel(os_asset_category_query("skill", &query))
            }
            panels::skill::SkillSubcommand::Activity(query) => {
                let Some(skill_dev) = self.skill_dev.clone() else {
                    self.pending_skill_subcommand =
                        Some(panels::skill::SkillSubcommand::Activity(query));
                    self.open_skill_panel();
                    return None;
                };
                self.open_runtime_activity_panel(runtime_asset_query(
                    "skill",
                    &skill_dev.name,
                    &query,
                ))
            }
            panels::skill::SkillSubcommand::Review => {
                if self.skill_dev.is_none() {
                    self.pending_skill_subcommand = Some(panels::skill::SkillSubcommand::Review);
                    self.open_skill_panel();
                    return None;
                }
                let skill = self.skill_dev.clone().expect("checked above");
                let body = match std::fs::read_to_string(&skill.path) {
                    Ok(body) => body,
                    Err(error) => {
                        self.push_line(&Style::new().fg(TN_RED).render(&format!(
                            "  could not read {}: {error}",
                            skill.path.display()
                        )));
                        return None;
                    }
                };
                self.messages
                    .push(user_bubble("/skill review", self.width as usize));
                self.engage_autonomy(4);
                self.review_pending = true;
                let prompt = panels::skill::skill_review_prompt(&skill.path, &body);
                let display = format!("✦ {} review", skill.name);
                self.start_stream_inner(prompt, display, true, true, false)
            }
            panels::skill::SkillSubcommand::Publish => {
                self.execute_skill_os_action(panels::skill::SkillOsAction::Publish)
            }
            panels::skill::SkillSubcommand::Deploy => {
                if self.skill_dev.is_none() {
                    self.pending_skill_subcommand = Some(panels::skill::SkillSubcommand::Deploy);
                    self.open_skill_panel();
                    return None;
                }
                self.execute_skill_os_action(panels::skill::SkillOsAction::Deploy)
            }
            panels::skill::SkillSubcommand::Open => {
                self.execute_skill_os_action(panels::skill::SkillOsAction::Open)
            }
            panels::skill::SkillSubcommand::Status => {
                self.execute_skill_os_action(panels::skill::SkillOsAction::Status)
            }
        }
    }

    fn execute_skill_os_action(
        &mut self,
        action: panels::skill::SkillOsAction,
    ) -> Option<Cmd<Msg>> {
        let Some(skill_dev) = self.skill_dev.clone() else {
            self.pending_skill_subcommand = Some(match action {
                panels::skill::SkillOsAction::Publish => panels::skill::SkillSubcommand::Publish,
                panels::skill::SkillOsAction::Deploy => panels::skill::SkillSubcommand::Deploy,
                panels::skill::SkillOsAction::Open => panels::skill::SkillSubcommand::Open,
                panels::skill::SkillOsAction::Status => panels::skill::SkillSubcommand::Status,
            });
            self.open_skill_panel();
            return None;
        };
        let Some(session) = self.os_session.clone() else {
            self.push_line(
                &Style::new()
                    .fg(TN_YELLOW)
                    .render("  /skill OS actions need /login first"),
            );
            return None;
        };
        self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
            "  ✦ {} → OS skill Function as a Service {}…",
            skill_dev.name,
            action.label()
        )));
        Some(cmd::cmd(move || async move {
            let result = panels::skill::publish_skill_to_os(session, skill_dev, action).await;
            Msg::SkillOsCompleted(result)
        }))
    }

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
                GRADIENT_PREVIEW_END,
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
            let runtime_hint = if self.os_session.is_some() {
                "  🎯 goal set · OS A3S Runtime parallel research · report + RemoteUI view (Esc stops)"
            } else {
                "  🎯 goal set · local deep research · report + HTML artifacts (Esc stops)"
            };
            self.push_line(&Style::new().fg(TN_GRAY).render(runtime_hint));
            let prompt = deep_research_prompt(&query, self.os_session.is_some());
            let display = format!("🔬 {query}");
            // Long-horizon budget: keep researching across turns toward the
            // goal, with tool prompts auto-approved for the run's duration.
            self.engage_autonomy(8);
            let runtime_expectation = self
                .os_session
                .is_some()
                .then(|| RuntimeExpectation::required_report_view("deep research"));
            if self.state == State::Idle {
                return self.start_stream_inner_with_runtime(
                    prompt,
                    display,
                    true,
                    true,
                    false,
                    runtime_expectation,
                );
            }
            self.seq += 1;
            self.queue.push(Queued {
                prio: 1,
                seq: self.seq,
                text: prompt,
                display,
                runtime_expectation,
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
        if let Some(rest) = slash_tail(trimmed, "/login") {
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
                    self.asset_list = None;
                    self.runtime_activity = None;
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
                    self.asset_list = None;
                    self.runtime_activity = None;
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
        // `/kb` opens the local personal knowledge-base panel. Notes/imports/search are
        // explicit subcommands so a mistyped path no longer becomes a note.
        // `/ctx <query>` searches past agent sessions; `/ctx <n>` stages hit n
        // as context for the next message (ctx CLI, local SQLite index).
        if let Some(rest) = slash_tail(trimmed, "/ctx") {
            return self.handle_ctx_command(rest);
        }
        if let Some(rest) = slash_tail(trimmed, "/okf") {
            return self.handle_okf_command(rest);
        }
        if let Some(rest) = slash_tail(trimmed, "/kb") {
            return self.handle_kb_command(rest);
        }
        // `/btw <prompt>` runs a background side-thread (separate ephemeral
        // session, the main conversation as context) without disturbing the
        // current turn; its answer arrives as a side note.
        if let Some(rest) = slash_tail(trimmed, "/btw") {
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
                    .with_timeout(BACKGROUND_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject);
                let sess = match agent.session(workspace, Some(tui_session_options(conf))) {
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
        if let Some(rest) = slash_tail(trimmed, "/goal") {
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
                if let Some(dev) = self.agent_dev.clone() {
                    let scoped = panels::agent::agent_goal_label(&dev, g);
                    self.goal = Some(scoped.clone());
                    self.goal_since = Some(Instant::now());
                    self.push_line(&gutter(
                        TN_CYAN,
                        &format!("🎯 agent goal set: {} · {g}", dev.name),
                    ));
                    let prompt = panels::agent::agent_dev_prompt(&dev, g);
                    let display = format!("◇ {} goal: {}", dev.name, truncate(g, 54));
                    return self.start_stream_inner(prompt, display, true, true, false);
                }
                // Set the persistent goal AND start working toward it now (the
                // goal is prepended to this and every later prompt).
                self.goal = Some(g.to_string());
                self.goal_since = Some(Instant::now());
                self.push_line(&gutter(TN_CYAN, &format!("🎯 goal set: {g}")));
                return Some(cmd::msg(Msg::Submit(g.to_string())));
            }
            return None;
        }
        // `/loop` — engineered loop dashboard + subcommands; unknown tails keep
        // the quick-loop contract (`/loop <task>`).
        if let Some(rest) = slash_tail(trimmed, "/loop") {
            return self.handle_loop_command(rest);
        }
        // `/sleep [focus]` — end-of-day consolidation: the `/loop` mechanism
        // drives the agent through reviewing today's work (cross-session via
        // `ctx` when installed) until a turn ends with the machine-readable
        // ```a3s-sleep report, which capture_sleep persists into long-term
        // memory (experience · preferences · knowledge). Idle-only.
        if let Some(rest) = slash_tail(trimmed, "/sleep") {
            let focus = rest.trim().to_string();
            self.textarea.clear();
            self.sleep_pending = true;
            self.engage_autonomy(8);
            self.push_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  ☾ sleep — consolidating today's work into memory… (Esc stops)"),
            );
            let directive = panels::sleep::sleep_directive(
                &focus,
                self.ctx_ready,
                &panels::sleep::sleep_today(),
            );
            // Like asset reviews: send the directive but show a short display
            // line (echoing the boilerplate as a user message is just noise).
            let display = if focus.is_empty() {
                "☾ sleep".to_string()
            } else {
                format!("☾ sleep · {focus}")
            };
            return self.start_stream_inner(directive, display, true, true, false);
        }
        // `/flow` — select a local DAG JSON and open it in the OS workflow
        // designer (login-gated); `/flow <description>` orchestrates a basic DAG into
        // the flows folder (local, no login needed). Token-boundary filtered
        // so "/flowx" stays a normal message and can't bypass the idle gate.
        if let Some(rest) = slash_tail(trimmed, "/flow") {
            let description = rest.trim().to_string();
            self.textarea.clear();
            if let Some(parsed) = panels::flow::parse_flow_subcommand(&description) {
                match parsed {
                    Ok(panels::flow::FlowSubcommand::Clone(url)) => {
                        return self.clone_asset_command("workflow", url, flow_dir());
                    }
                    Ok(panels::flow::FlowSubcommand::List(query)) => {
                        return self
                            .open_asset_list_panel(os_asset_category_query("workflow", &query));
                    }
                    Ok(panels::flow::FlowSubcommand::Activity(query)) => {
                        if self.os_session.is_none() {
                            self.push_line(&Style::new().fg(TN_YELLOW).render(
                                &os_required_message(
                                    "workflow runtime activity",
                                    self.os_config.is_some(),
                                ),
                            ));
                        } else {
                            self.pending_flow_subcommand =
                                Some(panels::flow::FlowSubcommand::Activity(query));
                            self.open_flow_panel();
                        }
                        return None;
                    }
                    Ok(panels::flow::FlowSubcommand::Workflow) => {
                        self.open_latest_dynamic_workflow();
                        return None;
                    }
                    Ok(panels::flow::FlowSubcommand::Review(target)) => {
                        let root = flow_dir();
                        let flows = panels::flow::list_flows(&root);
                        let picked = match target {
                            Some(target) => flows
                                .into_iter()
                                .find(|flow| flow == &target || flow.ends_with(&target)),
                            None if flows.len() == 1 => flows.into_iter().next(),
                            None => None,
                        };
                        let Some(file) = picked else {
                            self.pending_flow_subcommand =
                                Some(panels::flow::FlowSubcommand::Review(None));
                            self.open_flow_panel();
                            return None;
                        };
                        let path = root.join(&file);
                        let design = match std::fs::read_to_string(&path) {
                            Ok(value) => value,
                            Err(error) => {
                                self.push_line(&Style::new().fg(TN_RED).render(&format!(
                                    "  could not read {}: {error}",
                                    path.display()
                                )));
                                return None;
                            }
                        };
                        if serde_json::from_str::<serde_json::Value>(&design).is_err() {
                            self.push_line(
                                &Style::new()
                                    .fg(TN_RED)
                                    .render(&format!("  {} is not valid JSON", file)),
                            );
                            return None;
                        }
                        self.messages.push(user_bubble(
                            &format!("/flow review {file}"),
                            self.width as usize,
                        ));
                        self.engage_autonomy(4);
                        self.review_pending = true;
                        let prompt = panels::flow::flow_review_prompt(&path, &design);
                        let display = format!("⧉ flow review: {}", truncate(&file, 48));
                        return self.start_stream_inner(prompt, display, true, true, false);
                    }
                    Ok(action @ panels::flow::FlowSubcommand::Publish)
                    | Ok(action @ panels::flow::FlowSubcommand::Run)
                    | Ok(action @ panels::flow::FlowSubcommand::Deploy)
                    | Ok(action @ panels::flow::FlowSubcommand::Open)
                    | Ok(action @ panels::flow::FlowSubcommand::Logs)
                    | Ok(action @ panels::flow::FlowSubcommand::Status) => {
                        if self.os_session.is_none() {
                            self.push_line(
                                &Style::new().fg(TN_YELLOW).render(
                                    "  /flow publish/run/deploy/open/logs/status needs OS — sign in with /login first",
                                ),
                            );
                        } else {
                            self.pending_flow_subcommand = Some(action);
                            self.open_flow_panel();
                        }
                        return None;
                    }
                    Err(e) => {
                        self.push_line(&Style::new().fg(TN_RED).render(&format!("  {e}")));
                        return None;
                    }
                }
            }
            if description.is_empty() {
                if self.os_session.is_none() {
                    self.push_line(
                        &Style::new()
                            .fg(TN_YELLOW)
                            .render("  /flow needs OS — sign in with /login first"),
                    );
                } else {
                    self.open_flow_panel();
                }
                return None;
            }
            let dir = flow_dir();
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  ⧉ drafting a flow DAG → {} (then /flow opens it in the designer)",
                dir.display()
            )));
            self.engage_autonomy(8);
            let prompt = panels::flow::flow_gen_prompt(&description, &dir.to_string_lossy());
            let display = format!("⧉ flow: {}", truncate(&description, 60));
            return self.start_stream_inner(prompt, display, true, true, false);
        }
        // `/agent` — select a local a3s-code agent definition and enter local
        // multi-turn development mode; `/agent <description>` drafts a local Markdown
        // agent definition; OS subcommands publish/run/deploy the active local
        // definition through Agent as a Service or Function as a Service according to the kind.
        if let Some(rest) = slash_tail(trimmed, "/agent") {
            let description = rest.trim().to_string();
            self.textarea.clear();
            if let Some(parsed) = panels::agent::parse_agent_subcommand(&description) {
                return match parsed {
                    Ok(subcommand) => self.execute_agent_subcommand(subcommand),
                    Err(e) => {
                        self.push_line(&Style::new().fg(TN_RED).render(&format!("  {e}")));
                        None
                    }
                };
            }
            if description.is_empty() {
                self.open_agent_panel();
                return None;
            }
            let dir = agent_dir();
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  ◇ drafting an agent definition → {} (then /agent starts local dev)",
                dir.display()
            )));
            self.engage_autonomy(8);
            let prompt = panels::agent::agent_gen_prompt(&description, &dir.to_string_lossy());
            let display = format!("◇ agent: {}", truncate(&description, 60));
            return self.start_stream_inner(prompt, display, true, true, false);
        }
        // `/mcp` — select a local MCP server asset and enter local multi-turn
        // development mode; `/mcp <description>` drafts a local MCP asset.
        // OS publish/debug/test will map MCP tool calls to Function as a Service.
        if let Some(rest) = slash_tail(trimmed, "/mcp") {
            let description = rest.trim().to_string();
            self.textarea.clear();
            if let Some(parsed) = panels::mcp::parse_mcp_subcommand(&description) {
                return match parsed {
                    Ok(subcommand) => self.execute_mcp_subcommand(subcommand),
                    Err(e) => {
                        self.push_line(&Style::new().fg(TN_RED).render(&format!("  {e}")));
                        None
                    }
                };
            }
            if description.is_empty() {
                self.open_mcp_panel();
                return None;
            }
            let dir = mcp_dir();
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  ◆ drafting an MCP server asset → {} (then /mcp starts local dev)",
                dir.display()
            )));
            self.engage_autonomy(8);
            let prompt = panels::mcp::mcp_gen_prompt(&description, &dir.to_string_lossy());
            let display = format!("◆ mcp: {}", truncate(&description, 60));
            return self.start_stream_inner(prompt, display, true, true, false);
        }
        // `/skill` — select a local skill asset and enter local multi-turn
        // development mode; `/skill <description>` drafts a local skill asset.
        if let Some(rest) = slash_tail(trimmed, "/skill") {
            let description = rest.trim().to_string();
            self.textarea.clear();
            if let Some(parsed) = panels::skill::parse_skill_subcommand(&description) {
                return match parsed {
                    Ok(subcommand) => self.execute_skill_subcommand(subcommand),
                    Err(e) => {
                        self.push_line(&Style::new().fg(TN_RED).render(&format!("  {e}")));
                        None
                    }
                };
            }
            if description.is_empty() {
                self.open_skill_panel();
                return None;
            }
            let dir = skill_dir();
            self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                "  ✦ drafting a skill asset → {} (then /skill starts local dev)",
                dir.display()
            )));
            self.engage_autonomy(8);
            let prompt = panels::skill::skill_gen_prompt(&description, &dir.to_string_lossy());
            let display = format!("✦ skill: {}", truncate(&description, 60));
            return self.start_stream_inner(prompt, display, true, true, false);
        }
        // Slash commands run inline in any state.
        match trimmed {
            "/exit" => return Some(cmd::quit()),
            "/fork" => {
                // Branch a new session from the current one: copy the persisted
                // SessionData under a fresh id, then swap the active session to it
                // (Msg::Forked). The original id keeps its state, so it stays
                // resumable — the two diverge from here. Idle-only (guarded above),
                // so the last turn is already flushed to the store.
                self.textarea.clear();
                let store = self.store.clone();
                let src = self.session_id.clone();
                let dst = new_session_id();
                return Some(cmd::cmd(move || async move {
                    match store.load(&src).await {
                        Ok(Some(mut data)) => {
                            data.id = dst.clone();
                            match store.save(&data).await {
                                Ok(()) => Msg::Forked(Ok(dst)),
                                Err(e) => Msg::Forked(Err(format!("could not save the fork: {e}"))),
                            }
                        }
                        Ok(None) => Msg::Forked(Err(
                            "nothing to fork yet — start a conversation first".into(),
                        )),
                        Err(e) => Msg::Forked(Err(format!("could not read the session: {e}"))),
                    }
                }));
            }
            "/clear" => {
                self.messages.clear();
                self.plan.clear();
                self.runtime.clear_turn_entities();
                self.queue.clear();
                self.completed = 0;
                self.textarea.clear();
                // A fresh conversation can't deliver the old review's report
                // or sleep consolidation, and must not inherit a staged `/ctx`
                // window or stale hits.
                self.review_pending = false;
                self.sleep_pending = false;
                self.restore_autonomy();
                self.pending_ctx = None;
                self.ctx_hits.clear();
                self.agent_dev = None;
                self.pending_flow_subcommand = None;
                self.pending_agent_subcommand = None;
                self.mcp_dev = None;
                self.pending_mcp_subcommand = None;
                self.skill_dev = None;
                self.pending_skill_subcommand = None;
                self.okf_picker = None;
                self.pending_okf_subcommand = None;
                self.okf_dev = None;
                self.asset_list = None;
                self.runtime_activity = None;
                self.kb = None;
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
                        self.ctx_warned_tier = 0; // fresh window: re-arm fill warnings
                    }
                    Err(_) => self.session_id = prev_id,
                }
                self.relayout();
                self.rebuild_viewport();
                return None;
            }
            "/init" => {
                // Agent-driven: analyze the workspace and write AGENTS.md (auto-loaded
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
                     actually in the workspace, and write the file with your file-writing tool."
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
                        .with_timeout(BACKGROUND_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject);
                    let mut summary = String::new();
                    if let Ok(sess) = agent.session(workspace, Some(tui_session_options(conf))) {
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
                self.help_scroll = 0;
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
                // Signed in to OS + gateway models not fetched (or a previous fetch
                // failed → empty) → (re)fetch the OpenAI-compatible /v1/models so the
                // picker can offer the unified gateway, then open. A transient
                // gateway error thus recovers on the next /model instead of sticking
                // until restart. Otherwise open immediately.
                if let Some(s) = self.os_session.clone() {
                    let need_fetch = self.os_gateway_models.as_ref().is_none_or(|m| m.is_empty());
                    if need_fetch {
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
                self.ide = Some(Ide::browse(entries, "workspace"));
                return None;
            }
            "/plugin" => {
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
            "/memory" => {
                self.textarea.clear();
                // Open immediately ("loading…"); load the file snapshot off the
                // UI thread, with live session memory as a fallback.
                let dir = memory_dir();
                self.memory = Some(MemPanel {
                    entries: Vec::new(),
                    sel: 0,
                    details: std::collections::BTreeMap::new(),
                    graph: MemoryGraph::default(),
                    loaded_from_session: false,
                    detail: memutil::MemDetail::default(),
                    detail_scroll: 0,
                    dir: dir.clone(),
                    note: "loading…".into(),
                });
                return Some(self.load_memory_panel(dir));
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
        // One-shot `/ctx <n>` context: attach the staged transcript to THIS
        // genuine typed message only (never a `/loop` "Continue." re-entry),
        // invisibly — the display bubble above stays clean. Travels with the
        // message whether it runs now or is queued.
        let loop_cont = std::mem::take(&mut self.loop_continuation);
        let prompt = match (loop_cont, self.pending_ctx.take()) {
            (false, Some(c)) => format!("{c}\n\n{trimmed}"),
            _ => trimmed.to_string(),
        };
        let (prompt, display) = match &self.agent_dev {
            Some(dev) => (
                panels::agent::agent_dev_prompt(dev, &prompt),
                format!("◇ {}: {}", dev.name, truncate(trimmed, 60)),
            ),
            None => match &self.mcp_dev {
                Some(dev) => (
                    panels::mcp::mcp_dev_prompt(dev, &prompt),
                    format!("◆ {}: {}", dev.name, truncate(trimmed, 60)),
                ),
                None => match &self.skill_dev {
                    Some(dev) => (
                        panels::skill::skill_dev_prompt(dev, &prompt),
                        format!("✦ {}: {}", dev.name, truncate(trimmed, 60)),
                    ),
                    None => match &self.okf_dev {
                        Some(dev) => (
                            panels::okf::okf_dev_prompt(dev, &prompt),
                            format!("⌁ {}: {}", dev.name, truncate(trimmed, 60)),
                        ),
                        None => (prompt, trimmed.to_string()),
                    },
                },
            },
        };
        if self.state == State::Idle {
            self.start_stream_inner(prompt, display, true, true, false)
        } else {
            self.seq += 1;
            self.queue.push(Queued {
                prio: 1,
                seq: self.seq,
                text: prompt,
                display,
                runtime_expectation: None,
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
        self.start_stream_inner_with_runtime(
            prompt,
            display_task,
            clear_turn_artifacts,
            include_attachments,
            synthesis,
            None,
        )
    }

    fn start_stream_inner_with_runtime(
        &mut self,
        prompt: String,
        display_task: String,
        clear_turn_artifacts: bool,
        include_attachments: bool,
        synthesis: bool,
        runtime_expectation: Option<RuntimeExpectation>,
    ) -> Option<Cmd<Msg>> {
        self.streaming.clear();
        self.got_delta = false; // track if this turn streamed any text deltas
        self.turn_text.clear();
        self.turn_had_agent_activity = false;
        self.turn_text_after_activity = false;
        if let Some(expectation) = runtime_expectation {
            self.runtime_expectation = Some(expectation);
        }
        self.ultracode_synthesis_inflight = synthesis;
        if !synthesis {
            self.ultracode_synthesis_used = false;
        }
        self.last_paint = None; // first delta of the turn paints immediately
        self.viewport.set_auto_scroll(true); // sending a message jumps to latest
        if clear_turn_artifacts {
            self.plan.clear(); // fresh plan per user turn; planning events refill it
                               // Keep completed agents visible until the next user turn; a fresh
                               // user turn starts a fresh runtime-entity projection.
            self.runtime.clear_turn_entities();
        } else {
            self.runtime.clear_live_tools();
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
        // (A `/ctx <n>` staged transcript window is attached upstream, only to a
        // genuine typed user message — see on_submit — never to a `/loop`,
        // asset review, `?`, or synthesis continuation.)
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
                    Ok((rx, join)) => Msg::StreamStarted(Arc::new(Mutex::new(rx)), join),
                    Err(e) => Msg::StreamError(e.to_string()),
                }
            }),
            spinner_tick(),
        ]))
    }

    /// Pop the next queued message and start streaming it, if any.
    fn drain_queue(&mut self) -> Option<Cmd<Msg>> {
        let next = self.queue.pop()?;
        self.start_stream_inner_with_runtime(
            next.text,
            next.display,
            true,
            true,
            false,
            next.runtime_expectation,
        )
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
        self.warn_missing_runtime_evidence();
        let synthesis = self.prepare_ultracode_synthesis();
        self.finish();
        if let Some((prompt, display_task)) = synthesis {
            return self.start_ultracode_synthesis(prompt, display_task);
        }
        // Required OS Runtime evidence is a deliverable, not just a warning. In
        // autonomous runs, spend the next loop turn on a targeted correction
        // before falling back to the generic "Continue" prompt.
        if self.loop_remaining > 0 && self.queue.is_empty() {
            if let Some(prompt) = self
                .runtime_expectation
                .as_ref()
                .and_then(RuntimeExpectation::corrective_prompt)
            {
                self.loop_remaining -= 1;
                let n = self.loop_remaining;
                self.push_line(&Style::new().fg(TN_GRAY).render(&format!(
                    "  ↻ runtime evidence retry ({n} left · Esc to stop)"
                )));
                self.loop_continuation = true;
                return Some(cmd::msg(Msg::Submit(prompt)));
            }
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
            // Mark the continuation as machine-driven so on_submit doesn't
            // attach a staged `/ctx` window to it.
            self.loop_continuation = true;
            return Some(cmd::msg(Msg::Submit(
                "Continue. If the task is fully complete, reply DONE and stop.".to_string(),
            )));
        }
        // The loop is drained (or was never armed): an autonomous run that
        // auto-switched to auto mode is over — restore the user's mode.
        if self.loop_remaining == 0 {
            self.restore_autonomy();
        }
        // Run the next queued message (submitted while busy), if any.
        self.drain_queue()
    }

    fn resume_after_pending_confirmation(&self) -> Cmd<Msg> {
        resume_after_pending_confirmation_cmd(self.rx.clone())
    }

    /// An autonomous directive run is starting (/sleep, asset reviews,
    /// asset run/deploy, /flow drafts, /loop): switch to auto-approve so tool
    /// prompts can't stall it, and arm the loop budget that re-prompts until
    /// the deliverable lands. The prior mode is restored when the run ends
    /// (loop drained, interrupt, error, or /clear). A user already in auto
    /// mode keeps it — nothing is remembered or restored.
    fn engage_autonomy(&mut self, budget: usize) {
        self.loop_remaining = self.loop_remaining.max(budget);
        if self.mode != Mode::Auto {
            self.autonomy_restore = Some(self.mode);
            self.mode = Mode::Auto;
            self.push_line(&Style::new().fg(TN_GRAY).render(
                "  ⏵⏵ auto mode engaged for this task — restores when it completes (Esc stops)",
            ));
        }
    }

    fn record_runtime_tool_evidence(&mut self, name: &str) {
        if let Some(expectation) = &mut self.runtime_expectation {
            expectation.record_tool(name);
        }
    }

    fn record_runtime_parallel_evidence(&mut self) {
        if let Some(expectation) = &mut self.runtime_expectation {
            expectation.record_parallel_work();
        }
    }

    fn record_runtime_view_evidence(&mut self) {
        if let Some(expectation) = &mut self.runtime_expectation {
            expectation.record_remote_view();
        }
    }

    fn warn_missing_runtime_evidence(&mut self) {
        let warning = self
            .runtime_expectation
            .as_mut()
            .and_then(RuntimeExpectation::missing_warning);
        if let Some(warning) = warning {
            self.push_line(&Style::new().fg(TN_YELLOW).render(&warning));
        }
    }

    /// Restore the pre-autonomy mode (no-op when nothing was auto-switched).
    fn restore_autonomy(&mut self) {
        self.runtime_expectation = None;
        if let Some(prev) = self.autonomy_restore.take() {
            self.mode = prev;
            self.push_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render("  ⏵ autonomous task ended — auto mode restored to your previous mode"),
            );
        }
    }

    fn on_agent_event(&mut self, event: AgentEvent) -> Option<Cmd<Msg>> {
        // After an interrupt, rx is cleared — ignore any late buffered events.
        self.rx.as_ref()?;
        match event {
            AgentEvent::TextDelta { text } => {
                self.mark_assistant_text(&text);
                self.got_delta = true;
                self.turn_text.push_str(&text);
                self.streaming.push(&text);
                self.update_viewport_with_stream();
            }
            AgentEvent::ReasoningDelta { text } => {
                self.thinking.push_str(&text);
                self.update_viewport_with_stream();
            }
            AgentEvent::ToolStart { id, name } => {
                // Finalize any assistant text; show the tool live with a blinking
                // dot. The final "• action / └ result" lands on ToolEnd.
                self.mark_agent_activity();
                self.finalize_streaming();
                self.runtime.start_tool(id, name);
            }
            AgentEvent::ToolInputDelta { delta } => {
                self.runtime.push_tool_input(&delta);
            }
            AgentEvent::ToolOutputDelta { id, name, delta } => {
                self.runtime.push_tool_output(id, name, &delta);
                if let Some(output) = self
                    .runtime
                    .live_tool()
                    .map(|tool| tool.output().to_string())
                {
                    if let Some(spec) = self.find_remote_view_spec(&output) {
                        self.remember_remote_view(spec);
                    }
                }
                self.update_viewport_with_stream();
            }
            AgentEvent::ToolEnd {
                id,
                name,
                output,
                exit_code,
                metadata,
                ..
            } => {
                self.mark_agent_activity();
                let completed = self
                    .runtime
                    .end_tool(&id, name.clone(), output.clone(), exit_code);
                self.push_line(&render_tool_end(
                    &name,
                    exit_code,
                    &output,
                    metadata.as_ref(),
                    completed.args.as_ref(),
                    self.width as usize,
                ));
                self.record_runtime_tool_evidence(&name);
                self.capture_workflow(&name, completed.args.as_ref());
                if let Some(spec) = self.find_remote_view_spec(&output) {
                    self.remember_remote_view(spec);
                }
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
                self.record_runtime_parallel_evidence();
                // Track it in the live bottom panel instead of a transcript line.
                self.runtime
                    .start_subagent(task_id, agent, description, Instant::now());
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
                if let Some(t) = toks {
                    self.runtime.add_subagent_tokens(&task_id, t);
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
                self.runtime
                    .end_subagent(task_id, agent.clone(), success, Instant::now());
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
                // The core emits this whenever the threshold is crossed, even
                // when nothing could shrink (short histories can't summarize);
                // a note per round would spam "auto-compacted" while nothing
                // happened. Only surface real reductions — prune-only rounds
                // (equal count, smaller content) show up via ctx% instead.
                if after_messages < before_messages {
                    // `percent_before` is relative to the core's fixed 200k
                    // window; rescale to the model's REAL window to match ctx%.
                    let pct = if self.context_limit > 0 {
                        (percent_before * CORE_MAX_CONTEXT_TOKENS * 100.0
                            / self.context_limit as f32)
                            .round()
                            .min(100.0) as u32
                    } else {
                        (percent_before * 100.0).round() as u32
                    };
                    self.push_line(&Style::new().fg(TN_GRAY).italic().render(&format!(
                        "  ✦ context auto-compacted at {pct}% · {before_messages} → {after_messages} messages"
                    )));
                }
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
                    // frames per 80ms = the speed-up / slow-down cadence.
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
                self.runtime.remove_tool(&tool_id);
                self.approval_sel = 0;
                let label = tool_label(&tool_name, Some(&args));
                self.pending_tool = Some((tool_id, label));
                // Keep one pump parked on the event stream while awaiting input:
                // the confirmation can also resolve by timeout or an external
                // provider, and those events must clear the overlay.
                return self.rx.clone().map(pump);
            }
            AgentEvent::ConfirmationReceived {
                tool_id,
                approved,
                reason,
            } => {
                if let Some(label) = take_pending_tool_label(&mut self.pending_tool, &tool_id) {
                    self.state = State::Streaming;
                    if !approved {
                        let suffix = reason
                            .filter(|value| !value.trim().is_empty())
                            .map(|value| format!(" · {value}"))
                            .unwrap_or_default();
                        self.push_line(
                            &Style::new()
                                .fg(TN_RED)
                                .render(&format!("  ⎿ denied {label}{suffix}")),
                        );
                    }
                    return Some(self.resume_after_pending_confirmation());
                }
            }
            AgentEvent::ConfirmationTimeout {
                tool_id,
                action_taken,
            } => {
                if let Some(label) = take_pending_tool_label(&mut self.pending_tool, &tool_id) {
                    self.state = State::Streaming;
                    let (color, note) = if action_taken == "auto_approved" {
                        (
                            TN_YELLOW,
                            format!("  ⎿ confirmation timed out · auto-approved {label}"),
                        )
                    } else {
                        (
                            TN_RED,
                            format!("  ⎿ confirmation timed out · denied {label}"),
                        )
                    };
                    self.push_line(&Style::new().fg(color).render(&note));
                    return Some(self.resume_after_pending_confirmation());
                }
            }
            AgentEvent::PermissionDenied { tool_id, .. } => {
                self.runtime.remove_tool(&tool_id);
            }
            // Live context fill: every LLM round-trip reports its prompt size,
            // so ctx% (and the fill warnings) track DURING long multi-tool
            // turns instead of freezing until End.
            AgentEvent::TurnEnd { usage, .. } => {
                if usage.prompt_tokens > 0 {
                    self.last_prompt_tokens = usage.prompt_tokens;
                    self.maybe_warn_ctx();
                }
            }
            AgentEvent::End {
                text, usage, meta, ..
            } => {
                // /loop: stop once the agent signals completion (the word DONE).
                // Not during /sleep: its completion signal is the a3s-sleep
                // report itself, and consolidation narration ("what was done
                // today") would false-trigger this and end the run early.
                if self.loop_remaining > 0 && !self.sleep_pending {
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
                // Asset review scans the WHOLE turn's text: with a delta-only
                // provider a tool call after the report would have cleared the
                // live buffer, losing a fully delivered report.
                let review_text = if text.is_empty() {
                    self.turn_text.clone()
                } else {
                    text.clone()
                };
                // Only fall back to End.text when the provider never streamed
                // deltas this turn. Using the live buffer's emptiness here dups
                // text: a mid-turn finalize (e.g. a tool call) empties the buffer,
                // so End.text (the full message) would be appended a second time.
                if !self.got_delta && !text.is_empty() {
                    self.mark_assistant_text(&text);
                    self.streaming.push(&text);
                }
                self.finalize_streaming();
                // Asset code review: a ```a3s-review report in the final message
                // ends the review loop and opens the issue checklist.
                self.capture_review(&review_text);
                // `/sleep`: an ```a3s-sleep report ends the consolidation loop
                // and persists the distilled memories (async, batched below).
                let sleep_save = self.capture_sleep(&review_text);
                self.disarm_sleep_if_over(sleep_save.is_some());
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
                // ctx% is NOT updated here: End.usage.prompt_tokens is the
                // per-turn SUM of every round's prompt (the context is re-sent
                // each round, same ballooning as above), not the current
                // context size — a multi-round turn would read rounds× too
                // high and fire false fill warnings. The TurnEnd arm already
                // recorded the real per-round size, final round included.
                if self.model.is_none() {
                    self.model = meta.and_then(|m| m.response_model.or(m.request_model));
                }
                // Count the turn, idle, then continue /loop or drain the queue.
                // A captured sleep report's save runs alongside.
                return match (sleep_save, self.complete_turn()) {
                    (Some(save), Some(next)) => Some(cmd::batch(vec![save, next])),
                    (save, next) => save.or(next),
                };
            }
            AgentEvent::Error { message } => {
                self.push_line(
                    &Style::new()
                        .fg(TN_RED)
                        .render(&format!("  error: {message}")),
                );
                self.loop_remaining = 0; // a failed turn stops the /loop
                self.review_pending = false; // and abandons an asset review
                self.sleep_pending = false; // and a `/sleep` consolidation
                self.restore_autonomy();
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
            // TurnStart, ToolInputDelta, memory, confirmation echoes,
            // etc. — not surfaced in this MVP.
            _ => {}
        }
        // Keep draining the stream.
        self.rx.clone().map(pump)
    }

    /// One-shot transcript warning as the context fills: a heads-up at 70%
    /// and a red alert at 85% (the auto-compact point). Called wherever
    /// `last_prompt_tokens` updates; the latch re-arms when usage drops.
    fn maybe_warn_ctx(&mut self) {
        if self.context_limit == 0 {
            return;
        }
        let pct = (self.last_prompt_tokens * 100 / self.context_limit as usize).min(100);
        let (latch, warn) = ctx_warn_tier(pct, self.ctx_warned_tier);
        self.ctx_warned_tier = latch;
        if warn.is_some() {
            // push_line rebuilds the viewport from `messages` only, which
            // would hide a still-streaming round's text (invisible through a
            // whole approval wait if this round ends in a gated tool call).
            // Finalize it into the transcript first — same as ToolStart does.
            self.finalize_streaming();
        }
        match warn {
            Some(85) => self.push_line(&Style::new().fg(TN_RED).render(&format!(
                "  ✦ context {pct}% full — auto-compacting soon; /compact to summarize now"
            ))),
            Some(_) => self.push_line(&Style::new().fg(TN_YELLOW).render(&format!(
                "  ✦ context {pct}% full — auto-compacts near 85%; /compact to summarize early"
            ))),
            None => {}
        }
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

        let subagents = self.runtime.subagents();
        if !subagents.is_empty() {
            prompt.push_str("\nSubagents:\n");
            for agent in subagents {
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
        // Clear the parallel-subagent panel when the turn ends — it's a live
        // progress tracker, so leaving completed agents pinned at the bottom once
        // the work is done just clutters the idle screen.
        self.runtime.clear_turn_entities();
        self.ultracode_synthesis_inflight = false;
        self.relayout();
        self.stream_started = None;
        self.spinner.stop();
        self.rx = None;
        self.stream_join = None;
        self.interrupting = false;
        self.rebuild_viewport();
    }

    fn push_line(&mut self, line: &str) {
        self.messages.push(line.to_string());
        self.rebuild_viewport();
    }

    /// Open a OS viewUrl in the native `a3s-webview` window. Silent on success
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

    fn find_remote_view_spec(&self, output: &str) -> Option<remote_ui::ViewSpec> {
        // The progressive API returns a RELATIVE view url; complete it against
        // the signed-in OS origin (the TUI is "the edge").
        let os_origin = self
            .os_session
            .as_ref()
            .map(|s| crate::a3s_os::os_origin(&s.address));
        remote_ui::find_view_url(output, os_origin.as_deref())
    }

    fn remember_remote_view(&mut self, spec: remote_ui::ViewSpec) {
        // Remember the view for `/view`, and surface a clickable "Open view"
        // line ourselves (deterministic) rather than trusting the model to
        // print the marker — weaker models often forget it or jq the `.view`
        // object away.
        let is_new = is_new_remote_view(self.last_view.as_ref(), &spec);
        self.last_view = Some(spec.clone());
        self.record_runtime_view_evidence();
        if is_new {
            self.push_line(&gutter(
                ACCENT,
                &remote_view_button("click or /view to open"),
            ));
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
        // Login/logout flips whether the A3S Runtime `runtime` tool is available.
        self.sync_runtime_tool();
        let dirs = self.skill_dirs();
        self.skill_count = count_skill_files(&dirs);
        self.skills = load_skills(&dirs);
    }

    /// Register the A3S Runtime `runtime` offload tool while signed in to OS,
    /// unregister it while signed out — so it only appears in the model's toolset
    /// after login. Called after every auth change (login/logout), once the
    /// session has been (re)built.
    fn sync_runtime_tool(&self) {
        match self.os_session.as_ref() {
            Some(s) => self.session.register_dynamic_tool(std::sync::Arc::new(
                crate::runtime_tool::RuntimeTool::new(s),
            )),
            None => self.session.unregister_dynamic_tool("runtime"),
        }
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
        let mut ide = Ide::browse(ide_children(dir, 0), "config");
        ide.file = Some(IdeFile::new(path.to_path_buf(), lines, false, false));
        ide.focus_editor = true;
        self.ide = Some(ide);
    }

    /// Capture a `parallel_task`/`task` dispatch as a dynamic-workflow artifact:
    /// a readable plan of the fanned-out subtasks. Stored for `/flow workflow` and
    /// announced with a collapsed one-line message in the transcript.
    fn capture_workflow(&mut self, name: &str, args: Option<&serde_json::Value>) {
        let Some((doc, label)) = workflow_doc_for_tool(name, args) else {
            return;
        };
        self.last_workflow = Some(doc);
        // Collapsed indicator; the full artifact opens read-only via /flow workflow.
        self.push_line(&Style::new().fg(ACCENT).render(&format!("  ⊞ {label}")));
    }

    /// Open read-only text content in the built-in IDE (used by `/flow workflow` to
    /// show the dynamic-workflow artifact). Editor-focused for scroll/nav, but
    /// `readonly` blocks edits and Ctrl+S.
    fn open_readonly_in_ide(&mut self, title: &str, content: &str) {
        let lines: Vec<String> = content.lines().map(String::from).collect();
        let mut ide = Ide::browse(
            ide_children(std::path::Path::new(&self.cwd), 0),
            "workspace",
        );
        ide.file = Some(IdeFile::new(
            std::path::PathBuf::from(title),
            lines,
            false,
            true,
        ));
        ide.focus_editor = true;
        ide.flash = Some("read-only".to_string());
        self.ide = Some(ide);
    }

    /// Format every retained tool call for the `/output` viewer.
    fn format_tool_log(&self) -> Option<String> {
        format_tool_log_records(self.runtime.tool_log(), self.width as usize)
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
        // the animation ticks on the single-threaded loop (uneven cadence jitter).
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
        if let Some(tool) = self.runtime.live_tool() {
            let on = self.blink_tick % 8 < 4; // ~320ms on / 320ms off
            blocks.push(render_live_tool_status(
                &tool.name,
                tool.args().as_ref(),
                self.width as usize,
                on,
            ));
        }
        // Live stdout of the running tool — tail prefixed with "│" like Codex.
        if let Some(tool) = self.runtime.live_tool() {
            if let Some(body) = render_live_tool_output(tool.output(), self.width as usize) {
                blocks.push(body);
            }
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
        let menu = approval_menu_lines(label, self.approval_sel, self.width as usize);
        self.overlay_list(composed, &menu)
    }
}

fn approval_menu_lines(label: &str, selected: usize, width: usize) -> Vec<String> {
    MenuPanel::new(format!("⏵ Allow {label}?"))
        .items(vec![
            MenuItem::new("Yes"),
            MenuItem::new("Yes, and don't ask again"),
            MenuItem::new("No"),
        ])
        .selected(selected)
        .max_items(3)
        .show_scroll(false)
        .number_shortcuts(true)
        .indent(2)
        .marker("❯")
        .title_color(TN_YELLOW)
        .text_color(TN_FG)
        .muted_color(TN_GRAY)
        .selected_colors(Color::BrightWhite, ACCENT)
        .footer("Enter select · ↑/↓ · 1–3 · Esc")
        .view(width as u16, 5)
        .lines()
        .map(str::to_string)
        .collect()
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
    let context_limit = default_model
        .as_ref()
        .map(|m| ctx_limit_for_model(&model_ctx, m))
        .unwrap_or_else(|| resolve_ctx_limit(None));

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
    // The TUI is that manager (approve/deny modal, or /auto). Keep the human
    // confirmation wait separate from the tool execution timeout: reading and
    // deciding must not consume the tool's runtime budget.
    let confirmation = a3s_code_core::hitl::ConfirmationPolicy::enabled()
        .with_timeout(HITL_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject);
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
    // Past-session recall: when the ctx CLI is installed, teach the agent to
    // search local agent history before re-deriving prior work.
    let ctx_ready = panels::ctx::ctx_available();
    let with_instr = |o: SessionOptions| {
        let mut parts: Vec<String> = Vec::new();
        if let Some(i) = &instructions {
            parts.push(i.clone());
        }
        if let Some(addr) = &os_address {
            parts.push(os_platform_guide(addr));
        }
        if ctx_ready {
            parts.push(panels::ctx::ctx_history_guide());
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
            tui_session_options(confirmation.clone())
                .with_session_store(store.clone())
                .with_workspace_backend(workspace_services.clone())
                .with_skill_dirs(claude_dirs.clone())
                .with_auto_save(true)
                .with_auto_compact(true)
                // Scaled to the model's real window — the core triggers off a
                // fixed 200k, so a flat 0.85 would put the trigger past a
                // smaller window and auto-compact would never fire (see
                // `auto_compact_threshold_for`).
                .with_auto_compact_threshold(auto_compact_threshold_for(context_limit))
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
                tui_session_options(confirmation.clone())
                    .with_session_store(store.clone())
                    .with_session_id(session_id.as_str())
                    .with_workspace_backend(workspace_services.clone())
                    .with_skill_dirs(claude_dirs.clone())
                    .with_auto_save(true)
                    .with_auto_compact(true)
                    .with_auto_compact_threshold(auto_compact_threshold_for(context_limit))
                    .with_file_memory(memory_dir())
                    .with_max_parallel_tasks(8)
                    .with_auto_delegation_enabled(true)
                    .with_auto_parallel_delegation(true)
                    .with_manual_delegation_enabled(true),
                &workspace_manifest,
            ))),
        )?,
    };

    // A3S Runtime offload tool: registered only when signed in to OS, so the
    // model sees `runtime` after login and not before. Auth changes re-sync it via
    // `refresh_after_auth` → `sync_runtime_tool`.
    if let Some(os) = os_session.as_ref() {
        session.register_dynamic_tool(std::sync::Arc::new(crate::runtime_tool::RuntimeTool::new(
            os,
        )));
    }

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
        // readline line-editing (Ctrl+U = delete-to-start) in the input. PageUp/Down
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

    remote_ui::prime_webview_lookup();

    let mut app = App {
        session,
        agent: agent.clone(),
        store: store.clone(),
        confirmation,
        session_id: session_id.clone(),
        models,
        model_ctx,
        context_limit,
        last_prompt_tokens: 0,
        ctx_warned_tier: 0,
        model_menu: None,
        model_tab: 0,
        llm_override: None,
        os_config,
        os_session,
        os_refreshing: false,
        os_gateway_models: None,
        os_gateway_error: None,
        last_view: None,
        runtime_expectation: None,
        effort: 2, // high
        effort_panel: None,
        theme_panel: None,
        quit_armed: None,
        last_activity: Instant::now(),
        auto_reviewed: false,
        shell_mode: false,
        research_mode: false,
        review_pending: false,
        sleep_pending: false,
        review: None,
        review_open: false,
        flow: None,
        pending_flow_subcommand: None,
        agent_picker: None,
        pending_agent_subcommand: None,
        agent_dev: None,
        mcp_picker: None,
        pending_mcp_subcommand: None,
        mcp_dev: None,
        skill_picker: None,
        pending_skill_subcommand: None,
        skill_dev: None,
        okf_picker: None,
        pending_okf_subcommand: None,
        okf_dev: None,
        autonomy_restore: None,
        ctx_ready,
        ctx_hits: Vec::new(),
        pending_ctx: None,
        loop_continuation: false,
        turn_text: String::new(),
        selection: None,
        last_workflow: None,
        pending_images: Vec::new(),
        goal: None,
        goal_since: None,
        loop_remaining: 0,
        runtime: RuntimeProjection::default(),
        turn_had_agent_activity: false,
        turn_text_after_activity: false,
        ultracode_synthesis_inflight: false,
        ultracode_synthesis_used: false,
        instructions,
        workspace_manifest,
        workspace_manifest_rx,
        workspace_services,
        gradient_until: None,
        gradient_frame: 0,
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
        stream_join: None,
        interrupting: false,
        pending_tool: None,
        approval_sel: 0,
        history: history_seed,
        history_pos: None,
        model: default_model,
        output_tokens: 0,
        stream_started: None,
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
        ide: None,
        memory: None,
        asset_list: None,
        runtime_activity: None,
        kb: None,
        loop_panel: None,
        help_open: false,
        help_scroll: 0,
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

    // Apply the current effort (default `high`) to the launch session so the
    // FIRST turn already runs at the chosen depth. The session built above is
    // effort-naive — the scaled tool-round budget and the depth guideline live
    // only in effort_session_opts (reached via rebuild_session, as every
    // /effort switch does). Best-effort: keep the launch session if it can't
    // rebuild. (Resumes the same id, so transcript history is preserved.)
    let launch_model = app.model.clone();
    if let Ok((s, _)) = app.rebuild_session(launch_model.as_deref()) {
        app.session = Arc::new(s);
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
            Ok(bin) => {
                let restart_args = ["code", "resume", session_id.as_str()];
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
                    // exec replaces this process; only returns on failure → fall back.
                    let err = std::process::Command::new(&bin).args(restart_args).exec();
                    eprintln!(
                        "\n⚠  updated, but restart via {} failed: {err}",
                        bin.display()
                    );
                    if let Ok(exe) = std::env::current_exe() {
                        let err = std::process::Command::new(&exe).args(restart_args).exec();
                        eprintln!("⚠  fallback restart via {} failed: {err}", exe.display());
                    }
                    eprintln!(
                        "✓ updated to a3s {latest}; resume manually with: a3s code resume {session_id}\n"
                    );
                }
                #[cfg(not(unix))]
                {
                    match std::process::Command::new(&bin).args(restart_args).status() {
                        Ok(status) if status.success() => {}
                        Ok(status) => eprintln!(
                            "\n⚠  updated, but restart exited with status {status}; resume manually with: a3s code resume {session_id}\n"
                        ),
                        Err(err) => eprintln!(
                            "\n⚠  updated, but restart failed: {err}; resume manually with: a3s code resume {session_id}\n"
                        ),
                    }
                }
            }
            Err(error) => {
                eprintln!("\n✗ upgrade failed: {error}");
                eprintln!("get the latest from https://github.com/A3S-Lab/Cli/releases/latest\n");
            }
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

    fn rgb(color: Color) -> (u8, u8, u8) {
        match color {
            Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("expected RGB color, got {other:?}"),
        }
    }

    fn contains_cjk(s: &str) -> bool {
        s.chars().any(|ch| {
            ('\u{3400}'..='\u{4dbf}').contains(&ch)
                || ('\u{4e00}'..='\u{9fff}').contains(&ch)
                || ('\u{f900}'..='\u{faff}').contains(&ch)
        })
    }

    #[test]
    fn tui_palette_tracks_design_tokens() {
        assert_eq!(rgb(ACCENT), (0, 112, 243));
        assert_eq!(TN_GREEN, ACCENT);
        assert_eq!(rgb(TN_YELLOW), (245, 166, 35));
        assert_eq!(rgb(TN_RED), (238, 0, 0));
        assert_eq!(rgb(TN_CYAN), (80, 227, 194));
        assert_eq!(rgb(TN_FG), (237, 237, 237));
        assert_eq!(rgb(TN_GRAY), (143, 143, 143));
        assert_eq!(
            BRAND_GRADIENT,
            [
                Color::Rgb(0, 124, 240),
                Color::Rgb(0, 223, 216),
                TN_PURPLE,
                Color::Rgb(255, 0, 128),
                Color::Rgb(255, 77, 77),
                Color::Rgb(249, 203, 40),
            ]
        );
    }

    #[test]
    fn remote_view_button_is_styled_but_clickable_by_marker() {
        let rendered = remote_view_button("click or /view to open");
        let plain = a3s_tui::style::strip_ansi(&rendered);
        assert!(plain.contains(VIEW_BUTTON_MARKER), "{plain}");
        assert!(plain.contains("click or /view to open"), "{plain}");
        assert!(
            rendered.contains("\x1b["),
            "button should carry ANSI styling"
        );
    }

    #[test]
    fn footer_uses_shared_status_components() {
        let status = render_session_status_line(
            "/Users/roylin/code/a3s",
            Some("main"),
            Some("openai/gpt-5"),
            128_000,
            90_000,
            0,
            [SessionStatusChip::new("⚙", "1 running").color(TN_GRAY)],
            72,
        );
        let status_plain = a3s_tui::style::strip_ansi(&status);

        assert_eq!(a3s_tui::style::visible_len(&status), 72);
        assert!(status_plain.contains("a3s git:(main)"), "{status_plain}");
        assert!(status_plain.contains("gpt-5 (128k context)"), "{status_plain}");
        assert!(status_plain.contains("ctx:70%"), "{status_plain}");
        assert!(status_plain.contains("⚙ 1 running"), "{status_plain}");
        assert!(status.contains("\x1b["), "status should be styled");

        let mode = render_mode_status_line(Mode::Auto, 48);
        let mode_plain = a3s_tui::style::strip_ansi(&mode);
        assert_eq!(a3s_tui::style::visible_len(&mode), 48);
        assert!(mode_plain.starts_with("  ⏵⏵ auto mode on"), "{mode_plain}");
        assert!(mode_plain.contains("/help"), "{mode_plain}");
        assert!(mode.contains("\x1b["), "mode line should be styled");
    }

    #[test]
    fn tui_session_options_sets_separate_tool_timeout() {
        let confirmation = a3s_code_core::hitl::ConfirmationPolicy::enabled()
            .with_timeout(HITL_CONFIRM_TIMEOUT_MS, TimeoutAction::Reject);
        let opts = tui_session_options(confirmation);
        let dbg = format!("{opts:?}");

        assert_ne!(HITL_CONFIRM_TIMEOUT_MS, TOOL_EXEC_TIMEOUT_MS);
        assert!(
            dbg.contains(&format!("tool_timeout_ms: Some({TOOL_EXEC_TIMEOUT_MS})")),
            "{dbg}"
        );
    }

    #[test]
    fn approval_menu_uses_bounded_shared_panel() {
        let lines = approval_menu_lines(
            "Bash(cargo test very-long-filter-name-that-should-not-overflow)",
            1,
            42,
        );
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>();

        assert_eq!(plain.len(), 5);
        assert!(plain[0].contains("Allow"), "{plain:?}");
        assert!(plain[1].contains("1. Yes"), "{plain:?}");
        assert!(plain[2].contains("2. Yes, and"), "{plain:?}");
        assert!(plain[3].contains("3. No"), "{plain:?}");
        assert!(plain[4].contains("Enter select"), "{plain:?}");
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 42),
            "{plain:?}"
        );
        assert!(lines[2].contains("\x1b["), "selected row is styled");
    }

    #[test]
    fn effort_ladder_is_monotonic_and_well_formed() {
        // ULTRACODE indexes the last level, which is the ultracode profile.
        assert_eq!(ULTRACODE, EFFORT_LEVELS.len() - 1);
        assert_eq!(EFFORT_LEVELS[ULTRACODE].label, "ultracode");
        // Depth rises with effort: thinking budget and tool-round budget both
        // non-decreasing across low → max (so higher effort is never shallower).
        for w in EFFORT_LEVELS[..=ULTRACODE].windows(2) {
            assert!(
                w[1].thinking_budget >= w[0].thinking_budget,
                "thinking budget regressed"
            );
            assert!(
                w[1].max_tool_rounds >= w[0].max_tool_rounds,
                "tool-round budget regressed"
            );
            assert!(
                w[1].max_continuation_turns >= w[0].max_continuation_turns,
                "continuation budget regressed"
            );
        }
        // medium is the unsteered baseline; every other level carries a guideline
        // so effort is meaningful even on models with no thinking budget.
        assert!(
            EFFORT_LEVELS[1].guideline.is_none(),
            "medium should be the baseline"
        );
        for (i, p) in EFFORT_LEVELS.iter().enumerate() {
            if i != 1 {
                assert!(
                    p.guideline.is_some(),
                    "level {} has no depth steer",
                    p.label
                );
            }
        }
    }

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
        assert!(format_tool_log_records(&[], 80).is_none());
    }

    #[test]
    fn format_tool_log_renders_bounded_headers_args_and_output() {
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
        let out = format_tool_log_records(&recs, 48).unwrap();
        let plain = a3s_tui::style::strip_ansi(&out);
        assert!(plain.contains("#1 · Read /x · ok"), "{plain}");
        assert!(plain.contains("args: {\"file_path\":\"/x\"}"), "{plain}");
        assert!(
            plain.contains("    hello"),
            "output should be indented: {plain}"
        );
        assert!(plain.contains("#2 · Ran · exit 2"), "{plain}");
        for line in out.lines() {
            assert!(
                a3s_tui::style::visible_len(line) <= 48,
                "log line should be bounded: {line:?}"
            );
        }
    }

    #[test]
    fn format_tool_log_tails_long_output() {
        let recs = vec![ToolCallRecord {
            name: "bash".into(),
            args: Some(serde_json::json!({
                "command": "cargo test a-very-long-filter-that-must-not-overflow -- --nocapture"
            })),
            output: (0..30)
                .map(|i| format!("line-{i}-with-a-long-payload-that-must-stay-inside-the-editor"))
                .collect::<Vec<_>>()
                .join("\n"),
            exit_code: 0,
        }];
        let out = format_tool_log_records(&recs, 52).unwrap();
        let plain = a3s_tui::style::strip_ansi(&out);

        assert!(plain.contains("... +6 earlier lines"), "{plain}");
        assert!(!plain.contains("line-0-with"), "{plain}");
        assert!(plain.contains("line-6-with"), "{plain}");
        for line in out.lines() {
            assert!(
                a3s_tui::style::visible_len(line) <= 52,
                "log line should be bounded: {line:?}"
            );
        }
    }

    #[test]
    fn format_tool_log_respects_narrow_editor_width() {
        let recs = vec![ToolCallRecord {
            name: "bash".into(),
            args: Some(serde_json::json!({
                "command": "printf a-very-long-line-that-used-to-force-a-minimum-width"
            })),
            output: "a-very-long-output-line-that-must-be-clipped\n".into(),
            exit_code: 0,
        }];
        let out = format_tool_log_records(&recs, 16).unwrap();

        for line in out.lines() {
            assert!(
                a3s_tui::style::visible_len(line) <= 16,
                "log line should respect narrow width: {line:?}"
            );
        }
    }

    #[test]
    fn pending_tool_label_is_taken_only_for_matching_tool_id() {
        let mut pending = Some(("tool-a".to_string(), "edit file".to_string()));

        assert!(take_pending_tool_label(&mut pending, "tool-b").is_none());
        assert_eq!(
            pending,
            Some(("tool-a".to_string(), "edit file".to_string()))
        );

        assert_eq!(
            take_pending_tool_label(&mut pending, "tool-a"),
            Some("edit file".to_string())
        );
        assert!(pending.is_none());
    }

    #[tokio::test]
    async fn confirmation_resume_rearms_spinner_and_stream_pump() {
        let cmd = resume_after_pending_confirmation_cmd(None);
        match cmd.await {
            a3s_tui::cmd::CmdResult::Batch(cmds) => {
                assert_eq!(cmds.len(), 1, "spinner should resume without an rx");
            }
            _ => panic!("expected batched resume command"),
        }

        let (_tx, rx) = mpsc::channel::<AgentEvent>(1);
        let cmd = resume_after_pending_confirmation_cmd(Some(std::sync::Arc::new(
            tokio::sync::Mutex::new(rx),
        )));
        match cmd.await {
            a3s_tui::cmd::CmdResult::Batch(cmds) => {
                assert_eq!(cmds.len(), 2, "spinner and stream pump should resume");
            }
            _ => panic!("expected batched resume command"),
        }
    }

    // ── `?` deep-research mode ─────────────────────────────────────────────
    #[test]
    fn deep_research_prompt_directs_research_and_keeps_query() {
        let p = deep_research_prompt("rust async runtimes", false);
        assert!(p.contains("rust async runtimes"), "{p}");
        let lo = p.to_lowercase();
        assert!(lo.contains("deep research"), "{p}");
        assert!(lo.contains("web search") && lo.contains("web_fetch"), "{p}");
        assert!(lo.contains("source"), "should ask to cite sources: {p}");
        assert!(p.contains(".a3s/research/<slug>/"), "{p}");
        assert!(p.contains("standalone HTML"), "{p}");
    }

    #[test]
    fn deep_research_prompt_uses_os_runtime_and_remoteui_when_available() {
        let p = deep_research_prompt("rust async runtimes", true);
        assert!(p.contains("rust async runtimes"), "{p}");
        assert!(
            p.contains("OS A3S Runtime") && p.contains("parallel_task"),
            "{p}"
        );
        assert!(p.contains("Runtime evidence is required"), "{p}");
        assert!(p.contains("`runtime`"), "{p}");
        assert!(p.contains("shaped:true"), "{p}");
        assert!(p.contains("RemoteUI"), "{p}");
        assert!(p.contains(".view") && p.contains("viewUrl"), "{p}");
        assert!(p.contains("must include both fan-out"), "{p}");
        assert!(
            p.contains("Markdown report") && p.contains("HTML page"),
            "{p}"
        );
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
        // Wide glyphs are width-2: "あい" spans columns 0..4.
        assert_eq!(slice_cols("あい", 0, 2), "あ");
        assert_eq!(slice_cols("あい", 2, 4), "い");
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
    async fn hitl_wait_does_not_consume_tool_timeout_budget() {
        let dir = std::env::temp_dir().join(format!(
            "a3s-hitl-timeout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config.acl");
        test_config(&cfg);
        std::fs::write(dir.join("sample.txt"), "timeout sentinel").unwrap();

        let agent = a3s_code_core::Agent::new(cfg.to_string_lossy().to_string())
            .await
            .unwrap();
        let llm = Arc::new(CaptureLlmClient::new(vec![
            tool_call_response("read", serde_json::json!({"file_path": "sample.txt"})),
            done_response(),
        ]));
        let confirmation = a3s_code_core::hitl::ConfirmationPolicy::enabled()
            .with_timeout(5_000, TimeoutAction::Reject);
        let opts = tui_session_options(confirmation)
            .with_tool_timeout(300)
            .with_llm_client(llm)
            .with_permission_policy(a3s_code_core::permissions::PermissionPolicy::new())
            .with_planning_mode(a3s_code_core::PlanningMode::Disabled);
        let session = agent
            .session(dir.to_string_lossy().to_string(), Some(opts))
            .unwrap();

        let (mut rx, join) = session.stream("Read sample.txt.", None).await.unwrap();
        let mut saw_confirmation = false;
        let mut tool_output = None;
        while let Some(event) = rx.recv().await {
            match event {
                a3s_code_core::AgentEvent::ConfirmationRequired { tool_id, .. } => {
                    saw_confirmation = true;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    assert!(session
                        .confirm_tool_use(&tool_id, true, None)
                        .await
                        .unwrap());
                }
                a3s_code_core::AgentEvent::ToolEnd {
                    output, exit_code, ..
                } => {
                    assert_eq!(exit_code, 0, "{output}");
                    assert!(!output.contains("timed out"), "{output}");
                    tool_output = Some(output);
                }
                a3s_code_core::AgentEvent::End { .. } => break,
                a3s_code_core::AgentEvent::Error { message } => panic!("{message}"),
                _ => {}
            }
        }
        join.await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(saw_confirmation, "the tool call should require HITL");
        assert!(
            tool_output
                .as_deref()
                .is_some_and(|output| output.contains("timeout sentinel")),
            "tool output should come from read, got {tool_output:?}"
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
    fn estimate_tokens_counts_wide_unicode_heavier_than_ascii() {
        assert_eq!(estimate_tokens("abcd"), 1); // ASCII ~4 chars/token
        assert_eq!(estimate_tokens("かなテストあ"), 6); // wide text ~1 token/char
        assert_eq!(estimate_tokens("hi かな"), 2); // mixed: 3 ASCII -> 0, 2 wide -> 2
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn ctx_limit_falls_back_when_undeclared() {
        assert_eq!(resolve_ctx_limit(Some(200_000)), 200_000); // declared wins
        assert_eq!(resolve_ctx_limit(Some(0)), DEFAULT_CONTEXT_LIMIT); // zero -> default
        assert_eq!(resolve_ctx_limit(None), DEFAULT_CONTEXT_LIMIT); // missing -> default
    }

    #[test]
    fn ctx_limit_prefers_declared_then_infers_account_models() {
        let mut ctx = std::collections::HashMap::new();
        ctx.insert("openai/gpt-5".to_string(), 256_000);

        assert_eq!(ctx_limit_for_model(&ctx, "openai/gpt-5"), 256_000);
        assert_eq!(inferred_ctx_limit("claude-sonnet-4-6"), Some(200_000));
        assert_eq!(inferred_ctx_limit("claude-opus-4-8[1m]"), Some(1_000_000));
        assert_eq!(inferred_ctx_limit("gpt-4.1"), Some(1_000_000));
        assert_eq!(inferred_ctx_limit("glm-5.1"), Some(DEFAULT_CONTEXT_LIMIT));
        assert_eq!(
            ctx_limit_for_model(&ctx, "unknown-model"),
            DEFAULT_CONTEXT_LIMIT
        );
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
        // Small window: NOT floored past the window itself — 8k triggers at
        // 0.034 * 200k = 6.8k = 85% of 8k (the old 0.05 floor meant 10k > 8k,
        // i.e. compaction could never fire before overflow).
        assert!((auto_compact_threshold_for(8_000) - 0.034).abs() < 0.001);
    }

    #[test]
    fn ctx_warn_tier_latches_once_and_rearms_on_drop() {
        // Climb: 0 → warn at 70 tier → no re-warn inside the tier → warn at 85.
        assert_eq!(ctx_warn_tier(40, 0), (0, None));
        assert_eq!(ctx_warn_tier(72, 0), (70, Some(70)));
        assert_eq!(ctx_warn_tier(79, 70), (70, None)); // same tier: silent
        assert_eq!(ctx_warn_tier(91, 70), (85, Some(85)));
        assert_eq!(ctx_warn_tier(100, 85), (85, None));
        // Drop (compaction, /clear, wider model): latch re-arms.
        assert_eq!(ctx_warn_tier(30, 85), (0, None));
        assert_eq!(ctx_warn_tier(72, 0), (70, Some(70)));
        // Jump straight past both tiers warns the top one only.
        assert_eq!(ctx_warn_tier(90, 0), (85, Some(85)));
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
    fn slash_tail_requires_a_token_boundary() {
        let parameterized = [
            "/login", "/ctx", "/kb", "/okf", "/btw", "/goal", "/loop", "/sleep", "/flow", "/agent",
            "/mcp", "/skill",
        ];

        for cmd in parameterized {
            assert_eq!(slash_tail(cmd, cmd), Some(""), "{cmd} accepts bare form");
            assert_eq!(
                slash_tail(&format!("{cmd} argument"), cmd),
                Some(" argument"),
                "{cmd} accepts whitespace-delimited arguments"
            );
            assert_eq!(
                slash_tail(&format!("{cmd}x"), cmd),
                None,
                "{cmd}x must remain a normal message, not {cmd}"
            );
            assert_eq!(
                slash_tail(&format!("{cmd}-token"), cmd),
                None,
                "{cmd}-token must remain a normal message, not {cmd}"
            );
        }
    }

    #[test]
    fn cloned_asset_focus_matches_only_paths_inside_the_clone_root() {
        let clone_root = std::path::Path::new("/tmp/a3s-assets/weather-agent");
        assert!(App::path_is_within(clone_root, clone_root));
        assert!(App::path_is_within(
            std::path::Path::new("/tmp/a3s-assets/weather-agent/agent.md"),
            clone_root
        ));
        assert!(App::path_is_within(
            std::path::Path::new("/tmp/a3s-assets/weather-agent/nested/asset.json"),
            clone_root
        ));
        assert!(!App::path_is_within(
            std::path::Path::new("/tmp/a3s-assets/weather-agent-2/agent.md"),
            clone_root
        ));
    }

    #[test]
    fn runtime_asset_query_carries_asset_category_and_terms() {
        assert_eq!(
            runtime_asset_query("mcp", "Calc Tools", "failed calls"),
            "category:mcp Calc Tools failed calls"
        );
        assert_eq!(
            runtime_asset_query("workflow", "daily-flow", ""),
            "category:workflow daily-flow"
        );
        assert_eq!(runtime_asset_query("", "", "stale"), "stale");
    }

    #[test]
    fn slash_command_registry_is_unique_english_and_idle_safe() {
        let mut seen = HashSet::new();
        for (cmd, desc) in SLASH_COMMANDS {
            assert!(cmd.starts_with('/'), "{cmd} should be a slash command");
            assert!(
                !cmd.contains(char::is_whitespace),
                "{cmd} should be the bare command token"
            );
            assert!(seen.insert(*cmd), "{cmd} should not be registered twice");
            assert!(
                !desc.trim().is_empty(),
                "{cmd} should have a menu description"
            );
            assert!(
                !contains_cjk(desc),
                "{cmd} description should stay English-only: {desc}"
            );
            assert!(
                !desc.to_ascii_lowercase().contains("repo"),
                "{cmd} slash-menu copy should not expose asset-workspace management: {desc}"
            );
        }

        for cmd in IDLE_ONLY {
            assert!(
                SLASH_COMMANDS
                    .iter()
                    .any(|(registered, _)| registered == cmd),
                "{cmd} is idle-only but missing from the slash registry"
            );
        }

        let removed_commands = [
            "im", "run", "deploy", "review", "list", "ps", "workflow", "repo", "git", "relay",
        ]
        .into_iter()
        .map(|name| format!("/{name}"))
        .chain([
            format!("/{}{}", "evo", "lve"),
            format!("/{}{}", "evo", "love"),
        ]);
        for removed in removed_commands {
            assert!(
                !SLASH_COMMANDS
                    .iter()
                    .any(|(cmd, _)| *cmd == removed.as_str()),
                "{removed} should stay removed from the slash registry"
            );
        }
    }

    #[test]
    fn asset_root_commands_are_backed_by_lifecycle_services() {
        let asset_commands: HashSet<&str> = asset_lifecycle::ASSET_LIFECYCLES
            .iter()
            .map(|lifecycle| lifecycle.command)
            .collect();
        assert_eq!(
            asset_commands,
            HashSet::from(["/agent", "/mcp", "/skill", "/okf", "/flow"])
        );

        for command in asset_commands {
            let menu_desc = SLASH_COMMANDS
                .iter()
                .find_map(|(cmd, desc)| (*cmd == command).then_some(*desc))
                .unwrap_or_else(|| panic!("{command} should be registered in the slash menu"));
            let services: HashSet<&str> = asset_lifecycle::ASSET_LIFECYCLES
                .iter()
                .filter(|lifecycle| lifecycle.command == command)
                .map(|lifecycle| asset_lifecycle::service_label(lifecycle.service))
                .collect();

            for service in services {
                assert!(
                    menu_desc.contains(service),
                    "{command} slash-menu copy should name {service}: {menu_desc}"
                );
            }
            assert!(
                !menu_desc.contains("lifecycle"),
                "{command} slash-menu copy should name concrete OS services, not generic lifecycle wording: {menu_desc}"
            );
        }
    }

    #[test]
    fn cancel_pending_picker_clears_panel_and_deferred_asset_command() {
        let mut picker = Some("agent selector");
        let mut pending = Some("review");

        cancel_pending_picker(&mut picker, &mut pending);

        assert!(picker.is_none());
        assert!(pending.is_none());
    }

    #[test]
    fn registered_slash_commands_have_declared_handler_paths() {
        let parameterized = HashSet::from([
            "/login", "/ctx", "/kb", "/okf", "/btw", "/goal", "/loop", "/sleep", "/flow", "/agent",
            "/mcp", "/skill",
        ]);
        let exact = HashSet::from([
            "/logout", "/exit", "/fork", "/clear", "/init", "/compact", "/help", "/view", "/auto",
            "/config", "/model", "/effort", "/top", "/ide", "/plugin", "/theme", "/output",
            "/reload", "/update", "/memory",
        ]);

        for (cmd, _) in SLASH_COMMANDS {
            assert!(
                parameterized.contains(cmd) || exact.contains(cmd),
                "{cmd} is registered but not mapped to a handler category"
            );
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SlashHandlerKind {
        Exact,
        Parameterized,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SlashRuntimeScope {
        Local,
        OsAccount,
        RuntimeConditional,
    }

    #[derive(Clone, Copy, Debug)]
    struct SlashAuditRow {
        command: &'static str,
        handler: SlashHandlerKind,
        idle_only: bool,
        scope: SlashRuntimeScope,
    }

    fn slash_audit_rows() -> Vec<SlashAuditRow> {
        use SlashHandlerKind::{Exact, Parameterized};
        use SlashRuntimeScope::{Local, OsAccount, RuntimeConditional};

        vec![
            SlashAuditRow {
                command: "/model",
                handler: Exact,
                idle_only: true,
                scope: OsAccount,
            },
            SlashAuditRow {
                command: "/init",
                handler: Exact,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/config",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/theme",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/flow",
                handler: Parameterized,
                idle_only: true,
                scope: OsAccount,
            },
            SlashAuditRow {
                command: "/agent",
                handler: Parameterized,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/mcp",
                handler: Parameterized,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/skill",
                handler: Parameterized,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/okf",
                handler: Parameterized,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/output",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/login",
                handler: Parameterized,
                idle_only: false,
                scope: OsAccount,
            },
            SlashAuditRow {
                command: "/logout",
                handler: Exact,
                idle_only: false,
                scope: OsAccount,
            },
            SlashAuditRow {
                command: "/view",
                handler: Exact,
                idle_only: false,
                scope: OsAccount,
            },
            SlashAuditRow {
                command: "/plugin",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/reload",
                handler: Exact,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/update",
                handler: Exact,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/btw",
                handler: Parameterized,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/top",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/ide",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/memory",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/kb",
                handler: Parameterized,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/ctx",
                handler: Parameterized,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/effort",
                handler: Exact,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/compact",
                handler: Exact,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/goal",
                handler: Parameterized,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/loop",
                handler: Parameterized,
                idle_only: true,
                scope: RuntimeConditional,
            },
            SlashAuditRow {
                command: "/sleep",
                handler: Parameterized,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/help",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/fork",
                handler: Exact,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/clear",
                handler: Exact,
                idle_only: true,
                scope: Local,
            },
            SlashAuditRow {
                command: "/auto",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
            SlashAuditRow {
                command: "/exit",
                handler: Exact,
                idle_only: false,
                scope: Local,
            },
        ]
    }

    #[test]
    fn slash_command_audit_matrix_matches_registry_and_policies() {
        let rows = slash_audit_rows();
        let registered = SLASH_COMMANDS
            .iter()
            .map(|(cmd, _)| *cmd)
            .collect::<HashSet<_>>();
        let audited = rows.iter().map(|row| row.command).collect::<HashSet<_>>();

        assert_eq!(
            registered, audited,
            "every registered command must have explicit audit metadata"
        );

        let idle_from_rows = rows
            .iter()
            .filter(|row| row.idle_only)
            .map(|row| row.command)
            .collect::<HashSet<_>>();
        let idle_from_const = IDLE_ONLY.iter().copied().collect::<HashSet<_>>();
        assert_eq!(
            idle_from_rows, idle_from_const,
            "idle-only policy should stay in sync with the audit matrix"
        );

        let parameterized_names = HashSet::from([
            "/login", "/ctx", "/kb", "/okf", "/btw", "/goal", "/loop", "/sleep", "/flow", "/agent",
            "/mcp", "/skill",
        ]);
        for row in &rows {
            match row.handler {
                SlashHandlerKind::Parameterized => {
                    assert!(
                        parameterized_names.contains(row.command),
                        "{} should be in the token-boundary handler set",
                        row.command
                    );
                    assert!(
                        slash_tail(row.command, row.command).is_some(),
                        "{} should be token-boundary parsed",
                        row.command
                    );
                }
                SlashHandlerKind::Exact => {
                    assert!(
                        !parameterized_names.contains(row.command),
                        "{} exact command should not be in the token-boundary handler set",
                        row.command
                    );
                }
            }
        }

        let loop_row = rows.iter().find(|row| row.command == "/loop").unwrap();
        assert_eq!(loop_row.scope, SlashRuntimeScope::RuntimeConditional);
        for cmd in ["/agent", "/mcp", "/skill", "/okf", "/kb", "/ctx"] {
            let row = rows.iter().find(|row| row.command == cmd).unwrap();
            assert_eq!(row.scope, SlashRuntimeScope::Local);
        }
    }

    #[test]
    fn removed_top_level_aliases_stay_unregistered() {
        let removed = [
            "/plugins".to_string(),
            "/quit".to_string(),
            format!("/{}{}", "re", "po"),
            format!("/{}{}", "re", "lay"),
        ];
        for alias in removed {
            assert!(
                !SLASH_COMMANDS.iter().any(|(cmd, _)| *cmd == alias.as_str()),
                "{alias} should stay removed from the slash registry"
            );
        }
    }

    #[test]
    fn ampersand_clone_review_syntax_stays_removed() {
        assert!(
            slash_candidates("&").is_empty(),
            "asset clone shortcuts must not return to the slash menu"
        );
        assert!(
            !SLASH_COMMANDS.iter().any(|(cmd, _)| cmd.starts_with('&')),
            "asset clone/review flows must stay under typed asset subcommands"
        );
    }

    #[test]
    fn reload_is_idle_only_because_it_rebuilds_the_session() {
        assert!(IDLE_ONLY.contains(&"/reload"));
    }

    #[test]
    fn fork_is_idle_only_and_listed() {
        // /fork swaps the active session, so it must not run mid-stream…
        assert!(IDLE_ONLY.contains(&"/fork"));
        // …and it's offered in the slash menu.
        assert!(SLASH_COMMANDS.iter().any(|(name, _)| *name == "/fork"));
    }

    #[test]
    fn asset_workflow_commands_are_idle_only_and_listed() {
        for cmd in ["/flow", "/agent", "/mcp", "/skill", "/okf"] {
            assert!(
                IDLE_ONLY.contains(&cmd),
                "{cmd} must not arm asset workflows while another turn is running"
            );
            assert!(
                SLASH_COMMANDS.iter().any(|(name, _)| *name == cmd),
                "{cmd} should be visible in the slash menu while idle"
            );
        }
    }

    #[test]
    fn runtime_activity_are_asset_scoped_not_top_level() {
        let top_level_ps = format!("/{}", "ps");
        assert!(
            !SLASH_COMMANDS
                .iter()
                .any(|(name, _)| *name == top_level_ps.as_str()),
            "runtime activity browsing should stay asset-scoped"
        );
        assert!(matches!(
            panels::agent::parse_agent_subcommand("activity")
                .unwrap()
                .unwrap(),
            panels::agent::AgentSubcommand::Activity(_)
        ));
        assert!(panels::agent::parse_agent_subcommand("ps")
            .unwrap()
            .is_err());
        assert!(matches!(
            panels::mcp::parse_mcp_subcommand("activity")
                .unwrap()
                .unwrap(),
            panels::mcp::McpSubcommand::Activity(_)
        ));
        assert!(panels::mcp::parse_mcp_subcommand("ps").unwrap().is_err());
        assert!(matches!(
            panels::flow::parse_flow_subcommand("activity")
                .unwrap()
                .unwrap(),
            panels::flow::FlowSubcommand::Activity(_)
        ));
        assert!(panels::flow::parse_flow_subcommand("ps").unwrap().is_err());
        assert!(matches!(
            panels::skill::parse_skill_subcommand("activity")
                .unwrap()
                .unwrap(),
            panels::skill::SkillSubcommand::Activity(_)
        ));
        assert!(panels::skill::parse_skill_subcommand("ps")
            .unwrap()
            .is_err());
        assert!(matches!(
            panels::okf::parse_okf_command("activity"),
            panels::okf::OkfCommand::Activity(_)
        ));
        assert!(matches!(
            panels::okf::parse_okf_command("ps"),
            panels::okf::OkfCommand::Usage(_)
        ));
    }

    #[test]
    fn runtime_expectation_warns_once_until_evidence_arrives() {
        let mut missing = RuntimeExpectation::required("deep research");
        let warning = missing.missing_warning().unwrap();
        assert!(warning.contains("Runtime evidence missing"), "{warning}");
        assert!(missing.missing_warning().is_none());

        let mut via_runtime = RuntimeExpectation::required("run");
        via_runtime.record_tool("runtime");
        assert!(via_runtime.is_satisfied());
        assert!(via_runtime.missing_warning().is_none());

        let mut via_parallel = RuntimeExpectation::required("review");
        via_parallel.record_tool("parallel_task");
        assert!(via_parallel.is_satisfied());

        let mut via_view = RuntimeExpectation::required("deploy");
        via_view.record_remote_view();
        assert!(via_view.is_satisfied());

        let mut report_only_runtime = RuntimeExpectation::required_report_view("deep research");
        report_only_runtime.record_tool("runtime");
        assert!(!report_only_runtime.is_satisfied());
        let warning = report_only_runtime.missing_warning().unwrap();
        assert!(warning.contains("report"), "{warning}");
        assert!(warning.contains(".view"), "{warning}");
        let correction = report_only_runtime.corrective_prompt().unwrap();
        assert!(correction.contains("deep research"), "{correction}");
        assert!(correction.contains("OS Runtime"), "{correction}");
        assert!(correction.contains(".view"), "{correction}");
        assert!(correction.contains("viewUrl"), "{correction}");

        let mut report_only_view = RuntimeExpectation::required_report_view("loop daily-triage");
        report_only_view.record_remote_view();
        assert!(!report_only_view.is_satisfied());
        let warning = report_only_view.missing_warning().unwrap();
        assert!(warning.contains("fan-out"), "{warning}");
        let correction = report_only_view.corrective_prompt().unwrap();
        assert!(correction.contains("fan-out"), "{correction}");

        let mut full_report = RuntimeExpectation::required_report_view("deep research");
        full_report.record_tool("runtime");
        full_report.record_remote_view();
        assert!(full_report.is_satisfied());
        assert!(full_report.missing_warning().is_none());
        assert!(full_report.corrective_prompt().is_none());
    }

    #[test]
    fn remote_view_detection_only_marks_new_specs() {
        let spec = remote_ui::ViewSpec {
            url: "https://os.example.com/admin/runtime/jobs/1?embed=1".into(),
            width: Some(1200),
            height: Some(800),
            embeddable: true,
        };

        assert!(is_new_remote_view(None, &spec));
        assert!(!is_new_remote_view(Some(&spec), &spec));
    }

    #[test]
    fn os_required_message_distinguishes_missing_config_from_missing_login() {
        let configured = os_required_message("/agent run", true);
        assert!(configured.contains("/login"));
        assert!(!configured.contains("configure `os"));

        let missing = os_required_message("/agent deploy", false);
        assert!(missing.contains("configure `os"));
        assert!(missing.contains("/login"));
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
    fn char_byte_handles_ascii_and_wide_unicode() {
        assert_eq!(char_byte("hello", 0), 0);
        assert_eq!(char_byte("hello", 3), 3);
        assert_eq!(char_byte("hello", 5), 5); // past end clamps to len
                                              // These wide chars are 3 bytes each in UTF-8; cursor index 1 -> byte 3.
        assert_eq!(char_byte("あい", 1), 3);
        assert_eq!(char_byte("あい", 2), 6);
    }

    #[test]
    fn char_byte_supports_inplace_edits() {
        // Mirrors the /ide insert path: insert a wide char mid-string by char idx.
        let mut s = String::from("ab");
        let b = char_byte(&s, 1);
        s.insert(b, 'あ');
        assert_eq!(s, "aあb");
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
