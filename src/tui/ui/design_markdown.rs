//! DESIGN.md-aligned Markdown rendering for the coding transcript.
//!
//! The upstream `a3s-tui` renderer handles CommonMark and syntax highlighting,
//! but several block shapes can exceed the terminal width. This wrapper keeps
//! that parser/highlighter while enforcing the TUI's fixed-width contract and a
//! quieter Geist/Vercel-style surface.

use a3s_tui::style::{strip_ansi, truncate_visible, visible_len, Color, Style};

use super::{wrap_words, ACCENT, SURFACE_SOFT, TN_FG, TN_GRAY};

pub(crate) struct Markdown {
    width: usize,
}

impl Markdown {
    pub(crate) fn new() -> Self {
        Self { width: 80 }
    }

    pub(crate) fn with_width(mut self, width: usize) -> Self {
        self.width = width.max(12);
        self
    }

    pub(crate) fn render(&self, input: &str) -> String {
        let input = compact_pipe_tables(input, self.width);
        let rendered = a3s_tui::markdown::Markdown::new()
            .with_width(self.width)
            .render(&input);
        let rendered = normalize_markdown_colors(&rendered);
        bound_rendered_markdown(&rendered, self.width).join("\n")
    }
}

impl Default for Markdown {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct StreamingMarkdown {
    buffer: String,
    rendered_lines: Vec<String>,
    md: Markdown,
}

impl StreamingMarkdown {
    pub(crate) fn new(width: usize) -> Self {
        Self {
            buffer: String::new(),
            rendered_lines: Vec::new(),
            md: Markdown::new().with_width(width),
        }
    }

    pub(crate) fn push(&mut self, token: &str) {
        self.buffer.push_str(token);
        self.rerender();
    }

    pub(crate) fn clear(&mut self) {
        self.buffer.clear();
        self.rendered_lines.clear();
    }

    pub(crate) fn view(&self) -> String {
        self.rendered_lines.join("\n")
    }

    pub(crate) fn raw_content(&self) -> &str {
        &self.buffer
    }

    fn rerender(&mut self) {
        let rendered = self.md.render(&self.buffer);
        self.rendered_lines = rendered.lines().map(str::to_string).collect();
    }
}

fn bound_rendered_markdown(rendered: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in rendered.lines() {
        let plain = strip_ansi(line);
        if plain.starts_with("│ ") && visible_len(line) > width {
            rows.extend(wrap_blockquote_line(&plain, width));
        } else {
            rows.push(truncate_visible(line, width));
        }
    }
    rows
}

fn normalize_markdown_colors(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            let mut params = String::new();
            let mut final_byte = None;
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    final_byte = Some(next);
                    break;
                }
                params.push(next);
            }
            if final_byte == Some('m') {
                out.push_str(&normalize_sgr(&params));
            } else {
                out.push_str("\x1b[");
                out.push_str(&params);
                if let Some(final_byte) = final_byte {
                    out.push(final_byte);
                }
            }
            continue;
        }
        out.push(ch);
    }

    out
}

fn normalize_sgr(params: &str) -> String {
    let parts = if params.is_empty() {
        vec!["0"]
    } else {
        params.split(';').collect::<Vec<_>>()
    };
    let mut codes = Vec::new();
    let mut i = 0;

    while i < parts.len() {
        let Some(code) = parts[i].parse::<u16>().ok() else {
            i += 1;
            continue;
        };
        match code {
            38 if i + 1 < parts.len() && parts[i + 1] == "2" && i + 4 < parts.len() => {
                let rgb = (
                    parts[i + 2].parse::<u8>().ok(),
                    parts[i + 3].parse::<u8>().ok(),
                    parts[i + 4].parse::<u8>().ok(),
                );
                if let (Some(r), Some(g), Some(b)) = rgb {
                    codes.push(design_fg_for_rgb(r, g, b).fg_ansi());
                }
                i += 5;
            }
            48 if i + 1 < parts.len() && parts[i + 1] == "2" && i + 4 < parts.len() => {
                codes.push(SURFACE_SOFT.bg_ansi());
                i += 5;
            }
            38 if i + 1 < parts.len() && parts[i + 1] == "5" && i + 2 < parts.len() => {
                codes.push(TN_FG.fg_ansi());
                i += 3;
            }
            48 if i + 1 < parts.len() && parts[i + 1] == "5" && i + 2 < parts.len() => {
                codes.push(SURFACE_SOFT.bg_ansi());
                i += 3;
            }
            30..=37 | 90..=97 => {
                codes.push(design_fg_for_ansi(code).fg_ansi());
                i += 1;
            }
            40..=47 | 100..=107 => {
                codes.push(SURFACE_SOFT.bg_ansi());
                i += 1;
            }
            0 | 1 | 2 | 3 | 4 | 7 | 9 => {
                codes.push(code.to_string());
                i += 1;
            }
            _ => i += 1,
        }
    }

    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

fn design_fg_for_rgb(r: u8, g: u8, b: u8) -> Color {
    match (r, g, b) {
        // Upstream h1/list blue and h3 cyan become the single active accent.
        (122, 162, 247) | (125, 207, 255) => ACCENT,
        // Upstream table borders / low-emphasis syntax are muted structure.
        (86, 95, 137) | (128, 128, 128) => TN_GRAY,
        _ => TN_FG,
    }
}

fn design_fg_for_ansi(code: u16) -> Color {
    match code {
        32 | 34 | 36 | 92 | 94 | 96 => ACCENT,
        30 | 90 => TN_GRAY,
        _ => TN_FG,
    }
}

fn wrap_blockquote_line(plain: &str, width: usize) -> Vec<String> {
    let body = plain.strip_prefix("│ ").unwrap_or(plain).trim();
    let bar = Style::new().fg(TN_GRAY).render("│");
    let text_style = Style::new().fg(TN_FG);
    wrap_words(body, width.saturating_sub(2).max(8))
        .into_iter()
        .map(|line| {
            let row = format!("{bar} {}", text_style.render(&line));
            truncate_visible(&row, width)
        })
        .collect()
}

fn compact_pipe_tables(input: &str, width: usize) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if i + 1 < lines.len() && looks_like_table_row(lines[i]) && is_table_separator(lines[i + 1])
        {
            let start = i;
            i += 2;
            while i < lines.len() && looks_like_table_row(lines[i]) {
                i += 1;
            }
            out.extend(compact_table_block(&lines[start..i], width));
        } else {
            out.push(lines[i].to_string());
            i += 1;
        }
    }
    out.join("\n")
}

