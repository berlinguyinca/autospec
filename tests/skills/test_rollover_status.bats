#!/usr/bin/env bats
# tests/skills/test_rollover_status.bats — tests for autospec-rollover-status/show.sh

SHOW_SH="${BATS_TEST_DIRNAME}/../../skills/autospec-rollover-status/show.sh"

setup() {
    # Use a clean temp HOME for each test
    export ORIG_HOME="$HOME"
    export HOME="$(mktemp -d)"
}

teardown() {
    rm -rf "$HOME"
    export HOME="$ORIG_HOME"
}

@test "test_prints_no_logs_message_when_dir_missing" {
    # HOME points to empty dir with no .autospec/monitors
    run bash "$SHOW_SH"
    [ "$status" -eq 0 ]
    [[ "$output" == *"No monitor logs"* ]]
}

@test "test_prints_no_sessions_message_when_dir_empty" {
    mkdir -p "$HOME/.autospec/monitors"
    run bash "$SHOW_SH"
    [ "$status" -eq 0 ]
    [[ "$output" == *"No monitor sessions active"* ]]
}

@test "test_tails_most_recent_log_file" {
    mkdir -p "$HOME/.autospec/monitors"
    # Create two log files; the newer one should be used
    echo '{"event":"usage","ts":"2026-05-31T10:00:00Z","used":5000,"max":100000,"pct":0.05,"model":"test"}' \
        > "$HOME/.autospec/monitors/old-session.log"
    sleep 0.1
    echo '{"event":"usage","ts":"2026-05-31T11:00:00Z","used":80000,"max":100000,"pct":0.80,"model":"test"}' \
        > "$HOME/.autospec/monitors/new-session.log"

    run bash "$SHOW_SH"
    [ "$status" -eq 0 ]
    [[ "$output" == *"new-session.log"* ]]
    [[ "$output" != *"old-session.log"* ]]
}

@test "test_filters_to_pct_and_rollover_events" {
    mkdir -p "$HOME/.autospec/monitors"
    {
        echo '{"event":"start","ts":"2026-05-31T10:00:00Z","pid":12345}'
        echo '{"event":"usage","ts":"2026-05-31T10:01:00Z","used":50000,"max":100000,"pct":0.50,"model":"test"}'
        echo '{"event":"compact","ts":"2026-05-31T10:02:00Z"}'
        echo '{"event":"stop","ts":"2026-05-31T10:03:00Z"}'
    } > "$HOME/.autospec/monitors/test-session.log"

    run bash "$SHOW_SH"
    [ "$status" -eq 0 ]
    # Should include usage and compact events
    [[ "$output" == *"usage"* ]] || [[ "$output" == *"50.0%"* ]]
    [[ "$output" == *"compact"* ]]
    # Should NOT show the start/stop events in the event lines
    # (they don't match the filter regex — only the log path line could match)
}

@test "test_shows_current_context_percentage" {
    mkdir -p "$HOME/.autospec/monitors"
    echo '{"event":"usage","ts":"2026-05-31T10:00:00Z","used":75000,"max":100000,"pct":0.75,"model":"test"}' \
        > "$HOME/.autospec/monitors/my-session.log"

    run bash "$SHOW_SH"
    [ "$status" -eq 0 ]
    [[ "$output" == *"75.0%"* ]]
}
