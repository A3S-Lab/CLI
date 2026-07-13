//! Rendering of completed tool calls: labels, arg summaries, and file diffs.

use super::*;
use a3s_tui::components::{ChecklistItem, ChecklistStatus, DiffLineKind, DiffSpan, OutputStatus};

const DIFF_HEADER_BULLET: Color = Color::Rgb(120, 123, 125);
const DIFF_HEADER_ACTION: Color = Color::Rgb(255, 255, 255);
const DIFF_HEADER_DETAIL: Color = Color::Rgb(220, 220, 220);
const DIFF_CONTEXT_GUTTER: Color = Color::Rgb(120, 123, 125);
const DIFF_INSERT_GUTTER: Color = Color::Rgb(122, 139, 131);
const DIFF_DELETE_GUTTER: Color = Color::Rgb(150, 125, 123);
const DIFF_INSERT_MARKER: Color = Color::Rgb(0, 194, 0);
const DIFF_DELETE_MARKER: Color = Color::Rgb(180, 60, 42);
const DIFF_INSERT_BG: Color = Color::Rgb(24, 59, 42);
const DIFF_DELETE_BG: Color = Color::Rgb(80, 31, 27);
const DIFF_CODE_FG: Color = Color::Rgb(203, 214, 247);

/// Render a completed tool call. File-editing tools (`write`/`edit`) carry
/// `before`/`after`/`file_path` in their metadata — show those as a colored
/// diff; everything else shows a status line + a few lines of output.
pub(crate) fn render_tool_end(
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

    // For a successful file read on a known language, show the highlighted file
    // content under the action (keeps the nice read preview).
    if ok && matches!(name, "read" | "cat") {
        let header = render_tool_header(name, ok, args, width);
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
                let rendered = super::design_markdown::Markdown::new()
                    .with_width(width.saturating_sub(PAD + 4).max(20))
                    .render(&fenced);
                return format!("{header}\n{rendered}");
            }
        }
    }

    if matches!(name, "task" | "parallel_task") {
        let header = render_tool_header(name, ok, args, width);
        if let Some(summary) = render_task_tool_summary(name, output, meta, ok, width) {
            return format!("{header}{summary}");
        }
    }

    render_completed_tool_output_block(name, ok, output, args, width)
}

fn render_completed_tool_output_block(
    name: &str,
    ok: bool,
    output: &str,
    args: Option<&serde_json::Value>,
    width: usize,
) -> String {
    const TAIL: usize = 5;

    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let mut block = chrome
        .output(tool_verb(name))
        .indent(PAD)
        .bullet("●")
        .status(if ok {
            OutputStatus::Success
        } else {
            OutputStatus::Error
        })
        .title_color(TN_FG)
        .body_color(if ok { TN_GRAY } else { TN_RED })
        .max_body_lines(TAIL)
        .text(output);

    let arg = args
        .and_then(|args| arg_summary_for_tool(name, args))
        .unwrap_or_default();
    if !arg.is_empty() {
        block = if matches!(name, "bash" | "shell" | "run" | "exec") {
            block.styled_detail(highlight_shell(&arg))
        } else {
            block.detail(arg)
        };
    }

    block.view(width.min(u16::MAX as usize) as u16)
}

fn render_task_tool_summary(
    name: &str,
    output: &str,
    meta: Option<&serde_json::Value>,
    ok: bool,
    width: usize,
) -> Option<String> {
    let meta = meta?;
    match name {
        "task" => render_single_task_summary(output, meta, ok, width),
        "parallel_task" => render_parallel_task_summary(meta, ok, width),
        _ => None,
    }
}

fn render_single_task_summary(
    output: &str,
    meta: &serde_json::Value,
    ok: bool,
    width: usize,
) -> Option<String> {
    let agent = meta
        .get("agent")
        .and_then(|v| v.as_str())
        .unwrap_or("agent");
    let task_id = meta.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
    let success = meta.get("success").and_then(|v| v.as_bool()).unwrap_or(ok);
    let status = if success { "completed" } else { "failed" };
    let output_bytes = meta.get("output_bytes").and_then(|v| v.as_u64());
    let artifact = meta.get("artifact_uri").and_then(|v| v.as_str());
    let mut rows = vec![TaskSummaryRow::header(
        format!("Task {status} · {agent}{}", task_id_suffix(task_id)),
        success,
    )];
    if let Some(excerpt) = task_child_excerpt(output) {
        rows.extend(
            excerpt
                .lines()
                .map(|line| TaskSummaryRow::child(line, success)),
        );
    } else if output_bytes == Some(0) {
        rows.push(TaskSummaryRow::child(
            "no child text output; using plan/status for synthesis",
            success,
        ));
    } else {
        rows.push(TaskSummaryRow::child(
            "child output stored in task artifact",
            success,
        ));
    }
    if let Some(uri) = artifact {
        rows.push(TaskSummaryRow::child(
            format!("artifact: {}", truncate(uri, 96)),
            success,
        ));
    }
    Some(render_task_rows(&rows, width))
}

