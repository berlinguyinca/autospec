#!/usr/bin/env bash
# scripts/gen-pr-report.sh — render autospec-test PR comment from gate/drift/loop-log JSON.
#
# Usage:
#   scripts/gen-pr-report.sh --gate <file> [--drift <file>] [--loop-log <file>] [--mode <mode>]
#   scripts/gen-pr-report.sh --help
#
# Required:
#   --gate <file>      Gate JSON file (Stage 1 + Stage 2 + Stage 2.5 results)
#
# Optional:
#   --drift <file>     Drift JSON file (drift findings); defaults to empty
#   --loop-log <file>  Loop iteration log file; defaults to empty
#   --mode <mode>      Mode label (e.g. test, review); defaults to "test"
#
# Output: markdown PR comment on stdout, beginning with <!-- autospec-test-report-marker -->
#
# Dependencies: jq (only non-coreutils dependency)
#
# Exit codes:
#   0  — success
#   1  — missing required input or file not found

set -eu

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
GATE_FILE=""
DRIFT_FILE=""
LOOP_LOG=""
MODE="test"

usage() {
  cat <<'EOF'
gen-pr-report.sh — render autospec-test PR comment from gate/drift/loop-log JSON

Usage:
  scripts/gen-pr-report.sh --gate <file> [--drift <file>] [--loop-log <file>] [--mode <mode>]
  scripts/gen-pr-report.sh --help

Required:
  --gate <file>      Gate JSON (Stage 1 + Stage 2 + Stage 2.5 results)

Optional:
  --drift <file>     Drift JSON (drift findings)
  --loop-log <file>  Loop iteration log
  --mode <mode>      Mode label (default: test)

Exit: 0=success 1=error
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --gate)
      [ -z "${2:-}" ] && { printf 'MISSING_INPUT:gate\n' >&2; exit 1; }
      GATE_FILE="$2"; shift 2 ;;
    --drift)
      [ -z "${2:-}" ] && { printf 'gen-pr-report.sh: --drift requires a file\n' >&2; exit 1; }
      DRIFT_FILE="$2"; shift 2 ;;
    --loop-log)
      [ -z "${2:-}" ] && { printf 'gen-pr-report.sh: --loop-log requires a file\n' >&2; exit 1; }
      LOOP_LOG="$2"; shift 2 ;;
    --mode)
      [ -z "${2:-}" ] && { printf 'gen-pr-report.sh: --mode requires a value\n' >&2; exit 1; }
      MODE="$2"; shift 2 ;;
    *)
      printf 'gen-pr-report.sh: unknown option: %s\n' "$1" >&2; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------
if [ -z "$GATE_FILE" ]; then
  printf 'MISSING_INPUT:gate\n' >&2
  exit 1
fi

if [ ! -f "$GATE_FILE" ]; then
  printf 'gen-pr-report.sh: gate file not found: %s\n' "$GATE_FILE" >&2
  exit 1
fi

if [ -n "$DRIFT_FILE" ] && [ ! -f "$DRIFT_FILE" ]; then
  printf 'gen-pr-report.sh: drift file not found: %s\n' "$DRIFT_FILE" >&2
  exit 1
fi

if [ -n "$LOOP_LOG" ] && [ ! -f "$LOOP_LOG" ]; then
  printf 'gen-pr-report.sh: loop-log file not found: %s\n' "$LOOP_LOG" >&2
  exit 1
fi

# Check jq is available
if ! command -v jq &>/dev/null; then
  printf 'gen-pr-report.sh: jq is required but not found in PATH\n' >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Extract variables from gate JSON
# ---------------------------------------------------------------------------
GATE_PASSED="$(jq -r '.passed // false' "$GATE_FILE")"
OUTCOME="$(jq -r '.outcome // "UNKNOWN"' "$GATE_FILE")"
CODING_TIME_USED="$(jq -r '.coding_time_used // "?"' "$GATE_FILE")"
CODING_TIME_BUDGET="$(jq -r '.coding_time_budget // "?"' "$GATE_FILE")"
ITER_COUNT="$(jq -r '.iter_count // 0' "$GATE_FILE")"
MAX_ITERS="$(jq -r '.max_iters // 5' "$GATE_FILE")"

