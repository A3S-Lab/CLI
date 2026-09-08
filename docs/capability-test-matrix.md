# Capability test matrix (first principles)

Scope: a3s-code TUI / Core **8.5.1** paths for **zvec-grep (BM25)**, **ReMe-like memory**,
**Reviewer**, and **default Moli web search**.

**Release plan:** [`v8.4.0-capability-integration-tests.md`](./v8.4.0-capability-integration-tests.md)
(backlog, UX checklist, honest Effect-prompt vs Effect-detect split).

## First-principles gates

For each capability, every case must answer:

1. **Mission** — does this strengthen the product’s core loop?
2. **Effectiveness** — seeded truth is detected; false success is rejected.
3. **Efficiency** — heavy work is demand-driven, bounded, and non-blocking.
4. **UX** — chrome/copy matches the product contract (sticky ≠ git review, etc.).
5. **Evidence** — a hermetic automated test (or named ignored live test) proves it.

| Capability | Mission |
| --- | --- |
| BM25 / zvec | Local lexical retrieval without paying index cost until needed |
| Memory | Durable recall/extract without first-frame disk tax |
| Reviewer | Independent claim↔record check without blocking the main agent |
| Moli `web_search` | Default headless SERP with fail-closed / cascade fallback |

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
| M5 | Effect. | `/ctx memory` panel | Browseable items | `panels::context::memory` / `ctx` tests |
| M6 | UX | `/memory` hub tip + loading | `prefer /ctx memory`; note `loading…` | `memory_hub_tip_and_loading_note` |
| M7 | Effect. | File store → timeline | Seeded `index.json` appears in panel timeline | `seeded_file_store_items_appear_in_memory_timeline` |
| M8 | Effic. | Host write paths share one store Arc | `/ctx save`, `/sleep`, forget, evolution sync use `App.memory_store` (lazy) — no second `FileMemoryStore::new` on the session dir | `synchronize_memory_store` takes `Arc<dyn MemoryStore>`; panel/ctx/sleep callers |
| M9 | Effic. | `/memory` browse prefers shared store | Panel load uses `MemoryStore::get_recent` on `App.memory_store` first; filesystem snapshot is last resort | `memory_panel_loads_from_shared_store_arc`; `promoted_memory_roundtrips_through_the_real_store` |
| M10 | Live | Store → reopen → recall under ACL | Ignored integration with `.a3s/config.acl` | Core `tests/test_memory_store_real_llm.rs` (`real_config_memory_store_survives_*`) |
| M11 | Live | LLM extract → reopen → recall | Ignored; soft-skips on judge decline / provider block | Core `real_model_memory_extract_survives_reopen_or_soft_skips` |

**Efficiency invariant:** Session always *wires* memory; I/O waits until first op.
Host UI mutations **and** `/memory` browse must reuse the same `Arc` as the agent (lazy file backend).

---

## 3. Reviewer (claim-vs-record + git `/review`)