fn compact_table_block(lines: &[&str], width: usize) -> Vec<String> {
    if lines.len() < 2 {
        return lines.iter().map(|line| (*line).to_string()).collect();
    }

    let rows = lines
        .iter()
        .map(|line| table_cells(line))
        .collect::<Vec<_>>();
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let cell_budget = width
        .saturating_sub(cols.saturating_mul(3).saturating_add(1))
        .checked_div(cols)
        .unwrap_or(4)
        .clamp(4, 28);

    let mut out = Vec::new();
    for (idx, row) in rows.iter().enumerate() {
        if idx == 1 {
            out.push(format!(
                "|{}|",
                (0..cols)
                    .map(|_| format!(" {} ", "-".repeat(cell_budget.max(3))))
                    .collect::<Vec<_>>()
                    .join("|")
            ));
            continue;
        }
        let cells = (0..cols)
            .map(|col| {
                let cell = row.get(col).map(String::as_str).unwrap_or("");
                format!(" {} ", truncate_visible(cell.trim(), cell_budget))
            })
            .collect::<Vec<_>>()
            .join("|");
        out.push(format!("|{cells}|"));
    }
    out
}

fn looks_like_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('|') && trimmed.matches('|').count() >= 2
}

fn is_table_separator(line: &str) -> bool {
    if !looks_like_table_row(line) {
        return false;
    }
    table_cells(line).iter().all(|cell| {
        let cell = cell.trim();
        !cell.is_empty()
            && cell.chars().all(|ch| matches!(ch, '-' | ':' | ' '))
            && cell.chars().filter(|ch| *ch == '-').count() >= 3
    })
}

fn table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_bounded(rendered: &str, width: usize) {
        for line in rendered.lines() {
            assert!(
                visible_len(line) <= width,
                "line over width {width}: {:?}",
                strip_ansi(line)
            );
        }
    }

    #[test]
    fn code_blocks_are_width_bounded() {
        let rendered = Markdown::new().with_width(36).render(
            "```rust\nlet value = \"a very very very long code line that should not escape\";\n```",
        );
        let plain = strip_ansi(&rendered);

        assert!(plain.contains("rust"));
        assert!(plain.contains("let value"));
        assert_bounded(&rendered, 36);
    }

    #[test]
    fn pipe_tables_are_compacted_before_rendering() {
        let rendered = Markdown::new().with_width(48).render(
            "| Tool | Very long description column |\n\
             | --- | --- |\n\
             | bash | a very long explanation that should be compacted into the viewport |\n",
        );
        let plain = strip_ansi(&rendered);

        assert!(plain.contains("Tool"));
        assert!(plain.contains("bash"));
        assert_bounded(&rendered, 48);
    }

    #[test]
    fn blockquotes_wrap_instead_of_overflowing() {
        let rendered = Markdown::new().with_width(42).render(
            "> This is a deliberately long quote that needs to wrap inside the terminal transcript.",
        );
        let plain = strip_ansi(&rendered);

        assert!(plain.lines().count() > 1, "{plain}");
        assert!(plain.lines().all(|line| line.starts_with("│ ")));
        assert_bounded(&rendered, 42);
    }

    #[test]
    fn streaming_markdown_uses_same_bounded_renderer() {
        let mut streaming = StreamingMarkdown::new(34);
        streaming.push("| A | B |\n| --- | --- |\n| alpha | beta gamma delta epsilon zeta |\n");

        assert!(streaming.raw_content().contains("alpha"));
        assert_bounded(&streaming.view(), 34);
    }

    #[test]
    fn markdown_colors_are_normalized_to_design_palette() {
        let rendered = Markdown::new().with_width(72).render(
            "# Heading\n\
             - item with [link](https://example.com) and `code`\n\
             - [x] done\n\
             ```rust\n\
             let value = 1;\n\
             ```",
        );

        assert!(rendered.contains(&ACCENT.fg_ansi()), "{rendered:?}");
        assert!(rendered.contains(&SURFACE_SOFT.bg_ansi()), "{rendered:?}");
        assert!(!rendered.contains("122;162;247"), "{rendered:?}");
        assert!(!rendered.contains("187;154;247"), "{rendered:?}");
        assert!(!rendered.contains("\x1b[33m"), "{rendered:?}");
        assert!(!rendered.contains("\x1b[32m"), "{rendered:?}");
        assert!(!rendered.contains("\x1b[4;34m"), "{rendered:?}");
        assert_bounded(&rendered, 72);
    }
}
