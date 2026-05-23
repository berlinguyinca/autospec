#!/usr/bin/env bash
# scripts/run-mutation-test.sh — mutation testing orchestrator for autospec Phase 4 QA chain.
#
# Usage:
#   bash scripts/run-mutation-test.sh --base <ref> [options]
#
# Options:
#   --base <ref>          Git base ref to diff against (required)
#   --repo-root <path>    Path to repo root (default: $(git rev-parse --show-toplevel))
#   --issue-labels <csv>  Comma-separated issue labels (for opt-in gating)
#   --issue-body <file>   Path to file containing issue body (for escape-hatch check)
#   --bash-adapter <path> Override bash mutation adapter path
#   --skip-flag           Force mutation-testing: skip (escape hatch)
#   --help                Print this help and exit
#
# Opt-in: gate fires only when issue has area:safety, area:hardening, or
#         mutation-testing: required label. Exits 0 (skip) otherwise.
#
# Escape hatch: if issue body contains "mutation-testing: skip" (case-insensitive),
#               exits 0 with skip message.
#
# Gate threshold: MUTATION_KILL_FLOOR=80 (percent killed per changed file).
# Configurable via .autospec/mutation-testing.yml (key: kill_floor).
#
# Exit codes:
#   0 — gate passed (or skipped)
#   1 — one or more files below kill floor (mutation coverage gap)
#   2 — usage/setup error
#
# Directive emitted on failure (for adaptive-retry loop):
#   missing_mutation_coverage:<file>:<kill_pct>%: Add tests covering mutants that survived.

set -eu

# ── Defaults ───────────────────────────────────────────────────────────────────

MUTATION_KILL_FLOOR="${MUTATION_KILL_FLOOR:-80}"
BASE_REF=""
REPO_ROOT=""
ISSUE_LABELS=""
ISSUE_BODY_FILE=""
BASH_ADAPTER_OVERRIDE=""
SKIP_FLAG=0

# ── Argument parsing ───────────────────────────────────────────────────────────

while [ $# -gt 0 ]; do
    case "$1" in
        --base)         BASE_REF="$2";             shift 2 ;;
        --repo-root)    REPO_ROOT="$2";            shift 2 ;;
        --issue-labels) ISSUE_LABELS="$2";         shift 2 ;;
        --issue-body)   ISSUE_BODY_FILE="$2";      shift 2 ;;
        --bash-adapter) BASH_ADAPTER_OVERRIDE="$2"; shift 2 ;;
        --skip-flag)    SKIP_FLAG=1;               shift   ;;
        --help)
            sed -n '2,/^set -eu/p' "$0" | grep '^#' | sed 's/^# \?//'
            exit 0 ;;
        *)
            printf 'run-mutation-test.sh: unknown argument: %s\n' "$1" >&2
            exit 2 ;;
    esac
done

if [ -z "$BASE_REF" ]; then
    printf 'run-mutation-test.sh: --base <ref> is required\n' >&2
    exit 2
fi

# ── Resolve repo root ──────────────────────────────────────────────────────────

if [ -z "$REPO_ROOT" ]; then
    REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Load kill floor from .autospec/mutation-testing.yml if present ─────────────

MUTATION_CFG="$REPO_ROOT/.autospec/mutation-testing.yml"
if [ -f "$MUTATION_CFG" ] && command -v grep >/dev/null 2>&1; then
    _cfg_floor="$(grep -E '^[[:space:]]*kill_floor[[:space:]]*:' "$MUTATION_CFG" 2>/dev/null \
        | head -1 | sed 's/.*:[[:space:]]*//' | tr -d '[:space:]' || true)"
    if [ -n "$_cfg_floor" ] && printf '%s' "$_cfg_floor" | grep -qE '^[0-9]+$'; then
        MUTATION_KILL_FLOOR="$_cfg_floor"
    fi
fi

# ── Escape hatch: --skip-flag ──────────────────────────────────────────────────

if [ "$SKIP_FLAG" -eq 1 ]; then
    printf '[mutation] mutation-testing: skip flag set — skipping gate\n'
    exit 0
fi

# ── Escape hatch: "mutation-testing: skip" in issue body ──────────────────────

if [ -n "$ISSUE_BODY_FILE" ] && [ -f "$ISSUE_BODY_FILE" ]; then
    if grep -qiE '^mutation-testing:[[:space:]]*skip[[:space:]]*$' "$ISSUE_BODY_FILE" 2>/dev/null; then
        printf '[mutation] mutation-testing: skip found in issue body — skipping gate\n'
        exit 0
    fi
fi

# ── Opt-in label check ─────────────────────────────────────────────────────────
# Gate fires only when issue has area:safety, area:hardening, or mutation-testing: required.

_has_opt_in_label=0
if printf '%s' "$ISSUE_LABELS" | grep -qE '(^|,)(area:safety|area:hardening|mutation-testing: required)(,|$)'; then
    _has_opt_in_label=1
elif printf '%s' "$ISSUE_LABELS" | tr ',' '\n' | grep -qE '^(area:safety|area:hardening|mutation-testing: required)$'; then
    _has_opt_in_label=1
fi

if [ "$_has_opt_in_label" -eq 0 ]; then
    printf '[mutation] opt-in label not present (area:safety / area:hardening / mutation-testing: required) — gate disabled, skipping\n'
    exit 0
fi

# ── Detect changed files since base ref ───────────────────────────────────────

