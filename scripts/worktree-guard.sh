#!/usr/bin/env bash
# Deterministic linked-worktree preflight + creation guard.
# Installed to ~/.autospec/scripts/ by install.sh's copy_repo_scripts glob.
# Exit codes: 0 ok, 2 usage/non-git, 3 primary checkout, 4 dirty/reuse refusal,
# 5 stale base, 6 wrong branch, 7 mid-unit reset failure (the reset stopped at
# the failed step; nothing after it was attempted). `assert` can validate a
# branch glob or exact branch identity. resolve-branch emits
# {"state":"open-pr"|"branch-only"|"fresh","pr":N|null}.

set -eu

PROG="worktree-guard.sh"

usage() {
    cat <<'EOF'
Usage:
  worktree-guard.sh assert [--base <ref>] [--strict-base] [--branch-pattern <glob>] [--expected-branch <name>]
  worktree-guard.sh resolve-branch --branch <B> --repo <O/R>
  worktree-guard.sh resolve-base [--base <ref>] [--pr-base]
  worktree-guard.sh create --branch <B> [--base <ref>] [--path <P>] [--adopt]
  worktree-guard.sh reset --path <P> [--base <ref>] [--clean]

Subcommands:
  assert          Preflight the current directory. Exit 0 ok / 2 usage|non-git /
                  3 in_primary_checkout / 4 dirty / 5 stale_base / 6 wrong_branch.
  resolve-branch  PR-aware ladder verdict as JSON on stdout (exit 0 always):
                  {"state":"open-pr"|"branch-only"|"fresh","pr":N|null}.
  resolve-base    Emit the base ref selected by --base / env / config / default.
                  With --pr-base, emit the branch name for gh pr create --base.
  create          Create or verified-clean-reuse a fresh worktree off the base.
                  Dirty or wrong-branch reuse is refused (exit 4).
  reset           Park a linked worktree DETACHED at the base tip as one guarded
                  unit (issue #3653): never names a shared branch, checks every
                  step, asserts HEAD state after each step. --clean discards
                  local changes (refused with exit 4 without it).

Base selection:
  --base <ref> wins. Otherwise AUTOSPEC_BASE_BRANCH wins. Otherwise
  .autospec/autospec.yml git.base_branch wins. Otherwise origin/main is used,
  falling back to gh repo view's defaultBranchRef only when origin/main is absent.

Exit codes: 0 ok, 2 usage/non-git, 3 in_primary_checkout, 4 dirty, 5 stale_base,
6 wrong_branch, 7 mid-unit reset failure.
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

repo_root() {
    git rev-parse --show-toplevel 2>/dev/null || pwd
}

qualify_base_ref() {
    # Accept either a full remote ref (origin/master_ai) or a plain branch name
    # (master_ai). Plain branch names are intentionally interpreted as origin/*
    # because worktree-guard compares/fetches against the remote base.
    local ref="$1"
    case "$ref" in
        refs/heads/*)   printf 'origin/%s\n' "${ref#refs/heads/}" ;;
        refs/remotes/*) printf '%s\n' "${ref#refs/remotes/}" ;;
        */*)
            local remote_name="${ref%%/*}"
            if git remote 2>/dev/null | grep -Fx "$remote_name" >/dev/null 2>&1; then
                printf '%s\n' "$ref"
            else
                printf 'origin/%s\n' "$ref"
            fi
            ;;
        *) printf 'origin/%s\n' "$ref" ;;
    esac
}

autospec_config_base_branch() {
    local root config
    root="$(repo_root)"
    config="$root/.autospec/autospec.yml"
    [ -f "$config" ] || return 1
    awk '
        function clean(value) {
            sub(/[[:space:]]*#.*/, "", value)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", value)
            gsub(/^["'\'']|["'\'']$/, "", value)
            return value
        }
        /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
        /^[[:space:]]*git[.]base_branch[[:space:]]*:/ {
            sub(/^[^:]*:[[:space:]]*/, "", $0)
            value=clean($0)
            if (value != "") { print value; exit }
        }
        /^[^[:space:]][^:]*:/ {
            in_git = ($0 ~ /^git[[:space:]]*:/)
            next
        }
        in_git && /^[[:space:]]+base_branch[[:space:]]*:/ {
            sub(/^[[:space:]]*base_branch[[:space:]]*:[[:space:]]*/, "", $0)
            value=clean($0)
            if (value != "") { print value; exit }
        }
    ' "$config"
}

pr_base_from_ref() {
    local ref="$1"
    case "$ref" in
        refs/heads/*) printf '%s\n' "${ref#refs/heads/}" ;;
        refs/remotes/*/*) printf '%s\n' "${ref#refs/remotes/*/}" ;;
        */*)
            local remote_name="${ref%%/*}"
            if git remote 2>/dev/null | grep -Fx "$remote_name" >/dev/null 2>&1; then
                printf '%s\n' "${ref#*/}"
            else
                printf '%s\n' "$ref"
            fi
            ;;
        *) printf '%s\n' "$ref" ;;
    esac
}

