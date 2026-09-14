# Reviewer mode (Code TUI)

Claude Science / Desktop Auto-review analogue for **coding replies**, plus a
separate git code-review surface. These are two products — not one shared
“review chat”.

| Surface | Role | Execution |
| --- | --- | --- |
| Sticky `/reviewer` (Shift+Tab) | **Reply verifier** — claim↔record on the latest assistant message vs user request + bounded turn tool evidence | `ReplyVerifierLane` → Core Gate admission → independent `AuxiliaryExecutor` (forked side-path LLM via `StructuredAuxiliaryExecutor` when available; protocol rubric only for hermetic/offline) |
| Explicit `/review …` | **Git code review** — working-tree / commit / branch scope | `GitReviewLane` → side-session `AgentStyle::CodeReview` |

## Hard rules

- Neither product enters the main turn queue or main AgentEvent pump.
- Sticky does **not** re-run tools or judge method choice beyond claim↔record.
- Incomplete evidence (truncated tools, non-terminal tools, retention gaps) is
  **Gate fail-closed**: no published clean pass; surface fail-closed chrome.
  Prefer `inconclusive` / `warn` when a structured verdict is still produced.
- Sticky jobs coalesce (at most one queued sticky). Git reviews do not share
  that coalesce key and must not clear sticky `open_reply_findings`.
- Open sticky findings are stored in Core `session_review` under
  `reply.transcript` (same authority as Desktop). `App.open_reply_findings` is
  a UI projection synced from pending Findings.
- A new sticky **reply** report replaces the open set: Core pending findings
  absent from the report are waived (prevents stale inject after a clean or
  superseding verdict). Git `/review` reports do not touch sticky opens.
- Open findings inject into the next main-stream user prompt as DATA until
  addressed (checklist Enter → mark Core addressed on successful settle) or
  waived (`w` → Core waive).
- Sticky must not use `AgentStyle::CodeReview` as its identity; that style
  belongs to git `/review` only.
- Effectiveness evidence is mock/live verdict pipelines, not prompt-substring
  tests alone (those are Contract rows in the matrix).

## Desktop alignment

| Desktop Science Auto-review | Code TUI sticky `/reviewer` |
| --- | --- |
| Independent ReviewAgent LLM | Forked `LlmClient` + `StructuredAuxiliaryExecutor` |
| Durable Findings store | Core `session_review` / `SCENARIO_REPLY_TRANSCRIPT` |
| Address remediation turn | Checklist Enter → Address prompt → mark addressed on settle |
| Waive | Checklist `w` |
| Accept / reopen on re-review | Sticky capture reconciles addressed ids (accept cleared / reopen still-open); full auto Address→Accept host loop deferred |
| Inject authority | Submit refreshes projection from Core pending before DATA prefix |

## Triggers (Claude Science mapping)

| Claude Science | Code TUI |
| --- | --- |
| Auto-review after response | Sticky Reviewer mode after a successful main turn (arming gate) |
| Periodic during long work | Deferred (not required for Gate path) |
| Request review | Do not overload `/review`; optional reply re-run later |

Arming skips: non-Reviewer mode, deep-research, sleep, goal, missing evidence.

## Product arming contract (UX-R1)

Sticky Reviewer is a **mode the user arms** (`/reviewer` or Shift+Tab), not a
silent always-on auto-reviewer. That is intentional:

1. Claim↔record costs an independent LLM side-path; demand-driven arming is the
   efficiency contract.
2. Default Agent/Plan/Ask modes must not pay Reviewer latency or inject chrome.
3. Hermetic proof is the arming gate + finish/help copy + mock/live detect
   pipelines — **not** “session always had reviewer on.”

| Allowed | Forbidden (overfit) |
| --- | --- |
| User arms Reviewer → successful main turn → sticky lane may spawn | Claiming Effect because `/reviewer` appears in help text |
| Live `reviewer_claim_vs_record_detects_false` and `reviewer_git_review_names_planted_length_compare` under config `default_model` (DeepSeek Flash) | Soft-skip ignored live counted green |
| Help/status copy = claim↔record (not git `/review`) | Auto-arming every session to make a demo look “always on” |

Evidence: `sticky_arming_requires_reviewer_mode_and_evidence`,
`sticky_arming_skips_deep_research_sleep_and_goal`,
`slash_reviewer_help_is_claim_vs_record_not_git_review`, matrix R-live.
