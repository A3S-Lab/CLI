# Capability test matrix (first principles)

Scope: a3s-code TUI / Core **8.5.5** paths for **zvec-grep (BM25)**, **ReMe-like memory**,
**Reviewer**, and **default Moli web search**.

**Full TUI surface plan (all slash/workflows, refuse overfit):**
[`tui-first-principles-test-plan.md`](./tui-first-principles-test-plan.md)

**Release plan:** [`v8.4.0-capability-integration-tests.md`](./v8.4.0-capability-integration-tests.md)
(backlog, UX checklist, honest Effect-prompt vs Effect-detect split).

## First-principles gates

For each capability, every case must answer:

1. **Mission** — does this strengthen the product’s core loop?
2. **Effectiveness** — seeded truth is detected; false success is rejected.
3. **Efficiency** — heavy work is demand-driven, bounded, and non-blocking.
4. **UX** — chrome/copy matches the product contract (sticky ≠ git review, etc.).
5. **Evidence** — a hermetic automated test (or named ignored live test) proves it.

**Refuse overfit:** prompt/footer substring ≠ Effect.; soft-skip live ≠ green;
duplicate Core coverage in the TUI host; layout pixel snapshots as mission proof.
Full refuse table: [`tui-first-principles-test-plan.md`](./tui-first-principles-test-plan.md) §1.

| Capability | Mission |
| --- | --- |
| BM25 / zvec | Local lexical retrieval without paying index cost until needed |
| Memory | Durable recall/extract without first-frame disk tax |
| Reviewer | Independent claim↔record check without blocking the main agent |
| Moli `web_search` | Default headless SERP with fail-closed / cascade fallback |
| Flow step identity | Admit DeepResearch multi-source selector shards without plaintext retention |
| RemoteUI | Embed trusted view URLs / local file previews without leaving the workbench |
| skills / `skill_dir` | Discover and materialize instruction skills (including `$okf`) |
| `/kb` + `$okf` | Personal vault + compiled OKF wiki without five-pack package slash surface |
| WorkBuddy account models | Use signed-in WorkBuddy AI / classic account models without copying tokens |

---

## 0. WorkBuddy live ladder (account LLM)

**Mission:** prove the coding loop under a real WorkBuddy account model —
especially the account router `workbuddy/auto` — without treating soft-skip as
green.

**Product mapping:** WorkBuddy AI (`~/.workbuddy-ai`, `WorkBuddy AI.app`) is
preferred when signed in; classic `~/.workbuddy` / `WorkBuddy.app` remains the
fallback. Route family stays `workbuddy/<id>` (one provider, not dual tabs).

**Refuse:** Desktop UX parity as proof; counting `model list` chrome without
exec; requiring classic `.workbuddy` when only AI is signed in.

| ID | Kind | Case | Expected | Evidence |
| --- | --- | --- | --- | --- |
| W0 | Effic. | AI config/app discovery | Prefers signed-in `.workbuddy-ai` + AI.app | Hermetic: `prefers_signed_in_workbuddy_ai_*`, `falls_back_to_classic_*`, `workbuddy_ai_config_dir_is_discovered_*` (2026-09-11 green). CLI `1d00aa5`. |
| W1 | Effect. | `workbuddy/auto` selectable + one-shot | `model use` + `code exec` reply | Hermetic: `discover_model_list_keeps_auto_router_*`. Live: `model use workbuddy/auto` + exec PONG (debug `a3s`). |
| W2 | Effect. | Plan + in-workspace Read (rel/abs/`files[]`) | Allow; host `/etc` Deny | Hermetic: `plan_and_read_only_admit_*` green. Live plan/read-only: relative, abs in-workspace, and `files[]` returned `unique_token_wb_ladder_xyz`. |
| W3 | Effect. | Force write bounded file | Writes content | Live `--force`: `WRITE_OK.txt` / `WRITE_HY3.txt` (`workbuddy-ok` / `hy3-ok`). Weak prompts may claim-only — require MUST-call-Write. |
| W4 | Effect. | Sandbox bash when available | Ready + non-catastrophic Allow | Live: `a3s code sandbox status` → `macos-seatbelt` ready. |
| W5 | Effect. | Memory / session host cmds | `memory stats`, `session list` succeed | Live: `a3s code memory stats`; `a3s code session list` lists exec sessions under ladder workspace. |
| W6 | Effic. | Sensitive roots | `.workbuddy` **and** `.workbuddy-ai` denied to sandbox | Sandbox `default_sensitive_paths` includes both; pushed `a5df23d` (after reverting accidental `.a3s/skills` carve-out). |

