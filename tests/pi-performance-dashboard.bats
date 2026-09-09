#!/usr/bin/env bats
# tests/pi-performance-dashboard.bats — named smoke test for
# scripts/pi-performance-dashboard.sh (issue #3327).
#
# Replays a fixed ledger and snapshots the dashboard plus the routing advice.
# Deterministic fixtures only: no clock, no network, no model calls. The
# liveness lines (issue #3723) are pinned with a fixed --now epoch.

setup() {
    script="${BATS_TEST_DIRNAME}/../scripts/pi-performance-dashboard.sh"
    fixture="${BATS_TEST_DIRNAME}/fixtures/pi-performance"
    tmp="$(mktemp -d "${BATS_TMPDIR:-/tmp}/pi-perf-dash-XXXXXX")"
}

teardown() {
    rm -rf "$tmp"
}

@test "script is executable" {
    [ -x "$script" ]
}

@test "--help exits 0 and shows usage" {
    run bash "$script" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
}

@test "missing ledger is a usage error" {
    run bash "$script" --ledger "$tmp/nope.jsonl"
    [ "$status" -ne 0 ]
}

@test "unknown option is a usage error" {
    run bash "$script" --ledger "$fixture/ledger.jsonl" --bogus
    [ "$status" -ne 0 ]
}

@test "replays the fixed ledger and matches the snapshot" {
    # --now is fixed so the liveness/elapsed lines are reproducible:
    # 1785545700 = 2026-08-01T00:55:00Z, exactly 5 minutes after the
    # fixture's last heartbeat (the threshold edge = still progress).
    run bash "$script" --ledger "$fixture/ledger.jsonl" --live "$fixture/live.json" --now 1785545700
    [ "$status" -eq 0 ]
    diff -u "$fixture/snapshot.txt" <(printf '%s\n' "$output")
}

@test "AC1: live card exposes at least 10 required execution fields" {
    bash "$script" --ledger "$fixture/ledger.jsonl" --live "$fixture/live.json" > "$tmp/dash.txt"
    [ $? -eq 0 ]
    found=0
    for field in state node_id profile turns context_tokens ttft_ms decode_tokens_per_second cache_hit_rate tool_ms test_ms repair_count queue_ms started_ms last_heartbeat_ms; do
        if grep -q "^  ${field}:" "$tmp/dash.txt"; then
            found=$((found + 1))
        else
            echo "missing live field: $field" >&2
        fi
    done
    [ "$found" -ge 10 ]
}

@test "AC2: historical output reports p50, p90 and p95 issue duration" {
    for i in 1 2 3 4 5; do
        d=$((i * 1000))
        printf '{"issue_id":"a%s","ts":"2026-08-01T00:00:00Z","model_family":"qwen3.8","profile":"p","duration_ms":%d,"succeeded":true}\n' "$i" "$d" >> "$tmp/ledger.jsonl"
    done
    bash "$script" --ledger "$tmp/ledger.jsonl" > "$tmp/dash.txt"
    [ $? -eq 0 ]
    grep -q "^  p50_ms: 3000$" "$tmp/dash.txt"
    grep -q "^  p90_ms: 5000$" "$tmp/dash.txt"
    grep -q "^  p95_ms: 5000$" "$tmp/dash.txt"
    # issue #3723: the expected duration (mean) next to the percentiles.
    grep -q "^  mean_ms: 3000$" "$tmp/dash.txt"
}

@test "AC3: fewer than 20 configured samples preserve static profile selection" {
    bash "$script" --ledger "$fixture/ledger-small.jsonl" > "$tmp/dash.txt"
    [ $? -eq 0 ]
    grep -q "^  source: static$" "$tmp/dash.txt"
    grep -q "^  profile: qwen3.8-coding-local$" "$tmp/dash.txt"
}

