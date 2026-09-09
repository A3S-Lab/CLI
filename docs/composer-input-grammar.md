# Composer input grammar: Cursor CLI vs A3S Code

Interaction-grammar checklist for the `a3s code` PromptBar. Goal: align
everyday typing/submit feel with [Cursor CLI](https://cursor.com/docs/cli/using)
where that strengthens the product; keep intentional A3S differences.

Statuses:

- **对齐** — match Cursor CLI grammar (or already matches).
- **有意不同** — deliberate product/architecture choice; do not clone.
- **不做** — out of scope for this train (cost/risk without clear payoff).

| Item | Cursor CLI | A3S Code | Status | Evidence / notes |
| --- | --- | --- | --- | --- |
| Enter submits | Yes | Yes (`submit_on_enter`) | 对齐 | `Textarea` + `launch.rs` |
| Ctrl+J newline | Documented universal | Yes | 对齐 | `textarea_tests::ctrl_j_inserts_newline…` |
| Shift+Enter newline | When terminal reports Shift | Yes (Kitty enhanced keys) | 对齐 | `help.rs`, terminal protocol |
| `\+Enter` newline | Universal | Not special-cased | 不做 | Ctrl+J already covers multiplexers |
| Shift+Tab mode cycle | Agent → Plan → Ask | agent → plan → reviewer → auto → yolo; `/ask`→plan | 有意不同 + 薄对齐 | No Cursor-only Ask state; `/ask`/`--mode ask` map to Plan |
| Ask / Cloud `&` / model slash | Ask + Cloud + model slash | `/ask`→plan; no Cloud/`/opus` | 有意不同 | Ask alias only; Cloud/model slash remain out |
| Empty-state placeholder | Visible in empty prompt | Focused empty shows dim `Add a follow-up` | 对齐 | `Textarea` placeholder + `launch.rs` |
| Prompt glyph | Muted `→` | Muted non-bold `→` (shell `!` / research `?` stay bold) | 对齐 | `composer_prompt_glyph` + `composer_prompt_bar` |
| Visual grow / scroll cap | ~6 visual lines then scroll | `auto_grow(6)` then internal scroll | 对齐 | Matches Cursor changelog budget |
| Raised bottom PromptBar | Native scrollback tradeoff debated | Half-block caps + raised gray, pinned | 有意不同 | SessionChrome contract |
| Quiet footer under bar | Model/cwd style status | mode · model · ctx · path/branch | 有意不同 | A3S chrome ownership |
| Up/Down history | Session prompt history | Single-line always; multiline Up at row 0 | 对齐 | `update_dispatch` + `history_recall` |
| Multiline Up/Down mid-text | Move cursor | Move cursor (not history) | 对齐 | Same grammar |
| Ctrl+R / `/history` fuzzy | Present | Present | 对齐 | `cli-reference.md` |
| Diff review / follow-up | Ctrl+R review in some builds | **Ctrl+G** Diff review; ←/→/`h`/`l` files; `i` seeds draft; word-level peek | 有意不同 + 增强 | Ctrl+R stays history; `diff_review.rs` + `emphasize_inline_changes` |
| Image paste | Clipboard / attach | Ctrl+V chips; `@` image paths | 对齐 | attachments path |
| Large-paste pill collapse | Changelog behavior | Large pastes (≥15 lines or ≥400 chars) stage as PromptBar pills; submit/history send full text; click expands, × removes; empty Backspace pops paste before image | 对齐 | `ui/paste_pills.rs` + submit/history + mouse hit-test |
| Slash commands | `/` palette | `/` palette | 对齐 | `menu.rs` |
| `@` file refs | Yes | IDE-style file tree | 对齐 | `files.rs` |
| `$skill` mentions | N/A / different | Codex-style `$skill`; Alt/Option+Enter sticky until Esc/`/unstick` | 有意不同 + 薄对齐 | Sticky custom-mode spirit without Cursor JSON import |
| `!` shell / `?` research | Different | Sticky `!`; `?` DeepResearch | 有意不同 | A3S prefixes |
| Vim input mode | `/config` | Not in composer | 不做 | Separate workstream |
| `/setup-terminal` | First-class wizard | `/terminal` diagnostics + repair snippets; `A3S_CODE_NOTIFY` / `A3S_CODE_SUGGEST` | 有意不同 + 薄对齐 | Repair path without cloning Cursor wizard UI |
| Worktree isolation launch | `agent --worktree` | `a3s code --worktree [NAME]` → `.a3s-worktrees` + `/worktree` lifecycle | 有意不同 + 薄对齐 | Same isolation class as `/fork worktree`; non-forcing cleanup |
| Queue while busy | Queued messages | Enter queues; Ctrl+O send-now | 对齐 | Distinct Send-now is A3S-stronger |
| Drag-drop files into bar | Varies | No | 不做 | `@` + clipboard cover MVP |

## Non-goals

- Pixel-perfect Cursor skin, Ask mode, Cloud Agent handoff, or model family
  slash shortcuts (`/opus`, …).
- Replacing SessionChrome / PromptBar with a Cursor layout clone.

Broader TUI optimization beyond this input checklist:
[`tui-cursor-alignment-roadmap.md`](./tui-cursor-alignment-roadmap.md).

## Verification

Focused gates for this train:

1. `cargo test -p a3s-tui textarea` (placeholder + auto-grow + Ctrl+J).
2. `cargo test --bin a3s paste_` (pill thresholds, merge, hit-test expand/remove).
3. `cargo test --bin a3s should_recall_prompt_history` (multiline ↑ history edge).
4. Manual or PTY smoke (`tui_exit`): empty composer shows placeholder; sixth
   newline scrolls instead of growing the bar indefinitely; a ≥15-line paste
   stages a pill and submits the full body; click expands / × removes.