**Live efficiency note (2026-09-11):** workspace lexical find under `workbuddy/auto` returned the seeded needle without opening a durable zvec index (only `.a3s-code/grep-trigram/stamp.txt`). Explicit BM25 demand opens `.a3s-code/index/` (on-demand). RC dogfood hermetics 1→7 green via `./scripts/dogfood-rc.sh`.

**Must-capability live under `workbuddy/auto` (2026-09-11):**

| Must | Live evidence | Status |
| --- | --- | --- |
| WorkBuddy account models | W0–W1 + AI discovery commits | Pass |
| Coding loop (read/write/sandbox/session) | W2–W5 | Pass |
| BM25 / zvec | Seeded `bm25_unique_token_orchid_77` via `search`; durable index created on demand | Pass |
| Moli `web_search` | `tool: web_search` → `2023` | Pass |
| Memory (host) | `a3s code memory stats` | Pass |
| Memory (agent write path) | Bare `<tool_call>write>` was prose-leaked; fixed by host_tools bare parser → live `tool: write` / `MEM_NOTE.txt` | Pass (after fix) |
| skills / `skill_dir` | `code exec` omitted `with_skill_dirs`; wired + hermetic + live `search_skills` → `SKILL_WB_OK` | Pass |
| Reviewer sticky | Not reachable via `code exec` (TUI `/reviewer` lane) | Blocked: interactive TUI-only under WB auto; dogfood 1→4 + sticky hermetics (R*) green 2026-09-11 |
| RemoteUI | View URL / local file embed needs interactive host (`open_remote_view`) | Blocked for WB-auto exec: no non-interactive open path; hermetics `tui::remote_ui::tests::*` 20/20 + `a3s doctor webview` Ready |
| Flow / DeepResearch | `a3s code research --local-only` under `default_model=workbuddy/auto` | Pass (lifecycle): planning→retrieval→`source_backed` publication; preserved `flow_wb_token_coral_33` from `docs/local.md`. Synthesis stage degraded by design when independent-source/depth gates unmet on single local source (`publication=source_backed`, exit 0). |
| `/kb` host | `a3s code kb add/search/stats` under WB workspace config | Pass: seeded `wb_kb_token_amber_55` → 1 hit |
| `/kb` agent search | Workspace `search`/`grep` missed `.a3s/kb` (noise + gitignore) | Pass after Core allowlist (`29290957`, CLI pin `c4e219f`): live WB-auto `tool: search` → `wb_kb_live_token_ruby_77`; re-verified on pinned binary → `wb_kb_pin_token_jade_44`. Hermetics: `scan_includes_personal_kb_*`, `grep_finds_personal_kb_*`. |
| `$okf` / `use_knowledge_search` | Managed OKF Use projection tool | Blocked for WB-auto `code exec`: tool not in WorkBuddy exec toolset until Use knowledge carrier is wired into exec (TUI path). Host Use doctor Ready after 0.3.11. |
| Use capability projection | Homebrew formula bumped to `0.3.11`; CLI rejects Use `<0.3.0` as Broken (`host_protocol_requirement`) | Pass after upgrade |

---

## 1. zvec / BM25 / grep

