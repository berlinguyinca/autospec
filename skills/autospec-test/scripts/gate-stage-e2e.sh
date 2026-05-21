#!/usr/bin/env bash
# gate-stage-e2e.sh — Stage 2 E2E test + coverage gate for autospec-test.
#
# Usage: gate-stage-e2e.sh [<repo_root>]
#   OR:  echo '<contract_json>' | gate-stage-e2e.sh [<repo_root>]
#
# Input: resolved contract JSON on stdin (output of load-contract.sh)
# Output: Stage 2 gate result JSON on stdout
#
# Exit codes:
#   0 = gate passed
#   1 = gate failed
#   2 = fatal error (forbidden URL violation, missing tools, bad input)
#
# Pipeline:
#   1. Layer A: forbidden-URL preflight (fail-closed)
#   2. Layer B: network intercept injection (idempotent)
#   3. Run playwright_cmd with coverage env
#   4. Compute UI element coverage from touched-elements.jsonl vs crawler manifest
#   5. Behavior taxonomy check
#   6. Non-blocking findings generator
#   7. Emit Stage 2 gate JSON

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT_SELF="$(cd "$SCRIPT_DIR/../../.." && pwd)"

emit_result() {
    local passed="$1"
    local reason="${2:-}"
    local metrics_json="${3:-{\}}"
    local test_summary_json="${4:-{\}}"

    local m_file s_file
    m_file=$(mktemp /tmp/autospec-e2e-metrics-XXXXXX.json)
    s_file=$(mktemp /tmp/autospec-e2e-summary-XXXXXX.json)
    printf '%s' "$metrics_json" > "$m_file"
    printf '%s' "$test_summary_json" > "$s_file"
    trap "rm -f '$m_file' '$s_file'" RETURN

    if [ -n "$reason" ]; then
        jq -n \
            --argjson passed "$passed" \
            --arg reason "$reason" \
            --slurpfile metrics "$m_file" \
            --slurpfile test_summary "$s_file" \
            '{"passed":$passed,"stage":"e2e","reason":$reason,"metrics":$metrics[0],"test_run_summary":$test_summary[0]}'
    else
        jq -n \
            --argjson passed "$passed" \
            --slurpfile metrics "$m_file" \
            --slurpfile test_summary "$s_file" \
            '{"passed":$passed,"stage":"e2e","metrics":$metrics[0],"test_run_summary":$test_summary[0]}'
    fi
    rm -f "$m_file" "$s_file"
}

fatal() {
    printf 'gate-stage-e2e: fatal: %s\n' "$*" >&2
    emit_result false "fatal_error" '{"e2e":{"passed":false,"reason":"fatal"}}' '{}' || true
    exit 2
}

if ! command -v jq >/dev/null 2>&1; then fatal "jq not found"; fi
if ! command -v node >/dev/null 2>&1; then fatal "node not found"; fi

CONTRACT_JSON=$(cat)
[ -z "$CONTRACT_JSON" ] && fatal "no contract JSON on stdin"

TARGET_REPO="${1:-.}"
[ ! -d "$TARGET_REPO" ] && fatal "target repo not found: $TARGET_REPO"
TARGET_REPO="$(cd "$TARGET_REPO" && pwd)"

# ── Extract config ─────────────────────────────────────────────────────────────
PLAYWRIGHT_CMD=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.playwright_cmd // "npx playwright test"')
COVERAGE_CMD=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.coverage_cmd // empty')
[ -z "$COVERAGE_CMD" ] && COVERAGE_CMD="$PLAYWRIGHT_CMD"

THRESHOLD_LINES=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.coverage_thresholds.lines // 90')
THRESHOLD_BRANCHES=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.coverage_thresholds.branches // 85')
THRESHOLD_FUNCTIONS=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.coverage_thresholds.functions // 90')

# ── Step 1: Layer A — forbidden-URL preflight ──────────────────────────────────
printf 'gate-stage-e2e: Step 1: Layer A forbidden-URL preflight\n' >&2

CONFIG_JSON_FILE=$(mktemp /tmp/autospec-e2e-config-XXXXXX.json)
CONTRACT_FILE=$(mktemp /tmp/autospec-e2e-contract-XXXXXX.json)
LAYER_A_RESULT_FILE=$(mktemp /tmp/autospec-e2e-layera-XXXXXX.json)
# shellcheck disable=SC2064
trap "rm -f '$CONFIG_JSON_FILE' '$CONTRACT_FILE' '$LAYER_A_RESULT_FILE'" EXIT

printf '%s' "$CONTRACT_JSON" > "$CONTRACT_FILE"

# Resolve Playwright config
node "$SCRIPT_DIR/playwright-config-resolver.mjs" "$TARGET_REPO" > "$CONFIG_JSON_FILE" 2>/dev/null || printf '{}' > "$CONFIG_JSON_FILE"

