//! Built-in IDE state, selection helpers, viewport actions, and prompt queue types.

use super::*;

/// One visible row of the `/ide` file tree (a flattened, expandable tree).
pub(super) struct IdeEntry {
    pub(super) path: std::path::PathBuf,
    pub(super) name: String,
    pub(super) depth: usize,
    pub(super) is_dir: bool,
    pub(super) expanded: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum IdePrompt {
    Search { forward: bool, text: String },
    Command(String),
}

/// Editor input mode — vim-aligned: Normal navigates/operates, Insert types.
/// Freshly opened buffers start in Normal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum EditMode {
    Normal,
    Insert,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct PendingOp {
    pub(super) op: char,
    pub(super) count: usize,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum RepeatEdit {
    DeleteChar(usize),
    DeleteLine(usize),
    DeleteWord(usize),
    DeleteToEol,
    ChangeLine(usize),
    Replace(char),
    JoinLine(usize),
    ToggleCase(usize),
}

/// An open, editable file in the `/ide` panel.
pub(super) struct IdeFile {
    pub(super) path: std::path::PathBuf,
    pub(super) lines: Vec<String>, // text rows, or pre-rendered half-block rows if `image`
    pub(super) scroll: usize,      // top visible row (vertical scroll)
    pub(super) hscroll: usize,     // leftmost visible column (horizontal scroll; display columns)
    pub(super) row: usize,         // cursor line
    pub(super) col: usize,         // cursor column (char index)
    pub(super) dirty: bool,
    pub(super) image: bool,    // read-only image preview
    pub(super) readonly: bool, // view-only (e.g. a dynamic-workflow artifact) — edits blocked
    pub(super) mode: EditMode, // vim Normal/Insert (see `ide_key`)
    /// A pending operator/prefix awaiting its second keystroke (`d`, `c`, `g`, `y`).
    pub(super) pending: Option<PendingOp>,
    /// Normal-mode numeric prefix (`3j`, `5dd`, `2w`).
    pub(super) count: Option<usize>,
    /// Undo snapshots (lines + cursor) for `u`; bounded — configs are small.
    pub(super) undo: Vec<(Vec<String>, usize, usize)>,
    /// Redo snapshots for Ctrl+R.
    pub(super) redo: Vec<(Vec<String>, usize, usize)>,
    /// Last repeatable Normal-mode change for `.`.
    pub(super) last_change: Option<RepeatEdit>,
    /// Last search query and direction for `n` / `N`.
    pub(super) search: Option<(String, bool)>,
    /// Visual Line anchor row (`V`).
    pub(super) visual_line_anchor: Option<usize>,
    pub(super) clip: String,        // yank/delete register for p / P
    pub(super) clip_linewise: bool, // register holds whole lines (dd/yy) vs an inline span
}

impl IdeFile {
    /// A freshly opened buffer: cursor at the top, Normal mode, empty undo.
    pub(super) fn new(
        path: std::path::PathBuf,
        lines: Vec<String>,
        image: bool,
        readonly: bool,
    ) -> Self {
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
            count: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_change: None,
            search: None,
            visual_line_anchor: None,
            clip: String::new(),
            clip_linewise: false,
        }
    }
}

/// Overlay the shared one-column vertical scrollbar on the viewport's final
/// column. When content fits, every row keeps the full terminal width instead
/// of reserving an empty right gutter.
pub(super) fn append_scrollbar(
    view: &str,
    canvas_width: usize,
    total: usize,
    scroll_percent: u8,
) -> String {
    if canvas_width == 0 {
        return view
            .split('\n')
            .map(|_| String::new())
            .collect::<Vec<_>>()
            .join("\n");
    }

    let visible = view.split('\n').count();
    let scrollbar = Scrollbar::from_scroll_percent(total, visible, scroll_percent)
        .track_color(TN_GRAY)
        .thumb_color(ACCENT)
        .hide_when_not_overflowing(true);
    if scrollbar.has_overflow() {
        let content_width = canvas_width.saturating_sub(1);
        let bar = scrollbar.styled_view(visible);
        return view
            .split('\n')
            .zip(bar.lines())
            .map(|(row, bar)| format!("{}{bar}", fit_viewport_row(row, content_width)))
            .collect::<Vec<_>>()
            .join("\n");
    }

    view.split('\n')
        .map(|row| fit_viewport_row(row, canvas_width))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn fit_viewport_row(row: &str, width: usize) -> String {
    let fitted = a3s_tui::style::truncate_visible(row, width);
    let padding = width.saturating_sub(a3s_tui::style::visible_len(&fitted));
    if padding == 0 {
        return fitted;
    }

    let padding = a3s_tui::markdown::trailing_ansi_background(row)
        .map(|color| Style::new().bg(color).render(&" ".repeat(padding)))
        .unwrap_or_else(|| " ".repeat(padding));
    format!("{fitted}{padding}")
}

/// The OSC 52 escape that asks the terminal to set the system clipboard to
/// `text` (base64). Works over SSH on terminals that support OSC 52. Capped so a
/// long reply can't blow past a terminal's OSC 52 size limit.
pub(super) fn osc52_copy(text: &str) -> String {
    use base64::Engine;
    let capped: String = text.chars().take(64_000).collect();
    let b64 = base64::engine::general_purpose::STANDARD.encode(capped.as_bytes());
    format!("\x1b]52;c;{b64}\x07")
}

/// Marker the agent puts inline in its reply to offer the RemoteUI popup. The
/// host recognises a mouse click on any reply line containing it and opens the
/// remembered view. The styled button is still transcript text, so ANSI stripping
/// keeps this marker clickable.
pub(super) const VIEW_BUTTON_MARKER: &str = "Open view";
pub(super) const VIEW_BUTTON_CLICK_DRIFT_COLS: u16 = 2;
pub(super) const RESEARCH_VIEW_MARKER: &str = "A3S_RESEARCH_VIEW:";

pub(super) fn remote_view_button(detail: &str) -> String {
    InlineAction::new(VIEW_BUTTON_MARKER)
        .icon("↗")
        .colors(TN_FG, ACCENT)
        .detail_color(TN_GRAY)
        .detail(detail)
        .view()
}

/// Put `text` on the system clipboard: OSC 52 (portable, survives SSH on
/// supporting terminals) plus the native tool where we have one (macOS pbcopy).
pub(super) fn copy_to_clipboard(text: &str) {
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
pub(super) const SELECTION_BG: Color = SURFACE_SELECTED;

/// An in-progress mouse text-selection in the transcript viewport, in screen
/// cells (visible row, column). `anchor` = drag start, `head` = current point.
#[derive(Clone, Copy)]
pub(super) struct Selection {
    pub(super) anchor: (u16, u16),
    pub(super) head: (u16, u16),
}

impl Selection {
    pub(super) fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
    /// (top_row, top_col, bottom_row, bottom_col), as usize.
    pub(super) fn ordered(&self) -> (usize, usize, usize, usize) {
        let (a, b) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        (a.0 as usize, a.1 as usize, b.0 as usize, b.1 as usize)
    }
}

pub(super) fn viewport_mouse_cell(
    row: u16,
    column: u16,
    viewport_rows: usize,
    max_col: u16,
) -> Option<(u16, u16)> {
    if viewport_rows == 0 || row as usize >= viewport_rows {
        None
    } else {
        Some((row, column.min(max_col)))
    }
}

pub(super) fn viewport_mouse_cell_clamped(
    row: u16,
    column: u16,
    viewport_rows: usize,
    max_col: u16,
) -> Option<(u16, u16)> {
    if viewport_rows == 0 {
        None
    } else {
        Some((
            row.min(viewport_rows.saturating_sub(1) as u16),
            column.min(max_col),
        ))
    }
}

/// Substring of `s` spanning visible columns `[from, to)` (wide chars counted by
/// display width). A char straddling the start is dropped; one straddling the
/// end is kept.
pub(super) fn slice_cols(s: &str, from: usize, to: usize) -> String {
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
pub(super) fn selection_to_text(view: &str, r1: usize, c1: usize, r2: usize, c2: usize) -> String {
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

pub(super) fn viewport_row_contains_view_button(view: &str, row: u16) -> bool {
    view.split('\n')
        .nth(row as usize)
        .map(a3s_tui::style::strip_ansi)
        .is_some_and(|line| line.to_ascii_lowercase().contains("open view"))
}

pub(super) fn is_remote_view_click(view: &str, selection: Selection) -> bool {
    selection.anchor.0 == selection.head.0
        && selection.anchor.1.abs_diff(selection.head.1) <= VIEW_BUTTON_CLICK_DRIFT_COLS
        && viewport_row_contains_view_button(view, selection.anchor.0)
}

pub(super) fn is_quit_key(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&'c'))
}

pub(super) fn is_tool_output_key(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&'t'))
}

pub(super) fn quit_is_confirmed(armed: Option<Instant>, now: Instant) -> bool {
    armed.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_secs(2))
}

