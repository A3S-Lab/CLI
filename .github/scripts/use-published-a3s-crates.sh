#!/usr/bin/env bash
# The CLI repository keeps local [patch.crates-io] paths for monorepo
# development, but standalone CI/release jobs must build against published
# crates.io versions. Sibling repository main branches can move independently
# and are not a stable release input.

set -euo pipefail

manifest="${1:-Cargo.toml}"

perl -0pi -e 's/\n\[patch\.crates-io\][\s\S]*\z/\n/' "$manifest"
cargo generate-lockfile --manifest-path "$manifest"
