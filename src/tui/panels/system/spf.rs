//! Superfile-style chrome shared by `/ide`, `/config`, and the `/kb` browser:
//! rounded-corner panels with the title embedded in the top border, focus
//! shown by border colour, plus small file-metadata helpers.
//!
//! Contract: content rows passed to [`frame`] must already be fitted to the
//! panel's inner width (plain-truncate BEFORE styling — `truncate` on a styled
//! string would split ANSI codes). `frame` only pads (ANSI-safe) and borders.

use super::super::*;
use a3s_tui::components::{Breadcrumb, PanelFrame};

/// Split the screen for the tree | editor layout. Returns
/// `(tree_panel_width, right_panel_width)` — both are TOTAL widths including
/// each panel's two border columns.
pub(crate) fn ide_split(w: usize) -> (usize, usize) {
    let tw = (w / 3).clamp(18, 40).min(w.saturating_sub(24).max(14));
    (tw, w.saturating_sub(tw))
}

/// Editor/preview text width inside the right panel: total minus the two
/// border columns and (when there's room) the `%4d ` line-number gutter.
/// The key handler's horizontal-scroll math and the renderer both use this —
/// single source. Honest on tiny terminals: never claims more room than the
/// panel actually has (a flattering floor here overflowed the frame).
pub(crate) fn ide_content_width(w: usize) -> usize {
    let iw = ide_split(w).1.saturating_sub(2);
    if ide_gutter_on(w) {
        iw - 5
    } else {
        iw
    }
}

/// Whether the editor renders the line-number gutter (dropped on tiny panels).
pub(crate) fn ide_gutter_on(w: usize) -> bool {
    ide_split(w).1.saturating_sub(2) >= 12
}

/// Fit PLAIN text to exactly `width` display columns: truncate, pop any
/// residue a width-inconsistent sequence leaves behind (a3s-tui's truncate
/// counts emoji+VS16 as 1 column but visible_len as 2 — e.g. 🖼️ in a file
/// name), then pad. Style AFTER fitting, never before.
pub(crate) fn fit(s: &str, width: usize) -> String {
    let mut t = truncate(s, width);
    while a3s_tui::style::visible_len(&t) > width {
        t.pop();
    }
    pad_to(&t, width)
}

/// One rounded-border panel: `╭─ title ───╮ │…│ ╰───╯`, `w`×`h` cells exactly.
/// Focus drives the border colour (accent vs gray), superfile-style.
pub(crate) fn frame(
    title: &str,
    w: usize,
    h: usize,
    focused: bool,
    mut rows: Vec<String>,
) -> Vec<String> {
    rows.truncate(h.saturating_sub(2));
    PanelFrame::new(title)
        .rows(rows)
        .focused(focused)
        .border_color(TN_GRAY)
        .focused_border_color(ACCENT)
        .title_color(TN_FG)
        .focused_title_color(ACCENT)
        .lines(w.min(u16::MAX as usize) as u16, h)
}

/// Join two panels side by side (rows are already exact-width).
pub(crate) fn hjoin(left: &[String], right: &[String]) -> Vec<String> {
    (0..left.len().max(right.len()))
        .map(|i| {
            format!(
                "{}{}",
                left.get(i).cloned().unwrap_or_default(),
                right.get(i).cloned().unwrap_or_default()
            )
        })
        .collect()
}

/// Terminal-stable monochrome file mark followed by Codex's hair-space pad.
/// Every returned value is exactly two display columns, independent of emoji
/// presentation rules and the user's font fallback.
pub(crate) fn file_icon(name: &str, is_dir: bool, expanded: bool) -> &'static str {
    if is_dir {
        return if expanded {
            "▾\u{200A}"
        } else {
            "▸\u{200A}"
        };
    }
    match name.rsplit('.').next().unwrap_or("") {
        "rs" => "◆\u{200A}",
        "py" | "pyi" => "λ\u{200A}",
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "go" | "c" | "h" | "cpp" | "hpp" | "java"
        | "rb" | "html" | "css" | "sql" => "◇\u{200A}",
        "md" | "mdx" | "txt" | "rst" => "≡\u{200A}",
        "acl" | "toml" | "yaml" | "yml" | "json" | "hcl" | "lock" => "⌘\u{200A}",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" => "▧\u{200A}",
        "sh" | "bash" | "zsh" | "fish" => "$\u{200A}",
        _ => "·\u{200A}",
    }
}

/// Human-readable byte size ("312 B", "4.2 KB", "1.8 MB").
pub(crate) fn human_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b < K {
        format!("{bytes} B")
    } else if b < K * K {
        format!("{:.1} KB", b / K)
    } else if b < K * K * K {
        format!("{:.1} MB", b / K / K)
    } else {
        format!("{:.1} GB", b / K / K / K)
    }
}

pub(crate) fn file_meta_breadcrumb_line(
    path: &std::path::Path,
    loaded_lines: Option<usize>,
    width: usize,
) -> String {
    if width == 0 {
        return String::new();
    }

    let rendered = Breadcrumb::new(file_meta_parts(path, loaded_lines))
        .separator(" · ")
        .active_color(TN_FG)
        .inactive_color(TN_GRAY)
        .separator_color(TN_GRAY)
        .view(width.min(u16::MAX as usize) as u16);
    a3s_tui::style::fit_visible(&format!(" {rendered}"), width)
}

