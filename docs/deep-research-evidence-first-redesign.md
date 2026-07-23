# DeepResearch Evidence-First Redesign

Status: implemented for new CLI and TUI DeepResearch runs. The active path uses
the exact query plus one deterministic outcome-and-news companion query for
immediate acquisition, semantically admits provider candidates before fetch,
adds only bounded institutional or accountable-publisher resilience to a
non-empty selection, stages a deterministic source-backed artifact, and then
attempts a closed report proposal. Broader model-generated query expansion, research-brief, and
feedback-loop experiments remain rejected because they multiplied
probabilistic semantic boundaries. Their retained evidence is documented in
[`deep-research-evidence-loop.md`](deep-research-evidence-loop.md). Corpus and
website evaluation remain release gates rather than runtime authority.

Product acceptance is governed by
[`deep-research-product-validation.md`](deep-research-product-validation.md).
This document records the active publication architecture and the evidence
that motivated it. A passing implementation test or durable workflow state is
not a substitute for the product gates.

The terminal-authority rule remains central: report-generation failure must not
erase fetched evidence, and the Host must own publication. Source identity and
conclusion safety are enforced before fetch when semantic admission succeeds,
and deterministically at publication for every path.

## Decision

DeepResearch terminal publication must require zero successful report-synthesis
calls. Once useful fetched source text exists, the Host can publish a traceable
Markdown report and equivalent HTML site without waiting for a planner,
extractor, writer, reviewer, repairer, or presentation model. Web acquisition
normally uses one closed semantic candidate-admission decision before fetch. A
failed admission may choose bounded fallback URLs for transport, but provider
metadata never becomes evidence. Fallback text carries explicit provenance and
must pass deterministic query relevance, protected-domain,
publisher-accountability, and publication gates. The active evidence-first
path does not run a second post-fetch semantic selector.

The normal path may make at most two attempts at one structured report proposal
over a closed, Host-owned source packet: an initial attempt and one retry for a
transient generation failure. This is an optional quality upgrade, not a
terminal dependency. It receives no tools and no source URLs. The Host resolves
source aliases, admits safe cited blocks, rebuilds the source ledger, and
renders both artifacts. Exhausted attempts or invalid output select the
deterministic source-backed report already prepared from the source catalog.

The following stages are not admitted to the candidate active path:

- model-authored planning before the original query is searched;
- mandatory model evidence extraction;
- per-source selection or per-question review generations;
- per-section writing;
- model semantic self-audit;
- model report repair, open-ended retries, or replay of a completed attempt; and
- model-authored editorial, guidance, or presentation frames.

Model-generated query expansion and gap-directed acquisition remain evaluation
hypotheses. The only active companion is Host-owned and deterministic: it adds
the run date plus `最新进展 最终结果 新闻` for a Han query, or the run date plus
`latest development final outcome news` otherwise. It cannot fan out, retry, or adapt.
Broader expansion does not enter the active path until a live-corpus comparison
shows that it converts material misses into supported answers under the same
latency and source budget.

## Evidence For The Decision

The retained multi-stage Tokio versus async-std run completed in 214.68 seconds
but used only three sources for maintenance, ecosystem compatibility, and
production selection. Its provisional quality score is 1.85 out of 4, and it
fails the reader-boundary and language gates. Earlier v26 and v33 runs show
larger model fan-out producing serial latency, late search, and recovery output.

The untuned one-generation frozen baseline produced these results with the
configured `openai/glm5.1-w4a8` model:

| Case | Model time | Outcome | Product evidence |
| --- | ---: | --- | --- |
| F01 contradiction | 283.69 s | Returned | Preserved both conflicting primary records and cited each one. |
| F02 derivation | about 300.15 s | Transport failure | No semantic result and no artifact; the connection closed before completion. |
| F05 prompt injection | about 300.07 s | Transport failure | No semantic result and no artifact; the connection closed before completion. |
| F04 Chinese report | 280.23 s | Returned | Core facts were correct, but raw source identity, English website chrome, one uncited claim, and missing semantic headings passed the syntactic Host check. |

F01 and F04 prove that one closed-evidence generation can produce useful
synthesis. F02 and F05 prove that even one call is not reliable enough to own
terminal publication. All four calls took roughly five minutes or reached the
observed upstream boundary. Reducing fan-out is necessary, but it is not a
fallback strategy.

