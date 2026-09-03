#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "usage: check-release-state.sh <cli-version> <core-version> <tui-version> <search-version> [core-revision] [search-revision] [memory-version] [memory-revision]" >&2
}

if [ "$#" -lt 4 ]; then
  usage
  exit 2
fi

expected_version="$1"
expected_core="$2"
expected_tui="$3"
expected_search="$4"
expected_core_revision="${5:-}"
expected_search_revision="${6:-}"
expected_memory="${7:-}"
expected_memory_revision="${8:-}"

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

# Return the complete dependency declaration, including a multiline inline
# table when a manifest formats it over several lines.
manifest_dependency_block() {
  local dependency="$1"
  awk -v dependency="$dependency" '
    BEGIN { found = 0 }
    !found && $0 ~ "^[[:space:]]*" dependency "[[:space:]]*=" {
      found = 1
      print
      if (index($0, "}") > 0 || $0 !~ /\{/) exit
      next
    }
    found {
      print
      if (index($0, "}") > 0) exit
    }
  ' Cargo.toml
}

manifest_field() {
  local block="$1"
  local field="$2"
  awk -v field="$field" '
    {
      pattern = field "[[:space:]]*=[[:space:]]*\""
      if ($0 ~ pattern) {
        value = $0
        sub(".*" pattern, "", value)
        sub("\".*", "", value)
        print value
        exit
      }
    }
  ' <<<"$block"
}

lock_has_package() {
  local expected_name="$1"
  local expected_package_version="$2"
  local expected_source_kind="$3"
  local expected_source="${4:-}"

  awk \
    -v expected_name="$expected_name" \
    -v expected_version="$expected_package_version" \
    -v expected_kind="$expected_source_kind" \
    -v expected_source="$expected_source" '
    function inspect_package() {
      if (package_name != expected_name || package_version != expected_version) {
        return
      }
      if (expected_kind == "registry" && package_source ~ /^registry\+/) {
        found = 1
      } else if (expected_kind == "git" && package_source == expected_source) {
        found = 1
      } else if (expected_kind == "any" && package_source != "") {
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

require_exact_registry_dependency() {
  local dependency="$1"
  local expected_dependency_version="$2"
  local block
  block="$(manifest_dependency_block "$dependency")"
  if [ -z "$block" ] || ! grep -Eq \
    "^[[:space:]]*${dependency}[[:space:]]*=[[:space:]]*\"=${expected_dependency_version}\"[[:space:]]*$" \
    <<<"$block"; then
    echo "Cargo.toml must pin $dependency exactly to $expected_dependency_version" >&2
    exit 1
  fi
}

require_pinned_git_dependency() {
  local dependency="$1"
  local expected_dependency_version="$2"
  local expected_url="$3"
  local expected_revision="$4"
  local block
  local manifest_revision
  block="$(manifest_dependency_block "$dependency")"

  if [ -z "$block" ] || ! grep -Fq "version = \"=${expected_dependency_version}\"" <<<"$block"; then
    echo "Cargo.toml must pin $dependency exactly to $expected_dependency_version" >&2
    exit 1
  fi
  if ! grep -Fq "git = \"${expected_url}\"" <<<"$block"; then
    echo "Cargo.toml must source $dependency from $expected_url" >&2
    exit 1
  fi

  manifest_revision="$(manifest_field "$block" rev || true)"
  if ! [[ "$manifest_revision" =~ ^[0-9a-f]{40}$ ]]; then
    echo "Cargo.toml must pin $dependency to a 40-character commit revision" >&2
    exit 1
  fi
  if [ -n "$expected_revision" ] && [ "$manifest_revision" != "$expected_revision" ]; then
    echo "Cargo.toml $dependency revision $manifest_revision does not match expected $expected_revision" >&2
    exit 1
  fi

  local lock_source="git+${expected_url}?rev=${manifest_revision}#${manifest_revision}"
  lock_has_package "$dependency" "$expected_dependency_version" git "$lock_source" || {
    echo "Cargo.lock must resolve $dependency $expected_dependency_version from pinned git revision $manifest_revision" >&2
    exit 1
  }
}

if [ "$manifest_version" != "$expected_version" ]; then
  echo "Cargo.toml version $manifest_version does not match release $expected_version" >&2
  exit 1
fi
if [ "$lock_version" != "$expected_version" ]; then
  echo "Cargo.lock a3s version $lock_version does not match release $expected_version" >&2
  exit 1
fi

code_block="$(manifest_dependency_block a3s-code-core)"
if grep -Fq 'git =' <<<"$code_block"; then
  require_pinned_git_dependency \
    a3s-code-core "$expected_core" \
    "https://github.com/A3S-Lab/Code.git" "$expected_core_revision"
else
  if [ -n "$expected_core_revision" ]; then
    echo "Cargo.toml must pin a3s-code-core to git revision $expected_core_revision" >&2
    exit 1
  fi
  require_exact_registry_dependency a3s-code-core "$expected_core"
  lock_has_package a3s-code-core "$expected_core" registry || {
    echo "Cargo.lock must resolve a3s-code-core $expected_core from crates.io" >&2
    exit 1
  }
fi

require_exact_registry_dependency a3s-tui "$expected_tui"

if [ -n "$expected_search_revision" ]; then
  lock_source="git+https://github.com/A3S-Lab/Search.git?rev=${expected_search_revision}#${expected_search_revision}"
  lock_has_package a3s-search "$expected_search" git "$lock_source" || {
    echo "Cargo.lock must resolve a3s-search $expected_search from pinned git revision $expected_search_revision" >&2
    exit 1
  }
else
  lock_has_package a3s-search "$expected_search" registry || {
    echo "Cargo.lock must resolve a3s-search $expected_search from crates.io" >&2
    exit 1
  }
fi

if [ -n "$expected_memory" ]; then
  if [ -n "$expected_memory_revision" ]; then
    require_pinned_git_dependency \
      a3s-memory "$expected_memory" \
      "https://github.com/A3S-Lab/Memory.git" "$expected_memory_revision"
  else
    require_exact_registry_dependency a3s-memory "$expected_memory"
    lock_has_package a3s-memory "$expected_memory" registry || {
      echo "Cargo.lock must resolve a3s-memory $expected_memory from crates.io" >&2
      exit 1
    }
  fi
fi

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
