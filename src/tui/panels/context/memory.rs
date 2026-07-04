//! `/memory` panel: a graph view of the agent's durable memory.
//!
//! Left column is the event timeline, annotated with memory type, lifecycle
//! tier, and forgetting state. Right column shows the selected memory's
//! metadata, graph neighborhood, aliases, relations, and full content.

use super::super::*;

const MEMORY_PANEL_SESSION_LIMIT: usize = 1_000;

/// Short badge + accent colour for a memory type.
fn mem_type_style(t: &str) -> (&'static str, Color) {
    match t {
        "semantic" => ("sem", TN_CYAN),
        "procedural" => ("proc", TN_GREEN),
        "working" => ("work", TN_GRAY),
        _ => ("epis", TN_YELLOW), // episodic / unknown
    }
}

fn tier_style(tier: MemoryTier) -> Color {
    match tier {
        MemoryTier::Short => TN_GREEN,
        MemoryTier::Mid => TN_CYAN,
        MemoryTier::Long => TN_YELLOW,
    }
}

fn forget_mark(signal: ForgetSignal) -> &'static str {
    match signal {
        ForgetSignal::Keep => " ",
        ForgetSignal::Cooling => "~",
        ForgetSignal::Candidate => "!",
        ForgetSignal::Protected => "*",
    }
}

/// Importance as a 5-cell bar, e.g. `▰▰▰▰▱`.
fn imp_bar(importance: f32) -> String {
    let filled = (importance.clamp(0.0, 1.0) * 5.0).round() as usize;
    format!("{}{}", "▰".repeat(filled), "▱".repeat(5 - filled))
}

fn memory_line(rendered: &str, width: usize) -> String {
    pad_to(&truncate(rendered, width), width)
}

impl App {
    pub(crate) fn load_memory_panel(&self, dir: std::path::PathBuf) -> Cmd<Msg> {
        let memory = self.session.memory().cloned();
        cmd::cmd(
            move || async move { Msg::MemoryLoaded(load_memory_panel_data(dir, memory).await) },
        )
    }

