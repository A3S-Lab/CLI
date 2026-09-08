# Code TUI L2 case matrix

Priority: P0 must stay green in CI once wired. Higher IDs expand coverage.

## P0 — Boot and lifecycle

| ID | Suite | Assertion |
| --- | --- | --- |
| TUI-BOOT-01 | `01-boot.acl` | First frame shows `a3s-code v` and composer tip |
| TUI-EXIT-01 | `06-resize-exit.acl` | Ctrl+C twice ends after chrome is visible |
| TUI-NAV-01 | `06-resize-exit.acl` | Resize 80×24 → 120×40 keeps tip text |

## P1 — Composer and slash

| ID | Suite | Assertion |
| --- | --- | --- |
| TUI-CMD-01 | `02-composer-help.acl` | `/help` then Enter shows `A3S Code` help chrome |
| TUI-COMP-01 | _(planned)_ | Paste + Enter with fake LLM |
| TUI-COMP-02 | _(planned)_ | Ctrl+J multiline |

## P2 — Panels / HITL / sandbox

| ID | Status |
| --- | --- |
| TUI-PERM-* | Planned — approval bar verbs |
| TUI-SBX-* | Planned — `!` shell + fail-closed copy |
| TUI-DR-* / TUI-RUI-* | Planned — narrow DeepResearch / Open view text |
