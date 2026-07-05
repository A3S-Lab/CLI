//! `/btw` side-chat overlay: a background side-question + its answer.

use super::super::*;
use a3s_tui::components::SideNotePanel;

fn btw_panel_lines(question: &str, answer: Option<&str>, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let mut panel = SideNotePanel::new("↘ by the way · Esc to close")
        .question(question)
        .loading_text("thinking…")
        .max_body_lines(12)
        .indent(2)
        .title_color(TN_YELLOW)
        .question_color(TN_YELLOW)
        .answer_color(TN_YELLOW)
        .muted_color(TN_GRAY);
    if let Some(answer) = answer {
        panel = panel.answer(answer);
    }

    panel
        .view(width.min(u16::MAX as usize) as u16, usize::MAX)
        .lines()
        .map(str::to_string)
        .collect()
}

impl App {
    /// `/btw` side-chat panel above the input: the question and its answer.
    pub(crate) fn overlay_btw(&self, composed: String) -> String {
        let Some((q, a)) = &self.btw else {
            return composed;
        };
        let width = self.width as usize;
        let lines = btw_panel_lines(q, a.as_deref(), width);
        self.overlay_list(composed, &lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btw_panel_lines_are_width_bounded_with_styles() {
        let lines = btw_panel_lines(
            "Can this long side question stay inside the available width?",
            Some("Yes, the shared side-note panel wraps the compact answer safely."),
            24,
        );

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
            .collect::<Vec<_>>()
            .join("\n");
        assert!(plain.contains("by the way"), "{plain}");
        assert!(plain.contains("Q:"), "{plain}");
        assert!(plain.contains("shared"), "{plain}");
        assert!(plain.contains("side-note"), "{plain}");
        assert!(
            lines.iter().any(|line| line.contains("\x1b[")),
            "side note panel should carry styling"
        );
    }

    #[test]
    fn btw_panel_lines_use_loading_fallback() {
        let plain = btw_panel_lines("Still working?", None, 40)
            .into_iter()
            .map(|line| a3s_tui::style::strip_ansi(&line))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(plain.contains("Still working"), "{plain}");
        assert!(
            plain.contains("thinking"),
            "loading fallback should render: {plain}"
        );
    }
}
