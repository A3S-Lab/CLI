//! `/effort` overlay: the effort slider (incl. the ultracode flourish).

use super::super::*;

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
        let n = EFFORT_LEVELS.len();
        // Fill (almost) the whole width.
        let track_w = width.saturating_sub(8).max(n * 9);
        let posf = |i: usize| {
            if n > 1 {
                i * (track_w - 1) / (n - 1)
            } else {
                0
            }
        };
        let pos = posf(sel);
        // Track with a ▲ at the selected level and a ┆ divider before ultracode.
        let mut track: Vec<char> = "─".repeat(track_w).chars().collect();
        let div = (posf(ULTRACODE - 1) + posf(ULTRACODE)) / 2;
        if div < track.len() {
            track[div] = '┆';
        }
        if pos < track.len() {
            track[pos] = '▲';
        }
        let track: String = track.iter().collect();
        // Level names centred under their tick with sparse brand accents.
        let level_colors = [
            TN_GRAY,
            TN_CYAN,
            ACCENT,
            TN_YELLOW,
            GRADIENT_SHIP_START,
            TN_PURPLE,
        ];
        let mut labels = String::new();
        let mut vis = 0usize;
        for (i, profile) in EFFORT_LEVELS.iter().enumerate() {
            let name = profile.label;
            let nw = name.chars().count();
            let start = posf(i).saturating_sub(nw / 2);
            while vis < start {
                labels.push(' ');
                vis += 1;
            }
            let c = level_colors[i.min(level_colors.len() - 1)];
            let st = if i == sel {
                Style::new().fg(c).bold()
            } else {
                Style::new().fg(c)
            };
            labels.push_str(&st.render(name));
            vis += nw;
        }
        let faster_smarter = format!("Faster{}Smarter", " ".repeat(track_w.saturating_sub(13)));
        let desc = if sel == ULTRACODE {
            "ultracode: plans, then fans independent work out to parallel subagents."
        } else {
            "higher effort = deeper reasoning + stricter verification + longer tool budget (slower)."
        };
        let dim = |s: &str| Style::new().fg(TN_GRAY).render(s);
        let menu = vec![
            pad_to(&Style::new().fg(ACCENT).bold().render("  Effort"), width),
            pad_to(&format!("    {}", dim(&faster_smarter)), width),
            pad_to(
                &format!("    {}", Style::new().fg(TN_FG).render(&track)),
                width,
            ),
            pad_to(&format!("    {labels}"), width),
            pad_to(
                &Style::new()
                    .fg(ACCENT)
                    .bold()
                    .render(&format!("    ▸ {}", EFFORT_LEVELS[sel].label)),
                width,
            ),
            pad_to(&format!("    {}", dim(desc)), width),
            pad_to(&dim("  ←/→ adjust · Enter confirm · Esc cancel"), width),
        ];
        self.overlay_list(composed, &menu)
    }
}
