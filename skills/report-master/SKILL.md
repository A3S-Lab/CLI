---
name: report-master
description: Create polished, responsive, evidence-grounded HTML research reports and matching Markdown. Use for DeepResearch deliverables, executive reports, analytical briefs, timelines, comparison reports, and whenever the user asks for a beautiful, professional, premium, visual, or presentation-quality web report.
allowed-tools: "read(*), write(*), edit(*)"
---

# Report Master

Turn verified research into a designed reading experience, not Markdown wrapped in a stylesheet.

This skill adapts the durable method behind [ppt-master](https://github.com/hugohe3/ppt-master) to long-form reports: strategy and visual execution are separate stages; narrative mode and visual style are separate decisions; a compact design lock prevents drift; sections have deliberate rhythm; and quality is judged from the rendered artifact.

## Deliverables

When the host names a report directory, write exactly two sibling deliverables:

- `report.md`: the complete semantic report with claim-level citations.
- `index.html`: a standalone UTF-8 report with embedded CSS, no scripts, remote fonts, or required network assets.

Keep strategy and design-lock notes in working memory. Do not create extra deliverables unless the host explicitly permits them. `index.html` is the final visual authority, not an export of `report.md` through a generic template.

## Non-negotiable pipeline

Run these stages serially:

1. **Evidence inventory** — separate source families, individual evidence items, requested dimensions, supported claims, uncertainties, dates, units, and contradictions. Never call report sections “independent evidence tracks.”
2. **Report strategist** — determine audience, reading occasion, desired use, one communication mode, one visual-style position, a dominant thesis, and a section outline.
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

## Design standard

- Treat communication mode as how the report argues and visual style as how it looks. Lock them independently.
- Alternate `anchor`, `dense`, and `breathing` sections according to meaning; do not stamp every section into the same card grid.
- Use a visualization only when its semantic fit is stronger than prose or a table. Record the selected form and rejected runner-up in the section plan.
- Use one coherent token system and a small composition vocabulary, but make each major section composition-specific.
- Avoid the default “premium dashboard” look: excessive cards, glass effects, neon gradients, arbitrary metrics, decorative charts, and repeated two-column blocks are failures.
- Preserve reading comfort: constrained prose width, strong hierarchy, responsive tables, visible focus, sufficient contrast, reduced-motion support, and useful print CSS.

## Completion gate

Read both files after writing. Reject and repair the report if any of these are true:

- the executive summary does not answer the query;
- source families, evidence items, dimensions, and sections are conflated;
- unsupported claims or orphaned citations appear;
- the page resembles raw Markdown, a dashboard template, a recovery page, or a wall of identical cards;
- heading hierarchy, overflow, mobile layout, contrast, links, tables, or print output fail review;
- the rendered hierarchy disagrees with the report thesis or a section's editorial purpose.

Emit the host's report marker only after the two final artifacts pass these gates.