fn render_parallel_task_summary(
    meta: &serde_json::Value,
    ok: bool,
    width: usize,
) -> Option<String> {
    let results = meta.get("results").and_then(|v| v.as_array())?;
    if results.is_empty() {
        return None;
    }
    let done = results
        .iter()
        .filter(|r| r.get("success").and_then(|v| v.as_bool()).unwrap_or(ok))
        .count();
    let mut rows = vec![TaskSummaryRow::header(
        format!("{done}/{} agents done", results.len()),
        ok,
    )];
    for result in results.iter().take(4) {
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(ok);
        let agent = result
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("agent");
        let task_id = result.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
        let output_bytes = result.get("output_bytes").and_then(|v| v.as_u64());
        let formatted = result.get("output").and_then(|v| v.as_str()).unwrap_or("");
        let detail = if let Some(excerpt) = task_child_excerpt(formatted) {
            truncate(&excerpt.replace('\n', " "), 120)
        } else if output_bytes == Some(0) {
            "no child text output".to_string()
        } else {
            "output stored in artifact".to_string()
        };
        rows.push(TaskSummaryRow::result(
            format!("{agent}{} · {detail}", task_id_suffix(task_id)),
            success,
        ));
    }
    let more = results.len().saturating_sub(4);
    if more > 0 {
        rows.push(TaskSummaryRow::child(
            format!("+{more} more agent result(s)"),
            ok,
        ));
    }
    Some(render_task_rows(&rows, width))
}

#[derive(Debug, Clone)]
struct TaskSummaryRow {
    text: String,
    status: ChecklistStatus,
    glyph: char,
    glyph_color: Color,
    text_color: Color,
}

impl TaskSummaryRow {
    fn header(text: impl Into<String>, ok: bool) -> Self {
        Self::status(text, ok)
    }

    fn result(text: impl Into<String>, ok: bool) -> Self {
        Self::status(text, ok)
    }

    fn child(text: impl Into<String>, ok: bool) -> Self {
        Self {
            text: text.into(),
            status: ChecklistStatus::Pending,
            glyph: '·',
            glyph_color: TN_GRAY,
            text_color: if ok { TN_GRAY } else { TN_RED },
        }
    }

    fn status(text: impl Into<String>, ok: bool) -> Self {
        Self {
            text: text.into(),
            status: if ok {
                ChecklistStatus::Done
            } else {
                ChecklistStatus::Error
            },
            glyph: if ok { '✓' } else { '✗' },
            glyph_color: if ok { TN_GREEN } else { TN_RED },
            text_color: if ok { TN_GRAY } else { TN_RED },
        }
    }
}

fn render_task_rows(rows: &[TaskSummaryRow], width: usize) -> String {
    let items = rows
        .iter()
        .map(|row| {
            ChecklistItem::new(row.text.clone())
                .status(row.status)
                .glyph(row.glyph)
                .glyph_color(row.glyph_color)
                .text_color(row.text_color)
        })
        .collect();
    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let block = chrome
        .checklist(items)
        .indent(PAD + 2)
        .connector(true)
        .strikethrough_done(false)
        .view(width.min(u16::MAX as usize) as u16, rows.len());

    if block.is_empty() {
        String::new()
    } else {
        format!("\n{block}")
    }
}

