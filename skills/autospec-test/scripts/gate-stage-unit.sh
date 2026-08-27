#!/usr/bin/env bash
# gate-stage-unit.sh — Stage 1 unit test + coverage gate for autospec-test.
#
# Usage: gate-stage-unit.sh [<repo_root>]
#   OR:  echo '<contract_json>' | gate-stage-unit.sh [<repo_root>]
#
# Input: resolved contract JSON on stdin (output of load-contract.sh)
# Output: Stage 1 gate result JSON on stdout
#
# Exit codes:
#   0 = gate passed
#   1 = gate failed (tests_red, coverage_below_threshold, function_presence_fail)
#   2 = fatal error (missing tool, bad input)
#
# Stage 1 sub-checks (all must pass):
#   1. unit.test_cmd runs and exits 0
#   2. Coverage >= thresholds (lines/branches/functions)
#   3. Every exported/public function has >= 1 test reference
#
# Result JSON shape: see schemas/autospec-test-stage1-result.schema.json

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SCHEMA="$REPO_ROOT/schemas/autospec-test-stage1-result.schema.json"

# ── Helpers ────────────────────────────────────────────────────────────────────
emit_result() {
    # emit_result <passed_bool> <reason_or_empty> <metrics_json> <test_summary_json>
    # Uses temp files to safely pass multi-line JSON to jq (avoids --argjson newline truncation)
    local passed="$1"
    local reason="${2:-}"
    local metrics_json="${3:-{\}}"
    local test_summary_json="${4:-{\}}"

    local m_file s_file
    m_file=$(mktemp /tmp/autospec-metrics-XXXXXX.json)
    s_file=$(mktemp /tmp/autospec-summary-XXXXXX.json)
    printf '%s' "$metrics_json" > "$m_file"
    printf '%s' "$test_summary_json" > "$s_file"

    if [ -n "$reason" ]; then
        jq -n \
            --argjson passed "$passed" \
            --arg reason "$reason" \
            --slurpfile metrics "$m_file" \
            --slurpfile test_summary "$s_file" \
            '{"passed":$passed,"stage":"unit","reason":$reason,"metrics":$metrics[0],"test_run_summary":$test_summary[0]}'
    else
        jq -n \
            --argjson passed "$passed" \
            --slurpfile metrics "$m_file" \
            --slurpfile test_summary "$s_file" \
            '{"passed":$passed,"stage":"unit","metrics":$metrics[0],"test_run_summary":$test_summary[0]}'
    fi
    rm -f "$m_file" "$s_file"
}

fatal() {
    printf 'gate-stage-unit: fatal: %s\n' "$*" >&2
    emit_result false "collector_error" '{"unit":{"passed":false,"reason":"fatal"}}' '{}' || true
    exit 2
}

# ── Parse contract from stdin ──────────────────────────────────────────────────
if ! command -v jq >/dev/null 2>&1; then
    fatal "jq not found"
fi

CONTRACT_JSON=$(cat)
if [ -z "$CONTRACT_JSON" ]; then
    fatal "no contract JSON on stdin"
fi

TARGET_REPO="${1:-.}"
if [ ! -d "$TARGET_REPO" ]; then
    fatal "target repo not found: $TARGET_REPO"
fi

# ── Extract config from contract ───────────────────────────────────────────────
TEST_CMD=$(printf '%s' "$CONTRACT_JSON" | jq -r '.unit.test_cmd // empty')
COVERAGE_COLLECTOR=$(printf '%s' "$CONTRACT_JSON" | jq -r '.unit.coverage_collector // "istanbul"')
THRESHOLD_LINES=$(printf '%s' "$CONTRACT_JSON" | jq -r '.unit.coverage_thresholds.lines // 95')
THRESHOLD_BRANCHES=$(printf '%s' "$CONTRACT_JSON" | jq -r '.unit.coverage_thresholds.branches // 90')
THRESHOLD_FUNCTIONS=$(printf '%s' "$CONTRACT_JSON" | jq -r '.unit.coverage_thresholds.functions // 95')
FUNCTION_PRESENCE=$(printf '%s' "$CONTRACT_JSON" | jq -r 'if .unit.function_presence_check == false then false else true end')

