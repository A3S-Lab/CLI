//! RemoteUI host open gate (P1 UX-U1).
//!
//! The TUI cannot embed a WebView in-process. Opening `a3s-webview` (or a
//! browser fallback) is a host side-effect. Auto-open is therefore **gated**:
//!
//! | Source | Gate | Behavior |
//! | --- | --- | --- |
//! | Tool stream `view` / `viewUrl` | `RememberOnly` | Transcript "click to open"; no auto window |
//! | Host-owned local report (DeepResearch / research markers) | `AutoOpenAllowed` | Open when the view is new |
//! | Explicit user open action | `AutoOpenAllowed` | Open immediately |
//!
//! Refuse: treating `a3s doctor webview` Ready as proof that tool views
//! auto-embed; counting chrome substring tests as Effect (detect).

/// Whether the TUI host may spawn a RemoteUI window without an explicit click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteUiAutoOpenGate {
    /// Remember the view and render click-to-open chrome only.
    RememberOnly,
    /// Host may call `open_remote_view` when the view is new (or user-initiated).
    AutoOpenAllowed,
}

/// Tool-streamed RemoteUI payloads never auto-open (host-gated).
pub(crate) fn remote_ui_auto_open_gate_for_tool_stream() -> RemoteUiAutoOpenGate {
    RemoteUiAutoOpenGate::RememberOnly
}

/// Host-staged local reports (DeepResearch HTML, research markers) may auto-open.
pub(crate) fn remote_ui_auto_open_gate_for_host_local_report() -> RemoteUiAutoOpenGate {
    RemoteUiAutoOpenGate::AutoOpenAllowed
}

/// User-initiated open actions always may open.
pub(crate) fn remote_ui_auto_open_gate_for_user_open_action() -> RemoteUiAutoOpenGate {
    RemoteUiAutoOpenGate::AutoOpenAllowed
}

/// Decide whether a remembered view should auto-open under the given gate.
pub(crate) fn remote_ui_should_auto_open(gate: RemoteUiAutoOpenGate, is_new_view: bool) -> bool {
    matches!(gate, RemoteUiAutoOpenGate::AutoOpenAllowed) && is_new_view
}

#[cfg(test)]
mod remote_ui_auto_open_gate_tests {
    use super::*;

    #[test]
    fn tool_stream_views_are_remember_only() {
        assert_eq!(
            remote_ui_auto_open_gate_for_tool_stream(),
            RemoteUiAutoOpenGate::RememberOnly
        );
        assert!(!remote_ui_should_auto_open(
            remote_ui_auto_open_gate_for_tool_stream(),
            true
        ));
    }

    #[test]
    fn host_local_reports_auto_open_only_when_new() {
        let gate = remote_ui_auto_open_gate_for_host_local_report();
        assert_eq!(gate, RemoteUiAutoOpenGate::AutoOpenAllowed);
        assert!(remote_ui_should_auto_open(gate, true));
        assert!(!remote_ui_should_auto_open(gate, false));
    }

    #[test]
    fn user_open_action_allows_auto_open() {
        assert_eq!(
            remote_ui_auto_open_gate_for_user_open_action(),
            RemoteUiAutoOpenGate::AutoOpenAllowed
        );
    }

    #[test]
    fn events_source_remembers_tool_views_without_open_remote_view() {
        // ToolEnd / ToolOutputDelta must remember only — opening is click-gated.
        let src = include_str!("../app/events.rs");
        let tool_end = src
            .split("AgentEvent::ToolEnd")
            .nth(1)
            .expect("ToolEnd handler");
        let tool_end = tool_end.split("AgentEvent::").next().unwrap_or(tool_end);
        assert!(
            tool_end.contains("remember_remote_view"),
            "ToolEnd must remember RemoteUI views"
        );
        assert!(
            !tool_end.contains("open_remote_view"),
            "ToolEnd must not auto-open RemoteUI (UX-U1 RememberOnly)"
        );
    }

    #[test]
    fn deep_research_pending_view_auto_opens_when_new() {
        let src = include_str!("../app/view.rs");
        assert!(
            src.contains("open_pending_deep_research_report_view"),
            "host local DR reports must have an auto-open path"
        );
        let pending = src
            .split("fn open_pending_deep_research_report_view")
            .nth(1)
            .expect("open_pending_deep_research_report_view");
        let pending = pending.split("pub(super) fn").next().unwrap_or(pending);
        assert!(
            pending.contains("open_remote_view"),
            "pending DR report must call open_remote_view when new"
        );
    }
}
