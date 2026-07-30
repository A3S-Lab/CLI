# A3S Code Skill Optimization

Status: implemented for Evolution-managed local Skills in the Code TUI and
Code Web.

## Product Contract

A3S treats the reusable Skill summary and instruction list as trainable text.
The configured model remains unchanged. Optimization is an explicit background
workflow and adds no model calls to ordinary Code turns.

One run follows this lifecycle:

```text
queued
  -> isolated baseline replay on train + validation tasks
  -> bounded reflection on train tasks only
  -> candidate replay on validation tasks
  -> blind paired validation gate
  -> staged or rejected
  -> explicit adopt or dismiss
```

Adoption is never automatic. A staged proposal becomes a new immutable local
Skill version only after a TUI or Code Web approval. Existing Evolution
rollback restores an earlier version or the unmaterialized baseline.

## Evaluation Boundary

Each run uses between four and eight tasks. The default is four, split evenly
between training and held-out validation. A Code Web caller may provide tasks
and concrete rubrics; otherwise A3S generates cases from the bounded Skill
snapshot. If one caller-provided task declares a split, every task must declare
one and the set must contain at least two training and two validation tasks. Raw
memory evidence and workspace files are not added to this prompt.

The optimizer sees baseline outputs for training tasks and may propose at most
four edits. The default edit budget is three. Supported operations are:

- append one reusable instruction;
- replace one exact existing instruction; or
- delete one exact existing instruction.

The host rejects non-exact targets, duplicate or secret-shaped content,
oversized instructions, empty Skills, more than 16 instructions, and excessive
document growth. The model cannot bypass these deterministic limits.

Validation tasks are not included in reflection. A3S replays both the baseline
and candidate, deterministically blinds their A/B identities, and asks the
judge to score each output only against its rubric. A proposal is staged only
when:

1. its mean held-out score is strictly greater than the baseline; and
2. no held-out task regresses by more than 10 points.

Scores, edit rationales, digests, status transitions, and the final gate
decision remain in the local run record. Rollout text is intentionally not
persisted.

Each running record also carries the exact local optimizer-process identity.
Task cancellation marks the run failed before dropping it; after an unclean
process exit, the next status read detects the missing process and recovers the
record as failed. Interrupted work is never treated as a passing proposal and
can be retried with a new run.

## TUI

Run `/skill optimize` to open the shared Evolution surface. Select a Skill and
use:

| Key | Action |
| --- | --- |
| `t` | Start isolated replay, bounded reflection, and held-out evaluation |
| `a` | Adopt the newest staged proposal as a new local Skill version |
| `b` | Roll back the active Skill version through the existing recovery path |
| `x` | Reject the selected Evolution candidate |

The panel displays the latest run status, task/edit counts, baseline score,
candidate score, and improvement. The TUI stays responsive while model calls
run.

## Code Web

Open Memory, choose Learning, and select an Evolution-managed Skill. Its
`Skill optimization` card starts the same explicit background workflow and
polls queued or running work without blocking the rest of Code Web. Completed
runs show the baseline and candidate means, validation-gate reason, proposed
instructions, exact bounded edits, and held-out per-task scores. A passing run
remains staged until the user confirms adoption as a new immutable version;
rejected or failed runs remain inert and may be archived after review. Previous
runs remain selectable from the local run history.

## Code Web API

The routes use the existing `/api/v1/evolution` controller and the global Code
Web response envelope.

| Method and path | Behavior |
| --- | --- |
| `POST /{candidateId}/optimize` | Queue a background run; accepts `taskCount`, `editBudget`, and optional `tasks` |
| `GET /optimizations` | List local optimization runs, newest first |
| `GET /optimizations/{runId}` | Read the complete local run record |
| `POST /optimizations/{runId}/adopt` | Adopt a staged run, create a Skill version, and refresh affected Web sessions |
| `POST /optimizations/{runId}/dismiss` | Dismiss a staged, rejected, or failed run |

Example request with caller-owned cases:

```json
{
  "editBudget": 2,
  "tasks": [
    {
      "id": "focused-test",
      "prompt": "A parser change broke one fixture. Propose the verification order.",
      "rubric": "Starts with the smallest parser test and preserves the first diagnostic.",
      "split": "train"
    },
    {
      "id": "cross-crate",
      "prompt": "A shared type changed. Propose a bounded validation sequence.",
      "rubric": "Checks the owning crate before dependent packages.",
      "split": "train"
    },
    {
      "id": "held-out-cli",
      "prompt": "A CLI flag changed. Propose verification without running tools.",
      "rubric": "Names a focused CLI test before broad checks.",
      "split": "validation"
    },
    {
      "id": "held-out-regression",
      "prompt": "A previous failure disappeared after a retry. What should be retained?",
      "rubric": "Retains the first failing diagnostic and avoids claiming success early.",
      "split": "validation"
    }
  ]
}
```

## Storage And Privacy

Run records are atomic JSON files under
`.a3s/evolution/optimizations/`. Skill versions, snapshots, and recovery copies
continue to use the existing Evolution directories. Run IDs and candidate
digests prevent traversal and stale adoption; adoption fails if the Skill
changed after evaluation.

Optimization sends the bounded Skill snapshot, task prompts, rubrics, and
ephemeral rollout text to the currently configured model provider. It uses no
tools and grants no workspace access. Callers should still review custom tasks
before submitting sensitive text. Nothing is published to A3S OS or another
registry by this workflow.
