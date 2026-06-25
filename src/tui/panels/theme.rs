//! `/theme` overlay: theme list + live syntax-highlight preview.

use super::super::*;

impl App {
    /// `/theme` picker: a theme list + a live syntax-highlight preview.
    pub(crate) fn overlay_theme(&self, composed: String) -> String {
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
}
