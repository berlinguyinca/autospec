#!/usr/bin/env bats
# tests/performance-workstream.bats — contract tests for issue #1536 continuous performance workstream.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/performance-workstream.sh"
    WORK="$(mktemp -d -t performance-workstream.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

@test "benchmark: records per-commit baselines and gates statistically significant regressions" {
    LEDGER="$WORK/benchmarks.jsonl"

    run bash "$SCRIPT" record-benchmark --ledger "$LEDGER" --benchmark execution_fast_path --commit base123 --p50-ms 40 --p99-ms 45 --allocations 100 --samples 30 --stddev-ms 1 --timestamp 2026-07-07T00:00:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" record-benchmark --ledger "$LEDGER" --benchmark execution_fast_path --commit cand456 --p50-ms 41 --p99-ms 58 --allocations 125 --samples 30 --stddev-ms 1 --timestamp 2026-07-07T01:00:00Z
    [ "$status" -eq 0 ]

    run bash "$SCRIPT" gate --ledger "$LEDGER" --baseline-commit base123 --candidate-commit cand456 --max-regression-pct 10 --min-z-score 2
    [ "$status" -eq 1 ]
    [[ "$output" == *"P99_REGRESSION_SIGNIFICANT:execution_fast_path:45.0ms->58.0ms"* ]]
    [[ "$output" == *"ALLOCATION_REGRESSION:execution_fast_path:100->125"* ]]
    [[ "$output" == *"performance gate failed"* ]]

    run bash "$SCRIPT" record-benchmark --ledger "$LEDGER" --benchmark execution_fast_path --commit safe789 --p50-ms 39 --p99-ms 44 --allocations 98 --samples 30 --stddev-ms 1 --timestamp 2026-07-07T02:00:00Z
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" gate --ledger "$LEDGER" --baseline-commit base123 --candidate-commit safe789 --max-regression-pct 10 --min-z-score 2
    [ "$status" -eq 0 ]
    [[ "$output" == *"performance gate passed"* ]]
}

@test "regression: auto-files an auto-implement issue with the offending metric" {
    cat > "$WORK/regressions.jsonl" <<'JSONL'
{"benchmark":"execution_fast_path","metric":"p99_ms","baseline":45.0,"candidate":58.0,"delta_pct":28.9,"z_score":50.3,"commit":"cand456","fitness":"<50ms fast-path guard","test_cmd":"bash scripts/performance-workstream.sh gate --ledger .autospec/benchmarks/performance.jsonl --baseline-commit base123 --candidate-commit cand456"}
JSONL

    run bash "$SCRIPT" propose-regression-issue --regressions "$WORK/regressions.jsonl" --out "$WORK/issues"
    [ "$status" -eq 0 ]
    [ -f "$WORK/issues/execution-fast-path-p99-ms-regression.md" ]
    body="$(cat "$WORK/issues/execution-fast-path-p99-ms-regression.md")"
    [[ "$body" == *"auto-implement"* ]]
    [[ "$body" == *"p99_ms"* ]]
    [[ "$body" == *"58.0"* ]]
    [[ "$body" == *"<50ms fast-path guard"* ]]
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$WORK/issues/execution-fast-path-p99-ms-regression.md"
    [ "$status" -eq 0 ]
}

@test "optimization: report requires before/after deltas and rejects collateral regressions" {
    cat > "$WORK/before.jsonl" <<'JSONL'
{"benchmark":"execution_fast_path","commit":"before","p50_ms":40,"p99_ms":45,"allocations":100,"samples":30,"stddev_ms":1}
{"benchmark":"planner_queue","commit":"before","p50_ms":12,"p99_ms":20,"allocations":20,"samples":30,"stddev_ms":1}
JSONL
    cat > "$WORK/after.jsonl" <<'JSONL'
{"benchmark":"execution_fast_path","commit":"after","p50_ms":32,"p99_ms":39,"allocations":82,"samples":30,"stddev_ms":1}
{"benchmark":"planner_queue","commit":"after","p50_ms":13,"p99_ms":26,"allocations":22,"samples":30,"stddev_ms":1}
JSONL

    run bash "$SCRIPT" optimization-report --before "$WORK/before.jsonl" --after "$WORK/after.jsonl" --out "$WORK/report.md" --max-regression-pct 10
    [ "$status" -eq 1 ]
    [[ "$output" == *"P99_REGRESSION:planner_queue"* ]]
    [ ! -f "$WORK/report.md" ]

    cat > "$WORK/after-safe.jsonl" <<'JSONL'
{"benchmark":"execution_fast_path","commit":"after","p50_ms":32,"p99_ms":39,"allocations":82,"samples":30,"stddev_ms":1}
{"benchmark":"planner_queue","commit":"after","p50_ms":12,"p99_ms":19,"allocations":20,"samples":30,"stddev_ms":1}
JSONL
    run bash "$SCRIPT" optimization-report --before "$WORK/before.jsonl" --after "$WORK/after-safe.jsonl" --out "$WORK/report.md" --max-regression-pct 10
    [ "$status" -eq 0 ]
    grep -q 'execution_fast_path' "$WORK/report.md"
    grep -q 'p99_ms: 45.0 -> 39.0' "$WORK/report.md"
    grep -q 'No collateral benchmark regressions' "$WORK/report.md"
}

@test "fast-path: guard enforces the shared sub-50ms fitness budget" {
    run bash "$SCRIPT" fast-path-guard --metric execution_fast_path --p99-ms 49 --max-ms 50
    [ "$status" -eq 0 ]
    [[ "$output" == *"fast-path guard passed"* ]]

    run bash "$SCRIPT" fast-path-guard --metric execution_fast_path --p99-ms 51 --max-ms 50
    [ "$status" -eq 1 ]
    [[ "$output" == *"FAST_PATH_BUDGET_BREACH:execution_fast_path:51.0ms>50.0ms"* ]]
}

@test "direct Rust validation wires the performance workstream script, runbook, and bats suite" {
    catalog="$REPO_ROOT/crates/autospec-core/src/validation/catalog.rs"
    owner="$REPO_ROOT/crates/autospec-core/src/validation/external.rs"
    grep -q '"check_performance_workstream_contract"' "$catalog"
    grep -q 'ExternalCheck::PerformanceWorkstream' "$catalog"
    grep -q 'performance-workstream\.sh' "$owner"
    grep -q 'tests/autonomous/test_performance_workstream\.bats' "$owner"
    [ -f "$REPO_ROOT/docs/runbooks/performance-workstream.md" ]
}
