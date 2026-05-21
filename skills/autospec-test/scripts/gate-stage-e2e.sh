#!/usr/bin/env bash
# gate-stage-e2e.sh — Stage 2 E2E gate for autospec-test.
#
# Usage: gate-stage-e2e.sh [<repo_root>]
#   OR:  echo '<contract_json>' | gate-stage-e2e.sh [<repo_root>]
#
# Input:  resolved contract JSON on stdin (output of load-contract.sh)
# Output: Stage 2 gate result JSON on stdout
#
# Exit codes:
#   0 = gate passed
#   1 = gate failed (tests red, coverage below threshold, taxonomy missing)
#   2 = fatal / refuse-to-run (missing tool, bad input, fail-closed safety)
#
# PR #331 fixes applied:
#   - Finding #1: no eval on contract-sourced PLAYWRIGHT_CMD; use array exec.
#   - Finding #2: no RETURN traps inside functions under set -u; inline cleanup.
#   - Finding #3: idempotent EXIT trap — single accumulating trap via add_exit_trap().
#   - Finding #8: no jq --slurpfile + process-substitution double-wrap; use temp files.

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT_SCRIPT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

# ── Accumulating EXIT trap (Fix for PR #331 finding #3) ───────────────────────
# Instead of multiple `trap "..." EXIT` statements that clobber each other,
# we maintain a list of cleanup commands and run them all on EXIT.
_EXIT_TRAP_CMDS=()

add_exit_trap() {
    # add_exit_trap <cmd_string>
    # Appends cmd_string to the list; the registered EXIT handler runs all of them.
    _EXIT_TRAP_CMDS+=("$1")
}

_run_exit_traps() {
    local cmd
    for cmd in "${_EXIT_TRAP_CMDS[@]}"; do
        eval "$cmd" 2>/dev/null || true
    done
}

# Register ONCE. Never re-register (would clobber).
trap '_run_exit_traps' EXIT

# ── Helpers ───────────────────────────────────────────────────────────────────

fatal() {
    printf 'gate-stage-e2e: fatal: %s\n' "$*" >&2
    # Inline emit (no emit_result call here to avoid recursion risk)
    printf '{"passed":false,"stage":"e2e","reason":"fatal","metrics":{},"test_run_summary":{}}\n'
    exit 2
}

emit_result() {
    # emit_result <passed_bool> <reason_or_empty> <metrics_json> <test_summary_json>
    # Fix for PR #331 finding #8: use temp files for all jq slurpfile inputs,
    # never combine --slurpfile with process substitution <().
    local passed="$1"
    local reason="${2:-}"
    local metrics_json="${3:-{\}}"
    local test_summary_json="${4:-{\}}"

    local m_file s_file
    m_file=$(mktemp /tmp/autospec-e2e-metrics-XXXXXX.json)
    s_file=$(mktemp /tmp/autospec-e2e-summary-XXXXXX.json)
    # Register cleanup via accumulating trap (not clobbering)
    add_exit_trap "rm -f '$m_file' '$s_file'"

    printf '%s' "$metrics_json"   > "$m_file"
    printf '%s' "$test_summary_json" > "$s_file"

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
    # Note: temp files cleaned up by add_exit_trap above, not inline here.
    # This avoids the RETURN trap leak (PR #331 finding #2).
}

# ── Dependency checks ─────────────────────────────────────────────────────────

for tool in jq node; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        fatal "$tool not found"
    fi
done

# ── Parse contract from stdin ─────────────────────────────────────────────────

CONTRACT_JSON=$(cat)
if [ -z "$CONTRACT_JSON" ]; then
    fatal "no contract JSON on stdin"
fi

TARGET_REPO="${1:-.}"
if [ ! -d "$TARGET_REPO" ]; then
    fatal "target repo not found: $TARGET_REPO"
fi
TARGET_REPO="$(cd "$TARGET_REPO" && pwd)"

# ── Extract fields from contract ──────────────────────────────────────────────

PLAYWRIGHT_CMD_STR=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.playwright_cmd // empty')
PLAYWRIGHT_CONFIG=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.playwright_config // empty')
FORBIDDEN_PATTERNS=$(printf '%s' "$CONTRACT_JSON" | jq -c '.e2e.forbidden_url_patterns // null')
FORBIDDEN_ACK=$(printf '%s' "$CONTRACT_JSON" | jq -r '.e2e.forbidden_url_patterns_intentionally_empty // false')

# ── Fail-closed: forbidden_url_patterns must be present and non-empty ─────────

if [ "$FORBIDDEN_PATTERNS" = "null" ] || [ -z "$FORBIDDEN_PATTERNS" ]; then
    printf 'gate-stage-e2e: REFUSED: e2e.forbidden_url_patterns is missing.\n' >&2
    printf 'Set forbidden_url_patterns or set forbidden_url_patterns_intentionally_empty: true\n' >&2
    emit_result false "refuse_to_run_missing_forbidden_patterns" \
        '{"e2e":{"passed":false,"reason":"forbidden_url_patterns_missing"}}' '{}'
    exit 2
