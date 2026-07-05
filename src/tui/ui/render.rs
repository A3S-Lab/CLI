//! Rendering of completed tool calls: labels, arg summaries, and file diffs.

use super::*;
use a3s_tui::components::{ConnectorBlock, ConnectorRow};

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
    let header = render_tool_header(name, ok, args, width);

    // For a successful file read on a known language, show the highlighted file
    // content under the action (keeps the nice read preview).
    if ok && matches!(name, "read" | "cat") {
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
        if let Some(summary) = render_task_tool_summary(name, output, meta, ok, width) {
            return format!("{header}{summary}");
        }
    }

    // Show only the latest TAIL output lines under a "⎿" connector, with a
    // "… +N earlier lines" marker when there's more (keeps a noisy build tight).
    const TAIL: usize = 5;
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return header;
    }
    let body_color = if ok { TN_GRAY } else { TN_RED };
    let block = ConnectorBlock::new()
        .margin(PAD)
        .connector_indent(2)
        .connector_gap(2)
        .connector_color(TN_GRAY)
        .text_color(body_color)
        .omitted_color(TN_GRAY)
        .max_rows(TAIL)
        .rows(
            lines
                .into_iter()
                .map(|line| ConnectorRow::new(line.to_string()))
                .collect(),
        )
        .view(width.min(u16::MAX as usize) as u16);

    if block.is_empty() {
        header
    } else {
        format!("{header}\n{block}")
    }
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
    let mut rows = vec![format!(
        "Task {status} · {agent}{}",
        task_id_suffix(task_id)
    )];
    if let Some(excerpt) = task_child_excerpt(output) {
        rows.extend(excerpt.lines().map(str::to_string));
    } else if output_bytes == Some(0) {
        rows.push("no child text output; using plan/status for synthesis".to_string());
    } else {
        rows.push("child output stored in task artifact".to_string());
    }
    if let Some(uri) = artifact {
        rows.push(format!("artifact: {}", truncate(uri, 96)));
    }
    Some(render_task_rows(&rows, success, width))
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
    let mut rows = vec![format!("{done}/{} agents done", results.len())];
    for result in results.iter().take(4) {
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(ok);
        let mark = if success { "✓" } else { "✗" };
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
        rows.push(format!(
            "{mark} {agent}{} · {detail}",
            task_id_suffix(task_id)
        ));
    }
    let more = results.len().saturating_sub(4);
    if more > 0 {
        rows.push(format!("+{more} more agent result(s)"));
    }
    Some(render_task_rows(&rows, ok, width))
}

fn render_task_rows(rows: &[String], ok: bool, width: usize) -> String {
    let body_color = if ok { TN_GRAY } else { TN_RED };
    let block = ConnectorBlock::new()
        .margin(PAD)
        .connector_indent(2)
        .connector_gap(2)
        .connector_color(TN_GRAY)
        .text_color(body_color)
        .show_omitted_count(false)
        .rows(
            rows.iter()
                .map(|row| ConnectorRow::new(row.clone()))
                .collect(),
        )
        .view(width.min(u16::MAX as usize) as u16);

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
    let margin = " ".repeat(PAD);
    let dot = Style::new()
        .fg(if ok { TN_GREEN } else { TN_RED })
        .bold()
        .render("•");
    let verb = Style::new().bold().render(tool_verb(name));
    let base = format!("{margin}{dot} {verb}");
    let arg = args
        .and_then(|args| arg_summary_for_tool(name, args))
        .unwrap_or_default();
    if arg.is_empty() {
        return a3s_tui::style::truncate_visible(&base, width);
    }

    let arg_width = width
        .saturating_sub(a3s_tui::style::visible_len(&base) + 1)
        .max(8);
    let arg = truncate(&arg, arg_width);
    let arg_styled = if matches!(name, "bash" | "shell" | "run" | "exec") {
        highlight_shell(&arg)
    } else {
        Style::new().fg(TN_GRAY).render(&arg)
    };
    let header = format!("{base} {arg_styled}");
    a3s_tui::style::truncate_visible(&header, width)
}

