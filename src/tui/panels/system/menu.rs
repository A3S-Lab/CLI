//! `/` slash command menu + the shared overlay-list primitive.

use super::super::*;
use a3s_tui::components::{MenuItem, MenuPanel};

fn slash_menu_lines(
    candidates: &[(String, String)],
    selected: usize,
    width: usize,
    max_items: usize,
) -> Vec<String> {
    let items = candidates
        .iter()
        .map(|(cmd, desc)| {
            MenuItem::new(cmd.clone())
                .description(desc.clone())
                .color(TN_GRAY)
        })
        .collect::<Vec<_>>();
    let panel = MenuPanel::without_title()
        .items(items)
        .selected(selected)
        .max_items(max_items)
        .label_width(11)
        .indent(0)
        .marker(" ")
        .text_color(TN_GRAY)
        .muted_color(TN_GRAY)
        .selected_colors(Color::BrightWhite, ACCENT);
    let height = candidates.len().min(max_items).saturating_add(1);
    panel
        .view(width as u16, height)
        .lines()
        .map(str::to_string)
        .collect()
}

fn slash_menu_submit_text(cmd: &str) -> String {
    if SLASH_COMMANDS
        .iter()
        .any(|(registered, _)| *registered == cmd)
    {
        cmd.to_string()
    } else {
        let name = cmd.trim_start_matches('/');
        format!("Use your `{name}` skill.")
    }
}

impl App {
    /// Built-in commands + loaded skills (as `/<skill>`) matching `input`.
    pub(crate) fn slash_candidates_all(&self, input: &str) -> Vec<(String, String)> {
        // Hide session-mutating commands while a turn is streaming.
        let idle = self.state == State::Idle;
        let mut out: Vec<(String, String)> = slash_candidates(input)
            .into_iter()
            .filter(|(c, _)| idle || !IDLE_ONLY.contains(c))
            .filter(|(c, _)| self.os_config.is_some() || !matches!(*c, "/login" | "/logout"))
            .map(|(c, d)| (c.to_string(), d.to_string()))
            .collect();
        for (name, desc) in &self.skills {
            if self.disabled_skills.contains(name) {
                continue; // hidden via /plugin
            }
            let cmd = format!("/{name}");
            if cmd.starts_with(input) {
                out.push((cmd, format!("skill · {}", truncate(desc, 56))));
            }
        }
        out
    }

    pub(crate) fn slash_menu_open(&self) -> bool {
        let input = self.textarea.value();
        // Available while idle OR streaming (so /btw can fire mid-turn); not
        // during an approval prompt.
        self.state != State::Awaiting
            && input.starts_with('/')
            && !input.contains('\n')
            // Close once args are being typed (e.g. "/btw <prompt>") so Enter
            // submits the whole line instead of just the command.
            && !input.contains(' ')
            && !self.slash_candidates_all(&input).is_empty()
    }

    /// Keys while the slash menu is open: ↑/↓ select, Enter run, Tab complete,
    /// Esc dismiss. Returns `Some(handled)` to consume the key.
    pub(crate) fn handle_slash_key(&mut self, key: &KeyEvent) -> Option<Option<Cmd<Msg>>> {
        let cands = self.slash_candidates_all(&self.textarea.value());
        if cands.is_empty() {
            return None;
        }
        let last = cands.len() - 1;
        self.slash_sel = self.slash_sel.min(last);
        match key.code {
            KeyCode::Up => {
                self.slash_sel = self.slash_sel.saturating_sub(1);
                Some(None)
            }
            KeyCode::Down => {
                self.slash_sel = (self.slash_sel + 1).min(last);
                Some(None)
            }
            KeyCode::Enter => {
                let cmd = cands[self.slash_sel].0.clone();
                self.slash_sel = 0;
                self.textarea.clear();
                // Run directly on this key event so the redraw is immediate (a
                // Submit message would be frame-throttled and look like a no-op).
                Some(self.on_submit(slash_menu_submit_text(&cmd)))
            }
            KeyCode::Tab => {
                // Fill into the input (trailing space closes the menu) to add args.
                self.textarea
                    .set_value(&format!("{} ", cands[self.slash_sel].0));
                self.slash_sel = 0;
                Some(None)
            }
            KeyCode::Esc => {
                self.textarea.clear();
                self.slash_sel = 0;
                Some(None)
            }
            _ => None,
        }
    }

    /// Overlay the `/` command menu just above the input box.
    /// Overlay `menu` rows just above the input box (last row on the activity line).
    pub(crate) fn overlay_list(&self, composed: String, menu: &[String]) -> String {
        if menu.is_empty() {
            return composed;
        }
        let mut rows: Vec<String> = composed.lines().map(str::to_string).collect();
        let bottom = (self.height as usize).saturating_sub(6);
        let start = bottom.saturating_sub(menu.len().saturating_sub(1));
        for (i, ml) in menu.iter().enumerate() {
            let row = start + i;
            if row < rows.len() {
                rows[row] = ml.clone();
            }
        }
        rows.join("\n")
    }

    pub(crate) fn overlay_slash_menu(&self, composed: String) -> String {
        if !self.slash_menu_open() {
            return composed;
        }
        let cands = self.slash_candidates_all(&self.textarea.value());
        let total = cands.len();
        let sel = self.slash_sel.min(total - 1);
        let width = self.width as usize;
        // Cap the menu height (skills make the list long) and scroll a window so
        // the selection stays visible — Claude-Code style.
        let max_rows = (self.height as usize).saturating_sub(8).clamp(3, 10);
        let menu = slash_menu_lines(&cands, sel, width, max_rows);
        self.overlay_list(composed, &menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_menu_lines_truncate_long_descriptions_to_width() {
        let rows = slash_menu_lines(
            &[(
                "/agent".to_string(),
                "pick an agent definition with an intentionally long explanation that must not overflow"
                    .to_string(),
            )],
            0,
            42,
            10,
        );
        let row = &rows[0];

        assert!(
            a3s_tui::style::visible_len(row) <= 42,
            "row should stay bounded: {:?}",
            a3s_tui::style::strip_ansi(row)
        );
        assert!(a3s_tui::style::strip_ansi(row).contains("/agent"));
    }

    #[test]
    fn all_registered_slash_menu_rows_fit_narrow_width() {
        let width = 48;
        let candidates = SLASH_COMMANDS
            .iter()
            .map(|(cmd, desc)| ((*cmd).to_string(), (*desc).to_string()))
            .collect::<Vec<_>>();
        let rows = slash_menu_lines(&candidates, 0, width, SLASH_COMMANDS.len());
        for row in rows {
            assert!(
                a3s_tui::style::visible_len(&row) <= width,
                "slash menu row should stay bounded: {:?}",
                a3s_tui::style::strip_ansi(&row)
            );
        }
    }

    #[test]
    fn slash_menu_enter_submits_builtins_to_the_main_handler() {
        assert_eq!(slash_menu_submit_text("/model"), "/model");
        assert_eq!(slash_menu_submit_text("/skill"), "/skill");
        assert_eq!(slash_menu_submit_text("/agent"), "/agent");
    }

    #[test]
    fn slash_menu_enter_turns_skills_into_skill_prompts() {
        assert_eq!(
            slash_menu_submit_text("/inspect-surface"),
            "Use your `inspect-surface` skill."
        );
    }
}
