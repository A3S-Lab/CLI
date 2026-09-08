//! First-run welcome banner: static A3S logo ASCII, wordmark, and session details.

use super::super::*;
use a3s_tui::components::WelcomeBanner;

impl App {
    /// First-run welcome: product identity, version, model, and tips.
    pub(crate) fn banner(&self) -> String {
        let model = self.model.as_deref().unwrap_or("no model configured");
        let skills = if self.skill_count > 0 {
            format!("  ·  {} skills", self.skill_count)
        } else {
            String::new()
        };
        let os = match &self.os_session {
            Some(s) => format!("  ·  OS: {}", s.display_label()),
            None => String::new(),
        };
        banner_view(
            model,
            &skills,
            &os,
            &self.cwd,
            self.update_available.as_deref(),
            self.viewport_content_width().min(u16::MAX as usize) as u16,
        )
    }
}

fn banner_view(
    model: &str,
    skills: &str,
    os: &str,
    cwd: &str,
    update_available: Option<&str>,
    width: u16,
) -> String {
    let metadata = format!(
        "a3s-code v{}  ·  {model}{skills}{os}  ·  {cwd}",
        env!("CARGO_PKG_VERSION")
    );
    let mut banner = WelcomeBanner::new()
        .mascot_lines(banner_mascot())
        .art_lines(banner_wordmark())
        .art_offset(1)
        .margin(PAD)
        .gap(2)
        .mascot_color(ACCENT_BRIGHT)
        .art_color(ACCENT)
        .metadata_color(TN_GRAY)
        .tip_color(TN_GRAY)
        .notice_color(ACCENT)
        .metadata(metadata)
        .tip("Type a message · / for commands · Shift+Tab cycles agent/plan/reviewer/auto/yolo · /ask = plan (read-only) · Ctrl+G reviews diffs · Ctrl+C twice to exit");
    if let Some(v) = update_available {
        banner = banner.notice(format!(
            "⬆ a3s {v} is available (you have {}) — type /update to upgrade",
            env!("CARGO_PKG_VERSION")
        ));
    }

    let rendered = format!("\n{}\n", banner.view(width, usize::MAX));
    rendered
}

/// Static ASCII of apps/desktop `a3s-os-logo.png`: open ring, centered Λ, wave.
fn banner_mascot() -> Vec<String> {
    [
        r"      .------.     ",
        r"    .'        '.   ",
        r"   /    /\    \    ",
        r"  |    /  \        ",
        r"   \~~~~/ ~~ \~~~~  ",
        r"    '.        .'   ",
        r"      '------'     ",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn banner_wordmark() -> Vec<&'static str> {
    vec![
        r" █████╗ ██████╗ ███████╗     ██████╗ ██████╗ ██████╗ ███████╗",
        r"██╔══██╗╚════██╗██╔════╝    ██╔════╝██╔═══██╗██╔══██╗██╔════╝",
        r"███████║ █████╔╝███████╗    ██║     ██║   ██║██║  ██║█████╗",
        r"██╔══██║ ╚═══██╗╚════██║    ██║     ██║   ██║██║  ██║██╔══╝",
        r"██║  ██║██████╔╝███████║    ╚██████╗╚██████╔╝██████╔╝███████╗",
        r"╚═╝  ╚═╝╚═════╝ ╚══════╝     ╚═════╝ ╚═════╝ ╚═════╝ ╚══════╝",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_banner_uses_shared_component_and_fits_width() {
        let terminal_width = 52;
        let viewport_width = terminal_width;
        let rendered = banner_view(
            "gpt-5",
            "  ·  4 skills",
            "  ·  OS: dev@example",
            "/Users/roylin/code/a3s",
            Some("0.9.0"),
            viewport_width,
        );
        let plain = a3s_tui::style::strip_ansi(&rendered);

        assert!(plain.contains("a3s-code v"), "{plain}");
        assert!(
            plain.contains(r"/    /\    \") || plain.contains("████"),
            "{plain}"
        );
        assert!(plain.contains(r"\~~~~/ ~~ \~~~~"), "{plain}");
        assert!(
            !plain.contains(".-^-."),
            "old guard mascot must be gone: {plain}"
        );
        assert!(plain.contains("Type a message"), "{plain}");
        assert!(plain.contains("0.9.0"), "{plain}");
        assert!(rendered.contains("\x1b["), "banner should carry styling");
        for line in rendered.lines().filter(|line| !line.is_empty()) {
            assert!(
                a3s_tui::style::visible_len(line) <= viewport_width as usize,
                "{:?}",
                a3s_tui::style::strip_ansi(line)
            );
        }

        let wide = a3s_tui::style::strip_ansi(&banner_view(
            "gpt-5",
            "  ·  4 skills",
            "  ·  OS: dev@example",
            "/Users/roylin/code/a3s",
            Some("0.9.0"),
            120,
        ));
        assert!(
            wide.contains("agent/plan/reviewer/auto/yolo"),
            "welcome tip must include reviewer in the Shift+Tab ring: {wide}"
        );
    }

    #[test]
    fn welcome_identity_keeps_static_logo_ascii_and_wordmark() {
        let first = banner_view("gpt-5", "", "", "/workspace", None, 120);
        let later = banner_view("gpt-5", "", "", "/workspace", None, 120);
        let plain = a3s_tui::style::strip_ansi(&first);

        assert!(plain.contains(r"/    /\    \"), "{plain}");
        assert!(plain.contains(r"\~~~~/ ~~ \~~~~"), "{plain}");
        assert!(plain.contains("████"), "{plain}");
        assert!(!plain.contains(".-^-."), "{plain}");
        assert_eq!(first, later, "static logo must not animate across frames");
    }

    #[test]
    fn empty_transcript_rebuild_must_keep_welcome_logo_contract() {
        // Regression contract for `App::rebuild_viewport*`: when `messages` is
        // empty, viewport refresh must re-paint this banner (not blank
        // transcript padding). Deferred startup metadata used to wipe the logo
        // one frame after terminal takeover.
        let plain = a3s_tui::style::strip_ansi(&banner_view(
            "gpt-5",
            "",
            "",
            "/workspace",
            None,
            120,
        ));
        assert!(
            plain.contains("████") && plain.contains(r"/    /\    \"),
            "welcome logo identity required for empty-transcript rebuild: {plain}"
        );
    }

    #[test]
    fn welcome_banner_does_not_wrap_inside_scrollbar_viewport() {
        let terminal_width = 52;
        let viewport_width = terminal_width;
        let rendered = banner_view(
            "gpt-5",
            "  ·  4 skills",
            "  ·  OS: dev@example",
            "/Users/roylin/code/a3s",
            Some("0.9.0"),
            viewport_width,
        );
        let expected_rows = rendered.split('\n').count();
        let mut viewport = a3s_tui::components::Viewport::new(viewport_width, 80);

        viewport.set_content(&rendered);

        assert_eq!(viewport.total_lines(), expected_rows);
    }
}
