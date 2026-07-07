#!/usr/bin/env bash
# autospec-autonomous-run-drain.sh — one Tier-1 drain invocation for the conductor.
set -eu

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="${AUTOSPEC_REPO_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"

if ! command -v omx >/dev/null 2>&1; then
    printf 'autospec-autonomous-run-drain: omx not found on PATH\n' >&2
    exit 127
fi

exec omx exec \
    --cd "$REPO_DIR" \
    --dangerously-bypass-approvals-and-sandbox \
    '$autospec-run'
