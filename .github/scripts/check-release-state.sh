#!/usr/bin/env bash

set -euo pipefail

expected_version="${1:?usage: check-release-state.sh <cli-version> <core-version> <tui-version> <search-version>}"
expected_core="${2:?usage: check-release-state.sh <cli-version> <core-version> <tui-version> <search-version>}"
expected_tui="${3:?usage: check-release-state.sh <cli-version> <core-version> <tui-version> <search-version>}"
expected_search="${4:?usage: check-release-state.sh <cli-version> <core-version> <tui-version> <search-version>}"

if ! [[ "$expected_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "invalid CLI release version: $expected_version" >&2
  exit 1
fi

manifest_version="$(
  awk '
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && /^version = "/ {
      value = $0
      sub(/^version = "/, "", value)
      sub(/".*$/, "", value)
      print value
      exit
    }
  ' Cargo.toml
)"

lock_version="$(
  awk '
    /^\[\[package\]\]$/ { package_name = ""; next }
    /^name = "a3s"$/ { package_name = "a3s"; next }
    package_name == "a3s" && /^version = "/ {
      value = $0
      sub(/^version = "/, "", value)
      sub(/".*$/, "", value)
      print value
      exit
    }
  ' Cargo.lock
)"

lock_has_registry_package() {
  local expected_name="$1"
  local expected_package_version="$2"
  awk -v expected_name="$expected_name" -v expected_version="$expected_package_version" '
    function inspect_package() {
      if (package_name == expected_name && package_version == expected_version &&
          package_source ~ /^registry\+/) {
        found = 1
      }
    }
    /^\[\[package\]\]$/ {
      inspect_package()
      package_name = ""
      package_version = ""
      package_source = ""
      next
    }
    /^name = "/ {
      package_name = $0
      sub(/^name = "/, "", package_name)
      sub(/".*$/, "", package_name)
      next
    }
    /^version = "/ {
      package_version = $0
      sub(/^version = "/, "", package_version)
      sub(/".*$/, "", package_version)
      next
    }
    /^source = "/ {
      package_source = $0
      sub(/^source = "/, "", package_source)
      sub(/".*$/, "", package_source)
      next
    }
    END {
      inspect_package()
      exit !found
    }
  ' Cargo.lock
}

if [ "$manifest_version" != "$expected_version" ]; then
  echo "Cargo.toml version $manifest_version does not match release $expected_version" >&2
  exit 1
fi
if [ "$lock_version" != "$expected_version" ]; then
  echo "Cargo.lock a3s version $lock_version does not match release $expected_version" >&2
  exit 1
fi

grep -Fqx "a3s-code-core = \"=$expected_core\"" Cargo.toml || {
  echo "Cargo.toml must pin a3s-code-core exactly to $expected_core" >&2
  exit 1
}
lock_has_registry_package "a3s-code-core" "$expected_core" || {
  echo "Cargo.lock must resolve a3s-code-core $expected_core from crates.io" >&2
  exit 1
}
grep -Fqx "a3s-tui = \"=$expected_tui\"" Cargo.toml || {
  echo "Cargo.toml must pin a3s-tui exactly to $expected_tui" >&2
  exit 1
}
lock_has_registry_package "a3s-search" "$expected_search" || {
  echo "Cargo.lock must resolve transitive a3s-search $expected_search from crates.io" >&2
  exit 1
}

if ! awk -v header="## [$expected_version]" '
  index($0, header) == 1 { found = 1; next }
  found && /^## \[/ { exit }
  found && /^- / { entry = 1 }
  END { exit !(found && entry) }
' CHANGELOG.md; then
  echo "CHANGELOG.md has no release entry for $expected_version" >&2
  exit 1
fi

echo "release state is consistent at CLI $expected_version, Core $expected_core, TUI $expected_tui, Search $expected_search"