F04 also proves that the current admission check measures the wrong thing. One
known citation token and an H1 yielded an empty violation list even though the
artifact failed citation recall, language, reader-boundary, and website gates.
More model review calls would not repair that contract error.

## Root-Cause Audit Of The Replaced Path

The legacy failure was not located in one prompt, timeout, reducer event, or
renderer branch. It was an authority error across the former control flow:

1. `execute_inquiry_pipeline` starts useful bootstrap acquisition immediately,
   but also starts an optional semantic planner and then commits the resulting
   plan into the required Inquiry contract.
2. A valid raw acquisition packet is persisted, but publication cannot consume
   that packet directly. The active path next requires one batched model
   extraction to convert raw source text into accepted evidence.
3. When extraction fails, `extraction_workflow_output` correctly preserves the
   raw acquisition, but produces no accepted evidence. The TUI interprets an
   empty accepted-evidence ledger as terminal degradation, while the CLI
   rejects any output without a reportable Outlining Inquiry.
4. When extraction succeeds, publication still requires the legacy sectioned
   report transaction. That transaction fans out through section writing,
   revision, editorial/guidance/presentation framing, semantic audit, and
   repair or resume paths before the Markdown/HTML pair becomes publishable.
5. The final recovery path can list source anchors, but it does not recover the
   fetched excerpts as the reader's result. It therefore replaces useful
   source text with generic workflow-oriented recovery prose.

The value-destroying transition is therefore:

```text
durable fetched source text
  -> mandatory model-authored evidence representation
  -> mandatory model-authored report transaction
  -> publishable artifact
```

Reducing the extraction to one batch did not remove either mandatory semantic
transition. It reduced call count while preserving the architectural failure:
model output still decides whether already-fetched evidence is user-visible.

The replacement must cut the dependency at the source catalog, not add another
recovery branch after it:

```text
durable fetched source text
  -> Host report document
  -> publishable Markdown/HTML

optional model proposal
  -> Host admission
  -> replace selected report blocks only when valid
```

This audit rejects the following as root-cause fixes:

- increasing a client timeout beyond the observed upstream connection boundary;
- retrying mandatory extraction, review, or repair, and retrying report
  generation without a fixed bound and an already-staged artifact;
- generating accepted-evidence placeholders after extraction failure;
- retaining the sectioned transaction but reducing its number of sections;
- adding another model reviewer to catch report defects; and
- improving recovery wording without publishing the preserved source text.

Each may alter a symptom, but none changes terminal authority.

## Candidate Runtime

```text
run created
  |
  |-- search the exact user query immediately
  |-- search one Host-owned date-aware outcome-and-news companion
  |-- fetch, canonicalize, bound, and persist source text
  v
durable Host source catalog
  |
  |-- stage deterministic extractive Markdown + HTML
  |-- strict exact-span outcome extraction
  |       `-- admitted direct answer + Findings -----------|
  |
  `-- otherwise optional closed report proposal (initial attempt + one retry)
          |-- failed or timed out ---------------------------|
          |-- invalid blocks removed by Host ----------------|
          `-- valid cited blocks retained                    |
                                                              v
Host-owned report document + source ledger
  |
  `-- atomic Markdown and localized HTML publication
```

The deterministic report exists before model synthesis becomes a terminal
risk. A process restart resumes from persisted source and artifact state; it
does not replay completed search, fetch, or generation effects.

## 1. Acquisition

