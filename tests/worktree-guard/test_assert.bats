#!/usr/bin/env bats
# tests/worktree-guard/test_assert.bats — `worktree-guard.sh assert` exit codes.
#
# Covers (docs/specs/2026-06-03-worktree-guard-design.md §D1, issue #959 Shared
# contracts): the preflight `assert` returns each pinned exit code and each is
# tested here:
#   0  ok            — linked worktree, clean tree, HEAD == origin/main
#   2  usage         — bad subcommand / not a git dir
#   3  in_primary_checkout — git-dir == git-common-dir (cwd is main checkout)
#   4  dirty         — `git status --porcelain` non-empty (untracked included)
#   5  stale_base    — HEAD != origin/main after fetch (warn-level by default,
#                      fail under --strict-base)
#   6  wrong_branch  — current branch violates --branch-pattern or --expected-branch
#
# `assert` runs against REAL git fixture repos (a bare "origin", a primary
# checkout, and a linked worktree), so git-dir/git-common-dir detection is
# exercised for real rather than mocked. Network `git fetch` is neutralised by
# pointing `origin` at a local bare repo.

ROOT="${BATS_TEST_DIRNAME}/../.."
GUARD="$ROOT/scripts/worktree-guard.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" \
           GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"

    # Bare "remote" that origin points at — `git fetch origin` is local, no net.
    ORIGIN="$TEST_TMP/origin.git"
    git init -q --bare "$ORIGIN"

    # Primary checkout cloned from the bare remote, with one commit on main.
    PRIMARY="$TEST_TMP/primary"
    git clone -q "$ORIGIN" "$PRIMARY"
    git -C "$PRIMARY" checkout -q -b main 2>/dev/null || git -C "$PRIMARY" checkout -q main
    echo seed > "$PRIMARY/seed.txt"
    git -C "$PRIMARY" add seed.txt
    git -C "$PRIMARY" commit -q -m "seed"
    git -C "$PRIMARY" push -q -u origin main
}

teardown() { rm -rf "$TEST_TMP"; }

# Add a linked worktree off origin/main and echo its path.
mk_worktree() {
    local wt="$TEST_TMP/wt"
    git -C "$PRIMARY" worktree add -q -b feat/x "$wt" main >/dev/null 2>&1
    printf '%s' "$wt"
}

@test "assert: exit 0 in a clean linked worktree at origin/main" {
    wt="$(mk_worktree)"
    run bash -c "cd '$wt' && bash '$GUARD' assert"
    [ "$status" -eq 0 ]
}

@test "assert: exit 3 in_primary_checkout (git-dir == git-common-dir)" {
    run bash -c "cd '$PRIMARY' && bash '$GUARD' assert"
    [ "$status" -eq 3 ]
    [[ "$output" == *in_primary_checkout* ]]
}

@test "assert: exit 4 dirty when tracked file modified" {
    wt="$(mk_worktree)"
    echo change >> "$wt/seed.txt"
    run bash -c "cd '$wt' && bash '$GUARD' assert"
    [ "$status" -eq 4 ]
    [[ "$output" == *dirty* ]]
}

@test "assert: exit 4 dirty when an untracked file is present" {
    wt="$(mk_worktree)"
    echo new > "$wt/untracked.txt"
    run bash -c "cd '$wt' && bash '$GUARD' assert"
    [ "$status" -eq 4 ]
    [[ "$output" == *dirty* ]]
}

@test "assert: exit 5 stale_base under --strict-base when HEAD behind origin/main" {
    wt="$(mk_worktree)"
    # Advance origin/main beyond the worktree's HEAD.
    echo more >> "$PRIMARY/seed.txt"
    git -C "$PRIMARY" commit -q -am "advance"
    git -C "$PRIMARY" push -q origin main
    run bash -c "cd '$wt' && bash '$GUARD' assert --strict-base"
    [ "$status" -eq 5 ]
    [[ "$output" == *stale_base* ]]
}

@test "assert: stale base is warn-level (exit 0) without --strict-base" {
    wt="$(mk_worktree)"
    echo more >> "$PRIMARY/seed.txt"
    git -C "$PRIMARY" commit -q -am "advance"
    git -C "$PRIMARY" push -q origin main
    run bash -c "cd '$wt' && bash '$GUARD' assert"
    [ "$status" -eq 0 ]
    [[ "$output" == *stale_base* ]]
}

@test "assert: exit 2 on unknown subcommand" {
    run bash "$GUARD" bogus-subcommand
    [ "$status" -eq 2 ]
}

@test "assert: exit 2 when not inside a git repository" {
    run bash -c "cd '$TEST_TMP' && bash '$GUARD' assert"
    [ "$status" -eq 2 ]
}

@test "assert: cannot be fooled into passing from the primary checkout via a symlinked path" {
    # Adversarial: a symlink to the primary checkout must still be detected as
    # the primary checkout (pwd -P resolves it). Otherwise an agent could dodge
    # the exit-3 guard by cd-ing through a symlink.
    link="$TEST_TMP/primary-link"
    ln -s "$PRIMARY" "$link"
    run bash -c "cd '$link' && bash '$GUARD' assert"
    [ "$status" -eq 3 ]
    [[ "$output" == *in_primary_checkout* ]]
}

@test "assert: a nested standalone git repo inside a worktree is refused (exit 3)" {
    # Adversarial: a fresh `git init` subdir is its OWN primary checkout
    # (git-dir == git-common-dir). Asserting from inside it must refuse — an
    # agent must never commit into an unrelated nested repo thinking it is the
    # task worktree.
    wt="$(mk_worktree)"
    git init -q "$wt/nested"
    run bash -c "cd '$wt/nested' && bash '$GUARD' assert"
    [ "$status" -eq 3 ]
    [[ "$output" == *in_primary_checkout* ]]
}

@test "assert: passes from a symlinked path to a clean linked worktree" {
    # The mirror of the primary-symlink case: a symlink to a genuine linked
    # worktree must still pass (the symlink resolves to the real worktree).
    wt="$(mk_worktree)"
    link="$TEST_TMP/wt-link"
    ln -s "$wt" "$link"
    run bash -c "cd '$link' && bash '$GUARD' assert"
    [ "$status" -eq 0 ]
}

@test "assert: --branch-pattern rejects main even from a linked worktree" {
    git -C "$PRIMARY" checkout -q -b operator
    git -C "$PRIMARY" worktree add -q "$TEST_TMP/main-wt" main >/dev/null 2>&1
    run bash -c "cd '$TEST_TMP/main-wt' && bash '$GUARD' assert --branch-pattern 'feat/*'"
    [ "$status" -eq 6 ]
    [[ "$output" == *wrong_branch* ]]
}

@test "assert: --branch-pattern accepts a feature branch" {
    wt="$(mk_worktree)"
    run bash -c "cd '$wt' && bash '$GUARD' assert --branch-pattern 'feat/*'"
    [ "$status" -eq 0 ]
}

@test "assert: --expected-branch rejects a different feature branch" {
    wt="$(mk_worktree)"
    run bash -c "cd '$wt' && bash '$GUARD' assert --expected-branch 'feat/issue-999'"
    [ "$status" -eq 6 ]
    [[ "$output" == *wrong_branch* ]]
}

@test "assert: --expected-branch accepts the exact feature branch" {
    wt="$(mk_worktree)"
    run bash -c "cd '$wt' && bash '$GUARD' assert --expected-branch 'feat/x'"
    [ "$status" -eq 0 ]
}

@test "assert: --help exits 0" {
    run bash "$GUARD" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *assert* ]]
    [[ "$output" == *resolve-branch* ]]
    [[ "$output" == *create* ]]
}
