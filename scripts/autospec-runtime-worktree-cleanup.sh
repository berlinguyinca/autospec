#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || -z "${1:-}" ]]; then
    printf 'Usage: autospec-runtime-worktree-cleanup.sh PATH\n' >&2
    exit 2
fi

runtime="${AUTOSPEC_BIN:-autospec}"
exec "$runtime" runtime env gc --repo "$1"