fn file_meta_parts(path: &std::path::Path, loaded_lines: Option<usize>) -> Vec<String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut parts = vec![name];
    if let Ok(md) = std::fs::metadata(path) {
        if md.is_dir() {
            let n = std::fs::read_dir(path)
                .map(|rd| rd.take(1000).count())
                .unwrap_or(0);
            parts.push(format!("{n} items"));
        } else {
            parts.push(human_size(md.len()));
        }
        if let Ok(t) = md.modified() {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            parts.push(dt.format("%Y-%m-%d %H:%M").to_string());
        }
    }
    if let Some(n) = loaded_lines {
        parts.push(format!("{n} lines"));
    }
    parts
}

/// Delete `path` (file or dir), but only inside `root` — the guard that keeps
/// the `/kb` browser's delete from ever touching anything outside the vault.
pub(crate) fn delete_within(root: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
    let canon_root = root.canonicalize().map_err(|e| e.to_string())?;
    let canon = path.canonicalize().map_err(|e| e.to_string())?;
    if !canon.starts_with(&canon_root) || canon == canon_root {
        return Err("outside the KB vault".into());
    }
    if canon.is_dir() {
        std::fs::remove_dir_all(&canon).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(&canon).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_is_exactly_sized_with_rounded_corners() {
        let out = frame("title", 20, 5, true, vec!["hi".into()]);
        assert_eq!(out.len(), 5);
        for row in &out {
            assert_eq!(a3s_tui::style::visible_len(row), 20, "{row:?}");
        }
        let strip = |s: &str| {
            let mut o = String::new();
            let mut esc = false;
            for c in s.chars() {
                match (esc, c) {
                    (false, '\u{1b}') => esc = true,
                    (false, _) => o.push(c),
                    (true, 'm') => esc = false,
                    (true, _) => {}
                }
            }
            o
        };
        assert!(strip(&out[0]).starts_with("╭─ title"));
        assert!(strip(&out[0]).ends_with('╮'));
        assert!(strip(&out[4]).starts_with('╰') && strip(&out[4]).ends_with('╯'));
        assert!(strip(&out[1]).starts_with('│') && strip(&out[1]).ends_with('│'));
    }

    #[test]
    fn ide_content_width_tracks_split() {
        // Single source of truth: gutter (5) + right borders (2) accounted for.
        let (tw, right) = ide_split(120);
        assert_eq!(tw + right, 120);
        assert_eq!(ide_content_width(120), right - 7);
        // Tiny terminals: honest (≤ panel room), gutter dropped, never a lie
        // that overflows the frame.
        let iw20 = ide_split(20).1.saturating_sub(2);
        assert!(!ide_gutter_on(20));
        assert_eq!(ide_content_width(20), iw20);
    }

    #[test]
    fn icons_stay_within_truncate_budgets() {
        for icon in [
            "▾\u{200A}",
            "▸\u{200A}",
            "◆\u{200A}",
            "λ\u{200A}",
            "◇\u{200A}",
            "≡\u{200A}",
            "⌘\u{200A}",
            "▧\u{200A}",
            "$\u{200A}",
            "·\u{200A}",
        ] {
            assert_eq!(a3s_tui::style::visible_len(icon), 2, "{icon:?}");
            let row = format!("{icon} very-long-file-name-{}.ext", "x".repeat(40));
            let cut = truncate(&row, 20);
            assert!(
                a3s_tui::style::visible_len(&cut) <= 20,
                "{icon} breaks the column budget"
            );
        }
        assert_eq!(file_icon("src", true, false), "▸\u{200A}");
        assert_eq!(file_icon("src", true, true), "▾\u{200A}");
        assert_eq!(file_icon("lib.rs", false, false), "◆\u{200A}");
        assert_eq!(file_icon("README.md", false, false), "≡\u{200A}");
    }

    #[test]
    fn human_sizes_read_naturally() {
        assert_eq!(human_size(312), "312 B");
        assert_eq!(human_size(4300), "4.2 KB");
        assert_eq!(human_size(1_900_000), "1.8 MB");
    }

    #[test]
    fn file_meta_breadcrumb_line_uses_shared_breadcrumb_and_fits_width() {
        let root = std::env::temp_dir().join(format!("a3s-spf-meta-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("very-long-file-name-for-metadata.md");
        std::fs::write(&file, "one\ntwo\n").unwrap();

        let line = file_meta_breadcrumb_line(&file, Some(2), 96);
        let plain = a3s_tui::style::strip_ansi(&line);

        assert!(plain.contains("very-long"), "{plain}");
        assert!(plain.contains('·'), "{plain}");
        assert!(plain.contains("2 lines"), "{plain}");
        assert!(line.contains("\x1b["));
        assert!(a3s_tui::style::visible_len(&line) <= 96, "{}", plain);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delete_within_refuses_outside_and_root_itself() {
        let root = std::env::temp_dir().join(format!("a3s-spf-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/x.md"), "x").unwrap();
        let outside = std::env::temp_dir();
        assert!(delete_within(&root, &outside).is_err());
        assert!(
            delete_within(&root, &root).is_err(),
            "root itself is protected"
        );
        assert!(delete_within(&root, &root.join("sub/x.md")).is_ok());
        assert!(!root.join("sub/x.md").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