fn task_child_excerpt(formatted: &str) -> Option<String> {
    let tail = formatted
        .split_once("Output:\n")
        .map(|(_, tail)| tail)
        .or_else(|| {
            formatted
                .split_once("Output excerpt:")
                .map(|(_, tail)| tail)
        })?;
    let lines = tail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn task_id_suffix(task_id: &str) -> String {
    if task_id.is_empty() {
        String::new()
    } else {
        format!(" · {}", truncate(task_id, 24))
    }
}

/// Codex-style past-tense action verb for a completed tool call.
pub(crate) fn tool_verb(name: &str) -> &str {
    match name {
        "bash" | "shell" | "run" | "exec" => "Ran",
        "read" | "cat" => "Read",
        "write" | "create" => "Wrote",
        "edit" | "patch" | "apply_patch" => "Edited",
        "grep" | "search" => "Searched",
        "ls" | "glob" | "find" => "Listed",
        "web_search" => "Searched web",
        "web_fetch" => "Fetched",
        "task" | "parallel_task" => "Explored",
        "runtime" => "Used Runtime",
        "git" => "Ran git",
        "batch" => "Ran batch",
        "program" => "Ran program",
        "skill" | "Skill" => "Used skill",
        other => other,
    }
}

pub(crate) fn tool_running_verb(name: &str) -> &str {
    match name {
        "bash" | "shell" | "run" | "exec" => "Running",
        "read" | "cat" => "Reading",
        "write" | "create" => "Writing",
        "edit" | "patch" | "apply_patch" => "Editing",
        "grep" | "search" | "web_search" => "Searching",
        "ls" | "glob" | "find" => "Listing",
        "web_fetch" => "Fetching",
        "task" | "parallel_task" => "Exploring",
        "runtime" => "Running Runtime",
        "git" => "Running git",
        "batch" => "Running batch",
        "program" => "Running program",
        "skill" | "Skill" => "Using skill",
        _ => "Using",
    }
}

/// Claude-Code-style tool label: `Tool(arg)`, e.g. "Bash(npm test)",
/// "Read(src/main.rs)", "Update(lib.rs)". Used for the live-running indicator
/// and the approval prompt.
/// Codex-style coloring for a shell command in a tool header: the program name
/// stands out (bold cyan), flags are distinct (yellow), and positional args are
/// muted (gray) so the line is scannable at a glance.
pub(crate) fn highlight_shell(cmd: &str) -> String {
    let mut out = String::new();
    for (i, tok) in cmd.split_whitespace().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let styled = if i == 0 {
            Style::new().fg(TN_CYAN).bold().render(tok)
        } else if tok.starts_with('-') {
            Style::new().fg(TN_YELLOW).render(tok)
        } else {
            Style::new().fg(TN_GRAY).render(tok)
        };
        out.push_str(&styled);
    }
    out
}

pub(crate) fn tool_label(name: &str, args: Option<&serde_json::Value>) -> String {
    let target = args
        .and_then(|args| arg_summary_for_tool(name, args))
        .unwrap_or_default();
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
        "runtime" => "Runtime",
        "git" => "Git",
        "batch" => "Batch",
        "program" => "Program",
        "skill" | "Skill" => "Skill",
        other => other,
    };
    if target.is_empty() {
        display.to_string()
    } else {
        format!("{display}({target})")
    }
}

fn render_tool_header(
    name: &str,
    ok: bool,
    args: Option<&serde_json::Value>,
    width: usize,
) -> String {
    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let mut line = chrome
        .tool_status(tool_verb(name))
        .margin(PAD)
        .marker_color(if ok { TN_GREEN } else { TN_RED })
        .label_bold(true);
    let arg = args
        .and_then(|args| arg_summary_for_tool(name, args))
        .unwrap_or_default();

    if !arg.is_empty() {
        line = if matches!(name, "bash" | "shell" | "run" | "exec") {
            line.styled_detail(highlight_shell(&arg))
        } else {
            line.detail(arg)
        };
    }
    line.view(width.min(u16::MAX as usize) as u16)
}

pub(crate) fn render_live_tool_activity(
    name: &str,
    args: Option<&serde_json::Value>,
    output: &str,
    width: usize,
    active: bool,
) -> String {
    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let mut block = chrome
        .activity(tool_running_verb(name))
        .margin(PAD)
        .width(width)
        .marker_colors(if active { ACCENT } else { TN_GRAY }, TN_GRAY)
        .max_output_lines(13);
    let arg = args
        .and_then(|args| arg_summary_for_tool(name, args))
        .unwrap_or_default();
    if !arg.is_empty() {
        block = if matches!(name, "bash" | "shell" | "run" | "exec") {
            block.styled_detail(highlight_shell(&arg))
        } else {
            block.detail(arg)
        };
    }

    let output_lines: Vec<&str> = output.lines().collect();
    if output_lines.iter().any(|line| !line.trim().is_empty()) {
        let earlier = output_lines.len().saturating_sub(12);
        let mut lines = Vec::new();
        if earlier > 0 {
            lines.push(format!("… +{earlier} earlier lines"));
        }
        lines.extend(
            output_lines
                .iter()
                .skip(earlier)
                .map(|line| (*line).to_string()),
        );
        block = block.lines(lines);
    }

    block.view()
}

