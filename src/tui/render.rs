//! Rendering of completed tool calls: labels, arg summaries, and file diffs.

use super::*;

/// Compact token count: `50700` → `50.7k`.
pub(crate) fn fmt_tokens(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

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
    let margin = " ".repeat(PAD);
    // Header: "• Ran npm test" / "• Read src/main.rs" — Codex style, a past-tense
    // verb + the arg, the bullet colored by outcome.
    let dot = Style::new()
        .fg(if ok { TN_GREEN } else { TN_RED })
        .bold()
        .render("•");
    let arg = args.and_then(arg_summary).unwrap_or_default();
    // Bash commands get Codex-style token coloring (program vs flags vs args);
    // everything else keeps the muted single-color summary.
    let arg_styled = if matches!(name, "bash" | "shell" | "run" | "exec") {
        highlight_shell(&arg)
    } else {
        Style::new().fg(TN_GRAY).render(&arg)
    };
    let header = if arg.is_empty() {
        format!(
            "{margin}{dot} {}",
            Style::new().bold().render(tool_verb(name))
        )
    } else {
        format!(
            "{margin}{dot} {} {}",
            Style::new().bold().render(tool_verb(name)),
            arg_styled
        )
    };

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
    let conn = Style::new().fg(TN_GRAY).render("⎿");
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
    let mut out = header;
    let start = lines.len().saturating_sub(TAIL);
    if start > 0 {
        out.push_str(&format!(
            "\n{margin}  {conn}  {}",
            Style::new()
                .fg(TN_GRAY)
                .render(&format!("… +{start} earlier lines"))
        ));
        for line in lines.iter().skip(start) {
            out.push_str(&line_at(1, line));
        }
    } else {
        for (i, line) in lines.iter().enumerate() {
            out.push_str(&line_at(i, line));
        }
    }
    out
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
    let margin = " ".repeat(PAD);
    let conn = Style::new().fg(TN_GRAY).render("⎿");
    let body_color = if ok { TN_GRAY } else { TN_RED };
    let textw = width.saturating_sub(PAD + 7).max(20);
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        let shown = truncate(row, textw);
        if i == 0 {
            out.push_str(&format!(
                "\n{margin}  {conn}  {}",
                Style::new().fg(body_color).render(&shown)
            ));
        } else {
            out.push_str(&format!(
                "\n{margin}     {}",
                Style::new().fg(body_color).render(&shown)
            ));
        }
    }
    out
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
        other => other,
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

/// Extract a one-line summary of a tool's primary argument.
pub(crate) fn arg_summary(args: &serde_json::Value) -> Option<String> {
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
            lines.push(
                Style::new()
                    .fg(TN_GRAY)
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
                    lines.push(line);
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
        lines.push(Style::new().fg(TN_GRAY).render("    … (diff truncated)"));
    }
    let mut out = format!(
        "  {} {} {}",
        Style::new().fg(TN_GREEN).bold().render("•"),
        Style::new().render(&format!("Edited {path}")),
        Style::new()
            .fg(TN_GRAY)
            .render(&format!("(+{adds} -{dels})")),
    );
    if !lines.is_empty() {
        out.push('\n');
        out.push_str(&lines.join("\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_shell_colors_tokens_and_preserves_text() {
        let s = highlight_shell("curl -s -X POST http://x");
        // Styling was applied (escape sequences present)...
        assert!(s.contains('\u{1b}'));
        // ...but the visible text is unchanged (single-spaced tokens).
        assert_eq!(a3s_tui::style::strip_ansi(&s), "curl -s -X POST http://x");
        assert_eq!(highlight_shell(""), "");
    }
}
