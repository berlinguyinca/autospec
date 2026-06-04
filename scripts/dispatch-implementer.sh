#!/usr/bin/env bash
# scripts/dispatch-implementer.sh — canonical helper for parallel-safe
# Phase 4 implementer dispatch.
#
# Per issue #690: when the orchestrator dispatches multiple background
# implementer Agents in the same git workdir, they collide on `git checkout`
# even when file-level scopes are disjoint. This helper enforces worktree
# isolation by delegating to worktree-guard.sh (create/resolve-branch) and
# pre-pending a worktree directive to the implementer prompt so the LLM cannot
# stray into the main checkout.
#
# Per issue #960 (D2): the silent-reuse fallback has been removed. Worktree
# creation is fully delegated to `worktree-guard.sh create`, which refuses dirty
# or wrong-branch reuse with exit 4 / code_health:worktree_dirty_reuse_refused.
# `worktree-guard.sh resolve-branch` is called first so the orchestrator can act
# on open-pr / branch-only / fresh state before dispatch.
#
# Usage:
#   dispatch-implementer.sh --issue <N> --branch <name> --prompt-file <path>
#                            [--repo <O/R>] [--base <ref>]
#   dispatch-implementer.sh --issue <N> --branch <name> --cleanup
#
# Modes:
#   default: resolve branch, create worktree, emit augmented prompt on stdout.
#            BRANCH_VERDICT env is printed as a comment in the output header.
#   --cleanup: remove the worktree at /tmp/wt-<branch> and exit.
#
# Exit codes:
#   0  success
#   1  bad arguments / worktree-guard.sh not found
#   2  worktree creation/removal failed (propagated from worktree-guard.sh)
#   3  prompt-file missing or unreadable
#   4  dirty/wrong-branch reuse refused (propagated from worktree-guard.sh)

set -eu

usage() {
    cat <<'EOF'
Usage: dispatch-implementer.sh --issue <N> --branch <name> --prompt-file <path>
                                [--repo <O/R>] [--base <ref>]
       dispatch-implementer.sh --issue <N> --branch <name> --cleanup

Resolves the branch ladder verdict via worktree-guard.sh resolve-branch, then
creates /tmp/wt-<branch> via worktree-guard.sh create (no silent dirty reuse),
and pre-pends a worktree directive + verdict to the implementer prompt.
Pass --cleanup to remove the worktree after the implementer has completed.
EOF
}

ISSUE=""
BRANCH=""
PROMPT_FILE=""
CLEANUP=0
BASE_REF="${AUTOSPEC_DISPATCH_BASE_REF:-origin/main}"
REPO="${AUTOSPEC_DISPATCH_REPO:-}"

while [ $# -gt 0 ]; do
    case "$1" in
        --issue)       ISSUE="$2"; shift 2 ;;
        --branch)      BRANCH="$2"; shift 2 ;;
        --prompt-file) PROMPT_FILE="$2"; shift 2 ;;
        --cleanup)     CLEANUP=1; shift ;;
        --base)        BASE_REF="$2"; shift 2 ;;
        --repo)        REPO="$2"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *)             echo "dispatch-implementer.sh: unknown arg: $1" >&2; usage >&2; exit 1 ;;
    esac
done

if [ -z "$ISSUE" ] || [ -z "$BRANCH" ]; then
    echo "dispatch-implementer.sh: --issue and --branch are required" >&2
    usage >&2
    exit 1
fi

# Locate worktree-guard.sh: prefer the repo-local copy, then ~/.autospec/scripts/.
SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd -P)" || SCRIPT_DIR=""
GUARD=""
if [ -n "$SCRIPT_DIR" ] && [ -x "$SCRIPT_DIR/worktree-guard.sh" ]; then
    GUARD="$SCRIPT_DIR/worktree-guard.sh"
elif [ -x "${HOME}/.autospec/scripts/worktree-guard.sh" ]; then
    GUARD="${HOME}/.autospec/scripts/worktree-guard.sh"
else
    echo "dispatch-implementer.sh: worktree-guard.sh not found (checked $SCRIPT_DIR and ~/.autospec/scripts/)" >&2
    exit 1
fi

WT_PATH="/tmp/wt-${BRANCH}"

if [ "$CLEANUP" -eq 1 ]; then
    if [ -d "$WT_PATH" ]; then
        git worktree remove --force "$WT_PATH" 2>/dev/null \
            || rm -rf "$WT_PATH" \
            || { echo "dispatch-implementer.sh: failed to remove $WT_PATH" >&2; exit 2; }
        git worktree prune 2>/dev/null || true
    fi
    exit 0
fi

if [ -z "$PROMPT_FILE" ] || [ ! -r "$PROMPT_FILE" ]; then
    echo "dispatch-implementer.sh: --prompt-file must point to a readable file" >&2
    exit 3
fi

# Step 1: resolve-branch ladder — surface the verdict BEFORE worktree creation
# so the orchestrator can decide open-pr / branch-only / fresh handling.
VERDICT_JSON=""
if [ -n "$REPO" ]; then
    VERDICT_JSON="$("$GUARD" resolve-branch --branch "$BRANCH" --repo "$REPO" 2>/dev/null || true)"
else
    # No --repo provided: skip the open-PR rung (requires gh), do branch-only check only.
    VERDICT_JSON='{"state":"fresh","pr":null}'
fi
# Normalise: if guard returned nothing, treat as fresh.
if [ -z "$VERDICT_JSON" ]; then
    VERDICT_JSON='{"state":"fresh","pr":null}'
fi

# Step 2: delegate worktree creation to worktree-guard.sh create.
# This enforces: no silent dirty reuse (exit 4 with code_health:worktree_dirty_reuse_refused),
# fetch-before-branch (G4), and idempotent clean reuse.
guard_exit=0
"$GUARD" create --branch "$BRANCH" --path "$WT_PATH" --base "$BASE_REF" 2>&1 || guard_exit=$?
if [ "$guard_exit" -ne 0 ]; then
    echo "dispatch-implementer.sh: worktree-guard.sh create exited $guard_exit for $WT_PATH (branch=$BRANCH)" >&2
    exit "$guard_exit"
fi

# Emit the augmented prompt: verdict header + worktree directive + original prompt body.
cat <<EOF
<!-- dispatch-implementer: branch_verdict=$VERDICT_JSON -->

**Branch verdict:** \`$VERDICT_JSON\`
The orchestrator should act on \`state\` before proceeding:
- \`open-pr\`: an open PR already exists — validate + merge the existing PR, no re-implementation.
- \`branch-only\`: branch exists remotely but no open PR — adopt in this worktree and continue.
- \`fresh\`: no prior work exists — proceed with new implementation.

**Workdir:** \`$WT_PATH\` (worktree). All \`cd\`, \`git\`, \`gh\`, edit, and
test commands MUST run from this worktree. Do NOT touch the main checkout.
Do NOT \`git checkout\` other branches. This is parallel-safety isolation
per autospec-run Phase 4 worktree contract (issue #690).

Issue: #$ISSUE
Branch: $BRANCH

---

EOF
cat "$PROMPT_FILE"
