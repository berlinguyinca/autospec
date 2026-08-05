#!/usr/bin/env bats
# tests/unit/test_lint_gates_ratchet.bats — scripts/lint-implementation-gates.sh
#
# The ratchet replaces the absolute file-LOC rule: an oversized file may be edited
# as long as the change does not make it longer. See
# docs/superpowers/specs/2026-08-05-lint-gate-satisfiability-design.md §1.1 — the
# absolute rule made oversized files unfixable, because COMPLEXITY judges every
# intermediate commit on absolute size while PR_SIZE caps a commit at 400 changed
# lines.

bats_require_minimum_version 1.5.0

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    GATES="$REPO_ROOT/scripts/lint-implementation-gates.sh"
    WORK="$(mktemp -d)"
    cd "$WORK"
    git init -q .
    git config user.email t@example.com
    git config user.name Test
    printf 'seed\n' > seed.txt
    git add seed.txt
    git commit -q -m seed
}

teardown() {
    cd /
    rm -rf "${WORK:?}"
}

# Writes a file of $2 lines at path $1.
write_lines() {
    python3 -c "
import sys
open(sys.argv[1], 'w').write(''.join('line %d\n' % i for i in range(int(sys.argv[2]))))
" "$1" "$2"
}

@test "ratchet: a small file is unaffected" {
    write_lines small.py 20
    git add small.py
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'file is '
}

@test "ratchet: a brand new oversized file is still rejected" {
    write_lines huge.py 500
    git add huge.py
    run bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'COMPLEXITY:huge.py'
    printf '%s\n' "$output" | grep -q 'file is 500 LOC'
}

@test "ratchet: shrinking an oversized file is allowed — the case the old rule blocked" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 899
    git add big.py
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'file is '
}

@test "shrink exception: a large pure-removal change is not capped by PR_SIZE" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 380
    git add big.py
    run bash "$GATES" --staged
    # 520 removed lines is far past the 400 changed-line cap, but nothing grew.
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'PR_SIZE'
}

@test "shrink exception: removals plus a few added lines still count as a shrink" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    python3 -c "
lines = ['import helper\n'] * 6 + ['line %d\n' % i for i in range(500)]
open('big.py', 'w').writelines(lines)
"
    git add big.py
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'PR_SIZE'
}

@test "shrink exception: does not apply when any file grows" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 500
    write_lines feature.py 450
    git add big.py feature.py
    run bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'PR_SIZE'
}

@test "ratchet: growing an oversized file is rejected with both line counts" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 901
    git add big.py
    run bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'COMPLEXITY:big.py'
    printf '%s\n' "$output" | grep -q 'file is 901 LOC'
}

@test "ratchet: an oversized file held at the same length is allowed" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    python3 -c "
lines = open('big.py').read().splitlines()
lines[10] = 'changed'
open('big.py', 'w').write('\n'.join(lines) + '\n')
"
    git add big.py
    run bash "$GATES" --staged
    ! printf '%s\n' "$output" | grep -q 'file is '
}

@test "ratchet: bringing an oversized file under the limit is allowed" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 300
    git add big.py
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'file is '
}

@test "gates: the superseded absolute file-LOC finding is not passed through" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 899
    git add big.py
    run bash "$GATES" --staged
    # The delegated linter still emits its absolute finding; the wrapper drops it
    # because the file did not get longer.
    ! printf '%s\n' "$output" | grep -q 'file is 899 LOC'
    ! printf '%s\n' "$output" | grep -q 'AUTOSPEC_MAX_FILE_LOC'
}

@test "gates: documentation and data files are exempt, as before" {
    write_lines notes.md 900
    write_lines data.json 900
    git add notes.md data.json
    run bash "$GATES" --staged
    # PR_SIZE still fires on 1800 changed lines — that rule is not size-exempt by
    # extension and nothing shrank. Only the FILE_GROWTH ratchet skips these types.
    ! printf '%s\n' "$output" | grep -q 'file is '
}

@test "gates: findings from rules this wrapper does not replace still block" {
    # A leftover TODO rather than an injection pattern: the point is that a delegated
    # rule still blocks, and a test fixture should not carry a dangerous construct.
    printf 'def f():\n    pass  # TODO: finish this\n' > leftover.py
    git add leftover.py
    run bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'TODO_LEFT'
}
