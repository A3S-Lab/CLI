# DeepResearch Event-Sourced Runtime

DeepResearch has one active execution shape:

```text
semantic plan
→ initial retrieval
→ semantic chunk selection with typed source coverage
→ typed coverage evaluation
→ optional supplemental retrieval from the existing candidate catalog
→ final obligation-level closed-evidence review
→ Host research-contract reduction
→ sectioned report transaction
```

The host owns this sequence. There is no active scout, perspective discovery,
maker/checker route, query-generating follow-up wave, adaptive research route,
or hidden continuation.

## Automatic Loop Engineering contract

Before planning, the host creates one transient Loop Engineering contract and
places it in the durable workflow input. The contract records:

- `pattern = minimal-deep-research`;
- `controller = host_inquiry_reducer`;
- `quota.mode = unlimited`;
- `execution.mode = coverage_driven`;
- the exact eight-stage graph shown above;
- maximum logical cardinality two for semantic iteration, retrieval, and
  semantic selection; and
- cardinality one for the final obligation-review stage, Host contract
  assessment, report transaction, and optional targeted section revision.

Rust Inquiry validates the contract before invoking the planner. A changed
goal, quota mode, stage order, cardinality, planner identity, hidden control
field, or relaxed safety fuse fails closed. The same contract is retained when
the validated plan is attached to the retrieval input.

Cardinality counts logical stages and passes, not model requests. The first
semantic iteration is mandatory; the second is only an upper bound for a typed
or operational-gap supplement. Complete source-local selectors,
obligation-level reviews, and per-section generation remain independent durable
units inside their declared stage. They do not consume another semantic
iteration and cannot schedule a third one.

This is an automatically created per-run contract, not a user-managed loop
asset. DeepResearch does not create `.a3s/loops/`, consume a `/loop` iteration
budget, or inherit the durable loop panel's maker/checker runtime. Unlimited
quota does not mean unbounded execution: per-call deadlines, maximum output
sizes, concurrency ceilings, closed-catalog limits, and transport limits remain
safety fuses. They terminate a failed run but cannot authorize a supplemental
pass without a Host-observed typed or operational retrieval gap, create a new
provider query, or permit a third semantic research pass.

## Authority boundaries

- Loop Engineering defines the immutable one-run stage contract.
- A3S Flow is authoritative for durable tool effects and their stable run IDs.
- The typed Inquiry event stream is authoritative for research obligations,
  questions, accepted evidence and source-coverage edges, contract assessment,
  report outline, drafts, the optional single targeted section revision, and
  report audit.
- The DeepResearch journal stores normalized references and complete validated
  Inquiry prefixes.
- `GraphRuntime` projects research objects and relations from journaled events.
- TUI status, Markdown, HTML, and tool-card text are disposable projections.

Flow is not a second research planner. Its JavaScript only adapts the validated
plan into bounded retrieval and structured-generation steps. Terminal research
and publication decisions remain typed Rust policy.

## Active pipeline

### 1. Semantic plan

One schema-constrained planner call produces:

- a reader-facing report title;
- one to four stable research tracks;
- one or two questions and completion criteria per track;
- explicit primary-source and independent-corroboration requirements;
- up to four search queries and three stable seed URLs;
- one bounded plan-level retrieval safety envelope; and
- observable stop conditions.

Each materially distinct publisher or artifact family receives its own track
when combining it would make direct source binding ambiguous. Every completion
criterion names only one independently sourced target and is atomic enough for
one fetched source to resolve directly. Comparison-wide criteria that require
two publishers are invalid planner intent. Web
plans use exactly eight initial fetch slots; local-only plans use zero. Seed
URLs are semantic candidates in the same catalog as provider results and never
reserve a slot. The planner has one 360-second active-generation fuse, still
bounded by the shared Inquiry deadline and its protected review/finalization
reserves.

The planner does not choose an execution route, method, stage graph,
parallelism, supplemental-pass policy, or number of research rounds. New
inquiries always commit `ResearchMethod::Focused`.

Planner-authored search queries are validated as non-empty strings without
surrounding whitespace and are passed to search providers byte-for-byte. The
host never rewrites, tokenizes, translates, expands, or normalizes them, and
each query is issued to the provider exactly once.

### 2. Initial retrieval

For public evidence, the adapter performs one bounded initial pass:

