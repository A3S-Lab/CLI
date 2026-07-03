//! First-run welcome banner: animated ASCII-art logo + tips.

use super::super::*;

impl App {
    /// First-run welcome: ASCII-art logo, version, model, and tips.
    pub(crate) fn banner(&self) -> String {
        // A Song-dynasty soldier in a wide-brimmed helmet, holding a sword
        // (blade + `-+-` crossguard) in his right hand and a heater shield
        // (`|#|` tapering to `\#/`) in his left. Animated by `self.anim`: he
        // blinks, the crossguard glints, and he shifts his feet.
        let f = self.anim;
        let eyes = if f % 14 == 7 { "- -" } else { "o o" };
        let g = if f % 6 == 3 { "*" } else { "+" }; // crossguard glint
        let feet = if f.is_multiple_of(2) {
            r"/   \"
        } else {
            r"\   /"
        }; // shuffle
        let mascot = [
            r"     .-^-.      ".to_string(),
            r"    /_____\     ".to_string(),
            format!("    ( {eyes} )     "),
            r"  |  /|_|\  _   ".to_string(),
            format!(" -{g}- |   | |#|  "),
            r"  |  |___| \#/  ".to_string(),
            format!("     {feet}      "),
        ];
        let art = [
            r" █████╗ ██████╗ ███████╗     ██████╗ ██████╗ ██████╗ ███████╗",
            r"██╔══██╗╚════██╗██╔════╝    ██╔════╝██╔═══██╗██╔══██╗██╔════╝",
            r"███████║ █████╔╝███████╗    ██║     ██║   ██║██║  ██║█████╗",
            r"██╔══██║ ╚═══██╗╚════██║    ██║     ██║   ██║██║  ██║██╔══╝",
            r"██║  ██║██████╔╝███████║    ╚██████╗╚██████╔╝██████╔╝███████╗",
            r"╚═╝  ╚═╝╚═════╝ ╚══════╝     ╚═════╝ ╚═════╝ ╚═════╝ ╚══════╝",
        ];
        let margin = " ".repeat(PAD);
        let steel = TN_GRAY;
        // The 7-line mascot leads with its helmet; the 6-line wordmark aligns
        // from row 2 down (art row j sits on mascot row j+1).
        let logo = mascot
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let a = i
                    .checked_sub(1)
                    .and_then(|j| art.get(j))
                    .copied()
                    .unwrap_or("");
                format!(
                    "{margin}{}  {}",
                    Style::new().fg(steel).bold().render(m),
                    Style::new().fg(ACCENT).bold().render(a),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
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
        let meta = Style::new().fg(TN_GRAY).render(&format!(
            "{margin}a3s-code v{}  ·  {model}{skills}{os}  ·  {}",
            env!("CARGO_PKG_VERSION"),
            self.cwd
        ));
        let tips = Style::new().fg(TN_GRAY).italic().render(&format!(
            "{margin}Type a message · / for commands · Shift+Tab cycles mode · Ctrl+C twice to exit"
        ));
        let update = match &self.update_available {
            Some(v) => format!(
                "\n{margin}{}",
                Style::new().fg(ACCENT).bold().render(&format!(
                    "⬆ a3s {v} is available (you have {}) — type /update to upgrade",
                    env!("CARGO_PKG_VERSION")
                ))
            ),
            None => String::new(),
        };
        format!("\n{logo}\n\n{meta}\n{tips}{update}\n")
    }
}
