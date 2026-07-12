#!/usr/bin/env bats

@test "context monitor Rust parity smoke" {
    run cargo test context_monitor
    [ "$status" -eq 0 ]
    [[ "$output" == *"context_monitor_scripted_sequence_matches_python_engine_parity ... ok"* ]]
}