1. issue the planner queries once;
2. build the complete safe candidate catalog in seed, query, and
   provider-result order;
3. admit the initial fetch targets through one closed semantic candidate-ID
   decision that fills all available initial slots with material coverage and
   useful fetch-failure alternatives when enough candidates exist;
4. fetch up to eight admitted candidates (or the complete catalog when it is
   smaller);
5. recover a successful child hidden by aggregate batch-output truncation with
   one isolated fetch;
6. optionally retry a transient transport failure once; and
7. read bounded additional ranges of the same long document when its fetch
   response explicitly supplies a next offset.

The retry decision reads only the tool's structured `error_kind`: either the
string or typed `{ "type": ... }` value `network`, `timeout`, or `transport`.
Error prose never authorizes a retry.

The batch-output recovery, transport retry, and additional document ranges are
not additional logical retrieval passes. They do not create a query, revise
the plan, inspect a typed coverage gap, or authorize the supplemental pass.

Canonical GitHub release-list URLs are transported through the repository's
official Atom feed, preserving release titles, dates, and notes without global
site chrome. Batch children are checked against their original output sizes;
an omitted or truncation-marked successful child is refetched in isolation.
If the isolated result is still truncated, the source fails closed.

Search rank, URL, title, snippet, provider date, engine name, and source host
are discovery metadata. They are not used as evidence and are never scored
against the query. Provider dates may describe indexing, crawling, or a
documentation build, so they are never promoted into the closed evidence
packet; a report date must be established by fetched source text. URL
admission, transport validity, document type, and bounded substantive-text
checks may reject unusable payloads.

Seed URLs preserve planner order. Search results follow planner-query order and
each provider's returned result order. The host does not reorder candidates for
host diversity, per-query balancing, or another hidden admission policy.
Seed URLs compete semantically with provider results; they are not fetched
first and do not reduce the eight-slot initial allowance merely by existing.

Local-only research uses one bounded read-only local retrieval task. Its
reported source survives only when a successful `read` or `grep` observation
contains the exact same path. The task returns only observed paths and bounded
zero-indexed line ranges; it cannot return a fact, quotation, summary, or
conclusion. The host rereads every requested range and admits only text whose
returned range metadata and source anchor match the request. Directory
listings, ambiguous suffix paths, and model-authored paths are not evidence.

### 3. Durable semantic chunk selection and typed coverage

Web text and host-restored local ranges are split into stable bounded chunks.
Web chunks use `web-source-*` identities and local chunks use
`local-source-*` identities. Both are placed in one combined closed catalog
with the complete planner focus set. The combined source and chunk limits apply
before selection; the host never runs a second local selector or merges
model-authored local facts after selection.

Catalogs of at most 10 chunks use one durable selector. Larger catalogs use one
complete durable source-local selector per canonical source. Every fetched
chunk for a source enters that single call; the Host neither divides a source
into positional windows nor runs a second source reducer. Each source-local
selector returns at most four excerpt IDs, and independent sources retain
independent stable Flow identities.

Every selector judges meaning across languages and returns only:

```json
{
  "chunk_ids": [
    "web-source-1:chunk:2",
    "local-source-1:chunk:1"
  ],
  "source_relevance": [
    {
      "source_id": "web-source-1",
      "obligation_id": "runtime.behavior"
    }
  ],
  "source_coverage": [
    {
      "source_id": "web-source-1",
      "obligation_id": "runtime.behavior",
      "completion_criterion_indexes": [0],
      "roles": {
        "supporting": true,
        "primary": true,
        "independent": false
      }
    }
  ]
}
```

A relevance edge means the selected text materially addresses that obligation,
including useful partial support. Every selected source has at least one such
edge. A criterion index means the selected fetched text for that source directly
resolves every material element of the exact criterion. Related subject matter,
partial support, and discovery metadata do not create a coverage edge. A
conservative omitted edge remains an explicit gap so the one bounded
supplemental pass can try a stronger source. Strong partial source text remains
selected for closed review even without that edge; lack of full criterion
coverage alone never discards an otherwise material excerpt.

The host then:

- rejects every ID outside the catalog;
- rejects duplicate IDs;
- rejects missing, duplicate, unknown, or unselected source-relevance edges;
- rejects coverage for an unselected or unknown source;
- rejects unknown obligation IDs, invalid or duplicate criterion indexes,
  duplicate source/obligation edges, and incomplete or unknown role flags;
