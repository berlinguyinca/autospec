#!/usr/bin/env bats
# tests/unit/test_verify_gate.bats — guards #3535.
#
# Four incidents, one shape each, all of them a verification script reporting
# success because it had nothing to look at:
#
#   1. The tool the script measures with was not on PATH. The command produced
#      nothing, the script read nothing as "no failures", and the run went green
#      on a machine that had never executed the suite.
#   2. A suite's output did not parse, so zero result lines were counted. Zero
#      parsed results was reported as zero failures instead of "I did not
#      measure anything" — the worst possible output masquerading as the best.
#   3. A suite exited non-zero without printing anything. The script inferred
#      the verdict from output, saw no failure line, and passed it.
#   4. A command's exit status was lost between the command and the verdict —
#      piped away, or overwritten by a later command — so a failing lane whose
#      output merely *looked* healthy was recorded as passing.
#
# The rule this pins: a gate may report pass only from a measured result. Every
# other state has its own name — `unknown` when the gate ran and learned
# nothing, `unavailable` when the gate could not run at all — and neither one is
# the number 0.
#
# Every case runs the REAL script against REAL `sh -c` lanes in a temporary
# directory, so exit codes and captured output are produced by the shell rather
# than stubbed.

ROOT="${BATS_TEST_DIRNAME}/../.."
GATE="$ROOT/scripts/verify-gate.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    MANIFEST="$TEST_TMP/lanes.tsv"
    REPORT="$TEST_TMP/report.json"
    # A directory that exists and is empty: PATH set to it resolves no tool at
    # all, which is the condition the preflight exists to catch.
    NOBIN="$TEST_TMP/nobin"
    mkdir -p "$NOBIN"
}

teardown() { rm -rf "$TEST_TMP"; }

# lane <name> <command> [result-regex] — append one manifest row. Tab-separated,
# because that is what the script parses; a space here would silently test
# something else.
lane() {
    printf '%s\t%s' "$1" "$2" >>"$MANIFEST"
    if [ "$#" -ge 3 ]; then
        printf '\t%s' "$3" >>"$MANIFEST"
    fi
    printf '\n' >>"$MANIFEST"
}

# run_gate [extra flags...] — the gate over the manifest built so far.
run_gate() {
    sh "$GATE" --repo-root "$TEST_TMP" --report "$REPORT" "$@" "$MANIFEST"
}

# report_has <substring> — the written status record must contain it.
report_has() {
    grep -qF -- "$1" "$REPORT" || {
        printf 'report missing %s\n--- report ---\n%s\n' "$1" "$(cat "$REPORT")" >&2
        return 1
    }
}

# --- failure mode 1: the measuring tool is absent ----------------------------

@test "a required tool that is absent fails the run and names the tool" {
    lane build 'printf "ok\n"' 'ok'
    run run_gate --require-tool autospec-definitely-not-installed
    [ "$status" -eq 3 ]
    echo "$output" | grep -q "UNAVAILABLE"
    echo "$output" | grep -q "autospec-definitely-not-installed"
    if echo "$output" | grep -q "PASS"; then
        fail "an unavailable run must never print PASS: $output"
    fi
}

@test "every missing tool is named rather than swallowed" {
    lane build 'printf "ok\n"' 'ok'
    run run_gate --require-tool missing-tool-one --require-tool missing-tool-two
    [ "$status" -eq 3 ]
    echo "$output" | grep -q "missing-tool-one"
    echo "$output" | grep -q "missing-tool-two"
}

@test "a stack without awk is unavailable and reports no counts at all" {
    lane tests 'printf "4 passed\n"' '[0-9]+ passed'
    # `awk` counts the result lines. With it gone the run measures nothing, so
    # it must not be reported as though it had run.
    run env PATH="$NOBIN" /bin/sh "$GATE" --repo-root "$TEST_TMP" "$MANIFEST"
    [ "$status" -eq 3 ]
    echo "$output" | grep -q "UNAVAILABLE"
    echo "$output" | grep -q "awk"
    if echo "$output" | grep -q "PASS"; then
        fail "a tool-less run must never print PASS: $output"
    fi
}

