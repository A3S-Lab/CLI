//! `/btw` side-chat overlay: a background side-question + its answer.

use super::super::*;

fn btw_line(rendered: &str, width: usize) -> String {
    pad_to(&truncate(rendered, width), width)
}

impl App {
    /// `/btw` side-chat panel above the input: the question and its answer.
    pub(crate) fn overlay_btw(&self, composed: String) -> String {
        let Some((q, a)) = &self.btw else {
            return composed;
        };
        let width = self.width as usize;
        let cap = width.saturating_sub(4).max(8);
        let wrap = |s: &str| -> Vec<String> {
            s.lines()
                .flat_map(|l| {
                    let cs: Vec<char> = l.chars().collect();
                    if cs.is_empty() {
                        vec![String::new()]
                    } else {
                        cs.chunks(cap).map(|c| c.iter().collect()).collect()
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut lines = vec![btw_line(
            &Style::new()
                .fg(TN_YELLOW)
                .bold()
                .render("  ↘ by the way · Esc to close"),
            width,
        )];
        for l in wrap(&format!("Q: {q}")) {
            lines.push(btw_line(
                &Style::new().fg(TN_YELLOW).bold().render(&format!("  {l}")),
                width,
            ));
        }
        let ans = a.as_deref().unwrap_or("thinking…");
        for l in wrap(ans).into_iter().take(12) {
            lines.push(btw_line(
                &Style::new().fg(TN_YELLOW).render(&format!("  {l}")),
                width,
            ));
        }
        self.overlay_list(composed, &lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btw_lines_are_width_bounded_with_styles() {
        let line = btw_line(
            &Style::new()
                .fg(TN_YELLOW)
                .bold()
                .render("  by the way side answer with a very long heading"),
            24,
        );

        assert!(
            a3s_tui::style::visible_len(&line) <= 24,
            "{}",
            a3s_tui::style::strip_ansi(&line)
        );
    }
}
