#!/usr/bin/env bash
set -euo pipefail

support_root="${1:-support/managed-srt}"
executables=(
  "node_modules/@anthropic-ai/sandbox-runtime/dist/cli.js"
  "node_modules/@anthropic-ai/sandbox-runtime/vendor/seccomp/arm64/apply-seccomp"
  "node_modules/@anthropic-ai/sandbox-runtime/vendor/seccomp/x64/apply-seccomp"
)

for relative_path in "${executables[@]}"; do
  executable_path="${support_root}/${relative_path}"
  if [[ ! -f "$executable_path" ]]; then
    echo "Missing managed sandbox executable: ${executable_path}" >&2
    exit 1
  fi

  chmod 0755 "$executable_path"
  if [[ ! -x "$executable_path" ]]; then
    echo "Managed sandbox file is not executable: ${executable_path}" >&2
    exit 1
  fi
done
