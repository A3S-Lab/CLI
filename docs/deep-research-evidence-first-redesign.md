# DeepResearch Evidence-First Redesign

Status: proposed architecture, not yet implemented.

This document records the redesign decision before further runtime changes. It
does not describe the behavior of the current implementation. The existing
runtime remains plan-first until the migration described below is complete.

## Decision

DeepResearch will become an evidence-first, progressively publishable pipeline.
A complete model-authored research plan will no longer be a prerequisite for
search, fetch, evidence preservation, or a qualified report.

The active path will use at most four structured model generations during a
normal run and at most five when one bounded report repair is necessary:

1. optional research outline and query expansion;
2. batched evidence extraction for the initial source catalog;
3. optional incremental evidence extraction after one gap-directed pass;
4. complete report generation; and
5. at most one report repair.

Search, fetch, source validation, coverage reduction, report validation, and
HTML rendering remain Host-owned. They do not consume model-generation slots.

## Evidence Behind The Decision

The current failure is architectural rather than a missing timeout or prompt
instruction.

- The production endpoint answers a minimal completion in about two seconds,
  but complex structured planning calls have taken between roughly one and
  eight minutes and have repeatedly reached four- or eight-minute fuses.
- The endpoint reports a structured-generation admission concurrency of one.
  Logical fan-out therefore becomes a serial queue even when the Host schedules
  independent units concurrently.
- The v33 acceptance run spent 491 seconds in one outline and three track-detail
  generations. A fourth detail generation was still running. Search had not
  started and no source had been preserved.
- The v26 run completed 28 durable model workflows over approximately 94
  minutes. It fetched seven traceable sources but still published only a
  recovery report after repeated semantic audit and repair failed.
- The model has ignored explicit planning constraints: it introduced
  unrequested time or count thresholds, changed the working language, combined
  distinct evidence targets, and treated retrieval-contract writing as factual
  research.
- Earlier reports exposed the opposite failure mode: a report could be
  generated, but it mixed languages, contained damaged headings, derived
  release frequency from dates, and generalized secondary commentary beyond
  the fetched evidence.

Splitting the planner, adding retries, and adding self-review calls amplified
latency and failure probability without changing these facts. The same model
currently acts as planner, selector, resolver, writer, critic, and presentation
designer. Repeated calls to that model are correlated, not independent quality
checks.

## Product Outcomes

A successful redesign must produce all of the following:

- source acquisition begins without waiting for semantic planning;
- every successfully fetched source is durably preserved before later model
  work can fail;
- failure of one target, source, extraction item, or report section does not
  discard valid siblings;
- a useful evidence-backed report is published as `qualified` when some
  requested dimensions remain bounded;
- `degraded` is reserved for runs with no publishable evidence or no safe
  artifact, not for a single missing target;
- every cited URL and factual report block is traceable to the accepted source
  catalog;
- Markdown and a responsive HTML reading site are produced from the same
  validated report document; and
- latency and reliability are measured across a query corpus rather than one
  repeatedly tuned example.

## Runtime Shape

```text
run created
  |-- bootstrap search with the original query -------------------|
  |-- optional outline and query expansion (bounded, fail-soft) --|
  |                                                               v
  |<-- durable raw source catalog <--- deterministic admission + fetch
  |                                                               |
  |<-- model outline or Host fallback contract -------------------|
  v
one batched evidence extraction
  |
Host coverage reduction
  |-- enough evidence -------------------------------|
  `-- material gaps + remaining budget               |
        one gap-directed search/fetch/extraction ----|
                                                     v
one complete structured report generation
  |
deterministic provenance and artifact validation
  |-- valid -----------------------------------------|
  `-- invalid + repair budget -> one repair ----------|
                                                     v
atomic Markdown + HTML publication
```

### 1. Bootstrap acquisition

The exact user query is sent to the search provider immediately. Search and
fetch do not wait for the outline model.

Candidate admission is deterministic and may use provider rank and the query
that discovered a candidate only as acquisition metadata. Rank, title, URL,
snippet, and provider date never become report evidence. Candidates are
canonicalized, safety-checked, deduplicated, and admitted round-robin across
queries so one expanded query cannot consume every fetch slot.

The first fetch cohort is persisted as a raw source catalog. A later planning,
extraction, or synthesis failure cannot erase it.

### 2. Optional outline

One model call may return:

- a report title;
- stable evidence-target identities and titles;
- whether each target is material;
- at most one provider query per target; and
- freshness intent.

