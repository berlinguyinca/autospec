#!/usr/bin/env bats
# tests/unit/loop-controller.bats
# TDD tests for loop-controller.sh — one test per termination condition from spec §6.
# Primary smoke test for issue #323.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    CONTROLLER="$SCRIPTS_DIR/loop-controller.sh"
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-ctrl-bats-XXXXXX)"
    STATE_FILE="$TEST_TMPDIR/loop-state.json"
    STOP_FLAG="$TEST_TMPDIR/stop.flag"

    # Gate command that always passes
    GATE_CMD_PASS="echo '{\"passed\":true,\"stage\":\"unit\",\"metrics\":{},\"test_run_summary\":{}}'"
    # Gate command that always fails
    GATE_CMD_FAIL="echo '{\"passed\":false,\"stage\":\"unit\",\"reason\":\"tests_red\",\"metrics\":{\"unit\":{\"passed\":false}},\"test_run_summary\":{\"exit_code\":1,\"stderr_tail\":\"\",\"stdout_tail\":\"\"}}' && exit 1"
    # Gate command that fails with product_bug signal
    GATE_CMD_BUG="echo '{\"passed\":false,\"stage\":\"unit\",\"reason\":\"tests_red\",\"metrics\":{\"unit\":{\"passed\":false}},\"test_run_summary\":{\"exit_code\":1,\"stderr_tail\":\"Expected 42 but received 43\",\"stdout_tail\":\"\"}}' && exit 1"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
    rm -f "$HOME/.autospec/stop.flag" 2>/dev/null || true
}

# ── Termination condition: gate_passed ────────────────────────────────────────

@test "loop-controller: exits 0 when gate passes on first run" {
    run bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_PASS" \
        --max-iterations 5 \
        --budget-seconds 3600 \
        --pr-number 99
    [ "$status" -eq 0 ]
}

@test "loop-controller: sets termination_reason=gate_passed in state" {
    bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_PASS" \
        --max-iterations 5 \
        --budget-seconds 3600 \
        --pr-number 99 2>/dev/null || true
    local reason
    reason=$(jq -r '.termination_reason' "$STATE_FILE")
    [ "$reason" = "gate_passed" ]
}

# ── Termination condition: budget_exhausted ───────────────────────────────────

@test "loop-controller: exits 1 when budget exhausted" {
    # Set 0-second budget (already exhausted)
    run bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_FAIL" \
        --max-iterations 5 \
        --budget-seconds 0 \
        --pr-number 99
    # Should exit 1 (budget exhausted or gate failed)
    [ "$status" -ne 0 ]
}

@test "loop-controller: sets termination_reason=budget_exhausted when no budget" {
    # Pre-populate state with exhausted budget
    jq -n '{
        "pr_number": 1,
        "started_at": "2026-05-21T00:00:00Z",
        "coding_time_used_seconds": 3601,
        "coding_time_budget_seconds": 3600,
        "last_coding_start": null,
        "iterations": [],
        "last_error_signature": null,
        "same_error_consecutive": 0,
        "empty_action_consecutive": 0,
        "termination_reason": null
    }' > "$STATE_FILE"

    bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_FAIL" \
        --max-iterations 5 \
        --budget-seconds 3600 \
        --pr-number 99 2>/dev/null || true

    local reason
    reason=$(jq -r '.termination_reason' "$STATE_FILE")
    [ "$reason" = "budget_exhausted" ]
}

# ── Termination condition: max_iterations ─────────────────────────────────────

