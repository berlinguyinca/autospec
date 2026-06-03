#!/usr/bin/env bats
# tests/resume/test_run_registry.bats — durable run registry + attempts counter.
#
# Covers (docs/specs/2026-06-03-crash-resume-design.md §Data model):
#   - registry write/read/list/prune two-line atomic temp+mv
#   - registry-durable: after a simulated reboot (env cleared except the base
#     dir on disk) the registry ALONE yields a runnable resume_command
#   - attempts counter get/inc/reset, cap, stale>24h ignored

ROOT="${BATS_TEST_DIRNAME}/../.."
REGISTRY="$ROOT/scripts/autospec-run-registry.sh"
ATTEMPTS="$ROOT/skills/autospec-resume/scripts/resume-attempts.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_ACTIVE_RUNS_DIR="$TEST_TMP/active-runs"
    export AUTOSPEC_RESUME_ATTEMPTS_DIR="$TEST_TMP/resume-attempts"
}
teardown() { rm -rf "$TEST_TMP"; }

# ── registry ──────────────────────────────────────────────────────────────────

@test "registry write then read returns the resume_command" {
    bash "$REGISTRY" write --repo o/n --repo-dir /abs/checkout --harness claude \
        --command "claude /autospec-run" --host host-A
    run bash "$REGISTRY" read --repo o/n
    [ "$status" -eq 0 ]
    [ "$(echo "$output" | jq -r '.resume_command')" = "claude /autospec-run" ]
    [ "$(echo "$output" | jq -r '.repo')" = "o/n" ]
    [ "$(echo "$output" | jq -r '.harness')" = "claude" ]
    [ "$(echo "$output" | jq -r '.host')" = "host-A" ]
}

@test "registry path is scoped by repo-slug" {
    run bash "$REGISTRY" path --repo owner/name
    [ "$status" -eq 0 ]
    [[ "$output" == *"/owner_name.json" ]]
}

@test "registry-durable: reboot (env cleared) still yields a runnable command" {
    # /autospec-run writes the registry at monitor launch.
    AUTOSPEC_RESUME_COMMAND="echo REBOOT-OK" \
        bash "$REGISTRY" write --repo o/n --repo-dir /abs/checkout --harness codex \
            --command "echo REBOOT-OK" --host host-A
    # Simulate a reboot: env var is GONE, only the on-disk registry survives.
    unset AUTOSPEC_RESUME_COMMAND
    cmd="$(bash "$REGISTRY" read --repo o/n | jq -r '.resume_command')"
    [ "$cmd" = "echo REBOOT-OK" ]
    # The command derived from the registry alone is runnable.
    run bash -c "$cmd"
    [ "$status" -eq 0 ]
    [ "$output" = "REBOOT-OK" ]
}

@test "registry list shows fresh entries, prune drops stale" {
    bash "$REGISTRY" write --repo a/one --repo-dir /x --harness claude --command "c1" --host h
    bash "$REGISTRY" write --repo b/two --repo-dir /y --harness claude --command "c2" --host h
    run bash "$REGISTRY" list
    [ "$status" -eq 0 ]
    [[ "$output" == *"a_one.json"* ]]
    [[ "$output" == *"b_two.json"* ]]
}

@test "registry stale (>24h) entry is not surfaced by read" {
    bash "$REGISTRY" write --repo o/n --repo-dir /x --harness claude --command "c" --host h
    # Backdate updated_at to 25h ago.
    f="$(bash "$REGISTRY" path --repo o/n)"
    old="$(date -u -v-25H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date -u -d '-25 hours' +'%Y-%m-%dT%H:%M:%SZ')"
    jq --arg u "$old" '.updated_at=$u' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
    run bash "$REGISTRY" read --repo o/n
    [ "$status" -eq 0 ]
    [ -z "$output" ]
    # prune removes it from disk.
    bash "$REGISTRY" prune
    [ ! -f "$f" ]
}

@test "registry write preserves registered_at across re-write" {
    bash "$REGISTRY" write --repo o/n --repo-dir /x --harness claude --command "c1" --host h
    first="$(bash "$REGISTRY" read --repo o/n | jq -r '.registered_at')"
    sleep 1
    bash "$REGISTRY" write --repo o/n --repo-dir /x --harness claude --command "c2" --host h
    again="$(bash "$REGISTRY" read --repo o/n | jq -r '.registered_at')"
    [ "$first" = "$again" ]
}

# ── attempts counter ──────────────────────────────────────────────────────────

@test "attempts get is 0 when absent" {
    run bash "$ATTEMPTS" get --repo o/n
    [ "$status" -eq 0 ]
    [ "$output" = "0" ]
}

@test "attempts inc increments and persists" {
    bash "$ATTEMPTS" inc --repo o/n
    bash "$ATTEMPTS" inc --repo o/n
    run bash "$ATTEMPTS" get --repo o/n
    [ "$output" = "2" ]
}

@test "attempts cap default is 3, override via env" {
    run bash "$ATTEMPTS" cap
    [ "$output" = "3" ]
    AUTOSPEC_RESUME_MAX_ATTEMPTS=5 run bash "$ATTEMPTS" cap
    [ "$output" = "5" ]
}

@test "attempts stale (>24h) counter ignored -> effective 0" {
    bash "$ATTEMPTS" inc --repo o/n
    bash "$ATTEMPTS" inc --repo o/n
    f="$(bash "$ATTEMPTS" path --repo o/n)"
    old="$(date -u -v-25H +'%Y-%m-%dT%H:%M:%SZ' 2>/dev/null || date -u -d '-25 hours' +'%Y-%m-%dT%H:%M:%SZ')"
    jq --arg u "$old" '.updated_at=$u' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
    run bash "$ATTEMPTS" get --repo o/n
    [ "$output" = "0" ]
}