gh_default_base_ref() {
    local branch
    branch="$(gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name // empty' 2>/dev/null || true)"
    [ -n "$branch" ] || return 1
    qualify_base_ref "$branch"
}

resolve_base_ref() {
    # resolve_base_ref <cli-base> <cli-explicit>
    local cli_base="$1" cli_explicit="$2" configured=0 base=""
    if [ "$cli_explicit" -eq 1 ]; then
        configured=1
        base="$cli_base"
    elif [ -n "${AUTOSPEC_BASE_BRANCH:-}" ]; then
        configured=1
        base="$AUTOSPEC_BASE_BRANCH"
    else
        base="$(autospec_config_base_branch 2>/dev/null || true)"
        if [ -n "$base" ]; then
            configured=1
        else
            base="origin/main"
        fi
    fi

    base="$(qualify_base_ref "$base")"

    if [ "$configured" -eq 0 ] && ! git rev-parse --verify "$base^{commit}" >/dev/null 2>&1; then
        gh_default_base_ref 2>/dev/null || printf '%s\n' "$base"
    else
        printf '%s\n' "$base"
    fi
}

# ---------------------------------------------------------------------------
# assert
# ---------------------------------------------------------------------------
cmd_assert() {
    local base=""
    local base_explicit=0
    local strict_base=0
    local branch_pattern=""
    local expected_branch=""

    while [ $# -gt 0 ]; do
        case "$1" in
            --base)        [ $# -ge 2 ] || die 2 "--base requires a value"; base="$2"; base_explicit=1; shift 2 ;;
            --strict-base) strict_base=1; shift ;;
            --branch-pattern) [ $# -ge 2 ] || die 2 "--branch-pattern requires a value"; branch_pattern="$2"; shift 2 ;;
            --expected-branch) [ $# -ge 2 ] || die 2 "--expected-branch requires a value"; expected_branch="$2"; shift 2 ;;
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

    base="$(resolve_base_ref "$base" "$base_explicit")"

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

    if [ -n "$branch_pattern" ] || [ -n "$expected_branch" ]; then
        local current_branch
        current_branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
        if [ -n "$expected_branch" ] && [ "$current_branch" != "$expected_branch" ]; then
            emit "code_health:wrong_branch branch=$current_branch want=$expected_branch"
            die 6 "assert: current branch '$current_branch' does not equal expected branch '$expected_branch'"
        fi
    fi

    if [ -n "$branch_pattern" ]; then
        case "$current_branch" in
            $branch_pattern) : ;;
            *)
                emit "code_health:wrong_branch branch=$current_branch want=$branch_pattern"
                die 6 "assert: current branch '$current_branch' does not match required pattern '$branch_pattern'"
                ;;
        esac
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
# resolve-base
# ---------------------------------------------------------------------------
cmd_resolve_base() {
    local base="" base_explicit=0 pr_base=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --base)    [ $# -ge 2 ] || die 2 "--base requires a value"; base="$2"; base_explicit=1; shift 2 ;;
            --pr-base) pr_base=1; shift ;;
            -h|--help) usage; exit 0 ;;
            *)         die 2 "resolve-base: unknown arg: $1" ;;
        esac
    done

    base="$(resolve_base_ref "$base" "$base_explicit")"
    if [ "$pr_base" -eq 1 ]; then
        pr_base_from_ref "$base"
    else
        printf '%s\n' "$base"
    fi
    exit 0
}

