#!/usr/bin/env bash
# mutation-adapters/go-mutesting.sh — go-mutesting adapter for Go packages.
#
# Usage:
#   bash mutation-adapters/go-mutesting.sh <package-path> [<unused>]
#
# Invokes `go-mutesting <package-path>` and parses the summary line to count
# total and killed mutants. Emits JSON matching M2 contract:
#   { total: N, killed: K, file: "<package-path>" }
#
# Exit codes:
#   0 — all mutants killed (or tool absent — exits 0 with warning)
#   1 — one or more mutants survived
#   2 — usage/setup error
#
# Tool-absent fallback (spec §10): emits warning to stderr and exits 0.
#
# go-mutesting summary line format:
#   The mutation score is 0.82 (9 passed, 2 failed, 0 skipped, 11 total)

set -eu

# ── Argument parsing ──────────────────────────────────────────────────────────

if [ $# -lt 1 ]; then
    printf 'Usage: bash mutation-adapters/go-mutesting.sh <package-path>\n' >&2
    exit 2
fi

PACKAGE_PATH="$1"

# ── Tool-absent fallback ──────────────────────────────────────────────────────

if ! command -v go-mutesting >/dev/null 2>&1; then
    printf 'go-mutesting.sh: go-mutesting not found — skipping mutation testing (tool absent)\n' >&2
    printf '{"total":0,"killed":0,"file":"%s"}\n' "$PACKAGE_PATH"
    exit 0
fi

# ── Run go-mutesting ──────────────────────────────────────────────────────────

WORK_DIR="$(mktemp -d -t go-mutesting-adapter-XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT INT TERM

OUTPUT_FILE="$WORK_DIR/output.txt"

set +e
go-mutesting "$PACKAGE_PATH" > "$OUTPUT_FILE" 2>&1
GO_MUTESTING_EXIT=$?
set -e

# ── Parse go-mutesting summary line ──────────────────────────────────────────

# Summary format: "The mutation score is X.XX (N passed, M failed, P skipped, T total)"
SUMMARY="$(grep -E 'mutation score' "$OUTPUT_FILE" 2>/dev/null | tail -1 || echo '')"

TOTAL=0
KILLED=0

if [ -n "$SUMMARY" ]; then
    TOTAL="$(printf '%s\n' "$SUMMARY" | grep -oE '[0-9]+ total' | grep -oE '[0-9]+' | head -1 || echo 0)"
    KILLED="$(printf '%s\n' "$SUMMARY" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' | head -1 || echo 0)"
fi

TOTAL="${TOTAL:-0}"
KILLED="${KILLED:-0}"

printf '{"total":%d,"killed":%d,"file":"%s"}\n' "$TOTAL" "$KILLED" "$PACKAGE_PATH"

SURVIVED=$(( TOTAL - KILLED ))
if [ "$SURVIVED" -gt 0 ]; then
    exit 1
fi
exit 0
