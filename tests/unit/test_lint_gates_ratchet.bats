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
    write_lines huge.py 700
    git add huge.py
    run bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'COMPLEXITY:huge.py'
    printf '%s\n' "$output" | grep -q 'file is 700 LOC'
}

@test "threshold: the wrapper's limit is the one the delegate applies" {
    # The wrapper reads AUTOSPEC_MAX_FILE_LOC and defaults it to 600. If it does not
    # pass that through, the delegate falls back to its own 400 and a 500-line file
    # is rejected locally while the CI ratchet — which uses 600 — accepts it.
    write_lines mid.py 500
    git add mid.py
    run bash "$GATES" --staged
    # PR_SIZE still caps the 500 changed lines — that is a different rule, and this
    # test is about the size threshold only.
    ! printf '%s\n' "$output" | grep -q 'file is '
}

@test "threshold: an explicit AUTOSPEC_MAX_FILE_LOC still overrides the default" {
    write_lines mid.py 500
    git add mid.py
    run env AUTOSPEC_MAX_FILE_LOC=400 bash "$GATES" --staged
    [ "$status" -ne 0 ]
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

# The next three tests are about WHICH file the ratchet reports, not about whether a
# report blocks. COMPLEXITY is advisory unless AUTOSPEC_COMPLEXITY_ENFORCE=1 (design
# doc Fix 5), so they ask for enforcement rather than assuming it — otherwise a
# passing assertion would only be recording the default policy, and the ratchet's file
# selection would go untested. The default policy has its own tests below.
@test "ratchet: growing an oversized file is rejected with both line counts" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 901
    git add big.py
    # `run env VAR=1 …` rather than `VAR=1 run …`: the latter sets the variable for a
    # shell function without exporting it, so the linter subprocess never sees it.
    run env AUTOSPEC_COMPLEXITY_ENFORCE=1 bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q '^COMPLEXITY:big.py'
    printf '%s\n' "$output" | grep -q 'file is 901 LOC'
}

@test "policy: by default a grown oversized file is reported without blocking" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 901
    git add big.py
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^INFO:COMPLEXITY:big.py.*file is 901 LOC'
    # Growth still blocks where merges cannot skip it: .github/workflows/file-size-ratchet.yml
    # runs the same comparison against the merge base on every pull request.
    ! printf '%s\n' "$output" | grep -qE '^COMPLEXITY:'
}

@test "policy: a delegate that crashes is not reported as a clean gate" {
    write_lines small.py 20
    git add small.py
    # A delegate that exits outside its 0..64/200 contract has produced no findings, which
    # is not the same as having found none. Judging the run by stdout alone turned every
    # gate off at once when an earlier draft of the advisory routing recursed.
    printf '#!/usr/bin/env bash\nexit 139\n' > fake-delegate.sh
    chmod +x fake-delegate.sh
    mkdir -p fakescripts
    cp fake-delegate.sh fakescripts/lint-implementation.sh
    cp "$GATES" fakescripts/lint-implementation-gates.sh
    run bash fakescripts/lint-implementation-gates.sh --staged
    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q '^LINT_DELEGATE_FAILED:.*exited 139'
}

@test "delegate rc: a delegate whose count disagrees with its output is a broken contract" {
    write_lines small.py 20
    git add small.py
    # Tolerated for as long as the delegate undercounted itself — two rules emitted from
    # inside a pipeline, so their increments were lost to the subshell (#3080). #3081 restored
    # the contract, so a mismatch is a defect again and the wrapper says so rather than
    # guessing which of the two numbers to believe.
    printf '#!/usr/bin/env bash\nprintf "TODO_LEFT:a.py:1: first\\nTODO_LEFT:a.py:2: second\\n"\nexit 1\n' \
        > fake-delegate.sh
    chmod +x fake-delegate.sh
    mkdir -p fakescripts
    cp fake-delegate.sh fakescripts/lint-implementation.sh
    cp "$GATES" fakescripts/lint-implementation-gates.sh
    run bash fakescripts/lint-implementation-gates.sh --staged
    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q '^LINT_DELEGATE_FAILED:.*exited 1'
}

