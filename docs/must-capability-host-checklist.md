# Must-capability host checklist (WIRE-1)

Authority: [product maturity roadmap](../../../docs/a3s-code-product-maturity-optimization-roadmap.md) §P0
Living Effect evidence: [capability-test-matrix.md](./capability-test-matrix.md) §0

**Rule:** Core owning a capability ≠ product green. The CLI host (`code exec` +
TUI session boot) must attach each wire below. Missing wire fails CI via the
named hermetic tests — not via soft-skip live or chrome substrings.

## Checklist (1:1 with matrix must rows)

| Must capability | Host wire | Hermetic evidence | Live / Effect (not soft-skip) |
| --- | --- | --- | --- |
| WorkBuddy account models | Provider discovery in model list / exec | `discover_model_list_keeps_auto_router_*`, `prefers_signed_in_workbuddy_ai_*` | W1 PONG under `workbuddy/auto` |
| Coding loop read/write/sandbox/session | Exec policy + sandbox handle | `verified_sandbox_is_attached_*`, plan/read admit tests | W2–W5 |
| BM25 / zvec | Workspace retrieval options on session | Core Z* + host catalog configure | Matrix BM25 re-arm |
| Moli `web_search` | Default headless backend registration | `default_headless_web_search_backend_is_moli` | YEAR live |
| Memory (host + agent extract) | `host_must_wires::with_workspace_memory_store` + TUI `LazyFileMemoryStore` | `with_workspace_memory_store_sets_session_memory`, `code_exec_source_calls_workspace_memory_wire`, `tui_launch_source_wires_memory_and_skill_dirs`, `auto_mode_persists_llm_memory_*` | Workspace `code memory list` reopen |
| skills / `skill_dir` / `$okf` | `exec_policy` skill discovery + TUI `.with_skill_dirs` | `exec_session_options_include_skill_dirs_and_okf_builtin`, `tui_launch_source_wires_*` | `OKF_SKILL_LOADED` |
| Reviewer sticky | TUI `/reviewer` + Core `session_review` | Reviewer R* hermetics | Ignored live claim↔record |
| RemoteUI | `a3s-webview` + UX-U1 auto-open gate | `remote_ui*` + `remote_ui_auto_open_gate_*` | `a3s doctor webview` Ready |
| Flow / DeepResearch | Research host + digest-fold pin | `evidence_first*`, pin scripts | local-only `source_backed` |
| `/kb` | Host kb cmds + Core search allowlist | `kb_*` tests | Host search + agent `KB_HIT` |
| Use OKF / UKS | Host Use projection + brew pin | `okf*`, atomic flow | brew `0.3.12` UKS compose |

## CI gate

From `crates/cli`:

```bash
cargo test --bin a3s host_must_wires:: --quiet
cargo test --bin a3s remote_ui_auto_open_gate_ --quiet
./scripts/verify-capability-regression.sh
```

## Refuse

| Claim | Why rejected |
| --- | --- |
| Core memory unit test green | Does not prove CLI `.with_memory` |
| Soft-skip ignored live | Not Effect |
| Desktop Code screenshot | Not CLI host wire |
| `doctor webview` Ready ⇒ tool views auto-embed | Violates UX-U1 RememberOnly |
