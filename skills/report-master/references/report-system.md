# Report System Execution Contract

This is the execution kernel for `report-master`. It transfers the strategy, specification-lock, page-rhythm, semantic-visualization, anti-drift, and rendered-review principles of `ppt-master` to evidence-grounded long-form reports.

## Contents

1. [Build an evidence model before an outline](#1-build-an-evidence-model-before-an-outline)
2. [Strategist pass](#2-strategist-pass)
3. [Lock the report specification](#3-lock-the-report-specification)
4. [Select visual forms by semantic fit](#4-select-visual-forms-by-semantic-fit)
5. [Execute section by section](#5-execute-section-by-section)
6. [Visual system requirements](#6-visual-system-requirements)
7. [Visual review and repair](#7-visual-review-and-repair)
8. [Rejection conditions](#8-rejection-conditions)

## 1. Build an evidence model before an outline

Create a compact internal inventory with four distinct concepts:

| Concept | Meaning | Never infer it from |
| --- | --- | --- |
| Source family | An institution, publisher, dataset owner, or genuinely independent provenance chain | URL count or report section count |
| Evidence item | One traceable fact, quote, record, observation, or dataset slice | Source-family count |
| Requested dimension | A question or aspect the user expects answered | Evidence-item count |
| Report section | An editorial unit in the final narrative | Retrieval task or agent count |

For each material claim, retain its supporting evidence, source family, date/version, confidence, and any contradiction. If coverage is weak, narrow the claim or mark the gap. Never inflate depth by renaming sections or search tasks as independent evidence tracks.

Depth means useful explanatory work: answering why, how, compared with what, what changed, what follows, and what remains unknown. It does not mean adding audit prose.

When DeepResearch supplies semantic plan tracks, build a private treatment for every track before drafting:

```text
Track:
Status: answered | bounded
Finding:
Interpretation:
Implication:
Counterpoint or uncertainty:
```

The final section structure may combine or reorder tracks for the reader. The private map exists to prevent a polished narrative from silently dropping part of the research question. Do not expose the map as workflow or audit prose.

In a host-owned structured response, keep this map compact: one or two precise sentences per field are enough. Prefer the full plan-track name; otherwise use a unique semantic label rather than an ambiguous paraphrase. The report body receives the writing budget.

## 2. Strategist pass

Before HTML, lock a report strategy in working memory:

```text
Audience:
Reading occasion:
Decision or use:
Dominant thesis:
Communication mode:
Mode rationale:
Visual-style position:
Visual archetype:
Evidence boundary:
Required dimensions:
Section sequence:
```

The dominant thesis is a one-sentence answer, not a topic label. The section sequence must advance that thesis; remove any section whose only purpose is “showing more research.”

Before locking art direction, apply an identity test: name the dominant information relationship, the reader's intended use, and the structural choice that follows. If the rationale would still fit an unrelated report after removing topic nouns, reject it as generic. Structural choice means hero, navigation, rhythm, geometry, or composition—not palette alone.

### Communication mode: how the report argues

Choose one dominant mode. A section may lean differently, but do not blend modes accidentally.

- **Pyramid** — conclusion first, then mutually distinct supporting arguments and comparisons. Best for recommendations, decisions, and analytical reports.
- **Narrative** — situation, tension, turning point, resolution, implications. Best for events, investigations, case studies, and change over time.
- **Instructional** — concepts and steps build progressively toward mastery. Best for explainers, technical guides, and mechanisms.
- **Showcase** — a few high-salience findings with strong visual pacing and minimal copy. Best for portfolios or visual discoveries; rarely appropriate for evidence-dense research.
- **Briefing** — complete, scannable coverage with balanced topic weight. Best for status, landscape, reference, and monitoring reports.
- **Custom** — use only when no mode has a dominant fit. Describe its concrete sequence and title voice; never use “custom” as a substitute for judgment.

Mode and visual style are independent. A pyramid report may be restrained editorial or bold technical; a narrative report may be archival or minimal.

### Visual archetype: the composition family

In host-rendered reports choose one approved archetype. This is a layout decision, not a synonym for communication mode.

- **Editorial** — long-form synthesis with typographic hierarchy and a persistent reading rail.
- **Analytical** — comparisons, mechanisms, matrices, and dense evidence with precise geometry.
- **Chronicle** — ordered change, event arcs, investigations, and turning points with temporal rhythm.
- **Executive** — restrained, conclusion-first decision material with low ornament and compact scanning.
- **Field notes** — observations, case work, exploratory findings, and qualified interpretation with an annotated-paper character.

Then independently choose an approved palette (`ocean`, `graphite`, `forest`, `amber`, `plum`), density (`compact`, `balanced`, `spacious`), hero (`statement`, `split`, `metrics`), and stance (`safe`, `shifted`, `bold`). The combination must be explained by audience and information structure. Never map a subject keyword directly to a template or palette.

Choose a metrics hero only when an evidence profile helps the reader evaluate the work; do not elevate source, section, or reading-time counts merely to fill the cover. A statement hero is a genuinely thesis-only cover. A split hero uses its second region as a reading path; a metrics hero alone uses the evidence profile.

### Visual-style position: how the report looks

Choose deliberately on a three-position spectrum:

- **Safe** — restrained, familiar, low ornament; appropriate for formal or high-risk readers.
- **Shifted** — one distinctive editorial motif, asymmetric rhythm, or unusual but controlled palette; the default when no style is specified.
- **Bold** — strong art direction and higher contrast; use only when subject, audience, and evidence can sustain it.

Derive the motif from the subject's semantics, not from a generic “premium” aesthetic. Examples include archival dossier, field notebook, scientific plate, newspaper analysis, technical blueprint, or quiet institutional brief. Never imitate a motif at the cost of readability.

## 3. Lock the report specification

Keep this lock in working memory because the host permits only two final files:

```text
THESIS
- one-sentence answer

MODE
- communication mode and title voice
- visual-style position and semantic motif

TOKENS
- canvas, surface, ink, muted ink, line, accent, semantic status colors
- display and reading font stacks
- type scale, prose measure, spacing scale, radius, border, shadow

COMPOSITION VOCABULARY
- 3–5 forms this report may use
- forms explicitly forbidden for this report

SECTION MAP
- purpose, key claim, evidence, rhythm, composition, selected rationale, rejected runner-up

CITATION SYSTEM
- inline claim marker and final source-ledger format
```

The lock must be concise enough to re-read before every major section. Do not silently change tokens, motif, title voice, or geometry mid-report. If content makes a locked choice invalid, revise the lock once and apply the revision consistently.

### Section rhythm

Assign every major section one role:

- **Anchor** — establishes a thesis, turning point, or decisive conclusion. One dominant visual idea and little competition.
- **Dense** — carries comparisons, evidence, mechanisms, or detailed analysis. Strong grid and scanning support.
- **Breathing** — slows the reader with a synthesis, quote, implication, or transition. Generous whitespace; never a three-card grid.

Rhythm follows meaning, not an alternating formula. Avoid long runs of identical roles. A typical analytical sequence might be `anchor → dense → dense → breathing → anchor → dense`.

Build a private section map:

| Section | Editorial purpose | Key claim | Evidence | Rhythm | Composition | Why it fits | Rejected runner-up |
| --- | --- | --- | --- | --- | --- | --- | --- |

For a host-owned structured response, translate this map into the compact `presentation.section_plan` only after the Markdown outline is final. Keep no more than 12 H2 sections. Copy each H2 exactly and emit one entry per H2, in report order:

```text
heading: exact H2 text
rhythm: anchor | dense | breathing
composition: prose | key_points | comparison | timeline | process | evidence | source_ledger
```

The renderer rejects missing, renamed, or duplicate headings. Do not use the plan as another outline or rationale; its only job is to bind semantic sections to safe compositions.

## 4. Select visual forms by semantic fit

Choose the smallest form that makes the relationship easier to understand.

- Exact mappings or repeated-field comparisons → table or comparison matrix.
- Values with a meaningful common scale → bars, dot plot, or metric band.
- Change across ordered time → timeline or line chart when enough measured points exist.
- One cause or decision affecting several consequences → branching relationship or annotated causal chain.
- Ordered dependencies or state changes → process flow.
- Hierarchy or ownership → tree or nested structure.
- Geography with location-dependent meaning → map.
- A single conclusion with qualitative support → thesis block plus evidence annotations.
- Nuanced argument without a strong spatial relationship → edited prose, not a diagram.

Reject charts when data is sparse, scales are incomparable, or encoding adds decoration rather than comprehension. Reject a card grid when the items form a sequence, hierarchy, comparison matrix, or continuous argument. Never manufacture metrics for visual balance.

## 5. Execute section by section

For every major section:

1. Re-read the thesis, mode, tokens, and this section's map row.
2. State the section's editorial job in one sentence internally.
3. Use only claims supported by the evidence inventory.
4. Construct the selected composition with semantic HTML before decoration.
5. Attach citations to consequential claims.
6. Check that the visual emphasis matches the key claim.
7. Continue only after the section works at desktop and narrow widths by inspection.

Use deterministic CSS tokens for consistency, but do not batch-stamp identical HTML across sections. Shared scaffolding is appropriate; editorial composition requires per-section judgment.

In host-owned DeepResearch, express the strategy through the structured `editorial` and `presentation` fields. The host applies a safe renderer whose global variants and section plan implement these composition families. Never return arbitrary HTML, CSS, class names, scripts, or network assets in those fields. In ordinary file-authoring mode, execute the same lock directly in the standalone HTML.

The cover establishes subject, scope, evidence date, and visual thesis. It must not reproduce the full user prompt. The executive summary answers the question with conclusion, strongest support, implication, and confidence boundary. Methodology and source quality remain compact and secondary. The ending synthesizes consequences or next actions; it must not merely repeat the summary.

Copy citation targets byte-for-byte from accepted evidence. Never guess a corrected URL. When a useful label lacks an accepted target, leave it as plain text or remove the source item. During repair, preserve valid sections and apply the smallest correction that restores the content, source, or presentation contract; do not restart evidence collection or rewrite the whole report for a local defect.

## 6. Visual system requirements

- Standalone UTF-8 HTML; embedded CSS; no scripts, remote fonts, or required network assets.
- Semantic landmarks and ordered headings with exactly one `h1`.
- CSS custom properties for all repeated visual decisions.
- Readable prose measure of roughly 68–78 characters and fluid typography with `clamp()`.
- High-contrast ink, one primary accent, and semantic colors used only for real meaning.
- Visible links and focus states; color is never the sole semantic signal.
- Tables remain legible and scroll inside labeled wrappers on narrow screens.
- No horizontal page overflow at 390px; minimum mobile body size 15px.
- `prefers-reduced-motion` support and print CSS that removes chrome, preserves URLs, and controls breaks.
- Decorative SVG is hidden from assistive technology; meaningful diagrams have text alternatives.

## 7. Visual review and repair

Quality is judged from what a reader sees, not from HTML parsing alone.

When a renderer or browser is available, inspect at least:

1. Desktop at approximately 1440px width.
2. Mobile at 390px width.
3. Print preview or print stylesheet behavior.

Review in this order:

### Hard failures — always repair

- clipped, overlapping, off-canvas, hidden, or illegible content;
- broken hierarchy, missing key conclusion, or visual emphasis on the wrong claim;
- contrast failure, broken links, malformed tables, or horizontal page overflow;
- missing evidence required by the section map;
- unsupported facts, citation mismatch, or internal workflow leakage.

### Soft failures — repair when clearly harmful

- monotonous repeated cards or unchanged two-column rhythm;
- excessive density, dead whitespace, weak alignment, or inconsistent spacing;
- decorative visuals without explanatory value;
- audit metadata competing with findings;
- visual motif or token drift from the lock.

After each repair, re-check the affected viewport. Prefer one controlled repair pass over endless aesthetic churn. If rendering is unavailable, perform the same review against HTML structure and CSS, state nothing about the unavailable tooling in the report, and let the host's post-turn validator make the final acceptance decision.

## 8. Rejection conditions

Reject the report before completion if it:

- reads like a search log, evidence ledger, or recovery artifact;
- counts sources or tracks instead of explaining the subject;
- confuses topical coverage with independent corroboration;
- uses generic headings that could fit any topic without answering the query;
- repeats one component family across most sections;
- contains a chart whose data or encoding cannot be defended;
- looks like a generic admin dashboard or mechanically styled Markdown;
- has no dominant thesis, narrative spine, or meaningful visual hierarchy.
- omits a semantic plan track without marking its evidence boundary, or lists facts without interpretation and reader consequence.

Repair the artifact locally without restarting research. The final report must be self-contained, useful without workflow context, and visually authoritative.
