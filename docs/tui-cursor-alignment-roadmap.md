# A3S Code TUI ↔ Cursor CLI alignment roadmap

Optimize the `a3s code` TUI against Cursor CLI where interaction grammar and
review loops strengthen the product. Do **not** clone Cursor's product universe
(Cloud, Ask-as-editor-parity, `mcp.json` isomorphism, ACP, model-family slash).

Companion docs:

- Input grammar checklist: [`composer-input-grammar.md`](./composer-input-grammar.md)
- Product surface: [`cli-reference.md`](./cli-reference.md)

Statuses used below:

| Status | Meaning |
| --- | --- |
| **对齐** | Match Cursor grammar or already match |
| **增强** | Keep A3S-native design; improve discoverability / loop |
| **有意不同** | Deliberate fork; do not schedule as “parity” |
| **不做** | Out of scope for this roadmap |

---

## North star

1. **Composer feel** stays Cursor-familiar (already largely done).
2. **Trust model** stays A3S-hard (native sandbox + `permissions.acl` + Auto/Yolo).
3. **Review loop** becomes first-class (closest high-value gap vs Cursor Ctrl+R).
4. **Mode axis** stays autonomy-first; optionally add a thin **Ask** alias on Plan.

---

## Phase 0 — Freeze (done / do not reopen)

| Item | Status | Notes |
| --- | --- | --- |
| Enter / Ctrl+J / Shift+Enter newlines | 对齐 | |
| Focused empty placeholder | 对齐 | |
| `auto_grow(6)` + internal scroll | 对齐 | |
| Multiline ↑ history at row 0 | 对齐 | |
| Large-paste pills + mouse expand/× | 对齐 | |
| Image chips; queue + Ctrl+O send-now | 对齐 / 增强 | Send-now is A3S-stronger |
| Pixel chrome / Ask universe / Cloud `&` / Vim composer / drag-drop / `/opus` | 不做 | See Non-goals |

Gate: existing `textarea` / `paste_` / `should_recall_*` / `tui_exit` suites stay green.

---

## Phase 1 — Review loop (highest local ROI)

**Status:** Implemented (Ctrl+G). Word-level peek via `DiffView::emphasize_inline_changes`.
Ctrl+G while open refreshes and retains the focused path when possible.

**Goal:** Cursor-like “inspect diffs → instruct follow-up → switch files” without
leaving the TUI, on top of existing DiffView / transcript cards.

| Work item | Mechanism | Exit criteria |
| --- | --- | --- |
| 1.1 Diff review mode | **Ctrl+G** opens latest-turn file changes (`panels/system/diff_review.rs`) | From a completed edit turn, Ctrl+G enters review; Esc returns to composer |
| 1.2 File switch | ←/→ or `h`/`l` across changed paths | ≥2-file turn can switch without mouse |
| 1.3 Follow-up instruct | `i` seeds composer with path + action context | Submit continues the same session with explicit file focus |
| 1.4 Word-level peek | `DiffView::emphasize_inline_changes` after syntax highlight in `file_change_view` | Adjacent delete/insert pairs emphasize changed words (fg-only) |

**Verify:** `cargo test --bin a3s diff_review` + help Ctrl+G assertion.

**Non-goals for P1:** Cloud handoff of the review; GitHub PR UI.

**Depends on:** nothing blocking; can ship before release train companions.

---

## Phase 2 — Mode discoverability (thin Ask, keep autonomy axis)

**Status:** Implemented (`/ask` `/plan` → Plan; `--mode ask` alias; help/banner legend).

**Goal:** Users who know Cursor’s Agent/Plan/Ask find a readable mapping; do not
replace Auto/Yolo/Reviewer.

| Work item | Mechanism | Exit criteria |
| --- | --- | --- |
| 2.1 `/ask` alias | Maps to Plan (read-only) | `/ask`, `/plan`, and `code exec --mode ask` → Plan |
| 2.2 Shift+Tab legend | Help + banner + `Mode::axis_legend` | Documented |
| 2.3 Mode capture on queue | Already true; grammar doc updated | Grammar row updated |

**Non-goals for P2:** Three-mode-only Shift+Tab (would delete Auto/Yolo); Cloud.

**Verify:** `cargo test --bin a3s auto_and_yolo_stay_typed` + `parses_exec_mode_ask`.

---

## Phase 3 — Terminal setup & completion UX

**Status:** Implemented (`/terminal` repair hints; `A3S_CODE_NOTIFY` / `A3S_CODE_SUGGEST`).
Notify/suggest fire when the primary queue is empty even if sticky Reviewer spawns.

**Goal:** Reduce “my terminal eats Shift+Enter” and “did the agent finish?” friction.

