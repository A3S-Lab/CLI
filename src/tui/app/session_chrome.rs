//! SessionChrome: main-screen composition ownership.
//!
//! Layout ownership (default main screen), matching SessionChrome / PromptBar:
//! - Transcript — fill viewport (`viewport_view`)
//! - Spacer — one row (empty, or jump-to-latest when scrolled up)
//! - Attachments — optional image chips
//! - Composer — muted `→` / mode glyph + textarea
//! - Footer — quiet status + location under the prompt (prompt footer)
//!
//! Transient overlays (slash/file/model menus, approvals) stack on top after
//! chrome is composed. Full-screen panels (`/help`, `/ide`, …) bypass this
//! path entirely via `present_full_screen_page`.

use super::*;

/// Prompt glyph for the composer left gutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ComposerPromptGlyph {
    pub(super) symbol: &'static str,
    pub(super) color: Color,
    /// When true, tint the typed text with the glyph color (shell / research).
    pub(super) tint_input: bool,
    /// Agent arrow stays quiet (Cursor-style); shell / research keep a bold mode mark.
    pub(super) bold: bool,
}

impl ComposerPromptGlyph {
    /// Default agent PromptBar: muted right arrow, not a bold accent chevron.
    pub(super) fn agent() -> Self {
        Self {
            symbol: "→",
            color: COMPOSER_CHROME.faint,
            tint_input: false,
            bold: false,
        }
    }
}

/// Resolve the composer prompt glyph from exclusive input modes.
pub(super) fn composer_prompt_glyph(shell_mode: bool, research_mode: bool) -> ComposerPromptGlyph {
    if shell_mode {
        return ComposerPromptGlyph {
            symbol: "!",
            color: TN_RED,
            tint_input: true,
            bold: true,
        };
    }
    if research_mode {
        return ComposerPromptGlyph {
            symbol: "?",
            color: TN_CYAN,
            tint_input: true,
            bold: true,
        };
    }
    ComposerPromptGlyph::agent()
}

impl App {
    /// Compose the SessionChrome main screen, then apply transient overlays.
    pub(super) fn render_session_chrome_main(&self) -> String {
        let width = self.width as usize;
        let composer_width = self.viewport_content_width();
        let raw_view = self.viewport.view();
        let shown = match &self.selection {
            Some(s) if !s.is_empty() => {
                let (r1, c1, r2, c2) = s.ordered();
                highlight_selection(&raw_view, r1, c1, r2, c2)
            }
            _ => raw_view,
        };
        let viewport_view = append_scrollbar(
            &shown,
            width,
            self.viewport.total_lines(),
            self.viewport.scroll_percent(),
        );

        let glyph = composer_prompt_glyph(self.shell_mode, self.research_mode);
        let typed = self.textarea.view();
        let body_height = self.input_height();
        let mut input_view = composer_prompt_bar(
            glyph.symbol,
            glyph.color,
            &typed,
            glyph.tint_input,
            glyph.bold,
            composer_width,
        );
        // Pad only the middle body when textarea height exceeds rendered body.
        let expected_rows = composer_chrome_height(body_height) as usize;
        let mut rows: Vec<String> = if input_view.is_empty() {
            Vec::new()
        } else {
            input_view.lines().map(str::to_string).collect()
        };
        if rows.len() >= 2 {
            let mut body: Vec<String> = rows[1..rows.len().saturating_sub(1)].to_vec();
            while body.len() < body_height as usize {
                let inset = COMPOSER_INSET.min(composer_width.saturating_sub(1) / 2);
                let inner = composer_width
                    .saturating_sub(inset.saturating_mul(2))
                    .max(1);
                let side = if inset == 0 {
                    String::new()
                } else {
                    Style::new().bg(CANVAS).render(&" ".repeat(inset))
                };
                body.push(format!(
                    "{side}{}{side}",
                    Style::new().bg(SURFACE_COMPOSER).render(&" ".repeat(inner)),
                ));
            }
            let top = rows.first().cloned().unwrap_or_default();
            let bottom = rows.last().cloned().unwrap_or_default();
            rows = std::iter::once(top)
                .chain(body)
                .chain(std::iter::once(bottom))
                .collect();
        }
        while rows.len() < expected_rows {
            let inset = COMPOSER_INSET.min(composer_width.saturating_sub(1) / 2);
            let inner = composer_width
                .saturating_sub(inset.saturating_mul(2))
                .max(1);
            let side = if inset == 0 {
                String::new()
            } else {
                Style::new().bg(CANVAS).render(&" ".repeat(inset))
            };
            rows.push(format!(
                "{side}{}{side}",
                Style::new().bg(SURFACE_COMPOSER).render(&" ".repeat(inner)),
            ));
        }
        if expected_rows > 0 {
            input_view = rows.join("\n");
        }
        let chrome_height = composer_chrome_height(body_height);
        let pastes = paste_strip(&self.pending_pastes, composer_width);
        let paste_view = pastes.rows.join("\n");
        let attachments = attachment_strip(&self.pending_images, composer_width);
        let attachment_view = attachments.rows.join("\n");
        let working_label = self.ephemeral_working_label();
        let working_view = working_label
            .as_deref()
            .map(|label| render_working_line(label, self.blink_tick as usize, width))
            .unwrap_or_default();
        let working_rows = u16::from(!working_view.is_empty());
        let follow_up_rows_data = self.follow_up_strip_rows();
        let follow_up_view = render_follow_up_strip(&follow_up_rows_data, width);
        let follow_up_rows = follow_up_strip_row_count(follow_up_rows_data.len());
        let status = self.session_status_line(width);
        let footer_rows = status.lines().count().max(1).min(u16::MAX as usize) as u16;
        let spacer = if self.viewport.at_bottom() {
            String::new()
        } else {
            jump_to_latest_hint(width)
        };

        let composed = Layout::vertical()
            .item(&viewport_view, Constraint::Fill)
            .item(&spacer, Constraint::Fixed(1))
            .item(
                &paste_view,
                Constraint::Fixed(pastes.rows.len().min(u16::MAX as usize) as u16),
            )
            .item(
                &attachment_view,
                Constraint::Fixed(attachments.rows.len().min(u16::MAX as usize) as u16),
            )
            .item(&working_view, Constraint::Fixed(working_rows))
            .item(&follow_up_view, Constraint::Fixed(follow_up_rows))
            .item(&input_view, Constraint::Fixed(chrome_height))
            .item(&status, Constraint::Fixed(footer_rows))
            .render(self.height);

        let composed = paint_canvas_rows(&composed, width);

        self.apply_session_overlays(composed)
    }

