#!/usr/bin/env bash
# mutation-adapters/stryker.sh — Stryker mutation testing adapter for Node/TS/JS.
#
# Usage:
#   bash mutation-adapters/stryker.sh <source-file> [<test-dir>]
#
# Invokes `npx stryker run` for the given source file and emits aggregate
# JSON to stdout matching the M2 contract: { total: N, killed: K, file: "<path>" }
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
    printf 'Usage: bash mutation-adapters/stryker.sh <source-file> [<test-dir>]\n' >&2
    exit 2
fi

SOURCE_FILE="$1"
TEST_DIR="${2:-}"

# ── Tool-absent fallback ──────────────────────────────────────────────────────

if ! command -v npx >/dev/null 2>&1; then
    printf 'stryker.sh: npx not found — skipping mutation testing (tool absent)\n' >&2
    printf '{"total":0,"killed":0,"file":"%s"}\n' "$SOURCE_FILE"
    exit 0
fi

# Check if stryker is available
if ! npx stryker --version >/dev/null 2>&1; then
    printf 'stryker.sh: stryker not installed — skipping mutation testing (tool absent)\n' >&2
    printf '{"total":0,"killed":0,"file":"%s"}\n' "$SOURCE_FILE"
    exit 0
fi

if [ ! -f "$SOURCE_FILE" ]; then
    printf 'stryker.sh: source file not found: %s\n' "$SOURCE_FILE" >&2
    exit 2
fi

# ── Locate repo root ──────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── Run Stryker ───────────────────────────────────────────────────────────────

WORK_DIR="$(mktemp -d -t stryker-adapter-XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT INT TERM

STRYKER_REPORT="$WORK_DIR/stryker-report.json"

# Build stryker config targeting the single source file
STRYKER_CFG="$WORK_DIR/stryker.config.json"
cat > "$STRYKER_CFG" << STRYKER_JSON
{
  "mutate": ["${SOURCE_FILE}"],
  "reporters": ["json"],
  "jsonReporter": { "fileName": "${STRYKER_REPORT}" },
  "testRunner": "command",
  "commandRunner": { "command": "${TEST_DIR:+bats $TEST_DIR}" }
}
STRYKER_JSON

cd "$REPO_ROOT"
set +e
npx stryker run "$STRYKER_CFG" >/dev/null 2>&1
STRYKER_EXIT=$?
set -e

# ── Parse Stryker JSON report ─────────────────────────────────────────────────

if [ ! -f "$STRYKER_REPORT" ]; then
    printf 'stryker.sh: stryker did not produce a JSON report\n' >&2
    printf '{"total":0,"killed":0,"file":"%s"}\n' "$SOURCE_FILE"
    exit 1
fi

TOTAL=0
KILLED=0

if command -v node >/dev/null 2>&1; then
    TOTAL="$(node -e "
const r=JSON.parse(require('fs').readFileSync('$STRYKER_REPORT','utf8'));
const files=r.files||{};
let t=0;
for(const f of Object.values(files)){for(const m of f.mutants||[]){t++;}}
process.stdout.write(String(t));
" 2>/dev/null || echo 0)"

    KILLED="$(node -e "
const r=JSON.parse(require('fs').readFileSync('$STRYKER_REPORT','utf8'));
const files=r.files||{};
let k=0;
for(const f of Object.values(files)){for(const m of f.mutants||[]){if(m.status==='Killed')k++;}}
process.stdout.write(String(k));
" 2>/dev/null || echo 0)"
fi

printf '{"total":%d,"killed":%d,"file":"%s"}\n' "$TOTAL" "$KILLED" "$SOURCE_FILE"

SURVIVED=$(( TOTAL - KILLED ))
if [ "$SURVIVED" -gt 0 ]; then
    exit 1
fi
exit 0