cd "$REPO_ROOT"
CHANGED_FILES="$(git diff --name-only "$BASE_REF" HEAD 2>/dev/null | grep -v '^tests/' || true)"

if [ -z "$CHANGED_FILES" ]; then
    printf '[mutation] no changed source files since %s — skipping\n' "$BASE_REF"
    exit 0
fi

# ── Group changed files by extension ──────────────────────────────────────────

SH_FILES=""
TS_FILES=""
PY_FILES=""
GO_FILES=""

while IFS= read -r f; do
    [ -z "$f" ] && continue
    [ -f "$REPO_ROOT/$f" ] || continue
    case "$f" in
        *.sh)           SH_FILES="${SH_FILES:+$SH_FILES }$f" ;;
        *.ts|*.js|*.mjs) TS_FILES="${TS_FILES:+$TS_FILES }$f" ;;
        *.py)           PY_FILES="${PY_FILES:+$PY_FILES }$f" ;;
        *.go)           GO_FILES="${GO_FILES:+$GO_FILES }$f" ;;
    esac
done <<EOF
$CHANGED_FILES
EOF

printf '[mutation] changed source files: sh="%s" ts/js="%s" py="%s" go="%s"\n' \
    "${SH_FILES:-<none>}" "${TS_FILES:-<none>}" "${PY_FILES:-<none>}" "${GO_FILES:-<none>}"
printf '[mutation] kill floor: %s%%\n' "$MUTATION_KILL_FLOOR"

# ── Adapter paths ──────────────────────────────────────────────────────────────

BASH_ADAPTER="${BASH_ADAPTER_OVERRIDE:-$SCRIPT_DIR/../mutation-adapters/bash-mutate.sh}"
STRYKER_ADAPTER="$SCRIPT_DIR/../mutation-adapters/stryker.sh"
MUTMUT_ADAPTER="$SCRIPT_DIR/../mutation-adapters/mutmut.sh"
GOMUT_ADAPTER="$SCRIPT_DIR/../mutation-adapters/go-mutesting.sh"

# ── Dispatch and aggregate ─────────────────────────────────────────────────────

TOTAL_ALL=0
KILLED_ALL=0
FAIL=0
DIRECTIVES=""

_dispatch_adapter() {
    local adapter="$1"
    local lang="$2"
    local file="$3"

    if [ ! -f "$adapter" ]; then
        printf '[mutation] adapter not found for %s: %s — skipping file %s\n' "$lang" "$adapter" "$file" >&2
        return 0
    fi

    printf '[mutation] dispatching %s adapter for: %s\n' "$lang" "$file"

    local result
    result="$(bash "$adapter" "$REPO_ROOT/$file" 2>/dev/null || true)"

    local total killed
    total="$(printf '%s' "$result" | grep -o '"total":[0-9]*' | grep -o '[0-9]*' || echo 0)"
    killed="$(printf '%s' "$result" | grep -o '"killed":[0-9]*' | grep -o '[0-9]*' || echo 0)"
    total="${total:-0}"; killed="${killed:-0}"

    TOTAL_ALL=$((TOTAL_ALL + total))
    KILLED_ALL=$((KILLED_ALL + killed))

    if [ "$total" -gt 0 ]; then
        # Compute kill percent (integer arithmetic)
        local kill_pct
        kill_pct=$(( (killed * 100) / total ))
        printf '[mutation] %s: %d/%d killed (%d%%)\n' "$file" "$killed" "$total" "$kill_pct"
        if [ "$kill_pct" -lt "$MUTATION_KILL_FLOOR" ]; then
            printf 'missing_mutation_coverage:%s:%d%%: Add tests covering mutants that survived.\n' \
                "$file" "$kill_pct"
            DIRECTIVES="${DIRECTIVES}missing_mutation_coverage:${file}:${kill_pct}%: Add tests covering mutants that survived.\n"
            FAIL=1
        fi
    else
        printf '[mutation] %s: 0 mutants generated (adapter returned total=0)\n' "$file"
    fi
}

# Dispatch bash adapter
for f in $SH_FILES; do
    _dispatch_adapter "$BASH_ADAPTER" "bash" "$f"
done

# Dispatch stryker adapter (Node/TS/JS)
for f in $TS_FILES; do
    _dispatch_adapter "$STRYKER_ADAPTER" "stryker" "$f"
done

# Dispatch mutmut adapter (Python)
for f in $PY_FILES; do
    _dispatch_adapter "$MUTMUT_ADAPTER" "mutmut" "$f"
done

# Dispatch go-mutesting adapter (Go)
for f in $GO_FILES; do
    _dispatch_adapter "$GOMUT_ADAPTER" "go-mutesting" "$f"
done

# ── Summary ────────────────────────────────────────────────────────────────────

if [ "$TOTAL_ALL" -gt 0 ]; then
    OVERALL_PCT=$(( (KILLED_ALL * 100) / TOTAL_ALL ))
    printf '[mutation] overall: %d/%d killed (%d%%) — floor: %d%%\n' \
        "$KILLED_ALL" "$TOTAL_ALL" "$OVERALL_PCT" "$MUTATION_KILL_FLOOR"
else
    printf '[mutation] overall: no mutants generated across all changed files\n'
fi

if [ "$FAIL" -eq 1 ]; then
    printf '[mutation] GATE FAILED — mutation coverage below %d%% floor\n' "$MUTATION_KILL_FLOOR"
    exit 1
fi

printf '[mutation] gate passed\n'
exit 0
