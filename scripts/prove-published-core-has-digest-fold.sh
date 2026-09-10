#!/usr/bin/env bash
# Positive pin proof: Cargo.toml git rev of a3s-code-core admits DeepResearch
# multi-source step identity (~96 KiB) via digest-fold. Fails closed if a local
# Code path patch is still present (that would not prove the published pin).
#
# Usage (from crates/cli): ./scripts/prove-published-core-has-digest-fold.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ -f .cargo/config.toml ]] && rg -q 'path = "../code/core"' .cargo/config.toml; then
  echo "error: Code path patch still present; refuse to claim a published-pin proof" >&2
  exit 1
fi

if ! rg -q 'a3s-code-core = \{.*rev = "a93e80d2d810fe47ee7337ac2854bda0c44f6e57"' Cargo.toml \
  && ! rg -q 'rev = "[0-9a-f]{40}"' Cargo.toml; then
  echo "error: Cargo.toml missing a3s-code-core git rev pin" >&2
  exit 1
fi

echo "=== eight_source against published Core pin (no path patch) ==="
cargo test --bin a3s -- \
  eight_source_catalog_uses_byte_bounded_multi_source_selectors \
  independent_source_effects_avoid_cross_source_batch_truncation \
  --quiet

echo "ok: published Core digest-fold pin admits multi-source identity"
