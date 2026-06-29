//! `/plugins` overlay: enable/disable Claude skills.

use super::super::*;

impl App {
    /// `/plugins` panel: enable/disable Claude skills (checkbox list, scrolled).
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
        let mut menu = vec![pad_to(
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
        let descw = width.saturating_sub(28);
        for i in start..end {
            let (name, desc) = &self.skills[i];
            let on = !self.disabled_skills.contains(name);
            let marker = if i == sel { "▸" } else { " " };
            let check = if on {
                Style::new().fg(TN_GREEN).render("[✓]")
            } else {
                Style::new().fg(TN_GRAY).render("[ ]")
            };
            let nm_plain = format!("{:<16}", truncate(&format!("/{name}"), 16));
            let nm = if on {
                Style::new().fg(TN_CYAN).render(&nm_plain)
            } else {
                Style::new().fg(TN_GRAY).render(&nm_plain)
            };
            let raw = format!(
                "  {marker} {check} {nm}  {}",
                Style::new().fg(TN_GRAY).render(&truncate(desc, descw)),
            );
            menu.push(pad_to(&raw, width));
        }
        if total > max_rows {
            let up = if start > 0 { "↑" } else { " " };
            let down = if end < total { "↓" } else { " " };
            menu.push(pad_to(
                &Style::new()
                    .fg(TN_GRAY)
                    .render(&format!("  {up}{down} {}/{total}", sel + 1)),
                width,
            ));
        }
        self.overlay_list(composed, &menu)
    }
}
