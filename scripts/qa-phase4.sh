#!/usr/bin/env bash
# scripts/qa-phase4.sh — Phase 4 QA chain runner for autospec implementer subagents.
#
# Usage:
#   bash scripts/qa-phase4.sh [--pr <N>] [--issue <N>] [--base <ref>] [--issue-labels <csv>]
#                              [--issue-body <file>] [--dry-run]
#
# Runs the deterministic QA steps in order:
#   1. lint-implementation.sh (RULE_ID deterministic checks)
#   2. Project test suite (bats)
#   3. run-mutation-test.sh (opt-in: area:safety / area:hardening / mutation-testing: required)
#
# Steps 1 and 2 must pass before step 3 is invoked.
# Exits 0 if all enabled steps pass, non-zero otherwise.
#
# Options:
#   --pr <N>              PR number (passed to lint-implementation.sh)
#   --issue <N>           Issue number (passed to lint-implementation.sh)
#   --base <ref>          Git base ref for mutation testing diff (default: origin/main)
#   --issue-labels <csv>  Issue labels for mutation opt-in gating
#   --issue-body <file>   Issue body file for mutation escape-hatch check
#   --dry-run             Print steps that would run but do not execute them
#   --help                Print this help

set -eu

PR=""
ISSUE=""
BASE_REF="${AUTOSPEC_BASE_REF:-origin/main}"
ISSUE_LABELS=""
ISSUE_BODY_FILE=""
DRY_RUN=0

while [ $# -gt 0 ]; do
    _arg="$1"
    _val="${2:-}"
    case "$_arg" in
        --pr)           PR="$_val";           shift 2 ;;
        --issue)        ISSUE="$_val";        shift 2 ;;
        --base)         BASE_REF="$_val";     shift 2 ;;
        --issue-labels) ISSUE_LABELS="$_val"; shift 2 ;;
        --issue-body)   ISSUE_BODY_FILE="$_val"; shift 2 ;;
        --dry-run)      DRY_RUN=1;            shift   ;;
        --help)
            sed -n '2,/^set -eu/p' "$0" | grep '^#' | sed 's/^# \?//'
            exit 0 ;;
        *)
            printf 'qa-phase4.sh: unknown argument: %s\n' "$_arg" >&2
            exit 2 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

# ── Step 1: lint-implementation.sh ───────────────────────────────────────────

LINT_SCRIPT="$SCRIPT_DIR/lint-implementation.sh"

if [ ! -f "$LINT_SCRIPT" ]; then
    printf '[qa-phase4] WARN: lint-implementation.sh not found at %s — skipping lint\n' "$LINT_SCRIPT" >&2
else
    printf '[qa-phase4] Step 1: lint-implementation.sh\n'
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '[qa-phase4]   (dry-run) would run: bash %s%s%s --staged\n' \
            "$LINT_SCRIPT" "${PR:+ $PR}" "${ISSUE:+ --issue $ISSUE}"
    else
        LINT_ARGS=""
        [ -n "$PR" ]    && LINT_ARGS="$LINT_ARGS $PR"
        [ -n "$ISSUE" ] && LINT_ARGS="$LINT_ARGS --issue $ISSUE"
        set +e
        # shellcheck disable=SC2086
        bash "$LINT_SCRIPT" $LINT_ARGS 2>&1
        LINT_EXIT=$?
        set -e
        if [ "$LINT_EXIT" -ne 0 ]; then
            printf '[qa-phase4] Step 1 FAILED: lint found %d blocking finding(s)\n' "$LINT_EXIT"
            exit "$LINT_EXIT"
        fi
        printf '[qa-phase4] Step 1 passed\n'
    fi
fi

# ── Step 2: project test suite (bats) ────────────────────────────────────────

printf '[qa-phase4] Step 2: project test suite\n'
if [ "$DRY_RUN" -eq 1 ]; then
    printf '[qa-phase4]   (dry-run) would run: bats %s/tests/\n' "$REPO_ROOT"
else
    if command -v bats >/dev/null 2>&1 && [ -d "$REPO_ROOT/tests" ]; then
        set +e
        bats "$REPO_ROOT/tests/" 2>&1
        BATS_EXIT=$?
        set -e
        if [ "$BATS_EXIT" -ne 0 ]; then
            printf '[qa-phase4] Step 2 FAILED: test suite exit %d\n' "$BATS_EXIT"
            exit "$BATS_EXIT"
        fi
        printf '[qa-phase4] Step 2 passed\n'
    else
        printf '[qa-phase4] Step 2: bats not found or no tests/ dir — skipping\n'
    fi
fi

# ── Step 3: mutation testing gate (opt-in) ────────────────────────────────────

MUTATION_SCRIPT="$SCRIPT_DIR/run-mutation-test.sh"

if [ ! -f "$MUTATION_SCRIPT" ]; then
    printf '[qa-phase4] WARN: run-mutation-test.sh not found — skipping mutation gate\n' >&2
else
    printf '[qa-phase4] Step 3: mutation testing gate\n'
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '[qa-phase4]   (dry-run) would run: bash %s --base %s --issue-labels "%s"\n' \
            "$MUTATION_SCRIPT" "$BASE_REF" "$ISSUE_LABELS"
    else
        MUTATION_ARGS="--base $BASE_REF"
        [ -n "$ISSUE_LABELS" ]    && MUTATION_ARGS="$MUTATION_ARGS --issue-labels $ISSUE_LABELS"
        [ -n "$ISSUE_BODY_FILE" ] && MUTATION_ARGS="$MUTATION_ARGS --issue-body $ISSUE_BODY_FILE"
        MUTATION_ARGS="$MUTATION_ARGS --repo-root $REPO_ROOT"

        set +e
        # shellcheck disable=SC2086
        bash "$MUTATION_SCRIPT" $MUTATION_ARGS 2>&1
        MUTATION_EXIT=$?
        set -e

        case "$MUTATION_EXIT" in
            0)
                printf '[qa-phase4] Step 3 passed\n'
                ;;
            1)
                printf '[qa-phase4] Step 3 FAILED: mutation coverage below floor\n'
                exit 1
                ;;
            *)
                printf '[qa-phase4] Step 3 error (exit %d)\n' "$MUTATION_EXIT"
                exit "$MUTATION_EXIT"
                ;;
        esac
    fi
fi

printf '[qa-phase4] all QA steps passed\n'
exit 0
