//! `/effort` overlay: the effort slider (incl. the ultracode flourish).

use super::super::*;
use a3s_tui::components::{LevelSlider, LevelSliderMsg, ShimmerText, SliderLevel};
use a3s_tui::event::MouseEvent;

const EFFORT_OVERLAY_ROWS_BELOW: usize = 5;

fn effort_slider(selected: usize) -> LevelSlider {
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
        .hint("←/→ or wheel/click adjust · Enter confirm · Esc cancel")
}

fn effort_slider_lines(selected: usize, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    effort_slider(selected)
        .view(width.min(u16::MAX as usize) as u16)
        .lines()
        .map(str::to_string)
        .collect()
}

fn effort_overlay_y_offset(screen_height: usize, row_count: usize) -> u16 {
    screen_height
        .saturating_sub(EFFORT_OVERLAY_ROWS_BELOW)
        .saturating_sub(row_count)
        .min(u16::MAX as usize) as u16
}

fn ultracode_animation_lines(frame: usize, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }

    let wave_width = width.saturating_sub(8).max(1).min(width);
    let wave: String = (0..wave_width)
        .map(|i| {
            Style::new()
                .fg(BRAND_GRADIENT[(i + frame) % BRAND_GRADIENT.len()])
                .bold()
                .render("━")
        })
        .collect();
    let title = ShimmerText::new("⚡  U L T R A C O D E  ⚡")
        .phase(frame)
        .colors(GRADIENT_SHIP_START, TN_FG)
        .spread(4.0)
        .speed_divisor(1)
        .cycle_gap(6)
        .view();
    let status = Style::new()
        .fg(TN_GRAY)
        .render("planning a dynamic workflow · dispatching parallel subagents");

    vec![
        String::new(),
        center_visible_line(&wave, width),
        String::new(),
        center_visible_line(&title, width),
        String::new(),
        center_visible_line(&status, width),
        String::new(),
        center_visible_line(&wave, width),
    ]
}

fn center_visible_line(rendered: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let visible = a3s_tui::style::visible_len(rendered);
    if visible >= width {
        return a3s_tui::style::fit_visible(rendered, width);
    }
    let pad = (width - visible) / 2;
    a3s_tui::style::fit_visible(&format!("{}{rendered}", " ".repeat(pad)), width)
}

impl App {
    pub(crate) fn confirm_effort_selection(&mut self, selected: usize) -> Option<Cmd<Msg>> {
        let selected = selected.min(EFFORT_LEVELS.len().saturating_sub(1));
        if selected == ULTRACODE {
            // Play a flourish in the panel, then close + apply
            // (handled on the banner tick).
            self.effort_panel = Some(selected);
            self.effort_anim = Some(Instant::now());
            self.gradient_frame = 0;
            None
        } else {
            self.effort_panel = None;
            self.apply_effort(selected)
        }
    }

    pub(crate) fn handle_effort_mouse(&mut self, mouse: &MouseEvent) {
        let Some(selected) = self.effort_panel else {
            return;
        };
        if self.effort_anim.is_some() {
            return;
        }
        let width = (self.width as usize).min(u16::MAX as usize);
        if width == 0 {
            return;
        }
        let mut slider = effort_slider(selected);
        let row_count = slider.view(width as u16).lines().count();
        if row_count == 0 {
            return;
        }
        let y_offset = effort_overlay_y_offset(self.height as usize, row_count);
        let row = mouse.row as usize;
        let start = y_offset as usize;
        if row < start || row >= start.saturating_add(row_count) {
            return;
        }
        slider.set_y_offset(y_offset);
        let before = slider.selected_value();

        match slider.handle_mouse(mouse, width as u16) {
            Some(LevelSliderMsg::Selected(index)) => {
                self.effort_panel = Some(index.min(EFFORT_LEVELS.len().saturating_sub(1)));
            }
            None => {
                let after = slider
                    .selected_value()
                    .min(EFFORT_LEVELS.len().saturating_sub(1));
                if after != before {
                    self.effort_panel = Some(after);
                }
            }
        }
    }

    pub(crate) fn overlay_effort(&self, composed: String) -> String {
        let Some(sel) = self.effort_panel else {
            return composed;
        };
        let width = self.width as usize;
        // Ultracode confirm flourish: a compact brand-gradient burst.
        if self.effort_anim.is_some() {
            let menu = ultracode_animation_lines(self.gradient_frame, width);
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

    #[test]
    fn effort_slider_mouse_wheel_moves_selection() {
        use a3s_tui::event::MouseEventKind;

        let width = 48;
        let row_count = effort_slider_lines(0, width).len();
        let y_offset = effort_overlay_y_offset(24, row_count);
        let mut slider = effort_slider(0);
        slider.set_y_offset(y_offset);

        let msg = slider.handle_mouse(
            &MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: y_offset + 2,
                modifiers: a3s_tui::KeyModifiers::NONE,
            },
            width as u16,
        );

        assert_eq!(msg, None);
        assert_eq!(
            slider.selected_value(),
            1.min(EFFORT_LEVELS.len().saturating_sub(1))
        );
    }

    #[test]
    fn effort_slider_click_selects_level_at_overlay_offset() {
        use a3s_tui::event::{MouseButton, MouseEventKind};

        let width = 48;
        let row_count = effort_slider_lines(0, width).len();
        let y_offset = effort_overlay_y_offset(24, row_count);
        let mut slider = effort_slider(0);
        slider.set_y_offset(y_offset);

        let msg = slider.handle_mouse(
            &MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 21,
                row: y_offset + 2,
                modifiers: a3s_tui::KeyModifiers::NONE,
            },
            width as u16,
        );

        assert_eq!(msg, Some(LevelSliderMsg::Selected(2)));
        assert_eq!(slider.selected_value(), 2);
    }

    #[test]
    fn ultracode_animation_uses_shared_shimmer_and_fits_width() {
        let lines = ultracode_animation_lines(3, 32);
        let plain = lines
            .iter()
            .map(|line| a3s_tui::style::strip_ansi(line))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(lines.len(), 8);
        assert!(plain.contains("U L T R A C O D E"), "{plain}");
        assert!(plain.contains("planning a dynamic"), "{plain}");
        assert!(lines.iter().any(|line| line.contains("\x1b[")));
        assert!(
            lines
                .iter()
                .all(|line| a3s_tui::style::visible_len(line) <= 32),
            "{plain}"
        );
    }
}
