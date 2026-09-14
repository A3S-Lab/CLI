#!/usr/bin/env bash
# Validate digest-fold release readiness for the CLI pin.
# Does NOT commit or push.
#
# Usage (from crates/cli): ./scripts/prepare-digest-fold-release.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
# shellcheck source=lib/core-pin.sh
source "$ROOT/scripts/lib/core-pin.sh"

fail() { echo "error: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }

if core_pin_is_local; then
  echo "=== local Core path pin ($(core_pin_local_reason); pre-publish) ==="
  echo "=== 1. Code digest-fold staged surface ==="
  (
    cd "$ROOT/../code"
    [[ -f scripts/stage-digest-fold.sh ]] || fail "missing crates/code/scripts/stage-digest-fold.sh"
    ./scripts/stage-digest-fold.sh >/tmp/a3s-stage-digest-fold.log 2>&1 \
      || fail "stage-digest-fold.sh failed (see /tmp/a3s-stage-digest-fold.log)"
  )
  ok "digest-fold surface staged and units passed"

  echo "=== 2. Local capability gate ==="
  ./scripts/verify-capability-regression.sh >/tmp/a3s-capability-regression.log 2>&1 \
    || fail "verify-capability-regression.sh failed"
  ok "local capability regression green"

  echo "=== Ready: publish Code, then replace the path pin with that git rev ==="
  echo "    Do not point --require-published at this path pin."
  exit 0
fi

echo "=== published pin mode (no Code path pin) ==="
./scripts/prove-published-core-has-digest-fold.sh
./scripts/verify-capability-regression.sh --require-published
ok "published digest-fold pin and capability gate green"
