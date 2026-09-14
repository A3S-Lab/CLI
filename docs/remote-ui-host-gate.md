# RemoteUI host open gate (UX-U1)

Authority: [product maturity roadmap](../../../docs/a3s-code-product-maturity-optimization-roadmap.md) §P1 UX-U1

The terminal cannot embed a WebView. Spawning `a3s-webview` (or a browser
fallback) is an explicit host side-effect. Auto-open is therefore gated:

| Source | Gate | Behavior |
| --- | --- | --- |
| Tool stream `view` / `viewUrl` | `RememberOnly` | Transcript “click to open”; **no** auto window |
| Host-owned local report (DeepResearch HTML, research markers) | `AutoOpenAllowed` | Open when the view is **new** |
| Explicit user open (click / command) | `AutoOpenAllowed` | Open immediately |

Implementation: `src/tui/os/remote_ui_auto_open.rs`  
Hermetic: `remote_ui_auto_open_gate_*`  
Readiness (separate contract): `a3s doctor webview` → Ready

## Refuse

- `doctor webview` Ready as proof that tool views auto-embed
- Footer / “click to open” substring as Effect (detect)
- Silent no-op when open fails (must surface browser hint or error line)
