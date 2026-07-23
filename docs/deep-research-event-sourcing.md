# DeepResearch Durable Evidence-First Runtime

Status: active for new CLI and TUI DeepResearch runs.

The active runtime is evidence-first and progressively publishable:

```text
exact query + one deterministic outcome companion
  -> closed candidate admission or bounded acquisition fallback
  -> safe fetch, canonicalization, and source sanitization
  -> Host-owned source catalog
  -> deterministic source-backed Markdown and HTML
  -> optional closed report proposal with one transient retry
  -> Host block admission and atomic artifact replacement
```

The TUI `?` prefix and `a3s code research` use this same runtime. New runs do
not select between research engines. The older typed Inquiry and sectioned
report path remains only for existing-journal compatibility and focused
migration tests.

The publication architecture and product acceptance gates are defined in
[`deep-research-evidence-first-redesign.md`](deep-research-evidence-first-redesign.md)
and
[`deep-research-product-validation.md`](deep-research-product-validation.md).
This document describes the active durability and authority boundaries.

## Runtime Contract

Every run records a transient Loop Engineering contract with:

- `pattern = evidence-first-deep-research`;
- `controller = host_inquiry_reducer`;
- `quota.mode = bounded`;
- `execution.mode = progressively_publishable`;
- fixed acquisition, proposal, and finalization budgets; and
- fixed cardinality for optional model work.

The contract is runtime input, not a user-managed loop. It never creates a
`.a3s/loops/` asset and cannot inherit a user loop's iteration budget.

The DeepResearch state journal records one immutable wall-clock origin and the
Host budget. A restarted process derives its remaining deadline from that
origin. It cannot grant acquisition or proposal generation a fresh full budget.
Per-operation timeouts, output limits, catalog limits, and concurrency limits
remain narrower safety fuses inside the shared deadline.

## Authority Boundaries

| Authority | Owns |
| --- | --- |
| Host runtime | Exact query and evidence scope, time and cardinality budgets, safe source identity, fetched and sanitized text, publication status, artifact rendering, and terminal output |
| A3S Flow | Stable effect identities, event sequence, durable completion output, ambiguous-effect redelivery, and deterministic replay |
| Model | One closed candidate-admission proposal and, when eligible evidence exists, one closed report-block proposal with at most one transient retry |
| Versioned evaluation | Semantic quality, claim support, coverage, usefulness, language, and release admission |

Provider metadata, model output, and workflow status never become publication
authority. A valid schema proves structure, not truth or completeness.

## Durable State

DeepResearch uses three distinct durable surfaces.

### Run journal

The DeepResearch journal records the run creation event, immutable research
specification, shared deadline origin, and Host-owned checkpoints needed for
restart. It is authoritative for the run's operational identity and budget.

### Flow journals

Search, fetch, and structured-generation effects execute through stable Flow
run and step IDs. A completed effect is replayed from its journal. A running
effect whose external work may have completed before `StepCompleted` was
persisted is redelivered with the same attempt identity. This is explicit
at-least-once behavior; external tools must therefore tolerate an ambiguous
single redelivery.

The local Flow event store writes one JSON envelope per line, flushes it, and
syncs its data. On restart it:

- preserves a complete final envelope that is missing only its newline;
- returns the valid event prefix when the final unterminated bytes are a torn
  envelope;
- truncates only that torn tail before the next append; and
- rejects malformed newline-terminated records and interior corruption.

The local store is for one embedded writer. Shared multi-process writers use a
database-backed Flow store.

### Report artifacts

The Host owns `.a3s/research/<slug>/report.md` and `index.html`. It stages a
complete source-backed or no-evidence pair before optional report generation
can become a terminal risk. A synthesized pair replaces both artifacts only
after Host admission succeeds.

## Acquisition

Web acquisition starts immediately with the exact user query and one
Host-generated companion containing the run date and a fixed
outcome-and-news phrase. No model may generate, rewrite, translate, or retry an
additional provider query.

Provider candidates enter one closed semantic admission call before fetch. A
valid non-empty selection may be filled, within the existing source cap, only
with distinct-host institutional or accountable-publisher candidates. A real
admission failure uses a bounded deterministic fallback. An explicit empty
selection stays empty. Search title, snippet, rank, provider date, and engine
name remain discovery metadata and never enter the report as evidence.

Only safely fetched, canonicalized, sanitized, query-relevant text enters the
Host source catalog. Invalid or failed sources cannot erase valid siblings.
Source text remains untrusted data throughout report generation and rendering.

## Progressive Publication

The Host selects one of three operational publication states:

| State | Meaning |
| --- | --- |
| `no_evidence` | No safe source catalog exists; the Host publishes a specific evidence boundary in both formats |
| `source_backed` | A bounded source catalog and source ledger are published, but no synthesized answer passed admission |
| `synthesized` | The optional report proposal passed the Host's direct-answer, Findings, citation, language, atomic-claim, and strong-source gates |

These states describe artifact production. They do not claim that a research
request is epistemically complete.

When claim-eligible sources exist, the optional report proposal receives only
Host aliases and bounded source excerpts. It has no tools and cannot introduce
a source URL. Rust validates each report block independently, resolves its
citations against the closed catalog, removes invalid siblings, and rebuilds
the reader-facing source ledger from accepted blocks. Proposal failure,
timeout, or rejection leaves the already staged source-backed artifact intact.

The final `ToolCallResult` intentionally omits bootstrap Dynamic Workflow
metadata. Legacy canonicalization treats a workflow snapshot as authoritative
for that tool result; exposing child acquisition metadata would therefore risk
replacing the Host publication projection with the bootstrap output.

## Restart Invariants

A restart must preserve these invariants:

1. the run keeps its original wall-clock deadline;
2. a completed Flow effect is not executed again;
3. an ambiguous running effect reuses its stable identity and attempt number;
4. valid sibling effects survive a hanging, failed, or torn sibling append;
5. a source-backed artifact cannot be promoted to `synthesized` by replay
   metadata alone;
6. report-generation failure cannot erase fetched evidence; and
7. Markdown and HTML always describe the same operational publication state.

Process-level tests forcefully terminate workers after planner, retrieval,
section, frame, and semantic-audit effects cross their durable boundaries.
They verify stable journal identity, valid-prefix recovery, at-least-once
redelivery, and equality between resumed and uninterrupted projections.

## Legacy Journal Compatibility

Older journals may contain the typed Inquiry state machine, evidence ledger,
section drafts, semantic audits, and sectioned report transaction. Those types
and recovery paths remain readable while migration completes. They do not
authorize `quota.mode = unlimited`, `execution.mode = coverage_driven`, a new
multi-pass research route, or a selectable legacy engine for new requests.

New behavior and product documentation must describe the evidence-first path.
Compatibility tests may exercise the legacy reducer only when they explicitly
identify the old-journal boundary.
