# Reviewer mode (Code TUI)

Two separate products share the async `ReviewerLane`:

| Surface | Role |
| --- | --- |
| Sticky `/reviewer` (Shift+Tab) | **Reply verifier** — claim-vs-record on the latest assistant message vs user request + bounded turn tool evidence |
| Explicit `/review …` | **Git code review** — working-tree / commit / branch scope |

Hard rules:

- Reviewer work never enters the main turn queue or AgentEvent pump.
- Manual `/review` priority outranks sticky reply review; sticky jobs coalesce.
- Sticky review does **not** re-run tools or judge method choice beyond claim-vs-record.
- Incomplete evidence (truncated or non-terminal tools) must prefer `inconclusive` / `warn`.
- Open sticky findings inject into the next main-stream user prompt as DATA until addressed (checklist Enter) or waived (`w`).
- Sticky reply-verifier and git `/review` use **separate** host postures; a git review must not clear sticky open findings.
