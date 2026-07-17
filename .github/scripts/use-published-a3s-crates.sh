#!/usr/bin/env bash
# The CLI repository keeps local [patch.crates-io] paths for monorepo
# development, but standalone CI/release jobs must build against published
# crates.io versions. Sibling repository main branches can move independently
# and are not a stable release input.
#
# An optional second argument reuses a previously published Cargo.lock so a
# workflow rerun cannot resolve a different graph for the same release tag.

set -euo pipefail

manifest="${1:-Cargo.toml}"
published_lock="${2:-}"

perl -0pi -e 's/\n\[patch\.crates-io\][\s\S]*\z/\n/' "$manifest"
if [ -n "$published_lock" ]; then
  cp "$published_lock" "$(dirname "$manifest")/Cargo.lock"
else
  cargo generate-lockfile --manifest-path "$manifest"
fi
