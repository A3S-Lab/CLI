# Code TUI — first-principles test plan

Living plan for **what to test** in `a3s code` TUI and **what to refuse**.
Companion to the four-capability matrix
[`capability-test-matrix.md`](./capability-test-matrix.md) (zvec / memory /
Reviewer / Moli). This document covers the **full TUI product surface**.

**Pin:** host in `crates/cli`; Core **8.5.4** (update when the CLI pin moves).

---

## 1. Gates (every case)

A case is admitted only if it answers all five:

1. **Mission** — strengthens the coding loop (user → tools → evidence → next turn).
2. **Effectiveness** — seeded truth is detected; false success is rejected.
3. **Efficiency** — demand-driven, bounded, non-blocking where that is the product claim.
4. **UX** — chrome/copy matches the product contract (not decorative).
5. **Evidence** — hermetic automated test, or a named `#[ignore]` live test.

### Refuse overfit optimization

Do **not** add or promote tests that:

| Refuse | Why |
| --- | --- |
| Prompt / help / footer **substring** as “Effect.” | Proves packaging or chrome, not product outcome |
| Snapshot entire panels for layout pixels | Brittle; does not prove mission |
| Duplicate Core unit coverage in the TUI host | Wrong ownership; test at the owning layer |
| Soft-skip live tests counted as green Effect | False completeness |
| “Always warm” index / memory / Moli binary claims | Contradicts demand-driven efficiency |
| Heuristic prose classifiers as Reviewer Effect | Overfits language; claim↔record needs record |
| One mega-test that asserts ten unrelated strings | Hides which contract actually broke |

### Kind vocabulary (honest)

| Kind | Means | May claim |
| --- | --- | --- |
| **Effect.** | Seeded behavior / verdict / persistence under hermetic control | Product outcome |
| **Effect (fixture/mock)** | Deterministic detect without live LLM | Outcome under mock |
| **Contract.** | Prompt packaging, schema, DATA-vs-instructions | Host contract only |
| **Effic.** | Laziness, isolation, bounds, non-blocking | Efficiency invariant |
| **UX** | User-visible contract chrome | Copy / chip / bus only |
| **Live** | Real model / network / ACL | Never hermetic green |
| **Bound** | Truncation / caps | Safety envelope |

Promote Contract. → Effect. only when a detect/fixture/live row exists.

---

## 2. Already matrixed (do not duplicate)

Keep rows in [`capability-test-matrix.md`](./capability-test-matrix.md):

| Surface | Mission (one line) | Overfit refuse |
| --- | --- | --- |
| BM25 / grep / zvec | Local lexical retrieval; grep never pays durable index | Grep chrome ≠ “index always ready” |
| ReMe-like memory | Durable recall; lazy I/O; one shared `Arc` | Panel tip ≠ store roundtrip |
| Sticky Reviewer + git `/review` | Claim↔record Gate vs git CodeReview lane | Prompt inclusion ≠ detect; incomplete ≠ clean |
| Moli `web_search` | Default headless SERP; fail-closed + cascade | Default string ≠ live SERP |

Runbook: matrix §Runbook + `./scripts/dogfood-rc.sh`.

---

## 3. Expanded TUI surfaces (plan)

Status: `covered` (matrix or strong hermetics) · `partial` · `gap` · `defer`.

For each surface: **must-have** cases only. Optional polish is deferred.

### 3.1 Deep research (`?` / research workflow)

