#!/usr/bin/env bash
# scripts/worktree-guard.sh — deterministic worktree preflight + creation guard.
#
# The worktree rule (autospec-run SKILL.md) exists only as prose; this script
# makes it enforceable. It is the shared, no-LLM enforcement tool referenced by
# docs/specs/2026-06-03-worktree-guard-design.md §D1. Installed to
# ~/.autospec/scripts/ by install.sh's copy_repo_scripts glob.
#
# Subcommands:
#   worktree-guard.sh assert [--base <ref>] [--strict-base]
#       Preflight the current directory before any edit/commit.
#   worktree-guard.sh resolve-branch --branch <B> --repo <O/R>
#       The G2 PR-aware ladder. Emits a JSON verdict; exit 0 always.
#   worktree-guard.sh create --branch <B> [--base <ref>] [--path <P>] [--adopt]
#       Create (or verified-clean reuse) a fresh worktree off the base.
#
# Exit codes (PINNED — shared across spec-2 children #959-#966):
#   0  ok
#   2  usage / not a git directory
#   3  in_primary_checkout   (git-dir == git-common-dir)
#   4  dirty                 (git status --porcelain non-empty, incl. untracked)
#                            also: create dirty/wrong-branch reuse refusal
#   5  stale_base            (HEAD behind base after fetch; warn unless --strict-base)
#
# resolve-branch JSON: {"state":"open-pr"|"branch-only"|"fresh","pr":N|null}.
#
# Conventions: set -eu (no pipefail — we branch on subcommand exits explicitly);
# usage()/die() helpers per release-issue.sh style; no RETURN traps; if/then/fi
# for one-sided conditionals under set -e (repo gotchas).

set -eu

PROG="worktree-guard.sh"

usage() {
    cat <<'EOF'
Usage:
  worktree-guard.sh assert [--base <ref>] [--strict-base]
  worktree-guard.sh resolve-branch --branch <B> --repo <O/R>
  worktree-guard.sh create --branch <B> [--base <ref>] [--path <P>] [--adopt]

Subcommands:
  assert          Preflight the current directory. Exit 0 ok / 2 usage|non-git /
                  3 in_primary_checkout / 4 dirty / 5 stale_base.
  resolve-branch  PR-aware ladder verdict as JSON on stdout (exit 0 always):
                  {"state":"open-pr"|"branch-only"|"fresh","pr":N|null}.
  create          Create or verified-clean-reuse a fresh worktree off the base.
                  Dirty or wrong-branch reuse is refused (exit 4).

Exit codes: 0 ok, 2 usage/non-git, 3 in_primary_checkout, 4 dirty, 5 stale_base.
EOF
}

die() {
    # die <exit-code> <message...>
    local code="$1"; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

# Emit a stable code_health/identifier line so callers can grep it.
emit() { printf '%s\n' "$*" >&2; }

# ---------------------------------------------------------------------------
# assert
# ---------------------------------------------------------------------------
cmd_assert() {
    local base="origin/main"
    local strict_base=0

    while [ $# -gt 0 ]; do
        case "$1" in
            --base)        [ $# -ge 2 ] || die 2 "--base requires a value"; base="$2"; shift 2 ;;
            --strict-base) strict_base=1; shift ;;
            -h|--help)     usage; exit 0 ;;
            *)             die 2 "assert: unknown arg: $1" ;;
        esac
    done

    # Not a git directory -> usage error (exit 2).
    local git_dir common_dir
    if ! git_dir="$(git rev-parse --git-dir 2>/dev/null)"; then
        die 2 "assert: not inside a git repository"
    fi
    common_dir="$(git rev-parse --git-common-dir 2>/dev/null)" \
        || die 2 "assert: cannot resolve --git-common-dir"

    # Normalise both to absolute paths before comparing: in a linked worktree
    # --git-dir is <primary>/.git/worktrees/<name> and --git-common-dir is
    # <primary>/.git, so they differ. In the primary checkout both resolve to
    # the same .git directory.
    local abs_git_dir abs_common_dir
    abs_git_dir="$(cd "$git_dir" 2>/dev/null && pwd -P)" || abs_git_dir="$git_dir"
    abs_common_dir="$(cd "$common_dir" 2>/dev/null && pwd -P)" || abs_common_dir="$common_dir"
    if [ "$abs_git_dir" = "$abs_common_dir" ]; then
        emit "code_health:in_primary_checkout"
        die 3 "assert: cwd is the primary checkout (git-dir == git-common-dir); agents must work in a linked worktree"
    fi

    # Dirty tree (untracked included) -> exit 4.
    local porcelain
    porcelain="$(git status --porcelain 2>/dev/null || true)"
    if [ -n "$porcelain" ]; then
        emit "code_health:dirty"
        die 4 "assert: worktree is dirty (uncommitted or untracked changes present)"
    fi

    # Stale base: fetch the base, then compare HEAD against the base tip.
    # Fetch failure is surfaced (retry once) rather than silently passing.
    local base_remote base_ref
    base_remote="${base%%/*}"
    base_ref="${base#*/}"
    if [ "$base_remote" = "$base" ]; then
        base_remote="origin"
        base_ref="$base"
    fi
    if ! git fetch "$base_remote" "$base_ref" >/dev/null 2>&1; then
        if ! git fetch "$base_remote" "$base_ref" >/dev/null 2>&1; then
            emit "code_health:fetch_failed"
            die 5 "assert: git fetch $base_remote $base_ref failed (after retry)"
        fi
    fi

    local head_sha base_sha merge_base
    head_sha="$(git rev-parse HEAD 2>/dev/null || true)"
    base_sha="$(git rev-parse "$base" 2>/dev/null || true)"
    if [ -n "$base_sha" ] && [ "$head_sha" != "$base_sha" ]; then
        # HEAD differs from base tip. For an adopted branch HEAD legitimately
        # diverges, so "stale" only when base has commits HEAD lacks, i.e.
        # merge-base(HEAD, base) != base tip.
        merge_base="$(git merge-base HEAD "$base" 2>/dev/null || true)"
        if [ "$merge_base" != "$base_sha" ]; then
            emit "code_health:stale_base base=$base head=$head_sha base_tip=$base_sha"
            if [ "$strict_base" -eq 1 ]; then
                die 5 "assert: stale base — $base has advanced beyond this worktree (--strict-base)"
            fi
            emit "assert: WARN stale base — $base has advanced; rebase before merge (warn-level)"
        fi
    fi

    exit 0
}