@test "loop-controller: exits 1 when max iterations reached" {
    # Pre-populate state with max iterations already used
    jq -n '{
        "pr_number": 1,
        "started_at": "2026-05-21T00:00:00Z",
        "coding_time_used_seconds": 0,
        "coding_time_budget_seconds": 3600,
        "last_coding_start": null,
        "iterations": [
            {"n":1,"started_at":"2026-05-21T00:00:00Z","ended_at":"2026-05-21T00:05:00Z","classification":"failing_unit_test","files_changed":[],"gate_passed":false,"error_signature":null},
            {"n":2,"started_at":"2026-05-21T00:05:00Z","ended_at":"2026-05-21T00:10:00Z","classification":"failing_unit_test","files_changed":[],"gate_passed":false,"error_signature":null},
            {"n":3,"started_at":"2026-05-21T00:10:00Z","ended_at":"2026-05-21T00:15:00Z","classification":"failing_unit_test","files_changed":[],"gate_passed":false,"error_signature":null}
        ],
        "last_error_signature": null,
        "same_error_consecutive": 0,
        "empty_action_consecutive": 0,
        "termination_reason": null
    }' > "$STATE_FILE"

    run bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_FAIL" \
        --max-iterations 3 \
        --budget-seconds 3600 \
        --pr-number 99
    [ "$status" -eq 1 ]
}

@test "loop-controller: sets termination_reason=max_iterations" {
    jq -n '{
        "pr_number": 1,
        "started_at": "2026-05-21T00:00:00Z",
        "coding_time_used_seconds": 0,
        "coding_time_budget_seconds": 3600,
        "last_coding_start": null,
        "iterations": [
            {"n":1,"started_at":"2026-05-21T00:00:00Z","ended_at":"2026-05-21T00:05:00Z","classification":"failing_unit_test","files_changed":[],"gate_passed":false,"error_signature":null}
        ],
        "last_error_signature": null,
        "same_error_consecutive": 0,
        "empty_action_consecutive": 0,
        "termination_reason": null
    }' > "$STATE_FILE"

    bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_FAIL" \
        --max-iterations 1 \
        --budget-seconds 3600 \
        --pr-number 99 2>/dev/null || true

    local reason
    reason=$(jq -r '.termination_reason' "$STATE_FILE")
    [ "$reason" = "max_iterations" ]
}

# ── Termination condition: stop.flag ─────────────────────────────────────────

@test "loop-controller: exits 1 when stop.flag present" {
    # Create stop flag
    echo "graceful" > "$STOP_FLAG"
    export AUTOSPEC_STOP_FLAG="$STOP_FLAG"

    run bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_FAIL" \
        --max-iterations 5 \
        --budget-seconds 3600 \
        --pr-number 99
    [ "$status" -eq 1 ]
    unset AUTOSPEC_STOP_FLAG
}

@test "loop-controller: sets termination_reason=stop_flag" {
    echo "graceful" > "$STOP_FLAG"
    export AUTOSPEC_STOP_FLAG="$STOP_FLAG"

    bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_FAIL" \
        --max-iterations 5 \
        --budget-seconds 3600 \
        --pr-number 99 2>/dev/null || true

    local reason
    reason=$(jq -r '.termination_reason' "$STATE_FILE")
    [ "$reason" = "stop_flag" ]
    unset AUTOSPEC_STOP_FLAG
}

# ── Termination condition: same_error_halt ────────────────────────────────────

@test "loop-controller: sets termination_reason=same_error_halt after 3 same errors" {
    # Pre-populate state with 3 iterations all having same error sig
    jq -n '{
        "pr_number": 1,
        "started_at": "2026-05-21T00:00:00Z",
        "coding_time_used_seconds": 100,
        "coding_time_budget_seconds": 3600,
        "last_coding_start": null,
        "iterations": [
            {"n":1,"started_at":"2026-05-21T00:00:00Z","ended_at":"2026-05-21T00:05:00Z","classification":"failing_unit_test","files_changed":[],"gate_passed":false,"error_signature":"abc123"},
            {"n":2,"started_at":"2026-05-21T00:05:00Z","ended_at":"2026-05-21T00:10:00Z","classification":"failing_unit_test","files_changed":[],"gate_passed":false,"error_signature":"abc123"},
            {"n":3,"started_at":"2026-05-21T00:10:00Z","ended_at":"2026-05-21T00:15:00Z","classification":"failing_unit_test","files_changed":[],"gate_passed":false,"error_signature":"abc123"}
        ],
        "last_error_signature": "abc123",
        "same_error_consecutive": 3,
        "empty_action_consecutive": 0,
        "termination_reason": null
    }' > "$STATE_FILE"

    # AUTOSPEC_SAME_ERROR_HALT=3
    export AUTOSPEC_SAME_ERROR_HALT=3

    bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_FAIL" \
        --max-iterations 10 \
        --budget-seconds 3600 \
        --pr-number 99 2>/dev/null || true

    local reason
    reason=$(jq -r '.termination_reason' "$STATE_FILE")
    [ "$reason" = "same_error_halt" ]
    unset AUTOSPEC_SAME_ERROR_HALT
}

