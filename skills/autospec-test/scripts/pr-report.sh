#!/usr/bin/env bash
# pr-report.sh — compose and post the marker-replaced PR comment for autospec-test.
#
# Usage:
#   pr-report.sh --gate-json <gate.json> [--output <file>] [--pr <PR_NUMBER>] [--repo <REPO>]
#
# Reads the gate JSON produced by run-gate.sh and composes a markdown comment
# matching spec §7c, delimited by <!-- autospec-test-report-marker -->.
#
# When --pr is given: posts/edits the comment on the GitHub PR via gh CLI.
#   - If a prior comment with the marker exists: edits it (marker-replace).
#   - If no prior comment: posts a new one.
# When --output is given: writes the markdown to a file (for testing/dry-run).
# When neither: prints to stdout.
#
# Exit codes:
#   0 = success
#   1 = error (missing gate JSON, bad JSON, gh CLI failure)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Argument parsing ───────────────────────────────────────────────────────────

GATE_JSON_FILE=""
OUTPUT_FILE=""
PR_NUMBER=""
REPO=""

while [ $# -gt 0 ]; do
    case "$1" in
        --gate-json) GATE_JSON_FILE="${2:-}"; shift 2 ;;
        --output)    OUTPUT_FILE="${2:-}";    shift 2 ;;
        --pr)        PR_NUMBER="${2:-}";      shift 2 ;;
        --repo)      REPO="${2:-}";           shift 2 ;;
        *) printf 'pr-report: unknown flag: %s\n' "$1" >&2; exit 1 ;;
    esac
done

if [ -z "$GATE_JSON_FILE" ]; then
    printf 'pr-report: --gate-json is required\n' >&2
    exit 1
fi

if [ ! -f "$GATE_JSON_FILE" ]; then
    printf 'pr-report: gate JSON file not found: %s\n' "$GATE_JSON_FILE" >&2
    exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
    printf 'pr-report: jq not found\n' >&2
    exit 1
fi

# ── Parse gate JSON ────────────────────────────────────────────────────────────

GATE_JSON="$(cat "$GATE_JSON_FILE")"

OVERALL_PASSED=$(printf '%s' "$GATE_JSON" | jq -r '.overall_passed // false')
TARGET=$(printf '%s' "$GATE_JSON" | jq -r '.target // "unknown"')

# Stage 1
S1_PASSED=$(printf '%s' "$GATE_JSON" | jq -r '.stage1.passed // false')
S1_REASON=$(printf '%s' "$GATE_JSON" | jq -r '.stage1.reason // ""')
S1_LINES=$(printf '%s' "$GATE_JSON" | jq -r '.stage1.metrics.lines // "n/a"')
S1_BRANCHES=$(printf '%s' "$GATE_JSON" | jq -r '.stage1.metrics.branches // "n/a"')
S1_FUNCTIONS=$(printf '%s' "$GATE_JSON" | jq -r '.stage1.metrics.functions // "n/a"')
S1_TOTAL=$(printf '%s' "$GATE_JSON" | jq -r '.stage1.test_run_summary.total // 0')
S1_PASSED_COUNT=$(printf '%s' "$GATE_JSON" | jq -r '.stage1.test_run_summary.passed // 0')
S1_FAILED_COUNT=$(printf '%s' "$GATE_JSON" | jq -r '.stage1.test_run_summary.failed // 0')

# Stage 2
S2_PRESENT=$(printf '%s' "$GATE_JSON" | jq 'has("stage2")')
S2_PASSED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.passed // false')
S2_REASON=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.reason // ""')
S2_ASSERTION_SHIFT=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.assertion_shift // "none"')
S2_SCOPE_VIOLATION=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.scope_violation // false')
S2_RESTORE_INVOKED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.restore_invoked // false')
S2_RESTORE_SUCCEEDED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.restore_succeeded // false')
S2_MISSING_CATS=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.missing_behavior_categories // [] | join(", ")')
S2_MISSING_ELEMS=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.missing_ui_elements // [] | join(", ")')
S2_COVERED_CATS=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.behavior_categories_covered // [] | join(", ")')

# Stage 2.5
S25_PRESENT=$(printf '%s' "$GATE_JSON" | jq 'has("stage2_5")')
S25_PASSED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.passed // false')
S25_SKIPPED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.skipped // false')
S25_F_PASSED=$(printf '%s' "$GATE_JSON" | jq -r 'if .stage2_5.metrics.F.passed == false then false else true end')
S25_F_SKIPPED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.F.skipped // false')
S25_G_PASSED=$(printf '%s' "$GATE_JSON" | jq -r 'if .stage2_5.metrics.G.passed == false then false else true end')
S25_G_SKIPPED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.G.skipped // false')
S25_H_PASSED=$(printf '%s' "$GATE_JSON" | jq -r 'if .stage2_5.metrics.H.passed == false then false else true end')
S25_H_SKIPPED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.H.skipped // false')
S25_I_PASSED=$(printf '%s' "$GATE_JSON" | jq -r 'if .stage2_5.metrics.I.passed == false then false else true end')
S25_I_SKIPPED=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.I.skipped // false')
S25_SEEDS_OK=$(printf '%s' "$GATE_JSON" | jq -r 'if .stage2_5.seeds_ok == false then false else true end')
S25_SEEDS_COUNT=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.seeds_count // 0')

