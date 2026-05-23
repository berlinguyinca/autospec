#!/usr/bin/env bash
# mutation-adapters/bash-mutate.sh — bash mutation testing adapter.
#
# Usage:
#   bash mutation-adapters/bash-mutate.sh <source.sh> [<bats-test-dir>]
#
# Applies bash-mutate.mjs to <source.sh>, runs bats for each mutant, and
# emits aggregate JSON to stdout: { total: N, killed: K, file: "<path>" }
#
# Exit codes:
#   0 — all mutants killed (full mutation coverage)
#   1 — one or more mutants survived (coverage gap)
#   2 — usage/setup error
#
# Environment:
#   BASH_MUTATE_MJS   path to bash-mutate.mjs (default: scripts/bash-mutate.mjs
#                     relative to this script's repo root)
#   BASH_MUTATE_WORK  scratch directory for mutants (default: tmp dir, auto-cleaned)

set -eu

# ── Argument parsing ──────────────────────────────────────────────────────────

if [ $# -lt 1 ]; then
    printf 'Usage: bash mutation-adapters/bash-mutate.sh <source.sh> [<bats-test-dir>]\n' >&2
    exit 2
fi

SOURCE_FILE="$1"
BATS_TEST_DIR="${2:-}"

if [ ! -f "$SOURCE_FILE" ]; then
    printf 'bash-mutate.sh: source file not found: %s\n' "$SOURCE_FILE" >&2
    exit 2
fi

# Locate repo root (parent of this script's directory)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Default bash-mutate.mjs location
BASH_MUTATE_MJS="${BASH_MUTATE_MJS:-$REPO_ROOT/scripts/bash-mutate.mjs}"

if [ ! -f "$BASH_MUTATE_MJS" ]; then
    printf 'bash-mutate.sh: bash-mutate.mjs not found at %s\n' "$BASH_MUTATE_MJS" >&2
    exit 2
fi

if ! command -v node >/dev/null 2>&1; then
    printf 'bash-mutate.sh: node not found in PATH\n' >&2
    exit 2
fi

if ! command -v bats >/dev/null 2>&1; then
    printf 'bash-mutate.sh: bats not found in PATH\n' >&2
    exit 2
fi

# ── Resolve bats test directory ───────────────────────────────────────────────

if [ -z "$BATS_TEST_DIR" ]; then
    # Derive from source file: <repo>/tests/<basename>.bats or tests/ dir
    SRC_BASE="$(basename "$SOURCE_FILE" .sh)"
    CANDIDATE="$REPO_ROOT/tests/${SRC_BASE}.bats"
    if [ -f "$CANDIDATE" ]; then
        BATS_TEST_DIR="$CANDIDATE"
    else
        BATS_TEST_DIR="$REPO_ROOT/tests"
    fi
fi

# ── Work directory setup ──────────────────────────────────────────────────────

if [ -n "${BASH_MUTATE_WORK:-}" ]; then
    WORK_DIR="$BASH_MUTATE_WORK"
    mkdir -p "$WORK_DIR"
    CLEANUP_WORK=0
else
    WORK_DIR="$(mktemp -d -t bash-mutate-XXXXXX)"
    CLEANUP_WORK=1
fi

cleanup() {
    if [ "${CLEANUP_WORK:-0}" -eq 1 ] && [ -d "${WORK_DIR:-}" ]; then
        rm -rf "$WORK_DIR"
    fi
}
trap cleanup EXIT INT TERM

EMIT_DIR="$WORK_DIR/mutants"
mkdir -p "$EMIT_DIR"

# ── Generate mutants ──────────────────────────────────────────────────────────

MUTANTS_JSON="$WORK_DIR/mutants.json"
node "$BASH_MUTATE_MJS" --file "$SOURCE_FILE" --emit "$EMIT_DIR" > "$MUTANTS_JSON" 2>/dev/null || {
    printf 'bash-mutate.sh: bash-mutate.mjs failed on %s\n' "$SOURCE_FILE" >&2
    exit 2
}

TOTAL="$(node -e "const d=JSON.parse(require('fs').readFileSync('$MUTANTS_JSON','utf8')); process.stdout.write(String(d.length));" 2>/dev/null)"
TOTAL="${TOTAL:-0}"

if [ "$TOTAL" -eq 0 ]; then
    printf '{"total":0,"killed":0,"file":"%s"}\n' "$SOURCE_FILE"
    exit 0
fi

# ── Apply each mutant and run bats ────────────────────────────────────────────

KILLED=0
SURVIVED=0

for idx in $(seq 0 $((TOTAL - 1))); do
    OUT_FILE="$(node -e "const d=JSON.parse(require('fs').readFileSync('$MUTANTS_JSON','utf8')); process.stdout.write(d[$idx].out_file);" 2>/dev/null)"
    [ -z "$OUT_FILE" ] && continue
    [ ! -f "$OUT_FILE" ] && continue

    # Back up original and apply mutant
    BACKUP="$WORK_DIR/original_backup.sh"
    cp "$SOURCE_FILE" "$BACKUP"
    cp "$OUT_FILE" "$SOURCE_FILE"

    # Run bats against the mutant; exit non-zero = mutant killed (tests caught it)
    if ! bats "$BATS_TEST_DIR" >/dev/null 2>&1; then
        KILLED=$((KILLED + 1))
    else
        SURVIVED=$((SURVIVED + 1))
    fi

    # Restore original
    cp "$BACKUP" "$SOURCE_FILE"
done

# ── Emit result JSON ──────────────────────────────────────────────────────────

printf '{"total":%d,"killed":%d,"file":"%s"}\n' "$TOTAL" "$KILLED" "$SOURCE_FILE"

if [ "$SURVIVED" -gt 0 ]; then
    exit 1
fi
exit 0
