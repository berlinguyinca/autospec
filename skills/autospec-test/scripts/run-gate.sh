#!/usr/bin/env bash
# run-gate.sh — full gate runner for autospec-test (Phase 9).
#
# Usage: run-gate.sh <target_dir> [--output-gate <gate_json_path>]
#                    [--output-comment <comment_md_path>]
#                    [--pr <PR_NUMBER>] [--repo <owner/repo>]
#                    [--dry-run]
#
# Runs the full autospec-test pipeline:
#   1. Load + validate contract (.autospec/test.yml)
#   2. Stage 1: unit gate (gate-stage-unit.sh)
#   3. Stage 2: E2E gate (gate-stage-e2e.sh) — if Stage 1 passes
#   4. Compose PR comment via pr-report.sh
#   5. Apply e2e:* labels via bootstrap-labels.sh + gh label
#
# For targets with .autospec/stub-gate.json checked in, emits that directly
# (used by integration golden-diff tests in Phase 8).
#
# Exit codes:
#   0 = gate passed (overall_passed=true)
#   1 = gate failed (overall_passed=false) — block PR
#   2 = fatal error (target_dir missing, bad contract, refuse-to-run) — halt batch

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

TARGET_DIR="${1:-}"
if [ -z "$TARGET_DIR" ] || [ ! -d "$TARGET_DIR" ]; then
    printf 'run-gate: fatal: target directory not found: %s\n' "$TARGET_DIR" >&2
    exit 2
fi

OUTPUT_GATE=""
OUTPUT_COMMENT=""
PR_NUMBER=""
REPO_FLAG=""
DRY_RUN=false
shift
while [ $# -gt 0 ]; do
    case "$1" in
        --output-gate)    OUTPUT_GATE="${2:-}";         shift 2 ;;
        --output-comment) OUTPUT_COMMENT="${2:-}";      shift 2 ;;
        --pr)             PR_NUMBER="${2:-}";            shift 2 ;;
        --repo)           REPO_FLAG="--repo ${2:-}";    shift 2 ;;
        --dry-run)        DRY_RUN=true;                 shift ;;
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

# ── Write gate JSON output ────────────────────────────────────────────────────

if [ -n "$OUTPUT_GATE" ]; then
    printf '%s\n' "$GATE_JSON" > "$OUTPUT_GATE"
else
    printf '%s\n' "$GATE_JSON"
fi

# ── Compose PR comment via pr-report.sh ───────────────────────────────────────

GATE_TMP=$(mktemp -t run-gate-json.XXXXXX)
trap 'rm -f "$GATE_TMP"' EXIT
printf '%s\n' "$GATE_JSON" > "$GATE_TMP"

if [ -n "$OUTPUT_COMMENT" ]; then
    bash "$SCRIPT_DIR/pr-report.sh" --gate-json "$GATE_TMP" --output "$OUTPUT_COMMENT"
elif [ -n "$PR_NUMBER" ] && [ "$DRY_RUN" = "false" ]; then
    bash "$SCRIPT_DIR/pr-report.sh" --gate-json "$GATE_TMP" --pr "$PR_NUMBER" $REPO_FLAG
fi

# ── Apply e2e:* labels (when --pr given and not dry-run) ─────────────────────

if [ -n "$PR_NUMBER" ] && [ "$DRY_RUN" = "false" ] && command -v gh >/dev/null 2>&1; then
    PASSED_FOR_LABELS=$(printf '%s' "$GATE_JSON" | jq -r '.overall_passed // false')
    S2_REASON=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.reason // ""')
    ASSERTION_SHIFT=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.assertion_shift // "none"')
    SCOPE_VIOLATION=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.scope_violation // false')
    RESTORE_SUCCEEDED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.restore_succeeded // true')

    # Bootstrap the label set idempotently first
    bash "$SCRIPT_DIR/bootstrap-labels.sh" $REPO_FLAG 2>/dev/null || true

    # Apply appropriate labels
    if [ "$PASSED_FOR_LABELS" = "true" ]; then
        gh pr edit "$PR_NUMBER" $REPO_FLAG --add-label "e2e:passed" 2>/dev/null || true
    else
        if [ "$S2_REASON" = "unjustified-shift" ]; then
            gh pr edit "$PR_NUMBER" $REPO_FLAG --add-label "e2e:unjustified-shift,needs-human-review" 2>/dev/null || true
        elif [ "$SCOPE_VIOLATION" = "true" ]; then
            gh pr edit "$PR_NUMBER" $REPO_FLAG --add-label "e2e:scope-violation,e2e:scoped-prod" 2>/dev/null || true
            if [ "$RESTORE_SUCCEEDED" = "true" ]; then
                gh pr edit "$PR_NUMBER" $REPO_FLAG --add-label "e2e:restored,needs-human-review" 2>/dev/null || true
            else
                gh pr edit "$PR_NUMBER" $REPO_FLAG --add-label "e2e:restore-failed,CRITICAL" 2>/dev/null || true
            fi
        else
            gh pr edit "$PR_NUMBER" $REPO_FLAG --add-label "e2e:blocked" 2>/dev/null || true
        fi
    fi
fi

# ── Exit code based on overall_passed ─────────────────────────────────────────

PASSED=$(printf '%s' "$GATE_JSON" | jq -r '.overall_passed // false')
if [ "$PASSED" = "true" ]; then
    exit 0
else
    exit 1
fi