# Derive status emoji and text from outcome
case "$OUTCOME" in
  PASS)
    STATUS_EMOJI="✅"
    STATUS_TEXT="passed"
    OUTCOME_SHORT="it passed"
    ;;
  BEHIND_REBASED)
    STATUS_EMOJI="🔄"
    STATUS_TEXT="needs rebase"
    OUTCOME_SHORT="it needs a rebase"
    ;;
  MAX_ITERS_HIT)
    STATUS_EMOJI="⏱️"
    STATUS_TEXT="max iterations hit"
    OUTCOME_SHORT="max iterations were hit"
    ;;
  DRIFT_ONLY)
    STATUS_EMOJI="📄"
    STATUS_TEXT="doc drift detected"
    OUTCOME_SHORT="doc drift was detected"
    ;;
  *)
    if [ "$GATE_PASSED" = "true" ]; then
      STATUS_EMOJI="✅"
      STATUS_TEXT="passed"
      OUTCOME_SHORT="it passed"
    else
      STATUS_EMOJI="❌"
      STATUS_TEXT="failed"
      OUTCOME_SHORT="it failed"
    fi
    ;;
esac

# ---------------------------------------------------------------------------
# Build failed metrics list
# ---------------------------------------------------------------------------
METRICS_BLOCK=""
METRICS_COUNT="$(jq '.metrics | length' "$GATE_FILE" 2>/dev/null || echo 0)"
i=0
while [ "$i" -lt "$METRICS_COUNT" ]; do
  PASSED="$(jq -r ".metrics[$i].passed" "$GATE_FILE")"
  if [ "$PASSED" = "false" ]; then
    LABEL="$(jq -r ".metrics[$i].label" "$GATE_FILE")"
    REASON="$(jq -r ".metrics[$i].failure_reason" "$GATE_FILE")"
    METRICS_BLOCK="${METRICS_BLOCK}- ${LABEL}: ${REASON}
"
  fi
  i=$((i + 1))
done

# ---------------------------------------------------------------------------
# Build drift section
# ---------------------------------------------------------------------------
DRIFT_BLOCK=""
if [ -n "$DRIFT_FILE" ] && [ -f "$DRIFT_FILE" ]; then
  DRIFT_PASSED="$(jq -r 'if .passed == false then false else true end' "$DRIFT_FILE")"
  if [ "$DRIFT_PASSED" = "false" ]; then
    DRIFT_COUNT="$(jq '.drift | length' "$DRIFT_FILE" 2>/dev/null || echo 0)"
    if [ "$DRIFT_COUNT" -gt 0 ]; then
      DRIFT_BLOCK="
### Doc drift

"
      j=0
      while [ "$j" -lt "$DRIFT_COUNT" ]; do
        DOC="$(jq -r ".drift[$j].doc_file" "$DRIFT_FILE")"
        HEADING="$(jq -r ".drift[$j].heading" "$DRIFT_FILE")"
        DRIFT_BLOCK="${DRIFT_BLOCK}- \`${DOC}\` § ${HEADING}
"
        j=$((j + 1))
      done
    fi
  fi
fi

# ---------------------------------------------------------------------------
# Build loop summary
# ---------------------------------------------------------------------------
LOOP_BLOCK=""
if [ -n "$LOOP_LOG" ] && [ -f "$LOOP_LOG" ]; then
  LOOP_LINES="$(wc -l < "$LOOP_LOG" | tr -d ' ')"
  if [ "$LOOP_LINES" -gt 0 ]; then
    LOOP_BLOCK="
### Loop iterations

\`\`\`
$(cat "$LOOP_LOG")
\`\`\`
"
  fi
fi

# ---------------------------------------------------------------------------
# Render template
# ---------------------------------------------------------------------------
TODAY="$(date -u +%Y-%m-%d 2>/dev/null || date +%Y-%m-%d)"

cat <<REPORT
<!-- autospec-test-report-marker -->
## autospec-test — ${STATUS_EMOJI} ${STATUS_TEXT}

**Mode:** ${MODE}
**Coding time used:** ${CODING_TIME_USED} / ${CODING_TIME_BUDGET}
**Iterations:** ${ITER_COUNT} / ${MAX_ITERS}

### Why ${OUTCOME_SHORT}
${METRICS_BLOCK}${DRIFT_BLOCK}${LOOP_BLOCK}
---
*Generated ${TODAY} by gen-pr-report.sh (zero LLM calls)*
REPORT
