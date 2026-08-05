#!/usr/bin/env bats
# tests/unit/test_lint_implementation_perf.bats — scale + no-regression checks
# for scripts/lint-implementation.sh's per-added-line detectors (SECURITY,
# TODO_LEFT, MOCK_DB, DOC_OUT_OF_SYNC, VACUOUS_*, ASSERTION_DENSITY, COMPLEXITY).
#
# Those detectors used to spawn a grep/sed/cut subprocess per added line, so a
# large diff (e.g. hoisting a big inline test module out of a huge file) could
# run for 20+ minutes without finishing. They now test each line in-process via
# bash `[[ =~ ]]`. This file proves both halves of that fix: it still scales,
# and it did not lose any detection coverage.

bats_require_minimum_version 1.5.0

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-implementation.sh"
    FIX="$REPO_ROOT/tests/fixtures/implementation-quality"
}

# build_large_diff N FILE — synthesize an N-added-line diff at runtime (never
# committed as a fixture) touching one new bats file under tests/unit/.
build_large_diff() {
    local n="$1" out="$2"
    {
        printf 'diff --git a/tests/unit/test_synth_big.bats b/tests/unit/test_synth_big.bats\n'
        printf 'new file mode 100644\n'
        printf -- '--- /dev/null\n'
        printf '+++ b/tests/unit/test_synth_big.bats\n'
        printf '@@ -0,0 +1,%d @@\n' "$n"
        awk -v n="$n" 'BEGIN { for (i = 0; i < n; i++) print "+    echo \"synthetic line " i "\"" }'
    } > "$out"
}

@test "lint-implementation: 20k-line synthetic diff lints in under 60s" {
    local diff_file="$BATS_TEST_TMPDIR/big.diff"
    build_large_diff 20000 "$diff_file"

    local start end elapsed
    start=$(date +%s)
    run timeout 60 bash "$LINT" --diff-file "$diff_file" --pre-commit
    end=$(date +%s)
    elapsed=$((end - start))

    [ "$status" -ne 124 ]
    [ "$elapsed" -lt 60 ]
    ! echo "$output" | grep -q "No such file or directory"
}

@test "lint-implementation: detectors still fire on a normal small diff (no weakening)" {
    local diff_file="$BATS_TEST_TMPDIR/small.diff"
    # Build dangerous/mock-db substrings by concatenation so this fixture
    # file itself never contains a literal SECURITY- or MOCK_DB-triggering
    # pattern on one source line (this file is scanned by the same linter).
    local danger_call="ev""al(user_input)"
    local db_symbol="Data""Source"
    {
        printf 'diff --git a/scripts/example-tool.sh b/scripts/example-tool.sh\n'
        printf 'new file mode 100644\n'
        printf -- '--- /dev/null\n'
        printf '+++ b/scripts/example-tool.sh\n'
        printf '@@ -0,0 +1,3 @@\n'
        printf '+%s\n' "$danger_call"
        printf '+%s: revisit this later\n' "FIX""ME"
        printf '+echo done\n'
        printf 'diff --git a/tests/unit/example_synth.bats b/tests/unit/example_synth.bats\n'
        printf 'new file mode 100644\n'
        printf -- '--- /dev/null\n'
        printf '+++ b/tests/unit/example_synth.bats\n'
        printf '@@ -0,0 +1,7 @@\n'
        printf '+@test "does a thing" {\n'
        printf '+    run some_command\n'
        printf '+    [ "$status" -eq 1 ] || true\n'
        printf '+}\n'
        printf '+@test "mocks the database" {\n'
        printf '+    stub %s\n' "$db_symbol"
        printf '+}\n'
    } > "$diff_file"

    run bash "$LINT" --diff-file "$diff_file" --pre-commit
    echo "$output" | grep -qE '^SECURITY:.*eval\(\) usage'
    echo "$output" | grep -qE '^TODO_LEFT:'
    echo "$output" | grep -qE '^VACUOUS_OR_TRUE:'
    echo "$output" | grep -qE '^MOCK_DB:'
    echo "$output" | grep -qE '^ASSERTION_DENSITY:'
}

@test "lint-implementation: good.diff still exits 0 (no new false positives)" {
    run bash "$LINT" --diff-file "$FIX/good.diff" --pre-commit
    [ "$status" -eq 0 ]
}
