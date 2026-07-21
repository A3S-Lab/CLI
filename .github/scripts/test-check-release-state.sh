#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
checker="$script_dir/check-release-state.sh"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT

write_valid_fixture() {
  cat >"$fixture/Cargo.toml" <<'EOF'
[package]
name = "a3s"
version = "0.9.6"

[dependencies]
a3s-code-core = "=5.3.5"
a3s-tui = "=0.1.12"
EOF
  cat >"$fixture/Cargo.lock" <<'EOF'
version = 4

[[package]]
name = "a3s"
version = "0.9.6"
EOF
  cat >"$fixture/CHANGELOG.md" <<'EOF'
# Changelog

## [0.9.6] - 2026-07-17

### Added

- Added a release fixture.

## [0.9.5] - 2026-07-16
EOF
}

run_checker() {
  (cd "$fixture" && bash "$checker" "$@")
}

expect_failure() {
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "expected failure: $description" >&2
    exit 1
  fi
}

write_valid_fixture
run_checker 0.9.6 5.3.5 0.1.12 >/dev/null

expect_failure "prerelease tags are unsupported" \
  run_checker 0.9.6-rc.1 5.3.5 0.1.12

write_valid_fixture
sed -i.bak 's/version = "0.9.6"/version = "0.9.5"/' "$fixture/Cargo.lock"
expect_failure "the committed lock version must match" \
  run_checker 0.9.6 5.3.5 0.1.12

write_valid_fixture
sed -i.bak '/^- Added a release fixture\.$/d' "$fixture/CHANGELOG.md"
expect_failure "a heading alone is not a changelog entry" \
  run_checker 0.9.6 5.3.5 0.1.12

write_valid_fixture
sed -i.bak 's/a3s-code-core = "=5.3.5"/a3s-code-core = "5.3.5"/' "$fixture/Cargo.toml"
expect_failure "Core must be pinned exactly" \
  run_checker 0.9.6 5.3.5 0.1.12

echo "release-state checks passed"
