#!/usr/bin/env bash
# Prove Core digest-fold for DeepResearch multi-source identity.
# Usage (from crates/cli): ./scripts/verify-core-identity-pin.sh
#
# Modes:
# - Path patch present (.cargo/config.toml → ../code/core): local pin until publish.
#   Runs local Core unit tests + CLI multi-source hermetics.
# - Path patch absent: published Cargo.toml rev pin.
#   Runs CLI multi-source hermetics only (those compile against the locked
#   git rev). Local ../code units are NOT evidence for a published pin.
#
# Set REQUIRE_PUBLISHED=1 (or pass --require-published) to fail closed if the
# temporary Code path patch is still present.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

for arg in "$@"; do
  case "$arg" in
    --require-published) REQUIRE_PUBLISHED=1 ;;
    *)
      echo "usage: $0 [--require-published]" >&2
      exit 1
      ;;
  esac
done

HAS_PATH_PATCH=0
if rg -q 'path = "../code/core"' .cargo/config.toml 2>/dev/null; then
  HAS_PATH_PATCH=1
fi

if [[ "${REQUIRE_PUBLISHED:-0}" == "1" && "$HAS_PATH_PATCH" -eq 1 ]]; then
  echo "error: --require-published / REQUIRE_PUBLISHED=1 but Code path patch is present" >&2
  echo "       run ./scripts/bump-core-digest-fold-pin.sh <rev> after Code publish" >&2
  exit 1
fi

if [[ "$HAS_PATH_PATCH" -eq 1 ]]; then
  echo "mode: local Core path patch"
  echo "=== Core digest-fold unit (local tree) ==="
  (
    cd "$ROOT/../code"
    cargo test -p a3s-code-core --features dynamic-workflow --lib \
      dynamic_flow_step_identity_ --quiet
  )
else
  echo "mode: published Core rev pin (no path patch)"
  echo "      CLI hermetics below are the pin proof; local ../code units are not."
fi

echo "=== CLI multi-source identity hermetics ==="
cargo test --bin a3s -- \
  eight_source_catalog_uses_byte_bounded_multi_source_selectors \
  independent_source_effects_avoid_cross_source_batch_truncation \
  --quiet

echo "ok: Core digest-fold pin is effective"