pub(crate) fn render_live_tool_status(
    name: &str,
    args: Option<&serde_json::Value>,
    width: usize,
    active: bool,
) -> String {
    let margin = " ".repeat(PAD);
    let dot = Style::new()
        .fg(if active { ACCENT } else { TN_GRAY })
        .bold()
        .render("•");
    let verb = tool_running_verb(name);
    let suffix = Style::new().fg(TN_GRAY).render("…");
    let base = format!("{margin}{dot} {verb}");
    let arg = args
        .and_then(|args| arg_summary_for_tool(name, args))
        .unwrap_or_default();
    if arg.is_empty() {
        return a3s_tui::style::truncate_visible(&format!("{base}{suffix}"), width);
    }

    let arg_width = width
        .saturating_sub(PAD + 1 + 1 + verb.chars().count() + 1 + 1)
        .max(12);
    let arg = truncate(&arg, arg_width);
    let arg = if matches!(name, "bash" | "shell" | "run" | "exec") {
        highlight_shell(&arg)
    } else {
        Style::new().fg(TN_GRAY).render(&arg)
    };
    a3s_tui::style::truncate_visible(&format!("{base} {arg}{suffix}"), width)
}

pub(crate) fn render_live_tool_output(output: &str, width: usize) -> Option<String> {
    let lines: Vec<&str> = output.lines().collect();
    if lines.iter().all(|line| line.trim().is_empty()) {
        return None;
    }

    const TAIL: usize = 12;
    let margin = " ".repeat(PAD + 2);
    let bar = Style::new().fg(TN_GRAY).render("│");
    let textw = width.saturating_sub(PAD + 6).max(20);
    let start = lines.len().saturating_sub(TAIL);
    let mut rows = Vec::new();

    if start > 0 {
        let row = format!(
            "{margin}{bar} {}",
            Style::new()
                .fg(TN_GRAY)
                .render(&format!("… +{start} earlier lines"))
        );
        rows.push(a3s_tui::style::truncate_visible(&row, width));
    }

    for line in lines.iter().skip(start) {
        let shown = if line.trim().is_empty() {
            String::new()
        } else {
            truncate(line, textw)
        };
        let row = format!("{margin}{bar} {}", Style::new().fg(TN_GRAY).render(&shown));
        rows.push(a3s_tui::style::truncate_visible(&row, width));
    }

    Some(rows.join("\n"))
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
pub(crate) fn render_diff(path: &str, before: &str, after: &str, width: usize) -> String {
    use similar::{ChangeTag, TextDiff};
    const MAX_LINES: usize = 200;

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
            let row = Style::new()
                .fg(TN_GRAY)
                .render(&format!("    {} ⋮", " ".repeat(nw)));
            lines.push(a3s_tui::style::truncate_visible(&row, width));
        }
        for op in group {
            for change in diff.iter_changes(op) {
                if lines.len() >= MAX_LINES {
                    truncated = true;
                    break;
                }
                let raw = change.value();
                let raw = raw.strip_suffix('\n').unwrap_or(raw);
                // Deleted lines get a red wash, inserted lines use the active
                // blue wash from the DESIGN.md palette. Context lines stay dim.
                // Plain high-contrast text, not syntax-highlight: syntax colors
                // clash on a colored background.
                let (no, marker, line_bg, line_fg) = match change.tag() {
                    ChangeTag::Delete => (
                        change.old_index().map(|i| i + 1).unwrap_or(0),
                        '-',
                        Some(Color::Rgb(82, 30, 34)),
                        Color::Rgb(255, 215, 215),
                    ),
                    ChangeTag::Insert => (
                        change.new_index().map(|i| i + 1).unwrap_or(0),
                        '+',
                        Some(SURFACE_SELECTED),
                        Color::Rgb(211, 229, 255),
                    ),
                    ChangeTag::Equal => (
                        change.old_index().map(|i| i + 1).unwrap_or(0),
                        ' ',
                        None,
                        TN_GRAY,
                    ),
                };
                for (si, seg) in wrap_plain(raw, code_w).iter().enumerate() {
                    let content = if si == 0 {
                        format!("    {no:>width$} {marker} {seg}", width = nw)
                    } else {
                        format!("{cont_pad}{seg}")
                    };
                    let line = match line_bg {
                        // Changed line: fill the full row width with the bg color.
                        Some(bg) => Style::new()
                            .fg(line_fg)
                            .bg(bg)
                            .render(&pad_to(&content, width)),
                        // Context line: dim foreground, no background.
                        None => Style::new().fg(line_fg).render(&content),
                    };
                    lines.push(a3s_tui::style::truncate_visible(&line, width));
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
        let row = Style::new().fg(TN_GRAY).render("    … (diff truncated)");
        lines.push(a3s_tui::style::truncate_visible(&row, width));
    }
    let edit_prefix = format!(
        "  {} {} ",
        Style::new().fg(TN_GREEN).bold().render("•"),
        Style::new().render("Edited"),
    );
    let counts = Style::new()
        .fg(TN_GRAY)
        .render(&format!("(+{adds} -{dels})"));
    let path_width = width
        .saturating_sub(
            a3s_tui::style::visible_len(&edit_prefix) + a3s_tui::style::visible_len(&counts) + 1,
        )
        .max(8);
    let header = format!("{}{} {}", edit_prefix, truncate(path, path_width), counts);
    let mut out = a3s_tui::style::truncate_visible(&header, width);
    if !lines.is_empty() {
        out.push('\n');
        out.push_str(&lines.join("\n"));
    }
    out
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
    fn live_tool_status_is_bounded_and_uses_active_accent() {
        let args = serde_json::json!({
            "command": "cargo test extremely-long-filter-name-that-should-not-expand-the-layout -- --nocapture"
        });
        let rendered = render_live_tool_status("bash", Some(&args), 48, true);
        let plain = a3s_tui::style::strip_ansi(&rendered);

        assert!(rendered.contains("\x1b["));
        assert!(plain.contains("Running cargo test"));
        assert!(plain.ends_with('…'));
        assert!(
            plain.chars().count() <= 48,
            "line should stay inside the viewport: {plain:?}"
        );
    }

    #[test]
    fn live_tool_output_tails_and_bounds_lines() {
        let output = (0..16)
            .map(|i| {
                if i == 15 {
                    "final-line-with-a-very-long-payload-that-should-be-truncated".to_string()
                } else {
                    format!("line-{i}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let rendered = render_live_tool_output(&output, 44).expect("non-empty output renders");
        let plain = a3s_tui::style::strip_ansi(&rendered);

        assert!(plain.contains("… +4 earlier lines"));
        assert!(!plain.contains("line-0"));
        assert!(plain.contains("line-4"));
        for line in plain.lines() {
            assert!(
                line.chars().count() <= 44,
                "live output line should be bounded: {line:?}"
            );
        }
    }

    #[test]
    fn running_tool_matrix_has_bounded_design_status() {
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
            let rendered = render_live_tool_status(tool, Some(&args), width, true);
            let plain = a3s_tui::style::strip_ansi(&rendered);
            assert!(plain.contains(expected), "{tool} got:\n{plain}");
            assert!(
                plain.ends_with('…'),
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
    fn completed_tool_output_uses_shared_connector_block() {
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
            "failed tool tail should use connector block text color: {rendered:?}"
        );
        assert_visible_lines_bounded(&rendered, 48);
    }

    #[test]
    fn task_summary_uses_shared_connector_block() {
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
                .any(|line| line.starts_with("    ⎿  Task failed")),
            "{plain}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("       no child text output")),
            "{plain}"
        );
        assert!(
            rendered.contains(&format!("\x1b[{}mTask failed", TN_RED.fg_ansi())),
            "failed task summary should use the connector block text color: {rendered:?}"
        );
        assert_visible_lines_bounded(&rendered, 44);
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
    fn diff_rendering_bounds_long_paths_and_lines() {
        let before = "let old_value = \"a very long old value that should wrap instead of escaping the viewport\";\nkeep();\n";
        let after = "let new_value = \"a very long new value that should wrap instead of escaping the viewport\";\nkeep();\n";
        let rendered = render_diff(
            "src/tui/a/very/long/path/that/should/not/overflow/render.rs",
            before,
            after,
            48,
        );
        let plain = a3s_tui::style::strip_ansi(&rendered);

        assert!(plain.contains("Edited"));
        assert!(plain.contains("+1") && plain.contains("-1"));
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