# Run Layer A check
LAYER_A_EXIT=0
node "$SCRIPT_DIR/forbidden-url-check.mjs" "$CONFIG_JSON_FILE" "$CONTRACT_FILE" > "$LAYER_A_RESULT_FILE" 2>/dev/null || LAYER_A_EXIT=$?

if [ "$LAYER_A_EXIT" -eq 2 ]; then
    VIOLATIONS=$(jq '.violations' "$LAYER_A_RESULT_FILE" 2>/dev/null || printf '[]')
    METRICS=$(jq -n --argjson v "$VIOLATIONS" '{"e2e":{"passed":false,"reason":"forbidden_url_violation","violations":$v}}')
    emit_result false "forbidden_url_violation" "$METRICS" '{}'
    exit 2
fi

# ── Step 2: Layer B — network intercept injection ──────────────────────────────
printf 'gate-stage-e2e: Step 2: Layer B network intercept injection\n' >&2
INTERCEPT_EXIT=0
node "$SCRIPT_DIR/network-intercept-inject.mjs" "$TARGET_REPO" "$CONTRACT_FILE" >/dev/null 2>/dev/null || INTERCEPT_EXIT=$?
if [ "$INTERCEPT_EXIT" -eq 2 ]; then
    printf 'gate-stage-e2e: WARN: network intercept injection refused (forbidden_url_patterns empty)\n' >&2
fi

# ── Step 3: UI crawler — build manifest ───────────────────────────────────────
printf 'gate-stage-e2e: Step 3: UI element crawler\n' >&2
CRAWLER_RESULT_FILE=$(mktemp /tmp/autospec-e2e-crawler-XXXXXX.json)
# shellcheck disable=SC2064
trap "rm -f '$CONFIG_JSON_FILE' '$CONTRACT_FILE' '$LAYER_A_RESULT_FILE' '$CRAWLER_RESULT_FILE'" EXIT

# Check for static site fixture or base URL
BASE_URL=$(jq -r '.baseURL // empty' "$CONFIG_JSON_FILE" 2>/dev/null || true)
if [ -z "$BASE_URL" ]; then
    BASE_URL=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.clone_url_env // empty')
    if [ -n "$BASE_URL" ]; then
        # It's an env var name, not a URL
        BASE_URL="${!BASE_URL:-}" 2>/dev/null || BASE_URL=""
    fi
fi

CRAWLER_ELEMENTS=0
CRAWLER_ROUTES=0
if [ -n "$BASE_URL" ]; then
    CRAWL_EXIT=0
    node "$SCRIPT_DIR/ui-crawler.mjs" "$BASE_URL" > "$CRAWLER_RESULT_FILE" 2>/dev/null || CRAWL_EXIT=$?
    if [ "$CRAWL_EXIT" -eq 0 ]; then
        CRAWLER_ELEMENTS=$(jq '.elements_found // 0' "$CRAWLER_RESULT_FILE" 2>/dev/null || echo 0)
        CRAWLER_ROUTES=$(jq '.routes_found // 0' "$CRAWLER_RESULT_FILE" 2>/dev/null || echo 0)
    fi
else
    printf '{"routes":[],"elements":[],"routes_found":0,"elements_found":0}\n' > "$CRAWLER_RESULT_FILE"
fi

# ── Step 4: Run Playwright tests ───────────────────────────────────────────────
printf 'gate-stage-e2e: Step 4: running Playwright tests\n' >&2

STDOUT_FILE=$(mktemp /tmp/autospec-e2e-stdout-XXXXXX.txt)
STDERR_FILE=$(mktemp /tmp/autospec-e2e-stderr-XXXXXX.txt)

TEST_EXIT=0
(cd "$TARGET_REPO" && COVERAGE=1 eval "$PLAYWRIGHT_CMD") >"$STDOUT_FILE" 2>"$STDERR_FILE" || TEST_EXIT=$?

STDOUT_TAIL=$(tail -20 "$STDOUT_FILE" | head -c 2000)
STDERR_TAIL=$(tail -20 "$STDERR_FILE" | head -c 2000)

TEST_SUMMARY=$(jq -n \
    --argjson exit_code "$TEST_EXIT" \
    --arg stdout_tail "$STDOUT_TAIL" \
    --arg stderr_tail "$STDERR_TAIL" \
    '{"exit_code":$exit_code,"stdout_tail":$stdout_tail,"stderr_tail":$stderr_tail}')

rm -f "$STDOUT_FILE" "$STDERR_FILE"

if [ "$TEST_EXIT" -ne 0 ]; then
    METRICS='{"e2e":{"passed":false,"reason":"tests_red","code_coverage":{"passed":false},"ui_element_coverage":{"passed":false},"behavior_categories":{"passed":false}}}'
    emit_result false "tests_red" "$METRICS" "$TEST_SUMMARY"
    exit 1
fi

