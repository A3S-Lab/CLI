---
name: report-master
description: Create polished, responsive, evidence-grounded HTML research reports and matching Markdown. Use for DeepResearch deliverables, executive reports, analytical briefs, timelines, comparison reports, and whenever the user asks for a beautiful, professional, premium, visual, or presentation-quality web report.
allowed-tools: "read(*), write(*), edit(*)"
---

# Report Master

Turn verified research into a designed reading experience, not Markdown wrapped in a stylesheet.

This skill adapts the durable method behind [ppt-master](https://github.com/hugohe3/ppt-master) to long-form reports: strategy and visual execution are separate stages; narrative mode and visual style are separate decisions; a compact design lock prevents drift; sections have deliberate rhythm; and quality is judged from the rendered artifact.

## Deliverables

There are two execution modes. Never mix them.

When an ordinary agent turn names a report directory and permits file tools, write exactly two sibling deliverables:

- `report.md`: the complete semantic report with claim-level citations.
- `index.html`: a standalone UTF-8 report with embedded CSS, no scripts, remote fonts, or required network assets.

Keep strategy and design-lock notes in working memory. Do not create extra deliverables unless the host explicitly permits them. `index.html` is the final visual authority, not an export of `report.md` through a generic template.

In host-owned DeepResearch synthesis, do not write files or author arbitrary HTML/CSS. Complete the requested structured object instead:

- `markdown`: the finished report;
- `editorial`: a private thesis and one coverage treatment for every semantic research-plan track;
- `presentation`: a private, schema-constrained global art direction and per-section composition lock.

The host validates depth and citations, then renders only approved layout, palette, density, hero, and visual-stance tokens. This keeps content-driven variation without allowing generated code into the artifact pipeline.

Spend the completion budget on `markdown`. Keep the private `editorial` treatment for each track to one or two precise sentences per field and keep the presentation rationale concise; neither field is a second report. Reuse a planned track's full name when practical. A shorter label is valid only when it is unambiguous and preserves the same semantic obligation.

## Non-negotiable pipeline

Run these stages serially:

1. **Evidence inventory** — separate source families, individual evidence items, requested dimensions, supported claims, uncertainties, dates, units, and contradictions. Never call report sections “independent evidence tracks.”
2. **Report strategist** — determine audience, reading occasion, desired use, one communication mode, one visual archetype, one visual-style position, a dominant thesis, and a section outline.
3. **Specification lock** — lock typography roles, palette roles, geometry, spacing, composition vocabulary, section rhythm, citation style, and forbidden patterns before authoring HTML.
4. **Editorial execution** — author sections in sequence. Before each major section, re-read the thesis and lock; select a composition because it fits the information, not because a component is available.
5. **Visual review** — render and inspect desktop and mobile when rendering is available; otherwise perform the source-level fallback review. Repair defects before completion.

Read and follow [`references/report-system.md`](references/report-system.md) for the complete execution contract. In autonomous DeepResearch the host embeds that same contract directly, so do not pause to locate it or invoke this skill recursively.

## Content standard

- Lead with the answer the reader came for. The executive summary must contain the conclusion, material support, consequence, and confidence boundary.
- Give every section an editorial purpose. Omit sections that merely restate workflow status, evidence counts, or generic caveats.
- Attach citations to consequential claims and preserve source-specific definitions, dates, time zones, units, averaging periods, and versions.
- Distinguish facts, interpretations, conflicts, and unknowns without letting methodology dominate the report.
- Put compact source-quality notes in an appendix or source ledger. Never expose tool logs, JSON, agent state, internal paths, recovery instructions, or host terminology.
- Do not invent facts, quotations, URLs, precision, or visual data to fill a layout.
- Copy every citation target exactly from accepted evidence. If a useful source label has no accepted target, retain the label as plain text or omit the source item; never repair, autocomplete, or approximate a URL.
- Treat each semantic research-plan track as an answer obligation, not a section title. For every consequential track, connect the supported finding to interpretation, implication, and a counterpoint or uncertainty boundary. A track may be explicitly bounded when evidence is insufficient; it may not silently disappear.
- Depth is judged by explanatory coverage, not length: answer why, how, compared with what, what changed, what follows, and what remains unknown wherever those lenses materially fit the question.
- Apply the reader-value test to every major section: it must change the reader's understanding, decision, or confidence boundary. Delete methodology recap, source-count commentary, and generic limitations that do none of those jobs.

## Design standard

- Treat communication mode as how the report argues and visual style as how it looks. Lock them independently.
- Make art direction structural, not cosmetic. A different choice must materially affect the hero, navigation, section rhythm, geometry, or composition—not merely swap palette tokens.
- Lock the Markdown outline before its section plan. Copy each H2 exactly, then choose that section's rhythm and smallest semantically useful composition; never force prose into a decorative form.
- Alternate `anchor`, `dense`, and `breathing` sections according to meaning; do not stamp every section into the same card grid.
- Use a visualization only when its semantic fit is stronger than prose or a table. Record the selected form and rejected runner-up in the section plan.
- Use one coherent token system and a small composition vocabulary, but make each major section composition-specific.
- Use a metrics-led hero only when the evidence profile materially helps the reader judge the report. Prefer a statement or split hero when counts would be decorative.
- Avoid the default “premium dashboard” look: excessive cards, glass effects, neon gradients, arbitrary metrics, decorative charts, and repeated two-column blocks are failures.
- Preserve reading comfort: constrained prose width, strong hierarchy, responsive tables, visible focus, sufficient contrast, reduced-motion support, and useful print CSS.

### Host-owned presentation vocabulary

When the host requests a `presentation` object, select each axis independently from the report's argument, audience, evidence shape, and reading occasion:

- narrative mode: `pyramid`, `narrative`, `instructional`, or `briefing`;
- visual archetype: `editorial`, `analytical`, `chronicle`, `executive`, or `field-notes`;
- palette: `ocean`, `graphite`, `forest`, `amber`, or `plum`;
- density: `compact`, `balanced`, or `spacious`;
- hero: `statement`, `split`, or `metrics`;
- visual stance: `safe`, `shifted`, or `bold`.

For every Markdown H2, add one `section_plan` entry in report order with the exact `heading`, a rhythm (`anchor`, `dense`, or `breathing`), and a composition (`prose`, `key_points`, `comparison`, `timeline`, `process`, `evidence`, or `source_ledger`). Keep the plan compact. Use different forms only when the underlying relationships differ.

Write a concise rationale that ties the combination to information structure and audience. Do not select from topic keywords, literal brand colors, a memorized task template, or a desire for novelty. Reusing the same combination is valid only when the semantic rationale is genuinely the same.

Apply the identity test: the rationale must name the dominant information relationship, the reader's use, and at least one resulting structural choice. If it could describe an unrelated report after replacing the title, reject it and choose again.

Choose from relationships, not nouns: comparison and trade-off structure, temporal change, causal mechanism, procedural sequence, evidence uncertainty, decision stakes, and reading speed are useful design inputs. No input maps mechanically to one archetype. Select the combination that best exposes the report's dominant relationship, then use section rhythm to avoid a one-template page.

## Completion gate

Read both files after writing. Reject and repair the report if any of these are true:

- the executive summary does not answer the query;
- a planned research track is neither answered nor explicitly bounded, or a material finding lacks interpretation and consequence;
- source families, evidence items, dimensions, and sections are conflated;
- unsupported claims or orphaned citations appear;
- the page resembles raw Markdown, a dashboard template, a recovery page, or a wall of identical cards;
- heading hierarchy, overflow, mobile layout, contrast, links, tables, or print output fail review;
- the rendered hierarchy disagrees with the report thesis or a section's editorial purpose.
- the section plan omits, duplicates, or renames an H2, or assigns a composition that does not fit its information relationship.

Repair locally before rewriting globally. Preserve valid prose and evidence; correct a missing track treatment, malformed citation, weak thesis, or mismatched presentation field at the smallest possible scope. Never restart research from the report phase.

Emit the host's report marker only after the two final artifacts pass these gates.