- requires the schema-enforced `supporting=true` flag on every coverage edge;
- permits `primary=true` and `independent=true` only when the obligation
  declared the corresponding evidence requirement;
- canonicalizes the validated booleans into the ordered durable role enum set;
- enforces the four-excerpt and 2,800-character per-source limits;
- rejects empty or oversized catalog entries; and
- restores the exact text stored under each accepted ID.

Every structured selector uses the provider's typed finite generation
capacity. The `AgentSession` owns one gate shared by conversation loops and
every rebuilt host-direct runtime. A Flow step with the exact tool identity
`generate_object` waits for capacity before starting its bounded Program VM,
then passes a one-shot identity-checked permit into the nested tool call. Queue
wait is recorded separately and consumes neither the Program deadline nor the
bounded active-generation safety fuse. Each complete source-local selector uses
one 270-second attempt because a failed source can remain explicitly partial;
candidate admission and small-catalog selection retain the retryable
210-second fuse.
Concurrent nested and independent host-direct calls must enter the same
admission queue. This
dispatch is based on the Flow step's exact tool contract, not text scanning,
fuzzy matching, language detection, or prompt wording.

Selectors never return source text, a quotation, translation, or
summary. Their only semantic output is closed chunk/source/obligation
relevance identities, closed criterion indexes, and closed role enums. A missing,
malformed, failed, replay-divergent, or over-limit source-local selector
promotes none of that source's fetched text, while independently validated
sibling sources remain a partial packet with explicit diagnostics. If no valid
source remains, selection still fails closed. Completed sibling selectors are
durable effects and are not rerun because another source times out.

Materialization creates exactly one structured result and one accepted
evidence item per canonical source. Its excerpt-derived claims and typed
coverage edges name only that source. This preserves the claim-to-source
relationship through the evidence ledger and prevents a claim from borrowing a
citation carried by another source's item. There is no global truncation of the
source-local key-evidence lists.

The validated coverage graph follows the exact restored source packet into
structured evidence, the durable accepted-evidence ledger, stable source-ID
remapping, `EvidenceRef`, Inquiry replay, question packets, report synthesis,
and graph projection. No later layer reconstructs a role from a URL, title,
source name, language, or wording.

### 4. Typed coverage evaluation and optional supplemental retrieval

After the initial selection, the adapter deterministically evaluates every
obligation against the validated graph:

- a completion criterion is covered only when a `supporting` edge names its
  exact zero-based index;
- a required primary source is covered only when at least one distinct source
  carries `primary`; and
- required independent corroboration is covered only when at least two
  distinct source IDs carry `independent`.

Before any optional supplemental work starts, the complete initial
materialized evidence portfolio is committed as its own durable Flow step. If
the shared retrieval deadline later interrupts supplemental admission, fetch,
or selection, the Host may recover only that exact run/query checkpoint, and
only when it still contains valid traceable evidence and declares Host Inquiry
terminal authority. The optional pass therefore cannot erase already completed
material evidence or turn it into an empty recovery report.

If no typed or operational gap remains, retrieval closes immediately. If typed
coverage is missing, an admitted fetch fails, or a source-local selector drops
fetched text, and the original provider catalog still contains an unselected
safe candidate, the adapter may run one supplemental pass. A closed semantic
decision selects at most two remaining candidate IDs beyond the eight initial
web-fetch slots. Typed-gap packets are limited to the affected obligations;
operational replacement can restore the original packet even when initial
materialization retained no source. The pass applies the same complete
source-local selection, typed-edge validation, one-item-per-source
materialization, and exact-text restoration rules.

The supplemental selector receives exact operational outcomes for every initial
candidate. When a fetch retained no substantive text, remaining candidates on
that same transport surface are excluded if at least one different surface is
still available. A source whose semantic selector retained no text remains
eligible for replacement by a materially different artifact.

The supplemental pass never calls search. It reuses the original provider
candidate catalog and the planner queries only as immutable packet data; it
cannot add, rewrite, normalize, translate, or retry a query. A contradiction,
prose diagnostic, effort setting, or lack of remaining candidates cannot
authorize another pass. Only the validated typed graph and counted operational
loss from the initial fetch/selection path can authorize the single supplement.
After the supplement completes or cannot run, the initial and supplemental
accepted evidence are merged once and retrieval becomes closed.