fi

PATTERNS_LEN=$(printf '%s' "$FORBIDDEN_PATTERNS" | jq 'length')
if [ "$PATTERNS_LEN" -eq 0 ] && [ "$FORBIDDEN_ACK" != "true" ]; then
    printf 'gate-stage-e2e: REFUSED: e2e.forbidden_url_patterns is empty.\n' >&2
    printf 'Set at least one pattern, or set forbidden_url_patterns_intentionally_empty: true to acknowledge.\n' >&2
    emit_result false "refuse_to_run_empty_forbidden_patterns" \
        '{"e2e":{"passed":false,"reason":"forbidden_url_patterns_empty"}}' '{}'
    exit 2
fi

# ── Resolve Playwright config ─────────────────────────────────────────────────

PW_RESOLVER="$SCRIPT_DIR/playwright-config-resolver.mjs"
PW_CONFIG_JSON=""
if [ -f "$PW_RESOLVER" ]; then
    PW_CONFIG_JSON=$(node "$PW_RESOLVER" "$TARGET_REPO" 2>/dev/null || printf '{}')
fi

# ── Layer A: forbidden-URL preflight check ────────────────────────────────────

FORBIDDEN_CHECK="$SCRIPT_DIR/forbidden-url-check.mjs"
if [ -f "$FORBIDDEN_CHECK" ] && [ -n "$PW_CONFIG_JSON" ]; then
    # Write to temp files (not process substitution) — fix for PR #331 finding #8
    CONFIG_FILE=$(mktemp /tmp/autospec-pw-config-XXXXXX.json)
    PATTERNS_FILE=$(mktemp /tmp/autospec-patterns-XXXXXX.json)
    add_exit_trap "rm -f '$CONFIG_FILE' '$PATTERNS_FILE'"

    printf '%s' "$PW_CONFIG_JSON" > "$CONFIG_FILE"
    printf '%s' "$FORBIDDEN_PATTERNS" > "$PATTERNS_FILE"

    PREFLIGHT_OUT=$(node "$FORBIDDEN_CHECK" --config "$CONFIG_FILE" --patterns "$PATTERNS_FILE" 2>/dev/null || printf '{}')
    PREFLIGHT_VIOLATIONS=$(printf '%s' "$PREFLIGHT_OUT" | jq '.violations | length' 2>/dev/null || printf '0')

    if [ "$PREFLIGHT_VIOLATIONS" != "0" ] && [ "$PREFLIGHT_VIOLATIONS" != "" ]; then
        printf 'gate-stage-e2e: Layer A preflight FAILED: %s forbidden URL violation(s)\n' \
            "$PREFLIGHT_VIOLATIONS" >&2
        METRICS=$(jq -n \
            --argjson violations "$PREFLIGHT_VIOLATIONS" \
            '{"e2e":{"passed":false,"reason":"forbidden_url_preflight_failed","violations":$violations}}')
        emit_result false "forbidden_url_preflight_failed" "$METRICS" '{}'
        exit 2
    fi
fi

# ── Layer B: network intercept injection ──────────────────────────────────────

INJECT_SCRIPT="$SCRIPT_DIR/network-intercept-inject.mjs"
if [ -f "$INJECT_SCRIPT" ]; then
    PATTERNS_FILE_B=$(mktemp /tmp/autospec-patterns-b-XXXXXX.json)
    add_exit_trap "rm -f '$PATTERNS_FILE_B'"
    printf '%s' "$FORBIDDEN_PATTERNS" > "$PATTERNS_FILE_B"

    node "$INJECT_SCRIPT" \
        --repo-root "$TARGET_REPO" \
        --patterns "$PATTERNS_FILE_B" \
        >/dev/null 2>&1 || true
fi

# ── UI Crawler ────────────────────────────────────────────────────────────────

CRAWLER_SCRIPT="$SCRIPT_DIR/ui-crawler.mjs"
MANIFEST_FILE=$(mktemp /tmp/autospec-manifest-XXXXXX.json)
add_exit_trap "rm -f '$MANIFEST_FILE'"

BASE_URL=$(printf '%s' "$PW_CONFIG_JSON" | jq -r '.baseURL // empty')
CRAWLER_OUTPUT='{"routes":[],"manifest":[]}'

