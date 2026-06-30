//! `/memory` panel: a GitLens-style timeline of the agent's long-term memory.
//!
//! Left column is the timeline — memories newest-first, bucketed by day, each a
//! node tinted by its memory type. Right column is the selected memory's full
//! content + metadata (lazily read from its item file). Read-only.

use super::super::*;

/// Short badge + accent colour for a memory type.
fn mem_type_style(t: &str) -> (&'static str, Color) {
    match t {
        "semantic" => ("sem", TN_CYAN),
        "procedural" => ("proc", TN_GREEN),
        "working" => ("work", TN_GRAY),
        _ => ("epis", TN_YELLOW), // episodic / unknown
    }
}

/// Importance as a 5-cell bar, e.g. `▰▰▰▰▱`.
fn imp_bar(importance: f32) -> String {
    let filled = (importance.clamp(0.0, 1.0) * 5.0).round() as usize;
    format!("{}{}", "▰".repeat(filled), "▱".repeat(5 - filled))
}

impl App {
    /// Handle a key while the `/memory` panel is open.
    pub(crate) fn memory_key(&mut self, key: &KeyEvent) {
        if key.code == KeyCode::Esc {
            self.memory = None;
            return;
        }
        let body = (self.height as usize).saturating_sub(3);
        let Some(m) = self.memory.as_mut() else {
            return;
        };
        let last = m.entries.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                m.sel = m.sel.saturating_sub(1);
                m.refresh_detail();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                m.sel = (m.sel + 1).min(last);
                m.refresh_detail();
            }
            KeyCode::Char('g') => {
                m.sel = 0;
                m.refresh_detail();
            }
            KeyCode::Char('G') => {
                m.sel = last;
                m.refresh_detail();
            }
            // Page keys scroll the detail pane (long memories).
            KeyCode::PageUp => {
                m.detail_scroll = m
                    .detail_scroll
                    .saturating_sub(body.saturating_sub(1).max(1));
            }
            KeyCode::PageDown => m.detail_scroll += body.saturating_sub(1).max(1),
            // Reload from disk (new memories may have been recorded mid-session).
            KeyCode::Char('r') => {
                m.entries = memutil::load_timeline(&m.dir);
                m.sel = m.sel.min(m.entries.len().saturating_sub(1));
                m.note = format!("{} entries", m.entries.len());
                m.refresh_detail();
            }
            _ => {}
        }
    }

    /// Full-screen `/memory` panel: timeline (left) + selected detail (right).
    pub(crate) fn render_memory(&self, m: &MemPanel) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let now = chrono::Utc::now();

        let header = format!(
            "  memory · {} {}   {}",
            m.entries.len(),
            Style::new().fg(TN_GRAY).render("entries · ~/.a3s/memory"),
            Style::new().fg(TN_GRAY).render(&m.note),
        );
        let mut out = vec![
            pad_to(&header, width),
            pad_to(&Style::new().fg(TN_GRAY).render(&"─".repeat(width)), width),
        ];
        let body = h.saturating_sub(3);

        if m.entries.is_empty() {
            out.push(pad_to(
                &Style::new().fg(TN_GRAY).render(
                    "  no memories yet — the agent records them as you work (success/failure patterns, facts)",
                ),
                width,
            ));
            while out.len() + 1 < h {
                out.push(String::new());
            }
            out.push(pad_to(
                &Style::new().fg(TN_GRAY).render("  Esc close"),
                width,
            ));
            out.truncate(h);
            return out.join("\n");
        }

        let tw = (width / 3).clamp(26, 52);
        let sep = Style::new().fg(TN_GRAY).render(" │ ");
        let prefix = 15; // " ● " + time(4) + "  " + badge(4) + "  "

        // Left: timeline rows (day buckets + nodes); keep the selection visible.
        let rows = timeline_rows(&m.entries, now);
        let sel_row = rows
            .iter()
            .position(|r| matches!(r, TlRow::Node(i) if *i == m.sel))
            .unwrap_or(0);
        let start = sel_row.saturating_sub(body.saturating_sub(1));

        // Right: the selected memory's detail, scrollable.
        let right_lines = self.memory_detail_lines(m, now, width.saturating_sub(tw + 4));

        for i in 0..body {
            let left = match rows.get(start + i) {
                Some(TlRow::Day(label)) => {
                    let head = format!("  ── {label} ");
                    let bar = "─".repeat(tw.saturating_sub(a3s_tui::style::visible_len(&head)));
                    Style::new()
                        .fg(TN_GRAY)
                        .render(&pad_to(&format!("{head}{bar}"), tw))
                }
                Some(TlRow::Node(idx)) => {
                    let e = &m.entries[*idx];
                    let (badge, color) = mem_type_style(&e.memory_type);
                    let time = rel_time(e.timestamp, now);
                    let preview = e.content_lower.lines().next().unwrap_or("");
                    let preview = truncate(preview, tw.saturating_sub(prefix));
                    if *idx == m.sel {
                        let plain = format!(" ● {time:>4}  {badge:<4}  {preview}");
                        Style::new()
                            .fg(Color::Black)
                            .bg(color)
                            .render(&pad_to(&plain, tw))
                    } else {
                        // Truncate the plain preview first, then style segments, so
                        // we never cut an escape sequence mid-byte.
                        let rail = Style::new().fg(color).render(" ●");
                        let t = Style::new().fg(TN_GRAY).render(&format!(" {time:>4}"));
                        let b = Style::new().fg(color).render(&format!("  {badge:<4}"));
                        let pv = Style::new().fg(TN_FG).render(&format!("  {preview}"));
                        pad_to(&format!("{rail}{t}{b}{pv}"), tw)
                    }
                }
                None => " ".repeat(tw),
            };
            let right = right_lines
                .get(m.detail_scroll + i)
                .cloned()
                .unwrap_or_default();
            out.push(format!("{left}{sep}{right}"));
        }

        let hint =
            "  ↑↓/jk select · g/G top/bottom · PgUp/PgDn scroll detail · r refresh · Esc close";
        while out.len() + 1 < h {
            out.push(String::new());
        }
        out.push(pad_to(&Style::new().fg(TN_GRAY).render(hint), width));
        out.truncate(h);
        out.join("\n")
    }

    /// Build the right-pane lines for the selected memory: a metadata block, a
    /// rule, then the full original-case content (word-wrapped to `w`).
    fn memory_detail_lines(
        &self,
        m: &MemPanel,
        now: chrono::DateTime<chrono::Utc>,
        w: usize,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let Some(e) = m.entries.get(m.sel) else {
            return lines;
        };
        let (_, color) = mem_type_style(&e.memory_type);
        let ty = if e.memory_type.is_empty() {
            "memory"
        } else {
            &e.memory_type
        };
        lines.push(format!(
            "{}   {} {}",
            Style::new().fg(color).bold().render(&format!("● {ty}")),
            Style::new().fg(color).render(&imp_bar(e.importance)),
            Style::new().fg(TN_GRAY).render(&format!(
                "importance {:.2}  ·  {}",
                e.importance,
                rel_time(e.timestamp, now)
            )),
        ));
        if !e.tags.is_empty() {
            lines.push(
                Style::new()
                    .fg(TN_CYAN)
                    .render(&format!("tags: {}", e.tags.join(", "))),
            );
        }
        let created = e.timestamp.format("%Y-%m-%d %H:%M").to_string();
        let accessed = match m.detail.last_accessed {
            Some(la) => format!(" · last {}", la.format("%Y-%m-%d %H:%M")),
            None => String::new(),
        };
        lines.push(Style::new().fg(TN_GRAY).render(&format!(
            "created {created} · {}× accessed{accessed}",
            m.detail.access_count
        )));
        for (k, v) in &m.detail.metadata {
            let v = truncate(v, w.saturating_sub(k.len() + 2));
            lines.push(Style::new().fg(TN_GRAY).render(&format!("{k}: {v}")));
        }
        lines.push(Style::new().fg(TN_GRAY).render(&"─".repeat(w.min(48))));
        // Prefer the full original-case content; fall back to the index preview.
        let content = if m.detail.content.is_empty() {
            e.content_lower.as_str()
        } else {
            m.detail.content.as_str()
        };
        for raw in content.lines() {
            if raw.is_empty() {
                lines.push(String::new());
            } else {
                for wl in wrap_words(raw, w.max(8)) {
                    lines.push(Style::new().fg(TN_FG).render(&wl));
                }
            }
        }
        lines
    }
}