    /// Transient menus and decision modals stacked above SessionChrome.
    pub(super) fn apply_session_overlays(&self, composed: String) -> String {
        let composed = self.overlay_slash_menu(composed);
        let composed = self.overlay_file_menu(composed);
        let composed = self.overlay_model_menu(composed);
        let composed = self.overlay_relay_menu(composed);
        let composed = self.overlay_task_menu(composed);
        let composed = self.overlay_permission_menu(composed);
        let composed = self.overlay_history_menu(composed);
        let composed = self.overlay_diff_review(composed);
        let composed = self.overlay_review_menu(composed);
        let composed = self.overlay_effort(composed);
        let composed = self.overlay_theme(composed);
        let composed = self.overlay_plugins(composed);
        let composed = self.overlay_packages(composed);
        self.overlay_decision_modals(composed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_prompt_glyph_defaults_to_muted_arrow() {
        let glyph = composer_prompt_glyph(false, false);
        assert_eq!(glyph, ComposerPromptGlyph::agent());
        assert_eq!(glyph.symbol, "→");
        assert_eq!(glyph.color, COMPOSER_CHROME.faint);
        assert!(!glyph.tint_input);
        assert!(!glyph.bold);
    }

    #[test]
    fn composer_prompt_glyph_modes_are_exclusive_priority() {
        assert_eq!(composer_prompt_glyph(true, true).symbol, "!");
        assert!(composer_prompt_glyph(true, false).bold);
        assert_eq!(composer_prompt_glyph(false, true).symbol, "?");
        assert!(composer_prompt_glyph(false, true).bold);
        assert_eq!(composer_prompt_glyph(false, false).symbol, "→");
        assert!(!composer_prompt_glyph(false, false).bold);
    }

    #[test]
    fn session_chrome_layout_contract_matches_row_budget() {
        assert_eq!(panels::bottom::FIXED_ROWS_EXCLUDING_INPUT, 1);
        assert_eq!(panels::bottom::FIXED_ROWS_BELOW_INPUT, 2);
        assert_eq!(
            panels::bottom::BottomPaneProjection::default().dynamic_rows(),
            0
        );
    }
}
