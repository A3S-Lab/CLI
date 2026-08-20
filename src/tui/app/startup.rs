//! Opt-in startup timing and the Code TUI's explicit first-frame boundary.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{io::IsTerminal, io::Write};

use tokio_util::sync::CancellationToken;

const STARTUP_TRACE_ENV: &str = "A3S_CODE_STARTUP_TRACE";

/// Immediate feedback while the host constructs the correctness-critical
/// session state that must exist before terminal takeover.
pub(super) struct StartupLoadingIndicator {
    visible: bool,
}

impl StartupLoadingIndicator {
    pub(super) fn begin(interactive: bool) -> Self {
        let visible = interactive && std::io::stderr().is_terminal();
        if visible {
            let mut stderr = std::io::stderr().lock();
            let _ = write!(stderr, "\r\x1b[2K  ◌ a3s code · Loading workspace…");
            let _ = stderr.flush();
        }
        Self { visible }
    }

    pub(super) fn clear(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        let mut stderr = std::io::stderr().lock();
        let _ = write!(stderr, "\r\x1b[2K");
        let _ = stderr.flush();
    }
}

impl Drop for StartupLoadingIndicator {
    fn drop(&mut self) {
        self.clear();
    }
}

/// One-way acknowledgement opened only after the renderer has flushed the
/// first terminal frame.
///
/// `a3s-tui::Program` calls `Model::cursor` immediately after
/// `Renderer::render` returns. `Renderer::render` flushes its terminal output,
/// so the App acknowledges this gate at the beginning of `cursor`. Every
/// optional startup capability waits on the same gate instead of guessing the
/// renderer's progress with a timer.
#[derive(Clone, Debug)]
pub(super) struct FirstFrameGate {
    state: Arc<FirstFrameState>,
}

#[derive(Debug)]
struct FirstFrameState {
    ready: CancellationToken,
    acknowledged: AtomicBool,
    first_deferred_operation: AtomicBool,
    trace_enabled: bool,
    started_at: Instant,
}

impl FirstFrameGate {
    fn new(trace_enabled: bool, started_at: Instant) -> Self {
        Self {
            state: Arc::new(FirstFrameState {
                ready: CancellationToken::new(),
                acknowledged: AtomicBool::new(false),
                first_deferred_operation: AtomicBool::new(false),
                trace_enabled,
                started_at,
            }),
        }
    }

    /// Record the renderer's completed terminal flush and release waiters.
    pub(super) fn acknowledge_flushed(&self) {
        if self
            .state
            .acknowledged
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.trace_milestone("first_frame_flushed", None);
            self.state.ready.cancel();
        }
    }

    /// Headless smoke mode has no terminal renderer. It explicitly opens the
    /// same capability gate before issuing smoke requests without pretending a
    /// frame was rendered.
    pub(super) fn activate_headless(&self) {
        if self
            .state
            .acknowledged
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.trace_milestone("headless_startup_activated", None);
            self.state.ready.cancel();
        }
    }

    pub(super) async fn wait(&self) {
        self.state.ready.cancelled().await;
    }

    /// Mark the first deferred capability future that is actually polled. The
    /// operation name is a static, non-user-controlled diagnostic label.
    pub(super) fn record_deferred_operation(&self, operation: &'static str) {
        debug_assert!(
            self.state.ready.is_cancelled(),
            "deferred startup operation crossed the first-frame gate"
        );
        if self
            .state
            .first_deferred_operation
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.trace_milestone("first_deferred_operation", Some(operation));
        }
    }

    fn trace_milestone(&self, phase: &'static str, operation: Option<&'static str>) {
        if !self.state.trace_enabled {
            return;
        }
        let total = duration_ms(self.state.started_at.elapsed());
        match operation {
            Some(operation) => eprintln!(
                "[a3s-code-startup] phase={phase} operation={operation} phase_ms=0 total_ms={total}"
            ),
            None => eprintln!("[a3s-code-startup] phase={phase} phase_ms=0 total_ms={total}"),
        }
    }
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

    pub(super) fn first_frame_gate(&self) -> FirstFrameGate {
        FirstFrameGate::new(self.enabled, self.started_at)
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
    async fn first_frame_gate_is_event_driven_and_one_way() {
        let gate = FirstFrameGate::new(false, Instant::now());
        let waiter = {
            let gate = gate.clone();
            tokio::spawn(async move { gate.wait().await })
        };

        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        gate.acknowledge_flushed();
        waiter.await.expect("first-frame waiter");

        // Late subscribers observe the retained one-way acknowledgement.
        tokio::time::timeout(Duration::from_millis(20), gate.wait())
            .await
            .expect("late first-frame waiter");
    }
}