# ── Determine mode ─────────────────────────────────────────────────────────────

MODE="strict-isolation"
if printf '%s' "$GATE_JSON" | jq -e '.stage2.metrics.scope_violation != null' >/dev/null 2>&1; then
    MODE="scoped-production"
fi

# ── Compose markdown comment ───────────────────────────────────────────────────

if [ "$OVERALL_PASSED" = "true" ]; then
    STATUS_ICON="✅ Passed"
else
    STATUS_ICON="❌ Blocked"
fi

S1_STATUS=$([ "$S1_PASSED" = "true" ] && echo "passed" || echo "failed")
S2_STATUS=$([ "$S2_PASSED" = "true" ] && echo "passed" || echo "failed")

COMMENT="<!-- autospec-test-report-marker -->
## autospec-test — $STATUS_ICON

**Mode:** $MODE
**Stage 1 (unit):** $S1_STATUS"

if [ "$S2_PRESENT" = "true" ]; then
    COMMENT="$COMMENT
**Stage 2 (E2E):** $S2_STATUS"
fi

if [ "$S25_PRESENT" = "true" ] && [ "$S25_SKIPPED" != "true" ]; then
    S25_STATUS=$([ "$S25_PASSED" = "true" ] && echo "passed" || echo "failed")
    COMMENT="$COMMENT
**Stage 2.5 (Invariants):** $S25_STATUS"
fi

# Coverage
if [ "$S1_LINES" != "n/a" ]; then
    COMMENT="$COMMENT

### Coverage
- Lines: ${S1_LINES}%
- Branches: ${S1_BRANCHES}%
- Functions: ${S1_FUNCTIONS}%"
fi

# Why blocked section
if [ "$OVERALL_PASSED" = "false" ]; then
    COMMENT="$COMMENT

### Why blocked"
    if [ -n "$S1_REASON" ] && [ "$S1_PASSED" = "false" ]; then
        COMMENT="$COMMENT
- Stage 1: $S1_REASON"
    fi
    if [ -n "$S2_REASON" ] && [ "$S2_PASSED" = "false" ]; then
        COMMENT="$COMMENT
- Stage 2: $S2_REASON"
    fi
    if [ -n "$S2_MISSING_CATS" ]; then
        COMMENT="$COMMENT
- missing_behavior_categories: $S2_MISSING_CATS"
    fi
    if [ -n "$S2_MISSING_ELEMS" ]; then
        COMMENT="$COMMENT

