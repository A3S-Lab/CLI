# Applet UI Projection Parity (CLI)

Status: reference host delivered (P3); parity checklist for Desktop

Canonical multi-project plan:
[`docs/desktop-applet-plugin-path.md`](../../../docs/desktop-applet-plugin-path.md).

## Ownership

CLI remains the **reference** Use → Code UI projection host. Desktop must copy
contracts from this path and must not depend on the CLI binary.

## Mechanisms delivered

1. Stage `CapabilityKind::Ui` / `CapabilityValue::Ui` in
   `src/use_registry/capability_batch.rs` in the same atomic batch as
   Tool/MCP/Skill/Flow.
2. Integrity-bound asset load (`load_managed_ui_asset`).
3. UI dependency edges closed against Tool / Skill / MCP / Flow.
4. Runtime Tool Tasks stage Core `UseRuntimeTaskProjectionAdapter` (W2), with
   CLI owning only the Plugin Manager dispatch bridge
   (`RuntimeTaskInvokerDispatcher`).
5. Package-local Executable Tool Tasks stage via
   `executable_tools::projection_adapter`: reinspect `file_evidence_digest`,
   resolve the relative executable under the package root, and spawn with
   bounded argv — never invent a Runtime BindingStore `provider_id`.

## Work remaining (parity checklist)

1. When Desktop UiHost lands, document any projection deltas here.
2. Do not regress static-integrity loading while Desktop adopts UiHost.
3. Keep CLI/TUI as a non-Desktop proof path for UI batch staging (optional
   preview may use `a3s-webview`; that is not product Desktop UiHost).
4. Desktop must copy the Executable Tool host path (not depend on the CLI
   binary) when W1/P0 adopt UI `bind_tool`.

## Exit evidence

- Focused CLI tests stage UI contributions into `SessionCapabilityBatch`
  without path-escaping assets.
- First-principles: `registry_projects_real_applet_demo_package_ui_bytes`
  projects monorepo `use-registry/packages/applet-demo` HTML/CSS/JS bytes into
  `applet-demo:panel`, stages managed stdio MCP `context` from package
  `mcp/context`, projects Executable Tool `echo` from package `tools/echo`,
  and asserts projected Skill name/description/body match the package
  `SKILL.md` parse (not Host-authored fixtures); MCP evidence drift fails
  closed.
- First-principles Executable Tool host path:
  `applet_demo_package_executable_echo_reinspects_and_resolves` reinspecs,
  prepares, and spawns package `tools/echo`; evidence digest drift fails closed.
- First-principles atomic UI `bind_tool`:
  `applet_demo_executable_echo_satisfies_ui_bind_tool_in_atomic_batch` stages
  package `tools/echo` + package HTML into one `SessionCapabilityBatch`,
  publishes `applet-demo:panel` with Tool dependency `echo`, and fails closed
  when the Executable Tool is omitted.
- First-principles E2E (ignored without Use binaries):
  `real_use_installs_applet_demo_and_cli_projects_panel_ui` assembles admissions,
  installs `a3s/applet-demo` through a real `a3s-use` process (UI
  `bind_tool=["echo"]` + package-local Executable Tool), asserts
  `projected_ui("applet-demo:panel")` equals package HTML bytes, and checks
  frozen UI dependencies include Tool `echo` / MCP `context` / Skill
  `applet-demo` with the Executable Tool staged in the atomic projection.
- First-principles committed-tree E2E (ignored without Use binary):
  `real_use_installs_committed_applet_demo_and_cli_projects_panel_ui` serves
  `use-registry/registry/` as published (no fresh keygen/assemble), asserts
  archive `bind_tool=["echo"]`, plan-install catalog requires Tool+MCP+Skill,
  and CLI projects **archive** HTML bytes with the Executable Tool staged.
- Signed real-Use install convergence waits for `ui_ready` and asserts
  `projected_ui("report:reports")` contains package bytes (not a host-built
  HTML fixture) — see `wait_for_signed_report` and the
  `real_use_process_converges_signed_install_*` tests.

## Non-goals

- Desktop WebView or shell chrome
- Becoming Desktop’s runtime dependency
- Owning CSP / origin policy for product Applets
- Stuffing package-local Executable Tools into Release `ToolTaskProjection` /
  Runtime BindingStore receipts (that path stays provider-qualified)
