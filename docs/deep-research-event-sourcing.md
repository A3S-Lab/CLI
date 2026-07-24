# DeepResearch Durable Evidence-First Runtime

Status: active for new CLI and TUI DeepResearch runs.

The active runtime is evidence-first and progressively publishable:

```text
exact-query bootstrap + optional bounded semantic outline
  -> exact closed candidate IDs
  -> bounded fetch and closed chunk-ID admission
  -> Host-owned typed source catalog
  -> source-backed or no-evidence Markdown and HTML
  -> optional typed claim graph with one transient retry
  -> evidence-compiler admission and atomic artifact replacement
```

The TUI `?` prefix and `a3s code research` use this same standalone
`a3s-deep-research` runtime. New runs do not select between research engines.
Historical Inquiry events remain inspectable in the generic run journal, but
the former sectioned-report executor is not present.

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
| Model | One bounded outline, closed candidate/chunk decisions over typed packets, and one typed claim-graph proposal with at most one transient retry |
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

Web acquisition starts immediately with the exact user query. In parallel, one
bounded outline may return zero to three supplemental plain-text queries. The
Host validates exact values, cardinality, and URL exclusion; it does not create
queries from keywords, dates, scripts, publishers, domains, or URL vocabulary.

Provider candidates enter a closed ID-based admission before fetch. A failed
semantic call may preserve a bounded transport fallback, but fallback bytes
remain audit-only until the closed chunk selector admits their exact IDs. An
explicit empty selection stays empty. Search title, snippet, rank, provider
date, engine name, hostname, and publisher never become evidence authority.

Only safely fetched and structurally bounded text admitted through exact source
and chunk IDs enters the Host source catalog. Invalid or failed sources cannot
erase valid siblings. Source text remains untrusted data throughout report
generation and rendering. Small catalogs use one closed selector. Larger
catalogs are partitioned into complete source-local JSON windows of at most
32 KiB and then reduced by exact chunk ID to at most four excerpts per source.
The partitioner uses only UTF-8 byte budgets and source identity.

## Progressive Publication

The Host selects one of four operational publication states:

| State | Meaning |
| --- | --- |
| `no_evidence` | No safe source catalog exists; the Host publishes a specific evidence boundary in both formats |
| `source_backed` | A bounded source catalog and source ledger are published, but no synthesized answer passed admission |
| `qualified` | At least one useful typed claim passed admission, but a material dimension remains explicitly bounded by a typed gap |
| `synthesized` | The typed claim graph passed exact support, graph, complete material coverage, and scope-depth gates |

These states describe artifact production. They do not claim that a research
request is epistemically complete.

When claim-eligible sources exist, the optional proposal receives only closed
dimension, source, and chunk IDs plus bounded excerpts. It has no tools and
cannot introduce a source URL. Rust admits facts, inferences, recommendations,
relations, and gaps independently. The compiler validates exact evidence,
basis, derivation, and contradiction edges; derives coverage; and rebuilds the
reader-facing citations and source ledger. Proposal failure, timeout, or
rejection leaves the already staged source-backed artifact intact.

Rust never matches claim prose to query or source prose. It validates closed
IDs, graph edges, provenance, cardinality, and budgets. Complete material
coverage yields `synthesized`; useful claims plus a material typed gap yield
`qualified`. Artifact class comes only from matching versioned markers in
Markdown and HTML, never from reader-facing words.

The engine publication envelope, version-2 receipt, and TUI event journal
retain the accepted relation, derivation, basis-edge, and gap counts. A
`qualified` terminal event therefore requires a persisted nonzero gap, while
source-backed and no-evidence states require all claim-graph counts to remain
zero. A focused synthesized report may contain one sufficient direct-answer
claim and zero findings; the adapter does not replace the scope-aware compiler
gate with a second prose-shape rule.

The final `ToolCallResult` intentionally omits bootstrap Dynamic Workflow
metadata. Workflow snapshot canonicalization treats completed output as
authoritative; exposing child acquisition metadata would therefore risk
replacing the Host publication projection with bootstrap output.

## Restart Invariants

A restart must preserve these invariants:

1. the run keeps its original wall-clock deadline;
2. a completed Flow effect is not executed again;
3. an ambiguous running effect reuses its stable identity and attempt number;
4. valid sibling effects survive a hanging, failed, or torn sibling append;
5. a source-backed artifact cannot be promoted to `qualified` or `synthesized`
   by replay metadata alone;
6. report-generation failure cannot erase fetched evidence; and
7. Markdown and HTML always describe the same operational publication state;
   and
8. typed graph counts survive receipt recovery and strict journal replay.

Process-level and workflow-store tests exercise planner, acquisition, source
selection, report proposal, and artifact boundaries. They verify stable
identity, valid-prefix recovery, at-least-once redelivery, sibling salvage, and
that replay cannot upgrade a degraded artifact.

## Legacy Journal Compatibility

Older journals may contain typed Inquiry objects and event names from the
retired report pipeline. The generic graph journal can still replay and inspect
those immutable records. No legacy planner, section writer, report resume
transaction, or publication adapter is retained to continue them.

New behavior and product documentation describe only the evidence-first path.
Historical records cannot authorize a selectable legacy engine.
