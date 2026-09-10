# First-principles review — Code TUI capability testing

Audit date: 2026-09-08 (expanded to full TUI test planning). Scope: living
matrices, refuse-overfit policy, sticky Reviewer Gate path, zvec/BM25, memory,
Moli, and **other TUI surfaces** (deep research, goal/loop, plan, approvals,
diff, queue, topology, composer).

Gates (Rule 0–2): mission fit · ownership · demand-driven efficiency · no
false completeness · prune overfit tests.

---

## Verdict

**Keep** the four-capability matrix
([`capability-test-matrix.md`](./capability-test-matrix.md)) as the Z/M/R/B
contract with Effect / Contract / Effic / UX honesty, plus §5 Flow digest-fold
and §6 objective named surfaces (RemoteUI / skills / `/kb`+$okf / DW / Reviewer).

**Add** the full-surface plan
([`tui-first-principles-test-plan.md`](./tui-first-principles-test-plan.md)):
every major TUI workflow gets Mission + must-have cases + an explicit
**Refuse** line. Expand Effect. coverage only for false-completeness killers
(fail-closed, isolation, shared Arc, read-only Plan, sandbox), not chrome thrash.

**Refuse / prune:**

- Prompt-substring or footer chrome as “Effect.”
- Soft-skip live tests counted as green effectiveness
- Pixel / full-panel snapshots as mission proof
- Duplicating Core unit tests inside the TUI host
- Porting Desktop scientific citation rubrics into Code TUI Reviewer tests
- Overfit language heuristics as Reviewer detect

---

## Landed (testing architecture)

1. Sticky path: Gate + protocol executor; R1–R6 **Contract.**; R13-mock / R20 /
   R21 **Effect.**; lanes isolated.
2. Grep never opens durable zvec; BM25 demand path; memory lazy + shared Arc;
   Moli default under CLI/`scientific`.
3. Full TUI first-principles test plan with refuse-overfit table and prioritized
   backlog (DR/G/P/A/D/Q/F/U/K/ST gaps called out honestly as partial/gap/defer).

---

## What would fail the gate if claimed complete incorrectly

- Claiming sticky Reviewer “detects false test-pass claims” solely from prompt inclusion tests.
- Publishing a clean sticky verdict when evidence is incomplete (must fail-closed).
- Claiming zvec is “always ready at TUI start”.
- Claiming any `a3s-code-core` embed gets Moli without `headless-search`.
- Shipping dual primary memory browse on the live session memory directory.
- Treating M11 / R-live soft-skip as a green effectiveness proof.
- Claiming “all TUI features are Effect-tested” because help strings have unit tests.
- Labeling default-config / policy-packaging asserts (B1/B5) or vague panel suite
  pointers (M5) as Effect. — demoted to Contract. (2026-09-08 honesty pass).
- Shipping an always-`true` R21 helper as executor-independence proof (replaced with
  sticky Gate identity + git-only CodeReview style asserts).

---

## Follow-ups (ordered; no thrash)

Optimization path:
[`tui-first-principles-test-plan.md`](./tui-first-principles-test-plan.md) §4

- **A–C Exit proven** (2026-09-08): dogfood; P1/A1/A3; DR1/G1 Effect hermetics.
- **Honesty pass:** demoted B1/B5/M5 Effect→Contract; fixed always-true R21;
  linked existing D1/C1/U2 Effect hermetics (no chrome thrash).
- **S1 Effect landed:** `sleep_consolidation_persists_through_shared_store_arc`.
- **DR2 / P3 Effic:** interrupt settle hermetics linked; queued-mode freeze via
  `queued_turn_mode_stays_frozen_when_composer_mode_changes` (not panel chrome).
- **ST3 / K2 Effic:** quit clears loop budget + `should_schedule_loop_auto_continue`
  gate; paste strip stays within width (`paste_strip_rows_stay_within_width_budget`).
- **Honesty batch:** remaining matrix rows DR3/G3/D2–D3/Q1/F1–F4/U3/K1/DR4/G4/A4/ST1/ST2
  linked to named hermetics; footer ST1/ST2 tests repaired for two-line footer
  (authority-preserving density, not substring Effect thrash).
- **Phase D:** F2/F3/Q1 covered — invest further only on regression flakes.
- **RC dogfood:** steps 1/3/5 also pin footer density, quit≠loop-continue, and
  sleep→shared-store Arc hermetics (`scripts/dogfood-rc.sh`).
- **Next / RC:** Phase E deferred; human PTY sticky false-pass + `/review` + open inject.
- **Prune pass (2026-09-08):** removed dead TUI/config wrappers (`App.anim`, unused
  `flow_dir`/`agent_dir`/`mcp_dir`, orphan `open_window`/`humanize`/`write_asset_acl`,
  unused DeepResearch spawn shim, unused progressive HTTP entrypoints, unused
  `OsService`/`MessageTone`/`RuntimePolicy` variants). Kept live execute/journal/
  RemoteUI/`open_window_with` and Effect hermetics. CLI Use Flow hermetics no
  longer depend on a Core `PreparedCapability::into_value` escape hatch.
- **Prune pass 2 (docs + orphans):** deleted unused `tui/os/progressive.rs` module;
  removed unused chrome helpers (`cancel_pending_picker`, `os_required_*`); scrubbed
  `cli-reference.md` / knowledge notes so five-pack slash surfaces are not
  documented as live; config template comments match remaining `skill_dir` only.
  Synced slash audit/registry for `/ask`/`/plan`/`/unstick`, aligned compact-chrome
  and palette tests with live tokens, and fail-closed the removed `parallel_task`
  alias in the Core 6.8 TUI integration hermetic.
- **Prune pass 3 (Use Flow catalog honesty):** non-resident `UseFlowCatalog` /
  `InstalledFlowRuntime::{run,get,events,latest_for_design}` / design-parse path
  gated `#[cfg(test)]`. Production FullCompatibility keeps
  `projection_adapter` → `prepare_projected_binding` only. Docs already state
  `a3s code flow run` is not wired.
- **Prune pass 4:** deleted unused `ProjectedExecutableTool` getters + unused
  `InstalledFlowRun::status_label` / test `make_executable`; removed dishonest
  README `/flow run` rows; corrected `InstalledFlowRuntime` docs to projection-only;
  removed dead `ReviewerOrigin` dual-use helper (sticky tests call
  `background_reviewer_prompt_slots()`; git uses `git_review_side_session_prompt_slots`).
  One-shot gate: `scripts/verify-capability-regression.sh`. Matrix §6 lists
  objective named surfaces.
- **Core digest-fold (published):** Flow step identity soft-folds inputs above
  64 KiB to `sha256`+`bytes` (512 KiB hard ceiling). CLI pins
  `a3s-code-core` git rev `eda36019d9c6c27c72d573f98125d5bc31e0e712` (`=8.5.5`). Proof:
  `scripts/verify-capability-regression.sh --require-published` and
  `scripts/prove-published-core-has-digest-fold.sh`.
- **Prune pass 5:** deleted orphan `research/questioning/` (empty after hermetic
  removal); gated `AcceptedClaim`/`AcceptedEvidence` + ledger imports
  `#[cfg(test)]` (production keeps `AcceptedSource` for journal report audit);
  publish-pin scripts fail-closed via `--require-published` so local path-patch
  green cannot be mistaken for published Core.
- **Honesty pass 6:** scrubbed residual five-pack / live `/flow` claims in
  README EN/ZH, Use platform §7, product-design command tree, KB layout cite,
  loop audit tip, `engage_autonomy` docs, and hermetic Flow rustdoc — production
  path remains projection-only.