@test "AC4: advice never changes model_family away from qwen3.8" {
    # Full ledger: an eligible claude candidate exists but the benchmark
    # selection must stay inside the locked family.
    bash "$script" --ledger "$fixture/ledger.jsonl" > "$tmp/dash.txt"
    [ $? -eq 0 ]
    grep -q "^model-family: qwen3.8$" "$tmp/dash.txt"
    grep -q "^  source: benchmark$" "$tmp/dash.txt"
    grep -q "^  profile: qwen3.8-27b-q4/rtx4090$" "$tmp/dash.txt"

    # And a fully eligible claude-only ledger still cannot move the family.
    for i in $(seq 1 25); do
        printf '{"issue_id":"c%s","ts":"2026-08-01T00:00:00Z","model_family":"claude","profile":"claude-sonnet-5/provider-api","duration_ms":%d,"succeeded":true}\n' "$i" $((i * 1000)) >> "$tmp/claude.jsonl"
    done
    bash "$script" --ledger "$tmp/claude.jsonl" > "$tmp/dash2.txt"
    [ $? -eq 0 ]
    grep -q "^  model-family: qwen3.8$" "$tmp/dash2.txt"
    grep -q "^  source: static$" "$tmp/dash2.txt"
    grep -q "^  profile: qwen3.8-coding-local$" "$tmp/dash2.txt"
}

@test "ledger with a missing required key is rejected" {
    printf '{"issue_id":"1","ts":"t","model_family":"qwen3.8"}\n' > "$tmp/bad.jsonl"
    run bash "$script" --ledger "$tmp/bad.jsonl"
    [ "$status" -ne 0 ]
    [[ "$output" == *"missing required key"* ]]
}

@test "ledger with a duplicate issue_id is rejected" {
    printf '{"issue_id":"1","ts":"t","model_family":"qwen3.8","profile":"p","duration_ms":5,"succeeded":true}\n{"issue_id":"1","ts":"t","model_family":"qwen3.8","profile":"p","duration_ms":6,"succeeded":true}\n' > "$tmp/dup.jsonl"
    run bash "$script" --ledger "$tmp/dup.jsonl"
    [ "$status" -ne 0 ]
    [[ "$output" == *"duplicate issue_id"* ]]
}

@test "live record with an out-of-range cache_hit_rate is rejected" {
    printf '{"issue_id":"1","ts":"t","model_family":"qwen3.8","profile":"p","duration_ms":5,"succeeded":true}\n' > "$tmp/ok.jsonl"
    printf '{"issue_id":"1","state":"running","node_id":"n","profile":"p","turns":1,"context_tokens":1,"ttft_ms":1,"decode_tokens_per_second":1,"cache_hit_rate":1.5,"tool_ms":1,"test_ms":1,"repair_count":0,"queue_ms":1,"started_ms":1,"last_heartbeat_ms":1}\n' > "$tmp/badlive.json"
    run bash "$script" --ledger "$tmp/ok.jsonl" --live "$tmp/badlive.json"
    [ "$status" -ne 0 ]
    [[ "$output" == *"cache_hit_rate"* ]]
}

@test "AC5: stalled run is reported as no output for N minutes (issue #3723)" {
    # now 1785600000 (epoch s) minus the fixture heartbeat 1785545400000 ms
    # is 54,600,000 ms of silence = 910 minutes, far past the 5-minute bar.
    bash "$script" --ledger "$fixture/ledger.jsonl" --live "$fixture/live.json" --now 1785600000 > "$tmp/dash.txt"
    [ $? -eq 0 ]
    grep -q "^  liveness: no output for 910 minutes$" "$tmp/dash.txt"
}

@test "AC6: silence exactly at the threshold still reports progress (issue #3723)" {
    # now 1785545700 is exactly 300,000 ms (5 minutes) after the fixture
    # heartbeat: the edge is progress, and elapsed time is a multiple of the
    # ledger mean (3,300,000 / 63,936 = 51.61).
    bash "$script" --ledger "$fixture/ledger.jsonl" --live "$fixture/live.json" --now 1785545700 > "$tmp/dash.txt"
    [ $? -eq 0 ]
    grep -q "^  liveness: progressing$" "$tmp/dash.txt"
    grep -q "^  elapsed_ratio: 51.61$" "$tmp/dash.txt"
}

@test "live record missing started_ms is rejected" {
    printf '{"issue_id":"1","ts":"t","model_family":"qwen3.8","profile":"p","duration_ms":5,"succeeded":true}\n' > "$tmp/ok.jsonl"
    jq 'del(.started_ms)' "$fixture/live.json" > "$tmp/badlive.json"
    run bash "$script" --ledger "$tmp/ok.jsonl" --live "$tmp/badlive.json"
    [ "$status" -ne 0 ]
    [[ "$output" == *"started_ms"* ]]
}

@test "non-integer --now is a usage error" {
    run bash "$script" --ledger "$fixture/ledger.jsonl" --now now
    [ "$status" -ne 0 ]
    [[ "$output" == *"positive epoch-seconds integer"* ]]
}