    /// Handle a key while the `/memory` panel is open. Returns a `Cmd` only for
    /// the `c` back-jump (memory → its ctx source session).
    pub(crate) fn memory_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        if key.code == KeyCode::Esc {
            self.memory = None;
            return None;
        }
        // `c` — jump to the originating ctx session for a memory promoted from
        // history (`source=ctx`): pull `ctx show event <id>` into a read-only
        // viewer. The back-link that closes the ctx↔memory loop.
        if matches!(key.code, KeyCode::Char('c')) {
            if let Some(cmd) = self.memory_open_ctx_source() {
                return Some(cmd);
            }
            return None;
        }
        if matches!(key.code, KeyCode::Char('f')) {
            return self.memory_forget_candidate();
        }
        let session_memory = self.session.memory().cloned();
        let body = (self.height as usize).saturating_sub(3);
        let m = self.memory.as_mut()?;
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
            // Reload the durable snapshot; fall back to the live session store
            // when the file-backed view is unavailable.
            KeyCode::Char('r') => {
                let dir = m.dir.clone();
                m.note = "refreshing graph…".to_string();
                return Some(memory_panel_load_cmd(dir, session_memory));
            }
            _ => {}
        }
        None
    }

    /// Spawn `ctx show event <id>` for the selected memory's `ctx_event_id`
    /// provenance (nothing if it has none / ctx isn't installed).
    fn memory_open_ctx_source(&mut self) -> Option<Cmd<Msg>> {
        let m = self.memory.as_mut()?;
        let Some(event_id) = m.detail.metadata.get("ctx_event_id").cloned() else {
            m.note = "this memory has no ctx source (press c only on source=ctx)".to_string();
            return None;
        };
        if !self.ctx_ready {
            if let Some(m) = self.memory.as_mut() {
                m.note = "ctx is not installed — can't open the source session".to_string();
            }
            return None;
        }
        Some(cmd::cmd(move || async move {
            let out = tokio::process::Command::new("ctx")
                .args(["show", "event", &event_id, "--window", "8"])
                .output()
                .await;
            Msg::CtxMemorySource(match out {
                Ok(o) if o.status.success() => {
                    Ok((event_id, String::from_utf8_lossy(&o.stdout).into_owned()))
                }
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).into_owned()),
                Err(e) => Err(e.to_string()),
            })
        }))
    }

    /// Forget the selected memory, but only when the graph retention pass marks
    /// it as a low-value candidate. This keeps destructive cleanup explicit and
    /// auditable.
    fn memory_forget_candidate(&mut self) -> Option<Cmd<Msg>> {
        let session_memory = self.session.memory().cloned();
        let m = self.memory.as_mut()?;
        let Some(entry) = m.entries.get(m.sel).cloned() else {
            return None;
        };
        let Some(facet) = m.graph.by_memory.get(&entry.id) else {
            m.note = "graph data is still loading".to_string();
            return None;
        };
        if !facet.forget.is_candidate() {
            m.note = format!(
                "{} memory is {} · only forget candidates can be removed here",
                facet.tier.label(),
                facet.forget.label()
            );
            return None;
        }
        let id = entry.id.clone();
        let dir = m.dir.clone();
        let loaded_from_session = m.loaded_from_session;
        m.note = format!("forgetting candidate {id}…");
        Some(cmd::cmd(move || async move {
            let id_for_msg = id.clone();
            let result = async {
                delete_memory_item(&dir, session_memory.clone(), loaded_from_session, &id).await?;
                let data = load_memory_panel_data(dir, session_memory).await;
                Ok((id_for_msg, data))
            }
            .await;
            Msg::MemoryForgotten(result)
        }))
    }

    /// Full-screen `/memory` panel: timeline (left) + selected detail (right).
    pub(crate) fn render_memory(&self, m: &MemPanel) -> String {
        let width = self.width as usize;
        let h = self.height as usize;
        let now = chrono::Utc::now();

        let header = format!(
            "  memory graph · {} events · {} entities · {} aliases · {} relations · LLM {} · merged {} · conflicts {} · S/M/L {}/{}/{} · {} forget candidates   {}",
            m.graph.stats.events,
            m.graph.stats.entities,
            m.graph.stats.aliases,
            m.graph.stats.relations,
            m.graph.stats.llm_extracted,
            m.graph.stats.consolidated,
            m.graph.stats.conflicts,
            m.graph.stats.short,
            m.graph.stats.mid,
            m.graph.stats.long,
            m.graph.stats.forget_candidates,
            Style::new().fg(TN_GRAY).render(&m.note),
        );
        let mut out = vec![
            memory_line(&header, width),
            memory_line(&Style::new().fg(TN_GRAY).render(&"─".repeat(width)), width),
        ];
        let body = h.saturating_sub(3);

        if m.entries.is_empty() {
            out.push(memory_line(
                &Style::new().fg(TN_GRAY).render(
                    "  no memories yet — the agent records them as you work (success/failure patterns, facts)",
                ),
                width,
            ));
            while out.len() + 1 < h {
                out.push(String::new());
            }
            out.push(memory_line(
                &Style::new().fg(TN_GRAY).render("  Esc close"),
                width,
            ));
            out.truncate(h);
            return out.join("\n");
        }

        let tw = (width / 3).clamp(26, 52);
        let sep = Style::new().fg(TN_GRAY).render(" │ ");
        let prefix = 18; // " ● " + time(4) + " " + tier/forget + " " + badge(4) + "  "

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
                        .render(&memory_line(&format!("{head}{bar}"), tw))
                }
                Some(TlRow::Node(idx)) => {
                    let e = &m.entries[*idx];
                    let (badge, color) = mem_type_style(&e.memory_type);
                    let facet = m.graph.by_memory.get(&e.id);
                    let tier = facet.map(|f| f.tier).unwrap_or(MemoryTier::Short);
                    let forget = facet.map(|f| f.forget).unwrap_or(ForgetSignal::Keep);
                    let time = rel_time(e.timestamp, now);
                    let preview = e.content_lower.lines().next().unwrap_or("");
                    let preview = truncate(preview, tw.saturating_sub(prefix));
                    if *idx == m.sel {
                        let plain = format!(
                            " ● {time:>4} {}{} {badge:<4}  {preview}",
                            tier.badge(),
                            forget_mark(forget)
                        );
                        Style::new()
                            .fg(Color::Black)
                            .bg(color)
                            .render(&memory_line(&plain, tw))
                    } else {
                        // Truncate the plain preview first, then style segments, so
                        // we never cut an escape sequence mid-byte.
                        let rail = Style::new().fg(color).render(" ●");
                        let t = Style::new().fg(TN_GRAY).render(&format!(" {time:>4}"));
                        let tb = Style::new().fg(tier_style(tier)).render(&format!(
                            " {}{}",
                            tier.badge(),
                            forget_mark(forget)
                        ));
                        let b = Style::new().fg(color).render(&format!(" {badge:<4}"));
                        let pv = Style::new().fg(TN_FG).render(&format!("  {preview}"));
                        memory_line(&format!("{rail}{t}{tb}{b}{pv}"), tw)
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
            "  ↑↓/jk select · g/G top/bottom · PgUp/PgDn scroll · c ctx source · f forget candidate · r refresh · Esc close";
        while out.len() + 1 < h {
            out.push(String::new());
        }
        out.push(memory_line(&Style::new().fg(TN_GRAY).render(hint), width));
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
        if let Some(facet) = m.graph.by_memory.get(&e.id) {
            let lifecycle = lifecycle_labels(facet);
            if !lifecycle.is_empty() {
                let color = if facet.conflicts { TN_RED } else { TN_GREEN };
                lines.push(
                    Style::new()
                        .fg(color)
                        .render(&format!("lifecycle: {}", lifecycle.join(" · "))),
                );
            }
        }
        if let Some(event) = m.graph.event_for_memory(&e.id) {
            lines.push(Style::new().fg(TN_GRAY).render(&format!(
                "event: {} · {} · {} · {} entities",
                event.label,
                event.source,
                event.timestamp.format("%Y-%m-%d %H:%M"),
                event.entity_ids.len()
            )));
            lines.push(Style::new().fg(TN_GRAY).render(&format!(
                "event retention: {} · {:.2} · {}",
                event.tier.label(),
                event.retention_score,
                event.forget.label()
            )));
        }
        if let Some(facet) = m.graph.by_memory.get(&e.id) {
            lines.push(format!(
                "{} {}",
                Style::new()
                    .fg(tier_style(facet.tier))
                    .bold()
                    .render(&format!("tier: {}", facet.tier.label())),
                Style::new().fg(TN_GRAY).render(&format!(
                    "· retention {:.2} · {}",
                    facet.retention_score,
                    facet.forget.label()
                )),
            ));
            let entities = m.graph.entity_labels(&facet.entity_ids, 8);
            if !entities.is_empty() {
                lines.push(
                    Style::new()
                        .fg(TN_CYAN)
                        .render(&format!("entities: {}", entities.join(", "))),
                );
            }
            let aliases = m.graph.alias_labels(&facet.entity_ids, 6);
            if !aliases.is_empty() {
                lines.push(
                    Style::new()
                        .fg(TN_GRAY)
                        .render(&format!("aliases: {}", aliases.join(", "))),
                );
            }
            let relations = m.graph.relation_labels(&facet.relation_ids, 6);
            if !relations.is_empty() {
                lines.push(
                    Style::new()
                        .fg(TN_GRAY)
                        .render(&format!("relations: {}", relations.join(" · "))),
                );
            }
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
            .into_iter()
            .map(|line| memory_line(&line, w))
            .collect()
    }
}

fn lifecycle_labels(facet: &MemoryGraphFacet) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if facet.llm_extracted {
        labels.push("llm extracted");
    }
    if facet.consolidated {
        labels.push("consolidated");
    }
    if facet.conflicts {
        labels.push("conflict");
    }
    labels
}

