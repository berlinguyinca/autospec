#!/usr/bin/env bats
# tests/test-quality-workstream.bats — contract tests for issue #1534 continuous test-quality workstream.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/test-quality-workstream.sh"
    WORK="$(mktemp -d -t test-quality-workstream.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && chmod -R u+w "$WORK" 2>/dev/null || true
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

@test "metrics: records coverage and mutation per crate, then gates regressions" {
    LEDGER="$WORK/metrics.jsonl"

    run bash "$SCRIPT" record-metric --ledger "$LEDGER" --crate autospec-core --coverage 91 --mutation 86 --flakes 2 --timestamp 2026-07-07T00:00:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" record-metric --ledger "$LEDGER" --crate autospec-core --coverage 92 --mutation 87 --flakes 1 --timestamp 2026-07-07T01:00:00Z
    [ "$status" -eq 0 ]

    run bash "$SCRIPT" gate --ledger "$LEDGER" --min-coverage 90 --min-mutation 80 --max-flake-rate 2
    [ "$status" -eq 0 ]
    [[ "$output" == *"gate passed"* ]]

    run bash "$SCRIPT" record-metric --ledger "$LEDGER" --crate autospec-core --coverage 89 --mutation 79 --flakes 3 --timestamp 2026-07-07T02:00:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" gate --ledger "$LEDGER" --min-coverage 90 --min-mutation 80 --max-flake-rate 2
    [ "$status" -eq 1 ]
    [[ "$output" == *"COVERAGE_BELOW_FLOOR:autospec-core:89%<90%"* ]]
    [[ "$output" == *"MUTATION_BELOW_FLOOR:autospec-core:79%<80%"* ]]
    [[ "$output" == *"FLAKE_RATE_ABOVE_FLOOR:autospec-core:3>2"* ]]
}

@test "survivor: proposes an auto-implement issue with red/green verification commands" {
    MUTANTS="$WORK/survivors.jsonl"
    cat > "$MUTANTS" <<'JSONL'
{"crate":"autospec-core","file":"crates/autospec-core/src/lib.rs","mutant":"replace >= with >","test":"cargo test -p autospec-core rejects_boundary_value"}
JSONL

    run bash "$SCRIPT" propose-mutant-issue --mutants "$MUTANTS" --out "$WORK/issues"
    [ "$status" -eq 0 ]
    [ -f "$WORK/issues/autospec-core-crates-autospec-core-src-lib-rs.md" ]
    body="$(cat "$WORK/issues/autospec-core-crates-autospec-core-src-lib-rs.md")"
    [[ "$body" == *"auto-implement"* ]]
    [[ "$body" == *"Surviving mutant"* ]]
    [[ "$body" == *"replace >= with >"* ]]
    [[ "$body" == *"Verified red"* ]]
    [[ "$body" == *"Verified green"* ]]
    [[ "$body" == *"cargo test -p autospec-core rejects_boundary_value"* ]]
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$WORK/issues/autospec-core-crates-autospec-core-src-lib-rs.md"
    [ "$status" -eq 0 ]
}

@test "flake: quarantines nondeterministic tests and tracks flake-rate metric" {
    run bash "$SCRIPT" quarantine-flake --ledger "$WORK/flakes.jsonl" --quarantine "$WORK/quarantine.jsonl" --issues-dir "$WORK/issues" --crate autospec-cli --test "tests::sometimes_times_out" --reason "failed 2/5 retries" --timestamp 2026-07-07T03:00:00Z
    [ "$status" -eq 0 ]
    [[ "$output" == *"quarantined tests::sometimes_times_out"* ]]
    grep -q '"flakes":1' "$WORK/flakes.jsonl"
    ! grep -q '"coverage":100' "$WORK/flakes.jsonl"
    ! grep -q '"mutation":100' "$WORK/flakes.jsonl"
    grep -q '"test":"tests::sometimes_times_out"' "$WORK/quarantine.jsonl"
    grep -q 'failed 2/5 retries' "$WORK/issues/autospec-cli-tests-sometimes-times-out.md"
    grep -q 'hardening' "$WORK/issues/autospec-cli-tests-sometimes-times-out.md"
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$WORK/issues/autospec-cli-tests-sometimes-times-out.md"
    [ "$status" -eq 0 ]
}

@test "readonly: locks test files and check-readonly rejects writable assertions" {
    mkdir -p "$WORK/repo/tests"
    printf '@test "sample" {\n  [ 1 -eq 1 ]\n}\n' > "$WORK/repo/tests/sample.bats"

    run bash "$SCRIPT" check-readonly --repo-root "$WORK/repo" --paths tests
    [ "$status" -eq 1 ]
    [[ "$output" == *"TEST_FILE_WRITABLE:tests/sample.bats"* ]]

    run bash "$SCRIPT" lock-tests --repo-root "$WORK/repo" --paths tests
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" check-readonly --repo-root "$WORK/repo" --paths tests
    [ "$status" -eq 0 ]
    [[ "$output" == *"test files read-only"* ]]
}