/// Extract a one-line summary of a tool's primary argument.
pub(crate) fn arg_summary(args: &serde_json::Value) -> Option<String> {
    // parallel_task / task: surface the sub-task descriptions so the user can
    // see what's actually being dispatched (not just "Task").
    if let Some(tasks) = args.get("tasks").and_then(|v| v.as_array()) {
        if let Some(summary) = summarize_tasks(tasks, args.get("worker").and_then(|v| v.as_str())) {
            return Some(summary);
        }
    }
    if let Some(invocations) = args.get("invocations").and_then(|v| v.as_array()) {
        let names = invocations
            .iter()
            .filter_map(|inv| inv.get("tool").and_then(|v| v.as_str()))
            .map(|tool| truncate(tool, 18))
            .collect::<Vec<_>>();
        if !names.is_empty() {
            let head = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            let more = names.len().saturating_sub(3);
            let tail = if more > 0 {
                format!(" +{more} more")
            } else {
                String::new()
            };
            return Some(format!("{} tools: {head}{tail}", names.len()));
        }
    }
    if let Some(command) = args.get("command").and_then(|v| v.as_str()) {
        let sub = args.get("subcommand").and_then(|v| v.as_str());
        let target = args
            .get("target")
            .or_else(|| args.get("ref"))
            .or_else(|| args.get("name"))
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str());
        let mut parts = vec![command.to_string()];
        if let Some(sub) = sub {
            parts.push(sub.to_string());
        }
        if let Some(target) = target {
            parts.push(target.to_string());
        }
        return Some(truncate(&parts.join(" "), 120));
    }
    for key in [
        "file_path",
        "path",
        "pattern",
        "query",
        "url",
        "description",
        "prompt",
        "old_string",
        "skill_name",
        "type",
    ] {
        if let Some(v) = args.get(key).and_then(|v| v.as_str()) {
            let v = v.replace('\n', " ");
            return Some(truncate(v.trim(), 120));
        }
    }
    None
}

pub(crate) fn arg_summary_for_tool(name: &str, args: &serde_json::Value) -> Option<String> {
    match name {
        "grep" | "search" => arg_from_keys(args, &["pattern", "path"]),
        "web_search" => arg_from_keys(args, &["query"]),
        "web_fetch" => arg_from_keys(args, &["url"]),
        "read" | "cat" | "write" | "create" | "edit" | "patch" | "apply_patch" => {
            arg_from_keys(args, &["file_path", "path"]).or_else(|| arg_summary(args))
        }
        "ls" | "glob" | "find" => arg_from_keys(args, &["pattern", "path"]),
        "skill" | "Skill" => arg_from_keys(args, &["skill_name", "description", "prompt"]),
        _ => arg_summary(args),
    }
}

fn arg_from_keys(args: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        args.get(*key).and_then(|v| v.as_str()).map(|v| {
            let v = v.replace('\n', " ");
            truncate(v.trim(), 120)
        })
    })
}

fn summarize_tasks(tasks: &[serde_json::Value], worker: Option<&str>) -> Option<String> {
    let descs = tasks
        .iter()
        .filter_map(|task| {
            task.as_str().or_else(|| {
                task.get("description")
                    .or_else(|| task.get("prompt"))
                    .or_else(|| task.get("task"))
                    .and_then(|v| v.as_str())
            })
        })
        .map(|s| truncate(&s.replace('\n', " "), 40))
        .collect::<Vec<_>>();
    if descs.is_empty() {
        return None;
    }
    let head = descs.iter().take(2).cloned().collect::<Vec<_>>().join("; ");
    let more = descs.len().saturating_sub(2);
    let tail = if more > 0 {
        format!(" +{more} more")
    } else {
        String::new()
    };
    let worker = worker
        .filter(|worker| !worker.trim().is_empty())
        .map(|worker| format!(" via {}", truncate(worker.trim(), 28)))
        .unwrap_or_default();
    Some(format!("{} tasks{worker}: {head}{tail}", descs.len()))
}

/// IDE-style unified diff: `└ path (+a -d)` header, then hunks with context
/// lines (dim, no marker), `-`/`+` changes, `⋮` between hunks, and long lines
/// wrapped with the code indented under a blank gutter.
pub(crate) fn render_diff(path: &str, before: &str, after: &str, width: usize) -> String {
    const MAX_DIFF_ROWS: usize = 200;

    render_diff_action("Edited", path, before, after, width, MAX_DIFF_ROWS)
}