It does not author completion criteria, evidence-quality quotas, stop
conditions, recommendations, retrieval budgets, or report prose. Those fields
made planning large, slow, and brittle without improving acquisition.

The call has one durable identity and one attempt. Timeout, malformed output,
or constraint violation selects a Host fallback contract containing one
material target whose scope is the original query. The fallback uses the
bootstrap source catalog and can still produce a qualified report.

Expanded queries that arrive in time use only the unspent search budget. They
never delay or invalidate the bootstrap search.

### 3. Evidence extraction

Fetched text is converted into a bounded, immutable source packet. The Host
selects packet ranges using document structure, source-local position, and
query overlap. This selection affects context size only; it never establishes
a fact or source role.

One structured generation receives all targets and the bounded source packet.
It returns independently decodable target entries containing:

- exact source and excerpt references;
- supported findings;
- explicit contradictions;
- explicit evidence gaps; and
- source-role judgments needed by the target.

The Host validates every target entry independently. An invalid target entry is
bounded without rejecting valid siblings. Exact excerpt text is restored from
the immutable Host catalog, so the model cannot invent a quotation or URL.

There is no separate per-source selector and no separate question-review
generation. Those stages duplicate semantic work and become serial call fans
on a single-flight model.

### 4. Coverage and one gap-directed pass

The Host reduces target coverage from accepted evidence. Targets have one of
four states:

- `covered`: enough traceable evidence supports a useful answer;
- `partial`: traceable findings exist with a consequential limitation;
- `uncovered`: no accepted finding supports the target; or
- `supporting_missing`: an optional target has no accepted finding.

Only a material `partial` or `uncovered` target can authorize the single
gap-directed pass. The pass may issue one query for each affected target within
the remaining global search and fetch budgets. Existing sources and findings
remain immutable. New sources are extracted once and merged by stable identity.

Recommendations are derived during report synthesis from accepted findings.
They are not independent retrieval targets. HTTP-library compatibility and
database-library compatibility are distinct targets because they are published
by different source families.

### 5. Report document

One generation produces a complete typed report document rather than one model
call per section. The document contains:

- localized title and executive answer;
- ordered sections made of factual blocks;
- explicit evidence references for every factual block;
- evidence-bounded recommendations with their basis references;
- target-level limitations; and
- reader-facing labels needed by the renderer.

The Host, not the model, converts evidence references to exact source links.
URLs outside the accepted catalog are impossible to publish.

Deterministic validation checks at least:

- every factual block has one or more valid evidence references;
- every cited source belongs to the referenced finding;
- numerical and date literals are present in cited excerpts unless the block
  explicitly carries a validated derivation;
- no internal IDs, packets, workflow diagnostics, or model instructions leak;
- title, headings, labels, and narrative use the query language, excluding
  source-defined names and quotations;
- absence claims correspond to an explicit target gap;
- recommendations identify both their evidence basis and boundary; and
- the document contains no model-authored URL.

Validation issues authorize at most one repair call over the same closed
evidence. If repaired output still has invalid blocks, the Host removes those
blocks, marks the affected target bounded, and publishes the remaining report
as `qualified`. One bad paragraph must not turn a seven-source run into a
generic recovery report.

### 6. Website rendering

HTML presentation is deterministic and consumes the validated report document.
It does not require an art-director model call.

The initial renderer uses one polished, responsive reading layout with:

- executive answer and completion badge;
- material-target coverage summary;
- report sections and evidence-bound recommendations;
- explicit limitations;
- a deduplicated source ledger; and
- accessible typography, focus states, narrow-screen behavior, and print
  styles.

Markdown and HTML are written atomically from the same document. RemoteUI opens
only the committed HTML path. Presentation variation can be added later only
after the research path meets its reliability and quality gates.

## Budgets

Budgets are global run ceilings rather than model-authored promises.

| Resource | Normal ceiling | Rule |
| --- | ---: | --- |
| Structured model generations | 4 | Outline, initial extraction, report, and optional gap extraction |
| Report repair generations | 1 | Only deterministic validation can authorize it |
| Search queries | 6 | Bootstrap query plus target expansion or gap queries |
| Fetched sources | 12 | Shared by initial and gap-directed acquisition |
| Retrieval passes | 2 | Bootstrap/expanded acquisition and at most one gap pass |
| Full-run wall clock | 25 minutes | Includes model admission wait |
| Time to first persisted raw source | 90 seconds at p95 | Measured separately from report completion |