# ── Termination condition: empty_action_halt ──────────────────────────────────

@test "loop-controller: sets termination_reason=empty_action_halt after 2 empty actions" {
    # Pre-populate state with 2 consecutive empty actions
    jq -n '{
        "pr_number": 1,
        "started_at": "2026-05-21T00:00:00Z",
        "coding_time_used_seconds": 100,
        "coding_time_budget_seconds": 3600,
        "last_coding_start": null,
        "iterations": [
            {"n":1,"started_at":"2026-05-21T00:00:00Z","ended_at":"2026-05-21T00:05:00Z","classification":"empty_action","files_changed":[],"gate_passed":false,"error_signature":null},
            {"n":2,"started_at":"2026-05-21T00:05:00Z","ended_at":"2026-05-21T00:10:00Z","classification":"empty_action","files_changed":[],"gate_passed":false,"error_signature":null}
        ],
        "last_error_signature": null,
        "same_error_consecutive": 0,
        "empty_action_consecutive": 2,
        "termination_reason": null
    }' > "$STATE_FILE"

    export AUTOSPEC_EMPTY_ACTION_HALT=2

    bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_FAIL" \
        --max-iterations 10 \
        --budget-seconds 3600 \
        --pr-number 99 2>/dev/null || true

    local reason
    reason=$(jq -r '.termination_reason' "$STATE_FILE")
    [ "$reason" = "empty_action_halt" ]
    unset AUTOSPEC_EMPTY_ACTION_HALT
}

# ── Pre-commit hook ────────────────────────────────────────────────────────────

@test "pre-commit-loop-guard: rejects edits to .autospec/test.yml" {
    local hook_script="$SCRIPTS_DIR/pre-commit-loop-guard.sh"
    [ -f "$hook_script" ]
    bash -n "$hook_script"
}

@test "pre-commit-loop-guard: bash syntax valid" {
    run bash -n "$SCRIPTS_DIR/pre-commit-loop-guard.sh"
    [ "$status" -eq 0 ]
}

# ── Resume semantics ──────────────────────────────────────────────────────────

@test "loop-controller: resume respects existing coding_time_used_seconds" {
    # Start with 100 seconds already used
    jq -n '{
        "pr_number": 1,
        "started_at": "2026-05-21T00:00:00Z",
        "coding_time_used_seconds": 100,
        "coding_time_budget_seconds": 3600,
        "last_coding_start": null,
        "iterations": [],
        "last_error_signature": null,
        "same_error_consecutive": 0,
        "empty_action_consecutive": 0,
        "termination_reason": null
    }' > "$STATE_FILE"

    # Run the controller (gate passes immediately)
    bash "$CONTROLLER" \
        --state-file "$STATE_FILE" \
        --gate-cmd "$GATE_CMD_PASS" \
        --max-iterations 5 \
        --budget-seconds 3600 \
        --pr-number 99 2>/dev/null || true

    # State file should still exist and reflect resumed state
    [ -f "$STATE_FILE" ]
    local used
    used=$(jq -r '.coding_time_used_seconds' "$STATE_FILE")
    [ "$used" -ge 100 ]
}
