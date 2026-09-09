#!/usr/bin/env bash
# One-shot capability regression gate for the architecture-cleanup objective.
# Proves named surfaces are preserved (not weakened).
#
# Usage (from crates/cli):
#   ./scripts/verify-capability-regression.sh
#   ./scripts/verify-capability-regression.sh --require-published
#
# --require-published fails if .cargo still path-patches ../code/core. Use it
# after bump-core-digest-fold-pin.sh so green means published Core, not local.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

REQUIRE_PUBLISHED=0
for arg in "$@"; do
  case "$arg" in
    --require-published) REQUIRE_PUBLISHED=1 ;;
    *)
      echo "usage: $0 [--require-published]" >&2
      exit 1
      ;;
  esac
done

run_bin() {
  local title="$1"
  shift
  echo
  echo "=== ${title} ==="
  cargo test --bin a3s -- "$@" --quiet
}

echo "=== Flow step identity pin (digest-fold) ==="
if [[ "$REQUIRE_PUBLISHED" -eq 1 ]]; then
  ./scripts/verify-core-identity-pin.sh --require-published
else
  ./scripts/verify-core-identity-pin.sh
fi

run_bin "RemoteUI" remote_ui
run_bin "skills / skill_dir" skill_dir

echo
echo "=== skills symlink-cycle Effic (Core S3) ==="
(
  cd "$ROOT/../code"
  cargo test -p a3s-code-core --lib test_load_from_dir_tolerates_symlink_cycles --quiet
)

run_bin "/kb" kb_
run_bin "\$okf + OKF Use" okf
run_bin "DynamicWorkflow chrome/runtime" dynamic_workflow
run_bin "Reviewer" reviewer
run_bin "evidence_first DeepResearch" evidence_first
run_bin "Use Flow atomic projection" atomic_flow
run_bin "Use Flow projected runtime" projected_flow

echo
echo "=== Dogfood 1→7 ==="
./scripts/dogfood-rc.sh

echo
if [[ "$REQUIRE_PUBLISHED" -eq 1 ]]; then
  echo "ok: capability regression gate green against published Core pin"
else
  echo "ok: capability regression gate green"
  if rg -q 'path = "../code/core"' .cargo/config.toml 2>/dev/null; then
    echo "    (local Core path patch still active — not objective-complete)"
    echo "    After Code publish: ./scripts/bump-core-digest-fold-pin.sh <rev>"
    echo "    Then: ./scripts/verify-capability-regression.sh --require-published"
  fi
fi
