//! `/effort` overlay: the effort slider (incl. the ultracode flourish).

use super::super::*;
use a3s_tui::components::{LevelSlider, SliderLevel};

fn effort_slider_lines(selected: usize, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let level_colors = [
        TN_GRAY,
        TN_CYAN,
        ACCENT,
        TN_YELLOW,
        GRADIENT_SHIP_START,
        TN_PURPLE,
    ];
    let levels = EFFORT_LEVELS
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            let description = if index == ULTRACODE {
                "ultracode: plans, then fans independent work out to parallel subagents."
            } else {
                "higher effort = deeper reasoning + stricter verification + longer tool budget (slower)."
            };
            SliderLevel::new(profile.label)
                .description(description)
                .color(level_colors[index.min(level_colors.len() - 1)])
        })
        .collect::<Vec<_>>();

    LevelSlider::new(levels)
        .title("Effort")
        .range_labels("Faster", "Smarter")
        .selected(selected)
        .separator_after(ULTRACODE.saturating_sub(1))
        .margin(4)
        .marker('▲')
        .separator_char('┆')
        .pointer("▸")
        .title_color(ACCENT)
        .selected_color(ACCENT)
        .track_color(TN_FG)
        .muted_color(TN_GRAY)
        .hint("←/→ adjust · Enter confirm · Esc cancel")
        .view(width.min(u16::MAX as usize) as u16)
        .lines()
        .map(str::to_string)
        .collect()
}

impl App {
    pub(crate) fn overlay_effort(&self, composed: String) -> String {
        let Some(sel) = self.effort_panel else {
            return composed;
        };
        let width = self.width as usize;
        // Ultracode confirm flourish: a compact brand-gradient burst.
        if self.effort_anim.is_some() {
            let f = self.gradient_frame;
            let title = "⚡  U L T R A C O D E  ⚡";
            let colored: String = title
                .chars()
                .enumerate()
                .map(|(i, ch)| {
                    Style::new()
                        .fg(BRAND_GRADIENT[(i + f) % BRAND_GRADIENT.len()])
                        .bold()
                        .render(&ch.to_string())
                })
                .collect();
            let barw = width.saturating_sub(8).max(8);
            let wave: String = (0..barw)
                .map(|i| {
                    Style::new()
                        .fg(BRAND_GRADIENT[(i + f) % BRAND_GRADIENT.len()])
                        .bold()
                        .render("━")
                })
                .collect();
            let center = |s: &str, vis: usize| {
                let pad = width.saturating_sub(vis) / 2;
                format!("{}{s}", " ".repeat(pad))
            };
            let menu = vec![
                String::new(),
                format!("    {wave}"),
                String::new(),
                center(&colored, title.chars().count()),
                String::new(),
                center(
                    &Style::new()
                        .fg(TN_GRAY)
                        .render("planning a dynamic workflow · dispatching parallel subagents"),
                    61,
                ),
                String::new(),
                format!("    {wave}"),
            ];
            return self.overlay_list(composed, &menu);
        }
        let menu = effort_slider_lines(sel, width);
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_slider_lines_are_width_bounded() {
        let lines = effort_slider_lines(ULTRACODE, 30);

        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 30),
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
        assert!(plain.contains("Effort"), "{plain}");
        assert!(plain.contains("Faster"), "{plain}");
        assert!(plain.contains("Smarter"), "{plain}");
        assert!(plain.contains('▲'), "{plain}");
        assert!(plain.contains('┆'), "{plain}");
        assert!(plain.contains("▸ ultracode"), "{plain}");
    }

    #[test]
    fn effort_slider_clamps_selected_index() {
        let plain = effort_slider_lines(usize::MAX, 48)
            .into_iter()
            .map(|line| a3s_tui::style::strip_ansi(&line))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(plain.contains("▸ ultracode"), "{plain}");
    }
}
