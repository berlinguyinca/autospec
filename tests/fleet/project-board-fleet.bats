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

    # Stub the conductor so no real process is ever started. This stub
    # VALIDATES its arguments the way scripts/autospec-autonomous.sh's real
    # parser does for `start`: known flags are accepted, anything else is
    # rejected with "unknown argument: $1" and a non-zero exit — exactly
    # what the real binary does. An argument-blind stub (`printf '%s\n'
    # "$*"`) accepts anything, including a rejected flag like `--detach`,
    # and so can never catch fleet-lib.sh/fleet-run.sh building a command
    # the real conductor refuses. Do not weaken this back to argument-blind.
    cat > "$TMP/bin/autospec-autonomous" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
orig="$*"
sub="${1:-}"
shift || true
case "$sub" in
    start) ;;
    *) printf 'autospec-autonomous: unknown subcommand: %s\n' "$sub" >&2; exit 1 ;;
esac
while [ $# -gt 0 ]; do
    case "$1" in
        --repo-dir|--repo) shift 2 ;;
        --repo-dir=*|--repo=*) shift ;;
        --foreground|--force) shift ;;
        *) printf 'autospec-autonomous: unknown argument: %s\n' "$1" >&2; exit 1 ;;
    esac
done
printf '%s\n' "$orig" >> "$FLEET_SPAWN_LOG"
SH
    chmod +x "$TMP/bin/autospec-autonomous"

    # Stub the queue probe so every configured repo has ready work.
    cat > "$TMP/bin/autospec" <<'SH'
#!/usr/bin/env bash
case "$*" in *"queue ready"*) printf '{"batch":[{"number":1}]}' ;; *) printf '' ;; esac
SH
    chmod +x "$TMP/bin/autospec"
    export PATH="$TMP/bin:$PATH"

    # fleet-run.sh resolves its queue binary as
    # AUTOSPEC_FLEET_QUEUE_BIN -> AUTOSPEC_QUEUE_BIN -> AUTOSPEC_BIN ->
    # $repo_root/target/debug/autospec -> `command -v autospec` (PATH) — the
    # built target/debug binary is preferred *over* PATH. Prepending PATH
    # above is not enough: in a worktree that has ever been `cargo build`-ed,
    # target/debug/autospec exists and wins, so queue_has_work would invoke
    # the REAL autospec binary against these fake o/a and o/b repos instead
    # of this stub. Pin the highest-precedence seam explicitly so the stub
    # always wins regardless of what exists in target/ or on PATH.
    export AUTOSPEC_FLEET_QUEUE_BIN="$TMP/bin/autospec"

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

@test "the spawned command uses only flags the real conductor accepts (C1 regression pin)" {
    # scripts/autospec-autonomous.sh's `start` has no --detach case and its
    # catch-all is `die "unknown argument: $1"`; `start` already detaches by
    # default. The stub in setup() validates arguments the same way, so this
    # test is RED against the old `start --detach ...` command shape and
    # GREEN against the fixed one — a real regression pin, not a tautology.
    run bash "$RUN" --config "$TMP/fleet.yml"
    [ "$status" -eq 0 ]
    [ "$(wc -l < "$FLEET_SPAWN_LOG")" -eq 2 ]
    run grep -q -- '--detach' "$FLEET_SPAWN_LOG"
    [ "$status" -ne 0 ]
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