fn render_diff_action(
    action: &str,
    path: &str,
    before: &str,
    after: &str,
    width: usize,
    max_rows: usize,
) -> String {
    let theme = agent_chrome_theme();
    let chrome = agent_chrome(&theme);
    let lang = lang_of(std::path::Path::new(path));
    chrome
        .diff_texts(path, before, after)
        .action(action)
        .header_colors(DIFF_HEADER_BULLET, DIFF_HEADER_ACTION, DIFF_HEADER_DETAIL)
        .context_color(DIFF_CODE_FG)
        .separator_color(DIFF_CONTEXT_GUTTER)
        .gutter_colors(DIFF_CONTEXT_GUTTER, DIFF_INSERT_GUTTER, DIFF_DELETE_GUTTER)
        .marker_colors(DIFF_INSERT_MARKER, DIFF_DELETE_MARKER)
        .changed_content_colors(DIFF_CODE_FG, mix_diff_color(DIFF_CODE_FG, DIFF_DELETE_BG))
        .changed_backgrounds(Some(DIFF_INSERT_BG), Some(DIFF_DELETE_BG))
        .highlight_content(|kind, content| {
            highlight_diff_spans(content, lang)
                .into_iter()
                .map(|span| {
                    let color = span.color.unwrap_or(DIFF_CODE_FG);
                    let color = if kind == DiffLineKind::Delete {
                        mix_diff_color(color, DIFF_DELETE_BG)
                    } else {
                        color
                    };
                    DiffSpan::new(span.content).color(color)
                })
                .collect()
        })
        .max_lines(max_rows)
        .view(
            width.min(u16::MAX as usize) as u16,
            max_rows.saturating_add(2),
        )
}