The ceilings are safety limits, not required counts. A narrow query should stop
earlier. A broad query that cannot fit is published with explicit bounded
targets rather than silently compressing unrelated publishers into one target.

## Failure Semantics

| Failure | Required behavior |
| --- | --- |
| Outline timeout or invalid schema | Adopt the Host fallback contract; continue with bootstrap evidence |
| One search failure | Preserve other query results and continue |
| One fetch failure | Preserve successful sources; use an unspent or gap-pass slot when available |
| One invalid extraction target | Bound only that target; accept valid sibling entries |
| Gap pass timeout | Preserve the initial evidence catalog and continue as qualified |
| Report generation failure | Retry once with the same evidence and precise validation context |
| Invalid report block after repair | Remove the block, bound its target, and publish the valid remainder |
| HTML rendering failure | Preserve Markdown and emit a deterministic minimal reading page |
| No accepted evidence | Publish a diagnostic recovery artifact and classify the run as degraded |

## State And Durability

The active run journal must represent acquisition before planning. At minimum
it records:

- run identity, query, language, start time, and global budgets;
- optional-outline status and fallback reason;
- every search attempt and provider query identity;
- raw source identities, fetch outcomes, and immutable content digests;
- accepted findings and exact source/excerpt relationships;
- target coverage transitions;
- report generation and repair attempts; and
- Markdown/HTML publication commit.

Completed effects reuse their exact durable inputs after process restart. A
restart cannot grant another search pass, model generation, repair, or fresh
wall-clock budget. Legacy Inquiry journals remain readable, but new runs use
the evidence-first event order.

## Acceptance Corpus

One successful demonstration query is not a release gate. The benchmark corpus
must include at least these classes:

1. a broad, current technical comparison with multiple independent publishers;
2. a narrow current fact requiring one primary source;
3. a regulation or policy question requiring primary and secondary evidence;
4. a product or market comparison;
5. a local-repository architecture question;
6. a mixed local and public evidence question;
7. an ambiguous request that must remain explicitly bounded;
8. a query for which authoritative evidence is intentionally unavailable;
9. Chinese, English, and mixed-name queries;
10. injected search, fetch, extraction, and report-generation failures;
11. a long query whose requested dimensions exceed the normal target budget;
12. a restart during acquisition, extraction, synthesis, and publication.

The final gate runs each semantic query at least three times against the
supported default model configuration. Fault-injection cases run
deterministically in CI.

## Completion Gates

The redesign is not complete until current-state evidence proves all of the
following:

- at least 90% of real-model corpus runs finish as `completed` or `qualified`;
- no more than 5% finish as `degraded`, and no run with accepted traceable
  evidence degrades solely because another target or report block failed;
- 100% of terminal runs produce readable Markdown and HTML artifacts;
- 100% of published citations resolve to the accepted source catalog;
- 100% of factual blocks carry accepted evidence references;
- unsupported or untraceable URLs, numerical literals, and date literals are
  rejected or removed before publication;
- query-language consistency passes for all reader-facing fields in the
  multilingual corpus;
- p95 first-source persistence is at most 90 seconds under a healthy search and
  fetch provider;
- p95 terminal latency is at most 25 minutes on the supported default model;
- normal runs use no more than four structured generations and repaired runs
  no more than five; and
- the generated site passes artifact validation, link checks, narrow-screen
  rendering, and visual inspection for every corpus report shape.

## Migration Map

Retain and adapt:

- safe search and fetch transports, canonical URL handling, and bounded retry;
- immutable source identities and exact excerpt restoration;
- durable Flow identities and process-restart reconciliation;
- evidence/source ledger validation;
- atomic artifact publication and RemoteUI integration; and
- legacy journal replay.

Rewrite for the active path:

- plan-first `execute_inquiry_pipeline` ordering;
- planner schema and deadline ownership;
- candidate admission and evidence extraction;
- target coverage state and qualified partial-success semantics;
- report generation, validation, and salvage; and
- the report HTML data contract.

Remove from the active path after compatibility tests pass:

- outline plus per-track plus retrieval planner fan-out;
- per-source selector and source-reduction model calls;
- separate obligation-review model calls;
- per-section generation and semantic self-audit fan-out;
- repeated editorial, guidance, and presentation frame calls; and
- any rule that turns one failed model subcall into zero retained evidence.

The current experimental planner split should not be promoted. Its tests remain
useful only as evidence for durable effect reuse until the evidence-first tests
replace them.