# ---------------------------------------------------------------------------
# resolve-branch
# ---------------------------------------------------------------------------
cmd_resolve_branch() {
    local branch="" repo=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --branch)  [ $# -ge 2 ] || die 2 "--branch requires a value"; branch="$2"; shift 2 ;;
            --repo)    [ $# -ge 2 ] || die 2 "--repo requires a value"; repo="$2"; shift 2 ;;
            -h|--help) usage; exit 0 ;;
            *)         die 2 "resolve-branch: unknown arg: $1" ;;
        esac
    done
    [ -n "$branch" ] || die 2 "resolve-branch: --branch is required"
    [ -n "$repo" ]   || die 2 "resolve-branch: --repo is required"

    # Rung 1: an open PR whose head is this branch -> open-pr.
    local pr_json pr_number
    pr_json="$(gh pr list --repo "$repo" --head "$branch" --state open --json number 2>/dev/null || echo '[]')"
    pr_number="$(printf '%s' "$pr_json" | jq -r 'if type=="array" and length>0 then .[0].number else empty end' 2>/dev/null || true)"
    if [ -n "$pr_number" ]; then
        printf '{"state":"open-pr","pr":%s}\n' "$pr_number"
        exit 0
    fi

    # Rung 2: branch exists on origin -> branch-only.
    local lsr
    lsr="$(git ls-remote --heads origin "$branch" 2>/dev/null || true)"
    if [ -n "$lsr" ]; then
        printf '{"state":"branch-only","pr":null}\n'
        exit 0
    fi

    # Rung 3: nothing exists -> fresh.
    printf '{"state":"fresh","pr":null}\n'
    exit 0
}

# ---------------------------------------------------------------------------
# create
# ---------------------------------------------------------------------------
cmd_create() {
    local branch="" base="origin/main" path="" adopt=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --branch)  [ $# -ge 2 ] || die 2 "--branch requires a value"; branch="$2"; shift 2 ;;
            --base)    [ $# -ge 2 ] || die 2 "--base requires a value"; base="$2"; shift 2 ;;
            --path)    [ $# -ge 2 ] || die 2 "--path requires a value"; path="$2"; shift 2 ;;
            --adopt)   adopt=1; shift ;;
            -h|--help) usage; exit 0 ;;
            *)         die 2 "create: unknown arg: $1" ;;
        esac
    done
    [ -n "$branch" ] || die 2 "create: --branch is required"
    [ -n "$path" ]   || path="/tmp/wt-${branch}"

    # fetch-before-branch (G4) with a single retry; surface on persistent failure.
    if ! git fetch origin >/dev/null 2>&1; then
        if ! git fetch origin >/dev/null 2>&1; then
            emit "code_health:fetch_failed"
            die 2 "create: git fetch origin failed (after retry)"
        fi
    fi

    # Existing path: reuse ONLY if clean AND on the same branch; else refuse.
    if [ -e "$path" ]; then
        local existing_branch existing_porcelain
        existing_branch="$(git -C "$path" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
        existing_porcelain="$(git -C "$path" status --porcelain 2>/dev/null || true)"
        if [ "$existing_branch" = "$branch" ] && [ -z "$existing_porcelain" ]; then
            # Clean, same branch -> idempotent reuse.
            exit 0
        fi
        emit "code_health:worktree_dirty_reuse_refused path=$path branch=$existing_branch want=$branch"
        die 4 "create: refusing to reuse worktree at $path (dirty or wrong branch); never silent-reuse"
    fi

    # Adopt an existing remote branch, or create a fresh branch off the base.
    if [ "$adopt" -eq 1 ]; then
        if ! git worktree add "$path" "origin/$branch" >/dev/null 2>&1; then
            emit "code_health:worktree_create_failed"
            die 2 "create: git worktree add (adopt) failed for origin/$branch at $path"
        fi
        # Land on a local tracking branch named B, not detached at origin/B.
        git -C "$path" checkout -q -B "$branch" "origin/$branch" >/dev/null 2>&1 || true
    else
        if ! git worktree add -b "$branch" "$path" "$base" >/dev/null 2>&1; then
            emit "code_health:worktree_create_failed"
            die 2 "create: git worktree add -b $branch off $base at $path failed"
        fi
    fi

    exit 0
}

# ---------------------------------------------------------------------------
# dispatch
# ---------------------------------------------------------------------------
main() {
    [ $# -ge 1 ] || { usage >&2; exit 2; }
    local sub="$1"; shift
    case "$sub" in
        assert)         cmd_assert "$@" ;;
        resolve-branch) cmd_resolve_branch "$@" ;;
        create)         cmd_create "$@" ;;
        -h|--help)      usage; exit 0 ;;
        *)              echo "$PROG: unknown subcommand: $sub" >&2; usage >&2; exit 2 ;;
    esac
}

main "$@"