| ID | Kind | Case | Expected | Automated evidence |
| --- | --- | --- | --- | --- |
| Z1 | Effic. | Catalog configure / services create | No `.a3s-code/index` zvec open | `workspace_retrieval::host::tests::catalog_configure_does_not_open_durable_zvec`; Core `lifecycle::catalog_attach_does_not_open_durable_zvec` |
| Z2 | Effic. | Register builtins | Does not open durable zvec | Core `register_builtins_does_not_open_durable_zvec` |
| Z3 | Effect. | `search mode=bm25` on fixture corpus | Hits unique tokens; `index_kind` persistent when ready | Core `bm25` + `local_retrieval_automatically_uses_the_workspace_persistent_zvec_index` (`zvec-rust-fts`) |
| Z4 | Effect. | Grep path | Does **not** require zvec | Core `search::grep_*` / `grep` suite |
| Z5 | Effic. | BM25 before native ready | Falls back to catalog, not hang | Core `persistent_bm25_falls_back_to_the_catalog_before_native_ready` |
| Z6 | Effect. | Empty / invalid query bounds | Rejected or empty cleanly | Core `rejects_empty_*`, `validates_numeric_bounds_*` |
| Z7 | Live | Real LLM picks bm25 | Ignored integration | `tests/test_workspace_search_real_llm.rs` |
| Z8 | Effic. | Grep never opens durable zvec | No `.a3s-code/index`; `persistent_index` stays `None` | Core `grep_does_not_open_durable_zvec` |
| Z8-UX | UX | Grep Explored chrome | Never BM25 / persistent index / Rank | `grep_never_surfaces_bm25_or_durable_index_chrome`, `grep_explored_never_implies_bm25_or_durable_index` |
| Z9 | UX | BM25 explored label | `search mode=bm25` → BM25 chrome | `search_mode_bm25_explored_label_is_bm25_not_generic_search`; footer via `retrieval_footer_chips_*` |
| Z9+ | UX | Explored `index_kind` + freshness | Persistent/catalog; stale/unknown degraded | `bm25_surfaces_persistent_index_and_catalog_fallback`, `bm25_freshness_surfaces_stale_and_unknown` |

**Efficiency invariant:** Grep = FS/manifest only; zvec opens only on first durable BM25 demand.

---

## 2. ReMe-like durable memory

| ID | Kind | Case | Expected | Automated evidence |
| --- | --- | --- | --- | --- |
| M1 | Effic. | `LazyFileMemoryStore::new` | No disk touch / stays uninitialized | `tui::lazy_memory_store::tests::construction_does_not_touch_an_unreadable_index` |
| M2 | Effic. | First store/count/search | Opens backend once; creates `index.json` | `first_operation_initializes_the_file_backend_once`, `first_search_initializes_and_reads_the_file_backend` |
| M3 | Effect. | Store → reopen → search | Roundtrip durable item via LazyFileMemoryStore | `lazy_memory_store::store_survives_reopen_through_a_fresh_lazy_handle`; `ctx` promoted memory roundtrip |
| M4 | Effect. | Agent recall/extract | Routes through context assembly | Core `test_agent_memory_recall_*`, `test_agent_llm_memory_extraction_*` |
| M5 | Contract. | `/ctx memory` panel load path | Panel can load entries from store/fs helpers | `panels::context::memory` / `ctx` load helpers (not durable roundtrip — see M3/M7) |
| M6 | UX | `/memory` hub tip + loading | `prefer /ctx memory`; note `loading…` | `memory_hub_tip_and_loading_note` |
| M7 | Effect. | File store → timeline | Seeded `index.json` appears in panel timeline | `seeded_file_store_items_appear_in_memory_timeline` |
| M8 | Effic. | Host write paths share one store Arc | `/ctx save`, `/sleep`, forget, evolution sync use `App.memory_store` (lazy) — no second `FileMemoryStore::new` on the session dir | `synchronize_memory_store` takes `Arc<dyn MemoryStore>`; `sleep_consolidation_persists_through_shared_store_arc`; panel/ctx callers |
| M9 | Effic. | `/memory` browse prefers shared store | Panel load uses `MemoryStore::get_recent` on `App.memory_store` first; filesystem snapshot is last resort | `memory_panel_loads_from_shared_store_arc`; `promoted_memory_roundtrips_through_the_real_store` |
| M10 | Live | Store → reopen → recall under ACL | Ignored integration with `.a3s/config.acl` | Core `tests/test_memory_store_real_llm.rs` (`real_config_memory_store_survives_*`) |
| M11 | Live | LLM extract → reopen → recall | Ignored; soft-skips on judge decline / provider block | Core `real_model_memory_extract_survives_reopen_or_soft_skips` |

**Efficiency invariant:** Session always *wires* memory; I/O waits until first op.
Host UI mutations **and** `/memory` browse must reuse the same `Arc` as the agent (lazy file backend).

---

## 3. Reviewer (claim-vs-record + git `/review`)

Sticky `/reviewer` = Claude Science Auto-review analogue (coding claim↔record).
Explicit `/review` = separate git code review. Target execution: separate
`ReplyVerifierLane` (Gate + forked side-path LLM / `StructuredAuxiliaryExecutor`;
protocol rubric hermetic-only) vs `GitReviewLane` (`AgentStyle::CodeReview`
side-session). Open findings authority: Core `session_review` /
`reply.transcript` (Desktop-aligned). See [`reviewer-mode.md`](./reviewer-mode.md).