@test "unavailable is recorded as unknown numbers, never zeros" {
    lane tests 'printf "4 passed\n"' '[0-9]+ passed'
    run env PATH="$NOBIN" /bin/sh "$GATE" --repo-root "$TEST_TMP" \
        --report "$REPORT" "$MANIFEST"
    [ "$status" -eq 3 ]
    [ -f "$REPORT" ]
    # The record exists so a machine consumer cannot fall back to a default of
    # zero; every aggregate field says `unknown`.
    report_has '"status":"UNAVAILABLE"'
    report_has '"total":"unknown"'
    report_has '"failed":"unknown"'
    report_has '"missing_tools":['
    if grep -qF '"total":0' "$REPORT"; then
        fail "an unrun gate recorded a total of 0 instead of unknown"
    fi
}

# --- failure mode 2: nothing parsed was read as nothing failed ---------------

@test "a suite whose output never parses is unknown, not pass" {
    # Exit 0 and a wall of unrecognised text: zero result lines matched, which
    # means "did not measure", not "no failures".
    lane tests 'printf "Segmentation fault (core dumped)\n" >&2; exit 0' \
        '[0-9]+ tests passed'
    run run_gate
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "UNKNOWN"
    if echo "$output" | grep -q "PASS"; then
        fail "unparsed output was reported as PASS: $output"
    fi
}

@test "an empty-output suite records zero result lines and the unknown status" {
    lane tests 'true' '[0-9]+ tests passed'
    run run_gate
    [ "$status" -eq 2 ]
    # Zero is recorded as the measurement it is, and the *status* carries the
    # unknown — the two are never conflated into "0 failures, therefore pass".
    report_has '"status":"unknown"'
    report_has '"result_lines":0'
    report_has '"status":"UNKNOWN"'
    report_has '"failed":0'
    report_has '"unknown":1'
}

@test "a lane with no result regex cannot prove it measured anything" {
    # Fail closed: without a result pattern, nothing distinguishes "the suite
    # passed" from "the suite printed nothing".
    lane tests 'printf "4 passed\n"'
    run run_gate
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "UNKNOWN"
}

@test "an empty manifest measures nothing and is unknown, not green" {
    printf '# nothing configured here\n\n' >"$MANIFEST"
    run run_gate
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "UNKNOWN"
    if echo "$output" | grep -q "PASS"; then
        fail "an empty manifest was reported as PASS: $output"
    fi
}

# --- failure mode 3: non-zero exit with empty output -------------------------

@test "a non-zero exit with no output is a fail, not an unknown" {
    # The incident in its purest form: the command said it failed and printed
    # nothing to say so. A verdict inferred from output passes it.
    lane tests 'exit 3' '[0-9]+ tests passed'
    run run_gate
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "FAIL"
    report_has '"status":"fail"'
    report_has '"exit_code":3'
}

@test "a non-zero exit is a fail even when the output looks fine" {
    # The exit status outranks the text: a suite that prints "all green" and
    # then exits 1 failed.
    lane tests 'printf "31 tests passed\n"; exit 1' '[0-9]+ tests passed'
    run run_gate
    [ "$status" -eq 1 ]
    report_has '"status":"fail"'
    report_has '"result_lines":1'
}

@test "a command not found is unknown rather than a pass or a fail" {
    # 127 says the lane never ran. That is a different fact from "it ran and
    # failed", and both differ from "it ran and passed".
    lane tests 'autospec-definitely-not-installed --version' 'ok'
    run run_gate
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "UNKNOWN"
    report_has '"exit_code":127'
    report_has '"status":"unknown"'
}

# --- failure mode 4: the exit status surviving to the verdict ----------------

