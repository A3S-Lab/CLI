# Reviewer mode (Code TUI)

Claude Science Auto-review analogue for **coding replies**, plus a separate git
code-review surface. These are two products — not one shared “review chat”.

| Surface | Role | Execution |
| --- | --- | --- |
| Sticky `/reviewer` (Shift+Tab) | **Reply verifier** — claim↔record on the latest assistant message vs user request + bounded turn tool evidence | `ReplyVerifierLane` → Core Gate admission → independent `AuxiliaryExecutor` (not the main session) |
| Explicit `/review …` | **Git code review** — working-tree / commit / branch scope | `GitReviewLane` → side-session `AgentStyle::CodeReview` |

## Hard rules

- Neither product enters the main turn queue or main AgentEvent pump.
- Sticky does **not** re-run tools or judge method choice beyond claim↔record.
- Incomplete evidence (truncated tools, non-terminal tools, retention gaps) is
  **Gate fail-closed**: no published clean pass; surface fail-closed chrome.
  Prefer `inconclusive` / `warn` when a structured verdict is still produced.
- Sticky jobs coalesce (at most one queued sticky). Git reviews do not share
  that coalesce key and must not clear sticky `open_reply_findings`.
- Open sticky findings inject into the next main-stream user prompt as DATA
  until addressed (checklist Enter) or waived (`w`).
- Sticky must not use `AgentStyle::CodeReview` as its identity; that style
  belongs to git `/review` only.
- Effectiveness evidence is mock/live verdict pipelines, not prompt-substring
  tests alone (those are Contract rows in the matrix).

## Triggers (Claude Science mapping)

| Claude Science | Code TUI |
| --- | --- |
| Auto-review after response | Sticky Reviewer mode after a successful main turn (arming gate) |
| Periodic during long work | Deferred (not required for Gate path) |
| Request review | Do not overload `/review`; optional reply re-run later |

Arming skips: non-Reviewer mode, deep-research, sleep, goal, missing evidence.