| ID | Kind | Case | Expected | Status | Evidence / target |
| --- | --- | --- | --- | --- | --- |
| DR1 | Effect. | Fail-closed publish | Incomplete / empty evidence cannot publish a clean synthesis report | covered | `empty_acquisition_publishes_honest_artifacts_without_report_generation` (`publication=no_evidence`, skips report gen); `failed_exact_source_audit_downgrades_and_does_not_publish_artifact_head` |
| DR2 | Effic. | Child cancel / settle | Interrupt settles without leaking child work as success | covered | `deep_research_interruption_settles_only_current_children_and_never_opens_report`; `deep_research_completion_cancels_live_children_before_closing_footer` |
| DR3 | Contract. | Evidence-first loop | Plan → retrieve → assess order preserved | covered | `inquiry_publication_requires_the_completed_audit_phase` (Outlining ≠ report completion); DR1 publication fail-closed hermetics |
| DR4 | UX | Research mode chrome | `?` / research mode ≠ sticky Reviewer wording | covered | `composer_prompt_glyph_modes_are_exclusive_priority` (`?` vs `!`); `sticky_arming_skips_deep_research_sleep_and_goal` (research loop ≠ sticky reviewer) |
| DR5 | Live | End-to-end inquiry | Ignored; soft-skip ≠ green | defer | named ignored only |

**Refuse:** `contains("DeepResearch")` as proof of evidence selection quality.

### 3.2 Goal / loop

| ID | Kind | Case | Expected | Status | Evidence / target |
| --- | --- | --- | --- | --- | --- |
| G1 | Effect. | Goal progress event-gated | Plan progress / wrong goal / user-turn events do not mark goal complete | covered | `unverified_first_iteration_continues_and_second_can_complete`; `only_the_current_extracted_goal_can_latch_achievement`; `user_turn_goal_events_cannot_close_the_host_goal_iteration` |
| G2 | Effic. | Goal vs sticky Reviewer | Sticky arming skips while goal active (R15) | covered | `sticky_arming_skips_*_goal` |
| G3 | Contract. | Loop L1 report-only policy | Scaffold / audit prompts stay policy-true | covered | `loop_run_prompt_enforces_l1_report_only_policy`; `init_loop_scaffolds_state_budget_skills_and_audits_l1_ready` |
| G4 | UX | Goal/loop meter | Footer authority matches run state | covered | `footer_medium_width_keeps_live_goal_before_optional_identity`; `footer_wide_width_keeps_all_optional_detail_after_mode_and_context`; `active_retrieval_precedes_goal_while_ready_retrieval_follows_it` |
| G5 | Live | Multi-iteration goal | Ignored | defer | — |

**Refuse:** footer `◎ goal` substring as progress persistence proof.

### 3.3 Plan / Ask / Auto / Yolo modes

| ID | Kind | Case | Expected | Status | Evidence / target |
| --- | --- | --- | --- | --- | --- |
| P1 | Effect. | Plan is read-only | Tool writes denied even with session grants | covered | `plan_mode_is_read_only_even_with_session_grants` |
| P2 | Contract. | `/ask` ≡ Plan | Alias arms Plan, no sixth Shift+Tab state | covered | mode ring / help |
| P3 | Effic. | Queued turn mode freeze | Submission mode immutable for queued item | covered | `queued_turn_mode_stays_frozen_when_composer_mode_changes` |
| P4 | UX | Mode chip ring | agent→plan→reviewer→auto→yolo | covered | mode cycle tests |

**Refuse:** chip color/name alone as tool-deny proof (need P1).

### 3.4 Permissions / approvals / sandbox

| ID | Kind | Case | Expected | Status | Evidence / target |
| --- | --- | --- | --- | --- | --- |
| A1 | Effect. | HITL allow/deny | FIFO pending; scoped resolve never grants siblings | covered | `concurrent_tool_approvals_are_kept_in_fifo_order`; `scoped_approval_never_grants_other_pending_requests`; `out_of_order_tool_terminal_events_do_not_skip_the_fifo_head` |
| A2 | Effic. | Mode cycle ≠ rewrite active turn | Policy change applies to next turn | covered | `mode_cycle_*_without_touching_grants` |
| A3 | Effect. | Sandbox fail-closed | Bash denied until verified ready | covered | `deferred_sandbox_handle_stays_fail_closed_until_readiness_is_published` |
| A4 | UX | Approval countdown chrome | Timeout → Skip & tell contract | covered | `approval_surface_uses_cursor_style_selection_and_hotkeys`; `countdown_disabled_omits_bar_and_auto_reject_footer`; `approval_timeout_env_defaults_clamps_and_disables` |

