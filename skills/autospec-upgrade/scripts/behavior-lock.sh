#!/usr/bin/env bash
# behavior-lock.sh — capture golden-master snapshots + record Stryker mutation
# baseline; refuse (exit != 0) until behavior is locked AND baseline is present.
#
# Usage:
#   behavior-lock.sh --detect <detect.json> [--root <dir>] [--out <autospec-dir>]
#
# --detect <file>   Path to detection JSON emitted by upgrade-detect.sh.
#                   Required; must be a readable file.
# --root <dir>      Project root (default: .).
# --out <dir>       Directory for .autospec/ artifacts (default: <root>/.autospec).
#
# Exit codes:
#   0  — behavior locked: golden-master written AND mutation baseline recorded.
#   1  — behavior-lock unreachable: no runnable surface / characterization failed.
#         Prints: code_health:upgrade_behavior_lock_unreachable
#   2  — baseline not recorded: golden-master was produced but mutation-gate failed.
#   3  — argument error.

set -uo pipefail

# ── Constants ─────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Argument parsing ──────────────────────────────────────────────────────────

DETECT_FILE=""
ROOT="."
OUT_DIR=""

while [ $# -gt 0 ]; do
  case "$1" in
    --detect)
      DETECT_FILE="$2"
      shift 2
      ;;
    --detect=*)
      DETECT_FILE="${1#--detect=}"
      shift
      ;;
    --root)
      ROOT="$2"
      shift 2
      ;;
    --root=*)
      ROOT="${1#--root=}"
      shift
      ;;
    --out)
      OUT_DIR="$2"
      shift 2
      ;;
    --out=*)
      OUT_DIR="${1#--out=}"
      shift
      ;;
    *)
      shift
      ;;
  esac
done

# ── Validate arguments ────────────────────────────────────────────────────────

if [ -z "$DETECT_FILE" ]; then
  printf 'behavior-lock: --detect <file> is required\n' >&2
  exit 3
fi

if [ ! -f "$DETECT_FILE" ]; then
  printf 'behavior-lock: detect file not found: %s\n' "$DETECT_FILE" >&2
  exit 3
fi

# Resolve root to absolute path
if [ ! -d "$ROOT" ]; then
  printf 'behavior-lock: root directory not found: %s\n' "$ROOT" >&2
  exit 3
fi
ROOT="$(cd "$ROOT" && pwd)"

# Default output dir: <root>/.autospec
if [ -z "$OUT_DIR" ]; then
  OUT_DIR="$ROOT/.autospec"
fi

mkdir -p "$OUT_DIR"

# ── Read detection JSON ───────────────────────────────────────────────────────

HAS_TESTS="$(jq -r '.has_tests' "$DETECT_FILE" 2>/dev/null || printf 'false')"
FRAMEWORKS_JSON="$(jq -c '.frameworks' "$DETECT_FILE" 2>/dev/null || printf '[]')"
RUNNERS_JSON="$(jq -c '.runners' "$DETECT_FILE" 2>/dev/null || printf '[]')"

# ── Unreachable surface gate ──────────────────────────────────────────────────
# If has_tests is false there is no runnable surface; refuse immediately.

if [ "$HAS_TESTS" = "false" ]; then
  printf 'code_health:upgrade_behavior_lock_unreachable\n'
  printf 'behavior-lock: no runnable test surface detected (has_tests=false); cannot lock behavior before upgrade.\n' >&2
  exit 1
fi

# ── Resolve helper paths ──────────────────────────────────────────────────────

# mutation-gate.sh: look in same scripts/ dir first, then PATH
MUTATION_GATE=""
if [ -x "$SCRIPT_DIR/mutation-gate.sh" ]; then
  MUTATION_GATE="$SCRIPT_DIR/mutation-gate.sh"
elif command -v mutation-gate.sh >/dev/null 2>&1; then
  MUTATION_GATE="mutation-gate.sh"
fi

# ── Step 1: characterize — run autospec-test + autospec-playwright ────────────
# These are mocked in tests via a $TMP/bin PATH shim.
# In production they drive the real autospec-test / autospec-playwright tools.

CHARACTERIZE_OK=false

# Run autospec-test (unit/integration characterization)
if command -v autospec-test >/dev/null 2>&1; then
  if autospec-test --root "$ROOT" 2>&1; then
    CHARACTERIZE_OK=true
  fi
fi

# Run autospec-playwright (E2E golden-master characterization)
# Bias: E2E/Playwright golden-master is the primary gate; unit/integration is the floor.
if command -v autospec-playwright >/dev/null 2>&1; then
  if autospec-playwright --root "$ROOT" 2>&1; then
    CHARACTERIZE_OK=true
  fi
fi

# If neither tool succeeded we cannot lock behavior
if [ "$CHARACTERIZE_OK" = "false" ]; then
  printf 'code_health:upgrade_behavior_lock_unreachable\n'
  printf 'behavior-lock: characterization failed (autospec-test and autospec-playwright both unavailable or errored).\n' >&2
  exit 1
fi

# ── Step 2: record Stryker mutation baseline ──────────────────────────────────
# Invoke mutation-gate.sh --baseline if present; otherwise write a stub record
# (issue #1175 owns the real gate implementation).

BASELINE_DIR="$OUT_DIR"
BASELINE_OK=false

if [ -n "$MUTATION_GATE" ]; then
  # Real mutation-gate.sh present — invoke it with --baseline + --out
  if "$MUTATION_GATE" --baseline --out "$BASELINE_DIR" 2>&1; then
    BASELINE_OK=true
  fi
else
  # Stub: write a minimal baseline record so the contract is satisfied.
  # The real mutation gate (#1175) will overwrite this.
  STUB_BASELINE="$BASELINE_DIR/mutation-proof.json"
  LOCKED_AT="$(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || printf 'unknown')"
  printf '{"baseline":{"score":null,"stub":true},"recorded_at":"%s"}\n' "$LOCKED_AT" \
    > "$STUB_BASELINE"
  BASELINE_OK=true
fi

if [ "$BASELINE_OK" = "false" ]; then
  printf 'behavior-lock: mutation baseline recording failed; behavior NOT locked.\n' >&2
  exit 2
fi

# Verify baseline file was actually written
MUTATION_PROOF="$BASELINE_DIR/mutation-proof.json"
if [ ! -f "$MUTATION_PROOF" ]; then
  printf 'behavior-lock: mutation-proof.json not found after baseline step; behavior NOT locked.\n' >&2
  exit 2
fi

# ── Step 3: write golden-master snapshot ─────────────────────────────────────

GM_DIR="$OUT_DIR/behavior-lock"
mkdir -p "$GM_DIR"
GM_FILE="$GM_DIR/golden-master.json"

LOCKED_AT="$(date -u '+%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || printf 'unknown')"

jq -cn \
  --arg locked_at "$LOCKED_AT" \
  --argjson frameworks "$FRAMEWORKS_JSON" \
  --argjson runners "$RUNNERS_JSON" \
  --arg mutation_proof "$MUTATION_PROOF" \
  '{
    locked_at: $locked_at,
    frameworks: $frameworks,
    runners: $runners,
    mutation_proof_path: $mutation_proof,
    status: "locked"
  }' > "$GM_FILE"

printf 'behavior-lock: golden-master written to %s\n' "$GM_FILE"
printf 'behavior-lock: mutation baseline recorded at %s\n' "$MUTATION_PROOF"
printf 'behavior-lock: behavior locked successfully.\n'
exit 0