The exact user query is sent to the configured search provider immediately.
Web scope also sends exactly one Host-owned companion containing the current
date and a fixed outcome-and-news phrase localized for Han queries, with an
English fallback for other scripts. Search never waits for semantic planning, and no
model may add, rewrite, or retry another query. Search-engine selection and
generic fallback follow `config.acl`; the default avoids AnySearch unless the
user opts in. For explicit competition-result intent, the Host first checks
whether the complete catalog already contains at least two accountable,
cross-host candidates with outcome-bearing retrieval metadata. If so, it
fetches at most four of those candidates and skips model URL admission. Other
catalogs are passed to one closed semantic candidate-admission attempt before
any web fetch. Its active generation is capped at 60 seconds inside a
150-second acquisition stage. A valid non-empty semantic selection keeps its
selected candidates and fills unused fetch capacity only with distinct-host
verified institutions or accountable publishers. This deterministic resilience
floor cannot add unknown, social, or protected-publisher lookalike hosts. A real
step failure or timeout degrades acquisition only to at most six deterministic
candidates.
Explicit seeds remain first; candidates unique to each query are then reserved,
while verified institutions and accountable publishers rank ahead of unknown,
social, or lookalike hosts before the distinct-host fill. Within the same trust
tier, discovery titles or snippets that offer a result, score, outcome, or
latest-state retrieval opportunity rank ahead of background-only candidates;
that metadata still never becomes report evidence. An explicit empty selection stays empty, and an
out-of-catalog ID is never accepted. Fallback text carries its acquisition mode
into the Host catalog and must pass deterministic query relevance,
protected-domain, publisher-accountability, and publication gates. The current
safe fetch, canonicalization, URL validation, and durable bootstrap checkpoint
are retained. PDF extraction may use three trusted ranges; ordinary HTML uses
the initial range plus at most one trusted continuation so hydration and
navigation tails cannot consume the closed chunk catalog.

Provider result titles, snippets, ranks, engine names, and dates remain
acquisition metadata. Only safely fetched, sanitized, query-relevant text
becomes report evidence. Script/style/noscript payloads, JavaScript placeholder
pages, navigation piles, escaped hydration data, high-density serialized
application state, template expressions, image syntax, and inline transport
URLs are removed before publication while visible source-link labels survive.
Sources that fail the claim
eligibility boundary remain auditable in a degraded source view but carry an
explicit, visually distinct `not eligible for conclusions` warning. The Rust
publication boundary revalidates restored catalogs that did not carry
semantic-admission authority. Every source record owns:

- a Host-generated stable identity;
- requested and canonical anchors;
- reader-facing title;
- capture time and acquisition provenance;
- bounded immutable text chunks; and
- a content digest.

Successful siblings survive search or fetch failures. Source persistence is
not conditional on a later model assigning a semantic role or producing an
accepted claim. Community and streaming hosts, plus self-publishing pages whose
disclaimer assigns views only to the author or describes the platform as
storage, remain visible as bounded evidence when useful but are ineligible to
support report conclusions.

The active path uses only the original query and the one fixed outcome-and-news
companion. Broader query expansion is tested separately so any coverage
improvement and latency cost remain visible.

## 2. Deterministic Extractive Report

For every non-empty fetched source catalog, the Host stages a complete report
without model output. It contains:

- the user query as the bounded title or subject;
- a localized explanation that synthesis did not complete when fallback is
  selected;
- source-grouped verbatim excerpts selected by query overlap and document
  position;
- exact canonical source links;
- explicit acquisition and evidence limitations; and
- a deduplicated source ledger.

Fetched text is escaped as untrusted data. Instructions, Markdown, or HTML
inside a source cannot alter the report template. The fallback never prints
workflow statuses, packet IDs, model errors, hashes, local journal paths, or
retry instructions.

A source-backed extractive artifact is explicitly `degraded`: it retains useful
evidence but has no admitted direct answer, Findings, claims, or citations. It
must never be promoted to `qualified` or synthesized success merely because
sources were fetched. A run with no safely publishable evidence uses a separate
localized no-evidence boundary artifact.

This fallback is intentionally less polished than a valid synthesized report.
Its purpose is to preserve user value and make model failure non-destructive,
not to claim that excerpts are equivalent to analysis.

For explicit result, winner, champion, score, or standings intent, the Host may
compile a separate synthesized report directly from exact retained source
spans before calling the report model. This is not a promotion of the staged
source snapshot. The compiler splits topic pages at sentence and navigation
boundaries, requires an assertive atomic outcome for the direct answer, and
requires distinct claim-like Findings. It rejects questions, prospective or
schedule prose, generic result indexes, navigation piles, title lists, and a
bare `champion` token without an outcome predicate. Betting odds, predictions,
historical roundups, and time-only score widgets are also ineligible. Findings
remain on the selected direct-answer source unless another accountable source
contains exactly one matching score and at least two matching non-generic event
identity features. This admits bounded independent detail while preventing an
equal score from joining unrelated events. A title that only repeats the
selected outcome does not qualify as a distinct Finding. Direct-answer ranking
prefers complete scores and win/loss assertions; Findings ranking prefers
concrete numeric and aftermath facts. Candidate spans still pass the ordinary
current-date, query-language, numeric-literal, direct-answer, citation, and
strong-source admission gates. Only a verified institution or explicitly
accountable publisher can support this path. Success records
`synthesis_mode = deterministic_outcome_extract`, requires zero report-model
generations, and publishes through the same atomic Markdown and HTML boundary.
Failure changes no state classification and falls through to the closed report
proposal.