| ID | Kind | Case | Expected | Automated evidence |
| --- | --- | --- | --- | --- |
| R1 | Contract. | Failed tool vs “tests passed” claim | Prompt/evidence includes failure | `sticky_prompt_includes_failed_tool_evidence_for_claim_vs_record` |
| R2 | Contract. | Truncated / incomplete evidence | `evidence_complete: false` + inconclusive guidance | `sticky_prompt_marks_incomplete_when_truncated`, `latest_turn_evidence_marks_incomplete_for_non_terminal_tool`, `latest_turn_evidence_caps_tools_and_marks_incomplete` |
| R3 | Contract. | Sticky ≠ git scope | No working-tree inspection copy | `sticky_reply_review_targets_assistant_message_not_diff`, `sticky_prompt_forbids_rerunning_tools_and_method_judgement` |
| R4 | Contract. | Git `/review` stays code | `kind: code`, read-only | `prompt_is_read_only_and_carries_the_report_contract`, `workspace_review_prompt_kind_stays_code_not_reply` |
| R5 | Contract. | Report schema reply fields | verdict / evidence_refs / status | `parse_review_report_reads_reply_kind_and_verdict_fields`, `review_reply_contract_includes_verdict_evidence_refs_and_status` |
| R6 | Contract. | Open findings injection is DATA | Not instructions | `open_reply_findings_injection_is_data_not_instructions`, `open_reply_findings_injection_flattens_newlines_in_issue_text`, `with_open_reply_findings_prefix_injects_on_user_turns_only` |
| R7 | Effic. | Git lane vs reply lane isolation | Git jobs never claim sticky coalesce; reply lane independent | `reply_and_git_lanes_do_not_share_queued_jobs` |
| R8 | Effic. | Sticky coalesce | At most one queued sticky on reply lane | `sticky_reviews_coalesce_in_reply_lane` |
| R9 | Effic. | Serial inflight | Queue while busy on each lane | `git_review_lane_queues_while_inflight` |
| R10 | Effic. | Main stream mode | Reviewer → Default main stream | `reviewer_main_stream_stays_default`, `reviewer_mode_does_not_hijack_main_session_style` |
| R11 | Contract. | Address vs code-fix prompts | Reply address fence | `review_address_reply_prompt_is_data_not_instructions` |
| R12 | Bound | Tool args/output / tool count | Truncation + cap → incomplete | `summarize_tool_evidence_truncates_args_and_output`, `latest_turn_evidence_caps_tools_and_marks_incomplete` |
| R14 | Effect. | Code capture vs sticky opens | Git report preserves sticky opens; reply replaces | `code_capture_preserves_sticky_open_findings` |
| R15 | Effic. | Sticky arming gate | Skips non-Reviewer / deep-research / sleep / goal / no evidence | `sticky_arming_requires_reviewer_mode_and_evidence`, `sticky_arming_skips_deep_research_sleep_and_goal` |
| R20 | Effect. | Gate fail-closed | Incomplete evidence → no published clean; fail-closed chrome | `reply_verifier_gate_denies_incomplete_evidence` |
| R21 | Effect. | Executor independence | Sticky Gate identity is `cli.reply-verifier` / `cli-reply-verifier` (not `bg-review-*`); git alone sets `AgentStyle::CodeReview`; sticky slots/main style stay unset | `sticky_reply_verifier_does_not_use_code_review_style` |
| R22 | Effect. | Structured LLM fence map | Desktop-shaped findings JSON → `a3s-review` with `finding_id` | `structured_findings_map_into_a3s_review_fence` |
| R23 | Contract. | Address prompt Desktop prefix | “Address each open review finding…” + DATA fence | `review_address_reply_prompt_is_data_not_instructions` |
| R24 | Contract. | Accept reconcile by finding id | Clean / cleared → Accept; still reported → Reopen | `acceptance_reconcile_accepts_when_clean_or_id_cleared` |
| R25 | Effect. | Reply replace waives Core orphans | Pending ids not in new open set are waived on a real `SessionReviewStoreV1` | `reply_replace_waives_orphan_pending_in_session_review_store` |
| U1 | UX | Footer mode chip + ring | `reviewer` is mode slot; Shift+Tab ring includes reviewer | `reviewer_mode_chip_is_footer_mode_not_live_chip`, `reviewer_mode_does_not_hijack_main_session_style`, banner/help tips |
| R16 | UX | Lane / bus chrome | started · async bus; empty/fail / fail-closed lines | `reviewer_bus_chrome_*`, `reviewer_lane_and_finish_chrome_*`, `fail_closed_finish_does_not_capture_and_preserves_open_policy` |
| R17 | UX | Permissions / status copy | Claim↔record wording; not “async code review” alone | `reviewer_status_report_describes_reply_verifier_not_git_review` |
| R18 | UX | Deferred checklist + close/waive | Ready notice; reply close ≠ code close; admit only when composer+queue idle | `deferred_checklist_and_close_copy_split_reply_vs_code`, `deferred_checklist_policy_waits_for_idle_composer_and_queue` |
| R19 | UX | `/reviewer` notice + reply finish | Claim-vs-record sticky; open injection on finish | `reviewer_mode_notice_is_claim_vs_record_not_git_review`, `reply_capture_finish_names_open_injection`, `slash_reviewer_help_is_claim_vs_record_not_git_review` |
| R13 | Effect (fixture) | Handcrafted fail report | Parse + inject without live LLM | `review_fixture_claim_vs_record_fail_is_parseable_without_llm` |
| R13-mock | Effect (detect) | Mock false “tests passed” | Sticky evidence → rule verdict → parse → inject/finish notice | `mock_detect_false_tests_passed_claim_pipeline`, `mock_false_pass_capture_replaces_open_findings_and_names_finish` |
| R-live | Live | Real model false-pass | Ignored integration | `reviewer_claim_vs_record_detects_false_tests_passed_claim` |

