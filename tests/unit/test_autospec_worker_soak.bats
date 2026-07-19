#!/usr/bin/env bats
# tests/unit/test_autospec_worker_soak.bats — fixture soak for concurrent workers.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-worker-soak.sh"
}

@test "fixture soak: 5 workers claim and merge each issue exactly once" {
    run bash "$SCRIPT" --workers 5 --issues 8

    [ "$status" -eq 0 ]
    summary="$output"
    run jq -r '.status' <<<"$summary"
    [ "$output" = "pass" ]
    run jq -r '.workers' <<<"$summary"
    [ "$output" = "5" ]
    run jq -r '.issues' <<<"$summary"
    [ "$output" = "8" ]
    run jq -r '.claims' <<<"$summary"
    [ "$output" = "8" ]
    run jq -r '.duplicate_claims' <<<"$summary"
    [ "$output" = "0" ]
    run jq -r '.stale_active_labels' <<<"$summary"
    [ "$output" = "0" ]
    run jq -r '.queue_labels_remaining' <<<"$summary"
    [ "$output" = "0" ]
}

@test "fixture soak: 25 workers keep high-contention claims unique" {
    run bash "$SCRIPT" --workers 25 --issues 50

    [ "$status" -eq 0 ]
    summary="$output"
    run jq -r '.claims' <<<"$summary"
    [ "$output" = "50" ]
    run jq -r '.duplicate_claims' <<<"$summary"
    [ "$output" = "0" ]
    run jq -r '.stale_active_labels' <<<"$summary"
    [ "$output" = "0" ]
    run jq -r '.queue_labels_remaining' <<<"$summary"
    [ "$output" = "0" ]
}

@test "fixture soak routes acquire and release through AUTOSPEC_BIN" {
    export AUTOSPEC_CALLS="$BATS_TEST_TMPDIR/autospec-calls.log"
    export AUTOSPEC_REAL_BIN="$REPO_ROOT/target/debug/autospec"
    export AUTOSPEC_BIN="$BATS_TEST_TMPDIR/autospec"
    : > "$AUTOSPEC_CALLS"
    cat > "$AUTOSPEC_BIN" <<'EOF'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$AUTOSPEC_CALLS"
exec "$AUTOSPEC_REAL_BIN" "$@"
EOF
    chmod +x "$AUTOSPEC_BIN"

    run bash "$SCRIPT" --workers 2 --issues 2

    [ "$status" -eq 0 ]
    run grep -F 'claim acquire ' "$AUTOSPEC_CALLS"
    [ "$status" -eq 0 ]
    run grep -F 'claim release ' "$AUTOSPEC_CALLS"
    [ "$status" -eq 0 ]
}

@test "fixture soak: invalid worker count exits 2" {
    run bash "$SCRIPT" --workers 0 --issues 2

    [ "$status" -eq 2 ]
    [[ "$output" == *"--workers"* ]]
}
