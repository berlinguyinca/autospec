#!/usr/bin/env bats
# tests/unit/test_autospec_usage_limit.bats — deterministic usage-limit resume helper.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-usage-limit.sh"
    TEST_TMP="$(mktemp -d)"
    export HOME="$TEST_TMP/home"
    mkdir -p "$HOME" "$TEST_TMP/repo"
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "arm records a waiting resume state without launching the daemon" {
    run bash "$SCRIPT" arm \
        --run-id test-run \
        --harness codex \
        --repo-dir "$TEST_TMP/repo" \
        --command 'printf resumed > resumed.txt' \
        --wait-seconds 600 \
        --no-daemon

    [ "$status" -eq 0 ]
    state="$HOME/.autospec/usage-limits/test-run.json"
    [ -f "$state" ]
    jq -e '.status == "waiting" and .harness == "codex" and .interval_seconds == 300' "$state" >/dev/null
    [ ! -f "$TEST_TMP/repo/resumed.txt" ]
}

@test "poll before resume time leaves state waiting and exits 2" {
    bash "$SCRIPT" arm \
        --run-id test-run \
        --harness claude \
        --repo-dir "$TEST_TMP/repo" \
        --command 'printf resumed > resumed.txt' \
        --wait-seconds 600 \
        --no-daemon >/dev/null

    run bash "$SCRIPT" poll --run-id test-run --foreground

    [ "$status" -eq 2 ]
    jq -e '.status == "waiting" and .last_poll_at != ""' "$HOME/.autospec/usage-limits/test-run.json" >/dev/null
    [ ! -f "$TEST_TMP/repo/resumed.txt" ]
}

@test "poll after resume time runs the command once and marks succeeded" {
    bash "$SCRIPT" arm \
        --run-id test-run \
        --harness opencode \
        --repo-dir "$TEST_TMP/repo" \
        --command 'printf resumed > resumed.txt' \
        --wait-seconds 0 \
        --no-daemon >/dev/null

    run bash "$SCRIPT" poll --run-id test-run --foreground

    [ "$status" -eq 0 ]
    [ "$(cat "$TEST_TMP/repo/resumed.txt")" = "resumed" ]
    jq -e '.status == "succeeded" and .attempts == 1 and .resumed_at != ""' "$HOME/.autospec/usage-limits/test-run.json" >/dev/null

    run bash "$SCRIPT" poll --run-id test-run --foreground
    [ "$status" -eq 0 ]
    jq -e '.attempts == 1' "$HOME/.autospec/usage-limits/test-run.json" >/dev/null
}

@test "clear removes a recorded resume state" {
    bash "$SCRIPT" arm \
        --run-id test-run \
        --harness codex \
        --repo-dir "$TEST_TMP/repo" \
        --command 'printf resumed > resumed.txt' \
        --wait-seconds 600 \
        --no-daemon >/dev/null

    run bash "$SCRIPT" clear --run-id test-run

    [ "$status" -eq 0 ]
    [ ! -f "$HOME/.autospec/usage-limits/test-run.json" ]
}
