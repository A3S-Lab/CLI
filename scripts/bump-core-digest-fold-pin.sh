#!/usr/bin/env bash
# After Code publishes digest-fold, bump CLI's a3s-code-core rev and drop
# the local path patch. Does not commit.
#
# Proof is against the published git rev only (CLI hermetics with path patch
# absent). Local ../code unit tests are intentionally not used here — they
# would green even when Cargo still pointed at a pre-fold Core.
#
# Usage (from crates/cli):
#   ./scripts/bump-core-digest-fold-pin.sh <40-char-git-rev>
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

REV="${1:-}"
if [[ ! "$REV" =~ ^[0-9a-f]{40}$ ]]; then
  echo "usage: $0 <40-char-git-rev>" >&2
  exit 1
fi

if [[ ! -f .cargo/config.toml ]]; then
  echo "error: missing .cargo/config.toml" >&2
  exit 1
fi

python3 - "$REV" <<'PY'
import re
import sys
from pathlib import Path

rev = sys.argv[1]
cargo = Path("Cargo.toml")
text = cargo.read_text()
pattern = re.compile(
    r'(a3s-code-core = \{ version = "=8\.5\.[0-9]+", git = "https://github\.com/A3S-Lab/Code\.git", rev = ")([0-9a-f]{40})(")'
)
new_text, count = pattern.subn(rf"\g<1>{rev}\g<3>", text, count=1)
if count != 1:
    raise SystemExit("error: could not locate a3s-code-core rev pin in Cargo.toml")
cargo.write_text(new_text)
print(f"Cargo.toml a3s-code-core rev -> {rev}")

config = Path(".cargo/config.toml")
cfg = config.read_text()
marker = '[patch."https://github.com/A3S-Lab/Code.git"]'
if marker not in cfg:
    print("note: Code path patch already absent")
else:
    head, _, _ = cfg.partition(marker)
    # Drop trailing blank lines from the crates-io section preface comments.
    kept = head.rstrip() + "\n"
    # Also drop the digest-fold comment block immediately above the Code patch.
    lines = kept.splitlines(keepends=True)
    while lines and lines[-1].strip() == "":
        lines.pop()
    # Remove contiguous comment lines that document the temporary Code patch.
    while lines and lines[-1].lstrip().startswith("#"):
        lines.pop()
    while lines and lines[-1].strip() == "":
        lines.pop()
    config.write_text("".join(lines) + "\n")
    print("removed Code path patch from .cargo/config.toml")
PY

if rg -q 'path = "../code/core"' .cargo/config.toml 2>/dev/null; then
  echo "error: Code path patch still present after bump; refusing to verify" >&2
  exit 1
fi

echo "=== cargo update a3s-code-core ==="
# After dropping the path patch, refresh the lock against the git rev. A bare
# `cargo update -p` can fail if the previous lock only resolved via path patch;
# generate/update the graph first, then pin-check.
cargo generate-lockfile >/dev/null 2>&1 || true
if ! cargo update -p a3s-code-core; then
  # Fallback: force a resolution pass that pulls the git rev into the lock.
  cargo fetch
  cargo update -p a3s-code-core
fi

if ! rg -q "source = \"git\\+https://github.com/A3S-Lab/Code.git\\?rev=${REV}" Cargo.lock; then
  echo "error: Cargo.lock does not pin a3s-code-core at rev ${REV}" >&2
  exit 1
fi

echo "=== verify published pin (no path patch; CLI hermetics only) ==="
REQUIRE_PUBLISHED=1 ./scripts/verify-core-identity-pin.sh

echo "ok: CLI pin bumped to $REV and multi-source hermetics passed against published Core"
echo "Next: commit Cargo.toml + Cargo.lock + .cargo/config.toml when authorized."
echo "Then: ./scripts/verify-capability-regression.sh --require-published"
