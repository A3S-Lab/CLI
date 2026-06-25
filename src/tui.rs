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

/// Theme accent — ShuAn OS blue. Single source of truth for the UI accent color.
const ACCENT: Color = Color::Rgb(37, 99, 235);

/// Built-in slash commands shown in the `/` menu.
const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/model", "switch provider / model"),
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

/// The latest published version from GitHub releases (stripped of the `v`), or
/// `None` if offline / the lookup fails. Short timeout so startup never hangs.
async fn check_latest_version() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        std::process::Command::new("curl")
            .args([
                "-fsSL",
                "-m",
                "4",
                "https://api.github.com/repos/A3S-Lab/Cli/releases/latest",
            ])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
            .and_then(|v| {
                v.get("tag_name")?
                    .as_str()
                    .map(|s| s.trim_start_matches('v').to_string())
            })
    })
    .await
    .ok()
    .flatten()
}

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
/// A user-input message rendered with a subtle background "bubble" so it stands
/// out from agent output in the transcript.
fn user_bubble(content: &str, width: usize) -> String {
    let margin = " ".repeat(PAD);
    let bg = Color::Rgb(38, 45, 64);
    // Full-width bar (minus the outer margins) with inner left/right padding.
    let bar = width.saturating_sub(PAD * 2).max(8);
    content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let inner = if i == 0 {
                format!(" ● {line}")
            } else {
                format!("   {line}")
            };
            format!(
                "{margin}{}",
                Style::new()
                    .fg(Color::White)
                    .bg(bg)
                    .render(&pad_to(&inner, bar))
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

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

/// A resumable/relayable session from this or another coding agent.
/// Indent every line of `content` by `cols` spaces (keeps blocks off the edge).
fn indent(content: &str, cols: usize) -> String {
    let pad = " ".repeat(cols);
    content
        .lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
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

/// Byte offset of the char at index `char_idx` (for in-place string edits).
fn char_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
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

/// Map a file extension to a coarse language for syntax highlighting.
fn lang_of(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "rs" => "rust",
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => "js",
        "py" => "python",
        "go" => "go",
        "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" => "c",
        "sh" | "bash" | "zsh" => "sh",
        "toml" => "toml",
        _ => "",
    }
}

/// Keyword set per coarse language.
fn keywords(lang: &str) -> &'static [&'static str] {
    match lang {
        "rust" => &[
            "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "use", "mod", "match",
            "if", "else", "for", "while", "loop", "return", "const", "static", "async", "await",
            "move", "ref", "where", "as", "in", "crate", "super", "self", "Self", "type", "dyn",
            "unsafe", "extern", "break", "continue", "true", "false",
        ],
        "js" => &[
            "function",
            "const",
            "let",
            "var",
            "return",
            "if",
            "else",
            "for",
            "while",
            "class",
            "extends",
            "new",
            "async",
            "await",
            "import",
            "export",
            "from",
            "default",
            "try",
            "catch",
            "throw",
            "this",
            "typeof",
            "of",
            "in",
            "switch",
            "case",
            "break",
            "continue",
            "null",
            "undefined",
            "true",
            "false",
        ],
        "python" => &[
            "def", "class", "return", "if", "elif", "else", "for", "while", "import", "from", "as",
            "try", "except", "finally", "with", "lambda", "yield", "async", "await", "pass",
            "break", "continue", "raise", "global", "None", "True", "False", "and", "or", "not",
            "in", "is",
        ],
        "go" => &[
            "func",
            "var",
            "const",
            "type",
            "struct",
            "interface",
            "map",
            "chan",
            "go",
            "defer",
            "return",
            "if",
            "else",
            "for",
            "range",
            "switch",
            "case",
            "break",
            "continue",
            "package",
            "import",
            "nil",
            "true",
            "false",
        ],
        "c" => &[
            "int", "char", "void", "float", "double", "long", "short", "unsigned", "struct",
            "enum", "union", "const", "static", "return", "if", "else", "for", "while", "switch",
            "case", "break", "continue", "sizeof", "typedef",
        ],
        _ => &[],
    }
}

/// Lightweight per-line syntax highlighting → ANSI. Handles comments, strings,
/// numbers, keywords, types (CamelCase) and call sites. Single-line only.
/// Syntax-highlight palette for the IDE editor + diffs (`/theme` cycles these).
struct SyntaxTheme {
    name: &'static str,
    comment: Color,
    string: Color,
    number: Color,
    keyword: Color,
    typ: Color,
    func: Color,
}

/// Built-in themes; index 0 (Atom One Dark) is the default.
const THEMES: &[SyntaxTheme] = &[
    SyntaxTheme {
        name: "Atom One Dark",
        comment: Color::Rgb(92, 99, 112),
        string: Color::Rgb(152, 195, 121),
        number: Color::Rgb(209, 154, 102),
        keyword: Color::Rgb(198, 120, 221),
        typ: Color::Rgb(229, 192, 123),
        func: Color::Rgb(97, 175, 239),
    },
    SyntaxTheme {
        name: "Dracula",
        comment: Color::Rgb(98, 114, 164),
        string: Color::Rgb(241, 250, 140),
        number: Color::Rgb(189, 147, 249),
        keyword: Color::Rgb(255, 121, 198),
        typ: Color::Rgb(139, 233, 253),
        func: Color::Rgb(80, 250, 123),
    },
    SyntaxTheme {
        name: "Classic",
        comment: Color::BrightBlack,
        string: Color::Green,
        number: Color::Cyan,
        keyword: Color::Magenta,
        typ: Color::Yellow,
        func: Color::Blue,
    },
];

static SYNTAX_THEME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn current_theme() -> &'static SyntaxTheme {
    let i = SYNTAX_THEME
        .load(std::sync::atomic::Ordering::Relaxed)
        .min(THEMES.len() - 1);
    &THEMES[i]
}

fn highlight_code(line: &str, lang: &str) -> String {
    highlight_with(line, lang, current_theme())
}

fn highlight_with(line: &str, lang: &str, th: &SyntaxTheme) -> String {
    if lang.is_empty() {
        return line.to_string();
    }
    let kw = keywords(lang);
    let line_comment: &str = match lang {
        "python" | "sh" | "toml" => "#",
        "rust" | "js" | "go" | "c" => "//",
        _ => "",
    };
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Line comment → rest of the line.
        let is_comment = match line_comment {
            "//" => c == '/' && chars.get(i + 1) == Some(&'/'),
            "#" => c == '#',
            _ => false,
        };
        if is_comment {
            let rest: String = chars[i..].iter().collect();
            out.push_str(&Style::new().fg(th.comment).render(&rest));
            break;
        }
        // String literal.
        if c == '"' || c == '\'' || c == '`' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push_str(&Style::new().fg(th.string).render(&s));
            continue;
        }
        // Number.
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push_str(&Style::new().fg(th.number).render(&s));
            continue;
        }
        // Identifier / keyword / type / call.
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let styled = if kw.contains(&word.as_str()) {
                Style::new().fg(th.keyword).render(&word)
            } else if chars.get(i) == Some(&'(') {
                Style::new().fg(th.func).render(&word)
            } else if word.chars().next().is_some_and(|c| c.is_uppercase()) {
                Style::new().fg(th.typ).render(&word)
            } else {
                word
            };
            out.push_str(&styled);
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
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

/// True if `path` looks like a previewable raster image.
fn is_image_path(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
    )
}

/// Render an image as Unicode half-block lines — each line is one text row with
/// fg = the upper pixel and bg = the lower pixel, so it drops cleanly into the
/// line-based renderer and works in any truecolor terminal.
fn render_image_blocks(img: &image::DynamicImage, max_cols: usize, max_rows: usize) -> Vec<String> {
    use image::GenericImageView;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 || max_cols == 0 || max_rows == 0 {
        return Vec::new();
    }
    // Two vertical pixels per text row.
    let scale = (max_cols as f32 / w as f32)
        .min((max_rows * 2) as f32 / h as f32)
        .clamp(0.001, 1.0);
    let nw = ((w as f32 * scale) as u32).max(1);
    let nh = ((h as f32 * scale) as u32).max(2);
    let rgba = img
        .resize_exact(nw, nh, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let mut lines = Vec::new();
    let mut y = 0;
    while y < nh {
        let mut line = String::new();
        for x in 0..nw {
            let t = *rgba.get_pixel(x, y);
            let b = if y + 1 < nh {
                *rgba.get_pixel(x, y + 1)
            } else {
                t
            };
            line.push_str(
                &Style::new()
                    .fg(Color::Rgb(t[0], t[1], t[2]))
                    .bg(Color::Rgb(b[0], b[1], b[2]))
                    .render("▀"),
            );
        }
        lines.push(line);
        y += 2;
    }
    lines
}

/// Half-block preview of an image file, or `None` if it can't be decoded.
fn render_image_file(
    path: &std::path::Path,
    max_cols: usize,
    max_rows: usize,
) -> Option<Vec<String>> {
    let img = image::open(path).ok()?;
    Some(render_image_blocks(&img, max_cols, max_rows))
}

/// Write the macOS clipboard image to `dest` as PNG. Returns false (and cleans
/// up) if the clipboard holds no image. ponytail: macOS-only via osascript.
fn clipboard_image_to(dest: &std::path::Path) -> bool {
    let path = dest.to_string_lossy();
    let ok = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!("set f to open for access POSIX file \"{path}\" with write permission"),
            "-e",
            "write (the clipboard as «class PNGf») to f",
            "-e",
            "close access f",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let nonempty = std::fs::metadata(dest)
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    if ok && nonempty {
        true
    } else {
        let _ = std::fs::remove_file(dest);
        false
    }
}

/// Current git branch of `dir` (cheap: parse `.git/HEAD`), if any.
fn git_branch(dir: &str) -> Option<String> {
    let head = std::fs::read_to_string(format!("{dir}/.git/HEAD")).ok()?;
    head.strip_prefix("ref: refs/heads/")
        .map(|b| b.trim().to_string())
}

/// A changed file in the `/git` panel: porcelain X (staged) + Y (unstaged).
#[derive(Clone)]
struct GitFile {
    x: char,
    y: char,
    path: String,
}

impl GitFile {
    fn staged(&self) -> bool {
        self.x != ' ' && self.x != '?'
    }
    fn untracked(&self) -> bool {
        self.x == '?'
    }
}

#[derive(Clone, Copy, PartialEq)]
enum GitView {
    Status,
    Log,
}

/// State of the `/git` full-screen panel (a small gitui-style view).
struct Git {
    files: Vec<GitFile>,
    sel: usize,
    /// Right-pane content: the selected file's diff, or the selected commit's
    /// details in the Log view.
    diff: Vec<String>,
    diff_scroll: usize,
    log: Vec<String>,
    log_sel: usize,
    view: GitView,
    /// `Some` while the user is typing a commit message.
    commit_input: Option<String>,
    note: String,
}

/// Run a git subcommand in `repo`, returning stdout (+ stderr on failure).
async fn run_git(repo: String, args: Vec<String>) -> String {
    match tokio::process::Command::new("git")
        .current_dir(&repo)
        .args(&args)
        .output()
        .await
    {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            if !o.status.success() {
                s.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            s
        }
        Err(e) => format!("git error: {e}"),
    }
}

/// Working-tree status (porcelain) + recent log for the `/git` panel.
async fn git_status_log(repo: String) -> (Vec<GitFile>, Vec<String>) {
    let status = run_git(
        repo.clone(),
        vec![
            "status".into(),
            "--porcelain=v1".into(),
            "--untracked-files=all".into(),
        ],
    )
    .await;
    let files = status
        .lines()
        .filter_map(|l| {
            let b = l.as_bytes();
            if b.len() < 4 {
                return None;
            }
            Some(GitFile {
                x: b[0] as char,
                y: b[1] as char,
                path: l[3..].to_string(),
            })
        })
        .collect();
    let log = run_git(
        repo,
        vec![
            "log".into(),
            "--oneline".into(),
            "-n".into(),
            "30".into(),
            "--no-color".into(),
        ],
    )
    .await
    .lines()
    .map(String::from)
    .collect();
    (files, log)
}

/// Diff for one file (whole change vs HEAD; untracked shown as all-added).
async fn git_diff_file(repo: String, file: GitFile) -> Vec<String> {
    let args = if file.untracked() {
        vec![
            "diff".into(),
            "--no-color".into(),
            "--no-index".into(),
            "--".into(),
            "/dev/null".into(),
            file.path.clone(),
        ]
    } else {
        vec![
            "diff".into(),
            "--no-color".into(),
            "HEAD".into(),
            "--".into(),
            file.path.clone(),
        ]
    };
    run_git(repo, args)
        .await
        .lines()
        .map(String::from)
        .collect()
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

/// A fresh session id for each launch (timestamp + pid; UUID-ish, no dep).
fn new_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:016x}-{:x}", nanos, std::process::id())
}

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