@test "delegate rc: a count that matches its output is accepted" {
    write_lines small.py 20
    git add small.py
    printf '#!/usr/bin/env bash\nprintf "TODO_LEFT:a.py:1: first\\nTODO_LEFT:a.py:2: second\\n"\nexit 2\n' \
        > fake-delegate.sh
    chmod +x fake-delegate.sh
    mkdir -p fakescripts
    cp fake-delegate.sh fakescripts/lint-implementation.sh
    cp "$GATES" fakescripts/lint-implementation-gates.sh
    run bash fakescripts/lint-implementation-gates.sh --staged
    ! printf '%s\n' "$output" | grep -q 'LINT_DELEGATE_FAILED'
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^TODO_LEFT:a.py:2: second'
}

@test "delegate rc: enforcing complexity on a long file with a long function is not a crash" {
    # End to end against the real delegate, which is where this was found: the file is both
    # over the limit and holds one long function, so it prints two blocking COMPLEXITY
    # findings and exits 1.
    python3 -c "
open('big.py', 'w').write('def f():\n' + '    x = 1\n' * 700)
"
    git add big.py
    git commit -q -m "pre-existing oversized file"
    python3 -c "
lines = open('big.py').read().splitlines()
lines[10] = '    x = 2'
open('big.py', 'w').write('\n'.join(lines) + '\n')
"
    git add big.py
    run env AUTOSPEC_COMPLEXITY_ENFORCE=1 bash "$GATES" --staged
    ! printf '%s\n' "$output" | grep -q 'LINT_DELEGATE_FAILED'
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "^COMPLEXITY:big.py:.*AUTOSPEC_MAX_FUNC_LOC"
}

@test "policy: a low delegate error code without findings is not reported as clean" {
    write_lines small.py 20
    git add small.py
    run bash "$GATES" --staged --staged-base definitely-not-a-commit
    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q '^LINT_DELEGATE_FAILED:.*exited 1'
    printf '%s\n' "$output" | grep -q 'invalid staged base commit'
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

# ── logical units vs docs ─────────────────────────────────────────────────────
# DOC_OUT_OF_SYNC requires a public-surface change to touch a doc file; PR_SIZE then
# charged its three-unit cap for that file. A change at the limit failed the moment it
# was documented, so the two rules together had no solution.

@test "units: a doc file does not consume a logical unit" {
    for i in 1 2 3; do write_lines "src$i.py" 5; done
    mkdir -p docs
    write_lines docs/NOTES.md 5
    git add -A
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'exceeded=logical_units'
}

@test "units: four non-doc files still breach the cap" {
    for i in 1 2 3 4; do write_lines "src$i.py" 5; done
    mkdir -p docs
    write_lines docs/NOTES.md 5
    git add -A
    run bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'exceeded=logical_units'
}

@test "units: a skill trio collapses to one unit rather than vanishing" {
    # is_doc_file also matches skills/*/SKILL.md, so the trio case has to be handled
    # before the doc exclusion. Handled after, a trio would drop out of the count and
    # four real units would read as three.
    mkdir -p skills/demo/codex skills/demo/opencode
    write_lines skills/demo/SKILL.md 5
    write_lines skills/demo/codex/prompt.md 5
    write_lines skills/demo/opencode/agent.md 5
    for i in 1 2 3; do write_lines "src$i.py" 5; done
    git add -A
    run bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'exceeded=logical_units'
}

@test "units: the waiver does not leak into diff-file mode, which has no staged paths" {
    for i in 1 2 3 4; do write_lines "src$i.py" 5; done
    git add -A
    git diff --cached --output=change.diff
    run bash "$GATES" --diff-file change.diff
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'exceeded=logical_units'
}

# ── relocation ────────────────────────────────────────────────────────────────
# Extracting code out of a monolith into a sibling module is the remedy the size
# rule prescribes, and the CI ratchet's failure message recommends it by name. A
# split necessarily creates a file, so counting "any new file" as growth rejects the
# prescribed remedy — the §1.1 satisfiability defect, relocated to new files.

@test "relocation: extracting into a sibling module keeps the shrink waiver" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 300
    write_lines big_tests.py 590
    git add big.py big_tests.py
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'exceeded=changed_lines'
}