/// Re-render the viewport `view` with the selected span highlighted: selected
/// rows render in plain text (no syntax colour, transiently) with the selected
/// columns on `SELECTION_BG`; other rows keep their styling.
pub(super) fn highlight_selection(
    view: &str,
    r1: usize,
    c1: usize,
    r2: usize,
    c2: usize,
) -> String {
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
pub(super) struct Ide {
    pub(super) entries: Vec<IdeEntry>,
    pub(super) sel: usize,
    pub(super) tree_scroll: usize,
    pub(super) file: Option<IdeFile>,
    pub(super) focus_editor: bool,
    /// Transient save status shown in the footer (set on Ctrl+S).
    pub(super) flash: Option<String>,
    /// Left-panel title ("workspace" / "config" / "knowledge base").
    pub(super) title: String,
    /// Superfile-style hover preview of the tree-selected file, keyed by path
    /// so it reloads only when the selection actually moves.
    pub(super) preview: Option<(std::path::PathBuf, Vec<String>)>,
    /// Active command prompt inside the editor footer (`/`, `?`, `:`).
    pub(super) prompt: Option<IdePrompt>,
    /// `/kb` browser: the vault root. Enables `x` delete, hard-bounded to
    /// paths inside this root. `None` for /ide and /config.
    pub(super) kb_root: Option<std::path::PathBuf>,
    /// A path armed for deletion — the next `x` on the same selection deletes.
    pub(super) armed_delete: Option<std::path::PathBuf>,
    /// Selected action in the `/kb` delete confirmation row (`true` = delete).
    pub(super) delete_confirm_yes: bool,
}

impl Ide {
    /// A fresh panel over `entries` (no file open, tree focused).
    pub(super) fn browse(entries: Vec<IdeEntry>, title: &str) -> Self {
        Ide {
            entries,
            sel: 0,
            tree_scroll: 0,
            file: None,
            focus_editor: false,
            flash: None,
            title: title.to_string(),
            preview: None,
            prompt: None,
            kb_root: None,
            armed_delete: None,
            delete_confirm_yes: true,
        }
    }
}

/// Directory children for the tree, dirs first then files, noise skipped.
pub(super) fn ide_children(dir: &std::path::Path, depth: usize) -> Vec<IdeEntry> {
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
pub(super) fn project_instructions(workspace: &str) -> Option<String> {
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

/// Root content is full-bleed. Individual components still own the indentation
/// needed for prompts, markers, trees, and nested content.
pub(super) const PAD: usize = 0;

pub(super) fn viewport_content_width_for(width: u16) -> usize {
    width as usize
}

pub(super) fn transcript_markdown_width_for(width: u16) -> usize {
    viewport_content_width_for(width).saturating_sub(PAD + 2)
}

pub(super) fn textarea_width_for(width: u16) -> u16 {
    transcript_markdown_width_for(width).min(u16::MAX as usize) as u16
}

/// Run mode, cycled with Shift+Tab.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    /// Approve every tool call.
    Default,
    /// Read-only tools auto-approved; writes still prompt (exploration/planning).
    Plan,
    /// Auto-approve every tool call.
    Auto,
}

impl Mode {
    pub(super) fn next(self) -> Self {
        match self {
            Mode::Default => Mode::Plan,
            Mode::Plan => Mode::Auto,
            Mode::Auto => Mode::Default,
        }
    }

    pub(super) fn glyph(self) -> &'static str {
        match self {
            Mode::Default => "⏵",
            Mode::Plan => "✎",
            Mode::Auto => "⏵⏵",
        }
    }

    /// Short one-word name for the status line ("auto mode on").
    pub(super) fn name(self) -> &'static str {
        match self {
            Mode::Default => "default",
            Mode::Plan => "plan",
            Mode::Auto => "auto",
        }
    }

    pub(super) fn color(self) -> Color {
        match self {
            Mode::Default => TN_FG,
            Mode::Plan => TN_CYAN,
            Mode::Auto => TN_GREEN,
        }
    }

    /// Whether a tool call is auto-approved in this mode.
    pub(super) fn auto_approves(self, tool: &str) -> bool {
        match self {
            Mode::Auto => true,
            Mode::Plan => is_readonly_tool(tool),
            Mode::Default => false,
        }
    }
}

pub(super) fn is_readonly_tool(name: &str) -> bool {
    matches!(
        name,
        "read" | "grep" | "ls" | "glob" | "find" | "search" | "web_search" | "web_fetch"
    )
}

/// A user message queued while the agent is busy. Priority queue: lower `prio`
/// runs first, FIFO within a priority.
pub(super) struct Queued {
    pub(super) prio: u8,
    pub(super) seq: u64,
    pub(super) text: String,
    pub(super) display: String,
    pub(super) runtime_expectation: Option<RuntimeExpectation>,
    pub(super) deep_research: Option<(String, bool, DeepResearchEvidenceScope)>,
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
