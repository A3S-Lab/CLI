//! Full-width semantic transcript viewport used by the Codex-style `Ctrl+T` view.
//!
//! The caller owns semantic transcript composition. This component only keeps
//! the already-rendered ANSI content styled, wrapped, scrollable, and bounded to
//! the current terminal dimensions. It deliberately has no file-tree, editor
//! cursor, or line-number concepts.

use a3s_tui::components::{Viewport, ViewportMsg};
use a3s_tui::event::KeyEvent;
use a3s_tui::event::{MouseEvent, MouseEventKind};
use a3s_tui::{KeyCode, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TranscriptViewportAction {
    Ignored,
    Handled,
    CloseRequested,
}

pub(crate) struct SemanticTranscriptViewport {
    viewport: Viewport,
}

impl SemanticTranscriptViewport {
    pub(crate) fn new(content: &str, width: u16, height: u16) -> Self {
        let mut viewport = Viewport::new(width, height);
        viewport.set_content(content);
        Self { viewport }
    }

    /// Replace the semantic projection. A view at the bottom follows the latest
    /// content; a view that the user scrolled keeps its bounded row offset.
    pub(crate) fn set_content(&mut self, content: &str) {
        self.viewport.set_content(content);
    }

    pub(crate) fn resize(&mut self, width: u16, height: u16) {
        self.viewport.resize(width, height);
    }

    pub(crate) fn render(&self) -> String {
        self.viewport.view()
    }

    pub(crate) fn handle_key(&mut self, key: &KeyEvent) -> TranscriptViewportAction {
        let message = match key.code {
            KeyCode::Up | KeyCode::Char('k') if key.modifiers == KeyModifiers::NONE => {
                Some(ViewportMsg::ScrollUp(1))
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers == KeyModifiers::NONE => {
                Some(ViewportMsg::ScrollDown(1))
            }
            KeyCode::PageUp => Some(ViewportMsg::PageUp),
            KeyCode::PageDown => Some(ViewportMsg::PageDown),
            KeyCode::Home => Some(ViewportMsg::Top),
            KeyCode::End => Some(ViewportMsg::Bottom),
            KeyCode::Esc => return TranscriptViewportAction::CloseRequested,
            KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                return TranscriptViewportAction::CloseRequested;
            }
            KeyCode::Char(c)
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && c.eq_ignore_ascii_case(&'t') =>
            {
                return TranscriptViewportAction::CloseRequested;
            }
            _ => None,
        };

        let Some(message) = message else {
            return TranscriptViewportAction::Ignored;
        };
        self.viewport.update(message);
        self.viewport.set_auto_scroll(self.viewport.at_bottom());
        TranscriptViewportAction::Handled
    }

    pub(crate) fn handle_mouse(&mut self, mouse: &MouseEvent) -> TranscriptViewportAction {
        let message = match mouse.kind {
            MouseEventKind::ScrollUp => ViewportMsg::ScrollUp(3),
            MouseEventKind::ScrollDown => ViewportMsg::ScrollDown(3),
            _ => return TranscriptViewportAction::Ignored,
        };
        self.viewport.update(message);
        self.viewport.set_auto_scroll(self.viewport.at_bottom());
        TranscriptViewportAction::Handled
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> usize {
        self.viewport.scroll_offset()
    }

    #[cfg(test)]
    pub(crate) fn total_lines(&self) -> usize {
        self.viewport.total_lines()
    }

    #[cfg(test)]
    pub(crate) fn at_bottom(&self) -> bool {
        self.viewport.at_bottom()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3s_tui::style::{strip_ansi, visible_len, Color, Style};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn render_preserves_ansi_semantic_styles_without_editor_chrome() {
        let user = Style::new()
            .fg(Color::BrightWhite)
            .bg(Color::Rgb(31, 31, 31))
            .render("user request");
        let tool = Style::new().fg(Color::Green).render("• Ran cargo test");
        let content = format!("{user}\n{tool}");
        let view = SemanticTranscriptViewport::new(&content, 32, 2).render();

        assert!(view.contains("\x1b[97;48;2;31;31;31muser request\x1b[0m"));
        assert!(view.contains("\x1b[32m• Ran cargo test\x1b[0m"));
        assert_eq!(strip_ansi(&view), "user request\n• Ran cargo test");
        assert!(!strip_ansi(&view).contains("transcript.txt"));
        assert!(!strip_ansi(&view).contains("metadata"));
    }

    #[test]
    fn key_handling_scrolls_pages_and_bounds_home_end() {
        let content = (0..20)
            .map(|index| format!("row {index:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut view = SemanticTranscriptViewport::new(&content, 20, 4);

        assert_eq!(view.scroll_offset(), 16);
        assert!(view.at_bottom());
        assert_eq!(
            view.handle_key(&key(KeyCode::Home)),
            TranscriptViewportAction::Handled
        );
        assert_eq!(view.scroll_offset(), 0);
        assert!(!view.at_bottom());
        assert_eq!(
            view.handle_key(&key(KeyCode::PageDown)),
            TranscriptViewportAction::Handled
        );
        assert_eq!(view.scroll_offset(), 4);
        assert_eq!(
            view.handle_key(&key(KeyCode::End)),
            TranscriptViewportAction::Handled
        );
        assert_eq!(view.scroll_offset(), 16);
        assert!(view.at_bottom());

        assert_eq!(
            view.handle_key(&key(KeyCode::Esc)),
            TranscriptViewportAction::CloseRequested
        );
        assert_eq!(
            view.handle_key(&key(KeyCode::Char('x'))),
            TranscriptViewportAction::Ignored
        );
    }

    #[test]
    fn resize_rewraps_to_full_width_and_clamps_scroll_bounds() {
        let styled = Style::new()
            .fg(Color::Cyan)
            .render("abcdefghij klmnopqrst uvwxyz");
        let content = format!("{styled}\nsecond semantic row\nthird semantic row");
        let mut view = SemanticTranscriptViewport::new(&content, 14, 2);
        view.handle_key(&key(KeyCode::Home));

        view.resize(7, 3);
        let rendered = view.render();
        let rows = rendered.lines().collect::<Vec<_>>();

        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| visible_len(row) <= 7));
        assert!(rendered.contains("\x1b[36m"));
        assert_eq!(view.scroll_offset(), 0);
        assert!(view.total_lines() >= rows.len());

        view.handle_key(&key(KeyCode::End));
        assert!(view.at_bottom());
        assert_eq!(
            view.scroll_offset(),
            view.total_lines().saturating_sub(rows.len())
        );

        view.resize(40, 10);
        assert_eq!(view.scroll_offset(), 0);
        assert!(view.at_bottom());
        assert!(view.render().lines().all(|row| visible_len(row) <= 40));
    }

    #[test]
    fn content_refresh_follows_bottom_but_preserves_scrolled_offset() {
        let content = (0..8)
            .map(|index| format!("row {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut view = SemanticTranscriptViewport::new(&content, 20, 3);

        view.set_content(&format!("{content}\nrow 8"));
        assert!(view.at_bottom());
        assert_eq!(view.scroll_offset(), 6);

        view.handle_key(&key(KeyCode::PageUp));
        let anchored = view.scroll_offset();
        view.set_content(&format!("{content}\nrow 8\nrow 9"));
        assert_eq!(view.scroll_offset(), anchored);
        assert!(!view.at_bottom());
    }

    #[test]
    fn mouse_wheel_scrolls_without_leaking_to_the_hidden_chat() {
        use a3s_tui::event::MouseEvent;

        let content = (0..12)
            .map(|index| format!("row {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut view = SemanticTranscriptViewport::new(&content, 20, 3);
        let bottom = view.scroll_offset();

        let action = view.handle_mouse(&MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(action, TranscriptViewportAction::Handled);
        assert_eq!(view.scroll_offset(), bottom.saturating_sub(3));
        assert!(!view.at_bottom());
    }
}
