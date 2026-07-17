# DeepResearch Event-Sourced Runtime

DeepResearch uses one append-only domain journal to make collection, evidence,
convergence, report publication, and TUI cleanup replayable.

## Authority boundaries

- A3S Flow events are authoritative for workflow and step execution.
- `AgentEvent` is authoritative for tools, child agents, and session lifecycle.
- The typed Inquiry event stream is authoritative for research strategy,
  obligations, questions, evidence acceptance, contract assessment, outline,
  drafts, and report audit state.
- The DeepResearch journal normalizes references to those events.
- `GraphRuntime` materializes research-domain objects and relations.
- The TUI, Markdown, and HTML reports are disposable projections.

The graph is not another effect scheduler. Flow remains responsible for all
external effects.

## Storage

Runtime state is stored below:

```text
.a3s/research/runs/events/
.a3s/research/runs/checkpoints/
```

The implementation must not create `.a3s-flow/` or another peer runtime root.
`FileGraphEventStore` provides atomic replacement and `save_if_head` protects
against concurrent writers. Normalized sources retry a bounded number of times
after reloading a conflicting head.

Checkpoints are bounded, atomic indexes rather than a second source of truth.
They contain the run projection, event head/count, and graph dimensions. A
checkpoint is accepted only when it matches a freshly loaded strict event
generation; corruption or mismatch falls back to scanning and replaying the
event logs.

Domain payloads are bounded by depth, collection width, and string length.
Large raw tool or browser output must remain outside the graph; normalized
events retain only compact facts and safe diagnostics.

## Domain projection

The run projection contains its terminal grade, active steps and children,
accepted evidence/source/claim counts, convergence reason, report audit result,
and artifact evidence head.

The graph additionally materializes:

- `deep_research.source`
- `deep_research.evidence`
- `deep_research.claim`
- `deep_research.obligation`
- `deep_research.stop_condition`
- `deep_research.perspective`
- `deep_research.question`
- `deep_research.outline_section`
- `deep_research.section_draft`
- `deep_research.observed_in`
- `deep_research.supports`

Only evidence with at least one traceable source enters this graph. Report
synthesis consumes the accepted evidence projection rather than raw workflow or
tool output.

## Invariants

1. A terminal run cannot schedule or accept ordinary new work.
2. Duplicate external events are idempotent.
3. A stale graph generation cannot overwrite a newer head.
4. Terminal grades are monotonic: `Failed < Degraded < Qualified < Completed`.
5. Report artifacts are published only after claim/source audit succeeds.
6. A failed report audit downgrades the run and cannot produce an artifact head.
7. A terminal projection clears active steps, children, pinned plan, and footer
   state before restoring autonomous mode.
8. Retrieval stops at the finalization reserve, the round limit, or after two
   rounds without material evidence gain.
9. A first collection round does not trigger a generic corroboration round, but
   it may finalize only after satisfying the query-specific coverage gate.
   Another round is scheduled for explicit unresolved gaps or contradictions,
   and the typed host policy still owns the terminal grade.
10. Sufficiency is query-relative. A narrow factual lookup may finalize from a
    small corroborated evidence set, while a broad investigation must satisfy
    the LLM plan's independent coverage criteria. No subject-specific gate or
    report template may replace that semantic decision.
11. A host-managed Inquiry contains exactly one stable research-contract event
    and exactly one contract assessment before outlining. Removing both an
    event and its projected state never restores legacy completion authority.
12. Report recovery accepts only a journal that extends the workflow's exact
    Inquiry prefix. Outline, section, failed-audit revision, and completed
    boundaries are replayed strictly; stale or divergent prefixes fail closed.

## Diagnostics

The TUI exposes read-only diagnostics backed by strict replay:

```text
/research status [run-id]
/research explain [run-id]
/research replay [run-id]
/research diff <left-run-id> <right-run-id>
```

Without a run ID, an active run is selected first; otherwise the latest local
journal is used. `explain` reports convergence and report-audit reasons.
`replay` reports the verified event count, graph shape, and current event head.
`diff` compares two strictly replayed graphs and groups added, removed, and
changed objects and relations by semantic type.

## Event-point forks

