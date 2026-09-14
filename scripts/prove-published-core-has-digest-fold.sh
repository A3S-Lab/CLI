#!/usr/bin/env bash
# Positive pin proof: Cargo.toml git rev of a3s-code-core admits DeepResearch
# multi-source step identity (~96 KiB) via digest-fold. Fails closed if a local
# path pin is still present (.cargo/config.toml patch or Cargo.toml
# path = "../code/core"). That compiles this tree, not a published pin.
#
# Usage (from crates/cli): ./scripts/prove-published-core-has-digest-fold.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=lib/core-pin.sh
source "$ROOT/scripts/lib/core-pin.sh"

if core_pin_is_local; then
  echo "error: $(core_pin_local_reason); refuse to claim a published-pin proof" >&2
  exit 1
fi

if ! rg -q 'a3s-code-core = \{.*rev = "bcd4efe2fceb50cae9a6a5d40d29e3142143018d"' Cargo.toml \
  && ! rg -q 'a3s-code-core = \{[^}]*rev = "[0-9a-f]{40}"' Cargo.toml; then
  echo "error: Cargo.toml missing a3s-code-core git rev pin" >&2
  exit 1
fi

echo "=== eight_source against published Core pin (no path patch) ==="
cargo test --bin a3s -- \
  eight_source_catalog_uses_byte_bounded_multi_source_selectors \
  independent_source_effects_avoid_cross_source_batch_truncation \
  --quiet

echo "ok: published Core digest-fold pin admits multi-source identity"