**Efficiency invariant:** Reply verifier and git review never use the main turn
queue / AgentEvent pump. Sticky incomplete evidence is Gate fail-closed.

---

## 4. Default Moli headless `web_search`

| ID | Kind | Case | Expected | Automated evidence |
| --- | --- | --- | --- | --- |
| B1 | Contract. | Default backend wiring | `BrowserBackend::Moli`, `auto_download_moli: true` | `tui::tests::default_headless_web_search_backend_is_moli` + Core loader defaults |
| B2 | Effect. | Fail-closed without binary/download | Error, no hang | `test_moli_backend_fails_closed_without_download_or_executable` |
| B3 | Effic. | Runtime ensure concurrent | Single provision race-safe | Core `--features headless-search` `moli_runtime::tests::concurrent_first_use_downloads_once` |
| B4 | Effic. | Process timeout kills child | No zombie | `a3s-search` `process_timeout_kills_the_child_and_returns_typed_error` |
| B5 | Contract. | TUI policy allows web tools | Default allow (packaging; not live SERP) | `tui_default_policy_allows_readonly_research_tools` |
| B6 | Live | Real Moli SERP | Ignored | `test_web_search_headless` live cases |
| B7 | UX | Fail-closed tool chrome | Compact WebSearch failure; Headless→HTTP visible | `web_search_moli_fail_closed_chrome_stays_compact`, `surfaces_moli_headless_fail_closed_cascade_chrome` |
| B8 | Effect. | Cascade after empty headless | Continues to HTTP; failure retained | `a3s-search` `cascade_continues_after_empty_headless_then_http_can_stop` |

**Efficiency invariant:** Tool always registered; Moli binary provisioned on first headless tier only.

**Product profile:** Default Moli is a CLI/`scientific` (`headless-search`) contract,
not Core `default = local-code`. SDK embeds must opt in.

---

## 5. DynamicWorkflow / Flow step identity (DeepResearch multi-source)

| ID | Kind | Case | Expected | Automated evidence |
| --- | --- | --- | --- | --- |
| F1 | Effect. | Step input ≈96 KiB | Identity admits via digest-fold; no `65536` reject | Core `--features dynamic-workflow` `dynamic_flow_step_identity_folds_large_inputs_instead_of_rejecting_them` |
| F2 | Effect. | Eight-source catalog | Byte-bounded multi-source selectors complete | CLI `eight_source_catalog_uses_byte_bounded_multi_source_selectors` |
| F3 | Effect. | Independent source effects | No cross-source batch truncation | CLI `independent_source_effects_avoid_cross_source_batch_truncation` |

**Pin note:** CLI pins published Core at git rev
`c7e28eec514140f1610f1216ffe66bd81b29a523` (`=8.5.5`): digest-fold plus
`session_review` / sticky reply transcript authority. Proof:
`./scripts/verify-capability-regression.sh --require-published` and
`./scripts/prove-published-core-has-digest-fold.sh` (fails closed if a local
Code path patch remains).

