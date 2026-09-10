#!/usr/bin/env bash
# record-verify-evidence.sh — record full-suite evidence at the current HEAD
# (issue #3523).
#
# Phase 4 runs the full test suite in the PR worktree; this records what ran,
# the exit status, and the exact commit it ran against, into
# .autospec/verify-evidence/<PR>.json. The merge gate
# (autospec-guarded-merge.sh --verify-evidence) reads that file at merge time
# and refuses to merge when the recorded commit is no longer the PR head.
#
# Usage:
#   record-verify-evidence.sh --pr N --command <cmd> --status <exit>
#
# Writes .autospec/verify-evidence/<PR>.json (relative to the current
# directory, i.e. the worktree root the suite ran in) atomically, with
# exactly these keys:
#   head_sha    — git rev-parse HEAD of the recording worktree
#   command     — the full-suite command that was run
#   status      — its exit status (JSON number)
#   recorded_at — UTC RFC 3339 timestamp
#
# Exit codes:
#   0  recorded
#   2  usage / not-a-repo / serialization error — fail-closed, nothing written

set -euo pipefail

usage() {
    cat <<'EOF'
Usage: record-verify-evidence.sh --pr N --command <cmd> --status <exit>

Writes .autospec/verify-evidence/<PR>.json (worktree-relative) with keys
head_sha, command, status, recorded_at.
EOF
    exit 2
}

_die() {
    printf 'record-verify-evidence: %s\n' "$1" >&2
    exit 2
}

PR=""
COMMAND=""
STATUS=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --pr) [ "$#" -ge 2 ] || usage; PR="$2"; shift 2 ;;
        --command) [ "$#" -ge 2 ] || usage; COMMAND="$2"; shift 2 ;;
        --status) [ "$#" -ge 2 ] || usage; STATUS="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) usage ;;
    esac
done

[ -n "$PR" ] || usage
[ -n "$COMMAND" ] || usage
[ -n "$STATUS" ] || usage
case "$PR" in
    *[!0-9]*) _die "--pr must be a positive integer, got: $PR" ;;
esac
case "$STATUS" in
    *[!0-9]*) _die "--status must be a non-negative integer exit code, got: $STATUS" ;;
esac

command -v jq >/dev/null 2>&1 || _die "jq is required to serialize the evidence (fail-closed)"

HEAD_SHA="$(git rev-parse HEAD 2>/dev/null)" \
    || _die "not a git worktree (cannot record head_sha)"

RECORDED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

OUT_DIR="$(pwd)/.autospec/verify-evidence"
mkdir -p "$OUT_DIR"
OUT="$OUT_DIR/$PR.json"

TMP="$(mktemp "$OUT_DIR/.$PR.XXXXXX")"
if ! jq -n \
        --arg head_sha "$HEAD_SHA" \
        --arg command "$COMMAND" \
        --argjson status "$STATUS" \
        --arg recorded_at "$RECORDED_AT" \
        '{head_sha: $head_sha, command: $command, status: $status, recorded_at: $recorded_at}' > "$TMP"; then
    rm -f "$TMP"
    _die "could not serialize the evidence JSON (fail-closed)"
fi
mv "$TMP" "$OUT"
printf 'recorded verify evidence for PR #%s at %s -> %s\n' "$PR" "$HEAD_SHA" "$OUT"
