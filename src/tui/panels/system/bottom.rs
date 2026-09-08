//! One immutable snapshot of the dynamic bottom-pane rows for a render pass.
//!
//! Plan, subagent, and queue rows used to be recomputed independently by the
//! viewport, cursor, and renderer. Keeping them in one projection makes row
//! ownership explicit and prevents a terminal event from leaving a one-frame
//! gap or stale footer row.

use super::super::*;

/// SessionChrome fixed rows above the composer: spacer only.
/// Dual rules, activity strip, and pinned plan/subagent/task rows are no longer
/// part of the default main-screen chrome budget.
pub(crate) const FIXED_ROWS_EXCLUDING_INPUT: u16 = 1;
/// Prompt footer below the composer (status + location), matching SessionChrome.
pub(crate) const FIXED_ROWS_BELOW_INPUT: u16 = 2;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct BottomPaneProjection {
    pub(crate) plan: Vec<String>,
    pub(crate) subagents: Vec<String>,
    pub(crate) tasks: Vec<String>,
}

impl BottomPaneProjection {
    pub(crate) fn dynamic_rows(&self) -> usize {
        self.plan
            .len()
            .saturating_add(self.subagents.len())
            .saturating_add(self.tasks.len())
    }

    pub(crate) fn rows_below_input(&self) -> usize {
        self.subagents.len().saturating_add(self.tasks.len())
    }

    pub(crate) fn input_cursor_row(
        &self,
        terminal_height: u16,
        input_height: u16,
        input_cursor_row: u16,
    ) -> u16 {
        let dynamic_below = self.rows_below_input().min(u16::MAX as usize) as u16;
        let below = FIXED_ROWS_BELOW_INPUT.saturating_add(dynamic_below);
        terminal_height
            .saturating_sub(below.saturating_add(input_height))
            .saturating_add(input_cursor_row)
    }
}

impl App {
    /// Main-screen chrome pins are suppressed (SessionChrome). Plan /
    /// subagent / queue surfaces stay available via slash panels and status.
    pub(crate) fn bottom_pane_projection(&self) -> BottomPaneProjection {
        BottomPaneProjection::default()
    }

    /// Rows between an overlay's bottom edge and the terminal bottom.
    ///
    /// The overlay replaces the transcript spacer; attachments, composer, and
    /// the prompt footer remain below it.
    pub(crate) fn overlay_rows_below(&self) -> usize {
        overlay_rows_below_for(
            self.input_height(),
            self.composer_staged_rows(),
            self.ephemeral_rows_above_composer(),
            self.bottom_pane_projection().dynamic_rows(),
        )
    }
}

fn overlay_rows_below_for(
    input_height: u16,
    attachment_rows: usize,
    ephemeral_rows: usize,
    dynamic_rows: usize,
) -> usize {
    // Overlay replaces spacer. Remaining chrome: attachments + working/followups
    // + composer bar + footer.
    usize::from(composer_chrome_height(input_height))
        .saturating_add(attachment_rows)
        .saturating_add(ephemeral_rows)
        .saturating_add(usize::from(FIXED_ROWS_BELOW_INPUT))
        .saturating_add(dynamic_rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_counts_each_owned_surface_once() {
        let projection = BottomPaneProjection {
            plan: vec!["plan".into(), "plan 2".into()],
            subagents: vec!["agents".into()],
            tasks: vec!["queue".into(), "queue 2".into()],
        };

        assert_eq!(projection.dynamic_rows(), 5);
        assert_eq!(projection.rows_below_input(), 3);
    }

    #[test]
    fn terminal_projection_has_no_phantom_dynamic_rows() {
        let projection = BottomPaneProjection::default();

        assert_eq!(projection.dynamic_rows(), 0);
        assert_eq!(projection.rows_below_input(), 0);
    }

    #[test]
    fn terminal_transition_restores_the_baseline_cursor_row() {
        let baseline = BottomPaneProjection::default();
        let active = BottomPaneProjection {
            subagents: vec!["agent 1".into(), "agent 2".into()],
            tasks: vec!["queued".into()],
            ..BottomPaneProjection::default()
        };

        assert_eq!(baseline.input_cursor_row(30, 1, 0), 27);
        assert_eq!(active.input_cursor_row(30, 1, 0), 24);
        assert_eq!(
            BottomPaneProjection::default().input_cursor_row(30, 1, 0),
            27
        );
    }

    #[test]
    fn overlay_rows_preserve_the_default_composer_position() {
        // body(1) + caps(2) + footer(2) = 5
        assert_eq!(overlay_rows_below_for(1, 0, 0, 0), 5);
    }

    #[test]
    fn overlay_rows_follow_multiline_input_and_dynamic_bottom_surfaces() {
        // body(3)+caps(2)+footer(2)=7; +dynamic 4 => 11; +attachments 2 => 13
        assert_eq!(overlay_rows_below_for(3, 0, 0, 0), 7);
        assert_eq!(overlay_rows_below_for(3, 0, 0, 4), 11);
        assert_eq!(overlay_rows_below_for(3, 2, 0, 4), 13);
        // ephemeral working(1)+followups(3) sits above composer
        assert_eq!(overlay_rows_below_for(1, 0, 4, 0), 9);
    }

    #[test]
    fn session_chrome_row_budget_is_spacer_above_and_footer_below() {
        assert_eq!(FIXED_ROWS_EXCLUDING_INPUT, 1);
        assert_eq!(FIXED_ROWS_BELOW_INPUT, 2);
        let pins = BottomPaneProjection::default();
        assert_eq!(pins.dynamic_rows(), 0);
        assert_eq!(pins.rows_below_input(), 0);
        assert_eq!(Mode::Default.name(), "agent");
        assert_eq!(
            BottomPaneProjection::default().input_cursor_row(30, 1, 0),
            27
        );
    }
}