# ---------------------------------------------------------------------------
# create
# ---------------------------------------------------------------------------
cmd_create() {
    local branch="" base="" path="" adopt=0 base_explicit=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --branch)  [ $# -ge 2 ] || die 2 "--branch requires a value"; branch="$2"; shift 2 ;;
            --base)    [ $# -ge 2 ] || die 2 "--base requires a value"; base="$2"; base_explicit=1; shift 2 ;;
            --path)    [ $# -ge 2 ] || die 2 "--path requires a value"; path="$2"; shift 2 ;;
            --adopt)   adopt=1; shift ;;
            -h|--help) usage; exit 0 ;;
            *)         die 2 "create: unknown arg: $1" ;;
        esac
    done
    [ -n "$branch" ] || die 2 "create: --branch is required"
    [ -n "$path" ]   || path="/tmp/wt-${branch//\//-}"

    # fetch-before-branch (G4) with a single retry; surface on persistent failure.
    if ! git fetch origin >/dev/null 2>&1; then
        if ! git fetch origin >/dev/null 2>&1; then
            emit "code_health:fetch_failed"
            die 2 "create: git fetch origin failed (after retry)"
        fi
    fi
    base="$(resolve_base_ref "$base" "$base_explicit")"

    # Existing path: reuse ONLY if clean AND on the same branch AND it is a
    # linked worktree (NOT the primary checkout). Reusing the primary checkout
    # would let `create --branch main --path <primary>` pass while `assert` (the
    # very next preflight) rejects the same dir with exit 3 — the guard's core
    # safety property bypassed. So refuse primary-checkout reuse too.
    if [ -e "$path" ]; then
        local existing_branch existing_porcelain p_git_dir p_common_dir
        existing_branch="$(git -C "$path" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
        existing_porcelain="$(git -C "$path" status --porcelain 2>/dev/null || true)"
        p_git_dir="$(git -C "$path" rev-parse --git-dir 2>/dev/null || true)"
        p_common_dir="$(git -C "$path" rev-parse --git-common-dir 2>/dev/null || true)"
        # Normalise to absolute paths (git may report them relative to $path).
        if [ -n "$p_git_dir" ]; then
            p_git_dir="$(cd "$path" && cd "$p_git_dir" 2>/dev/null && pwd -P)" || p_git_dir=""
        fi
        if [ -n "$p_common_dir" ]; then
            p_common_dir="$(cd "$path" && cd "$p_common_dir" 2>/dev/null && pwd -P)" || p_common_dir=""
        fi
        local is_linked_worktree=0
        if [ -n "$p_git_dir" ] && [ "$p_git_dir" != "$p_common_dir" ]; then
            is_linked_worktree=1
        fi
        if [ "$existing_branch" = "$branch" ] && [ -z "$existing_porcelain" ] \
                && [ "$is_linked_worktree" -eq 1 ]; then
            # Clean, same branch, genuine linked worktree -> idempotent reuse.
            exit 0
        fi
        emit "code_health:worktree_dirty_reuse_refused path=$path branch=$existing_branch want=$branch linked=$is_linked_worktree"
        die 4 "create: refusing to reuse path $path (dirty, wrong branch, or primary checkout); never silent-reuse"
    fi

    # Adopt an existing remote branch, or create a fresh branch off the base.
    if [ "$adopt" -eq 1 ]; then
        if git worktree list --porcelain | grep -Fx "branch refs/heads/$branch" >/dev/null 2>&1; then
            emit "code_health:worktree_adopt_checkout_failed"
            die 2 "create: branch $branch is already checked out in another worktree"
        fi
        if ! git worktree add "$path" "origin/$branch" >/dev/null 2>&1; then
            emit "code_health:worktree_create_failed"
            die 2 "create: git worktree add (adopt) failed for origin/$branch at $path"
        fi
        # Land on a local tracking branch named B, not detached at origin/B.
        # A failure here (e.g. B already checked out in another worktree) leaves
        # a detached HEAD that violates the "local tracking branch named B"
        # contract — surface it rather than swallowing.
        if ! git -C "$path" checkout -q -B "$branch" "origin/$branch" >/dev/null 2>&1; then
            emit "code_health:worktree_adopt_checkout_failed"
            die 2 "create: could not check out branch $branch in adopted worktree $path (already checked out elsewhere?)"
        fi
    else
        if ! git worktree add -b "$branch" "$path" "$base" >/dev/null 2>&1; then
            emit "code_health:worktree_create_failed"
            die 2 "create: git worktree add -b $branch off $base at $path failed"
        fi
    fi

    exit 0
}

# ---------------------------------------------------------------------------
# reset — issue #3653 lesson: guard the worktree reset as ONE unit
# ---------------------------------------------------------------------------
# The original unguarded sequence (checkout -q -f <branch> -> reset --hard
# origin/<branch> -> clean -qfd) kept running after the FIRST step failed (a
# sibling worktree held the branch) and moved a local branch off its base
# commit. reset() is one guarded unit: it parks the worktree with a DETACHED
# HEAD — a shared branch is never named, so no other worktree can hold it and
# no branch ref moves — it checks every step and stops at the first failure
# (exit 7), and it asserts HEAD state (detached, at the base tip) after each
# step. A failed step never lets a later step run.
assert_reset_state() {
    # assert_reset_state <path> <base_ref> — die 7 unless HEAD is detached at
    # the base tip. Called after each step of the reset unit.
    local path="$1" base_ref="$2" head_sha base_sha
    head_sha="$(git -C "$path" rev-parse --verify 'HEAD^{commit}' 2>/dev/null || true)"
    base_sha="$(git -C "$path" rev-parse --verify "${base_ref}^{commit}" 2>/dev/null || true)"
    if [ -z "$head_sha" ] || [ -z "$base_sha" ] || [ "$head_sha" != "$base_sha" ]; then
        emit "code_health:reset_state_mismatch head=$head_sha base=$base_sha"
        die 7 "reset: HEAD is not at the $base_ref tip after a step; stopping"
    fi
    if git -C "$path" symbolic-ref -q HEAD >/dev/null 2>&1; then
        emit "code_health:reset_not_detached"
        die 7 "reset: HEAD is not detached after a step; stopping"
    fi
}

