#!/usr/bin/env bash
# BATS tests: worktree-guard.sh reset (issue #3653).
#
# The reset unit must:
#   - park the worktree DETACHED at the base tip — never name a shared branch;
#   - check every step and stop at the first failure (exit 7), never running a
#     later step after a failed one;
#   - assert HEAD state (detached, at the base tip) after each step;
#   - refuse the primary checkout (exit 3) and a dirty tree without --clean
#     (exit 4).
#
# Setup mirrors test_create.bats: bare origin + primary clone + one seed
# commit + linked worktrees created with plain `git worktree add`.

setup() {
    TEST_TMP="$(mktemp -d)"
    REPO="$TEST_TMP/origin.git"
    PRIMARY="$TEST_TMP/primary"
    GUARD="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/worktree-guard.sh"

    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"
    git init -q --bare "$REPO"
    git clone -q "$REPO" "$PRIMARY"
    # Create main from the (unborn) clone HEAD — a rename of the current
    # branch is not portable across git versions, so none is done here.
    git -C "$PRIMARY" checkout -q -b main 2>/dev/null || git -C "$PRIMARY" checkout -q main
    echo base > "$PRIMARY/seed.txt"
    git -C "$PRIMARY" add seed.txt
    git -C "$PRIMARY" commit -q -m base
    git -C "$PRIMARY" push -q -u origin main

    # These tests resolve the base through origin/main explicitly; skip on
    # remotes whose default branch is not main.
    if ! git -C "$PRIMARY" show-ref --verify -q refs/remotes/origin/main; then
        skip "origin/main absent (default branch not 'main')"
    fi
}

teardown() {
    [ -n "${TEST_TMP:-}" ] && rm -rf "$TEST_TMP"
}

new_wt() {
    # new_wt <branch> <path> — a clean linked worktree at the base tip.
    git -C "$PRIMARY" worktree add -q -b "$1" "$2" origin/main
}

wt_head_sha() {
    git -C "$1" rev-parse HEAD 2>/dev/null
}

@test "reset: clean worktree parks DETACHED at the base tip; no branch ref moves" {
    wt="$TEST_TMP/wt"
    new_wt feat/r "$wt"
    before_main="$(git -C "$PRIMARY" rev-parse refs/heads/main)"
    before_feat="$(git -C "$PRIMARY" rev-parse refs/heads/feat/r)"

    run bash -c "cd '$PRIMARY' && bash '$GUARD' reset --path '$wt'"
    [ "$status" -eq 0 ]

    # HEAD is detached (no symbolic ref) and at the origin/main tip.
    run bash -c "cd '$wt' && git symbolic-ref -q HEAD"
    [ "$status" -ne 0 ]
    run bash -c "cd '$wt' && git rev-parse HEAD"
    head_sha="$output"
    run bash -c "cd '$PRIMARY' && git rev-parse refs/remotes/origin/main"
    [ "$head_sha" = "$output" ]

    # No branch ref moved; the worktree branch still exists as a ref.
    [ "$(git -C "$PRIMARY" rev-parse refs/heads/main)" = "$before_main" ]
    [ "$(git -C "$PRIMARY" rev-parse refs/heads/feat/r)" = "$before_feat" ]
    git -C "$PRIMARY" show-ref --verify -q refs/heads/feat/r
}

@test "reset: refuses the primary checkout (exit 3)" {
    run bash -c "cd '$PRIMARY' && bash '$GUARD' reset --path '$PRIMARY'"
    [ "$status" -eq 3 ]
}

@test "reset: refuses a dirty worktree without --clean (exit 4 + identifier)" {
    wt="$TEST_TMP/wt"
    new_wt feat/r "$wt"
    echo dirty >> "$wt/seed.txt"

    run bash -c "cd '$PRIMARY' && bash '$GUARD' reset --path '$wt'"
    [ "$status" -eq 4 ]
    [[ "$output" == *reset_dirty_refused* ]]

    # The tree was left untouched.
    [ "$(cat "$wt/seed.txt")" = "$(printf 'base\ndirty\n')" ]
}

