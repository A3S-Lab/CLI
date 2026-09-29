//! On-disk locations for Code session documents and per-session host state.

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

/// Directory that stores interactive and `a3s code exec` session documents.
///
/// Prefers `.a3s/tui/sessions`. An older checkout that only has
/// `.a3s/tui-sessions` is renamed forward when that rename succeeds.
pub(crate) fn resolve_tui_session_store_dir(workspace: &Path) -> PathBuf {
    let tui_dir = workspace.join(".a3s/tui");
    let canonical = tui_dir.join("sessions");
    let legacy = workspace.join(".a3s/tui-sessions");
    if !canonical.exists() && legacy.exists() {
        let _ = std::fs::create_dir_all(&tui_dir);
        if std::fs::rename(&legacy, &canonical).is_err() {
            return legacy;
        }
    }
    canonical
}

pub(crate) fn tui_session_state_path(workspace: &Path, session_id: &str) -> PathBuf {
    let key = URL_SAFE_NO_PAD.encode(session_id.as_bytes());
    workspace
        .join(".a3s")
        .join("tui")
        .join("session-state")
        .join("v1")
        .join(format!("id_{key}.json"))
}
