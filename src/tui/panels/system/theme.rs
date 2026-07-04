//! `/theme` overlay: theme list + live syntax-highlight preview.

use super::super::*;

fn theme_line(rendered: &str, width: usize) -> String {
    pad_to(&truncate(rendered, width), width)
}

impl App {
    /// `/theme` picker: a theme list + a live syntax-highlight preview.
    pub(crate) fn overlay_theme(&self, composed: String) -> String {
        let Some(sel) = self.theme_panel else {
            return composed;
        };
        let width = self.width as usize;
        let mut menu = vec![theme_line(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  Theme — ↑/↓ preview · Enter apply · Esc"),
            width,
        )];
        for (i, th) in THEMES.iter().enumerate() {
            let marker = if i == sel { "▸" } else { " " };
            let raw = theme_line(&format!("  {marker} {}", th.name), width);
            menu.push(if i == sel {
                Style::new().fg(Color::BrightWhite).bg(ACCENT).render(&raw)
            } else {
                Style::new().fg(TN_GRAY).render(&raw)
            });
        }
        menu.push(theme_line(
            &Style::new().fg(TN_GRAY).render("  ── preview ──"),
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
            menu.push(theme_line(
                &format!("    {}", highlight_with(line, "rust", th)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_lines_are_width_bounded_with_styles() {
        let line = theme_line(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  Theme — a very long prompt that must not overflow"),
            24,
        );

        assert!(
            a3s_tui::style::visible_len(&line) <= 24,
            "{}",
            a3s_tui::style::strip_ansi(&line)
        );
    }
}
