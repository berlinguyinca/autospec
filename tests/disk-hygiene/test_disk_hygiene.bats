#!/usr/bin/env bats
# tests/disk-hygiene/test_disk_hygiene.bats — scripts/disk-hygiene.sh and its
# wiring into dispatch-implementer.sh. Real git repos, real bare remotes, real
# worktrees and real directories in a per-test temp dir; no mocks.

ROOT="${BATS_TEST_DIRNAME}/../.."
DH="$ROOT/scripts/disk-hygiene.sh"
DISPATCH="$ROOT/scripts/dispatch-implementer.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export GIT_AUTHOR_NAME="t" GIT_AUTHOR_EMAIL="t@e" \
           GIT_COMMITTER_NAME="t" GIT_COMMITTER_EMAIL="t@e"
    unset CARGO_TARGET_DIR CARGO_INCREMENTAL AUTOSPEC_DISK_HYGIENE \
          AUTOSPEC_DISK_CLEANUP AUTOSPEC_DISK_PREFLIGHT
    export AUTOSPEC_HERDR_ENG="$TEST_TMP/no-such-herdr-eng"
    ORIGIN="$TEST_TMP/origin.git"
    PRIMARY="$TEST_TMP/primary"
    git init -q --bare "$ORIGIN"
    git clone -q "$ORIGIN" "$PRIMARY" 2>/dev/null
    git -C "$PRIMARY" checkout -q -b main
    printf '[package]\nname = "x"\nversion = "0.0.0"\n' > "$PRIMARY/Cargo.toml"
    echo seed > "$PRIMARY/seed.txt"
    echo "target/" > "$PRIMARY/.gitignore"
    git -C "$PRIMARY" add -A
    git -C "$PRIMARY" commit -q -m seed
    git -C "$PRIMARY" push -q -u origin main
    WT="$TEST_TMP/wt-a"
    git -C "$PRIMARY" worktree add -q -b feat/a "$WT" origin/main
    mkdir -p "$WT/target/debug"
    head -c 4096 /dev/zero > "$WT/target/debug/blob"
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "cleanup-worktree removes a clean, pushed worktree and its target dir" {
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 0 ]
    [ ! -e "$WT" ]
    [ ! -e "$WT/target" ]
    run git -C "$PRIMARY" worktree list --porcelain
    [[ "$output" != *"$WT"* ]]
}

@test "cleanup-worktree keeps a dirty worktree and logs why" {
    echo wip > "$WT/new.txt"
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 1 ]
    [ -d "$WT" ]
    [[ "$output" == *"disk_hygiene:worktree_kept"* ]]
    [[ "$output" == *"dirty"* ]]
}

@test "cleanup-worktree keeps a worktree with unpushed commits" {
    echo c > "$WT/c.txt"
    git -C "$WT" add c.txt
    git -C "$WT" commit -q -m unpushed
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 1 ]
    [ -d "$WT" ]
    [[ "$output" == *"unpushed"* ]]
}

@test "cleanup-worktree removes the worktree once its commits are pushed" {
    echo c > "$WT/c.txt"
    git -C "$WT" add c.txt
    git -C "$WT" commit -q -m pushed
    git -C "$WT" push -q origin feat/a
    run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 0 ]
    [ ! -e "$WT" ]
}

@test "cleanup-worktree --force removes a dirty worktree" {
    echo wip > "$WT/new.txt"
    run bash "$DH" cleanup-worktree --force "$WT"
    [ "$status" -eq 0 ]
    [ ! -e "$WT" ]
}

@test "cleanup-worktree is a no-op success when the path is already gone" {
    run bash "$DH" cleanup-worktree "$TEST_TMP/never-existed"
    [ "$status" -eq 0 ]
}

@test "cleanup-worktree refuses the primary checkout" {
    run bash "$DH" cleanup-worktree --force "$PRIMARY"
    [ "$status" -eq 2 ]
    [ -d "$PRIMARY/.git" ]
}

@test "cleanup-worktree honours AUTOSPEC_DISK_CLEANUP=off" {
    AUTOSPEC_DISK_CLEANUP=off run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 0 ]
    [ -d "$WT" ]
}

@test "env: Rust repo gets CARGO_INCREMENTAL=0 and never a shared CARGO_TARGET_DIR" {
    run bash "$DH" env "$WT"
    [ "$status" -eq 0 ]
    [[ "$output" == *"export CARGO_INCREMENTAL=0"* ]]
    [[ "$output" != *"CARGO_TARGET_DIR"* ]]
}

