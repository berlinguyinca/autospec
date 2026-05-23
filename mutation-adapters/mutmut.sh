#!/usr/bin/env bash
# mutation-adapters/mutmut.sh — mutmut mutation testing adapter for Python.
#
# Usage:
#   bash mutation-adapters/mutmut.sh <source-file.py> [<test-dir>]
#
# Invokes `mutmut run --paths-to-mutate <file>` then parses `mutmut results`
# to count total and killed mutants. Emits JSON matching M2 contract:
#   { total: N, killed: K, file: "<path>" }
#
# Exit codes:
#   0 — all mutants killed (or tool absent — exits 0 with warning)
#   1 — one or more mutants survived
#   2 — usage/setup error
#
# Tool-absent fallback (spec §10): emits warning to stderr and exits 0.

set -eu

# ── Argument parsing ──────────────────────────────────────────────────────────

if [ $# -lt 1 ]; then
    printf 'Usage: bash mutation-adapters/mutmut.sh <source-file.py> [<test-dir>]\n' >&2
    exit 2
fi

SOURCE_FILE="$1"
TEST_DIR="${2:-}"

# ── Tool-absent fallback ──────────────────────────────────────────────────────

if ! command -v mutmut >/dev/null 2>&1; then
    printf 'mutmut.sh: mutmut not found — skipping mutation testing (tool absent)\n' >&2
    printf '{"total":0,"killed":0,"file":"%s"}\n' "$SOURCE_FILE"
    exit 0
fi

if [ ! -f "$SOURCE_FILE" ]; then
    printf 'mutmut.sh: source file not found: %s\n' "$SOURCE_FILE" >&2
    exit 2
fi

# ── Run mutmut ────────────────────────────────────────────────────────────────

WORK_DIR="$(mktemp -d -t mutmut-adapter-XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT INT TERM

# Run mutation testing on the specific file
set +e
mutmut run --paths-to-mutate "$SOURCE_FILE" >/dev/null 2>&1
set -e

# ── Parse mutmut results ──────────────────────────────────────────────────────

# mutmut results output format:
#   KILLED mutants:     12
#   SURVIVED mutants:   2
#   TIMEOUT mutants:    0
#   SUSPICIOUS mutants: 0

RESULTS="$(mutmut results 2>/dev/null || echo '')"

KILLED=0
SURVIVED=0
TIMEOUT_COUNT=0

if [ -n "$RESULTS" ]; then
    KILLED="$(printf '%s\n' "$RESULTS" | grep -iE '^KILLED' | grep -oE '[0-9]+' | head -1 || echo 0)"
    SURVIVED="$(printf '%s\n' "$RESULTS" | grep -iE '^SURVIVED' | grep -oE '[0-9]+' | head -1 || echo 0)"
    TIMEOUT_COUNT="$(printf '%s\n' "$RESULTS" | grep -iE '^TIMEOUT' | grep -oE '[0-9]+' | head -1 || echo 0)"
fi

KILLED="${KILLED:-0}"
SURVIVED="${SURVIVED:-0}"
TIMEOUT_COUNT="${TIMEOUT_COUNT:-0}"
TOTAL=$(( KILLED + SURVIVED + TIMEOUT_COUNT ))

printf '{"total":%d,"killed":%d,"file":"%s"}\n' "$TOTAL" "$KILLED" "$SOURCE_FILE"

if [ "$SURVIVED" -gt 0 ]; then
    exit 1
fi
exit 0
