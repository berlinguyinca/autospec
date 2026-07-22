#!/usr/bin/env bats
# tests/unit/test_autospec_watchdog.bats — heartbeat reconciliation and
# timeout behavior for scripts/autospec-watchdog.sh.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-watchdog.sh"

    export HOME="$(mktemp -d)"
    export AUTOSPEC_WATCHDOG_DIR="$HOME/.autospec/process-heartbeats"
    export AUTOSPEC_WATCHDOG_STATE_FILE="$HOME/.autospec/watchdog-state.tsv"
    export AUTOSPEC_WATCHDOG_STALE_SECS=1800
    export AUTOSPEC_WATCHDOG_RECLAIM_SECS=10800
    export AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS=300
    export AUTOSPEC_CONFIG_FILE="$HOME/missing-autospec.yml"
    mkdir -p "$AUTOSPEC_WATCHDOG_DIR"

    STUB_BIN="$(mktemp -d)"
    GH_LOG="$HOME/gh.log"
    export GH_LOG
    # Fix repo so watchdog can derive the scoped subdir
    export AUTOSPEC_WATCHDOG_REPO="owner/repo"
    cat > "$STUB_BIN/gh" <<'GHSTUB'
#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$GH_LOG"
if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
    printf 'owner/repo\n'
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
    case "$3" in
        406) printf 'CLOSED \n' ;;
        407) printf 'OPEN auto-implement\n' ;;
        408) printf 'OPEN in-progress-by-bot\n' ;;
        409) printf 'OPEN in-progress-by-bot\n' ;;
        410) printf 'OPEN in-progress-by-bot\n' ;;
        *) printf 'OPEN in-progress-by-bot\n' ;;
    esac
    exit 0
fi
exit 0
GHSTUB
    chmod +x "$STUB_BIN/gh"
    export PATH="$STUB_BIN:$PATH"

    # Heartbeats now live under <repo-slug>/ subdir (owner/repo -> owner_repo)
    REPO_SLUG="owner_repo"
    mkdir -p "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG"
    export REPO_SLUG

    NOW="$(date -u +%s)"
    export NOW
}

teardown() {
    rm -rf "$HOME" "$STUB_BIN"
}

@test "watchdog source avoids ambiguous any audit token" {
    run grep -nE '\bany\b' "$SCRIPT"

    [ "$status" -eq 1 ]
    [ -z "$output" ]
}

write_hb() {
    issue="$1"
    step="$2"
    age="$3"
    ts=$((NOW - age))
    # Write to the repo-scoped subdir
    cat > "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/$issue.json" <<EOF
{"issue":$issue,"branch":"feat/$issue","step":"$step","ts":$ts,"pr":"","repo":"owner/repo"}
EOF
}

@test "startup reconciliation garbage-collects closed and orphaned heartbeats" {
    write_hb 406 worktree_ready 10
    write_hb 407 worktree_ready 10

    run bash "$SCRIPT"

    [ "$status" -eq 0 ]
    [ ! -f "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/406.json" ]
    [ ! -f "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/407.json" ]
    echo "$output" | grep -q 'garbage_collected=2'
}

@test "old heartbeat schemas are rejected before issue processing" {
    cat > "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/408.json" <<EOF
{"issue":408,"status":"in_progress","ts":$((NOW - 10))}
EOF

    run bash "$SCRIPT"

    [ "$status" -eq 0 ]
    [ ! -f "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/408.json" ]
    echo "$output" | grep -q 'invalid_schema=1'
    ! grep -q 'issue edit 408' "$GH_LOG"
}

@test "claimed heartbeat older than timeout is released immediately" {
    write_hb 409 claimed 360

    run bash "$SCRIPT"

    [ "$status" -eq 0 ]
    [ ! -f "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/409.json" ]
    echo "$output" | grep -q 'claimed_released=1'
    grep -q 'issue edit 409' "$GH_LOG"
    grep -q 'remove-label in-progress-by-bot' "$GH_LOG"
}

@test "current schema is normalized without deleting active heartbeat" {
    cat > "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/410.json" <<EOF
{"issue":410,"branch":"feat/410","step":"tests_started","ts":$((NOW - 10))}
EOF

    run bash "$SCRIPT"

    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/410.json" ]
    jq -e '.issue == "410" and .pr == "" and .repo == ""' "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/410.json" >/dev/null
}

@test "missing jq fails open without deleting heartbeats" {
    write_hb 410 tests_started 10
    PATH="$STUB_BIN:/usr/bin:/bin" run bash "$SCRIPT"

    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/410.json" ]
    echo "$output" | grep -q 'invalid_schema=0'
}

@test "expand_start is a recognized step and survives reconciliation (F3 refresh)" {
    # The implementer writes an `expand_start` heartbeat to cover the
    # claim->worktree_ready window. The watchdog must treat it as a valid
    # step, not garbage-collect it as an unknown schema.
    cat > "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/410.json" <<EOF
{"issue":410,"branch":"feat/410","step":"expand_start","ts":$((NOW - 10))}
EOF

    run bash "$SCRIPT"

    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_WATCHDOG_DIR/$REPO_SLUG/410.json" ]   # not GC'd as invalid schema
    echo "$output" | grep -q 'invalid_schema=0'
}
