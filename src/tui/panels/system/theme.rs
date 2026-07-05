//! `/theme` overlay: theme list + live syntax-highlight preview.

use super::super::*;
use a3s_tui::components::{PreviewItem, PreviewPanel};

fn theme_panel_lines(selected: usize, width: usize) -> Vec<String> {
    let selected = selected.min(THEMES.len().saturating_sub(1));
    let items = THEMES
        .iter()
        .map(|theme| PreviewItem::new(theme.name))
        .collect::<Vec<_>>();
    let theme = &THEMES[selected];
    let sample = [
        "// syntax preview",
        "fn compute(n: usize) -> String {",
        "    let total = n * 42;",
        "    format!(\"sum: {}\", total)",
        "}",
    ];
    let preview = sample
        .into_iter()
        .map(|line| highlight_with(line, "rust", theme))
        .collect::<Vec<_>>();

    PreviewPanel::new("Theme")
        .subtitle("↑/↓ preview · Enter apply · Esc")
        .items(items)
        .selected(selected)
        .max_items(THEMES.len().max(1))
        .preview_title("preview")
        .preview_lines(preview)
        .indent(2)
        .marker("▸")
        .title_color(ACCENT)
        .subtitle_color(TN_GRAY)
        .text_color(TN_GRAY)
        .muted_color(TN_GRAY)
        .divider_color(TN_GRAY)
        .preview_color(TN_FG)
        .selected_colors(Color::BrightWhite, ACCENT)
        .view(width.min(u16::MAX as usize) as u16, THEMES.len() + 8)
        .lines()
        .map(str::to_string)
        .collect()
}

impl App {
    /// `/theme` picker: a theme list + a live syntax-highlight preview.
    pub(crate) fn overlay_theme(&self, composed: String) -> String {
        let Some(sel) = self.theme_panel else {
            return composed;
        };
        let width = self.width as usize;
        let menu = theme_panel_lines(sel, width);
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_lines_are_width_bounded_with_styles() {
        let lines = theme_panel_lines(THEMES.len() + 10, 24);

        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 24),
            "{:?}",
            lines
                .iter()
                .map(|line| a3s_tui::style::strip_ansi(line))
                .collect::<Vec<_>>()
        );
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>();
        assert!(plain.iter().any(|line| line.contains("Theme")), "{plain:?}");
        assert!(
            plain.iter().any(|line| line.contains("preview")),
            "{plain:?}"
        );
    }
}
