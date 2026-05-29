#!/usr/bin/env bash
# scripts/dispatch-implementer.sh — canonical helper for parallel-safe
# Phase 4 implementer dispatch.
#
# Per issue #690: when the orchestrator dispatches multiple background
# implementer Agents in the same git workdir, they collide on `git checkout`
# even when file-level scopes are disjoint. This helper enforces worktree
# isolation by creating /tmp/wt-<branch> and pre-pending a worktree directive
# to the implementer prompt so the LLM cannot stray into the main checkout.
#
# Usage:
#   dispatch-implementer.sh --issue <N> --branch <name> --prompt-file <path>
#   dispatch-implementer.sh --issue <N> --branch <name> --prompt-file <path> --cleanup
#
# Modes:
#   default: create worktree, emit augmented prompt on stdout, exit 0
#   --cleanup: remove the worktree at /tmp/wt-<branch> and exit
#
# Exit codes:
#   0  success
#   1  bad arguments
#   2  worktree creation/removal failed
#   3  prompt-file missing or unreadable

set -eu

usage() {
    cat <<'EOF'
Usage: dispatch-implementer.sh --issue <N> --branch <name> --prompt-file <path>
       dispatch-implementer.sh --issue <N> --branch <name> --cleanup

Creates /tmp/wt-<branch> via `git worktree add` and pre-pends a worktree
directive to the implementer prompt. Pass --cleanup to remove the worktree
after the implementer has completed.
EOF
}

ISSUE=""
BRANCH=""
PROMPT_FILE=""
CLEANUP=0
BASE_REF="${AUTOSPEC_DISPATCH_BASE_REF:-origin/main}"

while [ $# -gt 0 ]; do
    case "$1" in
        --issue)       ISSUE="$2"; shift 2 ;;
        --branch)      BRANCH="$2"; shift 2 ;;
        --prompt-file) PROMPT_FILE="$2"; shift 2 ;;
        --cleanup)     CLEANUP=1; shift ;;
        --base)        BASE_REF="$2"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *)             echo "dispatch-implementer.sh: unknown arg: $1" >&2; usage >&2; exit 1 ;;
    esac
done

if [ -z "$ISSUE" ] || [ -z "$BRANCH" ]; then
    echo "dispatch-implementer.sh: --issue and --branch are required" >&2
    usage >&2
    exit 1
fi

WT_PATH="/tmp/wt-${BRANCH}"

if [ "$CLEANUP" -eq 1 ]; then
    if [ -d "$WT_PATH" ]; then
        git worktree remove --force "$WT_PATH" 2>/dev/null \
            || rm -rf "$WT_PATH" \
            || { echo "dispatch-implementer.sh: failed to remove $WT_PATH" >&2; exit 2; }
    fi
    exit 0
fi

if [ -z "$PROMPT_FILE" ] || [ ! -r "$PROMPT_FILE" ]; then
    echo "dispatch-implementer.sh: --prompt-file must point to a readable file" >&2
    exit 3
fi

# Create the worktree. If it already exists (rare; orchestrator restart), reuse.
if [ ! -d "$WT_PATH" ]; then
    git worktree add -b "$BRANCH" "$WT_PATH" "$BASE_REF" >/dev/null 2>&1 \
        || git worktree add "$WT_PATH" "$BRANCH" >/dev/null 2>&1 \
        || { echo "dispatch-implementer.sh: failed to create worktree at $WT_PATH" >&2; exit 2; }
fi

# Emit the augmented prompt: worktree directive + original prompt body.
cat <<EOF
**Workdir:** \`$WT_PATH\` (worktree). All \`cd\`, \`git\`, \`gh\`, edit, and
test commands MUST run from this worktree. Do NOT touch the main checkout.
Do NOT \`git checkout\` other branches. This is parallel-safety isolation
per autospec-run Phase 4 worktree contract (issue #690).

Issue: #$ISSUE
Branch: $BRANCH

---

EOF
cat "$PROMPT_FILE"