/// Render `text` with a soft highlight gliding left→right (loading shimmer).
/// Each glyph's colour is interpolated from the base accent up to a bright tint
/// by its distance from the moving head, giving a gradient glow rather than a
/// hard band. `phase` is divided down so the glide is slow and gentle.
fn shimmer(text: &str, phase: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return String::new();
    }
    let span = (n + 12) as isize;
    let head = (phase as isize / 3) % span; // sweep speed; +12 = pause between sweeps
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        // Smooth falloff over ~5 glyphs either side of the head.
        let d = (head - i as isize).abs() as f32;
        let t = (1.0 - d / 5.0).clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
        // ACCENT (37,99,235) → bright tint (228,236,255).
        let mut s = Style::new().fg(Color::Rgb(
            lerp(37.0, 228.0),
            lerp(99.0, 236.0),
            lerp(235.0, 255.0),
        ));
        if t > 0.65 {
            s = s.bold();
        }
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

/// Compact token count: `50700` → `50.7k`.
fn fmt_tokens(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
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
                let label = self
                    .pending_tool
                    .as_ref()
                    .map(|(_, l)| l.as_str())
                    .unwrap_or("this tool");
                Style::new().fg(Color::Yellow).bold().render(&format!(
                    "  ⏵ Allow {label}?   (y) yes · (a) always · (n) no · Esc"
                ))
            }
            State::Idle => String::new(),
        };

        let prompt = Style::new().fg(icolor).bold().render(&format!("{sym} "));
        let typed = self.textarea.view();
        let typed = if sym == "!" || inp.starts_with("/btw") {
            Style::new().fg(icolor).render(&typed)
        } else {
            typed
        };
        let input_view = format!("{}{}{}", " ".repeat(PAD), prompt, typed);

        // Bottom status bar (two lines): cwd/branch + model/tokens, then mode + hints.
        let dir = self.cwd.rsplit('/').next().unwrap_or(&self.cwd);
        let mut ctx = format!("  {dir}");
        if let Some(b) = &self.branch {
            ctx.push_str(&format!("  ⎇ {b}"));
        }
        if let Some(g) = &self.goal {
            let short: String = g.chars().take(24).collect();
            ctx.push_str(&format!("  🎯 {short}"));
        }
        if self.loop_remaining > 0 {
            ctx.push_str(&format!("  ↻{}", self.loop_remaining));
        }
        // Live parallelism: running subagents + tools.
        if self.active_agents > 0 {
            ctx.push_str(&format!("  ⇉ {} agents", self.active_agents));
        }
        if self.active_tools > 0 {
            ctx.push_str(&format!("  ⚙ {} running", self.active_tools));
        }
        if let Some(v) = &self.update_available {
            ctx.push_str(&format!("  ⬆ {v}"));
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
            .item(&input_view, Constraint::Fixed(1))
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
        // input sits above: border, status×2, and the bottom task panel.
        let row = self
            .height
            .saturating_sub(4 + self.task_lines().len() as u16);
        let col = (PAD + 2) as u16 + self.textarea.cursor_display_col() as u16; // PAD + "❯ "
        Some((col, row))
    }
}

impl App {
    /// True when the `/` command menu should be shown (idle, single-line input
    /// starting with `/` that matches at least one command).
    /// Built-in commands + loaded skills (as `/<skill>`) matching `input`.
    fn slash_candidates_all(&self, input: &str) -> Vec<(String, String)> {
        // Hide session-mutating commands while a turn is streaming.
        let idle = self.state == State::Idle;
        let mut out: Vec<(String, String)> = slash_candidates(input)
            .into_iter()
            .filter(|(c, _)| idle || !IDLE_ONLY.contains(c))
            .map(|(c, d)| (c.to_string(), d.to_string()))
            .collect();
        for (name, desc) in &self.skills {
            if self.disabled_skills.contains(name) {
                continue; // hidden via /plugins
            }
            let cmd = format!("/{name}");
            if cmd.starts_with(input) {
                out.push((cmd, format!("skill · {}", truncate(desc, 56))));
            }
        }
        out
    }

    fn slash_menu_open(&self) -> bool {
        let input = self.textarea.value();
        // Available while idle OR streaming (so /btw can fire mid-turn); not
        // during an approval prompt.
        self.state != State::Awaiting
            && input.starts_with('/')
            && !input.contains('\n')
            // Close once args are being typed (e.g. "/btw <prompt>") so Enter
            // submits the whole line instead of just the command.
            && !input.contains(' ')
            && !self.slash_candidates_all(&input).is_empty()
    }

    /// Keys while the slash menu is open: ↑/↓ select, Enter run, Tab complete,
    /// Esc dismiss. Returns `Some(handled)` to consume the key.
    fn handle_slash_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let cands = self.slash_candidates_all(&self.textarea.value());
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
                let cmd = cands[self.slash_sel].0.clone();
                self.slash_sel = 0;
                self.textarea.clear();
                // Run directly on this key event so the redraw is immediate (a
                // Submit message would be frame-throttled and look like a no-op).
                if cmd == "/model" {
                    self.open_model_menu();
                    return Some(None);
                }
                // A skill (not a built-in command) → ask the agent to use it.
                if !SLASH_COMMANDS.iter().any(|(c, _)| *c == cmd) {
                    let name = cmd.trim_start_matches('/');
                    return Some(self.on_submit(format!("Use your `{name}` skill.")));
                }
                Some(self.on_submit(cmd))
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
        let cands = self.slash_candidates_all(&self.textarea.value());
        let total = cands.len();
        let sel = self.slash_sel.min(total - 1);
        let width = self.width as usize;
        // Cap the menu height (skills make the list long) and scroll a window so
        // the selection stays visible — Claude-Code style.
        let max_rows = (self.height as usize).saturating_sub(8).clamp(3, 10);
        let start = if sel < max_rows {
            0
        } else {
            sel + 1 - max_rows
        };
        let end = (start + max_rows).min(total);
        let mut menu: Vec<String> = (start..end)
            .map(|i| {
                let (cmd, desc) = &cands[i];
                let raw = pad_to(&format!("  {cmd:<11} {desc}"), width);
                if i == sel {
                    Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
                } else {
                    Style::new().fg(Color::BrightBlack).render(&raw)
                }
            })
            .collect();
        if total > max_rows {
            // Scroll position footer: ↑ if more above, ↓ if more below.
            let up = if start > 0 { "↑" } else { " " };
            let down = if end < total { "↓" } else { " " };
            menu.push(pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!("  {up}{down} {}/{total}", sel + 1)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }

    /// The `@<query>` after the last `@` in the input (no whitespace), if any.
    fn at_query(&self) -> Option<String> {
        let val = self.textarea.value();
        let at = val.rfind('@')?;
        let after = &val[at + 1..];
        if after.contains(char::is_whitespace) {
            None
        } else {
            Some(after.to_string())
        }
    }

    /// Workspace files matching the current `@` query (substring match).
    fn file_candidates(&self) -> Vec<String> {
        let Some(q) = self.at_query() else {
            return Vec::new();
        };
        let q = q.to_lowercase();
        // Sorted so same-directory files group together for the tree view; the
        // overlay scrolls a window, so we can keep plenty for browsing.
        let mut v: Vec<String> = self
            .files
            .iter()
            .filter(|f| q.is_empty() || f.to_lowercase().contains(&q))
            .take(400)
            .cloned()
            .collect();
        v.sort();
        v
    }

    fn file_menu_open(&self) -> bool {
        self.state != State::Awaiting
            && !self.textarea.value().contains('\n')
            && self.at_query().is_some()
            && !self.file_candidates().is_empty()
    }

    /// Keys while the `@` file picker is open: ↑/↓ select, Enter/Tab insert,
    /// Esc dismiss (drops the trailing `@query`).
    fn handle_file_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let cands = self.file_candidates();
        if cands.is_empty() {
            return None;
        }
        let last = cands.len() - 1;
        self.file_sel = self.file_sel.min(last);
        match key.code {
            KeyCode::Up => {
                self.file_sel = self.file_sel.saturating_sub(1);
                Some(None)
            }
            KeyCode::Down => {
                self.file_sel = (self.file_sel + 1).min(last);
                Some(None)
            }
            KeyCode::Enter | KeyCode::Tab => {
                let val = self.textarea.value();
                if let Some(at) = val.rfind('@') {
                    let picked = &cands[self.file_sel];
                    self.textarea
                        .set_value(&format!("{}@{picked} ", &val[..at]));
                }
                self.file_sel = 0;
                Some(None)
            }
            KeyCode::Esc => {
                let val = self.textarea.value();
                if let Some(at) = val.rfind('@') {
                    self.textarea.set_value(&val[..at]);
                }
                self.file_sel = 0;
                Some(None)
            }
            _ => None,
        }
    }

