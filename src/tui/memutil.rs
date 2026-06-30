//! `/memory` panel data: read the agent's long-term memory store
//! (`~/.a3s/memory`, an `a3s-memory` FileMemoryStore) for a GitLens-style
//! timeline. The store keeps an `index.json` (an array of lightweight entries)
//! plus one `items/{id}.json` per memory; we read the index for the list and
//! lazily read a single item for the detail pane — so opening the panel parses
//! one file, not hundreds.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::Path;

/// A memory as listed in the timeline (from the store's `index.json`).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MemEntry {
    pub id: String,
    /// Lowercased content — the index only stores this; used for the preview.
    #[serde(default)]
    pub content_lower: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub importance: f32,
    pub timestamp: DateTime<Utc>,
    /// `episodic` | `semantic` | `procedural` | `working`.
    #[serde(default)]
    pub memory_type: String,
}

/// A memory's full content + metadata (lazily read for the detail pane).
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct MemDetail {
    #[serde(default)]
    pub content: String,
    /// BTreeMap so the detail pane shows metadata in a stable order.
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub access_count: u32,
    #[serde(default)]
    pub last_accessed: Option<DateTime<Utc>>,
}

/// Read the store index, newest first. Empty if the store is absent/unreadable.
pub(crate) fn load_timeline(dir: &Path) -> Vec<MemEntry> {
    let mut v: Vec<MemEntry> = std::fs::read_to_string(dir.join("index.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    v.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
    v
}

/// Lazily read one memory's full item file for the detail pane.
pub(crate) fn load_detail(dir: &Path, id: &str) -> Option<MemDetail> {
    // ids are UUIDs; reject anything that could escape the items dir.
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return None;
    }
    let path = dir.join("items").join(format!("{id}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Compact "time since" for a node (`now`, `5m`, `3h`, `2d`, `4w`, `5mo`, `1y`).
pub(crate) fn rel_time(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - ts).num_seconds().max(0);
    match secs {
        0..=59 => "now".to_string(),
        60..=3_599 => format!("{}m", secs / 60),
        3_600..=86_399 => format!("{}h", secs / 3_600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        604_800..=2_591_999 => format!("{}w", secs / 604_800),
        2_592_000..=31_535_999 => format!("{}mo", secs / 2_592_000),
        _ => format!("{}y", secs / 31_536_000),
    }
}

/// Day-bucket header for the timeline (`Today` / `Yesterday` / `YYYY-MM-DD`).
/// Buckets by UTC date — a memory near midnight may land a day off in the
/// viewer's local zone, which is fine for a coarse timeline.
pub(crate) fn day_label(ts: DateTime<Utc>, now: DateTime<Utc>) -> String {
    match (now.date_naive() - ts.date_naive()).num_days() {
        d if d <= 0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        _ => ts.format("%Y-%m-%d").to_string(),
    }
}

/// One rendered timeline row: a day-bucket header, or a memory node (the index
/// of the entry in `MemPanel::entries`).
pub(crate) enum TlRow {
    Day(String),
    Node(usize),
}

/// Build the timeline rows — day headers interleaved with nodes, newest first.
pub(crate) fn timeline_rows(entries: &[MemEntry], now: DateTime<Utc>) -> Vec<TlRow> {
    let mut rows = Vec::with_capacity(entries.len() + 8);
    let mut last = String::new();
    for (i, e) in entries.iter().enumerate() {
        let d = day_label(e.timestamp, now);
        if d != last {
            rows.push(TlRow::Day(d.clone()));
            last = d;
        }
        rows.push(TlRow::Node(i));
    }
    rows
}

/// Panel state for `/memory`.
pub(crate) struct MemPanel {
    pub entries: Vec<MemEntry>,
    pub sel: usize,
    /// Full content + metadata of `entries[sel]`, lazily loaded on selection.
    pub detail: MemDetail,
    pub detail_scroll: usize,
    pub dir: std::path::PathBuf,
    pub note: String,
}

impl MemPanel {
    /// Reload the selected entry's full detail (called on open + selection move).
    pub fn refresh_detail(&mut self) {
        self.detail_scroll = 0;
        self.detail = self
            .entries
            .get(self.sel)
            .and_then(|e| load_detail(&self.dir, &e.id))
            .unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn rel_time_buckets() {
        let now = ts("2026-06-30T12:00:00Z");
        assert_eq!(rel_time(ts("2026-06-30T11:59:30Z"), now), "now");
        assert_eq!(rel_time(ts("2026-06-30T11:30:00Z"), now), "30m");
        assert_eq!(rel_time(ts("2026-06-30T09:00:00Z"), now), "3h");
        assert_eq!(rel_time(ts("2026-06-27T12:00:00Z"), now), "3d");
        assert_eq!(rel_time(ts("2026-06-10T12:00:00Z"), now), "2w");
    }

    #[test]
    fn day_labels() {
        let now = ts("2026-06-30T12:00:00Z");
        assert_eq!(day_label(ts("2026-06-30T01:00:00Z"), now), "Today");
        assert_eq!(day_label(ts("2026-06-29T23:00:00Z"), now), "Yesterday");
        assert_eq!(day_label(ts("2026-06-20T10:00:00Z"), now), "2026-06-20");
    }

    #[test]
    fn load_timeline_reads_index_newest_first() {
        let dir = std::env::temp_dir().join(format!("a3s-mem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let index = r#"[
            {"id":"a","content_lower":"older","tags":[],"importance":0.5,"timestamp":"2026-06-20T10:00:00Z","memory_type":"episodic"},
            {"id":"b","content_lower":"newer","tags":["x"],"importance":0.9,"timestamp":"2026-06-29T10:00:00Z","memory_type":"semantic"}
        ]"#;
        std::fs::write(dir.join("index.json"), index).unwrap();
        let tl = load_timeline(&dir);
        assert_eq!(tl.len(), 2);
        assert_eq!(tl[0].id, "b"); // newest first
        assert_eq!(tl[1].id, "a");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_detail_reads_item_and_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("a3s-memd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("items")).unwrap();
        std::fs::write(
            dir.join("items/abc.json"),
            r#"{"content":"Hello World","metadata":{"k":"v"},"access_count":3,"last_accessed":null}"#,
        )
        .unwrap();
        let d = load_detail(&dir, "abc").unwrap();
        assert_eq!(d.content, "Hello World"); // original case preserved
        assert_eq!(d.access_count, 3);
        assert_eq!(d.metadata.get("k").unwrap(), "v");
        assert!(load_detail(&dir, "../secret").is_none()); // traversal rejected
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timeline_rows_group_by_day() {
        let now = ts("2026-06-30T12:00:00Z");
        let entries = vec![
            mk("a", "2026-06-30T11:00:00Z"),
            mk("b", "2026-06-30T09:00:00Z"),
            mk("c", "2026-06-29T09:00:00Z"),
        ];
        let rows = timeline_rows(&entries, now);
        // Day(Today), Node(0), Node(1), Day(Yesterday), Node(2)
        assert!(matches!(&rows[0], TlRow::Day(d) if d == "Today"));
        assert!(matches!(rows[1], TlRow::Node(0)));
        assert!(matches!(rows[2], TlRow::Node(1)));
        assert!(matches!(&rows[3], TlRow::Day(d) if d == "Yesterday"));
        assert!(matches!(rows[4], TlRow::Node(2)));
    }

    fn mk(id: &str, ts_s: &str) -> MemEntry {
        MemEntry {
            id: id.into(),
            content_lower: String::new(),
            tags: vec![],
            importance: 0.5,
            timestamp: ts(ts_s),
            memory_type: "episodic".into(),
        }
    }
}