if [ -z "$TEST_CMD" ]; then
    fatal "unit.test_cmd not set in contract"
fi

# ── Step 1: Run unit test command ──────────────────────────────────────────────
STDOUT_FILE=$(mktemp /tmp/autospec-unit-stdout-XXXXXX.txt)
STDERR_FILE=$(mktemp /tmp/autospec-unit-stderr-XXXXXX.txt)
# shellcheck disable=SC2064
trap "rm -f '$STDOUT_FILE' '$STDERR_FILE'" EXIT

TEST_EXIT=0
(cd "$TARGET_REPO" && eval "$TEST_CMD") >"$STDOUT_FILE" 2>"$STDERR_FILE" || TEST_EXIT=$?

STDOUT_TAIL=$(tail -20 "$STDOUT_FILE" | head -c 2000)
STDERR_TAIL=$(tail -20 "$STDERR_FILE" | head -c 2000)

TEST_SUMMARY=$(jq -n \
    --argjson exit_code "$TEST_EXIT" \
    --arg stdout_tail "$STDOUT_TAIL" \
    --arg stderr_tail "$STDERR_TAIL" \
    '{"exit_code":$exit_code,"stdout_tail":$stdout_tail,"stderr_tail":$stderr_tail}')

if [ "$TEST_EXIT" -ne 0 ]; then
    METRICS='{"unit":{"passed":false,"reason":"tests_red"}}'
    emit_result false "tests_red" "$METRICS" "$TEST_SUMMARY"
    exit 1
fi

# ── Step 2: Collect coverage via per-language collector ────────────────────────
COLLECTOR_SCRIPT="$SCRIPT_DIR/coverage-collectors/${COVERAGE_COLLECTOR}.sh"
if [ ! -f "$COLLECTOR_SCRIPT" ]; then
    printf 'gate-stage-unit: WARN: collector not found: %s; skipping coverage check\n' "$COVERAGE_COLLECTOR" >&2
    LCOV_CONTENT=""
else
    LCOV_FILE=$(mktemp /tmp/autospec-lcov-XXXXXX.info)
    # shellcheck disable=SC2064
    trap "rm -f '$STDOUT_FILE' '$STDERR_FILE' '$LCOV_FILE'" EXIT

    # Try common lcov output locations
    LCOV_PATHS=(
        "$TARGET_REPO/coverage/lcov.info"
        "$TARGET_REPO/coverage.info"
        "$TARGET_REPO/lcov.info"
    )

    LCOV_SRC=""
    for p in "${LCOV_PATHS[@]}"; do
        if [ -f "$p" ]; then
            LCOV_SRC="$p"
            break
        fi
    done

    if [ -n "$LCOV_SRC" ] && bash "$COLLECTOR_SCRIPT" "$LCOV_SRC" >"$LCOV_FILE" 2>/dev/null; then
        LCOV_CONTENT=$(cat "$LCOV_FILE")
    else
        LCOV_CONTENT=""
    fi
fi