    /// Overlay the `@` file picker just above the input box.
    fn overlay_file_menu(&self, composed: String) -> String {
        if !self.file_menu_open() {
            return composed;
        }
        let cands = self.file_candidates();
        let total = cands.len();
        if total == 0 {
            return composed;
        }
        let sel = self.file_sel.min(total - 1);
        let width = self.width as usize;
        // Cap height + scroll a window (like the / menu); group by directory and
        // indent the files so it reads as a tree, not a flat path list.
        let max_rows = (self.height as usize).saturating_sub(9).clamp(3, 8);
        let start = if sel < max_rows {
            0
        } else {
            sel + 1 - max_rows
        };
        let end = (start + max_rows).min(total);

        let mut menu = vec![pad_to(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  @ file · ↑/↓ · Enter insert · Esc"),
            width,
        )];
        let mut last_dir: Option<String> = None;
        for (i, f) in cands.iter().enumerate().take(end).skip(start) {
            let (dir, base) = match f.rsplit_once('/') {
                Some((d, b)) => (d.to_string(), b.to_string()),
                None => (String::new(), f.clone()),
            };
            // Directory header (full path, cyan) whenever the group changes.
            if last_dir.as_deref() != Some(dir.as_str()) {
                let label = if dir.is_empty() {
                    "./".to_string()
                } else {
                    format!("{dir}/")
                };
                menu.push(pad_to(
                    &Style::new().fg(Color::Cyan).render(&format!("  {label}")),
                    width,
                ));
                last_dir = Some(dir);
            }
            let raw = pad_to(&format!("    {base}"), width);
            menu.push(if i == sel {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(Color::White).render(&raw)
            });
        }
        if total > max_rows {
            let up = if start > 0 { "↑" } else { " " };
            let down = if end < total { "↓" } else { " " };
            menu.push(pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!("  {up}{down} {}/{total}", sel + 1)),
                width,
            ));
        }
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
    /// Base session options carrying the current effort. `ultracode` turns on
    /// planning + parallel subagent delegation (a3s-code PTC), so a turn plans a
    /// dynamic workflow and fans tasks out to multiple subagents.
    fn effort_session_opts(&self, thinking: bool) -> SessionOptions {
        let mut opts = SessionOptions::new()
            .with_session_store(self.store.clone())
            .with_session_id(self.session_id.as_str())
            .with_confirmation_policy(self.confirmation.clone())
            .with_auto_save(true)
            // Auto-compact the context when it nears the window (Claude-style).
            .with_auto_compact(true)
            .with_auto_compact_threshold(0.85);
        // Keep project instructions (CLAUDE.md) + any /compact summary across
        // model/effort/compact rebuilds, injected into the system prompt.
        let extra = match (&self.instructions, &self.compact_summary) {
            (Some(i), Some(s)) => Some(format!("{i}\n\n# Earlier conversation (compacted)\n\n{s}")),
            (Some(i), None) => Some(i.clone()),
            (None, Some(s)) => Some(format!("# Earlier conversation (compacted)\n\n{s}")),
            (None, None) => None,
        };
        if let Some(e) = extra {
            opts = opts.with_prompt_slots(SystemPromptSlots::default().with_extra(e));
        }
        // Extended thinking is Anthropic-only; only request it when asked.
        if thinking {
            opts = opts.with_thinking_budget(EFFORT_LEVELS[self.effort].1);
        }
        if self.effort == ULTRACODE {
            opts = opts
                .with_planning_mode(a3s_code_core::PlanningMode::Enabled)
                .with_goal_tracking(true)
                .with_max_parallel_tasks(8)
                .with_auto_delegation_enabled(true)
                .with_auto_parallel_delegation(true)
                .with_max_tool_rounds(40);
        }
        opts
    }

    /// Rebuild the session under the current effort. Tries with the thinking
    /// budget, then falls back without it (so models that don't support extended
    /// thinking don't error). Returns (session, thinking_dropped).
    fn rebuild_session(&self, model: Option<&str>) -> Result<(AgentSession, bool), String> {
        let build = |thinking: bool| {
            let o = self.effort_session_opts(thinking);
            match model {
                Some(m) => o.with_model(m),
                None => o,
            }
        };
        // Resume keeps history if the session was saved. Before the first turn
        // it isn't in the store ("Session not found"), so fall back to a fresh
        // session with the same id (no turns yet = no history to lose). Each is
        // also retried without the thinking budget for non-Anthropic models.
        for thinking in [true, false] {
            if let Ok(s) = self
                .agent
                .resume_session(self.session_id.as_str(), build(thinking))
            {
                return Ok((s, !thinking));
            }
            if let Ok(s) = self.agent.session(self.cwd.clone(), Some(build(thinking))) {
                return Ok((s, !thinking));
            }
        }
        Err("could not rebuild the session".into())
    }

    fn switch_model(&mut self, model: &str) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(Color::Yellow)
                    .render("  finish the current turn before switching models"),
            );
            return;
        }
        match self.rebuild_session(Some(model)) {
            Ok((s, _)) => {
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

    /// Apply the selected effort by rebuilding the session (keeps model + history).
    fn apply_effort(&mut self) {
        if self.state != State::Idle {
            self.push_line(
                &Style::new()
                    .fg(Color::Yellow)
                    .render("  finish the current turn before changing effort"),
            );
            return;
        }
        let model = self.model.clone();
        match self.rebuild_session(model.as_deref()) {
            Ok((s, dropped)) => {
                self.session = Arc::new(s);
                if self.effort == ULTRACODE {
                    // Unattended fan-out: auto-approve so subagents run freely.
                    self.mode = Mode::Auto;
                    self.rainbow_until = Some(Instant::now()); // rainbow flourish
                    self.rainbow_frame = 0;
                    self.push_line(&Style::new().fg(ACCENT).bold().render(
                        "  ◆ ultracode — planning a dynamic workflow + parallel subagents (auto-approve on)",
                    ));
                } else if dropped {
                    self.push_line(&Style::new().fg(Color::BrightBlack).render(&format!(
                        "  ◇ effort: {} (this model uses its default depth)",
                        EFFORT_LEVELS[self.effort].0
                    )));
                } else {
                    self.push_line(
                        &Style::new()
                            .fg(Color::Green)
                            .render(&format!("  ◇ effort: {}", EFFORT_LEVELS[self.effort].0)),
                    );
                }
            }
            Err(e) => self.push_line(
                &Style::new()
                    .fg(Color::Red)
                    .render(&format!("  failed to set effort: {e}")),
            ),
        }
    }

    /// The `/effort` slider panel (overlaid like the model picker).
    /// `/plugins` panel: enable/disable Claude skills (checkbox list, scrolled).
    fn overlay_plugins(&self, composed: String) -> String {
        let Some(sel) = self.plugins_panel else {
            return composed;
        };
        let total = self.skills.len();
        if total == 0 {
            return composed;
        }
        let sel = sel.min(total - 1);
        let width = self.width as usize;
        let on_count = total - self.disabled_skills.len().min(total);
        let mut menu = vec![pad_to(
            &Style::new().fg(ACCENT).bold().render(&format!(
                "  Plugins & skills ({on_count}/{total} on) — ↑/↓ · Space toggle · Esc"
            )),
            width,
        )];
        let max_rows = (self.height as usize).saturating_sub(8).clamp(3, 12);
        let start = if sel < max_rows {
            0
        } else {
            sel + 1 - max_rows
        };
        let end = (start + max_rows).min(total);
        let descw = width.saturating_sub(28);
        for i in start..end {
            let (name, desc) = &self.skills[i];
            let on = !self.disabled_skills.contains(name);
            let marker = if i == sel { "▸" } else { " " };
            let check = if on {
                Style::new().fg(Color::Green).render("[✓]")
            } else {
                Style::new().fg(Color::BrightBlack).render("[ ]")
            };
            let nm_plain = format!("{:<16}", truncate(&format!("/{name}"), 16));
            let nm = if on {
                Style::new().fg(Color::Cyan).render(&nm_plain)
            } else {
                Style::new().fg(Color::BrightBlack).render(&nm_plain)
            };
            let raw = format!(
                "  {marker} {check} {nm}  {}",
                Style::new()
                    .fg(Color::BrightBlack)
                    .render(&truncate(desc, descw)),
            );
            menu.push(pad_to(&raw, width));
        }
        if total > max_rows {
            let up = if start > 0 { "↑" } else { " " };
            let down = if end < total { "↓" } else { " " };
            menu.push(pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!("  {up}{down} {}/{total}", sel + 1)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }

    /// `/theme` picker: a theme list + a live syntax-highlight preview.
    fn overlay_theme(&self, composed: String) -> String {
        let Some(sel) = self.theme_panel else {
            return composed;
        };
        let width = self.width as usize;
        let mut menu = vec![pad_to(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  Theme — ↑/↓ preview · Enter apply · Esc"),
            width,
        )];
        for (i, th) in THEMES.iter().enumerate() {
            let marker = if i == sel { "▸" } else { " " };
            let raw = pad_to(&format!("  {marker} {}", th.name), width);
            menu.push(if i == sel {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(Color::BrightBlack).render(&raw)
            });
        }
        menu.push(pad_to(
            &Style::new()
                .fg(Color::BrightBlack)
                .render("  ── preview ──"),
            width,
        ));
        let th = &THEMES[sel];
        let sample = [
            "// syntax preview",
            "fn compute(n: usize) -> String {",
            "    let total = n * 42;",
            "    format!(\"sum: {}\", total)",
            "}",
        ];
        for line in sample {
            menu.push(pad_to(
                &format!("    {}", highlight_with(line, "rust", th)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }

    fn overlay_effort(&self, composed: String) -> String {
        let Some(sel) = self.effort_panel else {
            return composed;
        };
        let width = self.width as usize;
        // Ultracode confirm flourish: a rainbow "⚡ ULTRACODE ⚡" burst.
        if self.effort_anim.is_some() {
            const PALETTE: [Color; 7] = [
                Color::Rgb(255, 0, 0),
                Color::Rgb(255, 127, 0),
                Color::Rgb(255, 255, 0),
                Color::Rgb(0, 220, 0),
                Color::Rgb(0, 150, 255),
                Color::Rgb(75, 0, 200),
                Color::Rgb(160, 0, 230),
            ];
            let f = self.rainbow_frame;
            let title = "⚡  U L T R A C O D E  ⚡";
            let colored: String = title
                .chars()
                .enumerate()
                .map(|(i, ch)| {
                    Style::new()
                        .fg(PALETTE[(i + f) % PALETTE.len()])
                        .bold()
                        .render(&ch.to_string())
                })
                .collect();
            let barw = width.saturating_sub(8).max(8);
            let wave: String = (0..barw)
                .map(|i| {
                    Style::new()
                        .fg(PALETTE[(i + f) % PALETTE.len()])
                        .bold()
                        .render("━")
                })
                .collect();
            let center = |s: &str, vis: usize| {
                let pad = width.saturating_sub(vis) / 2;
                format!("{}{s}", " ".repeat(pad))
            };
            let menu = vec![
                String::new(),
                format!("    {wave}"),
                String::new(),
                center(&colored, title.chars().count()),
                String::new(),
                center(
                    &Style::new()
                        .fg(Color::BrightBlack)
                        .render("planning a dynamic workflow · dispatching parallel subagents"),
                    61,
                ),
                String::new(),
                format!("    {wave}"),
            ];
            return self.overlay_list(composed, &menu);
        }
        let n = EFFORT_LEVELS.len();
        // Fill (almost) the whole width.
        let track_w = width.saturating_sub(8).max(n * 9);
        let posf = |i: usize| {
            if n > 1 {
                i * (track_w - 1) / (n - 1)
            } else {
                0
            }
        };
        let pos = posf(sel);
        // Track with a ▲ at the selected level and a ┆ divider before ultracode.
        let mut track: Vec<char> = "─".repeat(track_w).chars().collect();
        let div = (posf(ULTRACODE - 1) + posf(ULTRACODE)) / 2;
        if div < track.len() {
            track[div] = '┆';
        }
        if pos < track.len() {
            track[pos] = '▲';
        }
        let track: String = track.iter().collect();
        // Level names centred under their tick, each in its own colour
        // (faster→smarter gradient; ultracode is magenta).
        let level_colors = [
            Color::Green,
            Color::Cyan,
            Color::Blue,
            Color::Yellow,
            Color::Rgb(255, 140, 0),
            Color::Magenta,
        ];
        let mut labels = String::new();
        let mut vis = 0usize;
        for (i, (name, _)) in EFFORT_LEVELS.iter().enumerate() {
            let nw = name.chars().count();
            let start = posf(i).saturating_sub(nw / 2);
            while vis < start {
                labels.push(' ');
                vis += 1;
            }
            let c = level_colors[i.min(level_colors.len() - 1)];
            let st = if i == sel {
                Style::new().fg(c).bold()
            } else {
                Style::new().fg(c)
            };
            labels.push_str(&st.render(name));
            vis += nw;
        }
        let faster_smarter = format!("Faster{}Smarter", " ".repeat(track_w.saturating_sub(13)));
        let desc = if sel == ULTRACODE {
            "ultracode: plans a dynamic workflow and runs tasks on parallel subagents (PTC)."
        } else {
            "higher effort = more reasoning tokens (slower, deeper). Use sparingly."
        };
        let dim = |s: &str| Style::new().fg(Color::BrightBlack).render(s);
        let menu = vec![
            pad_to(&Style::new().fg(ACCENT).bold().render("  Effort"), width),
            pad_to(&format!("    {}", dim(&faster_smarter)), width),
            pad_to(
                &format!("    {}", Style::new().fg(Color::White).render(&track)),
                width,
            ),
            pad_to(&format!("    {labels}"), width),
            pad_to(
                &Style::new()
                    .fg(ACCENT)
                    .bold()
                    .render(&format!("    ▸ {}", EFFORT_LEVELS[sel].0)),
                width,
            ),
            pad_to(&format!("    {}", dim(desc)), width),
            pad_to(&dim("  ←/→ adjust · Enter confirm · Esc cancel"), width),
        ];
        self.overlay_list(composed, &menu)
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

    /// The agent tabs, always shown (even when a tab has no sessions) so the
    /// user can switch between them and see each agent's history.
    fn relay_tabs(&self) -> Vec<&'static str> {
        vec!["a3s-code", "claude code", "codex"]
    }

    /// Indices into `self.relay` for the sessions under the active tab.
    fn relay_tab_indices(&self) -> Vec<usize> {
        let tabs = self.relay_tabs();
        let Some(agent) = tabs.get(self.relay_tab).copied() else {
            return Vec::new();
        };
        self.relay
            .iter()
            .enumerate()
            .filter(|(_, s)| s.agent == agent)
            .map(|(i, _)| i)
            .collect()
    }

    fn handle_relay_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let sel = self.relay_menu?;
        let tabs = self.relay_tabs();
        let last = self.relay_tab_indices().len().saturating_sub(1);
        match key.code {
            // ←/→ switch agent tab, resetting the row selection.
            KeyCode::Left => {
                self.relay_tab = self.relay_tab.saturating_sub(1);
                self.relay_menu = Some(0);
                Some(None)
            }
            KeyCode::Right => {
                self.relay_tab = (self.relay_tab + 1).min(tabs.len().saturating_sub(1));
                self.relay_menu = Some(0);
                Some(None)
            }
            KeyCode::Up => {
                self.relay_menu = Some(sel.saturating_sub(1));
                Some(None)
            }
            KeyCode::Down => {
                self.relay_menu = Some((sel + 1).min(last));
                Some(None)
            }
            KeyCode::Enter => {
                let idxs = self.relay_tab_indices();
                self.relay_menu = None;
                idxs.get(sel.min(last)).map(|&i| self.relay_select(i))
            }
            KeyCode::Esc => {
                self.relay_menu = None;
                Some(None)
            }
            _ => None,
        }
    }

    /// Resume a native a3s-code session, or continue a foreign agent's task here.
    fn relay_select(&mut self, idx: usize) -> Option<Cmd<Msg>> {
        let (native_id, seed, agent) = {
            let s = self.relay.get(idx)?;
            (s.native_id.clone(), s.seed.clone(), s.agent)
        };
        if let Some(id) = native_id {
            let mut opts = SessionOptions::new()
                .with_session_store(self.store.clone())
                .with_session_id(id.as_str())
                .with_confirmation_policy(self.confirmation.clone())
                .with_auto_save(true);
            // Resume under the CURRENT model, not whatever the saved session used
            // (e.g. a smoke-test's gpt-4o that this config doesn't have).
            if let Some(m) = self.model.clone().or_else(|| self.models.first().cloned()) {
                opts = opts.with_model(&m);
            }
            match self.agent.resume_session(id.as_str(), opts) {
                Ok(sess) => {
                    self.session = Arc::new(sess);
                    self.session_id = id.clone();
                    self.messages.clear();
                    let w = (self.width as usize).saturating_sub(PAD + 2);
                    for m in self.session.history() {
                        let text = m.text();
                        if text.trim().is_empty() {
                            continue;
                        }
                        match m.role.as_str() {
                            "user" => self.messages.push(gutter(ACCENT, text.trim())),
                            "assistant" => {
                                let mut md = StreamingMarkdown::new(w);
                                md.push(&text);
                                self.messages.push(gutter(Color::Green, &md.view()));
                            }
                            _ => {}
                        }
                    }
                    self.push_line(
                        &Style::new()
                            .fg(Color::Green)
                            .render(&format!("  ⮌ resumed a3s-code session {id}")),
                    );
                }
                Err(e) => self.push_line(
                    &Style::new()
                        .fg(Color::Red)
                        .render(&format!("  failed to resume: {e}")),
                ),
            }
            None
        } else if let Some(seed) = seed {
            if self.state != State::Idle {
                self.push_line(
                    &Style::new()
                        .fg(Color::Yellow)
                        .render("  finish the current turn before relaying"),
                );
                return None;
            }
            self.messages.push(gutter(
                Color::Magenta,
                &format!("⮌ relaying from {agent}: {}", truncate(&seed, 60)),
            ));
            self.start_stream(format!(
                "The following task was last being worked on in {agent}. Analyze where it \
                 left off, then continue and finish the unfinished work:\n\n{seed}"
            ))
        } else {
            None
        }
    }

    fn overlay_relay_menu(&self, composed: String) -> String {
        let Some(sel) = self.relay_menu else {
            return composed;
        };
        let tabs = self.relay_tabs();
        if tabs.is_empty() {
            return composed;
        }
        let width = self.width as usize;
        let active = tabs.get(self.relay_tab).copied().unwrap_or("");

        // Tab strip: each agent in its theme colour; the active one boxed.
        let mut strip = String::from("  ");
        for t in &tabs {
            let c = agent_color(t);
            if *t == active {
                strip.push_str(
                    &Style::new()
                        .fg(Color::Black)
                        .bg(c)
                        .bold()
                        .render(&format!(" {t} ")),
                );
            } else {
                strip.push_str(&Style::new().fg(c).render(&format!(" {t} ")));
            }
            strip.push(' ');
        }
        let mut menu = vec![
            pad_to(&strip, width),
            pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render("  ←/→ agent · ↑/↓ session · Enter continue · Esc"),
                width,
            ),
        ];

        let idxs = self.relay_tab_indices();
        let color = agent_color(active);
        if idxs.is_empty() {
            menu.push(pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!("    (no {active} sessions for this directory)")),
                width,
            ));
        }
        for (row, &gi) in idxs.iter().enumerate().take(12) {
            let s = &self.relay[gi];
            let raw = pad_to(
                &format!("  {}", truncate(&s.label, width.saturating_sub(4))),
                width,
            );
            menu.push(if row == sel.min(idxs.len().saturating_sub(1)) {
                Style::new().fg(Color::Black).bg(color).render(&raw)
            } else {
                Style::new().fg(color).render(&raw)
            });
        }
        self.overlay_list(composed, &menu)
    }

    /// Full-screen `/top` process monitor; coding-agent rows are highlighted.
    fn render_top_panel(&self, rows: &[ProcRow]) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let agents = rows.iter().filter(|r| r.agent.is_some()).count();
        let title = Style::new().fg(ACCENT).bold().render(&format!(
            "  /top — {} processes · {agents} coding agent(s) · Enter to kill",
            rows.len()
        ));
        let mut out = vec![
            pad_to(&title, width),
            pad_to(
                &Style::new().fg(Color::BrightBlack).render(
                    "  PID      CPU%   MEM%   COMMAND                        Esc close · ↑/↓ select",
                ),
                width,
            ),
        ];
        let body = h.saturating_sub(3);
        let start = self
            .top_scroll
            .min(rows.len().saturating_sub(body.min(rows.len())));
        for (i, r) in rows.iter().enumerate().skip(start).take(body) {
            let cmd = truncate(&r.cmd, width.saturating_sub(44).max(10));
            let tag = r.agent.map(|a| format!("   ◀ {a}")).unwrap_or_default();
            let raw = pad_to(
                &format!("  {:<7} {:>5.1}  {:>5.1}   {cmd}{tag}", r.pid, r.cpu, r.mem),
                width,
            );
            // Agent rows wear their brand colour; the selected row inverts it.
            let color = r.agent.map(agent_color).unwrap_or(Color::White);
            let styled = if i == self.top_sel {
                Style::new().fg(Color::Black).bg(color).bold().render(&raw)
            } else if r.agent.is_some() {
                Style::new().fg(color).bold().render(&raw)
            } else {
                Style::new().fg(Color::White).render(&raw)
            };
            out.push(styled);
        }
        while out.len() < h {
            out.push(String::new());
        }
        out.truncate(h);

        // Force-kill confirmation: a bright dialog box centred on the panel.
        if let Some((pid, cmd)) = &self.top_kill {
            let bw = 44.min(width.saturating_sub(2)).max(20);
            let inner = bw - 2;
            let vis = a3s_tui::style::visible_len;
            let center = |s: &str| {
                let pad = inner.saturating_sub(vis(s)) / 2;
                format!("{}{s}{}", " ".repeat(pad), " ".repeat(inner - pad - vis(s)))
            };
            let bx = [
                format!("┌{}┐", "─".repeat(inner)),
                format!("│{}│", center("⚠  FORCE-KILL THIS PROCESS?")),
                format!("│{}│", center("")),
                format!("│{}│", center(&format!("PID {pid}"))),
                format!("│{}│", center(&truncate(cmd, inner.saturating_sub(4)))),
                format!("│{}│", center("")),
                format!("│{}│", center("[ Y ] yes        [ N ] no")),
                format!("└{}┘", "─".repeat(inner)),
            ];
            let row0 = h.saturating_sub(bx.len()) / 2;
            let col0 = width.saturating_sub(bw) / 2;
            for (k, line) in bx.iter().enumerate() {
                if let Some(slot) = out.get_mut(row0 + k) {
                    let styled = Style::new()
                        .fg(Color::BrightWhite)
                        .bg(Color::Red)
                        .bold()
                        .render(line);
                    *slot = format!("{}{styled}", " ".repeat(col0));
                }
            }
        }
        out.join("\n")
    }

    /// Handle a key while the `/ide` panel is open. Returns true if consumed.
    fn ide_key(&mut self, key: &KeyEvent) -> bool {
        if self.ide.is_none() {
            return false;
        }
        // Esc leaves the editor first (back to the tree), then closes the panel.
        if key.code == KeyCode::Esc {
            let editing = self.ide.as_ref().is_some_and(|i| i.focus_editor);
            if editing {
                if let Some(i) = self.ide.as_mut() {
                    i.focus_editor = false;
                }
            } else {
                self.ide = None;
            }
            return true;
        }
        let h = self.height as usize;
        let w = self.width as usize;
        let ide = self.ide.as_mut().unwrap();
        match key.code {
            // Editor focused: full text editing of the open file.
            _ if ide.focus_editor && ide.file.is_some() => {
                let body = h.saturating_sub(2);
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let f = ide.file.as_mut().unwrap();
                if f.image {
                    return true; // image preview is read-only
                }
                let nlines = f.lines.len();
                match key.code {
                    // Ctrl+S saves to disk.
                    KeyCode::Char('s') if ctrl => {
                        let content = format!("{}\n", f.lines.join("\n"));
                        if std::fs::write(&f.path, content).is_ok() {
                            f.dirty = false;
                        }
                    }
                    KeyCode::Up => f.row = f.row.saturating_sub(1),
                    KeyCode::Down => f.row = (f.row + 1).min(nlines.saturating_sub(1)),
                    KeyCode::Left => {
                        if f.col > 0 {
                            f.col -= 1;
                        } else if f.row > 0 {
                            f.row -= 1;
                            f.col = f.lines[f.row].chars().count();
                        }
                    }
                    KeyCode::Right => {
                        let len = f.lines.get(f.row).map_or(0, |l| l.chars().count());
                        if f.col < len {
                            f.col += 1;
                        } else if f.row + 1 < nlines {
                            f.row += 1;
                            f.col = 0;
                        }
                    }
                    KeyCode::Home => f.col = 0,
                    KeyCode::End => f.col = f.lines.get(f.row).map_or(0, |l| l.chars().count()),
                    KeyCode::PageUp => f.row = f.row.saturating_sub(body),
                    KeyCode::PageDown => f.row = (f.row + body).min(nlines.saturating_sub(1)),
                    KeyCode::Char(c) => {
                        let b = char_byte(&f.lines[f.row], f.col);
                        f.lines[f.row].insert(b, c);
                        f.col += 1;
                        f.dirty = true;
                    }
                    KeyCode::Tab => {
                        let b = char_byte(&f.lines[f.row], f.col);
                        f.lines[f.row].insert_str(b, "    ");
                        f.col += 4;
                        f.dirty = true;
                    }
                    KeyCode::Enter => {
                        let b = char_byte(&f.lines[f.row], f.col);
                        let right = f.lines[f.row].split_off(b);
                        f.lines.insert(f.row + 1, right);
                        f.row += 1;
                        f.col = 0;
                        f.dirty = true;
                    }
                    KeyCode::Backspace => {
                        if f.col > 0 {
                            let b0 = char_byte(&f.lines[f.row], f.col - 1);
                            let b1 = char_byte(&f.lines[f.row], f.col);
                            f.lines[f.row].replace_range(b0..b1, "");
                            f.col -= 1;
                            f.dirty = true;
                        } else if f.row > 0 {
                            let cur = f.lines.remove(f.row);
                            f.row -= 1;
                            f.col = f.lines[f.row].chars().count();
                            f.lines[f.row].push_str(&cur);
                            f.dirty = true;
                        }
                    }
                    KeyCode::Delete => {
                        let len = f.lines[f.row].chars().count();
                        if f.col < len {
                            let b0 = char_byte(&f.lines[f.row], f.col);
                            let b1 = char_byte(&f.lines[f.row], f.col + 1);
                            f.lines[f.row].replace_range(b0..b1, "");
                            f.dirty = true;
                        } else if f.row + 1 < nlines {
                            let next = f.lines.remove(f.row + 1);
                            f.lines[f.row].push_str(&next);
                            f.dirty = true;
                        }
                    }
                    _ => {}
                }
                // Clamp cursor column + scroll the cursor into view.
                let len = f.lines.get(f.row).map_or(0, |l| l.chars().count());
                f.col = f.col.min(len);
                if f.row < f.scroll {
                    f.scroll = f.row;
                } else if f.row >= f.scroll + body {
                    f.scroll = f.row + 1 - body;
                }
                return true;
            }
            // Tree focused: Tab enters the editor.
            KeyCode::Tab => ide.focus_editor = !ide.focus_editor,
            KeyCode::Up | KeyCode::Char('k') => ide.sel = ide.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                ide.sel = (ide.sel + 1).min(ide.entries.len().saturating_sub(1))
            }
            // ← jumps to the parent directory entry.
            KeyCode::Left => {
                if let Some(d) = ide.entries.get(ide.sel).map(|e| e.depth) {
                    if d > 0 {
                        let mut j = ide.sel;
                        while j > 0 && ide.entries[j].depth >= d {
                            j -= 1;
                        }
                        ide.sel = j;
                    }
                }
            }
            // Enter/→ toggles a directory or opens a file.
            KeyCode::Enter | KeyCode::Right if !ide.entries.is_empty() => {
                let sel = ide.sel.min(ide.entries.len() - 1);
                let (is_dir, expanded, depth, path) = {
                    let e = &ide.entries[sel];
                    (e.is_dir, e.expanded, e.depth, e.path.clone())
                };
                if is_dir && expanded {
                    ide.entries[sel].expanded = false;
                    let mut j = sel + 1;
                    while j < ide.entries.len() && ide.entries[j].depth > depth {
                        j += 1;
                    }
                    ide.entries.drain(sel + 1..j);
                } else if is_dir {
                    ide.entries[sel].expanded = true;
                    let at = sel + 1;
                    for (k, c) in ide_children(&path, depth + 1).into_iter().enumerate() {
                        ide.entries.insert(at + k, c);
                    }
                } else if is_image_path(&path) {
                    let tw = (w / 3).clamp(16, 38);
                    let lines =
                        render_image_file(&path, w.saturating_sub(tw + 4), h.saturating_sub(3))
                            .unwrap_or_else(|| vec!["<cannot decode image>".into()]);
                    ide.file = Some(IdeFile {
                        path,
                        lines,
                        scroll: 0,
                        row: 0,
                        col: 0,
                        dirty: false,
                        image: true,
                    });
                    ide.focus_editor = false; // read-only; keep tree focus
                } else {
                    let lines: Vec<String> = std::fs::read_to_string(&path)
                        .unwrap_or_else(|err| format!("<cannot read: {err}>"))
                        .replace('\t', "    ")
                        .lines()
                        .map(String::from)
                        .collect();
                    ide.file = Some(IdeFile {
                        path,
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
                    });
                    ide.focus_editor = true;
                }
            }
            _ => {}
        }
        // Keep the tree selection within the visible window.
        let body = h.saturating_sub(2);
        if ide.sel < ide.tree_scroll {
            ide.tree_scroll = ide.sel;
        } else if body > 0 && ide.sel >= ide.tree_scroll + body {
            ide.tree_scroll = ide.sel + 1 - body;
        }
        true
    }

    /// Full-screen `/ide`: file tree on the left, file viewer on the right.
    fn render_ide(&self, ide: &Ide) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let tw = (width / 3).clamp(16, 38);
        let body = h.saturating_sub(2);
        let fname = ide
            .file
            .as_ref()
            .map(|f| {
                let p = f
                    .path
                    .strip_prefix(&self.cwd)
                    .unwrap_or(&f.path)
                    .to_string_lossy()
                    .into_owned();
                if f.dirty {
                    format!("{p} ●")
                } else {
                    p
                }
            })
            .unwrap_or_else(|| "(no file)".into());
        let hint = if ide.focus_editor {
            "edit · Ctrl+S save · Esc back to tree"
        } else {
            "Tab edit · ↑↓ nav · Enter open · Esc close"
        };
        let mut out = vec![
            pad_to(
                &Style::new()
                    .fg(ACCENT)
                    .bold()
                    .render(&format!("  IDE — {fname}    {hint}")),
                width,
            ),
            pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&"─".repeat(width)),
                width,
            ),
        ];
        let sep = Style::new().fg(Color::BrightBlack).render(" │ ");
        for i in 0..body {
            let left = if let Some(e) = ide.entries.get(ide.tree_scroll + i) {
                let icon = if e.is_dir {
                    if e.expanded {
                        "▾"
                    } else {
                        "▸"
                    }
                } else {
                    "·"
                };
                let plain = pad_to(
                    &truncate(&format!(" {}{icon} {}", "  ".repeat(e.depth), e.name), tw),
                    tw,
                );
                if ide.tree_scroll + i == ide.sel && !ide.focus_editor {
                    Style::new().fg(Color::Black).bg(ACCENT).render(&plain)
                } else if e.is_dir {
                    Style::new().fg(ACCENT).render(&plain)
                } else {
                    Style::new().fg(Color::White).render(&plain)
                }
            } else {
                " ".repeat(tw)
            };
            let right = if let Some(f) = &ide.file {
                if f.image {
                    // Pre-rendered half-block rows; show raw, no line numbers.
                    f.lines.get(f.scroll + i).cloned().unwrap_or_default()
                } else if let Some(line) = f.lines.get(f.scroll + i) {
                    let lineno = f.scroll + i;
                    let num = Style::new()
                        .fg(if ide.focus_editor && lineno == f.row {
                            Color::Yellow
                        } else {
                            Color::BrightBlack
                        })
                        .render(&format!("{:>4} ", lineno + 1));
                    // Truncate the plain line first, then syntax-highlight it.
                    let plain = truncate(line, width.saturating_sub(tw + 8).max(8));
                    format!("{num}{}", highlight_code(&plain, lang_of(&f.path)))
                } else {
                    String::new()
                }
            } else if i == 0 {
                Style::new()
                    .fg(Color::BrightBlack)
                    .render("  ← pick a file to view")
            } else {
                String::new()
            };
            out.push(format!("{left}{sep}{right}"));
        }
        out.truncate(h);
        while out.len() < h {
            out.push(String::new());
        }
        out.join("\n")
    }

    /// Spawn a diff fetch for the currently selected `/git` file.
    fn git_load_diff(&self) -> Option<Cmd<Msg>> {
        let g = self.git.as_ref()?;
        let file = g.files.get(g.sel)?.clone();
        let repo = self.cwd.clone();
        Some(cmd::cmd(move || async move {
            Msg::GitDiff(git_diff_file(repo, file).await)
        }))
    }

    /// Spawn a `git show` for the selected commit (Log view's right pane).
    fn git_load_commit(&self) -> Option<Cmd<Msg>> {
        let g = self.git.as_ref()?;
        let hash = g.log.get(g.log_sel)?.split_whitespace().next()?.to_string();
        let repo = self.cwd.clone();
        Some(cmd::cmd(move || async move {
            let out = run_git(
                repo,
                vec![
                    "show".into(),
                    "--no-color".into(),
                    "--stat".into(),
                    "-p".into(),
                    hash,
                ],
            )
            .await;
            Msg::GitDiff(out.lines().map(String::from).collect())
        }))
    }

    /// Handle a key while the `/git` panel is open.
    fn git_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let repo = self.cwd.clone();
        // Commit-message input mode.
        if self.git.as_ref().is_some_and(|g| g.commit_input.is_some()) {
            let g = self.git.as_mut().unwrap();
            let inp = g.commit_input.as_mut().unwrap();
            match key.code {
                KeyCode::Esc => g.commit_input = None,
                KeyCode::Backspace => {
                    inp.pop();
                }
                KeyCode::Char(c) => inp.push(c),
                KeyCode::Enter => {
                    let m = inp.trim().to_string();
                    g.commit_input = None;
                    if !m.is_empty() {
                        g.note = "committing…".into();
                        return Some(cmd::cmd(move || async move {
                            run_git(repo.clone(), vec!["commit".into(), "-m".into(), m]).await;
                            let (f, l) = git_status_log(repo).await;
                            Msg::GitStatus(f, l)
                        }));
                    }
                }
                _ => {}
            }
            return None;
        }
        if key.code == KeyCode::Esc {
            self.git = None;
            return None;
        }
        let mut reload = false;
        let mut reload_commit = false;
        {
            let g = self.git.as_mut()?;
            let last = g.files.len().saturating_sub(1);
            let last_commit = g.log.len().saturating_sub(1);
            let log_view = g.view == GitView::Log;
            match key.code {
                KeyCode::Tab => {
                    g.diff_scroll = 0;
                    if log_view {
                        g.view = GitView::Status;
                        reload = true;
                    } else {
                        g.view = GitView::Log;
                        reload_commit = true;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if log_view {
                        g.log_sel = g.log_sel.saturating_sub(1);
                        g.diff_scroll = 0;
                        reload_commit = true;
                    } else {
                        g.sel = g.sel.saturating_sub(1);
                        reload = true;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if log_view {
                        g.log_sel = (g.log_sel + 1).min(last_commit);
                        g.diff_scroll = 0;
                        reload_commit = true;
                    } else {
                        g.sel = (g.sel + 1).min(last);
                        reload = true;
                    }
                }
                KeyCode::PageUp => g.diff_scroll = g.diff_scroll.saturating_sub(15),
                KeyCode::PageDown => g.diff_scroll += 15,
                // Space / s toggles staging of the selected file.
                KeyCode::Char(' ') | KeyCode::Char('s') => {
                    if let Some(f) = g.files.get(g.sel) {
                        let path = f.path.clone();
                        let unstage = f.staged() && f.y == ' ';
                        g.note = "…".into();
                        return Some(cmd::cmd(move || async move {
                            let args = if unstage {
                                vec![
                                    "reset".into(),
                                    "-q".into(),
                                    "HEAD".into(),
                                    "--".into(),
                                    path,
                                ]
                            } else {
                                vec!["add".into(), "--".into(), path]
                            };
                            run_git(repo.clone(), args).await;
                            let (f, l) = git_status_log(repo).await;
                            Msg::GitStatus(f, l)
                        }));
                    }
                }
                KeyCode::Char('u') => {
                    if let Some(f) = g.files.get(g.sel) {
                        let path = f.path.clone();
                        return Some(cmd::cmd(move || async move {
                            run_git(
                                repo.clone(),
                                vec![
                                    "reset".into(),
                                    "-q".into(),
                                    "HEAD".into(),
                                    "--".into(),
                                    path,
                                ],
                            )
                            .await;
                            let (f, l) = git_status_log(repo).await;
                            Msg::GitStatus(f, l)
                        }));
                    }
                }
                KeyCode::Char('a') => {
                    g.note = "staging all…".into();
                    return Some(cmd::cmd(move || async move {
                        run_git(repo.clone(), vec!["add".into(), "-A".into()]).await;
                        let (f, l) = git_status_log(repo).await;
                        Msg::GitStatus(f, l)
                    }));
                }
                KeyCode::Char('c') => g.commit_input = Some(String::new()),
                KeyCode::Char('r') => {
                    return Some(cmd::cmd(move || async move {
                        let (f, l) = git_status_log(repo).await;
                        Msg::GitStatus(f, l)
                    }));
                }
                _ => {}
            }
        }
        if reload {
            return self.git_load_diff();
        }
        if reload_commit {
            return self.git_load_commit();
        }
        None
    }

    /// Full-screen `/git` panel (gitui-style): status + diff / log + commit.
    fn render_git(&self, g: &Git) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let branch = self.branch.as_deref().unwrap_or("(detached)");
        let tab = |label: &str, active: bool| {
            if active {
                Style::new()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .bold()
                    .render(&format!(" {label} "))
            } else {
                Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!(" {label} "))
            }
        };
        let logtab = if g.log.is_empty() {
            "Log".to_string()
        } else {
            format!("Log ({})", g.log.len())
        };
        let header = format!(
            "  git · {branch}   {} {}  {}   {}",
            tab("Status", g.view == GitView::Status),
            tab(&logtab, g.view == GitView::Log),
            Style::new()
                .fg(ACCENT)
                .render("⇄ Tab to switch · commits in Log"),
            Style::new().fg(Color::BrightBlack).render(&g.note)
        );
        let mut out = vec![
            pad_to(&header, width),
            pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&"─".repeat(width)),
                width,
            ),
        ];
        let body = h.saturating_sub(3);

        if g.view == GitView::Log {
            if g.log.is_empty() {
                let msg = if g.note.is_empty() {
                    "  no commits in this repository yet"
                } else {
                    "  loading commits…"
                };
                out.push(pad_to(
                    &Style::new().fg(Color::BrightBlack).render(msg),
                    width,
                ));
                out.truncate(h);
                while out.len() < h {
                    out.push(String::new());
                }
                return out.join("\n");
            }
            // Two columns: the commit list (selectable) + the selected commit's
            // details (`git show`) on the right.
            let tw = (width / 3).clamp(20, 46);
            let sep = Style::new().fg(Color::BrightBlack).render(" │ ");
            // keep the selected commit visible
            let start = g.log_sel.saturating_sub(body.saturating_sub(1));
            for i in 0..body {
                let ci = start + i;
                let left = if let Some(line) = g.log.get(ci) {
                    let (hash, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                    let raw = pad_to(&truncate(&format!(" {hash}  {rest}"), tw), tw);
                    if ci == g.log_sel {
                        Style::new().fg(Color::Black).bg(Color::Yellow).render(&raw)
                    } else {
                        format!(
                            "{}{}",
                            Style::new()
                                .fg(Color::Yellow)
                                .render(&pad_to(&format!(" {hash} "), hash.len() + 2)),
                            truncate(rest, tw.saturating_sub(hash.len() + 3))
                        )
                    }
                } else {
                    " ".repeat(tw)
                };
                let right = if let Some(line) = g.diff.get(g.diff_scroll + i) {
                    let st = if line.starts_with("@@") {
                        Style::new().fg(Color::Cyan)
                    } else if line.starts_with("commit ") {
                        Style::new().fg(Color::Yellow).bold()
                    } else if line.starts_with('+') {
                        Style::new().fg(Color::Green)
                    } else if line.starts_with('-') {
                        Style::new().fg(Color::Red)
                    } else if line.starts_with("diff ") || line.starts_with("index ") {
                        Style::new().fg(Color::BrightBlack)
                    } else {
                        Style::new()
                    };
                    st.render(&truncate(line, width.saturating_sub(tw + 4)))
                } else {
                    String::new()
                };
                out.push(format!("{left}{sep}{right}"));
            }
        } else {
            let tw = (width / 3).clamp(20, 46);
            let sep = Style::new().fg(Color::BrightBlack).render(" │ ");
            for i in 0..body {
                // left: file list
                let left = if let Some(f) = g.files.get(i) {
                    let mark = format!("{}{}", f.x, f.y);
                    let raw = pad_to(&truncate(&format!(" {mark}  {}", f.path), tw), tw);
                    let color = if f.untracked() {
                        Color::Red
                    } else if f.staged() {
                        Color::Green
                    } else {
                        Color::Yellow
                    };
                    if i == g.sel {
                        Style::new().fg(Color::Black).bg(color).render(&raw)
                    } else {
                        Style::new().fg(color).render(&raw)
                    }
                } else if i == 0 && g.files.is_empty() {
                    pad_to(
                        &Style::new()
                            .fg(Color::BrightBlack)
                            .render("  working tree clean"),
                        tw,
                    )
                } else {
                    " ".repeat(tw)
                };
                // right: diff
                let right = if let Some(line) = g.diff.get(g.diff_scroll + i) {
                    let st = if line.starts_with("@@") {
                        Style::new().fg(Color::Cyan)
                    } else if line.starts_with('+') {
                        Style::new().fg(Color::Green)
                    } else if line.starts_with('-') {
                        Style::new().fg(Color::Red)
                    } else if line.starts_with("diff ")
                        || line.starts_with("index ")
                        || line.starts_with("--- ")
                        || line.starts_with("+++ ")
                    {
                        Style::new().fg(Color::BrightBlack)
                    } else {
                        Style::new()
                    };
                    st.render(&truncate(line, width.saturating_sub(tw + 4)))
                } else {
                    String::new()
                };
                out.push(format!("{left}{sep}{right}"));
            }
        }

        // Bottom row: commit input, or the key hints.
        let bottom = if let Some(msg) = &g.commit_input {
            Style::new().fg(Color::Yellow).bold().render(&format!(
                "  commit message: {msg}_   (Enter commit · Esc cancel)"
            ))
        } else {
            Style::new().fg(Color::BrightBlack).render(
                "  ↑↓ select · Space/s stage · u unstage · a stage-all · c commit · Tab log · r refresh · Esc",
            )
        };
        while out.len() + 1 < h {
            out.push(String::new());
        }
        out.push(pad_to(&bottom, width));
        out.truncate(h);
        out.join("\n")
    }

    /// Full-screen `/help` panel: a detailed usage guide.
    fn render_help(&self) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let head = |s: &str| Style::new().fg(ACCENT).bold().render(s);
        let row = |k: &str, d: &str| {
            format!(
                "    {}  {}",
                Style::new()
                    .fg(Color::White)
                    .bold()
                    .render(&format!("{k:<16}")),
                Style::new().fg(Color::BrightBlack).render(d)
            )
        };
        let mut lines: Vec<String> = vec![
            head("  A3S CODE — help   (Esc to close)"),
            String::new(),
            head("  Slash commands"),
            row("/model", "pick the model"),
            row("/config", "open config.acl in your editor"),
            row("/ide", "file tree + code viewer"),
            row("/top", "live process monitor (Enter to force-kill)"),
            row(
                "/relay",
                "continue a session from a3s-code / Claude / Codex",
            ),
            row("/btw <q>", "ask a background side-question (yellow panel)"),
            row("/help", "this panel"),
            row("/clear", "clear the conversation"),
            row("/auto", "auto-approve tools for this session"),
            row("/exit", "quit"),
            String::new(),
            head("  Input modes"),
            row("! <cmd>", "run a shell command (pink) · Esc leaves"),
            row("/btw <q>", "side-channel question, kept out of the chat"),
            String::new(),
            head("  Keys"),
            row("Enter", "send · while busy, the message is queued"),
            row("Shift+Tab", "cycle run mode: default → plan → auto"),
            row("↑ / ↓", "recall input history"),
            row("PgUp / PgDn", "scroll the transcript"),
            row("Shift+End", "jump to the latest output"),
            row("Esc", "interrupt the running turn"),
            row("Ctrl+C ×2", "quit"),
            String::new(),
            head("  Run modes"),
            row("default", "asks before file-modifying tools"),
            row("plan", "pinned TODO plan, tracks each step ▶/✔/✗"),
            row("auto", "auto-approves tools"),
            String::new(),
            Style::new()
                .fg(Color::BrightBlack)
                .render("  Resume a past session:  a3s code resume <id>  (printed on exit)"),
        ];
        for l in &mut lines {
            *l = pad_to(l, width);
        }
        lines.truncate(h);
        while lines.len() < h {
            lines.push(String::new());
        }
        lines.join("\n")
    }

    /// a3s-lane task detail lines for the very bottom: the running task plus
    /// each queued message. Empty when there's nothing in flight.
    fn task_lines(&self) -> Vec<String> {
        let running = self
            .running_task
            .as_ref()
            .filter(|_| self.state != State::Idle);
        // Only show the panel when work is actually queued — a lone running
        // task would otherwise resize the viewport every turn (transcript jump).
        if self.queue.is_empty() {
            return Vec::new();
        }
        let width = self.width as usize;
        let cap = width.saturating_sub(8);
        let mut lines = vec![pad_to(
            &Style::new()
                .fg(Color::BrightBlack)
                .render(&format!("  ─ tasks · ✓ {} done ────────", self.completed)),
            width,
        )];
        if let Some(t) = running {
            lines.push(pad_to(
                &Style::new()
                    .fg(Color::Yellow)
                    .render(&format!("  ⏳ {}", truncate(t, cap))),
                width,
            ));
        }
        let mut q: Vec<&Queued> = self.queue.iter().collect();
        q.sort_by_key(|x| (x.prio, x.seq));
        for item in q.iter().take(6) {
            lines.push(pad_to(
                &Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!("  ▱ {}", truncate(&item.text, cap))),
                width,
            ));
        }
        lines
    }

    /// Resize the viewport so the pinned plan panel and the bottom task panel
    /// both fit without covering the transcript.
    fn relayout(&mut self) {
        let n = (self.task_lines().len() + self.plan_lines().len() + self.subagent_lines().len())
            as u16;
        self.viewport
            .resize(self.width, self.height.saturating_sub(7 + n));
    }

    /// Replace the pinned plan from a planning-mode task list.
    fn set_plan(&mut self, tasks: &[a3s_code_core::planning::Task]) {
        self.plan = tasks
            .iter()
            .map(|t| {
                let (g, c) = task_status_style(t.status);
                (t.id.clone(), t.content.clone(), g, c)
            })
            .collect();
        self.relayout();
    }

    /// Update one plan task's status by id (from StepStart/StepEnd events).
    fn set_task_status(&mut self, id: &str, glyph: char, color: Color) {
        if let Some(t) = self.plan.iter_mut().find(|t| t.0 == id) {
            t.2 = glyph;
            t.3 = color;
        }
    }

    /// The pinned plan/TODO panel lines (header + each task), empty if no plan.
    fn plan_lines(&self) -> Vec<String> {
        if self.plan.is_empty() {
            return Vec::new();
        }
        let width = self.width as usize;
        let cap = width.saturating_sub(8);
        let done = self.plan.iter().filter(|(_, _, g, _)| *g == '✔').count();
        let mut lines = vec![pad_to(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render(&format!("  ▪ Plan · {done}/{}", self.plan.len())),
            width,
        )];
        for (_, text, glyph, color) in self.plan.iter().take(8) {
            lines.push(pad_to(
                &Style::new()
                    .fg(*color)
                    .render(&format!("  {glyph} {}", truncate(text, cap))),
                width,
            ));
        }
        lines
    }

    /// Bottom tracker for running parallel subagents (Claude-style): one row per
    /// task with the agent type, description, elapsed time, and tokens.
    fn subagent_lines(&self) -> Vec<String> {
        if self.subagents.is_empty() {
            return Vec::new();
        }
        let width = self.width as usize;
        let mut out = vec![pad_to(
            &Style::new().fg(Color::White).bold().render("  ⏺ main"),
            width,
        )];
        for s in &self.subagents {
            let secs = s.started.elapsed().as_secs();
            let el = if secs >= 60 {
                format!("{}m {}s", secs / 60, secs % 60)
            } else {
                format!("{secs}s")
            };
            let right = if s.tokens > 0 {
                format!("{el} · ↓ {} tokens", fmt_tokens(s.tokens))
            } else {
                el
            };
            let glyph = if s.done { '●' } else { '◯' };
            let rlen = a3s_tui::style::visible_len(&right);
            let maxleft = width.saturating_sub(rlen + 3).max(8);
            let left = truncate(
                &format!("  {glyph} {}  {}", s.agent, s.description),
                maxleft,
            );
            let pad = width.saturating_sub(a3s_tui::style::visible_len(&left) + rlen + 1);
            out.push(format!(
                "{}{}{}",
                Style::new().fg(Color::Magenta).render(&left),
                " ".repeat(pad),
                Style::new().fg(Color::BrightBlack).render(&right),
            ));
        }
        out
    }

    /// `/btw` side-chat panel above the input: the question and its answer.
    fn overlay_btw(&self, composed: String) -> String {
        let Some((q, a)) = &self.btw else {
            return composed;
        };
        let width = self.width as usize;
        let cap = width.saturating_sub(4).max(8);
        let wrap = |s: &str| -> Vec<String> {
            s.lines()
                .flat_map(|l| {
                    let cs: Vec<char> = l.chars().collect();
                    if cs.is_empty() {
                        vec![String::new()]
                    } else {
                        cs.chunks(cap).map(|c| c.iter().collect()).collect()
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut lines = vec![pad_to(
            &Style::new()
                .fg(Color::Yellow)
                .bold()
                .render("  ↘ by the way · Esc to close"),
            width,
        )];
        for l in wrap(&format!("Q: {q}")) {
            lines.push(pad_to(
                &Style::new()
                    .fg(Color::Yellow)
                    .bold()
                    .render(&format!("  {l}")),
                width,
            ));
        }
        let ans = a.as_deref().unwrap_or("thinking…");
        for l in wrap(ans).into_iter().take(12) {
            lines.push(pad_to(
                &Style::new().fg(Color::Yellow).render(&format!("  {l}")),
                width,
            ));
        }
        self.overlay_list(composed, &lines)
    }

    fn on_submit(&mut self, text: String) -> Option<Cmd<Msg>> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
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
                self.textarea.clear();
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
                self.push_line(
                    &Style::new()
                        .fg(Color::BrightBlack)
                        .render("  ✦ compacting context…"),
                );
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
            self.messages.push(gutter(Color::Green, &rendered));
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
        let mut blocks: Vec<String> = self.messages.clone();
        if !self.thinking.trim().is_empty() {
            let body = indent(&format!("💭 {}", self.thinking.trim()), PAD);
            blocks.push(Style::new().fg(Color::BrightBlack).italic().render(&body));
        }
        let rendered = self.streaming.view();
        if !rendered.is_empty() {
            blocks.push(gutter(Color::Green, &rendered));
        }
        // Currently-executing tool: "● action…" with a blinking dot.
        if let Some(name) = &self.running_tool {
            let args: Option<serde_json::Value> = serde_json::from_str(&self.tool_args).ok();
            let action = tool_label(name, args.as_ref());
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
            blocks.push(
                Style::new()
                    .fg(Color::BrightBlack)
                    .render(&indent(&tail, PAD + 2)),
            );
        }
        // Same "\n…\n" framing as rebuild_viewport so the transcript doesn't
        // jump a line when streaming starts/ends.
        self.viewport
            .set_content(&format!("\n{}\n", blocks.join("\n\n")));
    }

    /// First-run welcome: ASCII-art logo, version, model, and tips.
    fn banner(&self) -> String {
        // A Song-dynasty soldier in a wide-brimmed helmet, holding a sword
        // (blade + `-+-` crossguard) in his right hand and a heater shield
        // (`|#|` tapering to `\#/`) in his left. Animated by `self.anim`: he
        // blinks, the crossguard glints, and he shifts his feet.
        let f = self.anim;
        let eyes = if f % 14 == 7 { "- -" } else { "o o" };
        let g = if f % 6 == 3 { "*" } else { "+" }; // crossguard glint
        let feet = if f.is_multiple_of(2) {
            r"/   \"
        } else {
            r"\   /"
        }; // shuffle
        let mascot = [
            r"     .-^-.      ".to_string(),
            r"    /_____\     ".to_string(),
            format!("    ( {eyes} )     "),
            r"  |  /|_|\  _   ".to_string(),
            format!(" -{g}- |   | |#|  "),
            r"  |  |___| \#/  ".to_string(),
            format!("     {feet}      "),
        ];
        let art = [
            r" █████╗ ██████╗ ███████╗     ██████╗ ██████╗ ██████╗ ███████╗",
            r"██╔══██╗╚════██╗██╔════╝    ██╔════╝██╔═══██╗██╔══██╗██╔════╝",
            r"███████║ █████╔╝███████╗    ██║     ██║   ██║██║  ██║█████╗",
            r"██╔══██║ ╚═══██╗╚════██║    ██║     ██║   ██║██║  ██║██╔══╝",
            r"██║  ██║██████╔╝███████║    ╚██████╗╚██████╔╝██████╔╝███████╗",
            r"╚═╝  ╚═╝╚═════╝ ╚══════╝     ╚═════╝ ╚═════╝ ╚═════╝ ╚══════╝",
        ];
        let margin = " ".repeat(PAD);
        let steel = Color::Rgb(150, 162, 188);
        // The 7-line mascot leads with its helmet; the 6-line wordmark aligns
        // from row 2 down (art row j sits on mascot row j+1).
        let logo = mascot
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let a = i
                    .checked_sub(1)
                    .and_then(|j| art.get(j))
                    .copied()
                    .unwrap_or("");
                format!(
                    "{margin}{}  {}",
                    Style::new().fg(steel).bold().render(m),
                    Style::new().fg(ACCENT).bold().render(a),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let model = self.model.as_deref().unwrap_or("no model configured");
        let skills = if self.skill_count > 0 {
            format!("  ·  {} skills", self.skill_count)
        } else {
            String::new()
        };
        let meta = Style::new().fg(Color::BrightBlack).render(&format!(
            "{margin}a3s-code v{}  ·  {model}{skills}  ·  {}",
            env!("CARGO_PKG_VERSION"),
            self.cwd
        ));
        let tips = Style::new()
            .fg(Color::BrightBlack)
            .italic()
            .render(&format!(
            "{margin}Type a message · / for commands · Shift+Tab cycles mode · Ctrl+C twice to exit"
        ));
        let update = match &self.update_available {
            Some(v) => format!(
                "\n{margin}{}",
                Style::new().fg(ACCENT).bold().render(&format!(
                    "⬆ a3s {v} is available (you have {}) — type /update to upgrade",
                    env!("CARGO_PKG_VERSION")
                ))
            ),
            None => String::new(),
        };
        format!("\n{logo}\n\n{meta}\n{tips}{update}\n")
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
            return render_diff(path, before, after, width);
        }
    }
    let ok = exit_code == 0;
    let margin = " ".repeat(PAD);
    // Header: "⏺ Bash(npm test)" / "⏺ Read(src/main.rs)" — Claude-Code style,
    // the dot colored by outcome.
    let dot = Style::new()
        .fg(if ok { Color::Green } else { Color::Red })
        .bold()
        .render("⏺");
    let header = format!(
        "{margin}{dot} {}",
        Style::new().bold().render(&tool_label(name, args))
    );

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

    // Otherwise the first few output lines under a "⎿" connector, with a
    // "… +N lines" overflow marker (Claude-Code style).
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return header;
    }
    // Head + tail window with a "… +N lines" marker in the middle so a long
    // command (a big build, etc.) stays a fixed height instead of flooding.
    const HEAD: usize = 3;
    const TAIL: usize = 2;
    let body_color = if ok { Color::BrightBlack } else { Color::Red };
    let conn = Style::new().fg(Color::BrightBlack).render("⎿");
    let textw = width.saturating_sub(PAD + 7).max(20);
    let line_at = |i: usize, line: &str| -> String {
        let shown = truncate(line, textw);
        if i == 0 {
            format!(
                "\n{margin}  {conn}  {}",
                Style::new().fg(body_color).render(&shown)
            )
        } else {
            format!(
                "\n{margin}     {}",
                Style::new().fg(body_color).render(&shown)
            )
        }
    };
    let n = lines.len();
    let mut out = header;
    if n <= HEAD + TAIL + 1 {
        for (i, line) in lines.iter().enumerate() {
            out.push_str(&line_at(i, line));
        }
    } else {
        for (i, line) in lines.iter().take(HEAD).enumerate() {
            out.push_str(&line_at(i, line));
        }
        let hidden = n - HEAD - TAIL;
        out.push_str(&format!(
            "\n{margin}     {}",
            Style::new()
                .fg(Color::BrightBlack)
                .render(&format!("… +{hidden} lines"))
        ));
        for line in lines.iter().skip(n - TAIL) {
            out.push_str(&line_at(1, line));
        }
    }
    out
}

/// Claude-Code-style tool label: `Tool(arg)`, e.g. "Bash(npm test)",
/// "Read(src/main.rs)", "Update(lib.rs)".
fn tool_label(name: &str, args: Option<&serde_json::Value>) -> String {
    let target = args.and_then(arg_summary).unwrap_or_default();
    let display = match name {
        "bash" | "shell" | "run" | "exec" => "Bash",
        "read" | "cat" => "Read",
        "write" | "create" => "Write",
        "edit" | "patch" | "apply_patch" => "Update",
        "grep" | "search" => "Grep",
        "ls" | "glob" | "find" => "Glob",
        "web_search" => "WebSearch",
        "web_fetch" => "WebFetch",
        "task" | "parallel_task" => "Task",
        other => other,
    };
    if target.is_empty() {
        display.to_string()
    } else {
        format!("{display}({target})")
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
    // parallel_task / task: surface the sub-task descriptions so the user can
    // see what's actually being dispatched (not just "Task").
    if let Some(tasks) = args.get("tasks").and_then(|v| v.as_array()) {
        let descs: Vec<String> = tasks
            .iter()
            .filter_map(|t| {
                t.get("description")
                    .or_else(|| t.get("prompt"))
                    .or_else(|| t.get("task"))
                    .and_then(|v| v.as_str())
            })
            .map(|s| truncate(&s.replace('\n', " "), 40))
            .collect();
        if !descs.is_empty() {
            let head = descs.iter().take(2).cloned().collect::<Vec<_>>().join("; ");
            let more = descs.len().saturating_sub(2);
            let tail = if more > 0 {
                format!(" +{more} more")
            } else {
                String::new()
            };
            return Some(format!("{} ⇉ {head}{tail}", descs.len()));
        }
    }
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "description",
        "prompt",
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
/// Split a plain string into visible-width-bounded segments (char-based wrap).
fn wrap_plain(s: &str, w: usize) -> Vec<String> {
    if w == 0 || s.is_empty() {
        return vec![s.to_string()];
    }
    s.chars()
        .collect::<Vec<_>>()
        .chunks(w)
        .map(|c| c.iter().collect())
        .collect()
}

/// IDE-style unified diff: `└ path (+a -d)` header, then hunks with context
/// lines (dim, no marker), `-`/`+` changes, `⋮` between hunks, and long lines
/// wrapped with the code indented under a blank gutter.
fn render_diff(path: &str, before: &str, after: &str, width: usize) -> String {
    use similar::{ChangeTag, TextDiff};
    const MAX_LINES: usize = 200;

    let lang = lang_of(std::path::Path::new(path));
    let diff = TextDiff::from_lines(before, after);
    let (mut adds, mut dels) = (0usize, 0usize);
    for c in diff.iter_all_changes() {
        match c.tag() {
            ChangeTag::Insert => adds += 1,
            ChangeTag::Delete => dels += 1,
            ChangeTag::Equal => {}
        }
    }
    let nw = before
        .lines()
        .count()
        .max(after.lines().count())
        .max(1)
        .to_string()
        .len()
        .max(3);
    let code_col = 4 + nw + 3; // "    " + lineno + " " + marker + " "
    let code_w = width.saturating_sub(code_col).max(16);
    let cont_pad = " ".repeat(code_col);

    let mut lines: Vec<String> = Vec::new();
    let mut truncated = false;
    for (gi, group) in diff.grouped_ops(3).iter().enumerate() {
        if gi > 0 {
            lines.push(
                Style::new()
                    .fg(Color::BrightBlack)
                    .render(&format!("    {} ⋮", " ".repeat(nw))),
            );
        }
        for op in group {
            for change in diff.iter_changes(op) {
                if lines.len() >= MAX_LINES {
                    truncated = true;
                    break;
                }
                let raw = change.value();
                let raw = raw.strip_suffix('\n').unwrap_or(raw);
                let (no, marker, mcol, dim) = match change.tag() {
                    ChangeTag::Delete => (
                        change.old_index().map(|i| i + 1).unwrap_or(0),
                        '-',
                        Color::Red,
                        false,
                    ),
                    ChangeTag::Insert => (
                        change.new_index().map(|i| i + 1).unwrap_or(0),
                        '+',
                        Color::Green,
                        false,
                    ),
                    ChangeTag::Equal => (
                        change.old_index().map(|i| i + 1).unwrap_or(0),
                        ' ',
                        Color::BrightBlack,
                        true,
                    ),
                };
                for (si, seg) in wrap_plain(raw, code_w).iter().enumerate() {
                    let code = if dim {
                        Style::new().fg(Color::BrightBlack).render(seg)
                    } else {
                        highlight_code(seg, lang)
                    };
                    if si == 0 {
                        let gutter = Style::new()
                            .fg(mcol)
                            .render(&format!("    {no:>width$} {marker} ", width = nw));
                        lines.push(format!("{gutter}{code}"));
                    } else {
                        lines.push(format!("{cont_pad}{code}"));
                    }
                }
            }
            if truncated {
                break;
            }
        }
        if truncated {
            break;
        }
    }
    if truncated {
        lines.push(
            Style::new()
                .fg(Color::BrightBlack)
                .render("    … (diff truncated)"),
        );
    }
    let mut out = format!(
        "  {} {} {}",
        Style::new().fg(Color::BrightBlack).render("└"),
        Style::new().fg(Color::Cyan).render(path),
        Style::new()
            .fg(Color::BrightBlack)
            .render(&format!("(+{adds} -{dels})")),
    );
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

/// A starter `config.acl` (HCL-like ACL) with placeholders, generated on first
/// launch so a new user has something to edit instead of an error.
fn config_template() -> &'static str {
    r#"# A3S coding-agent config (HCL-like ACL).
# Fill in your provider apiKey/baseUrl + a model, set default_model, then save
# with Ctrl+S. Docs: https://a3s-lab.github.io/a3s/

default_model = "openai/my-model"

providers "openai" {
  apiKey  = "sk-REPLACE-ME"
  baseUrl = "https://api.openai.com/v1/"   # or any OpenAI-compatible endpoint

  models "my-model" {
    name        = "My Model"
    toolCall    = true
    temperature = true
    modalities  = { input = ["text"], output = ["text"] }
    limit       = { context = 128000, output = 4096 }
  }
}
"#
}

/// `~/.a3s/config.acl` — the default user-global config location.
fn default_config_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".a3s/config.acl"))
}