**Refuse:** “Allow (y)” label as ACL persistence proof.

### 3.5 Diff review (Ctrl+G)

| ID | Kind | Case | Expected | Status | Evidence / target |
| --- | --- | --- | --- | --- | --- |
| D1 | Effect. | Latest-turn writes only | Overlay lists successful path edits from last turn | covered | `latest_turn_skips_failed_and_keeps_last_path_write`; `latest_turn_empty_without_user_or_edits` |
| D2 | Contract. | Seed composer | `i` seeds follow-up about selected path | covered | `move_file_and_seed_draft`; `seed_and_close_are_host_effects_for_app_wiring` |
| D3 | UX | Overlay open/close | Esc closes; does not steal main stream wrongly | covered | `move_file_and_seed_draft` (Esc→Close); `seed_and_close_are_host_effects_for_app_wiring` |

**Refuse:** path header chrome as multi-refresh selection correctness.

### 3.6 Queue / sleep / ctx

| ID | Kind | Case | Expected | Status | Evidence / target |
| --- | --- | --- | --- | --- | --- |
| Q1 | Effect. | FIFO pending turns | Order + mode preserved | covered | `admission_failure_restores_the_original_lane_position`; `matching_lane_claim_is_consumed_at_most_once`; mode freeze via P3 `queued_turn_mode_stays_frozen_when_composer_mode_changes` |
| Q2 | Effic. | Idle signals vs sticky | Main idle fires even if sticky would arm | covered | `main_idle_signals_fire_even_when_sticky_*` |
| S1 | Effect. | Sleep → shared memory Arc | Consolidation writes `App.memory_store` | covered | `sleep_consolidation_persists_through_shared_store_arc` |
| C1 | Effect. | Ctx attach / promote | Bounded attach; promote durable with provenance | covered | `promoted_memory_roundtrips_through_the_real_store` |
| C2 | Bound | Ctx budgets | Oversized / hang fixtures fail closed | covered | ctx budget fixtures |

**Refuse:** “2 queued” status string as FIFO proof.

### 3.7 Fork / rewind / worktree / relay

| ID | Kind | Case | Expected | Status | Evidence / target |
| --- | --- | --- | --- | --- | --- |
| F1 | Contract. | Fork command shape | Parser / quoting stable | covered | `fork_parser_keeps_session_compatibility_and_adds_worktree_isolation`; `worktree_launch_command_quotes_paths_and_session_ids` |
| F2 | Effect. | Worktree isolation | `.a3s-worktrees` handoff validated | covered | `handoff_captures_committed_uncommitted_and_untracked_content`; `cleanup_guidance_never_suggests_force`; `parser_exposes_bounded_lifecycle_actions` |
| F3 | Effect. | Rewind last turn | Checkpoint restore bounds + warning | covered | `rewound_session_uses_the_pre_turn_conversation_under_a_new_id`; `warnings_remain_bounded_and_readable`; `rewind_is_idle_only_and_listed` |
| F4 | UX | Relay selection | Pin/search preserves selection across refresh | covered | `semantic_selection_survives_refresh_reordering_and_metadata_changes`; `transcript_path_keeps_selection_stable_after_an_append`; `each_source_tab_retains_its_own_selection` |

**Refuse:** resume command string as filesystem isolation proof.

### 3.8 Skills / `/use` / composer

