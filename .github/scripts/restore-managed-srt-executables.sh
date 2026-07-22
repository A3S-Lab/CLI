#!/usr/bin/env bash
set -euo pipefail

support_root="${1:-support/managed-srt}"
# Git Bash cannot reliably represent POSIX executable bits on Windows. The
# Windows bundle uses a ZIP and does not execute these Unix sandbox helpers.
verify_executable_mode=true
case "$(uname -s)" in
  CYGWIN* | MINGW* | MSYS*) verify_executable_mode=false ;;
esac

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
  if [[ "$verify_executable_mode" == true && ! -x "$executable_path" ]]; then
    echo "Managed sandbox file is not executable: ${executable_path}" >&2
    exit 1
  fi
done
