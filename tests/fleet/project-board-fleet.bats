#!/usr/bin/env bats
# tests/fleet/project-board-fleet.bats
#
# fleet-run must actually start per-repo conductor workers, not print what
# it would have started. Every test stubs `autospec-autonomous` (and
# `autospec`, the queue probe) on PATH — no test may reach a real binary,
# because a real spawn would start unattended work against a real repo.

setup() {
    TMP="$(mktemp -d)"
    mkdir -p "$TMP/bin" "$TMP/ws" "$TMP/hb"
    RUN="${BATS_TEST_DIRNAME}/../../skills/autospec-fleet/scripts/fleet-run.sh"

    export FLEET_SPAWN_LOG="$TMP/spawn.log"
    : > "$FLEET_SPAWN_LOG"

    # Isolate the liveness heartbeat store from the real machine — fleet-run
    # must never read or write the operator's actual ~/.autospec state, and
    # tests must never leak liveness markers into each other.
    export AUTOSPEC_HEARTBEAT_DIR="$TMP/hb"

    # Stub the conductor so no real process is ever started.
    cat > "$TMP/bin/autospec-autonomous" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FLEET_SPAWN_LOG"
SH
    chmod +x "$TMP/bin/autospec-autonomous"

    # Stub the queue probe so every configured repo has ready work.
    cat > "$TMP/bin/autospec" <<'SH'
#!/usr/bin/env bash
case "$*" in *"queue ready"*) printf '{"batch":[{"number":1}]}' ;; *) printf '' ;; esac
SH
    chmod +x "$TMP/bin/autospec"
    export PATH="$TMP/bin:$PATH"

    cat > "$TMP/fleet.yml" <<YML
version: 1
workspace: $TMP/ws
parallel_repos: 2
repos:
  - url: https://github.com/o/a.git
    enabled: true
  - url: https://github.com/o/b.git
    enabled: true
YML

    # repo_checkout_path (fleet-lib.sh) keys checkouts by the canonical
    # owner__name slug, not a literal owner/name directory tree.
    mkdir -p "$TMP/ws/o__a" "$TMP/ws/o__b"
}

teardown() {
    rm -rf "$TMP"
}

@test "dry-run prints and starts nothing" {
    run bash "$RUN" --config "$TMP/fleet.yml" --dry-run
    [ "$status" -eq 0 ]
    [ ! -s "$FLEET_SPAWN_LOG" ]
}

@test "a live run actually spawns one worker per eligible repo" {
    run bash "$RUN" --config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$FLEET_SPAWN_LOG")" -eq 2 ]
}

@test "the spawned command is a conductor, not a one-shot autospec-run" {
    run bash "$RUN" --config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    grep -q 'start' "$FLEET_SPAWN_LOG"
    grep -q -- '--repo-dir' "$FLEET_SPAWN_LOG"
    grep -q -- '--repo o/a' "$FLEET_SPAWN_LOG"
    ! grep -q 'autospec-run' "$FLEET_SPAWN_LOG"
}

@test "parallel_repos caps the number of spawned workers" {
    sed -i.bak 's/parallel_repos: 2/parallel_repos: 1/' "$TMP/fleet.yml"
    run bash "$RUN" --config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$FLEET_SPAWN_LOG")" -eq 1 ]
}

@test "a repo with a live worker is not spawned twice" {
    bash "$RUN" --config "$TMP/fleet.yml" >/dev/null
    before="$(wc -l < "$FLEET_SPAWN_LOG")"
    run bash "$RUN" --config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$FLEET_SPAWN_LOG")" -eq "$before" ]
}

@test "a spawn failure quarantines that repo and continues to the next" {
    # Two separate facts, two separate logs: FLEET_SPAWN_ATTEMPT_LOG records
    # every invocation the stub receives regardless of outcome (proves
    # fleet-run actually tried o/a), while FLEET_SPAWN_LOG only gains a line
    # on a successful (simulated) launch. Without the attempt log, "o/a is
    # absent from FLEET_SPAWN_LOG" is true by construction for a stub that
    # never logs a failing repo — a tautology, not a real check. Recording
    # the attempt first makes "o/a was tried but did not succeed" falsifiable.
    export FLEET_SPAWN_ATTEMPT_LOG="$TMP/spawn-attempt.log"
    : > "$FLEET_SPAWN_ATTEMPT_LOG"
    cat > "$TMP/bin/autospec-autonomous" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FLEET_SPAWN_ATTEMPT_LOG"
case "$*" in *"o/a"*) exit 1 ;; esac
printf '%s\n' "$*" >> "$FLEET_SPAWN_LOG"
SH
    chmod +x "$TMP/bin/autospec-autonomous"

    run bash "$RUN" --config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    grep -q 'o/b' "$FLEET_SPAWN_LOG"
    echo "$output" | grep -q 'code_health:fleet_worker_spawn_failed repo=o/a'
    grep -q 'o/a' "$FLEET_SPAWN_ATTEMPT_LOG"
    run grep -q 'o/a' "$FLEET_SPAWN_LOG"
    [ "$status" -ne 0 ]
}

@test "a missing checkout directory is skipped with a clear message, not spawned" {
    rm -rf "$TMP/ws/o__a"
    run bash "$RUN" --config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    grep -q 'o/b' "$FLEET_SPAWN_LOG"
    echo "$output" | grep -q "o/a: checkout not found"
    run grep -q 'o/a' "$FLEET_SPAWN_LOG"
    [ "$status" -ne 0 ]
}

@test "dry-run does not require the checkout to exist and still previews it" {
    rm -rf "$TMP/ws/o__a"
    run bash "$RUN" --config "$TMP/fleet.yml" --dry-run
    [ "$status" -eq 0 ]
    [ ! -s "$FLEET_SPAWN_LOG" ]
    echo "$output" | grep -q 'o/a'
}