### 5. Obligation-level closed-evidence reviews

The host builds one identity-closed packet per obligation group. It includes
evidence carrying an exact source-relevance edge for that obligation plus
legacy or non-selector evidence that is explicitly unscoped. It never chooses
evidence by lexical, positional, language-specific, or score-ranked matching.
If the complete scoped set exceeds the packet contract, that review fails
closed.

Questions with the same exact obligation linkage share one stable durable
review unit and receive the closed packet only once. Up to three obligation
groups execute concurrently. Each durable group may make at most two identical
300-second active attempts so a stuck provider generation can recover without
changing the semantic packet; the shared 15-minute stage remains the portfolio
limit. Completed sibling groups survive interruption and reuse their durable
effects after resume.

- `answered` requires at least one allowed evidence reference;
- `partial` retains a useful traceable answer plus one explicit human-facing
  limitation; or
- `bounded` records why the closed packet does not support an answer.

The provider-facing schema uses short Host-owned references such as `E1`, so a
model never needs to reproduce a long evidence hash. The Host deterministically
maps each reference through the question's exact allowed evidence-ID set before
emitting an Inquiry event. After validating the shared wire envelope, the Host
decodes each expected question independently. One malformed entry or invalid
reference bounds only that question and cannot discard a valid sibling from the
same obligation review. If an entry says `answered` but also carries an explicit
non-empty limitation, the Host safely demotes it to `partial`; it never publishes
the qualified result as fully answered or discards its traceable supported part.

The review cannot browse, call tools, create questions, defer questions, request
retrieval, or schedule a continuation.

The review also preserves the source's semantic granularity. It lists exact
dated observations instead of inventing intervals, does not reinterpret an
`updated` timestamp as a release date, and does not derive incompatibility,
future maintenance guarantees, replacement properties, governance authority,
or ecosystem-wide prevalence from narrower evidence.

Provider queries remain plan-level retrieval inputs. Active questions are
linked to stable research obligations and never receive queries by array
position.

### 6. Research-contract assessment

After obligation-level review, the Host deterministically reduces the typed
obligation-to-question-to-evidence graph into assessments for every completion
criterion, declared evidence-quality requirement, stop condition,
contradiction, and evidence gap. There is exactly one Host assessment event and
no model contract-assessment journal.

A primary-source requirement can become satisfied only through an accepted
`primary` edge on that obligation's answer path. Independent corroboration can
become satisfied only through accepted `independent` edges for at least two
distinct answer-path source IDs. Multiple excerpts or ranges from one canonical
source still count as one source. Without those typed roles, traceable evidence
remains bounded rather than being upgraded from lexical metadata.

The assessment produces one typed contract outcome:

- `satisfied`;
- `qualified`, with explicit bounded limitations; or
- `unsatisfied`.

An unsatisfied material contract cannot enter normal report publication.

### 7. Report

The report stage consumes only the replayed Inquiry contract and accepted
evidence. The Host deterministically derives the outline from
obligation-to-question-to-evidence edges. Each section then owns an independent
durable Flow identity, with at most three section calls in flight. Completed
sibling sections are never rerun because another section fails.

Section models return body Markdown only. The Host owns section, claim, and
source identity; derives the actual cited source IDs from exact Markdown
anchors; requires every committed claim to have at least one actually cited
source from the same accepted evidence binding; and does not require every
alternative source within that binding to appear. Each closed generation
packet marks every evidence binding as requiring one exact source citation. If
the first draft misses a binding, the single targeted revision receives the
precise uncited binding and its accepted source alternatives rather than only
an opaque claim ID. Because revision returns a complete replacement, its packet
also repeats the exact accepted source alternatives for every other binding;
repairing one omission cannot silently remove a valid sibling citation. Each
binding also carries bounded accepted-claim excerpts,
which control dates and numerical literals if a reviewed answer was
mis-transcribed. Before a section is committed, the Host normalizes ISO,
English, and Chinese full dates and rejects any date that cannot be traced to a
committed claim; the targeted revision receives that exact diagnostic and the
same claim excerpts. The final source ledger
therefore contains only sources cited in report bodies, while the full
claim/source graph remains available for Host audit. Model-authored H1 and H2
syntax is deterministically demoted below the Host-owned section heading, with
a structural Markdown check as a fail-closed backstop.
An exact accepted URL emitted as citation-shaped `[https://…]` text is
canonicalized to a CommonMark autolink outside fenced and inline code before
the Host derives source IDs; no prose or language rule participates.
A model-authored URL that is a strict same-origin path descendant of a committed
source is reduced to the longest matching committed parent. Lexical siblings,
domain-root parents, query-bearing parents, and URLs inside code remain exact
and cannot satisfy the citation audit.