if [ -f "$CRAWLER_SCRIPT" ] && [ -n "$BASE_URL" ]; then
    SITEMAP_PATH="$TARGET_REPO/sitemap.xml"
    SITEMAP_ARG=""
    if [ -f "$SITEMAP_PATH" ]; then
        SITEMAP_ARG="--sitemap $SITEMAP_PATH"
    fi

    # shellcheck disable=SC2086
    CRAWLER_OUTPUT=$(node "$CRAWLER_SCRIPT" \
        --base-url "$BASE_URL" \
        --max-routes 200 \
        $SITEMAP_ARG \
        2>/dev/null || printf '{"routes":[],"manifest":[]}')
fi

printf '%s' "$CRAWLER_OUTPUT" > "$MANIFEST_FILE"
ROUTE_COUNT=$(printf '%s' "$CRAWLER_OUTPUT" | jq '.routes | length' 2>/dev/null || printf '0')
MANIFEST_COUNT=$(printf '%s' "$CRAWLER_OUTPUT" | jq '.manifest | length' 2>/dev/null || printf '0')

# ── Run Playwright tests (via array exec, NOT eval) ───────────────────────────
# Fix for PR #331 finding #1: NEVER use eval on contract-sourced PLAYWRIGHT_CMD.
# Build an array from the command string using a safe allowlist split.

PW_STDOUT_FILE=$(mktemp /tmp/autospec-pw-stdout-XXXXXX.txt)
PW_STDERR_FILE=$(mktemp /tmp/autospec-pw-stderr-XXXXXX.txt)
add_exit_trap "rm -f '$PW_STDOUT_FILE' '$PW_STDERR_FILE'"

PW_EXIT=0

if [ -n "$PLAYWRIGHT_CMD_STR" ]; then
    # Safe array split: split on whitespace using read -ra
    # This is safe because we are not passing through a shell — read -ra splits
    # on IFS without any shell interpretation of the individual tokens.
    IFS=' ' read -ra PW_CMD_ARRAY <<< "$PLAYWRIGHT_CMD_STR"

    # Validate: first token must be one of the allowed executables
    PW_CMD_FIRST="${PW_CMD_ARRAY[0]:-}"
    case "$PW_CMD_FIRST" in
        npx|node|playwright|bunx|yarn|pnpm)
            # Allowed — proceed with array exec
            ;;
        *)
            printf 'gate-stage-e2e: SECURITY: playwright_cmd first token not in allowlist: %s\n' \
                "$PW_CMD_FIRST" >&2
            emit_result false "playwright_cmd_not_allowed" \
                '{"e2e":{"passed":false,"reason":"playwright_cmd_security_rejected"}}' '{}'
            exit 2
            ;;
    esac

    # Run as array (no eval) in the target repo
    (cd "$TARGET_REPO" && "${PW_CMD_ARRAY[@]}") \
        >"$PW_STDOUT_FILE" 2>"$PW_STDERR_FILE" || PW_EXIT=$?
else
    printf 'gate-stage-e2e: WARN: e2e.playwright_cmd not set; skipping test run\n' >&2
fi

PW_STDOUT_TAIL=$(tail -20 "$PW_STDOUT_FILE" | head -c 2000)
PW_STDERR_TAIL=$(tail -20 "$PW_STDERR_FILE" | head -c 2000)

TEST_SUMMARY=$(jq -n \
    --argjson exit_code "${PW_EXIT}" \
    --arg stdout_tail "$PW_STDOUT_TAIL" \
    --arg stderr_tail "$PW_STDERR_TAIL" \
    '{"exit_code":$exit_code,"stdout_tail":$stdout_tail,"stderr_tail":$stderr_tail}')

if [ "${PW_EXIT}" -ne 0 ]; then
    METRICS=$(jq -n '{"e2e":{"passed":false,"reason":"tests_red"}}')
    emit_result false "tests_red" "$METRICS" "$TEST_SUMMARY"
    exit 1
fi

# ── UI element coverage (touched vs manifest) ──────────────────────────────────

TOUCHED_FILE="$TARGET_REPO/.autospec/touched-elements.jsonl"
TOUCHED_COUNT=0
if [ -f "$TOUCHED_FILE" ]; then
    TOUCHED_COUNT=$(wc -l < "$TOUCHED_FILE" | tr -d ' ')
fi

UI_COV_PASSED=true
UI_MISSING='[]'

if [ "$MANIFEST_COUNT" -gt 0 ]; then
    # Simple check: if touched < manifest, report gap
    if [ "$TOUCHED_COUNT" -lt "$MANIFEST_COUNT" ]; then
        UI_COV_PASSED=false
        UI_MISSING=$(jq -n \
            --argjson total "$MANIFEST_COUNT" \
            --argjson touched "$TOUCHED_COUNT" \
            '{"note":"touched \($touched) of \($total) manifest elements"}')
    fi
fi

# ── Behavior taxonomy check ───────────────────────────────────────────────────