/// Write the starter config to `path` (creating parent dirs). Never overwrites.
fn write_template_config(path: &std::path::Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, config_template())
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

/// Discover Claude Code skill directories — personal (`~/.claude/skills`),
/// project (`<ws>/.claude/skills`), and plugin-bundled (`~/.claude/plugins/**/
/// skills`) — so a3s can load Claude `SKILL.md` skills directly. a3s's skill
/// loader already understands the `<name>/SKILL.md` layout and YAML frontmatter.
/// Parse a SKILL.md's YAML frontmatter for `name` + `description`.
fn parse_skill_meta(path: &std::path::Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    let rest = content.trim_start().strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let (mut name, mut desc) = (None, None);
    for line in rest[..end].lines() {
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().trim_matches(['"', '\'']).to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            desc = Some(v.trim().trim_matches(['"', '\'']).to_string());
        }
    }
    let name = name?;
    if name.is_empty() {
        return None;
    }
    Some((name, desc.unwrap_or_default()))
}

/// `~/.a3s/disabled_skills` — names the user has turned off via `/plugins`.
fn disabled_skills_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".a3s/disabled_skills"))
}

fn load_disabled_skills() -> std::collections::HashSet<String> {
    disabled_skills_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn save_disabled_skills(set: &std::collections::HashSet<String>) {
    if let Some(p) = disabled_skills_path() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut names: Vec<&String> = set.iter().collect();
        names.sort();
        let body = names
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(p, body);
    }
}