The same inference guardrails are repeated in initial section generation, the
single targeted revision, and editorial framing. A recommendation may combine
supported premises, but it remains a recommendation and cannot become a new
factual claim. Operational `Fetched source text...` discovery/review metadata
stays available to the closed assessment and is omitted from the published
source ledger. Reader-facing review, section/revision, and frame prose must use
the query language. A local evidence packet can qualify only its linked claim or
dimension; it cannot assert that the complete report has no evidence for facts
owned by another obligation.

Qualified-report cautions are generated from typed question limitations,
obligation titles, completion criteria, source-quality status, and accepted
evidence diagnostics. Internal assessment rationales are never copied into the
reader-facing report.

The report commits evidence-bound drafts, an editorial frame, and a
deterministic audit before atomically materializing Markdown and HTML.

Active report generation has one global targeted-section-revision allowance. A
failure found during section validation or the final audit consumes that same
allowance. The replacement section is validated and the complete report is
audited one final time. A second revision is never scheduled.

Frame generation has two identical attempts with a 270-second active fuse per
attempt. Capacity admission remains outside that active clock, while both
attempts remain inside the single durable report deadline.

Process recovery resumes the same durable sectioned-report transaction from its
committed outline, draft, or audit boundary. Resume replays the same event
prefix and stable Flow identities; it is not a content revision, does not reset
the shared deadline or revision allowance, and cannot start another completed
report pipeline.

Publication rejects unaccepted citations, unsupported quantitative claims,
unsafe artifacts, leaked workflow output, and an Inquiry state that does not
strictly replay to the terminal prefix.

## Language and relevance invariants

Active DeepResearch contains no:

- keyword or token overlap;
- stopword filtering;
- fuzzy or n-gram matching;
- local relevance score;
- topic or named-entity classifier;
- query-length route;
- language detection or language-specific routing; or
- source admission based on shared spelling, morphology, transliteration, or
  writing system.

Multilingual relevance is decided only by the schema-constrained semantic
selector and the later obligation-level closed-evidence reviews. Provider
queries remain exactly the planner output.

## Storage and replay

Runtime state is stored below:

```text
.a3s/research/runs/events/
.a3s/research/runs/checkpoints/
.a3s/workflow/
```

The research journal must not create `.a3s-flow/` or another peer runtime root.
`FileGraphEventStore` uses atomic replacement, and compare-and-swap heads
prevent a stale writer from overwriting a newer generation.

Each Inquiry checkpoint contains the complete event prefix and its strictly
replayed state. A writer may temporarily reconstruct a shorter in-memory prefix
only when it is byte-for-byte equal to the corresponding durable prefix. At the
durable head, projected state must also match before a suffix can be appended.
Any changed plan, retrieval result, generation result, event order, or identity
fails at the first divergence.

Planner, initial retrieval, semantic selection, optional supplemental
selection/retrieval, obligation-review, contract-assessment, outline, draft,
targeted-section-revision, audit, and frame effects use stable Flow identities
derived from the root run and durable input. A process restart reuses completed
effects. An effect that was running when the process disappeared may be
redelivered with the same Flow identity; this does not authorize a new query, a
second supplement, a third semantic research pass, or a parallel report
transaction.

The Inquiry deadline is reconstructed from the original run-creation journal
timestamp, so restarting does not grant a fresh planning, retrieval, review, or
assessment budget. Its typed ResearchSpec records a 45-minute total, a
25-minute whole-retrieval-stage cap, a protected 15-minute obligation-review
stage, and a two-minute finalization reserve. Planner and retrieval work cannot
spend the review reserve; review can spend that reserve but not finalization.
The Host wraps the complete retrieval future and the complete review stream in
their stage clocks. Per-step Program limits remain active-work fuses and model
admission queue time is accounted only by the appropriate whole-stage clock. A
wall clock that moves behind the durable origin fails closed with no remaining
model budget.

