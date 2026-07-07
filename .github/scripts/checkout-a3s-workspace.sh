#!/usr/bin/env bash
# Prepare the sibling A3S crates expected by this repository's local
# [patch.crates-io] paths. GitHub Actions checks out Cli as a standalone repo,
# so the parent directory is otherwise missing ../code, ../memory, and friends.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
workspace_parent="$(dirname "$repo_root")"
ref="${A3S_WORKSPACE_REF:-main}"
token="${GH_TOKEN:-${GITHUB_TOKEN:-}}"

repo_url() {
  local repo="$1"
  if [ -n "$token" ]; then
    printf 'https://x-access-token:%s@github.com/A3S-Lab/%s.git' "$token" "$repo"
  else
    printf 'https://github.com/A3S-Lab/%s.git' "$repo"
  fi
}

clone_repo() {
  local repo="$1"
  local dir="$2"
  local target="${workspace_parent}/${dir}"

  if [ -d "${target}/.git" ]; then
    if [ "${GITHUB_ACTIONS:-}" != "true" ] && [ -n "$(git -C "$target" status --porcelain)" ]; then
      printf 'Keeping existing dirty checkout at %s\n' "$target"
      return
    fi
    git -C "$target" fetch --depth 1 origin "$ref" || git -C "$target" fetch origin "$ref"
    git -C "$target" checkout --force FETCH_HEAD
    return
  fi

  rm -rf "$target"
  git clone --depth 1 --branch "$ref" "$(repo_url "$repo")" "$target"
}

clone_common() {
  local target="${workspace_parent}/common"
  local checkout="${workspace_parent}/a3s-common-checkout"

  if [ -f "${target}/Cargo.toml" ]; then
    return
  fi

  rm -rf "$target" "$checkout"
  git clone --depth 1 --filter=blob:none --sparse --branch "$ref" "$(repo_url a3s)" "$checkout"
  git -C "$checkout" sparse-checkout set crates/common
  cp -R "${checkout}/crates/common" "$target"
  rm -rf "$checkout"
}

clone_repo Code code
clone_common
clone_repo Flow flow
clone_repo Lane lane
clone_repo Memory memory
clone_repo Search search
clone_repo TUI tui

printf 'Prepared A3S workspace siblings under %s\n' "$workspace_parent"