| ID | Kind | Case | Expected | Automated evidence |
| --- | --- | --- | --- | --- |
| R1 | Effect. | Failed tool vs “tests passed” claim | Prompt/evidence includes failure | `sticky_prompt_includes_failed_tool_evidence_for_claim_vs_record` |
| R2 | Effect. | Truncated / incomplete evidence | `evidence_complete: false` + inconclusive guidance | `sticky_prompt_marks_incomplete_when_truncated`, `latest_turn_evidence_marks_incomplete_for_non_terminal_tool`, `latest_turn_evidence_caps_tools_and_marks_incomplete` |
| R3 | Effect. | Sticky ≠ git scope | No working-tree inspection copy | `sticky_reply_review_targets_assistant_message_not_diff`, `sticky_prompt_forbids_rerunning_tools_and_method_judgement` |
| R4 | Effect. | Git `/review` stays code | `kind: code`, read-only | `prompt_is_read_only_and_carries_the_report_contract`, `workspace_review_prompt_kind_stays_code_not_reply` |
| R5 | Effect. | Report schema reply fields | verdict / evidence_refs / status | `parse_review_report_reads_reply_kind_and_verdict_fields`, `review_reply_contract_includes_verdict_evidence_refs_and_status` |
| R6 | Effect. | Open findings injection is DATA | Not instructions | `open_reply_findings_injection_is_data_not_instructions`, `open_reply_findings_injection_flattens_newlines_in_issue_text`, `with_open_reply_findings_prefix_injects_on_user_turns_only` |
| R7 | Effic. | Manual priority > sticky | Manual claimed first | `reviewer_priority_queue_admits_manual_before_sticky` |
| R8 | Effic. | Sticky coalesce | At most one queued sticky | `sticky_reviews_coalesce_in_the_lane` |
| R9 | Effic. | Serial inflight | Queue while busy | `reviewer_lane_queues_while_inflight` |
| R10 | Effic. | Main stream mode | Reviewer → Default main stream | `reviewer_main_stream_stays_default`, `reviewer_mode_does_not_hijack_main_session_style` |
| R11 | Effect. | Address vs code-fix prompts | Reply address fence | `review_address_reply_prompt_is_data_not_instructions` |
| R12 | Bound | Tool args/output / tool count | Truncation + cap → incomplete | `summarize_tool_evidence_truncates_args_and_output`, `latest_turn_evidence_caps_tools_and_marks_incomplete` |
| R14 | Effect. | Code capture vs sticky opens | Git report preserves sticky opens; reply replaces | `code_capture_preserves_sticky_open_findings` |
| R15 | Effic. | Sticky arming gate | Skips non-Reviewer / deep-research / sleep / goal / no evidence | `sticky_arming_requires_reviewer_mode_and_evidence`, `sticky_arming_skips_deep_research_sleep_and_goal` |
| U1 | UX | Footer mode chip + ring | `reviewer` is mode slot; Shift+Tab ring includes reviewer | `reviewer_mode_chip_is_footer_mode_not_live_chip`, `reviewer_mode_does_not_hijack_main_session_style`, banner/help tips |
| R16 | UX | Lane / bus chrome | started · async bus; empty/fail lines | `reviewer_bus_chrome_*`, `reviewer_lane_and_finish_chrome_*` |
| R17 | UX | Permissions / status copy | Claim↔record wording; not “async code review” alone | `reviewer_status_report_describes_reply_verifier_not_git_review` |
| R18 | UX | Deferred checklist + close/waive | Ready notice; reply close ≠ code close; admit only when composer+queue idle | `deferred_checklist_and_close_copy_split_reply_vs_code`, `deferred_checklist_policy_waits_for_idle_composer_and_queue` |
| R19 | UX | `/reviewer` notice + reply finish | Claim-vs-record sticky; open injection on finish | `reviewer_mode_notice_is_claim_vs_record_not_git_review`, `reply_capture_finish_names_open_injection`, `slash_reviewer_help_is_claim_vs_record_not_git_review` |
| R13 | Effect (fixture) | Handcrafted fail report | Parse + inject without live LLM | `review_fixture_claim_vs_record_fail_is_parseable_without_llm` |
| R13-mock | Effect (detect) | Mock false “tests passed” | Sticky prompt → rule verdict → parse → inject/finish notice | `mock_detect_false_tests_passed_claim_pipeline`, `mock_false_pass_capture_replaces_open_findings_and_names_finish` |
| R-live | Live | Real model false-pass | Ignored integration | `reviewer_claim_vs_record_detects_false_tests_passed_claim` |

**Efficiency invariant:** ReviewerLane only; never main turn queue / AgentEvent pump.

---

## 4. Default Moli headless `web_search`

| ID | Kind | Case | Expected | Automated evidence |
| --- | --- | --- | --- | --- |
| B1 | Effect. | Default backend | `BrowserBackend::Moli`, `auto_download_moli: true` | `tui::tests::default_headless_web_search_backend_is_moli` + Core loader defaults |
| B2 | Effect. | Fail-closed without binary/download | Error, no hang | `test_moli_backend_fails_closed_without_download_or_executable` |
| B3 | Effic. | Runtime ensure concurrent | Single provision race-safe | `moli_runtime` concurrent first-use tests |
| B4 | Effic. | Process timeout kills child | No zombie | `a3s-search` `moli_tests` timeout kill |
| B5 | Effect. | TUI policy allows web tools | Default allow | `tui_default_policy_allows_readonly_research_tools` |
| B6 | Live | Real Moli SERP | Ignored | `test_web_search_headless` live cases |
| B7 | UX | Fail-closed tool chrome | Compact WebSearch failure; Headless→HTTP visible | `web_search_moli_fail_closed_chrome_stays_compact`, `surfaces_moli_headless_fail_closed_cascade_chrome` |
| B8 | Effect. | Cascade after empty headless | Continues to HTTP; failure retained | `a3s-search` `cascade_continues_after_empty_headless_then_http_can_stop` |

**Efficiency invariant:** Tool always registered; Moli binary provisioned on first headless tier only.

**Product profile:** Default Moli is a CLI/`scientific` (`headless-search`) contract,
not Core `default = local-code`. SDK embeds must opt in.

---

## Runbook (focused)

```bash
# CLI TUI / host
cd crates/cli
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
