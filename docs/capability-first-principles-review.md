# First-principles review — capability / Reviewer work

Audit date: 2026-09-08 (updated after follow-up optimization pass 3). Scope:
sticky Reviewer claim-vs-record, capability test matrix, zvec/BM25 demand path,
ReMe-like memory, default Moli.

Gates (Rule 0–2): mission fit · ownership · demand-driven efficiency · no
false completeness · prune dead coupling.

---

## Verdict

**Keep** the product split (sticky reply verifier vs git `/review`), the
async `ReviewerLane`, `TurnEvidenceBundle`, and the capability matrix as the
test contract.

**Fixed in the original review:**

1. Manual `/review` no longer inherits sticky **reply-verifier** host posture.
2. Capturing a git/code review no longer clears independent sticky
   `open_reply_findings`.

**Fixed in optimization pass 1:**

1. Core docs no longer claim Grep opens durable zvec; BM25-only demand path.
2. Host Memory write/sync paths reuse `App.memory_store` (`LazyFileMemoryStore`
   Arc).
3. CLI `a3s-code-core` pin aligned to `=8.5.1`.
4. Docs state Moli default is CLI/`scientific`, not Core `default=local-code`.

**Fixed in optimization pass 2:**

1. `/memory` panel browse prefers the shared store Arc.
2. `/ctx save` and `/sleep` refresh asynchronously through the same load path.
3. Named ignored live durability gate (M10).

**Fixed in optimization pass 3 (this update):**

1. Dogfood #5 includes M9 shared-store browse hermetics
   (`memory_panel_loads_from_shared_store`, `promoted_memory_roundtrips`).
2. Named ignored live LLM extract gate (M11) with soft-skip on judge decline /
   provider block — hermetic Core extract suites remain the effectiveness
   contract.
3. Integration/plan docs pin and backlog rows updated to Core **8.5.1** and
   M8–M11.

**Keep watching (non-blocking):**

| Item | Why |
| --- | --- |
| `AgentStyle::CodeReview` for sticky | Host guidelines carry the real claim-vs-record contract. |
| Matrix “effect” rows that only assert prompt text | Evidence packaging ≠ LLM verdict correctness. |
| Human PTY RC dogfood | Hermetics cannot catch terminal feel / key timing. |

---

## Capability-by-capability

### Reviewer / zvec / Memory / Moli

Unchanged mission verdicts from prior passes. Memory now has M8–M11 evidence
(shared Arc browse + live durability + optional live extract soft-skip).

---

## What would fail the gate if claimed complete incorrectly

- Claiming sticky Reviewer “detects false test-pass claims” solely from prompt inclusion tests.
- Claiming zvec is “always ready at TUI start”.
- Claiming any `a3s-code-core` embed gets Moli without `headless-search`.
- Shipping dual `FileMemoryStore::new` / dual primary `index.json` browse on the live session memory directory.
- Treating M11 soft-skip as a green effectiveness proof of live extract.

---

## Follow-ups (optional, not blocking)

1. Human PTY RC dogfood once per release candidate.
2. Tighten M11 only if product requires hard-fail live extract (expect flakiness).