## 3. Optional Closed Report Generation

One structured report proposal receives:

- the exact query and query language;
- bounded titles and the highest-ranked readable chunk from each source that
  already passed deterministic claim eligibility;
- the count, but not the content, of excluded ineligible sources;
- opaque source aliases, but no URLs; and
- a small report contract.

It receives no tools. Source content is explicitly untrusted evidence data.
The model may cite only exact Host aliases in structured `source_aliases`
arrays and may not author a source ledger URL.

Each attempt has a 90-second active bound. A transient stream or generation
failure may retry once under the same closed evidence and durable workflow
identity. There is no validation-driven rewrite, reviewer, repair wave, or
third attempt. When the second attempt fails, times out, or returns no
publishable proposal, the Host keeps the source-backed artifact that was
already staged.

## 4. Host Admission And Salvage

The Host treats the model response as an untrusted proposal. It applies these
rules without another model call:

1. Reject every model-authored URL and unknown alias.
2. Reject a raw alias appearing outside an exact citation token.
3. Ignore the model's source ledger and rebuild it from cited catalog entries.
4. Require every model-authored prose paragraph and list item to contain at
   least one valid source token; remove an uncited block instead of rejecting
   valid siblings.
5. Resolve adjacent aliases to separated numeric citations and one deduplicated
   source ledger.
6. Reject internal terminology, prompt text, runtime diagnostics, and control
   characters from reader-facing content.
7. Enforce the query language for prose and select Host-owned localized labels.
8. Reject a new date or numerical literal absent from the cited excerpts. For
   Summary and Findings, every individual cited source must contain every date
   and number in the atomic block; several partial sources cannot be stitched
   into one claim.
9. Require each core Summary or Findings block to have one complete verified
   institutional source or one explicitly accountable publisher that establishes
   the complete block. Prefer independent corroboration when available, but do
   not add an unrelated citation merely to increase source count.
10. For current-result and status queries, reject an explicit stage snapshot
    that is more than seven days behind the freshest eligible retained source.
11. For competition-result intent, reject a Summary that supplies only dates,
    format, participants, or other background without an outcome or score.
12. Use a fixed Host section structure when the model omits or damages Markdown
   headings. Model formatting never controls website navigation or evidence
   counts.

If no useful model block survives, the Host publishes the staged extractive
report. If some blocks survive, it combines only those blocks with Host-owned
limitations and the source ledger. It does not ask the same model to audit or
repair itself.

A normal synthesized publication additionally requires at least one direct
answer block, one distinct Findings block, two admitted cited claim blocks, one
cited eligible source, and closed citations for every admitted claim. An
ineligible source retained for audit cannot support a claim, but its presence
does not poison valid claims from eligible sources. Source titles do not count
as Findings. A failure at this final Host gate cannot be relabeled as qualified
success; it remains the explicitly degraded source-backed artifact.

Claim entailment cannot be proved by token presence. Corpus evaluation remains
the release authority for semantic citation precision. Production self-review
is not admitted merely to create the appearance of semantic certainty.

## 5. Deterministic Website

Markdown and HTML consume the same Host report document. The renderer owns:

- localized titles, evidence labels, limitations, and navigation;
- actual cited-source and admitted-Findings counts, excluding source headings;
- a truthful read-time label, including a one-minute or sub-minute report;
- numeric inline citations, densely numbered in first-citation order from only
  the sources actually used, with visible `[n]` labels;
- one matching deduplicated linked source ledger without a second list number;
- semantic H1-to-H2 heading order;
- focus states, overflow handling, narrow-screen layout, and print styles; and
- a fixed reading design until visual evaluation justifies variation.

No art-director generation is permitted. F01 and F04 already show that content
shape must not drive unvalidated counters or English-only renderer chrome.

## Budgets

The product-validation contract owns release latency thresholds. The active
path adds these architectural ceilings:

| Resource | Active ceiling | Rule |
| --- | ---: | --- |
| Required report-synthesis generations | 0 | A fetched catalog can publish without a successful report proposal. |
| Report proposal attempts | 2 maximum | One initial closed-evidence proposal and at most one transient retry. |
| Query-expansion generations | 0 | No model may generate or adapt a search query. |
| Semantic audit or repair generations | 0 | Host salvage replaces model self-review. |
| Original-query search | 1 | Starts immediately. |
| Outcome companion search | 1 | Uses the run date plus one fixed localized outcome-and-news phrase. |
| Fetched sources | 8 | Shared deterministic acquisition budget. |
| Bootstrap acquisition stage | 150 seconds | Covers discovery, one source-admission attempt, and actual fetches. |
| Web source-admission active time | 60 seconds | Explicit competition outcomes skip this model step when discovery already contains at least two accountable cross-host outcome candidates; otherwise the Flow step is not retried, and failure degrades acquisition only while leaving room for bounded HTML range fetches. |
| Report-model active time | 90 seconds per attempt | The two-attempt stage and whole Host run remain independently bounded. |

Model admission wait, active generation, search, fetch, first-source
persistence, fallback readiness, and terminal publication are timed separately.

## Failure Semantics

| Failure | Required behavior |
| --- | --- |
| Search provider fails | Publish an honest localized no-evidence artifact. |
| Web source admission fails or times out | Fetch at most six Host-ranked cross-query, distinct-host fallback candidates; record fallback provenance and require deterministic query, protected-domain, publisher-accountability, and publication gates. |
| One search or fetch sibling fails | Preserve successful sources and continue. |
| First report attempt fails transiently | Retry once with the same closed evidence and durable identity. |
| Second report attempt fails, times out, or is invalid | Publish the explicitly degraded staged source-backed report. |
| One report block is uncited or malformed | Remove that block and retain valid siblings. |
| All report blocks are invalid | Publish the staged extractive report. |
| HTML enhancement fails | Publish the same report through a deterministic minimal HTML document. |
| Publication is interrupted | Recover the atomic pair from staged state without repeating external effects. |

## State And Authority

The durable source catalog and Host report document are authoritative. Model
plans, extractions, reviews, and prose are proposals, not state authority.

New runs need only record:

- run identity, query, language, budgets, and start time;
- search and fetch attempts;
- immutable source records and failure siblings;
- at most two report attempts and their bounded terminal reasons;
- admitted report blocks and cited source identities; and
- staged and committed Markdown/HTML artifacts.

Legacy Inquiry journals remain readable for recovery. Their event vocabulary
does not justify keeping the legacy pipeline active for new runs.

## Active Path And Compatibility

Retain:

- bootstrap search and fetch transports;
- URL safety, canonicalization, and bounded source persistence;
- exact source/excerpt restoration;
- atomic artifact-pair publication;
- CLI/TUI result handoff and browser opening; and
- legacy journal replay needed for already-created runs.

The new-run control flow no longer uses:

- mandatory batched evidence extraction;
- accepted-evidence gating before raw sources can be reported;
- section generation and section revision;
- editorial, guidance, and presentation-frame generations;
- semantic-audit waves and model repair waves;
- report resume that repeats the same provider risk; and
- generic recovery prose when fetched source text exists.

Do not delete compatibility code before old-journal tests identify what replay
still needs. Legacy replay must remain explicitly isolated; dead wrappers can
be removed only when their recovery contracts are no longer exercised.

The active seam is the persisted bootstrap source catalog. New-run control flow
parses that catalog into a Host-owned report document before invoking the
report model. CLI and TUI settlement consume the resulting artifact outcome
directly; they do not infer artifact availability from Inquiry convergence or
an accepted-evidence count. Legacy Inquiry state remains an input only when
replaying an already-created journal.

## Next Decision Gates

Before release promotion:

1. run the live corpus for C01, C02, C05, C07, and C08 with persisted failures;
2. verify F01, F02, F04, F05, F06, and F08 against the current Host admission
   and source-backed degradation rules;
3. measure direct-answer, Findings, citation, source-relevance, and latency
   gates from the terminal artifact rather than workflow completion;
4. compare the exact-plus-fixed-companion baseline with broader query expansion
   before admitting any extra search generation; and
5. pass real desktop and 390-pixel website inspection for synthesized,
   source-backed, and no-evidence artifacts.

The production architecture is selected; release readiness remains governed
by the corpus and visual gates above.