The completed-report transaction separately records one idempotent
`research.report.started` event. Outline, section generation, the optional
targeted revision, re-audit, and editorial frame share the one report budget
derived from that immutable timestamp. Durable resume and caller timeouts may
shorten the remaining budget but cannot extend it; a regressed clock likewise
leaves no report-model budget. There are no independent synthesis and repair
clocks.

## Core invariants

1. A terminal run cannot schedule or accept ordinary new work.
2. Duplicate external events are idempotent.
3. A stale graph generation cannot overwrite a newer head.
4. Terminal grades are monotonic:
   `Failed < Degraded < Qualified < Completed`.
5. A host-managed run contains exactly one committed research contract and one
   terminal contract assessment before report outlining.
6. Every accepted evidence item has at least one traceable source.
7. Long-document excerpts remain nested under one canonical source identity;
   excerpts never multiply independent-corroboration counts.
8. Selector and replay failures fail closed.
9. One logical obligation-review stage resolves the entire closed question set;
   questions with identical obligation linkage share one durable review unit.
10. Only a Host-validated typed coverage gap or counted operational loss from
    initial fetch/source selection can authorize the single supplemental
    retrieval pass, and that pass can use only unselected candidates from the
    original provider catalog. No contradiction, effort profile, queued user
    turn, or generated prose can authorize a query or a third retrieval pass.
11. A queued user turn cannot interleave between DeepResearch retrieval and
    terminal report settlement.
12. Report validation and final audit share one targeted section revision
    allowance; durable resume consumes neither a second allowance nor a second
    report pipeline.
13. Rust stamps every current Inquiry projection with
    `terminal_authority = host_inquiry_reducer`; historical checker metadata
    cannot classify or publish a current run.
14. A section must cite at least one source from the accepted binding of every
    committed claim; unused alternative sources never enter the published
    source ledger or trigger a model revision.
15. Reader-facing qualified disclosures cannot reuse internal contract
    assessment rationale, and a section body cannot introduce H1 or H2
    structure above its Host-owned heading.

## Restart reconciliation

The frozen research specification records the host PID as a local operation
lease. On startup, an active run is reconciled only when that host process no
longer exists. A live host is never modified.

For an interrupted run, still-running observed children are cancelled through
the session API and missing children are classified as orphaned before the old
run is terminalized. Completed tool effects are recovered through Flow history,
not silently executed under a new identity.

Process-level tests cover interruption after planner completion, during
retrieval, after obligation review but before its Inquiry checkpoint, after
contract assessment but before its event commit, and across report
outline/draft/audit boundaries. Retrieval integration tests separately prove
that a typed source-role gap can drive exactly one supplemental pass.

## Legacy replay compatibility

Historical journals may contain:

- `PerspectiveGuided`, `Scouting`, or `PerspectiveDiscovery`;
- `ScoutCompleted`, `PerspectiveBudgetSelected`, or
  `PerspectivesCommitted`;
- perspective IDs, parent question IDs, nonzero question rounds, or
  `QuestionDeferred`;
- `inquiry_collection_wave`;
- old maker/checker and follow-up retrieval metadata; or
- more than one historical section revision within the wider replay limit.

These values remain decodable only so existing journals can be strictly
replayed and diagnosed. They are hidden from the active planner, have no public
constructor for new perspective or follow-up values, and are never emitted by
the active runtime. Compatibility metadata cannot regain scheduling or
publication authority.

## Diagnostics

Read-only diagnostics are backed by strict replay:

```text
/research status [run-id]
/research explain [run-id]
/research replay [run-id]
/research diff <left-run-id> <right-run-id>
```

Without a run ID, diagnostics select the active run first and otherwise use the
latest local journal. `replay` reports the verified event count, graph shape,
and event head. `diff` compares two strictly replayed graphs by semantic object
and relation type.

## Migration rule

New active DeepResearch state must be represented by a typed domain event and a
replayable projection field. Do not add a mutable TUI latch, lexical relevance
gate, alternate planner, retrieval route, or hidden continuation.

The JavaScript fragments under `deep_research/workflow/` may perform only the
bounded retrieval adapter and durable generation scheduling described above.
Research sufficiency, contract outcome, section-revision limits, terminal
convergence, and publication remain typed host policy.
