---
name: okf
description: "Compile the project's knowledge into an Open Knowledge Format (OKF) v0.2 bundle — a directory of cross-linked Markdown concept files under .a3s/kb/wiki/ (the LLM-wiki pattern). Use when the user asks to build, compile, or refresh the knowledge base, wiki, or project docs, or mentions $okf. Read the codebase plus existing notes, write OKF concepts with required `type` frontmatter and standard Markdown links, and recompile incrementally from recorded provenance."
kind: instruction
allowed-tools: "read(*), grep(*), glob(*), ls(*), write(*), edit(*), bash(*), parallel_task(*), task(*)"
---

# Knowledge compilation → Open Knowledge Format (OKF)

Compile this project's knowledge — its code plus any existing `.a3s/kb/` notes —
into an **OKF v0.2 bundle**: a directory of Markdown *concept* files the human
browses (in `/kb` or any editor) and that you read back as context. Google's Open
Knowledge Format formalizes exactly this LLM-wiki pattern — "just markdown, just
files, just YAML frontmatter." It is a *compile*: sources in, a cross-linked
bundle out, rebuilt incrementally — not a one-shot dump.

## Output contract — an OKF bundle, written ONLY here

- The bundle root is **`.a3s/kb/wiki/`** (create it if missing); everything you
  write lives under it. NEVER touch human-authored notes (`.a3s/kb/` *outside*
  `wiki/`) or the agent memory — link to them, don't rewrite them.
- **One file per concept.** A *concept* is anything worth capturing: a module,
  crate/package, data model, key abstraction, architecture decision, runbook, or
  API. The **file path is the concept's identity** (`modules/box-runtime.md`).
  Group related concepts in subdirectories.
- **Every concept file is Markdown with YAML frontmatter. OKF requires exactly one
  field — `type` — plus these standard optional fields:**
  ```yaml
  ---
  type: Rust Crate                 # REQUIRED: the concept's kind (free-form string)
  title: a3s-box                   # optional
  description: Docker-like MicroVM runtime for Linux OCI workloads.   # optional
  resource: "workspace:crates/box/" # optional producer URI for the canonical workspace source
  tags: [runtime, microvm]         # optional
  generated: { by: a3s-okf-compiler/0.2, at: 2026-06-30T12:00:00Z }
  # OKF permits producer extensions; source_digest drives incremental recompiles:
  source: compiled                 # producer extension: agent-generated content
  sources:
    - id: runtime-source
      resource: "workspace:crates/box/src/runtime.rs"
      title: Runtime implementation
    - id: runtime-readme
      resource: "workspace:crates/box/README.md"
      title: Runtime documentation
  source_digest: <hash of the concatenated sources>
  ---
  ```
- **Links are standard Markdown links, not `[[wikilinks]]`.** OKF turns the
  directory into a graph via normal links: reference another concept with
  `[a3s-box-cri](/modules/box-cri.md)` (bundle-relative, preferred) or a standard
  relative path such as `[neighbor](./neighbor.md)`, and reference code outside
  the bundle with an explicit producer URI such as
  `[runtime.rs](workspace:crates/box/src/runtime.rs#L42)`. Structured frontmatter
  `sources` is authoritative; use footnotes keyed to `sources[].id` for
  claim-level attribution. A human-readable `## Sources` summary is optional and
  must not replace that structured provenance.
- **Reserved filenames:** `index.md` and `log.md` are navigation/history files,
  not concepts. Generate an `index.md` for the bundle root and each concept
  directory as a compiler convention. Only the bundle-root `index.md` may have
  frontmatter, and when present it contains exactly `okf_version: "0.2"`.
  Nested indexes have no frontmatter. Optionally keep a frontmatter-free top-level
  `log.md` with chronological compile history.

## Pipeline

1. **Survey.** Map the repo before writing a word: `ls`/`glob` the top level and
   key dirs; read the root README + manifest(s) (`Cargo.toml`, `package.json`, …)
   and each module's entry point + README. In a monorepo, each crate/package is a
   module concept. Read existing `.a3s/kb/` notes so the bundle complements and
   links to them — never duplicates.
2. **Plan the bundle.** Choose a BOUNDED concept set + directory layout (e.g.
   `modules/`, `concepts/`, `decisions/`), each directory with an `index.md`, plus
   the root `index.md`. Deterministic kebab-case slugs. Show the planned layout to
   the user before generating.
3. **Generate concepts.** Per concept: read its sources, then write an OKF file —
   required `type`, the standard fields, and a synthesized explanation grounded
   entirely in what you read (key types/functions with `[file](path#Lline)` links,
   connections to other concepts with `[name](/dir/other.md)`). Fill structured
   `sources` entries (`id`, required `resource`, optional `title`) and
   `source_digest` honestly; use matching footnotes when attributing individual
   claims. Use bundle-relative or relative standard Markdown links inside the
   bundle. **Fan out with
   `parallel_task`** (one concept per subtask) when available; else do them one at
   a time.
4. **Index.** Write each directory's `index.md` and the root `index.md` last,
   linking every concept with a one-line summary, so the bundle is a navigable
   graph, not a flat pile.
5. **Verify.** Parse every concept's YAML, require a non-empty scalar `type`, and
   check every Markdown link. Broken internal links are OKF diagnostics rather
   than hard conformance failures, but fix them when possible and report every
   unresolved target. Report concept, rebuilt, skipped, failed, and warning counts.

## Incremental recompile (this is what makes it a *compile*)

On a re-run, before regenerating a concept read its frontmatter `source_digest`
and recompute the digest of its `sources` (e.g. `cat <sources> | shasum -a 256`).
Unchanged ⇒ **SKIP** it. Only regenerate concepts whose sources changed, plus the
affected `index.md` files. Report rebuilt vs. skipped — a dependency-tracked
rebuild that keeps recompiling cheap and the bundle fresh after code changes.

## Rules

- **Ground every claim** in a file you read; link file+line; mark genuine
  uncertainty ("appears to …"); never invent an API, type, or flow. A hallucinated
  concept is worse than none.
- **No secrets** — never copy tokens/keys/`.env` values into a file.
- **Bound the run** — document concepts + modules, not every file; for a very
  large repo compile by area and report what you covered.
- **Stay in your lane** — `source: compiled` on every file; you own
  `.a3s/kb/wiki/`, the human owns `.a3s/kb/*.md`. Don't clobber either.

> OKF v0.2 — canonical spec, conformance criteria, and sample bundles:
> `https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md`.
> A concept's only universally required metadata key is `type`; compiler
> provenance and per-directory indexes are producer conventions.
