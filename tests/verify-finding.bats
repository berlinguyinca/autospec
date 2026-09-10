#!/usr/bin/env bats
# tests/verify-finding.bats — tests for scripts/verify-finding.sh (issue #3492).
#
# Contract under test: every review finding must carry a `repro:` command.
# The script runs it inside the PR worktree and prints one of three verdicts:
#   reproduced          repro exited 0
#   could-not-reproduce repro exited non-zero  (a recorded result, exit 0)
#   no-repro-command    no `repro:` line       (not reportable, exit 4)
# Each verdict carries the repro command's exit status as evidence.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/verify-finding.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    # macOS: mktemp may return /var/... while `git rev-parse --show-toplevel`
    # resolves symlinks to /private/var/...; compare like-for-like.
    TEST_TMP="$(cd "$TEST_TMP" && pwd -P)"
    WORKTREE="$TEST_TMP/wt"
    mkdir -p "$WORKTREE"
    (
        cd "$WORKTREE" || exit 1
        git init -q .
        git config user.email "t@t.t"
        git config user.name "t"
        printf 'fixture\n' > file.txt
        git add file.txt
        git commit -qm "init"
    )
    FINDING="$WORKTREE/finding.md"
}

teardown() {
    rm -rf "$TEST_TMP"
}

# Writes $1 as the repro command into a finding file at $FINDING.
make_finding() {
    {
        printf '# Finding: replay double-pays\n\n'
        printf 'severity: HIGH\n\n'
        printf 'repro: %s\n' "$1"
    } >"$FINDING"
}

@test "script is executable and has a usage block" {
    [ -x "$SCRIPT" ]
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"--finding-file"* ]]
}

@test "repro command that reproduces prints reproduced and exits 0" {
    make_finding "true"
    run bash "$SCRIPT" --finding-file "$FINDING"
    [ "$status" -eq 0 ]
    [[ "$output" == *"verdict: reproduced"* ]]
}

@test "repro command that does not reproduce prints could-not-reproduce and exits 0" {
    make_finding "exit 7"
    run bash "$SCRIPT" --finding-file "$FINDING"
    [ "$status" -eq 0 ]
    [[ "$output" == *"verdict: could-not-reproduce"* ]]
}

@test "finding with no repro: line prints no-repro-command and exits 4" {
    printf '# Finding: replay double-pays\n\nseverity: HIGH\n\nno command here\n' >"$FINDING"
    run bash "$SCRIPT" --finding-file "$FINDING"
    [ "$status" -eq 4 ]
    [[ "$output" == *"verdict: no-repro-command"* ]]
}

@test "reproduced verdict carries the repro exit status and first output line as evidence" {
    make_finding "echo first-out-line; echo second-line; exit 0"
    run bash "$SCRIPT" --finding-file "$FINDING"
    [ "$status" -eq 0 ]
    [[ "$output" == *"repro-exit=0"* ]]
    [[ "$output" == *"repro-output-1=first-out-line"* ]]
    # Only the first output line is evidence; the command echo aside, exactly
    # one repro-output-1= line is printed.
    [ "$(printf '%s\n' "$output" | grep -c '^repro-output-1=')" -eq 1 ]
}

@test "could-not-reproduce verdict carries the non-zero repro exit status as evidence" {
    make_finding "echo boom >&2; exit 7"
    run bash "$SCRIPT" --finding-file "$FINDING"
    [ "$status" -eq 0 ]
    [[ "$output" == *"verdict: could-not-reproduce"* ]]
    [[ "$output" == *"repro-exit=7"* ]]
    [[ "$output" == *"boom"* ]]
}

@test "no-repro-command verdict records that there was no exit status" {
    printf 'severity: HIGH\n' >"$FINDING"
    run bash "$SCRIPT" --finding-file "$FINDING"
    [ "$status" -eq 4 ]
    [[ "$output" == *"repro-exit=n/a"* ]]
}

@test "the repro command runs inside the PR worktree, not the caller cwd" {
    make_finding "pwd"
    run bash "$SCRIPT" --finding-file "$FINDING"
    [ "$status" -eq 0 ]
    [[ "$output" == *"repro-output-1=$WORKTREE"* ]]
}

@test "the repro command runs inside the --worktree directory" {
    other="$TEST_TMP/other"
    mkdir -p "$other"
    (
        cd "$other" || exit 1
        git init -q .
        git config user.email "t@t.t"
        git config user.name "t"
        printf 'x\n' > f
        git add f
        git commit -qm init
    )
    make_finding "pwd"
    run bash "$SCRIPT" --finding-file "$FINDING" --worktree "$other"
    [ "$status" -eq 0 ]
    [[ "$output" == *"repro-output-1=$other"* ]]
}

@test "the repro command keeps the repository working tree as its cwd from a subdirectory" {
    make_finding "pwd"
    mkdir -p "$WORKTREE/sub"
    run bash -c 'cd "$1/sub" && bash "$2" --finding-file "$3"' _ "$WORKTREE" "$SCRIPT" "$FINDING"
    [ "$status" -eq 0 ]
    [[ "$output" == *"repro-output-1=$WORKTREE"* ]]
}

@test "no git worktree: refuses to run the repro command and exits 3" {
    nongit="$TEST_TMP/nogit"
    mkdir -p "$nongit"
    printf 'repro: pwd\n' >"$nongit/finding.md"
    run bash -c 'cd "$1" && bash "$2" --finding-file "$3/finding.md"' _ "$nongit" "$SCRIPT" "$nongit"
    [ "$status" -eq 3 ]
    [[ "$output" != *"repro-output-1="* ]]
    [[ "$output" == *"worktree"* ]]
}

@test "--worktree pointing outside a git worktree is refused with exit 3" {
    make_finding "pwd"
    run bash "$SCRIPT" --finding-file "$FINDING" --worktree "$TEST_TMP"
    [ "$status" -eq 3 ]
    [[ "$output" != *"repro-output-1="* ]]
}

@test "missing finding file is a usage error (exit 64)" {
    run bash "$SCRIPT" --finding-file "$WORKTREE/absent.md"
    [ "$status" -eq 64 ]
    [[ "$output" == *"finding file not found"* ]]
}

@test "missing --finding-file is a usage error (exit 64)" {
    run bash "$SCRIPT"
    [ "$status" -eq 64 ]
    [[ "$output" == *"--finding-file"* ]]
}

@test "unknown option is a usage error (exit 64)" {
    make_finding "true"
    run bash "$SCRIPT" --finding-file "$FINDING" --bogus
    [ "$status" -eq 64 ]
}

@test "a backtick-quoted repro: line is unwrapped before running" {
    printf 'repro: `echo quoted-ok`\n' >"$FINDING"
    run bash "$SCRIPT" --finding-file "$FINDING"
    [ "$status" -eq 0 ]
    [[ "$output" == *"verdict: reproduced"* ]]
    [[ "$output" == *"repro-output-1=quoted-ok"* ]]
}