### Untouched UI elements
| Selector |
|---|"
        for elem in $(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.missing_ui_elements[]? // empty'); do
            COMMENT="$COMMENT
| $elem |"
        done
    fi
    if [ "$S2_ASSERTION_SHIFT" != "none" ] && [ -n "$S2_ASSERTION_SHIFT" ]; then
        SHIFT_DETAILS=$(printf '%s' "$GATE_JSON" | jq -r '.stage2.metrics.assertion_shift_details // ""')
        COMMENT="$COMMENT
- assertion_shift: $S2_ASSERTION_SHIFT — $SHIFT_DETAILS"
    fi
    if [ "$S2_SCOPE_VIOLATION" = "true" ]; then
        RESTORE_STATUS=$([ "$S2_RESTORE_SUCCEEDED" = "true" ] && echo "✅ succeeded" || echo "❌ failed")
        COMMENT="$COMMENT
- e2e:scope-violation — test mutated an out-of-scope row
- Restore invoked: $RESTORE_STATUS"
    fi
fi

# Behavior categories (for passing or partial info)
if [ -n "$S2_COVERED_CATS" ]; then
    COMMENT="$COMMENT

### Behavior categories covered
$S2_COVERED_CATS"
fi

# Stage 2.5 subsection (spec §6a) — appended after Stage 2 section
if [ "$S25_PRESENT" = "true" ] && [ "$S25_SKIPPED" != "true" ]; then
    COMMENT="$COMMENT

### Stage 2.5 — Invariants & contracts"

    # Metric F — Structural invariants
    if [ "$S25_F_SKIPPED" = "true" ]; then
        COMMENT="$COMMENT
**Metric F — Structural invariants:** skipped (runner not installed)"
    else
        F_ICON=$([ "$S25_F_PASSED" = "true" ] && echo "✅" || echo "❌")
        F_REASON=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.F.reason // ""')
        COMMENT="$COMMENT
**Metric F — Structural invariants:** $F_ICON"
        if [ -n "$F_REASON" ] && [ "$S25_F_PASSED" != "true" ]; then
            COMMENT="$COMMENT
- $F_REASON"
        fi
        while IFS= read -r detail; do
            [ -n "$detail" ] && COMMENT="$COMMENT
- ❌ $detail"
        done < <(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.F.details[]? // empty' 2>/dev/null)
    fi

    # Metric G — Window-contract symmetry
    if [ "$S25_G_SKIPPED" = "true" ]; then
        COMMENT="$COMMENT
**Metric G — Window contracts:** skipped (runner not installed)"
    else
        G_ICON=$([ "$S25_G_PASSED" = "true" ] && echo "✅" || echo "❌")
        G_REASON=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.G.reason // ""')
        COMMENT="$COMMENT
**Metric G — Window contracts:** $G_ICON"
        if [ -n "$G_REASON" ] && [ "$S25_G_PASSED" != "true" ]; then
            COMMENT="$COMMENT
- $G_REASON"
        fi
        while IFS= read -r detail; do
            [ -n "$detail" ] && COMMENT="$COMMENT
- ❌ $detail"
        done < <(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.G.details[]? // empty' 2>/dev/null)
    fi

    # Metric H — Extended crawler
    if [ "$S25_H_SKIPPED" = "true" ]; then
        COMMENT="$COMMENT
**Metric H — Extended crawler:** skipped (runner not installed)"
    else
        H_ICON=$([ "$S25_H_PASSED" = "true" ] && echo "✅" || echo "❌")
        H_UNAFFORDABLE=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.H.unaffordable_count // 0')
        H_THRESHOLD=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.H.threshold // 0')
        COMMENT="$COMMENT
**Metric H — Extended crawler:** $H_ICON"
        if [ "$S25_H_PASSED" != "true" ] && [ "$H_UNAFFORDABLE" != "0" ]; then
            COMMENT="$COMMENT ($H_UNAFFORDABLE unaffordable elements, threshold: $H_THRESHOLD)"
        fi
        while IFS= read -r detail; do
            [ -n "$detail" ] && COMMENT="$COMMENT
- ❌ $detail"
        done < <(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.H.details[]? // empty' 2>/dev/null)
    fi

    # Metric I — Contract symmetry
    if [ "$S25_I_SKIPPED" = "true" ]; then
        COMMENT="$COMMENT
**Metric I — Contract symmetry:** skipped (runner not installed)"
    else
        I_ICON=$([ "$S25_I_PASSED" = "true" ] && echo "✅" || echo "❌")
        I_PASSED_COUNT=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.I.passed_count // "?"')
        I_TOTAL_COUNT=$(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.I.total_count // "?"')
        COMMENT="$COMMENT
**Metric I — Contract symmetry:** $I_ICON $I_PASSED_COUNT/$I_TOTAL_COUNT passed"
        while IFS= read -r detail; do
            [ -n "$detail" ] && COMMENT="$COMMENT
- ❌ $detail"
        done < <(printf '%s' "$GATE_JSON" | jq -r '.stage2_5.metrics.I.details[]? // empty' 2>/dev/null)
    fi

    # Edge-case seeds handshake
    if [ "$S25_SEEDS_COUNT" != "0" ] && [ "$S25_SEEDS_COUNT" != "null" ]; then
        SEEDS_ICON=$([ "$S25_SEEDS_OK" = "true" ] && echo "✅" || echo "❌")
        SEEDS_LABEL=$([ "$S25_SEEDS_OK" = "true" ] && echo "all $S25_SEEDS_COUNT shapes present" || echo "missing shapes — re-run Skill C provisioning")
        COMMENT="$COMMENT

**Edge-case seeds (Skill C handshake):** $SEEDS_ICON $SEEDS_LABEL"
    fi
fi

# ── Write or post ──────────────────────────────────────────────────────────────

if [ -n "$OUTPUT_FILE" ]; then
    printf '%s\n' "$COMMENT" > "$OUTPUT_FILE"
elif [ -n "$PR_NUMBER" ]; then
    if ! command -v gh >/dev/null 2>&1; then
        printf 'pr-report: gh CLI not found\n' >&2
        exit 1
    fi
    REPO_FLAG=""
    if [ -n "$REPO" ]; then
        REPO_FLAG="--repo $REPO"
    fi
    # Check if a prior comment with the marker exists
    PRIOR_ID=$(gh pr view "$PR_NUMBER" $REPO_FLAG --json comments \
        --jq '.comments[] | select(.body | contains("autospec-test-report-marker")) | .databaseId' \
        2>/dev/null | head -1 || echo "")
    if [ -n "$PRIOR_ID" ]; then
        # Edit the existing comment
        gh api "repos/{owner}/{repo}/issues/comments/$PRIOR_ID" \
            --method PATCH \
            --field "body=$COMMENT" $REPO_FLAG >/dev/null
        printf 'pr-report: updated comment %s on PR #%s\n' "$PRIOR_ID" "$PR_NUMBER"
    else
        gh pr comment "$PR_NUMBER" $REPO_FLAG --body "$COMMENT"
        printf 'pr-report: posted new comment on PR #%s\n' "$PR_NUMBER"
    fi
else
    printf '%s\n' "$COMMENT"
fi
