#!/usr/bin/env bats
# run-batch-start.bats — tests for run-batch-start.sh (write/read of ~/.autospec/.run-batch-start)

SCRIPT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/run-batch-start.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$TEST_TMP"
}

teardown() {
    rm -rf "$TEST_TMP"
}

@test "run-batch-start.sh is executable" {
    [ -x "$SCRIPT" ]
}

@test "run-batch-start.sh --help exits 0 and prints Usage" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage:"* ]]
}

@test "--write creates the batch-start file with a UTC ISO timestamp" {
    run bash "$SCRIPT" --write
    [ "$status" -eq 0 ]
    [ -f "$TEST_TMP/.run-batch-start" ]
    # ISO 8601 UTC, e.g. 2026-05-24T18:42:11Z
    run cat "$TEST_TMP/.run-batch-start"
    [[ "$output" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]]
}

@test "--read echoes the previously written timestamp" {
    printf '2026-05-24T00:00:00Z\n' > "$TEST_TMP/.run-batch-start"
    run bash "$SCRIPT" --read
    [ "$status" -eq 0 ]
    [ "$output" = "2026-05-24T00:00:00Z" ]
}

@test "--read with no file falls back to epoch sentinel and exits 0" {
    run bash "$SCRIPT" --read
    [ "$status" -eq 0 ]
    [ "$output" = "1970-01-01T00:00:00Z" ]
}

@test "--write does not overwrite an existing batch-start (idempotent within a run)" {
    printf '2026-05-24T00:00:00Z\n' > "$TEST_TMP/.run-batch-start"
    run bash "$SCRIPT" --write
    [ "$status" -eq 0 ]
    run cat "$TEST_TMP/.run-batch-start"
    [ "$output" = "2026-05-24T00:00:00Z" ]
}

@test "--write --force overwrites an existing batch-start" {
    printf '2026-05-24T00:00:00Z\n' > "$TEST_TMP/.run-batch-start"
    run bash "$SCRIPT" --write --force
    [ "$status" -eq 0 ]
    run cat "$TEST_TMP/.run-batch-start"
    [ "$output" != "2026-05-24T00:00:00Z" ]
}
