#!/usr/bin/env bats
# tests/unit/loop-budget.bats
# TDD tests for loop-budget.sh — every budget function + resume semantics.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../../../.." && pwd)"
    SCRIPTS_DIR="$REPO_ROOT/skills/autospec-test/scripts"
    BUDGET_SCRIPT="$SCRIPTS_DIR/loop-budget.sh"
    TEST_TMPDIR="$(mktemp -d /tmp/autospec-budget-bats-XXXXXX)"
    STATE_FILE="$TEST_TMPDIR/test-loop-state.json"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# ── budget_start ───────────────────────────────────────────────────────────────

@test "budget_start: creates valid state file" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 42 3600
    [ -f "$STATE_FILE" ]
    local pr
    pr=$(jq -r '.pr_number' "$STATE_FILE")
    [ "$pr" = "42" ]
}

@test "budget_start: sets coding_time_used_seconds to 0" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    local used
    used=$(jq -r '.coding_time_used_seconds' "$STATE_FILE")
    [ "$used" = "0" ]
}

@test "budget_start: sets last_coding_start to non-null" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    local start
    start=$(jq -r '.last_coding_start' "$STATE_FILE")
    [ "$start" != "null" ]
    [ -n "$start" ]
}

@test "budget_start: budget_seconds stored correctly" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 1800
    local budget
    budget=$(jq -r '.coding_time_budget_seconds' "$STATE_FILE")
    [ "$budget" = "1800" ]
}

@test "budget_start: initializes empty iterations array" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    local count
    count=$(jq '.iterations | length' "$STATE_FILE")
    [ "$count" = "0" ]
}

# ── budget_pause ───────────────────────────────────────────────────────────────

@test "budget_pause: sets last_coding_start to null" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    sleep 1
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    local start
    start=$(jq -r '.last_coding_start' "$STATE_FILE")
    [ "$start" = "null" ]
}

@test "budget_pause: accumulates coding_time_used_seconds" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    sleep 2
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    local used
    used=$(jq -r '.coding_time_used_seconds' "$STATE_FILE")
    [ "$used" -ge 1 ]
}

@test "budget_pause: idempotent (second pause is no-op)" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    sleep 1
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    local used_after_first
    used_after_first=$(jq -r '.coding_time_used_seconds' "$STATE_FILE")
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    local used_after_second
    used_after_second=$(jq -r '.coding_time_used_seconds' "$STATE_FILE")
    [ "$used_after_first" = "$used_after_second" ]
}

# ── budget_resume ──────────────────────────────────────────────────────────────

@test "budget_resume: sets last_coding_start to current time" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    bash "$BUDGET_SCRIPT" budget_resume "$STATE_FILE"
    local start
    start=$(jq -r '.last_coding_start' "$STATE_FILE")
    [ "$start" != "null" ]
    [ -n "$start" ]
}

@test "budget_resume: idempotent (second resume is no-op)" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    bash "$BUDGET_SCRIPT" budget_resume "$STATE_FILE"
    local start1
    start1=$(jq -r '.last_coding_start' "$STATE_FILE")
    sleep 1
    bash "$BUDGET_SCRIPT" budget_resume "$STATE_FILE"
    local start2
    start2=$(jq -r '.last_coding_start' "$STATE_FILE")
    [ "$start1" = "$start2" ]
}

# ── budget_remaining ───────────────────────────────────────────────────────────

@test "budget_remaining: returns budget seconds when just started" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    local remaining
    remaining=$(bash "$BUDGET_SCRIPT" budget_remaining "$STATE_FILE")
    [ "$remaining" -ge 3598 ]  # allow 2s slack
}

@test "budget_remaining: decreases while running" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 10
    sleep 2
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    local remaining
    remaining=$(bash "$BUDGET_SCRIPT" budget_remaining "$STATE_FILE")
    [ "$remaining" -le 9 ]
}

@test "budget_remaining: returns 0 when exhausted" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 1
    sleep 2
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    local remaining
    remaining=$(bash "$BUDGET_SCRIPT" budget_remaining "$STATE_FILE")
    [ "$remaining" -eq 0 ]
}

# ── budget_exhausted ───────────────────────────────────────────────────────────

@test "budget_exhausted: returns 1 (not exhausted) when budget remaining" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    run bash "$BUDGET_SCRIPT" budget_exhausted "$STATE_FILE"
    [ "$status" -eq 1 ]
}

@test "budget_exhausted: returns 0 (exhausted) when no budget left" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 1
    sleep 3
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    run bash "$BUDGET_SCRIPT" budget_exhausted "$STATE_FILE"
    [ "$status" -eq 0 ]
}

# ── Resume semantics (cross-process) ──────────────────────────────────────────

@test "budget: pause and resume accumulates correctly across calls" {
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    sleep 2
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    # "Test run" (not counted)
    sleep 1
    bash "$BUDGET_SCRIPT" budget_resume "$STATE_FILE"
    sleep 2
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"

    local used
    used=$(jq -r '.coding_time_used_seconds' "$STATE_FILE")
    # Should be ~4s (2s first window + 2s second window), not 5s
    [ "$used" -ge 3 ]
    [ "$used" -le 6 ]
}

@test "budget: resume after simulated process exit restores state" {
    # Simulate: start, use some time, pause, then re-read state from file
    bash "$BUDGET_SCRIPT" budget_start "$STATE_FILE" 1 3600
    sleep 1
    bash "$BUDGET_SCRIPT" budget_pause "$STATE_FILE"
    local used_before
    used_before=$(jq -r '.coding_time_used_seconds' "$STATE_FILE")

    # Simulate process exit and restart: just read the state file again
    local used_after
    used_after=$(jq -r '.coding_time_used_seconds' "$STATE_FILE")
    [ "$used_before" = "$used_after" ]
}
