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
#  75  disk low (disk-hygiene.sh preflight): issue parked, fresh worktree removed

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

# disk-hygiene.sh lives next to this script (repo) or in ~/.autospec/scripts/.
DISK_HYGIENE=""
if [ -n "$SCRIPT_DIR" ] && [ -x "$SCRIPT_DIR/disk-hygiene.sh" ]; then
    DISK_HYGIENE="$SCRIPT_DIR/disk-hygiene.sh"
elif [ -x "${HOME}/.autospec/scripts/disk-hygiene.sh" ]; then
    DISK_HYGIENE="${HOME}/.autospec/scripts/disk-hygiene.sh"
fi

if [ "$CLEANUP" -eq 1 ]; then
    if [ -d "$WT_PATH" ]; then
        # Guarded removal: a dirty / unpushed worktree is KEPT and logged (exit 0
        # so callers never treat "kept" as a failure). Without disk-hygiene.sh
        # fall back to the legacy forced removal.
        if [ -n "$DISK_HYGIENE" ]; then
            hygiene_rc=0
            bash "$DISK_HYGIENE" cleanup-worktree --reason dispatch-cleanup "$WT_PATH" || hygiene_rc=$?
            if [ "$hygiene_rc" -eq 2 ]; then
                # A leftover that is NOT a git worktree at all (e.g. an aborted
                # `mkdir`/copy) was removable before disk-hygiene existed and
                # would wedge the issue. Remove it only when it is a plain
                # directory we own under our own prefix that holds NO `.git`
                # entry (file or dir: a dangling worktree .git file means real
                # work), is outside any git repo and is not a registered
                # worktree (compared by canonical path: macOS lists /private/tmp).
                # Anything else is kept and reported.
                wt_real="$(cd "$WT_PATH" 2>/dev/null && pwd -P || true)"
                if [ -n "$wt_real" ] && [ ! -L "$WT_PATH" ] && [ -O "$WT_PATH" ] \
                    && case "$WT_PATH" in *..*) false ;; /tmp/wt-?*) true ;; *) false ;; esac \
                    && [ ! -e "$WT_PATH/.git" ] && [ ! -L "$WT_PATH/.git" ] \
                    && ! git -C "$WT_PATH" rev-parse --git-dir >/dev/null 2>&1 \
                    && ! git worktree list --porcelain 2>/dev/null | grep -qxF "worktree $wt_real"; then
                    rm -rf "$WT_PATH" 2>/dev/null || true
                fi
                if [ -e "$WT_PATH" ]; then
                    echo "dispatch-implementer.sh: failed to remove $WT_PATH" >&2
                    exit 2
                fi
            fi
        else
            git worktree remove --force "$WT_PATH" 2>/dev/null \
                || rm -rf "$WT_PATH" \
                || { echo "dispatch-implementer.sh: failed to remove $WT_PATH" >&2; exit 2; }
            git worktree prune 2>/dev/null || true
        fi
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
# A worktree this run CREATES is removed again (guarded: clean + pushed only)
# if the run aborts before the prompt is handed off. Ownership comes from
# worktree-guard itself (it writes this file only when it really created the
# worktree), never from a pre-create existence check: when two dispatchers race
# on one branch the loser reuses the winner's worktree and must not delete it.
CREATED_FILE="$(mktemp "${TMPDIR:-/tmp}/dispatch-created.XXXXXX")"
HANDOFF_DONE=0
abort_cleanup() {
    if [ "$HANDOFF_DONE" -eq 0 ] && [ -s "$CREATED_FILE" ] && [ -n "$DISK_HYGIENE" ] && [ -e "$WT_PATH" ]; then
        bash "$DISK_HYGIENE" cleanup-worktree --reason dispatch-abort "$WT_PATH" >/dev/null 2>&1 || true
    fi
    rm -f "$CREATED_FILE"
}
trap abort_cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT

guard_exit=0
WORKTREE_GUARD_CREATED_FILE="$CREATED_FILE" "$GUARD" create --branch "$BRANCH" --path "$WT_PATH" --base "$BASE_REF" 2>&1 || guard_exit=$?
if [ "$guard_exit" -ne 0 ]; then
    echo "dispatch-implementer.sh: worktree-guard.sh create exited $guard_exit for $WT_PATH (branch=$BRANCH)" >&2
    exit "$guard_exit"
fi

# Per-worktree build output (target/ inside the worktree) + disk pre-flight (ON by default; see disk-hygiene.sh).
HYGIENE_ENV=""
if [ -n "$DISK_HYGIENE" ]; then
    HYGIENE_ENV="$(bash "$DISK_HYGIENE" env "$WT_PATH" 2>/dev/null || true)"
    preflight_rc=0
    bash "$DISK_HYGIENE" preflight --label "issue-$ISSUE" --path "$WT_PATH" >&2 || preflight_rc=$?
    if [ "$preflight_rc" -eq 75 ]; then
        echo "dispatch-implementer.sh: disk low; parked issue #$ISSUE (code_health:disk_low_parked)" >&2
        exit 75
    fi
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

EOF
if [ -n "$HYGIENE_ENV" ]; then
    # shellcheck disable=SC2016 # literal markdown backticks
    printf '**Build env:** each worktree builds into its own `target/` (do not set `CARGO_TARGET_DIR`); run this in your shell before any cargo command.\n\n```bash\n%s\n```\n\n' "$HYGIENE_ENV"
fi
printf -- '---\n\n'
cat "$PROMPT_FILE"
HANDOFF_DONE=1