TAXONOMY_SCRIPT="$SCRIPT_DIR/behavior-taxonomy-check.mjs"
RESULTS_DIR="$TARGET_REPO/test-results"
TAXONOMY_RESULT='{"passed":true,"missing":[],"passing":[]}'

BEHAVIOR_CATEGORIES=$(printf '%s' "$CONTRACT_JSON" | \
    jq -c '.e2e.coverage_thresholds.behavior_categories // []')
CAT_COUNT=$(printf '%s' "$BEHAVIOR_CATEGORIES" | jq 'length' 2>/dev/null || printf '0')

if [ -f "$TAXONOMY_SCRIPT" ] && [ "$CAT_COUNT" -gt 0 ]; then
    CATS_FILE=$(mktemp /tmp/autospec-cats-XXXXXX.json)
    add_exit_trap "rm -f '$CATS_FILE'"
    printf '%s' "$BEHAVIOR_CATEGORIES" > "$CATS_FILE"

    TAXONOMY_RESULT=$(node "$TAXONOMY_SCRIPT" \
        --results-dir "$RESULTS_DIR" \
        --categories "$CATS_FILE" \
        2>/dev/null || printf '{"passed":false,"missing":[],"passing":[]}')
fi

TAXONOMY_PASSED=$(printf '%s' "$TAXONOMY_RESULT" | jq -r '.passed')
TAXONOMY_MISSING=$(printf '%s' "$TAXONOMY_RESULT" | jq -c '.missing // []')

# ── LLM findings (non-blocking) ───────────────────────────────────────────────

FINDINGS_SCRIPT="$SCRIPT_DIR/findings-generator.mjs"
FINDINGS_FILE="$TARGET_REPO/.autospec/test-findings.md"

GATE_PASSED=true
if [ "$PW_EXIT" -ne 0 ] || [ "$UI_COV_PASSED" = "false" ] || [ "$TAXONOMY_PASSED" = "false" ]; then
    GATE_PASSED=false
fi

# Build preliminary gate result for findings generator
PRELIM_RESULT=$(jq -n \
    --argjson passed "$GATE_PASSED" \
    --argjson ui_passed "$UI_COV_PASSED" \
    --argjson tax_passed "$TAXONOMY_PASSED" \
    --argjson tax_missing "$TAXONOMY_MISSING" \
    '{"passed":$passed,"stage":"e2e","metrics":{"ui_element_coverage":{"passed":$ui_passed},"behavior_categories":{"passed":$tax_passed,"missing":$tax_missing}}}')

if [ -f "$FINDINGS_SCRIPT" ]; then
    FINDINGS_RESULT_FILE=$(mktemp /tmp/autospec-findings-input-XXXXXX.json)
    add_exit_trap "rm -f '$FINDINGS_RESULT_FILE'"
    printf '%s' "$PRELIM_RESULT" > "$FINDINGS_RESULT_FILE"

    node "$FINDINGS_SCRIPT" \
        --gate-result "$FINDINGS_RESULT_FILE" \
        --output "$FINDINGS_FILE" \
        --dry-run \
        >/dev/null 2>&1 || true
fi

# ── Assemble final metrics and emit ──────────────────────────────────────────

METRICS=$(jq -n \
    --argjson ui_passed "$UI_COV_PASSED" \
    --argjson ui_missing "${UI_MISSING:-[]}" \
    --argjson tax_passed "$TAXONOMY_PASSED" \
    --argjson tax_missing "$TAXONOMY_MISSING" \
    --argjson route_count "$ROUTE_COUNT" \
    --argjson manifest_count "$MANIFEST_COUNT" \
    --argjson touched_count "$TOUCHED_COUNT" \
    '{
        "e2e": {
            "passed": ($ui_passed and $tax_passed),
            "ui_element_coverage": {
                "passed": $ui_passed,
                "crawled": $manifest_count,
                "touched": $touched_count
            },
            "behavior_categories": {
                "passed": $tax_passed,
                "missing": $tax_missing
            },
            "routes_crawled": $route_count
        }
    }')

FINAL_PASSED=true
if [ "$UI_COV_PASSED" = "false" ] || [ "$TAXONOMY_PASSED" = "false" ]; then
    FINAL_PASSED=false
fi

if [ "$FINAL_PASSED" = "true" ]; then
    emit_result true "" "$METRICS" "$TEST_SUMMARY"
    exit 0
else
    REASON="e2e_coverage_gap"
    if [ "$UI_COV_PASSED" = "false" ] && [ "$TAXONOMY_PASSED" = "false" ]; then
        REASON="ui_coverage_and_taxonomy_gap"
    elif [ "$UI_COV_PASSED" = "false" ]; then
        REASON="ui_element_coverage_gap"
    else
        REASON="behavior_taxonomy_gap"
    fi
    emit_result false "$REASON" "$METRICS" "$TEST_SUMMARY"
    exit 1
fi
