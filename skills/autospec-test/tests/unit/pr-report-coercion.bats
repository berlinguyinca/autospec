#!/usr/bin/env bats
# skills/autospec-test/tests/unit/pr-report-coercion.bats
#
# jq alternative-operator ("//") coercion guard for pr-report.sh.
#
# jq's "//" treats a literal `false` the same as an absent key, so
# `.stage2_5.metrics.F.passed // true` (etc.) silently rendered a ✅ for a
# metric that actually failed. Covers all five affected fields:
#   stage2_5.metrics.{F,G,H,I}.passed and stage2_5.seeds_ok.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    BIN="$REPO_ROOT/skills/autospec-test/scripts/pr-report.sh"

    TEST_TMPDIR="$(mktemp -d /tmp/autospec-pr-report-bats-XXXXXX)"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# Build a gate JSON where every stage2_5 metric is present, not skipped, and
# passing (✅ baseline). $1 overrides one field to a literal false.
make_gate_json() {
    local override_path="$1"
    jq -n --arg path "$override_path" '
        {
            "target": "stub",
            "overall_passed": false,
            "stage1": {"passed": true},
            "stage2_5": {
                "passed": false,
                "skipped": false,
                "metrics": {
                    "F": {"passed": true, "skipped": false},
                    "G": {"passed": true, "skipped": false},
                    "H": {"passed": true, "skipped": false},
                    "I": {"passed": true, "skipped": false, "passed_count": 3, "total_count": 3}
                },
                "seeds_ok": true,
                "seeds_count": 2
            }
        }
        | setpath(($path | split(".")); false)
    '
}

@test "pr-report: stage2_5.metrics.F.passed:false renders a failing icon for Metric F" {
    local gate_file="$TEST_TMPDIR/gate.json"
    make_gate_json "stage2_5.metrics.F.passed" > "$gate_file"
    run bash "$BIN" --gate-json "$gate_file"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '\*\*Metric F — Structural invariants:\*\* ❌'
}

@test "pr-report: stage2_5.metrics.G.passed:false renders a failing icon for Metric G" {
    local gate_file="$TEST_TMPDIR/gate.json"
    make_gate_json "stage2_5.metrics.G.passed" > "$gate_file"
    run bash "$BIN" --gate-json "$gate_file"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '\*\*Metric G — Window contracts:\*\* ❌'
}

@test "pr-report: stage2_5.metrics.H.passed:false renders a failing icon for Metric H" {
    local gate_file="$TEST_TMPDIR/gate.json"
    make_gate_json "stage2_5.metrics.H.passed" > "$gate_file"
    run bash "$BIN" --gate-json "$gate_file"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '\*\*Metric H — Extended crawler:\*\* ❌'
}

@test "pr-report: stage2_5.metrics.I.passed:false renders a failing icon for Metric I" {
    local gate_file="$TEST_TMPDIR/gate.json"
    make_gate_json "stage2_5.metrics.I.passed" > "$gate_file"
    run bash "$BIN" --gate-json "$gate_file"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '\*\*Metric I — Contract symmetry:\*\* ❌'
}

@test "pr-report: stage2_5.seeds_ok:false renders a failing icon for seeds handshake" {
    local gate_file="$TEST_TMPDIR/gate.json"
    make_gate_json "stage2_5.seeds_ok" > "$gate_file"
    run bash "$BIN" --gate-json "$gate_file"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'Edge-case seeds.*❌'
}

# ── Absent-key default preserved ─────────────────────────────────────────────

@test "pr-report: absent stage2_5.metrics.F.passed key still defaults to a passing icon" {
    local gate_file="$TEST_TMPDIR/gate.json"
    jq -n '{
        "target": "stub",
        "overall_passed": true,
        "stage1": {"passed": true},
        "stage2_5": {
            "passed": true,
            "skipped": false,
            "metrics": {
                "F": {"skipped": false},
                "G": {"passed": true, "skipped": false},
                "H": {"passed": true, "skipped": false},
                "I": {"passed": true, "skipped": false, "passed_count": 3, "total_count": 3}
            },
            "seeds_count": 0
        }
    }' > "$gate_file"
    run bash "$BIN" --gate-json "$gate_file"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '\*\*Metric F — Structural invariants:\*\* ✅'
}