reset_preflight() {
    # reset_preflight <path> <clean> — refuse non-worktrees (2), the primary
    # checkout (3), and a dirty tree without --clean (4). Runs BEFORE any
    # mutation so a refused reset leaves the worktree exactly where it was.
    local path="$1" clean="$2"
    local git_dir common_dir abs_git_dir abs_common_dir porcelain
    git_dir="$(git -C "$path" rev-parse --git-dir 2>/dev/null)" \
        || die 2 "reset: not a git worktree: $path"
    common_dir="$(cd "$path" && git rev-parse --git-common-dir 2>/dev/null)" \
        || die 2 "reset: cannot resolve --git-common-dir: $path"
    abs_git_dir="$(cd "$path" && cd "$git_dir" 2>/dev/null && pwd -P)" || abs_git_dir="$git_dir"
    abs_common_dir="$(cd "$path" && cd "$common_dir" 2>/dev/null && pwd -P)" || abs_common_dir="$common_dir"
    if [ "$abs_git_dir" = "$abs_common_dir" ]; then
        emit "code_health:in_primary_checkout"
        die 3 "reset: refusing the primary checkout: $path"
    fi
    # A reset discards local work — require an explicit --clean for that.
    porcelain="$(git -C "$path" status --porcelain 2>/dev/null || true)"
    if [ -n "$porcelain" ] && [ "$clean" -eq 0 ]; then
        emit "code_health:reset_dirty_refused"
        die 4 "reset: worktree is dirty (pass --clean to discard): $path"
    fi
}

cmd_reset() {
    local path="" base="" base_explicit=0 clean=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --path)  [ $# -ge 2 ] || die 2 "--path requires a value"; path="$2"; shift 2 ;;
            --base)  [ $# -ge 2 ] || die 2 "--base requires a value"; base="$2"; base_explicit=1; shift 2 ;;
            --clean) clean=1; shift ;;
            -h|--help) usage; exit 0 ;;
            *)       die 2 "reset: unknown arg: $1" ;;
        esac
    done
    [ -n "$path" ] || die 2 "reset: --path is required"
    [ -d "$path" ] || die 2 "reset: not a directory: $path"
    reset_preflight "$path" "$clean"

    # Resolve the base inside the target repo, then fetch it (retry once).
    cd "$path"
    local base_ref
    base_ref="$(resolve_base_ref "$base" "$base_explicit")"
    if ! git fetch origin >/dev/null 2>&1; then
        if ! git fetch origin >/dev/null 2>&1; then
            emit "code_health:fetch_failed"
            die 5 "reset: git fetch origin failed (after retry)"
        fi
    fi
    git rev-parse --verify "${base_ref}^{commit}" >/dev/null 2>&1 \
        || die 5 "reset: base ref does not resolve: $base_ref"

    # Step 1: detach at the base tip. -f only when --clean, so a plain reset
    # can never silently discard tracked changes. On failure, stop: step 2
    # (clean) is NOT attempted.
    if [ "$clean" -eq 1 ]; then
        git -C "$path" checkout -q -f --detach "$base_ref" >/dev/null 2>&1 \
            || { emit "code_health:reset_checkout_failed"; die 7 "reset: detached checkout failed; later steps were NOT attempted: $path"; }
    else
        git -C "$path" checkout -q --detach "$base_ref" >/dev/null 2>&1 \
            || { emit "code_health:reset_checkout_failed"; die 7 "reset: detached checkout failed; later steps were NOT attempted: $path"; }
    fi
    assert_reset_state "$path" "$base_ref"

    if [ "$clean" -eq 1 ]; then
        # Step 2 (opt-in): discard untracked files, then re-assert state.
        git -C "$path" clean -qfd >/dev/null 2>&1 \
            || { emit "code_health:reset_clean_failed"; die 7 "reset: clean failed; worktree left detached at base: $path"; }
        assert_reset_state "$path" "$base_ref"
        [ -z "$(git -C "$path" status --porcelain 2>/dev/null || true)" ] \
            || { emit "code_health:reset_tree_not_clean"; die 7 "reset: tree is not clean after clean: $path"; }
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
        resolve-base)   cmd_resolve_base "$@" ;;
        create)         cmd_create "$@" ;;
        reset)          cmd_reset "$@" ;;
        -h|--help)      usage; exit 0 ;;
        *)              echo "$PROG: unknown subcommand: $sub" >&2; usage >&2; exit 2 ;;
    esac
}

main "$@"
