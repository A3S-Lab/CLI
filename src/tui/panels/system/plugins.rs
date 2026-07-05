//! `/plugin` overlay: enable/disable Claude skills.

use super::super::*;
use a3s_tui::components::{MenuItem, MenuPanel};
use std::collections::HashSet;

fn plugin_panel_lines(
    skills: &[(String, String)],
    disabled_skills: &HashSet<String>,
    selected: usize,
    width: usize,
    height: usize,
) -> Vec<String> {
    let total = skills.len();
    if total == 0 {
        return Vec::new();
    }

    let selected = selected.min(total - 1);
    let on_count = total - disabled_skills.len().min(total);
    let max_items = height.saturating_sub(8).clamp(3, 12);
    let scroll = selected.saturating_add(1).saturating_sub(max_items);
    let label_width = width.saturating_sub(14).clamp(8, 18);
    let items = skills
        .iter()
        .map(|(name, desc)| {
            let on = !disabled_skills.contains(name);
            MenuItem::new(format!("/{name}"))
                .description(desc.clone())
                .checked(on)
                .color(if on { TN_CYAN } else { TN_GRAY })
        })
        .collect::<Vec<_>>();

    MenuPanel::new(format!(
        "Plugins & skills ({on_count}/{total} on) — ↑/↓ · Space toggle · Esc"
    ))
    .items(items)
    .selected(selected)
    .scroll(scroll)
    .max_items(max_items)
    .label_width(label_width)
    .show_scroll(total > max_items)
    .indent(2)
    .marker("▸")
    .title_color(ACCENT)
    .text_color(TN_GRAY)
    .muted_color(TN_GRAY)
    .checked_color(TN_GREEN)
    .selected_colors(Color::BrightWhite, ACCENT)
    .view(width as u16, max_items + 3)
    .lines()
    .map(str::to_string)
    .collect()
}

impl App {
    /// `/plugin` panel: enable/disable Claude skills (checkbox list, scrolled).
    pub(crate) fn overlay_plugins(&self, composed: String) -> String {
        let Some(sel) = self.plugins_panel else {
            return composed;
        };
        let total = self.skills.len();
        if total == 0 {
            return composed;
        }
        let sel = sel.min(total - 1);
        let width = self.width as usize;
        let menu = plugin_panel_lines(
            &self.skills,
            &self.disabled_skills,
            sel,
            width,
            self.height as usize,
        );
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_panel_uses_bounded_shared_menu_items() {
        let skills = vec![(
            "very-long-skill-name-that-keeps-going".to_string(),
            "a very long skill description that should never overflow the overlay width"
                .to_string(),
        )];
        let disabled = HashSet::new();
        for width in [28, 44, 80] {
            let lines = plugin_panel_lines(&skills, &disabled, 0, width, 24);
            assert!(
                lines
                    .iter()
                    .all(|line| a3s_tui::style::visible_len(line) <= width),
                "plugin panel should fit width {width}: {:?}",
                lines
                    .iter()
                    .map(|line| a3s_tui::style::strip_ansi(line))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn plugin_panel_scrolls_and_marks_disabled_skills() {
        let skills = (0..16)
            .map(|idx| (format!("skill-{idx}"), format!("description {idx}")))
            .collect::<Vec<_>>();
        let disabled = HashSet::from(["skill-14".to_string()]);
        let lines = plugin_panel_lines(&skills, &disabled, 14, 40, 18);
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>();

        assert!(
            plain.iter().any(|line| line.contains("[ ] /skill-14")),
            "{plain:?}"
        );
        assert!(
            plain.iter().any(|line| line.contains("↑↓ 15/16")),
            "{plain:?}"
        );
    }
}