| ID | Kind | Case | Expected | Status | Evidence / target |
| --- | --- | --- | --- | --- | --- |
| U2 | Effect. | Skill enter attaches sticky name | Menu enter → sticky skill contract | covered | `sticky_skill_is_selected_even_without_dollar_mention`; `sticky_skill_outranks_disabled_skill_list`; `alt_or_meta_enter_attaches_sticky_skill_plain_enter_does_not` |
| U3 | Contract. | `/use` packages confirm identity | Digest / apply identity checks | covered | `apply_result_requires_the_confirmed_identity_and_advanced_generation`; `review_rendering_preserves_the_complete_digest_and_width_bound` |
| K1 | Contract. | Composer glyphs | `!` shell / `?` research exclusive priority; `$` skill mentions stay visible | covered | `composer_prompt_glyph_modes_are_exclusive_priority`; `composer_prompt_glyph_defaults_to_agent_chevron`; `dollar_mentions_select_enabled_skills_without_rewriting_the_visible_prompt` |
| K2 | Effic. | Width / paste bounds | Paste pills + width budget | covered | `paste_strip_rows_stay_within_width_budget`; `large_paste_thresholds_cover_lines_or_chars` |

**Refuse:** help “prefer /use packages” as registry trust proof.

### 3.9 Status / display / session lifecycle

| ID | Kind | Case | Expected | Status | Evidence / target |
| --- | --- | --- | --- | --- | --- |
| ST1 | UX | Status authority fields | Mode / ctx / goal / queue present | covered | `status_report_exposes_session_authority_and_resume_without_overflow` |
| ST2 | UX | Display density | Compact/zen do not drop required authority | covered | `footer_narrow_width_uses_compact_mode_and_context_fallback`; `footer_wide_width_keeps_all_optional_detail_after_mode_and_context`; `retrieval_footer_chip_never_overflows_narrow_terminals` |
| ST3 | Effic. | Clear / quit settle | Quit cancels without starting auto-continue | covered | `quitting_suppresses_loop_auto_continue_scheduling`; `graceful_quit_settles_a_completed_stream`; `graceful_quit_aborts_a_stream_after_its_own_deadline` |

**Refuse:** meter substring as live token-count authority.

---

## 4. Optimization path (phased; refuse thrash)

Order is **mission risk first**, not “easiest green test”. Exit a phase only when
its Exit gate is proven; do not start the next phase’s UX chrome early.

```text
Stabilize Z/M/R/B ──► Safety Effect ──► Research/Goal false-complete
        │                    │                      │
        ▼                    ▼                      ▼
   RC dogfood          Plan/Sandbox/HITL      DR1 + G1 hermetics
        │                    │                      │
        └────────────────────┴──────────► Topology on regression only
                                         │
                                         ▼
                              Defer live / optional product
```

### Phase A — Hold the line (no new chrome)

| Do | Don’t |
| --- | --- |
| Keep dogfood 1→7 green on every Reviewer/Z/M/B change | Add footer/help substring “Effect.” rows |
| Human PTY once per RC: sticky false-pass + `/review` + open inject | Count R-live / M11 soft-skip as green |
| Fix matrix evidence names when tests rename | Port Desktop science citation rubric into TUI |

**Exit (proven 2026-09-08):** dogfood 1→7 all green; matrix R/Z/M/B evidence strings resolve.

### Phase B — Safety Effect. (next code investment)

| ID | Work | Proof |
| --- | --- | --- |
| P1 | Keep Plan write-deny under grants | `plan_mode_is_read_only_even_with_session_grants` |
| A3 | Sandbox fail-closed until verified | `deferred_sandbox_handle_stays_fail_closed_until_readiness_is_published` |
| A1 | HITL FIFO / scoped settle | `concurrent_tool_approvals_are_kept_in_fifo_order`; `scoped_approval_never_grants_other_pending_requests` |

**Exit (proven 2026-09-08):** P1 + A1 + A3 hermetics green.
**Refuse:** approval hotkey / overlay layout tests as A1 Effect.

### Phase C — False-complete killers (research + goal)

| ID | Work | Proof |
| --- | --- | --- |
| DR1 | Empty / failed evidence cannot publish clean synthesis | `empty_acquisition_publishes_honest_artifacts_without_report_generation`; `failed_exact_source_audit_downgrades_and_does_not_publish_artifact_head` |
| G1 | Progress / wrong goal / user-turn ≠ goal complete | `unverified_first_iteration_continues_and_second_can_complete`; `only_the_current_extracted_goal_can_latch_achievement`; `user_turn_goal_events_cannot_close_the_host_goal_iteration` |