@test "relocation: module scaffolding and comments do not forfeit the waiver" {
    # Measured on the real thing: extracting the harness cluster out of executor_bridge.rs
    # moved 595 lines out and 626 in, a net +31 — and every one of those 31 was a comment, a
    # blank line, or a `mod`/`use` declaration. Net *code* lines: exactly 0.
    #
    # The rule's own rationale is that a feature cannot hide in a relocation because features
    # add lines. A feature cannot hide in a module declaration or a header comment either, so
    # counting them makes the test reject the extraction it exists to permit — the §1.1 defect
    # once more, in the waiver rather than in the rule.
    #
    # The arithmetic below is deliberate: 900 lines leave big.py, 900 lines of code arrive in
    # moved.py, and the scaffolding pushes the raw net positive while the code net stays zero.
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 300
    write_lines moved.py 600
    printf '# why this module exists\n# and what it holds\nimport os\n\n' >> moved.py
    printf 'import moved\n' >> big.py
    git add big.py moved.py
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'exceeded=changed_lines'
}

@test "relocation: adding real code alongside a move still forfeits the waiver" {
    # The counterpart. Comments and imports are free; statements are not, or the waiver would
    # launder a feature through a refactor.
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 300
    write_lines moved.py 600
    write_lines extra.py 200
    git add big.py moved.py extra.py
    run bash "$GATES" --staged
    printf '%s\n' "$output" | grep -q 'exceeded=changed_lines'
}

@test "relocation: a new oversized file is reported but does not block a pure move" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 150
    write_lines big_tests.py 740
    git add big.py big_tests.py
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    # Still visible — the extracted file is over the limit and needs splitting further
    # — but not blocking, because rejecting it would preserve the 900-line monolith.
    printf '%s\n' "$output" | grep -q 'INFO:COMPLEXITY:big_tests.py'
}

# Writes a file of $2 lines at path $1 where every 7th line carries 24 spaces of
# indentation, deep enough to trip the COMPLEXITY nesting-depth rule.
write_deep_lines() {
    python3 -c "
import sys
path, count = sys.argv[1], int(sys.argv[2])
out = []
for i in range(count):
    if i % 7 == 0:
        out.append('                        deep_%d = 1' % i)
    else:
        out.append('    plain_%d = %d' % (i, i))
open(path, 'w').write('\n'.join(out) + '\n')
" "$1" "$2"
}

@test "relocation: per-line findings on relocated code do not block" {
    # A new file has nothing to diff against, so every line reads as freshly authored
    # and per-line rules fire on code that only moved. Moved code keeps its indentation,
    # so nesting depth is the common case.
    write_deep_lines parent.py 900
    git add parent.py
    git commit -q -m "parent with deep indentation"
    write_deep_lines parent.py 100
    write_deep_lines parent_tests.py 795
    git add parent.py parent_tests.py
    run bash "$GATES" --staged
    [ "$status" -eq 0 ]
    # Recorded, not silently dropped.
    printf '%s\n' "$output" | grep -q 'INFO:COMPLEXITY:parent_tests.py:.*nesting depth'
    ! printf '%s\n' "$output" | grep -qE '^COMPLEXITY:parent_tests.py:[0-9]+: nesting depth'
}

@test "relocation: per-line findings on genuinely new code still block" {
    write_deep_lines parent.py 900
    git add parent.py
    git commit -q -m "parent with deep indentation"
    write_deep_lines feature.py 200
    git add feature.py
    run env AUTOSPEC_COMPLEXITY_ENFORCE=1 bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -qE '^COMPLEXITY:feature.py:[0-9]+: nesting depth'
}

@test "relocation: a new oversized file that adds material still blocks" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 890
    write_lines feature.py 700
    git add big.py feature.py
    # Anchored and enforced: unanchored, this passed on the INFO line the advisory
    # default emits while PR_SIZE supplied the nonzero status, so it stopped testing the
    # rule it names.
    run env AUTOSPEC_COMPLEXITY_ENFORCE=1 bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q '^COMPLEXITY:feature.py'
    printf '%s\n' "$output" | grep -q 'file is 700 LOC'
}

@test "relocation: growing a pre-existing file forfeits the waiver even alongside a move" {
    write_lines big.py 900
    git add big.py
    git commit -q -m "pre-existing oversized file"
    write_lines big.py 901
    write_lines extracted.py 10
    git add big.py extracted.py
    run env AUTOSPEC_COMPLEXITY_ENFORCE=1 bash "$GATES" --staged
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'file is 901 LOC'
}
