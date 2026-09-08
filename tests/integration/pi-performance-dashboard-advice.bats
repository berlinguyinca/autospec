#!/usr/bin/env bats
# tests/integration/pi-performance-dashboard-advice.bats — integration
# coverage for the routing-advice flow behind the dashboard (issue #3327):
# the configured sample threshold as a live boundary, cost-based winner
# selection between two eligible profiles, window-driven throughput, and
# fail-closed validation before any number is printed.

setup() {
    script="${BATS_TEST_DIRNAME}/../../scripts/pi-performance-dashboard.sh"
    tmp="$(mktemp -d "${BATS_TMPDIR:-/tmp}/pi-perf-adv-XXXXXX")"
}

teardown() {
    rm -rf "$tmp"
}

write_rows() {
    # $1 = file, $2 = issue prefix, $3 = profile, $4 = cost_micros, $5 = count
    local file="$1" prefix="$2" profile="$3" cost="$4" count="$5" i d
    for i in $(seq 1 "$count"); do
        d=$((i * 1000))
        printf '{"issue_id":"%s%s","ts":"2026-08-01T00:00:00Z","model_family":"qwen3.8","profile":"%s","duration_ms":%d,"succeeded":true,"wall_ms":%d,"cost_micros":%d}\n' \
            "$prefix" "$i" "$profile" "$d" "$d" "$cost" >> "$file"
    done
}

@test "exactly 20 samples cross into benchmark selection" {
    write_rows "$tmp/ledger.jsonl" "a" "qwen3.8-27b-q4/rtx4090" 0 20
    bash "$script" --ledger "$tmp/ledger.jsonl" > "$tmp/dash.txt"
    [ $? -eq 0 ]
    grep -q "^  source: benchmark$" "$tmp/dash.txt"
    grep -q "^  profile: qwen3.8-27b-q4/rtx4090$" "$tmp/dash.txt"
}

@test "19 samples stay on static selection" {
    write_rows "$tmp/ledger.jsonl" "a" "qwen3.8-27b-q4/rtx4090" 0 19
    bash "$script" --ledger "$tmp/ledger.jsonl" > "$tmp/dash.txt"
    [ $? -eq 0 ]
    grep -q "^  source: static$" "$tmp/dash.txt"
    grep -q "19 samples below configured 20" "$tmp/dash.txt"
}

@test "cheaper eligible profile wins the advisory" {
    write_rows "$tmp/ledger.jsonl" "a" "qwen3.8-27b-q4/rtx4090" 0 20
    write_rows "$tmp/ledger.jsonl" "b" "qwen3.8-27b-bf16/dual-turing" 5000 20
    bash "$script" --ledger "$tmp/ledger.jsonl" > "$tmp/dash.txt"
    [ $? -eq 0 ]
    grep -q "^  source: benchmark$" "$tmp/dash.txt"
    grep -q "^  profile: qwen3.8-27b-q4/rtx4090$" "$tmp/dash.txt"
    grep -q "selected qwen3.8-27b-q4/rtx4090: cheapest eligible profile under locked family qwen3.8" "$tmp/dash.txt"
}

@test "window hours change the reported issue throughput" {
    write_rows "$tmp/ledger.jsonl" "a" "qwen3.8-27b-q4/rtx4090" 0 20
    bash "$script" --ledger "$tmp/ledger.jsonl" --window-hours 24 > "$tmp/dash24.txt"
    [ $? -eq 0 ]
    grep -q "^  successful_issues_per_hour: 0.83$" "$tmp/dash24.txt"
    bash "$script" --ledger "$tmp/ledger.jsonl" --window-hours 8 > "$tmp/dash8.txt"
    [ $? -eq 0 ]
    grep -q "^  successful_issues_per_hour: 2.50$" "$tmp/dash8.txt"
}

@test "a malformed ledger row fails closed before any number is printed" {
    write_rows "$tmp/ledger.jsonl" "a" "qwen3.8-27b-q4/rtx4090" 0 5
    printf '{"issue_id":"broken","ts":"t","model_family":"qwen3.8","profile":"p","duration_ms":5,"succeeded":"yes"}\n' >> "$tmp/ledger.jsonl"
    run bash "$script" --ledger "$tmp/ledger.jsonl"
    [ "$status" -ne 0 ]
    [[ "$output" == *"succeeded must be a boolean"* ]]
}
