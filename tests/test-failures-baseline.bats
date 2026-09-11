#!/usr/bin/env bats
# tests/test-failures-baseline.bats — the known-failing-test ratchet (issue #4291).
#
# `scripts/test-failures-baseline.sh` is the gate that turns "the suite always
# fails 2 tests" into a recorded set in autospec/baseline-failures.txt, so a
# conversion gate can tell NEW-TEST-FAILURES from UNKNOWN-NO-BASELINE
# (docs/conversion-gate.md). These tests pin the gate's behaviour on synthetic
# cargo logs: a new failure blocks, a stale entry blocks, a harness that never
# ran blocks, and --rebaseline only ever lowers the file.

setup() {
    REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
    GATE="$REPO_ROOT/scripts/test-failures-baseline.sh"
    COMMITTED="$REPO_ROOT/autospec/baseline-failures.txt"
    FIXTURE="$REPO_ROOT/tests/fixtures/test-failures-baseline/main-baseline.log"
    TMP="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP"
}

# write_baseline FILE [extra entry lines...] — a lint-clean baseline with the
# two ids the incident produced, plus any extra entries passed in.
write_baseline() {
    local file="$1"
    shift
    {
        printf '# baseline header line\n'
        printf '# second header line\n'
        printf 'autonomous_stale_startup_recovery::foreground_scan_recovers_stale_pending_startup_heartbeat_pending_before_acquire\tissue=#4291\tbinary=autonomous_conductor_commands\n'
        printf 'foreground_recovers_with_integrated_inactive_local_branch\tissue=#4291\tbinary=autonomous_conductor_commands\n'
        if [ $# -gt 0 ]; then printf '%s\n' "$@"; fi
    } >"$file"
}

# cargo_log FILE FAILED_ID... — a minimal cargo test log whose declared failure
# count matches the number of `test ... FAILED` lines it contains.
cargo_log() {
    local file="$1"
    shift
    {
        printf '     Running tests/autonomous_conductor_commands.rs (target/debug/deps/x)\n'
        local n=0 id
        for id in "$@"; do
            printf 'test %s ... FAILED\n' "$id"
            n=$((n + 1))
        done
        printf 'test result: FAILED. %d passed; %d failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s\n' "${PASSED:-0}" "$n"
    } >"$file"
}

# --------------------------------------------------------------------------

@test "--help prints the gate header and exits 0" {
    run bash "$GATE" --help
    [ "$status" -eq 0 ]
    printf '%s\n' "${lines[@]}" | grep -q 'autospec/baseline-failures.txt'
    printf '%s\n' "${lines[@]}" | grep -q -- '--rebaseline'
}

@test "committed baseline lints clean" {
    run bash "$GATE" --lint-baseline-only
    [ "$status" -eq 0 ]
    printf '%s\n' "${lines[@]}" | grep -q '^BASELINE_OK:'
}

@test "committed baseline check passes against the real-run fixture" {
    [ -f "$FIXTURE" ]
    run bash "$GATE" --check --run-log "$FIXTURE"
    [ "$status" -eq 0 ]
    printf '%s\n' "${lines[@]}" | grep -q '^BASELINE_MATCH: 2 failing test'
}

@test "an entry without an issue reference is rejected" {
    printf 'some_test\tbinary=autonomous_conductor_commands\n' >"$TMP/bl.txt"
    run bash "$GATE" --lint-baseline-only --baseline "$TMP/bl.txt"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'BASELINE_ENTRY_NO_ISSUE'
}

@test "duplicate ids are rejected" {
    printf 'a_test\tissue=#4291\na_test\tissue=#4291\n' >"$TMP/bl.txt"
    run bash "$GATE" --lint-baseline-only --baseline "$TMP/bl.txt"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'BASELINE_DUPLICATE_ID'
}

@test "entries not sorted with LC_ALL=C are rejected" {
    printf 'b_test\tissue=#4291\na_test\tissue=#4291\n' >"$TMP/bl.txt"
    run bash "$GATE" --lint-baseline-only --baseline "$TMP/bl.txt"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'BASELINE_UNSORTED'
}

@test "missing baseline file is an environment error (exit 2)" {
    run bash "$GATE" --check --baseline "$TMP/nope.txt" --run-log "$FIXTURE"
    [ "$status" -eq 2 ]
    printf '%s\n' "${lines[@]}" | grep -q 'baseline file not found'
}

@test "a failing test that is not baselined blocks with NEW_TEST_FAILURE" {
    write_baseline "$TMP/bl.txt"
    cargo_log "$TMP/run.log" \
        autonomous_stale_startup_recovery::foreground_scan_recovers_stale_pending_startup_heartbeat_pending_before_acquire \
        foreground_recovers_with_integrated_inactive_local_branch \
        brand_new_broken_test
    run bash "$GATE" --check --baseline "$TMP/bl.txt" --run-log "$TMP/run.log"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'NEW_TEST_FAILURE: `brand_new_broken_test`'
    printf '%s\n' "${lines[@]}" | grep -q 'target=autonomous_conductor_commands'
}

@test "an all-green run passes" {
    write_baseline "$TMP/bl.txt"
    {
        printf '     Running tests/autonomous_conductor_commands.rs (target/debug/deps/x)\n'
        printf 'test unrelated_test ... ok\n'
        printf 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 1.00s\n'
    } >"$TMP/green.log"
    run bash "$GATE" --check --baseline "$TMP/bl.txt" --run-log "$TMP/green.log"
    [ "$status" -eq 0 ]
    printf '%s\n' "${lines[@]}" | grep -q '^BASELINE_MATCH: 0 failing test'
}

@test "a baselined test that now passes is a blocking stale entry" {
    write_baseline "$TMP/bl.txt"
    {
        printf '     Running tests/autonomous_conductor_commands.rs (target/debug/deps/x)\n'
        printf 'test foreground_recovers_with_integrated_inactive_local_branch ... ok\n'
        printf 'test autonomous_stale_startup_recovery::foreground_scan_recovers_stale_pending_startup_heartbeat_pending_before_acquire ... FAILED\n'
        printf 'test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s\n'
    } >"$TMP/stale.log"
    run bash "$GATE" --check --baseline "$TMP/bl.txt" --run-log "$TMP/stale.log"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'STALE_BASELINE_ENTRY: `foreground_recovers_with_integrated_inactive_local_branch`'
}

@test "a baselined id that was not executed warns but is not called fixed" {
    write_baseline "$TMP/bl.txt"
    cargo_log "$TMP/one.log" \
        autonomous_stale_startup_recovery::foreground_scan_recovers_stale_pending_startup_heartbeat_pending_before_acquire
    run bash "$GATE" --check --baseline "$TMP/bl.txt" --run-log "$TMP/one.log"
    [ "$status" -eq 0 ]
    printf '%s\n' "${lines[@]}" | grep -q 'neither run nor failed'
    ! printf '%s\n' "${lines[@]}" | grep -q 'STALE_BASELINE_ENTRY'
}

@test "a harness that never ran is HARNESS_NEVER_RAN, not a pass" {
    write_baseline "$TMP/bl.txt"
    printf '   Compiling autospec-cli v0.1.0\n    Finished dev [unoptimized] in 0.10s\n' >"$TMP/nores.log"
    run bash "$GATE" --check --baseline "$TMP/bl.txt" --run-log "$TMP/nores.log"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'HARNESS_NEVER_RAN'
}

@test "a compile failure is TESTS_DO_NOT_COMPILE" {
    write_baseline "$TMP/bl.txt"
    {
        printf 'error[E0433]: failed to resolve: use of undeclared type `Nope`\n'
        printf 'error: could not compile `autospec-cli` (test "autonomous_conductor_commands")\n'
    } >"$TMP/compile.log"
    run bash "$GATE" --check --baseline "$TMP/bl.txt" --run-log "$TMP/compile.log"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'TESTS_DO_NOT_COMPILE'
}

@test "declared failures with no parsed id is FAILURE_ATTRIBUTION" {
    write_baseline "$TMP/bl.txt"
    {
        printf '     Running tests/autonomous_conductor_commands.rs (target/debug/deps/x)\n'
        printf 'test result: FAILED. 0 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s\n'
    } >"$TMP/unattributed.log"
    run bash "$GATE" --check --baseline "$TMP/bl.txt" --run-log "$TMP/unattributed.log"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'FAILURE_ATTRIBUTION'
}

@test "--rebaseline refuses to grow the baseline and leaves the file alone" {
    write_baseline "$TMP/bl.txt"
    cp "$TMP/bl.txt" "$TMP/bl.orig"
    cargo_log "$TMP/up.log" \
        autonomous_stale_startup_recovery::foreground_scan_recovers_stale_pending_startup_heartbeat_pending_before_acquire \
        foreground_recovers_with_integrated_inactive_local_branch \
        brand_new_broken_test
    run bash "$GATE" --rebaseline --baseline "$TMP/bl.txt" --run-log "$TMP/up.log"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'REBASELINE_REFUSED_UPWARD: `brand_new_broken_test`'
    cmp -s "$TMP/bl.txt" "$TMP/bl.orig"
}

@test "--rebaseline drops an entry proven fixed and keeps the header" {
    write_baseline "$TMP/bl.txt"
    {
        printf '     Running tests/autonomous_conductor_commands.rs (target/debug/deps/x)\n'
        printf 'test foreground_recovers_with_integrated_inactive_local_branch ... ok\n'
        printf 'test autonomous_stale_startup_recovery::foreground_scan_recovers_stale_pending_startup_heartbeat_pending_before_acquire ... FAILED\n'
        printf 'test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s\n'
    } >"$TMP/down.log"
    run bash "$GATE" --rebaseline --baseline "$TMP/bl.txt" --run-log "$TMP/down.log"
    [ "$status" -eq 0 ]
    grep -q '^BASELINE_SHRUNK: .* 2 -> 1' <<<"$output"
    grep -q '^# baseline header line$' "$TMP/bl.txt"
    grep -q '^# second header line$' "$TMP/bl.txt"
    grep -q '^foreground_recovers_with_integrated_inactive_local_branch' "$TMP/bl.txt" && return 1
    grep -q 'autonomous_stale_startup_recovery::foreground_scan_recovers_stale_pending_startup_heartbeat_pending_before_acquire' "$TMP/bl.txt"
}

@test "--rebaseline refuses a log whose harness never ran" {
    write_baseline "$TMP/bl.txt"
    printf 'nothing useful here\n' >"$TMP/empty.log"
    run bash "$GATE" --rebaseline --baseline "$TMP/bl.txt" --run-log "$TMP/empty.log"
    [ "$status" -eq 1 ]
    printf '%s\n' "${lines[@]}" | grep -q 'HARNESS_NEVER_RAN'
}

@test "--rebaseline never creates a missing baseline" {
    run bash "$GATE" --rebaseline --baseline "$TMP/nope.txt" --run-log "$FIXTURE"
    [ "$status" -eq 2 ]
    printf '%s\n' "${lines[@]}" | grep -q 'never creates one'
    [ ! -e "$TMP/nope.txt" ]
}

@test "unknown option is a usage error (exit 2)" {
    run bash "$GATE" --bogus-flag
    [ "$status" -eq 2 ]
    printf '%s\n' "${lines[@]}" | grep -q 'unknown option: --bogus-flag'
}

@test "AUTOSPEC_TEST_FAILURE_BASELINE overrides the default path" {
    write_baseline "$TMP/bl.txt"
    run env AUTOSPEC_TEST_FAILURE_BASELINE="$TMP/bl.txt" bash "$GATE" --lint-baseline-only
    [ "$status" -eq 0 ]
    printf '%s\n' "${lines[@]}" | grep -q "BASELINE_OK: $TMP/bl.txt"
}
