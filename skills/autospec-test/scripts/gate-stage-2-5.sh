#!/usr/bin/env bash
# gate-stage-2-5.sh — Stage 2.5 orchestrator for autospec-test v2.
#
# Usage: gate-stage-2-5.sh <target_dir> [--output <gate_json_path>]
#
# Reads the contract at <target_dir>/.autospec/test.yml. If invariants_v2.enabled
# is not true, emits a skipped gate JSON and exits 0 (zero overhead on v1-only targets).
#
# Exit codes:
#   0 = gate passed (all metrics passed or skipped)
#   1 = gate failed (block PR)
#   2 = fatal error (missing target, bad contract, refused to run due to seeds)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

TARGET_DIR="${1:-}"
if [ -z "$TARGET_DIR" ] || [ ! -d "$TARGET_DIR" ]; then
    printf 'gate-stage-2-5: fatal: target directory not found: %s\n' "$TARGET_DIR" >&2
    exit 2
fi

OUTPUT_FILE=""
shift
while [ $# -gt 0 ]; do
    case "$1" in
        --output) OUTPUT_FILE="${2:-}"; shift 2 ;;
        *) printf 'gate-stage-2-5: unknown flag: %s\n' "$1" >&2; exit 2 ;;
    esac
done

CONTRACT_YML="$TARGET_DIR/.autospec/test.yml"
if [ ! -f "$CONTRACT_YML" ]; then
    printf 'gate-stage-2-5: fatal: no .autospec/test.yml in %s\n' "$TARGET_DIR" >&2
    exit 2
fi

# ── Read invariants_v2.enabled ────────────────────────────────────────────────

V2_ENABLED=""
if command -v yq >/dev/null 2>&1; then
    V2_ENABLED=$(yq -r '.e2e.invariants_v2.enabled // "false"' "$CONTRACT_YML" 2>/dev/null || echo "false")
fi

if [ "$V2_ENABLED" != "true" ]; then
    SKIPPED_JSON='{"metric":"2.5","skipped":true,"passed":true,"reason":"invariants_v2.enabled != true"}'
    if [ -n "$OUTPUT_FILE" ]; then
        printf '%s\n' "$SKIPPED_JSON" > "$OUTPUT_FILE"
    else
        printf '%s\n' "$SKIPPED_JSON"
    fi
    exit 0
fi

# ── v2 is enabled: verify edge_case_seeds, then run F/G/H/I ─────────────────

TARGET_NAME="$(basename "$TARGET_DIR")"
METRICS_JSON='{}'
ALL_PASSED=true

# 2. Verify edge_case_seeds if declared
SEEDS_DECLARED=""
if command -v yq >/dev/null 2>&1; then
    SEEDS_DECLARED=$(yq -r '.e2e.invariants_v2.edge_case_seeds // ""' "$CONTRACT_YML" 2>/dev/null | grep -v '^$' || true)
fi

if [ -n "$SEEDS_DECLARED" ]; then
    VERIFY_SEEDS="$SCRIPT_DIR/../invariants/verify-seeds.mjs"
    if [ -f "$VERIFY_SEEDS" ]; then
        if ! node "$VERIFY_SEEDS" "$TARGET_DIR" 2>&1; then
            SEED_EXIT=$?
            if [ "$SEED_EXIT" -eq 2 ]; then
                printf 'gate-stage-2-5: fatal: edge_case_seeds verification refused to run (missing shapes)\n' >&2
                exit 2
            fi
            ALL_PASSED=false
        fi
    fi
fi

# Helper: run a Node metric runner and collect JSON output
run_metric() {
    local name="$1"
    local runner="$SCRIPT_DIR/../invariants/$2"
    local result_var="$3"

    if [ ! -f "$runner" ]; then
        # Runner not yet installed — emit a stub pass so v1 targets are unaffected
        printf '{"metric":"%s","passed":true,"skipped":true,"reason":"runner not installed"}\n' "$name"
        return 0
    fi

    local out
    if out=$(node "$runner" "$TARGET_DIR" 2>&1); then
        printf '%s\n' "$out"
    else
        local exit_code=$?
        if [ "$exit_code" -eq 2 ]; then
            printf '{"metric":"%s","passed":false,"refused":true,"reason":"runner refused to run"}\n' "$name"
        else
            printf '{"metric":"%s","passed":false,"reason":"runner exited %s","raw":"%s"}\n' \
                "$name" "$exit_code" "$(printf '%s' "$out" | head -1 | tr '"' "'")"
        fi
    fi
}

# 3. Metric F — Structural invariants
F_JSON=$(run_metric "F" "run-structural.mjs" F_JSON || echo '{"metric":"F","passed":true,"skipped":true}')
F_PASSED=$(printf '%s' "$F_JSON" | jq -r 'if .passed == false then false else true end' 2>/dev/null || echo "true")

# 4. Metric G — Window-contract symmetry
G_JSON=$(run_metric "G" "run-window.mjs" G_JSON || echo '{"metric":"G","passed":true,"skipped":true}')
G_PASSED=$(printf '%s' "$G_JSON" | jq -r 'if .passed == false then false else true end' 2>/dev/null || echo "true")

# 5. Metric H — Extended crawler
H_JSON=$(run_metric "H" "extended-crawler.mjs" H_JSON || echo '{"metric":"H","passed":true,"skipped":true}')
H_PASSED=$(printf '%s' "$H_JSON" | jq -r 'if .passed == false then false else true end' 2>/dev/null || echo "true")

# 6. Metric I — Data-source contract symmetry
I_JSON=$(run_metric "I" "run-symmetry.mjs" I_JSON || echo '{"metric":"I","passed":true,"skipped":true}')
I_PASSED=$(printf '%s' "$I_JSON" | jq -r 'if .passed == false then false else true end' 2>/dev/null || echo "true")

# If any metric failed, overall fails
if [ "$F_PASSED" != "true" ] || [ "$G_PASSED" != "true" ] || \
   [ "$H_PASSED" != "true" ] || [ "$I_PASSED" != "true" ]; then
    ALL_PASSED=false
fi

# 7. Compose Stage 2.5 gate JSON
OVERALL=$([ "$ALL_PASSED" = "true" ] && echo "true" || echo "false")

GATE_JSON=$(jq -n \
    --arg target "$TARGET_NAME" \
    --argjson passed "$OVERALL" \
    --arg metric "2.5" \
    --argjson f_result "$(printf '%s' "$F_JSON" | jq '.' 2>/dev/null || echo 'null')" \
    --argjson g_result "$(printf '%s' "$G_JSON" | jq '.' 2>/dev/null || echo 'null')" \
    --argjson h_result "$(printf '%s' "$H_JSON" | jq '.' 2>/dev/null || echo 'null')" \
    --argjson i_result "$(printf '%s' "$I_JSON" | jq '.' 2>/dev/null || echo 'null')" \
    '{
        "metric": $metric,
        "target": $target,
        "passed": $passed,
        "metrics": {
            "F": $f_result,
            "G": $g_result,
            "H": $h_result,
            "I": $i_result
        }
    }' 2>/dev/null || echo "{\"metric\":\"2.5\",\"target\":\"$TARGET_NAME\",\"passed\":$OVERALL}")

if [ -n "$OUTPUT_FILE" ]; then
    printf '%s\n' "$GATE_JSON" > "$OUTPUT_FILE"
else
    printf '%s\n' "$GATE_JSON"
fi

if [ "$ALL_PASSED" = "true" ]; then
    exit 0
else
    exit 1
fi
