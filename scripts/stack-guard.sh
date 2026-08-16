#!/usr/bin/env bash
# scripts/stack-guard.sh — per-layer size + linearity gate for always-stacked PRs.
#
# The PR size cap is enforced on a SINGLE stack layer (the base...head delta) rather
# than the whole accumulated diff, so a large change can ship as a chain of small PRs,
# each within the cap. It also validates the stack is linear: a PR's base must be the
# default branch or the head branch of another open PR (no orphan target branches).
#
# Advisory by default (INFO findings, exit 0). Under AUTOSPEC_PR_SIZE_STRICT=1 the
# findings are blocking (ERROR, exit 1). The size cap is reused from
# scripts/lint-implementation.sh via its --diff-file mode — a single source of truth,
# so this gate never drifts from the pre-commit / PR linter's PR_SIZE rule.
#
# Usage:
#   scripts/stack-guard.sh --pr N [--repo OWNER/NAME]
#   scripts/stack-guard.sh --base <ref> --head <ref> [--default-branch BR]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LINT_IMPL="${SCRIPT_DIR}/lint-implementation.sh"

PR=""
BASE=""
HEAD_REF=""
REPO=""
DEFAULT_BRANCH="${AUTOSPEC_STACK_DEFAULT_BRANCH:-}"
ASSUME_LINEAR=0
STRICT="${AUTOSPEC_PR_SIZE_STRICT:-0}"

usage() {
    cat <<'EOF'
Usage: scripts/stack-guard.sh [options]

Gate one stack layer: enforce the PR size cap on the base...head delta and validate
the base is linear (the default branch or an open PR's head branch). Advisory by
default; blocking under AUTOSPEC_PR_SIZE_STRICT=1.

Options:
  --pr N             Gate PR N (base/head resolved via gh)
  --base REF         Base ref (branch or commit)
  --head REF         Head ref (branch or commit)
  --repo OWNER/NAME  Repo for gh queries (default: current repo)
  --default-branch B Default branch name (default: gh repo view, else main)
  --assume-linear    Skip the open-PR linearity check (base just needs to resolve)
  -h, --help         Show this help

Environment:
  AUTOSPEC_PR_SIZE_STRICT=1      Make findings blocking instead of advisory
  AUTOSPEC_STACK_DEFAULT_BRANCH  Default branch name
EOF
}

die() { printf 'stack-guard: %s\n' "$*" >&2; exit 2; }

while [ $# -gt 0 ]; do
    case "$1" in
        --pr) [ $# -ge 2 ] || die "--pr requires an argument"; PR="$2"; shift 2 ;;
        --base) [ $# -ge 2 ] || die "--base requires an argument"; BASE="$2"; shift 2 ;;
        --head) [ $# -ge 2 ] || die "--head requires an argument"; HEAD_REF="$2"; shift 2 ;;
        --repo) [ $# -ge 2 ] || die "--repo requires an argument"; REPO="$2"; shift 2 ;;
        --default-branch) [ $# -ge 2 ] || die "--default-branch requires an argument"; DEFAULT_BRANCH="$2"; shift 2 ;;
        --assume-linear) ASSUME_LINEAR=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "unknown argument: $1" ;;
    esac
done

gh_cmd() { if [ -n "$REPO" ]; then gh --repo "$REPO" "$@"; else gh "$@"; fi; }

# Resolve base/head from the PR number when not supplied directly.
if [ -n "$PR" ]; then
    [ -n "$BASE" ] || BASE="$(gh_cmd pr view "$PR" --json baseRefName --jq .baseRefName)"
    [ -n "$HEAD_REF" ] || HEAD_REF="$(gh_cmd pr view "$PR" --json headRefName --jq .headRefName)"
fi
[ -n "$BASE" ] && [ -n "$HEAD_REF" ] || die "need --pr N or both --base and --head"

# Default branch: explicit flag > env > gh > main.
if [ -z "$DEFAULT_BRANCH" ]; then
    DEFAULT_BRANCH="$(gh_cmd repo view --json defaultBranchRef --jq .defaultBranchRef.name 2>/dev/null || true)"
    [ -n "$DEFAULT_BRANCH" ] || DEFAULT_BRANCH="main"
fi
base_name="${BASE#origin/}"

TMP_DIFF="$(mktemp -t stack-guard-diff.XXXXXX)"
trap 'rm -f "$TMP_DIFF"' EXIT

# Per-layer delta = merge-base(base,head)..head: what this layer adds on top of base.
if ! git diff "${BASE}...${HEAD_REF}" > "$TMP_DIFF" 2>/dev/null; then
    git diff "origin/${BASE}...${HEAD_REF}" > "$TMP_DIFF" 2>/dev/null \
        || die "cannot compute diff for ${BASE}...${HEAD_REF}"
fi

# ── Per-layer size: reuse lint-implementation.sh's PR_SIZE detector ──────────
pr_size_line="$(AUTOSPEC_PR_SIZE_STRICT="$STRICT" bash "$LINT_IMPL" --diff-file "$TMP_DIFF" 2>/dev/null \
    | grep -E '^(INFO|ERROR):PR_SIZE' | head -1 || true)"
pr_size_blocking=0
case "$pr_size_line" in ERROR:*) pr_size_blocking=1 ;; esac

# ── Linearity: base must be the default branch or an open PR's head branch ───
linear=1
linear_reason="base ${base_name} == default branch ${DEFAULT_BRANCH}"
if [ "$ASSUME_LINEAR" -ne 1 ] && [ "$base_name" != "$DEFAULT_BRANCH" ]; then
    linear=0
    heads="$(gh_cmd pr list --state open --limit 200 --json headRefName --jq '.[].headRefName' 2>/dev/null || true)"
    while IFS= read -r h; do
        if [ "$h" = "$base_name" ]; then
            linear=1
            linear_reason="base ${base_name} is the head of an open PR"
            break
        fi
    done <<< "$heads"
fi

# ── Emit findings ─────────────────────────────────────────────────────────────
blocking=0
if [ -n "$pr_size_line" ]; then
    printf '%s\n' "$pr_size_line"
    [ "$pr_size_blocking" -eq 1 ] && blocking=1
else
    printf 'INFO:PR_SIZE:-:-: per-layer (base...head) under cap\n'
fi
if [ "$linear" -eq 1 ]; then
    printf 'INFO:STACK_BASE:-:-: linear: %s\n' "$linear_reason"
else
    if [ "$STRICT" = "1" ]; then
        printf 'ERROR:STACK_BASE:-:-: base %s is not %s and not an open PR head; the stack must be linear\n' \
            "$base_name" "$DEFAULT_BRANCH"
        blocking=1
    else
        printf 'INFO:STACK_BASE:-:-: base %s is not %s and not an open PR head; the stack must be linear (advisory)\n' \
            "$base_name" "$DEFAULT_BRANCH"
    fi
fi

if [ "$blocking" -eq 1 ]; then
    printf 'stack-guard: FAIL\n'
    exit 1
fi
printf 'stack-guard: OK\n'
exit 0