**Exit (proven 2026-09-08):** DR1 + G1 named tests green.
**Refuse:** more DeepResearch/Goal UX copy tests as false-complete proof.

### Phase D — Topology (covered by existing hermetics)

| ID | Evidence |
| --- | --- |
| F2 worktree isolation | `handoff_captures_*`; `cleanup_guidance_never_suggests_force` |
| F3 rewind restore | `rewound_session_uses_the_pre_turn_conversation_under_a_new_id`; `warnings_remain_bounded_and_readable` |
| Q1 queue FIFO + mode | lane claim hermetics + P3 |

**Refuse:** proactive topology UX expansion “for coverage %”; soft-skip live ≠ Effect.

### Phase E — Explicit defer (product, not test thrash)

| Item | Why deferred |
| --- | --- |
| Live Z7 / B6 / R-live / M10–M11 | Network/model; soft-skip ≠ Effect. |
| Periodic sticky review | Spam risk; not Gate path |
| `/review-reply` | Don’t overload git `/review` |
| Desktop↔CLI shared review crate | After TUI Gate path stays green |
| Pixel / full-panel snapshots | Overfit |

### Phase map → owners

| Phase | Primary owner | Doc update |
| --- | --- | --- |
| A | CLI host CI + RC checklist | matrix + dogfood |
| B | `crates/cli` permissions/sandbox | this plan status → covered |
| C | deep_research + goal modules | this plan DR1/G1 → covered |
| D | fork/worktree/rewind/queue | covered — further invest only on regression |
| E | product roadmap | first-principles review follow-ups (defer) |

---

## 5. Priority backlog (short form)

**A–C Exit gates proven** (dogfood + P1/A1/A3 + DR1/G1). Honesty promotions
(existing Effect hermetics only, no new chrome tests): D1 / C1 / U2 → `covered`.
**S1** → `covered` via `sleep_consolidation_persists_through_shared_store_arc`.
**Honesty promotions (named existing hermetics only):** DR3 / G3 / D2 / D3 /
Q1 / F1–F4 / U3 / K1 / DR4 / G4 / A4 / ST1 / ST2 → `covered`.
Footer density tests updated to the two-line prompt-footer contract (mode +
ctx survive narrow widths; optional identity may drop). Remaining path:

- **E** defer live / optional product / pixel thrash / `/review-reply`
- RC: human PTY sticky false-pass + `/review` + open inject (still manual)
- **Refuse:** inventing soft-skip live as Effect.; pixel/full-panel snapshots

Do **not** expand UX chrome rows until the matching Effect/Effic row exists or is
explicitly marked Contract/UX only.

---

## 6. How to add a case

1. Name the **mission** in one sentence.
2. Pick **Kind** from §1 (default to Contract. or UX if unsure).
3. Write **Expected** as a falsifiable outcome.
4. Point **Evidence** at an existing test or add a **target** name — no silent “covered by vibe”.
5. State one **Refuse** line (what substring/chrome must not be mistaken for Effect.).
6. Place the case in the correct **§4 phase** (not “wherever green is easy”).
7. Update this plan **and** (if Z/M/R/B) the capability matrix.

---

## 7. Related docs

- [`capability-test-matrix.md`](./capability-test-matrix.md) — Z/M/R/B living rows + runbook
- [`capability-first-principles-review.md`](./capability-first-principles-review.md) — audit verdict
- [`v8.4.0-capability-integration-tests.md`](./v8.4.0-capability-integration-tests.md) — RC dogfood for the four caps
- [`reviewer-mode.md`](./reviewer-mode.md) — sticky vs git product split
- [`deep-research-product-validation.md`](./deep-research-product-validation.md) — research validation notes