@test "env: non-Rust repo emits nothing" {
    git -C "$PRIMARY" rm -q Cargo.toml
    git -C "$PRIMARY" commit -q -m norust
    git -C "$PRIMARY" push -q origin main
    NR="$TEST_TMP/wt-nr"
    git -C "$PRIMARY" worktree add -q -b feat/nr "$NR" main
    run bash "$DH" env "$NR"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "env: a caller-provided CARGO_INCREMENTAL wins" {
    CARGO_INCREMENTAL=1 run bash "$DH" env "$WT"
    [ "$status" -eq 0 ]
    [[ "$output" != *"CARGO_INCREMENTAL"* ]]
}

@test "preflight passes with satisfiable thresholds" {
    AUTOSPEC_DISK_MIN_FREE_PCT=0 AUTOSPEC_DISK_MIN_FREE_GIB=0 run bash "$DH" preflight --path "$TEST_TMP"
    [ "$status" -eq 0 ]
    [[ "$output" == *"disk_hygiene:preflight_ok"* ]]
}

@test "preflight parks (exit 75) when free space is below the threshold" {
    AUTOSPEC_DISK_MIN_FREE_PCT=100 AUTOSPEC_DISK_MIN_FREE_GIB=99999999 run bash "$DH" preflight --path "$TEST_TMP"
    [ "$status" -eq 75 ]
    [[ "$output" == *"disk_hygiene:disk_low_parked"* ]]
}

@test "preflight is skipped with AUTOSPEC_DISK_PREFLIGHT=off" {
    AUTOSPEC_DISK_PREFLIGHT=off AUTOSPEC_DISK_MIN_FREE_PCT=100 AUTOSPEC_DISK_MIN_FREE_GIB=99999999 run bash "$DH" preflight --path "$TEST_TMP"
    [ "$status" -eq 0 ]
}

# The installed herdr-eng has no `cleanup` subcommand today, so the "herdr-eng
# does the removal" branch cannot be exercised without a stand-in binary (not
# allowed). What is tested for real: the contract probe against the REAL binary
# and the real fallback path; the herdr-eng-removes-it branch has no test.
@test "the real herdr-eng (when installed) is probed and cleanup-worktree still removes via git" {
    command -v herdr-eng >/dev/null 2>&1 || skip "herdr-eng not installed"
    AUTOSPEC_HERDR_ENG="$(command -v herdr-eng)" run bash -c '. "$0"; dh_herdr_has_cleanup && echo has || echo none' "$DH" 2>/dev/null
    [ "$status" -eq 0 ]
    AUTOSPEC_HERDR_ENG="$(command -v herdr-eng)" run bash "$DH" cleanup-worktree "$WT"
    [ "$status" -eq 0 ]
    [ ! -e "$WT" ]
}

@test "dispatch-implementer --cleanup keeps dirty work and removes clean worktrees" {
    cd "$PRIMARY"
    BR="dh-keep-$$-$RANDOM"
    run bash "$DISPATCH" --issue 1 --branch "$BR" --cleanup
    [ "$status" -eq 0 ]
    # The helper's worktree lives at /tmp/wt-<branch>; create + dirty it.
    git -C "$PRIMARY" worktree add -q -b "$BR" "/tmp/wt-$BR" origin/main
    echo wip > "/tmp/wt-$BR/wip.txt"
    run bash "$DISPATCH" --issue 1 --branch "$BR" --cleanup
    [ "$status" -eq 0 ]
    [ -d "/tmp/wt-$BR" ]
    rm "/tmp/wt-$BR/wip.txt"
    run bash "$DISPATCH" --issue 1 --branch "$BR" --cleanup
    [ "$status" -eq 0 ]
    [ ! -e "/tmp/wt-$BR" ]
}

@test "dispatch-implementer leaves no worktree behind when creation fails" {
    cd "$PRIMARY"
    BR="dh-fail-$$-$RANDOM"
    printf 'p\n' > "$TEST_TMP/prompt.md"
    run bash "$DISPATCH" --issue 1 --branch "$BR" --prompt-file "$TEST_TMP/prompt.md" --base origin/does-not-exist
    [ "$status" -ne 0 ]
    [ ! -e "/tmp/wt-$BR" ]
}

@test "worktree-guard create writes a git-excluded .autospec-env.sh for Rust repos" {
    cd "$PRIMARY"
    NEW="$TEST_TMP/wt-guard"
    run bash "$ROOT/scripts/worktree-guard.sh" create --branch feat/guard --path "$NEW" --base origin/main
    [ "$status" -eq 0 ]
    run grep -q "CARGO_TARGET_DIR" "$NEW/.autospec-env.sh"
    [ "$status" -ne 0 ]
    grep -q "CARGO_INCREMENTAL=0" "$NEW/.autospec-env.sh"
    [ -z "$(git -C "$NEW" status --porcelain)" ]
}
