//! First-run welcome banner: animated ASCII-art logo + tips.

use super::super::*;
use a3s_tui::components::WelcomeBanner;

impl App {
    /// First-run welcome: ASCII-art logo, version, model, and tips.
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
            self.anim,
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
    anim: u8,
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
        .mascot_lines(banner_mascot(anim))
        .art_lines(banner_wordmark())
        .art_offset(1)
        .margin(PAD)
        .gap(2)
        .mascot_color(TN_GRAY)
        .art_color(ACCENT)
        .metadata_color(TN_GRAY)
        .tip_color(TN_GRAY)
        .notice_color(ACCENT)
        .metadata(metadata)
        .tip("Type a message · / for commands · Shift+Tab cycles mode · Ctrl+C twice to exit");
    if let Some(v) = update_available {
        banner = banner.notice(format!(
            "⬆ a3s {v} is available (you have {}) — type /update to upgrade",
            env!("CARGO_PKG_VERSION")
        ));
    }

    format!("\n{}\n", banner.view(width, usize::MAX))
}

fn banner_mascot(anim: u8) -> Vec<String> {
    // A Song-dynasty soldier in a wide-brimmed helmet, holding a sword
    // (blade + `-+-` crossguard) in his right hand and a heater shield
    // (`|#|` tapering to `\#/`) in his left. Animated by `anim`: he
    // blinks, the crossguard glints, and he shifts his feet.
    let eyes = if anim % 14 == 7 { "- -" } else { "o o" };
    let g = if anim % 6 == 3 { "*" } else { "+" };
    let feet = if anim.is_multiple_of(2) {
        r"/   \"
    } else {
        r"\   /"
    };
    vec![
        r"     .-^-.      ".to_string(),
        r"    /_____\     ".to_string(),
        format!("    ( {eyes} )     "),
        r"  |  /|_|\  _   ".to_string(),
        format!(" -{g}- |   | |#|  "),
        r"  |  |___| \#/  ".to_string(),
        format!("     {feet}      "),
    ]
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
        let viewport_width = terminal_width - 1;
        let rendered = banner_view(
            3,
            "gpt-5",
            "  ·  4 skills",
            "  ·  OS: dev@example",
            "/Users/roylin/code/a3s",
            Some("0.9.0"),
            viewport_width,
        );
        let plain = a3s_tui::style::strip_ansi(&rendered);

        assert!(plain.contains("a3s-code v"), "{plain}");
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
    }

    #[test]
    fn welcome_banner_does_not_wrap_inside_scrollbar_viewport() {
        let terminal_width = 52;
        let viewport_width = terminal_width - 1;
        let rendered = banner_view(
            3,
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