@test "the recorded exit code is the lane status, not a later command status" {
    # The lane prints, then fails. Anything executed between the lane and the
    # verdict — a status-file write, a counter — leaves `$?` holding that
    # command's status instead of the lane's.
    lane tests 'printf "2 tests passed\n"; exit 9' '[0-9]+ tests passed'
    run run_gate
    [ "$status" -eq 1 ]
    report_has '"exit_code":9'
}

@test "lane output is redirected to a file instead of piped away" {
    # `cmd | tee log` and `cmd | grep` replace the lane's status with the last
    # stage's status. The capture must be a redirection, and the status must be
    # read on the line right after it.
    grep -qE 'sh -c "\$lane_command" >"\$WORK/\$lane_name\.out" 2>&1' "$GATE"
    grep -q 'lane_code=$?' "$GATE"
    if grep -qE '\btee\b' "$GATE"; then
        fail "the gate pipes lane output through tee"
    fi
}

# --- findings outrank unknown, plus the positive control ---------------------

@test "a real failure outranks an unknown lane in the same run" {
    lane unit 'printf "5 tests passed\n"' '[0-9]+ tests passed'
    lane lint 'exit 7' '[0-9]+ problems'
    lane e2e 'printf "nothing recognised\n"' '[0-9]+ scenarios'
    run run_gate
    [ "$status" -eq 1 ]
    report_has '"status":"FAIL"'
    report_has '"passed":1'
    report_has '"failed":1'
    report_has '"unknown":1'
}

@test "an unknown lane outranks an otherwise passing run" {
    lane unit 'printf "5 tests passed\n"' '[0-9]+ tests passed'
    lane e2e 'printf "nothing recognised\n"' '[0-9]+ scenarios'
    run run_gate
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "UNKNOWN"
    if echo "$output" | grep -q "PASS"; then
        fail "one unmeasured lane was averaged away into PASS: $output"
    fi
}

@test "a genuinely passing lane passes, so the gate is not always red" {
    lane unit 'printf "12 tests passed\n0 failed\n"' '[0-9]+ tests passed'
    run run_gate
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "PASS"
    report_has '"status":"PASS"'
    report_has '"result_lines":1'
    report_has '"failed_lines":0'
}

@test "a lane reporting a failing result line fails despite exit 0" {
    lane unit 'printf "2 tests passed\n1 test FAILED\n"' '[0-9]+ test'
    run run_gate
    [ "$status" -eq 1 ]
    report_has '"result_lines":2'
    report_has '"failed_lines":1'
}

@test "a summary line reporting zero failures is not read as one" {
    # The shape every test harness prints. A gate that calls this a failure is a
    # gate that gets switched off, which is how the word "failed" in a summary
    # line ends up unpoliced for real.
    lane unit 'printf "test result: ok. 5 passed; 0 failed; 0 ignored\n"' 'test result:'
    run run_gate
    [ "$status" -eq 0 ]
    report_has '"status":"PASS"'
    report_has '"result_lines":1'
    report_has '"failed_lines":0'
}

@test "a zero count does not mask a non-zero one on the same line" {
    lane unit 'printf "suite: 0 failed, 2 errors\n"' 'suite:'
    run run_gate
    [ "$status" -eq 1 ]
    report_has '"result_lines":1'
    report_has '"failed_lines":1'
}

# --- usage and portability ---------------------------------------------------

@test "a missing manifest is a usage error, not a green run" {
    run sh "$GATE" --repo-root "$TEST_TMP"
    [ "$status" -eq 64 ]
}

@test "the gate is POSIX sh and parses under dash" {
    head -1 "$GATE" | grep -q '^#!/usr/bin/env sh$'
    if command -v dash >/dev/null 2>&1; then
        run dash -n "$GATE"
        [ "$status" -eq 0 ]
    fi
    # No bash-only constructs: no [[, no PIPESTATUS (the very thing a POSIX
    # capture must not need), no arrays, no +=.
    if grep -nE '\[\[|\bPIPESTATUS\b|\+=\(|declare -|<\(' "$GATE"; then
        fail "verify-gate.sh uses a bash-only construct"
    fi
}
