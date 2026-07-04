//! `/plugin` overlay: enable/disable Claude skills.

use super::super::*;

fn plugin_row(name: &str, desc: &str, on: bool, selected: bool, width: usize) -> String {
    let marker = if selected { "▸" } else { " " };
    let check = if on {
        Style::new().fg(TN_GREEN).render("[✓]")
    } else {
        Style::new().fg(TN_GRAY).render("[ ]")
    };
    let name_width = width.saturating_sub(12).clamp(8, 16);
    let nm_plain = format!(
        "{:<width$}",
        truncate(&format!("/{name}"), name_width),
        width = name_width
    );
    let nm = if on {
        Style::new().fg(TN_CYAN).render(&nm_plain)
    } else {
        Style::new().fg(TN_GRAY).render(&nm_plain)
    };
    let prefix_width = 2 + 1 + 1 + 3 + 1 + name_width + 2;
    let descw = width.saturating_sub(prefix_width);
    let raw = format!(
        "  {marker} {check} {nm}  {}",
        Style::new().fg(TN_GRAY).render(&truncate(desc, descw)),
    );
    pad_to(&truncate(&raw, width), width)
}

fn plugin_panel_line(text: &str, width: usize) -> String {
    pad_to(&truncate(text, width), width)
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
        let on_count = total - self.disabled_skills.len().min(total);
        let mut menu = vec![plugin_panel_line(
            &Style::new().fg(ACCENT).bold().render(&format!(
                "  Plugins & skills ({on_count}/{total} on) — ↑/↓ · Space toggle · Esc"
            )),
            width,
        )];
        let max_rows = (self.height as usize).saturating_sub(8).clamp(3, 12);
        let start = if sel < max_rows {
            0
        } else {
            sel + 1 - max_rows
        };
        let end = (start + max_rows).min(total);
        for i in start..end {
            let (name, desc) = &self.skills[i];
            let on = !self.disabled_skills.contains(name);
            menu.push(plugin_row(name, desc, on, i == sel, width));
        }
        if total > max_rows {
            let up = if start > 0 { "↑" } else { " " };
            let down = if end < total { "↓" } else { " " };
            menu.push(plugin_panel_line(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("  {up}{down} {}/{total}", sel + 1)),
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
    fn plugin_rows_stay_bounded_for_long_skill_names_and_descriptions() {
        for width in [28, 44, 80] {
            let row = plugin_row(
                "very-long-skill-name-that-keeps-going",
                "a very long skill description that should never overflow the overlay width",
                true,
                true,
                width,
            );
            assert!(
                a3s_tui::style::visible_len(&row) <= width,
                "plugin row should fit width {width}: {:?}",
                a3s_tui::style::strip_ansi(&row)
            );
        }
    }

    #[test]
    fn plugin_panel_lines_stay_bounded() {
        let line = plugin_panel_line(
            &Style::new()
                .fg(ACCENT)
                .bold()
                .render("  Plugins & skills (120/120 on) — ↑/↓ · Space toggle · Esc"),
            32,
        );

        assert!(
            a3s_tui::style::visible_len(&line) <= 32,
            "plugin header should fit: {:?}",
            a3s_tui::style::strip_ansi(&line)
        );
    }
}