fn memory_panel_load_cmd(
    dir: std::path::PathBuf,
    memory: Option<std::sync::Arc<a3s_code_core::memory::AgentMemory>>,
) -> Cmd<Msg> {
    cmd::cmd(move || async move { Msg::MemoryLoaded(load_memory_panel_data(dir, memory).await) })
}

async fn load_memory_panel_data(
    dir: std::path::PathBuf,
    memory: Option<std::sync::Arc<a3s_code_core::memory::AgentMemory>>,
) -> MemPanelData {
    let file_dir = dir.clone();
    let file_data = tokio::task::spawn_blocking(move || memutil::load_panel_data(&file_dir))
        .await
        .unwrap_or_default();
    if !file_data.entries.is_empty() {
        return file_data;
    }

    let Some(memory) = memory else {
        return file_data;
    };
    match memory.get_recent(MEMORY_PANEL_SESSION_LIMIT).await {
        Ok(items) if !items.is_empty() => memutil::panel_data_from_memory_items(items),
        Ok(_) => file_data,
        Err(_) => file_data,
    }
}

async fn delete_memory_item(
    dir: &std::path::Path,
    memory: Option<std::sync::Arc<a3s_code_core::memory::AgentMemory>>,
    prefer_session: bool,
    id: &str,
) -> Result<(), String> {
    if prefer_session {
        let Some(memory) = memory else {
            return Err("session memory is unavailable".to_string());
        };
        let store = memory.store().clone();
        return a3s_memory::MemoryStore::delete(store.as_ref(), id)
            .await
            .map_err(|e| e.to_string());
    }

    let store = a3s_memory::FileMemoryStore::new(dir)
        .await
        .map_err(|e| e.to_string())?;
    a3s_memory::MemoryStore::delete(&store, id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_lines_are_width_bounded_with_styles() {
        let line = memory_line(
            &Style::new().fg(TN_GRAY).render(
                "  ↑↓/jk select · g/G top/bottom · PgUp/PgDn scroll · c open ctx source · r refresh · Esc close",
            ),
            40,
        );

        assert!(
            a3s_tui::style::visible_len(&line) <= 40,
            "{}",
            a3s_tui::style::strip_ansi(&line)
        );
    }
}
