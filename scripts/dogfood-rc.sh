#!/usr/bin/env bash
# Run v8.4.0 RC dogfood hermetics in checklist order (1→7).
# Usage (from crates/cli): ./scripts/dogfood-rc.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

run() {
  local step="$1"
  local title="$2"
  shift 2
  echo
  echo "=== Dogfood #${step}: ${title} ==="
  cargo test --bin a3s -- "$@" --quiet
}

run 1 "Footer mode ring + reviewer chip" \
  reviewer_mode_chip_is_footer_mode_not_live_chip \
  reviewer_mode_does_not_hijack_main_session_style \
  welcome_banner_uses_shared \
  help_body_starts_with_orientation

run 2 "/reviewer notice + /help claim-vs-record" \
  reviewer_mode_notice_is_claim_vs_record \
  slash_reviewer_help_is_claim_vs_record \
  reply_capture_finish_names_open_injection

run 3 "Sticky lane + main stream free" \
  sticky_reviews_coalesce \
  reviewer_lane_queues \
  reviewer_bus_chrome \
  reviewer_main_stream_stays_default \
  reviewer_priority_queue \
  sticky_arming \
  sticky_prompt_includes_failed \
  mock_detect_false

run 4 "/review code kind + sticky opens" \
  code_capture_preserves_sticky \
  workspace_review_prompt_kind_stays_code \
  prompt_is_read_only

run 5 "/ctx memory + /memory prefer tip + shared-store browse" \
  memory_hub_tip_and_loading \
  seeded_file_store_items \
  store_survives_reopen \
  memory_panel_loads_from_shared_store \
  promoted_memory_roundtrips

run 6 "Grep ≠ durable zvec + BM25/footer chrome" \
  grep_never_surfaces \
  grep_explored_never \
  retrieval_footer_chips \
  bm25_surfaces \
  bm25_freshness \
  search_mode_bm25_explored

(
  echo
  echo "=== Dogfood #6 (Core): grep_does_not_open_durable_zvec ==="
  cd "$ROOT/../code/core"
  cargo test --lib grep_does_not_open_durable_zvec --quiet
)

run 7 "Moli fail-closed + cascade chrome" \
  default_headless_web_search_backend_is_moli \
  web_search_moli_fail_closed \
  surfaces_moli_headless_fail_closed

(
  echo
  echo "=== Dogfood #7 (Search): cascade_continues_after_empty_headless ==="
  cd "$ROOT/../search"
  cargo test --lib cascade_continues_after_empty_headless --quiet
)

echo
echo "RC dogfood hermetics 1→7: all green."
echo "Optional live (needs .a3s/config.acl):"
echo "  cargo test -p a3s-code-core --test test_memory_store_real_llm -- --ignored --nocapture"
echo "Remaining human step: walk the seven rows in a real a3s code PTY session."