| Work item | Mechanism | Exit criteria |
| --- | --- | --- |
| 3.1 `/terminal` repair path | Diagnostics append copy-paste snippets for Kitty/iTerm/tmux / Ctrl+J | One command surfaces actionable fix, not only capability dump |
| 3.2 Turn-complete signal | Optional BEL + OSC 9 via `A3S_CODE_NOTIFY=1` | Off by default; documented env flag |
| 3.3 Suggest next prompt (optional) | After idle turn, one dim tip via `A3S_CODE_SUGGEST=1` (Ctrl+G when files changed) | Feature-flagged; no model call |

**Verify:** `cargo test --bin a3s terminal::` (repair + idle suggestion).

**Non-goals for P3:** Composer Vim; full Cursor cli-config.json import.

---

## Phase 4 — Session isolation productization

**Status:** Implemented (`a3s code --worktree [NAME]`).

**Goal:** Make worktree isolation as obvious as Cursor `agent --worktree`.

| Work item | Mechanism | Exit criteria |
| --- | --- | --- |
| 4.1 Launch flag | `a3s code --worktree [NAME]` creates sibling `.a3s-worktrees` checkout and starts TUI there; binds managed lifecycle for `/worktree status|handoff|cleanup` | Flag round-trips in clap + create/bind unit tests |
| 4.2 Retention policy | Cleanup prints non-forcing `git worktree remove` / `branch -d` only | No silent deletion of user branches |

**Verify:** `cargo test --bin a3s parses_code_worktree` + `launch_worktree_identity`.

**Non-goals for P4:** Cursor `~/.cursor/worktrees` path layout.

---

## Phase 5 — Sticky custom modes (skills)

**Status:** Implemented (Alt/Option+Enter sticky `$skill`; Esc/`/unstick` clear).
Sticky attach clears the composer so Esc can detach immediately.

**Goal:** Cursor Option+Enter “skill stays until exit” without inventing a second skill system.

| Work item | Mechanism | Exit criteria |
| --- | --- | --- |
| 5.1 Sticky skill mode | Alt/Option+Enter on `$skill` menu attaches until Esc (empty) or `/unstick`; footer shows `sticky:<name>` (including Zen); sticky outranks disabled-skill list | Footer chip + expand_skill_mentions includes sticky |
| 5.2 One-shot vs sticky | Enter = complete mention (one message); Alt/Option+Enter = sticky | Documented in help |

**Verify:** `cargo test --bin a3s sticky_skill_is_selected` + help `/unstick`.

**Non-goals for P5:** Import Cursor custom mode JSON.

---

## Phase 6 — Strategic / platform (schedule only with product decision)

These are **not** TUI polish. Track separately; do not block Phases 1–3.

| Item | Status | Why deferred |
| --- | --- | --- |
| Cloud Agent `&` | 有意不同 | Requires A3S cloud agent product |
| Cursor `mcp.json` isomorphism | 有意不同 | Use registry / `/plugin` ownership |
| ACP server | 有意不同 | New host protocol; Core/SDK concern |
| Editor rules (`.cursor/rules`) auto-load | 有意不同 | ACL + AGENTS.md policy already exists; bridge is optional later |
| `/usage` billing streaks | 有意不同 | Different commercial surface |

---

## Sequencing and release coupling

```text
P0 (done) ──► P1 Review loop ──► P2 Ask alias ──► P3 Terminal/notify
                      │
                      └──────────► P4 Worktree flag (can parallel after P1)
                      └──────────► P5 Sticky skill (can parallel after P2)

P6 platform ………………………………………… independent product train
Release train (Code 8.4 / tui / webview / CLI 0.15) ………………………………………… orthogonal
```

- **Ship P1 with or before** the next CLI minor if review is ready; do not hold
  the companion publish train on P2–P5.
- Composer grammar doc stays the contract for input; this roadmap owns
  **modes / review / setup / isolation**.

---

## Per-phase definition of done

1. First-principles note in PR: what user pain, why this layer owns it.
2. Tests first where behavior is stateful (review mode, ask policy).
3. `docs/composer-input-grammar.md` and/or this file updated.
4. `CHANGELOG.md` Unreleased entry.
5. Focused `cargo test` + relevant PTY smoke; no orphaned `#[ignore]`.

---

## Explicit refuse list

- Pixel-perfect Cursor skin or un-pinning SessionChrome.
- Replacing Auto/Yolo/Reviewer with Agent/Plan/Ask only.
- Cloud `&`, ACP, or `mcp.json` as a TUI “quick win”.
- Composer Vim mode as part of this roadmap.
- Drag-drop into the PromptBar.
- Model-family slash shortcuts (`/opus`, …).