fn mix_diff_color(foreground: Color, background: Color) -> Color {
    match (foreground, background) {
        (Color::Rgb(fr, fg, fb), Color::Rgb(br, bg, bb)) => Color::Rgb(
            ((u16::from(fr) + u16::from(br)) / 2) as u8,
            ((u16::from(fg) + u16::from(bg)) / 2) as u8,
            ((u16::from(fb) + u16::from(bb)) / 2) as u8,
        ),
        (color, _) => color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_visible_lines_bounded(rendered: &str, width: usize) {
        for line in rendered.lines() {
            assert!(
                a3s_tui::style::visible_len(line) <= width,
                "line exceeds width {width}: {:?}",
                a3s_tui::style::strip_ansi(line)
            );
        }
    }

    #[test]
    fn highlight_shell_colors_tokens_and_preserves_text() {
        let s = highlight_shell("curl -s -X POST http://x");
        // Styling was applied (escape sequences present)...
        assert!(s.contains('\u{1b}'));
        // ...but the visible text is unchanged (single-spaced tokens).
        assert_eq!(a3s_tui::style::strip_ansi(&s), "curl -s -X POST http://x");
        assert_eq!(highlight_shell(""), "");
    }

    #[test]
    fn live_tool_activity_uses_shared_activity_block_for_status_and_output() {
        let args = serde_json::json!({
            "command": "cargo test very-long-filter-name -- --nocapture"
        });
        let output = (0..16)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = render_live_tool_activity("bash", Some(&args), &output, 48, true);
        let plain = a3s_tui::style::strip_ansi(&rendered);
        let rows = plain.lines().collect::<Vec<_>>();

        assert!(plain.contains("Running cargo test"), "{plain}");
        assert!(plain.contains("… +4 earlier lines"), "{plain}");
        assert!(!plain.contains("line-0"));
        assert!(plain.contains("line-4"));
        assert!(rows[1].starts_with("    │"), "{plain}");
        assert_visible_lines_bounded(&rendered, 48);
        assert!(rendered.contains("\x1b["));
    }

    #[test]
    fn running_tool_matrix_has_bounded_design_activity() {
        let width = 54;
        let cases = [
            (
                "bash",
                serde_json::json!({"command":"cargo test long-filter -- --nocapture"}),
                "Running cargo test",
            ),
            (
                "read",
                serde_json::json!({"file_path":"src/tui/ui/render.rs"}),
                "Reading src/tui/ui/render.rs",
            ),
            (
                "write",
                serde_json::json!({"file_path":"src/tui/new.rs"}),
                "Writing src/tui/new.rs",
            ),
            (
                "edit",
                serde_json::json!({"file_path":"src/tui/ui/render.rs"}),
                "Editing src/tui/ui/render.rs",
            ),
            (
                "grep",
                serde_json::json!({"pattern":"RuntimeExpectation", "path":"src"}),
                "Searching RuntimeExpectation",
            ),
            (
                "glob",
                serde_json::json!({"pattern":"src/**/*.rs"}),
                "Listing src/**/*.rs",
            ),
            (
                "web_search",
                serde_json::json!({"query":"A3S Runtime RemoteUI"}),
                "Searching A3S Runtime",
            ),
            (
                "web_fetch",
                serde_json::json!({"url":"https://example.com/very/long/path"}),
                "Fetching https://example.com",
            ),
            (
                "task",
                serde_json::json!({"description":"Audit terminal rendering"}),
                "Exploring Audit terminal",
            ),
            (
                "parallel_task",
                serde_json::json!({"tasks":["audit running state", "audit failure state"]}),
                "Exploring 2 tasks",
            ),
            (
                "runtime",
                serde_json::json!({"worker":"researcher", "tasks":["collect evidence", "summarize"]}),
                "Running Runtime 2 tasks",
            ),
            (
                "git",
                serde_json::json!({"command":"status", "target":"--short"}),
                "Running git status --short",
            ),
            (
                "batch",
                serde_json::json!({"invocations":[{"tool":"read"}, {"tool":"grep"}]}),
                "Running batch 2 tools",
            ),
            (
                "program",
                serde_json::json!({"type":"script"}),
                "Running program script",
            ),
            (
                "Skill",
                serde_json::json!({"skill_name":"inspect-surface", "prompt":"Apply"}),
                "Using skill inspect-surface",
            ),
        ];

        for (tool, args, expected) in cases {
            let rendered = render_live_tool_activity(tool, Some(&args), "", width, true);
            let plain = a3s_tui::style::strip_ansi(&rendered);
            assert!(plain.contains(expected), "{tool} got:\n{plain}");
            assert!(
                plain.trim_end().ends_with('…'),
                "{tool} should show running suffix: {plain}"
            );
            assert_visible_lines_bounded(&rendered, width);
        }
    }

    #[test]
    fn completed_tool_matrix_has_bounded_design_headers() {
        let width = 56;
        let cases = [
            (
                "bash",
                serde_json::json!({"command":"cargo test very-long-filter-name -- --nocapture"}),
                "Ran cargo test",
            ),
            (
                "read",
                serde_json::json!({"file_path":"README.md"}),
                "Read README.md",
            ),
            (
                "write",
                serde_json::json!({"file_path":"src/tui/new.rs"}),
                "Wrote src/tui/new.rs",
            ),
            (
                "edit",
                serde_json::json!({"file_path":"src/tui/ui/render.rs"}),
                "Edited src/tui/ui/render.rs",
            ),
            (
                "grep",
                serde_json::json!({"pattern":"RuntimeExpectation", "path":"src/tui/mod.rs"}),
                "Searched RuntimeExpectation",
            ),
            (
                "glob",
                serde_json::json!({"pattern":"src/**/*.rs"}),
                "Listed src/**/*.rs",
            ),
            (
                "web_search",
                serde_json::json!({"query":"A3S Runtime parallel remote UI report generation"}),
                "Searched web A3S Runtime",
            ),
            (
                "web_fetch",
                serde_json::json!({"url":"https://example.com/a/very/long/path/that/should/not/overflow"}),
                "Fetched https://example.com",
            ),
            (
                "runtime",
                serde_json::json!({
                    "worker":"research-agent-with-a-long-name",
                    "tasks":["collect sources for topic one", "compare alternatives for topic two", "summarize risks"]
                }),
                "Used Runtime 3 tasks",
            ),
            (
                "git",
                serde_json::json!({"command":"diff", "target":"HEAD~1"}),
                "Ran git diff HEAD~1",
            ),
            (
                "batch",
                serde_json::json!({
                    "invocations":[
                        {"tool":"read", "args":{"file_path":"README.md"}},
                        {"tool":"glob", "args":{"pattern":"**/*.rs"}},
                        {"tool":"grep", "args":{"pattern":"TODO"}}
                    ]
                }),
                "Ran batch 3 tools",
            ),
            (
                "task",
                serde_json::json!({"description":"Audit tool rendering states"}),
                "Explored Audit tool",
            ),
            (
                "parallel_task",
                serde_json::json!({"tasks":["audit running state", "audit failure state"]}),
                "Explored 2 tasks",
            ),
            (
                "program",
                serde_json::json!({"type":"script", "source":"async function run() {}"}),
                "Ran program script",
            ),
            (
                "Skill",
                serde_json::json!({"skill_name":"inspect-surface", "prompt":"Apply the skill"}),
                "Used skill inspect-surface",
            ),
        ];

        for (tool, args, expected) in cases {
            let rendered = render_tool_end(tool, 0, "ok\n", None, Some(&args), width);
            let plain = a3s_tui::style::strip_ansi(&rendered);
            assert!(
                plain.contains(expected),
                "{tool} should include {expected:?}, got:\n{plain}"
            );
            assert_visible_lines_bounded(&rendered, width);
        }
    }

    #[test]
    fn failed_completed_tool_uses_error_chrome_and_stays_bounded() {
        let args = serde_json::json!({
            "command": "npm run a-script-with-a-very-long-name-that-fails"
        });
        let rendered = render_tool_end(
            "bash",
            1,
            "first\nsecond\nthird\nfourth\nfifth\nsixth with a long tail that should be clipped",
            None,
            Some(&args),
            48,
        );
        let plain = a3s_tui::style::strip_ansi(&rendered);

        assert!(plain.contains("Ran npm run"));
        assert!(plain.contains("… +1 earlier lines"));
        assert_visible_lines_bounded(&rendered, 48);
    }

    #[test]
    fn completed_tool_output_uses_shared_output_block() {
        let rendered = render_tool_end(
            "bash",
            1,
            "first\nsecond\nthird\nfourth\nfifth\nsixth with a long tail that should be clipped",
            None,
            Some(&serde_json::json!({"command": "npm test"})),
            48,
        );
        let plain = a3s_tui::style::strip_ansi(&rendered);
        let lines = plain.lines().collect::<Vec<_>>();

        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("    ⎿  … +1 earlier lines")),
            "{plain}"
        );
        assert!(
            lines.iter().any(|line| line.starts_with("       second")),
            "{plain}"
        );
        assert!(
            rendered.contains(&format!("\x1b[{}msecond", TN_RED.fg_ansi())),
            "failed tool tail should use output block text color: {rendered:?}"
        );
        assert_visible_lines_bounded(&rendered, 48);
    }

    #[test]
    fn task_summary_uses_shared_checklist() {
        let meta = serde_json::json!({
            "agent": "review",
            "task_id": "task-with-a-long-id-that-still-fits",
            "success": false,
            "output_bytes": 0
        });
        let rendered = render_tool_end("task", 1, "Task failed\nOutput:\n", Some(&meta), None, 44);
        let plain = a3s_tui::style::strip_ansi(&rendered);
        let lines = plain.lines().collect::<Vec<_>>();

        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("    ⎿  ✗ Task failed")),
            "{plain}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("       · no child text output")),
            "{plain}"
        );
        assert!(
            rendered.contains(&format!("\x1b[{}m✗", TN_RED.fg_ansi())),
            "failed task summary should use checklist error glyph color: {rendered:?}"
        );
        assert!(
            rendered.contains(&format!("\x1b[{}mTask failed", TN_RED.fg_ansi())),
            "failed task summary should use checklist error text color: {rendered:?}"
        );
        assert_visible_lines_bounded(&rendered, 44);
    }

    #[test]
    fn parallel_task_summary_marks_each_result_with_checklist_status() {
        let meta = serde_json::json!({
            "results": [
                {
                    "agent": "plan",
                    "task_id": "task-ok",
                    "success": true,
                    "output_bytes": 42,
                    "output": "Task completed\nOutput:\nready"
                },
                {
                    "agent": "review",
                    "task_id": "task-fail",
                    "success": false,
                    "output_bytes": 0,
                    "output": "Task failed\nOutput:\n"
                }
            ]
        });
        let rendered = render_tool_end("parallel_task", 0, "", Some(&meta), None, 58);
        let plain = a3s_tui::style::strip_ansi(&rendered);

        assert!(plain.contains("    ⎿  ✓ 1/2 agents done"), "{plain}");
        assert!(plain.contains("       ✓ plan · task-ok · ready"), "{plain}");
        assert!(
            plain.contains("       ✗ review · task-fail · no child text output"),
            "{plain}"
        );
        assert!(
            rendered.contains(&format!("\x1b[{}m✓", TN_GREEN.fg_ansi())),
            "successful task rows should use checklist success glyphs: {rendered:?}"
        );
        assert!(
            rendered.contains(&format!("\x1b[{}m✗", TN_RED.fg_ansi())),
            "failed task rows should use checklist error glyphs: {rendered:?}"
        );
        assert_visible_lines_bounded(&rendered, 58);
    }

    #[test]
    fn failed_completed_tool_matrix_uses_error_accent_and_stays_bounded() {
        let width = 46;
        let cases = [
            (
                "bash",
                serde_json::json!({"command":"cargo test failing-filter"}),
            ),
            (
                "grep",
                serde_json::json!({"pattern":"needle", "path":"src"}),
            ),
            (
                "web_fetch",
                serde_json::json!({"url":"https://example.com/failing"}),
            ),
            (
                "runtime",
                serde_json::json!({"tasks":["run failing branch"]}),
            ),
            ("Skill", serde_json::json!({"skill_name":"inspect-surface"})),
        ];

        for (tool, args) in cases {
            let rendered = render_tool_end(
                tool,
                1,
                "first\nsecond\nthird\nfourth\nfifth\nsixth",
                None,
                Some(&args),
                width,
            );
            assert!(
                rendered.contains(&TN_RED.fg_ansi()),
                "{tool} got:\n{rendered:?}"
            );
            assert_visible_lines_bounded(&rendered, width);
        }
    }

    #[test]
    fn diff_rendering_uses_shared_diff_view_and_bounds_rows() {
        let before = "let old_value = \"a very long old value that should wrap instead of escaping the viewport\";\nkeep();\n";
        let after = "let new_value = \"a very long new value that should wrap instead of escaping the viewport\";\nkeep();\n";
        let rendered = render_diff(
            "src/tui/a/very/long/path/that/should/not/overflow/render.rs",
            before,
            after,
            48,
        );
        let plain = a3s_tui::style::strip_ansi(&rendered);
        let header = rendered.lines().next().expect("diff header");

        assert!(plain.contains("Edited"));
        assert!(plain.contains("+1") && plain.contains("-1"));
        assert!(
            header.contains(&Style::new().fg(DIFF_HEADER_BULLET).bold().render("•")),
            "diff bullet should use the muted header color: {header:?}"
        );
        assert!(
            header.contains(&Style::new().fg(DIFF_HEADER_ACTION).bold().render("Edited")),
            "diff action should use the bright header color: {header:?}"
        );
        assert!(
            header.contains(&Style::new().fg(DIFF_INSERT_MARKER).bold().render("+1")),
            "addition count should use the insert marker color: {header:?}"
        );
        assert!(
            header.contains(&Style::new().fg(DIFF_DELETE_MARKER).bold().render("-1")),
            "deletion count should use the delete marker color: {header:?}"
        );
        assert!(
            rendered.contains(&DIFF_INSERT_BG.bg_ansi()),
            "insert rows should use the reference background: {rendered:?}"
        );
        assert!(
            rendered.contains(&DIFF_DELETE_BG.bg_ansi()),
            "delete rows should use the reference background: {rendered:?}"
        );
        assert!(
            rendered.contains(
                &Style::new()
                    .fg(Color::Rgb(210, 164, 253))
                    .bg(DIFF_INSERT_BG)
                    .render("let")
            ),
            "inserted Rust should retain syntax highlighting: {rendered:?}"
        );
        assert!(
            rendered.contains(
                &Style::new()
                    .fg(mix_diff_color(Color::Rgb(210, 164, 253), DIFF_DELETE_BG))
                    .bg(DIFF_DELETE_BG)
                    .render("let")
            ),
            "deleted Rust should use the muted syntax color: {rendered:?}"
        );
        assert_visible_lines_bounded(&rendered, 48);
    }

    #[test]
    fn arg_summary_handles_runtime_string_tasks_and_batch_invocations() {
        assert_eq!(
            arg_summary(&serde_json::json!({
                "worker":"researcher",
                "tasks":["alpha branch", "beta branch"]
            })),
            Some("2 tasks via researcher: alpha branch; beta branch".to_string())
        );
        assert_eq!(
            arg_summary(&serde_json::json!({
                "invocations":[
                    {"tool":"read", "args":{}},
                    {"tool":"grep", "args":{}}
                ]
            })),
            Some("2 tools: read, grep".to_string())
        );
    }
}
