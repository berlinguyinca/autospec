#!/usr/bin/env bats
# tests/worktree-guard/test_create.bats — `worktree-guard.sh create`.
#
# Covers (docs/specs/2026-06-03-worktree-guard-design.md §D1 `create`, issue
# #959 Shared contracts):
#   - fresh create:  `git worktree add -b B <path> origin/main`
#   - adopt create:  `git worktree add <path> origin/B` (branch-only recovery)
#   - idempotent reuse: existing path reused ONLY if clean AND same branch
#   - dirty-reuse refusal: dirty existing path -> exit 4
#     `code_health:worktree_dirty_reuse_refused`
#   - wrong-branch reuse refusal: clean but different branch -> exit 4
#   - fetch-failure: retry once then surface (non-zero)
#
# Uses REAL git fixture repos (bare origin + primary) so `git worktree add`
# behaves for real; `git fetch origin` is local (no network).

ROOT="${BATS_TEST_DIRNAME}/../.."
GUARD="$ROOT/scripts/worktree-guard.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" \
           GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"

    ORIGIN="$TEST_TMP/origin.git"
    git init -q --bare "$ORIGIN"

    PRIMARY="$TEST_TMP/primary"
    git clone -q "$ORIGIN" "$PRIMARY"
    git -C "$PRIMARY" checkout -q -b main 2>/dev/null || git -C "$PRIMARY" checkout -q main
    echo seed > "$PRIMARY/seed.txt"
    git -C "$PRIMARY" add seed.txt
    git -C "$PRIMARY" commit -q -m "seed"
    git -C "$PRIMARY" push -q -u origin main
}

teardown() { rm -rf "$TEST_TMP"; }

@test "create: fresh worktree off origin/main on a new branch" {
    wt="$TEST_TMP/wt-fresh"
    run bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/new --path '$wt'"
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [ "$(git -C "$wt" rev-parse --abbrev-ref HEAD)" = "feat/new" ]
}

@test "create: idempotent reuse of a clean same-branch worktree (exit 0)" {
    wt="$TEST_TMP/wt-idem"
    bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/idem --path '$wt'"
    run bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/idem --path '$wt'"
    [ "$status" -eq 0 ]
    [ "$(git -C "$wt" rev-parse --abbrev-ref HEAD)" = "feat/idem" ]
}

@test "create: refuses dirty reuse with exit 4 + code_health identifier" {
    wt="$TEST_TMP/wt-dirty"
    bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/dirty --path '$wt'"
    echo uncommitted > "$wt/scratch.txt"   # make it dirty
    run bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/dirty --path '$wt'"
    [ "$status" -eq 4 ]
    [[ "$output" == *worktree_dirty_reuse_refused* ]]
}

@test "create: refuses clean-but-wrong-branch reuse with exit 4" {
    wt="$TEST_TMP/wt-wrong"
    bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/aaa --path '$wt'"
    run bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/bbb --path '$wt'"
    [ "$status" -eq 4 ]
    [[ "$output" == *worktree_dirty_reuse_refused* ]]
}

@test "create: adopt mode checks out an existing remote branch" {
    # Publish a branch on origin, then adopt it.
    git -C "$PRIMARY" checkout -q -b feat/published
    echo work >> "$PRIMARY/seed.txt"
    git -C "$PRIMARY" commit -q -am "published work"
    git -C "$PRIMARY" push -q -u origin feat/published
    git -C "$PRIMARY" checkout -q main

    wt="$TEST_TMP/wt-adopt"
    run bash -c "cd '$PRIMARY' && bash '$GUARD' create --branch feat/published --adopt --path '$wt'"
    [ "$status" -eq 0 ]
    [ -d "$wt" ]
    [ "$(git -C "$wt" rev-parse --abbrev-ref HEAD)" = "feat/published" ]
    # The adopted branch carries its published commit (file gained the "work" line).
    grep -q "work" "$wt/seed.txt"
    [ "$(git -C "$wt" log -1 --pretty=%s)" = "published work" ]
}

@test "create: missing --branch is a usage error (exit 2)" {
    run bash -c "cd '$PRIMARY' && bash '$GUARD' create --path '$TEST_TMP/wt-x'"
    [ "$status" -eq 2 ]
}

@test "create: fetch failure surfaces non-zero after a retry" {
    # Point origin at a non-existent path so `git fetch origin` fails.
    bad="$TEST_TMP/bad-clone"
    git clone -q "$ORIGIN" "$bad"
    git -C "$bad" remote set-url origin "$TEST_TMP/does-not-exist.git"
    wt="$TEST_TMP/wt-fetchfail"
    run bash -c "cd '$bad' && bash '$GUARD' create --branch feat/z --path '$wt'"
    [ "$status" -ne 0 ]
    [[ "$output" == *fetch* ]]
}