@test "reset --clean: discards tracked changes and untracked files; ends clean and detached" {
    wt="$TEST_TMP/wt"
    new_wt feat/r "$wt"
    echo dirty >> "$wt/seed.txt"
    echo scratch > "$wt/scratch.txt"
    base_seed="$(git -C "$PRIMARY" show origin/main:seed.txt)"

    run bash -c "cd '$PRIMARY' && bash '$GUARD' reset --path '$wt' --clean"
    [ "$status" -eq 0 ]

    [ ! -e "$wt/scratch.txt" ]
    [ "$(cat "$wt/seed.txt")" = "$base_seed" ]
    run bash -c "cd '$wt' && git status --porcelain"
    [ -z "$output" ]
    run bash -c "cd '$wt' && git symbolic-ref -q HEAD"
    [ "$status" -ne 0 ]
    run bash -c "cd '$wt' && git rev-parse HEAD"
    head_sha="$output"
    run bash -c "cd '$PRIMARY' && git rev-parse refs/remotes/origin/main"
    [ "$head_sha" = "$output" ]
}

@test "reset: succeeds when another worktree holds the base branch and does not move it (#3653 regression)" {
    # The unguarded `checkout -q -f main` failed in this shape (sibling
    # worktree held main) and the old sequence plowed on, moving a local
    # branch off its base. The detached park must not fail here at all.
    wt1="$TEST_TMP/wt1"
    wt2="$TEST_TMP/wt2"
    # Free the local main ref from the primary, then park it in wt1.
    git -C "$PRIMARY" checkout -q -b dev
    git -C "$PRIMARY" worktree add -q "$wt1" main
    new_wt feat/other "$wt2"

    main_before="$(git -C "$PRIMARY" rev-parse refs/heads/main)"

    run bash -c "cd '$PRIMARY' && bash '$GUARD' reset --path '$wt2' --base main"
    [ "$status" -eq 0 ]

    # wt2 is detached at the base tip.
    run bash -c "cd '$wt2' && git symbolic-ref -q HEAD"
    [ "$status" -ne 0 ]
    run bash -c "cd '$wt2' && git rev-parse HEAD"
    head_sha="$output"
    run bash -c "cd '$PRIMARY' && git rev-parse refs/remotes/origin/main"
    [ "$head_sha" = "$output" ]

    # wt1 still holds branch main, and the main ref did not move.
    run bash -c "cd '$wt1' && git rev-parse --abbrev-ref HEAD"
    [ "$output" = "main" ]
    [ "$(git -C "$PRIMARY" rev-parse refs/heads/main)" = "$main_before" ]
}

@test "reset: unknown base ref is exit 5" {
    wt="$TEST_TMP/wt"
    new_wt feat/r "$wt"
    run bash -c "cd '$PRIMARY' && bash '$GUARD' reset --path '$wt' --base definitely-missing"
    [ "$status" -eq 5 ]
    # The worktree was left where it was: still on its branch, tree clean.
    run bash -c "cd '$wt' && git rev-parse --abbrev-ref HEAD"
    [ "$output" = "feat/r" ]
}

@test "reset: missing --path is a usage error (exit 2)" {
    run bash -c "cd '$PRIMARY' && bash '$GUARD' reset"
    [ "$status" -eq 2 ]
}

@test "reset: re-running on an already-detached worktree is idempotent" {
    wt="$TEST_TMP/wt"
    new_wt feat/r "$wt"
    run bash -c "cd '$PRIMARY' && bash '$GUARD' reset --path '$wt'"
    [ "$status" -eq 0 ]
    run bash -c "cd '$PRIMARY' && bash '$GUARD' reset --path '$wt'"
    [ "$status" -eq 0 ]
    run bash -c "cd '$wt' && git symbolic-ref -q HEAD"
    [ "$status" -ne 0 ]
    run bash -c "cd '$wt' && git rev-parse HEAD"
    head_sha="$output"
    run bash -c "cd '$PRIMARY' && git rev-parse refs/remotes/origin/main"
    [ "$head_sha" = "$output" ]
}
