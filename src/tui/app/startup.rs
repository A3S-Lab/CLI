//! Opt-in startup phase timing and post-first-frame scheduling for Code TUI.

use std::time::{Duration, Instant};

const STARTUP_TRACE_ENV: &str = "A3S_CODE_STARTUP_TRACE";
const POST_FIRST_FRAME_DELAY: Duration = Duration::from_millis(50);

/// Keep maintenance futures out of the initial renderer's CPU and I/O window.
///
/// `Program` dispatches `Model::init` commands immediately before its first
/// render. A short delay is therefore the explicit boundary that lets the
/// first frame reach the terminal before optional maintenance begins.
pub(super) fn after_first_frame<M: Send + 'static>(command: a3s_tui::Cmd<M>) -> a3s_tui::Cmd<M> {
    Box::pin(async move {
        tokio::time::sleep(POST_FIRST_FRAME_DELAY).await;
        command.await
    })
}

#[derive(Debug)]
pub(super) struct StartupTrace {
    enabled: bool,
    started_at: Instant,
    phase_started_at: Instant,
}

impl StartupTrace {
    pub(super) fn from_env() -> Self {
        Self::new(startup_trace_enabled(std::env::var_os(STARTUP_TRACE_ENV)))
    }

    fn new(enabled: bool) -> Self {
        let now = Instant::now();
        Self {
            enabled,
            started_at: now,
            phase_started_at: now,
        }
    }

    /// Record time since the preceding checkpoint without exposing user data.
    pub(super) fn checkpoint(&mut self, phase: &'static str) {
        let now = Instant::now();
        let phase_elapsed = now.saturating_duration_since(self.phase_started_at);
        let total_elapsed = now.saturating_duration_since(self.started_at);
        self.phase_started_at = now;
        if self.enabled {
            eprintln!(
                "[a3s-code-startup] phase={phase} phase_ms={} total_ms={}",
                duration_ms(phase_elapsed),
                duration_ms(total_elapsed),
            );
        }
    }
}

fn startup_trace_enabled(value: Option<std::ffi::OsString>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.to_string_lossy().trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_flag_requires_an_explicit_truthy_value() {
        assert!(!startup_trace_enabled(None));
        assert!(!startup_trace_enabled(Some("0".into())));
        assert!(!startup_trace_enabled(Some("false".into())));
        assert!(startup_trace_enabled(Some("1".into())));
        assert!(startup_trace_enabled(Some("ON".into())));
    }

    #[test]
    fn duration_conversion_preserves_milliseconds() {
        assert_eq!(duration_ms(Duration::from_millis(42)), 42);
    }

    #[tokio::test]
    async fn deferred_command_starts_after_the_first_frame_boundary() {
        let started_at = Instant::now();
        let result = after_first_frame(a3s_tui::cmd::cmd(|| async { 42_u8 })).await;

        assert!(started_at.elapsed() >= POST_FIRST_FRAME_DELAY);
        match result {
            a3s_tui::cmd::CmdResult::Msg(value) => assert_eq!(value, 42),
            _ => panic!("deferred command must preserve its message result"),
        }
    }
}