/// Load skill (name, description) pairs from the skill dirs, for the slash menu.
fn load_skills(dirs: &[std::path::PathBuf]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let md = if p.is_dir() {
                p.join("SKILL.md")
            } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
                p.clone()
            } else {
                continue;
            };
            if md.is_file() {
                if let Some(meta) = parse_skill_meta(&md) {
                    out.push(meta);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Count discoverable Claude skills (`<name>/SKILL.md` dirs + flat `*.md`)
/// across the skill dirs — shown on the start screen so compatibility is visible.
fn count_skill_files(dirs: &[std::path::PathBuf]) -> usize {
    let mut n = 0;
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                let is_skill_dir = p.is_dir() && p.join("SKILL.md").is_file();
                let is_flat_md = p.extension().and_then(|x| x.to_str()) == Some("md");
                if is_skill_dir || is_flat_md {
                    n += 1;
                }
            }
        }
    }
    n
}

fn claude_skill_dirs(workspace: &str) -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    let project = std::path::Path::new(workspace).join(".claude/skills");
    if project.is_dir() {
        dirs.push(project);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        let personal = home.join(".claude/skills");
        if personal.is_dir() {
            dirs.push(personal);
        }
        // Depth 6 covers nested plugin layouts: plugins/cache/<plugin>/<plugin>/
        // <version>/skills and marketplaces/<mkt>/external_plugins/<plugin>/skills.
        collect_skills_dirs(&home.join(".claude/plugins"), 0, 6, &mut dirs);
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Recursively collect directories literally named `skills` (Claude plugins
/// bundle their skills there), bounded in depth and skipping dotfiles.
fn collect_skills_dirs(
    dir: &std::path::Path,
    depth: usize,
    max: usize,
    out: &mut Vec<std::path::PathBuf>,
) {
    if depth > max {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "skills" {
            out.push(p);
        } else if !name.starts_with('.') && name != "node_modules" {
            collect_skills_dirs(&p, depth + 1, max, out);
        }
    }
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
                .with_auto_compact_threshold(0.85),
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
                    .with_auto_compact_threshold(0.85),
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
        // No mouse capture: lets the terminal handle text selection + copy.
        // Scroll the transcript with PgUp/PgDn / Shift+End instead.
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
        assert!(plain.contains('└'), "tree-connector path header");
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
        // Action-verb summary ("Ran …") + the output; no diff marker.
        assert!(out.contains("Bash") && out.contains("hello"));
        assert!(!out.contains('✎'), "no diff marker for non-edit tools");
    }

    #[test]
    fn tool_end_shows_primary_arg_summary() {
        let args = serde_json::json!({ "command": "npm test", "timeout": 60 });
        let out = render_tool_end("bash", 0, "ok\n", None, Some(&args), 80);
        assert!(out.contains("Bash"), "tool label for bash");
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
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4,
            6,
            image::Rgba([10, 20, 30, 255]),
        ));
        let lines = render_image_blocks(&img, 80, 40);
        assert_eq!(lines.len(), 3, "6px / 2 = 3 rows");
        assert!(lines[0].contains('▀'), "uses upper half-block");
        assert!(lines[0].contains("\x1b["), "carries ANSI color");
    }

    #[test]
    fn half_block_render_fits_within_bounds() {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::new(400, 400));
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