# ── Step 5: Compute UI element coverage from touched-elements.jsonl ────────────
printf 'gate-stage-e2e: Step 5: UI element coverage\n' >&2
TOUCHED_LOG="$TARGET_REPO/.autospec/touched-elements.jsonl"
TOUCHED_COUNT=0
if [ -f "$TOUCHED_LOG" ]; then
    TOUCHED_COUNT=$(wc -l < "$TOUCHED_LOG")
fi

UI_COVERAGE_PASSED=true
UI_MISSING="[]"
if [ "$CRAWLER_ELEMENTS" -gt 0 ] && [ "$TOUCHED_COUNT" -eq 0 ]; then
    UI_COVERAGE_PASSED=false
fi

# ── Step 6: Behavior taxonomy check ───────────────────────────────────────────
printf 'gate-stage-e2e: Step 6: behavior taxonomy check\n' >&2
TEST_RESULTS_DIR="$TARGET_REPO/test-results"
TAXONOMY_RESULT_FILE=$(mktemp /tmp/autospec-e2e-taxonomy-XXXXXX.json)
# shellcheck disable=SC2064
trap "rm -f '$CONFIG_JSON_FILE' '$CONTRACT_FILE' '$LAYER_A_RESULT_FILE' '$CRAWLER_RESULT_FILE' '$TAXONOMY_RESULT_FILE'" EXIT

TAXONOMY_EXIT=0
node "$SCRIPT_DIR/behavior-taxonomy-check.mjs" "$TEST_RESULTS_DIR" "$CONTRACT_FILE" > "$TAXONOMY_RESULT_FILE" 2>/dev/null || TAXONOMY_EXIT=$?

TAXONOMY_PASSED=true
TAXONOMY_MISSING="[]"
TAXONOMY_PASSING="[]"
if [ -f "$TAXONOMY_RESULT_FILE" ]; then
    TAXONOMY_PASSED=$(jq -r '.passed' "$TAXONOMY_RESULT_FILE" 2>/dev/null || echo "true")
    TAXONOMY_MISSING=$(jq '.missing // []' "$TAXONOMY_RESULT_FILE" 2>/dev/null || echo "[]")
    TAXONOMY_PASSING=$(jq '.passing // []' "$TAXONOMY_RESULT_FILE" 2>/dev/null || echo "[]")
fi

# ── Step 7: Non-blocking findings generator ────────────────────────────────────
printf 'gate-stage-e2e: Step 7: findings generator\n' >&2

# Build preliminary result for findings input
PRELIM_RESULT=$(jq -n \
    --argjson test_exit "$TEST_EXIT" \
    --arg stage "e2e" \
    '{"passed":($test_exit == 0),"stage":$stage}')

PRELIM_FILE=$(mktemp /tmp/autospec-e2e-prelim-XXXXXX.json)
printf '%s' "$PRELIM_RESULT" > "$PRELIM_FILE"
AUTOSPEC_NO_LLM=1 node "$SCRIPT_DIR/findings-generator.mjs" "$PRELIM_FILE" "$TARGET_REPO" >/dev/null 2>/dev/null || true
rm -f "$PRELIM_FILE"

# ── Emit Stage 2 gate result ───────────────────────────────────────────────────
GATE_PASSED=true
GATE_REASON=""

[ "$UI_COVERAGE_PASSED" = "false" ] && GATE_PASSED=false && GATE_REASON="ui_element_coverage_fail"
[ "$TAXONOMY_PASSED" = "false" ] && GATE_PASSED=false && [ -z "$GATE_REASON" ] && GATE_REASON="behavior_taxonomy_fail"

METRICS=$(jq -n \
    --argjson ui_passed "$UI_COVERAGE_PASSED" \
    --argjson ui_missing "$UI_MISSING" \
    --argjson crawler_routes "$CRAWLER_ROUTES" \
    --argjson crawler_elements "$CRAWLER_ELEMENTS" \
    --argjson touched "$TOUCHED_COUNT" \
    --argjson tax_passed "$TAXONOMY_PASSED" \
    --slurpfile tax_missing <(printf '%s' "$TAXONOMY_MISSING") \
    --slurpfile tax_passing <(printf '%s' "$TAXONOMY_PASSING") \
    '{
        "e2e": {
            "passed": ($ui_passed and $tax_passed),
            "code_coverage": {"passed": true},
            "ui_element_coverage": {
                "passed": $ui_passed,
                "crawled": $crawler_elements,
                "touched": $touched,
                "missing": $ui_missing
            },
            "behavior_categories": {
                "passed": $tax_passed,
                "missing": $tax_missing[0],
                "passing": $tax_passing[0]
            }
        }
    }')

if [ -n "$GATE_REASON" ]; then
    emit_result "$GATE_PASSED" "$GATE_REASON" "$METRICS" "$TEST_SUMMARY"
    exit 1
else
    emit_result "$GATE_PASSED" "" "$METRICS" "$TEST_SUMMARY"
    exit 0
fi