---

## 6. Objective named surfaces (architecture-cleanup gate)

These rows exist so the cleanup objective cannot claim green without the named
capabilities. One-shot runner: `./scripts/verify-capability-regression.sh`.

| ID | Kind | Case | Expected | Automated evidence |
| --- | --- | --- | --- | --- |
| U1 | Effect. | RemoteUI view URL / embed | Parses nested/top-level view; webview args | `tui::remote_ui::tests::*` (`remote_ui` filter) |
| U2 | Effect. | Trusted local file/image view | Local HTTP / preview title+mime | `trusted_local_file_view_uses_local_http_server`, `local_image_view_uses_preview_title_mime_and_bounded_size` |
| S1 | Effect. | `skill_dir` discovery | Agent/Codex/Claude roots + flat md counts | `agent_skill_dirs_include_agents_codex_and_claude_roots`, `counts_skill_dirs_and_flat_md` |
| S2 | Effect. | Builtin `$okf` skill | Materializes and parses | `okf_skill_materializes_and_parses` |
| S3 | Effic. | Skill walk symlink cycles | Bounded depth + seen-set; cycle dirs do not hang load | Core `test_load_from_dir_tolerates_symlink_cycles` (Unix; local Code submodule — publish pin when releasing) |
| K1 | Contract. | `/kb` subcommands + chrome | Explicit parse; width-bounded panels | `panels::kb::tests::*`, `kbutil::tests::search_kb_finds_source_lines` |
| K2 | Effect. | Use OKF lease/query | Exact generation lease through search; fail-closed | `use_registry::knowledge::tests::*`, `atomic_flow_resolves_one_digest_bound_okf_surface_across_exact_scopes` |
| DW1 | UX | DynamicWorkflow terminal card | Progress without raw snapshot dump | `dynamic_workflow_*` render/event tests |
| DW2 | Effect. | evidence_first publication | Real ToolUse; honest settle / recovery | `evidence_first_tests::*`, `evidence_first_journal_tests::*` |
| R* | — | Reviewer | See §3 | §3 R1–R25 + dogfood 1–4 |

**Refuse:** treating path-patched Core as a published pin; counting soft-skip live as Effect.

---

## Runbook (focused)

```bash
# One-shot objective gate (RemoteUI, skills, /kb+$okf, DynamicWorkflow,
# Reviewer, evidence_first, Flow digest-fold pin, dogfood 1→7)
cd crates/cli
./scripts/verify-capability-regression.sh --require-published

# Published Core digest-fold pin proof
./scripts/prove-published-core-has-digest-fold.sh
./scripts/verify-core-identity-pin.sh --require-published
./scripts/prepare-digest-fold-release.sh

# CLI TUI / host (narrow filters)
cargo test --bin a3s workspace_review::
cargo test --bin a3s panels::review::
cargo test --bin a3s reviewer_mode_chip_is_footer_mode
cargo test --bin a3s reviewer_status_report_describes_reply
cargo test --bin a3s seeded_file_store_items_appear
cargo test --bin a3s web_search_moli_fail_closed
cargo test --bin a3s search_mode_bm25_explored
cargo test --bin a3s surfaces_moli_headless_fail
cargo test --bin a3s lazy_memory_store::
cargo test --bin a3s memory_panel_loads_from_shared_store
cargo test --bin a3s catalog_configure_does_not_open
cargo test --bin a3s default_headless_web_search_backend_is_moli
cargo test --bin a3s web_tools_registered_for_q

# Core (from crates/code/core)
cargo test --lib grep_does_not_open_durable_zvec
cargo test --lib catalog_attach_does_not_open_durable_zvec
cargo test --lib register_builtins_does_not_open_durable_zvec
cargo test --lib persistent_bm25_falls_back
cargo test --lib test_agent_memory_recall
cargo test -p a3s-code-core --features headless-search --test test_web_search_headless test_moli_backend_fails_closed
# Live durability (needs .a3s/config.acl)
cargo test -p a3s-code-core --test test_memory_store_real_llm -- --ignored --nocapture

# Search cascade (from crates/search)
cargo test --lib cascade_continues_after_empty_headless
```

Update this matrix when adding cases; do not claim coverage without a row in the tables above.