Forks are reserved for evidence-bearing alternatives, not retries. When an
accepted evidence package contains explicit contradictions, DeepResearch forks
the current event head into an isolated `contradiction-review` branch and
projects only the contradictory evidence there. The main branch remains
unchanged until its ordinary accepted-evidence event is committed.

Branch stores have independent compare-and-swap heads and never overwrite the
primary run checkpoint. A fork must start at a valid strict-replay boundary and
must contain validated evidence; empty or source-free strategy forks are
rejected.

## Restart reconciliation

The frozen research specification records the host PID as a local operation
lease. On TUI startup, the latest active run is reconciled only when that host
process no longer exists. A live host is never modified.

For an interrupted run, the journal is compared with the resumed session's
subagent tracker. Still-running children are cancelled through the session API;
missing children are classified as orphaned. The reconciliation and its reason
are appended before the old run is terminalized as failed. Completed tool calls
are not replayed or executed again.

Within an active run, Inquiry checkpoints are committed after logical
transactions rather than after each evidence item. Report generation reloads
that prefix before doing model work: a committed outline is never regenerated,
existing section drafts are reused, and A3S Flow journals resume stable
parallel section and frame runs. A failed full-report audit and its replacement
section drafts are persisted as one authoritative prefix, so interruption
cannot expose a half-revised report state.

## Migration rule

New DeepResearch state must be added as a domain event and a projection field.
Do not add another mutable TUI latch when the value can be derived from the
journal. Embedded workflow JavaScript may collect evidence and translate
explicit gaps into a bounded follow-up step, but terminal convergence and
publication policy belong in typed Rust code.

The planner first owns one explicit semantic execution route:
`direct_only`, `direct_then_review`, `direct_then_maker`, or `maker_first`. The route is part of the
validated plan event and is never reconstructed from the topic, query length,
answer shape, or track count. `direct_then_review` lets bounded multi-query
retrieval feed one structured synthesis-and-coverage review, so a substantive
public investigation does not pay for separate no-tool maker and checker model
turns. `direct_then_maker` is reserved for investigations that need adaptive
source reading or multi-step evidence collection after direct retrieval has
seeded the source set.

The same semantic plan chooses `focused` or `perspective_guided` research.
Focused work avoids a perspective-discovery tax for bounded questions.
Perspective-guided work first scouts the source landscape, derives two to four
viewpoints only from retained scout sources, and then asks bounded follow-up
questions linked to stable obligations. This preserves the useful
perspective-guided inquiry mechanism without a fixed expert roster, topic
classifier, or domain-specific template.

The checker owns the semantic routing decision for each unresolved gap after
the planned route. A
single externally retrievable fact is expressed as focused search queries or
stable seed URLs and runs as a uniquely identified direct-retrieval follow-up.
Public benchmarks, maintenance facts, source excerpts, and migration documents
also remain direct-retrieval work; only actual evidence production or required
local/non-web work is delegated to a maker.
Follow-up evidence is accumulated before the next check; a continuation with no
actionable in-budget work terminalizes as degraded instead of replaying or
silently escalating.

If the checker itself times out after a maker has already produced traceable,
schema-valid evidence, the terminal projection records verification as
degraded without inventing a checker decision. Publication derives a
provisional report directly from that accepted evidence and states the missing
independent verification as a limitation. Recovery remains reserved for runs
that do not retain reportable evidence.

For public evidence, the initial direct step is itself a Loop maker. It uses the
A3S Code 5.3.2 capability contract and `batch` tool to run independent searches,
then independent fetches, concurrently while retaining nested source anchors
and typed partial failures. Query-specific candidates receive fetch capacity
before seed URLs; an unconfigured zero-result search has one bounded Brave
fallback. `direct_only` and `direct_then_review` send that evidence to the
checker, with the latter requiring cross-track synthesis in the same structured
turn. This preserves semantic depth without treating parallel subagents as the
default transport for ordinary web retrieval.

The bundled workflow source is kept outside `tui/mod.rs` and compiled with
`include_str!` from bounded JavaScript fragments under
`deep_research/workflow/`. This keeps the Rust state machine reviewable and
prevents the TUI from treating the full implementation as its semantic intent.
The JavaScript is a collection adapter; host convergence, evidence acceptance,
and publication decisions remain typed Rust policies.