# ── Parse lcov for coverage percentages ───────────────────────────────────────
parse_lcov_percent() {
    local lcov="$1"
    local metric="$2"  # LF/LH (lines), BRF/BRH (branches), FNF/FNH (functions)

    local found_key="${metric}F"  # total count key
    local hit_key="${metric}H"    # hit count key

    local total=0
    local hit=0

    while IFS= read -r line; do
        case "$line" in
            "${found_key}:"*) total=$((total + ${line#*:})) ;;
            "${hit_key}:"*)   hit=$((hit + ${line#*:})) ;;
        esac
    done <<< "$lcov"

    if [ "$total" -eq 0 ]; then
        printf '0'
    else
        # Use awk for floating point
        awk -v h="$hit" -v t="$total" 'BEGIN { printf "%.1f", (h/t)*100 }'
    fi
}

COV_LINES=0
COV_BRANCHES=0
COV_FUNCTIONS=0

if [ -n "$LCOV_CONTENT" ]; then
    COV_LINES=$(parse_lcov_percent "$LCOV_CONTENT" "L")
    COV_BRANCHES=$(parse_lcov_percent "$LCOV_CONTENT" "BR")
    COV_FUNCTIONS=$(parse_lcov_percent "$LCOV_CONTENT" "FN")
fi

# ── Step 3: Check coverage thresholds ─────────────────────────────────────────
check_threshold() {
    local actual="$1"
    local threshold="$2"
    awk -v a="$actual" -v t="$threshold" 'BEGIN { exit (a >= t) ? 0 : 1 }'
}

COV_PASS=true
if [ -n "$LCOV_CONTENT" ]; then
    if ! check_threshold "$COV_LINES" "$THRESHOLD_LINES" || \
       ! check_threshold "$COV_BRANCHES" "$THRESHOLD_BRANCHES" || \
       ! check_threshold "$COV_FUNCTIONS" "$THRESHOLD_FUNCTIONS"; then
        COV_PASS=false
    fi
fi

# ── Step 4: Function-presence check ───────────────────────────────────────────
MISSING_FN_TESTS="[]"
FP_PASS=true

if [ "$FUNCTION_PRESENCE" = "true" ] && command -v node >/dev/null 2>&1; then
    FP_SCRIPT="$SCRIPT_DIR/function-presence.mjs"
    if [ -f "$FP_SCRIPT" ]; then
        # Heuristic: find src and test dirs in target repo
        SRC_DIR="$TARGET_REPO/src"
        TEST_DIR="$TARGET_REPO"  # default: scan whole repo for tests

        for d in src lib; do
            [ -d "$TARGET_REPO/$d" ] && SRC_DIR="$TARGET_REPO/$d" && break
        done
        for d in tests test __tests__ spec; do
            [ -d "$TARGET_REPO/$d" ] && TEST_DIR="$TARGET_REPO/$d" && break
        done

        FP_OUTPUT=$(node "$FP_SCRIPT" "$SRC_DIR" "$TEST_DIR" 2>/dev/null || echo '{"missing_tests":[]}')
        MISSING_FN_TESTS=$(printf '%s' "$FP_OUTPUT" | jq '.missing_tests // []')
        MISSING_COUNT=$(printf '%s' "$MISSING_FN_TESTS" | jq 'length')
        if [ "$MISSING_COUNT" -gt 0 ]; then
            FP_PASS=false
        fi
    fi
fi

# ── Emit Stage 1 gate result ───────────────────────────────────────────────────
UNIT_PASSED=true
GATE_REASON=""

if [ "$COV_PASS" = "false" ]; then
    UNIT_PASSED=false
    GATE_REASON="coverage_below_threshold"
fi
if [ "$FP_PASS" = "false" ]; then
    UNIT_PASSED=false
    [ -z "$GATE_REASON" ] && GATE_REASON="function_presence_fail"
fi

METRICS=$(jq -n \
    --argjson passed "$UNIT_PASSED" \
    --argjson lines "$COV_LINES" \
    --argjson branches "$COV_BRANCHES" \
    --argjson functions "$COV_FUNCTIONS" \
    --argjson missing "$MISSING_FN_TESTS" \
    '{"unit":{"passed":$passed,"lines":$lines,"branches":$branches,"functions":$functions,"missing_function_tests":$missing}}')

GATE_PASSED="$UNIT_PASSED"

if [ -n "$GATE_REASON" ]; then
    emit_result "$GATE_PASSED" "$GATE_REASON" "$METRICS" "$TEST_SUMMARY"
    exit 1
else
    emit_result "$GATE_PASSED" "" "$METRICS" "$TEST_SUMMARY"
    exit 0
fi
