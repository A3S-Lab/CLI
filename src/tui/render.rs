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
        .fg(if ok { Color::Green } else { Color::Red })
        .bold()
        .render("•");
    let arg = args.and_then(arg_summary).unwrap_or_default();
    let header = if arg.is_empty() {
        format!(
            "{margin}{dot} {}",
            Style::new().bold().render(tool_verb(name))
        )
    } else {
        format!(
            "{margin}{dot} {} {}",
            Style::new().bold().render(tool_verb(name)),
            Style::new().fg(Color::BrightBlack).render(&arg)
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

    // Show only the latest TAIL output lines under a "⎿" connector, with a
    // "… +N earlier lines" marker when there's more (keeps a noisy build tight).
    const TAIL: usize = 5;
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return header;
    }
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
    let mut out = header;
    let start = lines.len().saturating_sub(TAIL);
    if start > 0 {
        out.push_str(&format!(
            "\n{margin}  {conn}  {}",
            Style::new()
                .fg(Color::BrightBlack)
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
                // Deleted lines get a red background, inserted lines a green one,
                // so the change type reads at a glance. Context lines stay dim and
                // unbackgrounded. (Plain high-contrast text, not syntax-highlight:
                // syntax colors clash on a colored background.)
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
                        Some(Color::Rgb(26, 60, 36)),
                        Color::Rgb(205, 255, 210),
                    ),
                    ChangeTag::Equal => (
                        change.old_index().map(|i| i + 1).unwrap_or(0),
                        ' ',
                        None,
                        Color::BrightBlack,
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
        lines.push(
            Style::new()
                .fg(Color::BrightBlack)
                .render("    … (diff truncated)"),
        );
    }
    let mut out = format!(
        "  {} {} {}",
        Style::new().fg(Color::Green).bold().render("•"),
        Style::new().render(&format!("Edited {path}")),
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
