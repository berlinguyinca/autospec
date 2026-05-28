#!/usr/bin/env bash
# compute-release-verdict.sh — deterministic release verdict from qa-verdict.json.
#
# Reads .autospec/qa-verdict.json (or a path given as $1) plus the current git
# HEAD (or override given as $2), applies /autospec-release Stage 9's gating
# rules in code, and emits exactly one of PASS / PARTIAL / FAIL on stdout.
#
# This script is the single source of truth for the release verdict
# computation. /autospec-release's prose Stage 9 describes the same rules but
# delegates to this script so a future LLM cannot rationalize past them.
#
# Usage:
#   compute-release-verdict.sh [VERDICT_PATH] [HEAD_SHA_OVERRIDE]
#
# Exit codes:
#   0 PASS
#   1 PARTIAL
#   2 FAIL
#   3 infrastructure error (jq missing, not a git repo, etc.)
#
# stdout: single uppercase verdict word.
# stderr: one-line reason describing which rule fired.

set -u

VERDICT_FILE="${1:-.autospec/qa-verdict.json}"
HEAD_SHA="${2:-}"

emit() {
    # $1 = PASS|PARTIAL|FAIL, $2 = reason
    printf '%s\n' "$1"
    printf 'reason: %s\n' "$2" >&2
    case "$1" in
        PASS) exit 0 ;;
        PARTIAL) exit 1 ;;
        FAIL) exit 2 ;;
        *) exit 3 ;;
    esac
}

if ! command -v jq >/dev/null 2>&1; then
    printf 'compute-release-verdict: jq is required\n' >&2
    exit 3
fi

if [ -z "$HEAD_SHA" ]; then
    HEAD_SHA=$(git rev-parse HEAD 2>/dev/null || true)
fi

if [ -z "$HEAD_SHA" ]; then
    printf 'compute-release-verdict: cannot determine git HEAD (run inside a repo or pass HEAD_SHA_OVERRIDE)\n' >&2
    exit 3
fi

if [ ! -f "$VERDICT_FILE" ]; then
    emit FAIL "qa-verdict.json not produced by Stage 7 (path: $VERDICT_FILE)"
fi

if ! jq -e . "$VERDICT_FILE" >/dev/null 2>&1; then
    emit FAIL "qa-verdict.json is not valid JSON (path: $VERDICT_FILE)"
fi

verdict_head=$(jq -r '.head_sha // empty' "$VERDICT_FILE")
if [ -z "$verdict_head" ]; then
    emit FAIL "qa-verdict.json is missing head_sha field"
fi

if [ "$verdict_head" != "$HEAD_SHA" ]; then
    emit FAIL "QA verdict stale relative to HEAD (verdict=$verdict_head, head=$HEAD_SHA)"
fi

live_app_proof=$(jq -r '.live_app_proof // false' "$VERDICT_FILE")

blocking_fail=$(jq '[.findings[]? | select(.release_blocking == true and .status == "FAIL")] | length' "$VERDICT_FILE")
blocking_partial=$(jq '[.findings[]? | select(.release_blocking == true and (.status == "PARTIAL" or .status == "NOT_TESTED"))] | length' "$VERDICT_FILE")

if [ "$blocking_fail" -gt 0 ]; then
    summaries=$(jq -r '[.findings[]? | select(.release_blocking == true and .status == "FAIL") | .summary] | join("; ")' "$VERDICT_FILE")
    emit FAIL "$blocking_fail release-blocking finding(s) with status FAIL: $summaries"
fi

if [ "$live_app_proof" != "true" ]; then
    emit PARTIAL "live_app_proof is false; Stage 7 lacks browser/app-level artifacts"
fi

if [ "$blocking_partial" -gt 0 ]; then
    summaries=$(jq -r '[.findings[]? | select(.release_blocking == true and (.status == "PARTIAL" or .status == "NOT_TESTED")) | .summary] | join("; ")' "$VERDICT_FILE")
    emit PARTIAL "$blocking_partial release-blocking finding(s) with status PARTIAL/NOT_TESTED: $summaries"
fi

emit PASS "all gates green; verdict file confirms live-app proof and no blocking findings"
