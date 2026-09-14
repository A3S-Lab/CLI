//! P0 WIRE-1 — must-capability host wires for `a3s code exec`.
//!
//! Core owning a capability is not product green. The CLI host must attach the
//! wires below before admitting a Run. Checklist authority:
//! [`docs/must-capability-host-checklist.md`](../../../docs/must-capability-host-checklist.md).
//!
//! Refuse: counting Core unit tests as host wiring; soft-skip live as Effect.

use std::path::PathBuf;
use std::sync::Arc;

use a3s_code_core::SessionOptions;

use crate::lazy_memory_store::LazyFileMemoryStore;

/// Attach the workspace durable memory store used by LLM extract / recall.
///
/// `code exec` historically omitted this and silently produced `extract_calls=0`.
pub(super) fn with_workspace_memory_store(
    options: SessionOptions,
    memory_dir: impl Into<PathBuf>,
) -> SessionOptions {
    options.with_memory(Arc::new(LazyFileMemoryStore::new(memory_dir.into())))
}

#[cfg(test)]
mod tests {
    use super::super::exec_policy::session_options_with_sandbox;
    use super::*;
    use crate::cli::args::{CodeMode, CodeToolPolicy};

    #[test]
    fn with_workspace_memory_store_sets_session_memory() {
        let dir = tempfile::tempdir().unwrap();
        let options = SessionOptions::new();
        let wired = with_workspace_memory_store(options, dir.path());
        assert!(
            wired.memory_store.is_some(),
            "must-capability Memory wire missing on SessionOptions"
        );
    }

    #[tokio::test]
    async fn exec_session_options_include_skill_dirs_and_okf_builtin() {
        let workspace = tempfile::tempdir().unwrap();
        let options = session_options_with_sandbox(
            CodeMode::Plan,
            CodeToolPolicy::ReadOnly,
            workspace.path(),
            "host-wire-skills",
            None,
        );
        assert!(
            !options.skill_dirs.is_empty(),
            "must-capability skills/skill_dir wire missing"
        );
        assert!(
            options
                .skill_dirs
                .iter()
                .any(|dir| dir.ends_with(".a3s/cli/skills")),
            "must-capability $okf builtin skill root missing: {:?}",
            options.skill_dirs
        );
    }

    #[tokio::test]
    async fn exec_host_composition_applies_memory_on_top_of_session_options() {
        let workspace = tempfile::tempdir().unwrap();
        let memory = tempfile::tempdir().unwrap();
        let options = session_options_with_sandbox(
            CodeMode::Auto,
            CodeToolPolicy::Standard,
            workspace.path(),
            "host-wire-memory",
            None,
        );
        let wired = with_workspace_memory_store(options, memory.path());
        assert!(wired.memory_store.is_some());
        assert!(
            !wired.skill_dirs.is_empty(),
            "skill_dirs must survive memory wire composition"
        );
    }

    #[test]
    fn code_exec_source_calls_workspace_memory_wire() {
        // Catches the exact regression class: SessionOptions built without
        // `.with_memory` / helper. Not Effect proof — see checklist live rows.
        let src = include_str!("exec.rs");
        assert!(
            src.contains("host_must_wires::with_workspace_memory_store")
                || src.contains("with_workspace_memory_store("),
            "exec.rs must call with_workspace_memory_store (WIRE-1 Memory)"
        );
    }

    #[test]
    fn tui_launch_source_wires_memory_and_skill_dirs() {
        let src = include_str!("../../tui/app/launch.rs");
        assert!(
            src.contains(".with_memory("),
            "TUI launch must wire .with_memory (WIRE-1 Memory)"
        );
        assert!(
            src.contains(".with_skill_dirs("),
            "TUI launch must wire .with_skill_dirs (WIRE-1 skills)"
        );
        assert!(
            src.contains("LazyFileMemoryStore"),
            "TUI launch must use LazyFileMemoryStore (efficiency + Effect)"
        );
    }

    #[test]
    fn memory_host_docs_do_not_claim_home_default_store() {
        // UX-M1: chrome/module docs must not imply ~/.a3s/memory is the default
        // when the host resolver uses workspace .a3s/memory.
        for (label, src) in [
            ("memutil", include_str!("../../tui/context/memutil.rs")),
            ("sleep", include_str!("../../tui/panels/context/sleep.rs")),
            ("config template", include_str!("../../config.rs")),
        ] {
            assert!(
                !src.contains("~/.a3s/memory"),
                "{label} must not claim ~/.a3s/memory as the default durable store"
            );
        }
    }
}
