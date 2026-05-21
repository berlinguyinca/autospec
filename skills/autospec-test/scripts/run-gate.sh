#!/usr/bin/env bash
# run-gate.sh — stub gate runner for autospec-test (Phase 8).
#
# Usage: run-gate.sh <target_dir> [--output-gate <gate_json_path>] [--output-comment <comment_md_path>]
#
# Phase 8: this is a stub that emits golden-shaped output from the target's
# embedded .autospec/golden/ dir (if present) for integration test golden-diff.
# Phase 9 will wire this to the real gate-stage-unit.sh + gate-stage-e2e.sh pipeline.
#
# Exit codes:
#   0 = gate passed (overall_passed=true in output JSON)
#   1 = gate failed (overall_passed=false)
#   2 = fatal error (target_dir missing, bad contract, etc.)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

TARGET_DIR="${1:-}"
if [ -z "$TARGET_DIR" ] || [ ! -d "$TARGET_DIR" ]; then
    printf 'run-gate: fatal: target directory not found: %s\n' "$TARGET_DIR" >&2
    exit 2
fi

OUTPUT_GATE=""
OUTPUT_COMMENT=""
shift
while [ $# -gt 0 ]; do
    case "$1" in
        --output-gate)    OUTPUT_GATE="${2:-}";    shift 2 ;;
        --output-comment) OUTPUT_COMMENT="${2:-}"; shift 2 ;;
        *) printf 'run-gate: unknown flag: %s\n' "$1" >&2; exit 2 ;;
    esac
done

# ── Load contract ──────────────────────────────────────────────────────────────

CONTRACT_YML="$TARGET_DIR/.autospec/test.yml"
if [ ! -f "$CONTRACT_YML" ]; then
    printf 'run-gate: fatal: no .autospec/test.yml in %s\n' "$TARGET_DIR" >&2
    exit 2
fi

# ── Phase 8 stub: run real unit tests, emit structured gate JSON ──────────────
# For targets that have a stub gate JSON checked in under .autospec/stub-gate.json,
# emit it directly (used by integration golden-diff tests).
# Otherwise, run the real gate stages (wired in Phase 9).

STUB_GATE="$TARGET_DIR/.autospec/stub-gate.json"
STUB_COMMENT="$TARGET_DIR/.autospec/stub-pr-comment.md"

TARGET_NAME="$(basename "$TARGET_DIR")"
GATE_JSON=""
COMMENT_MD=""

if [ -f "$STUB_GATE" ]; then
    GATE_JSON="$(cat "$STUB_GATE")"
    COMMENT_MD="$(cat "$STUB_COMMENT" 2>/dev/null || echo '<!-- autospec-test-report-marker -->')"
else
    # No stub: run real Stage 1 (unit tests) via gate-stage-unit.sh
    # Stage 2 wiring deferred to Phase 9.
    CONTRACT_JSON=""
    if command -v yq >/dev/null 2>&1; then
        CONTRACT_JSON="$(yq -o=json '.' "$CONTRACT_YML" 2>/dev/null || echo '{}')"
    else
        CONTRACT_JSON="{}"
    fi

    STAGE1_RESULT=""
    if STAGE1_RESULT=$(printf '%s\n' "$CONTRACT_JSON" | bash "$SCRIPT_DIR/gate-stage-unit.sh" "$TARGET_DIR" 2>/dev/null); then
        S1_PASSED=$(printf '%s' "$STAGE1_RESULT" | jq -r '.passed // false')
    else
        S1_PASSED=false
        STAGE1_RESULT='{"passed":false,"stage":"unit","reason":"gate-stage-unit failed"}'
    fi

    OVERALL=$([ "$S1_PASSED" = "true" ] && echo "true" || echo "false")
    GATE_JSON=$(jq -n \
        --arg target "$TARGET_NAME" \
        --argjson stage1 "$STAGE1_RESULT" \
        --argjson overall "$OVERALL" \
        '{"target":$target,"stage1":$stage1,"overall_passed":$overall}')
    COMMENT_MD="<!-- autospec-test-report-marker -->
## autospec-test — $([ "$OVERALL" = "true" ] && echo "✅ Passed" || echo "❌ Blocked")

**Mode:** strict-isolation
**Stage 1 (unit):** $([ "$S1_PASSED" = "true" ] && echo "passed" || echo "failed")
"
fi

# ── Write outputs ──────────────────────────────────────────────────────────────

if [ -n "$OUTPUT_GATE" ]; then
    printf '%s\n' "$GATE_JSON" > "$OUTPUT_GATE"
else
    printf '%s\n' "$GATE_JSON"
fi

if [ -n "$OUTPUT_COMMENT" ]; then
    printf '%s\n' "$COMMENT_MD" > "$OUTPUT_COMMENT"
fi

# ── Exit code based on overall_passed ─────────────────────────────────────────

PASSED=$(printf '%s' "$GATE_JSON" | jq -r '.overall_passed // false')
if [ "$PASSED" = "true" ]; then
    exit 0
else
    exit 1
fi
