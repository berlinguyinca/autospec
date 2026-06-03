#!/usr/bin/env bats
# tests/autospec-run/test_session_lock.bats — per-session single-instance guard
# for /autospec-run. Scope: harness session id. On contention: refuse + report.

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    LOCK="$REPO_ROOT/skills/autospec-run/scripts/autospec-run-session-lock.sh"
    AUTOSPEC_LOCK_DIR="$(mktemp -d)"
    export AUTOSPEC_LOCK_DIR
}

teardown() {
    rm -rf "$AUTOSPEC_LOCK_DIR"
}

@test "acquire succeeds on first call and writes a lock file" {
    AUTOSPEC_SESSION_ID=sessA run bash "$LOCK" acquire --repo o/r
    [ "$status" -eq 0 ]
    [ -n "$(ls -A "$AUTOSPEC_LOCK_DIR")" ]
}

@test "second acquire in the SAME session is refused (exit 3) and reports status" {
    AUTOSPEC_SESSION_ID=sessA bash "$LOCK" acquire --repo o/r
    AUTOSPEC_SESSION_ID=sessA run bash "$LOCK" acquire --repo o/r
    [ "$status" -eq 3 ]
    [[ "$output" == *"already active"* ]]
}

@test "a DIFFERENT session acquires independently while the first is held" {
    AUTOSPEC_SESSION_ID=sessA bash "$LOCK" acquire --repo o/r
    AUTOSPEC_SESSION_ID=sessB run bash "$LOCK" acquire --repo o/r
    [ "$status" -eq 0 ]
}

@test "release frees the lock so the same session can re-acquire" {
    AUTOSPEC_SESSION_ID=sessA bash "$LOCK" acquire --repo o/r
    AUTOSPEC_SESSION_ID=sessA run bash "$LOCK" release
    [ "$status" -eq 0 ]
    AUTOSPEC_SESSION_ID=sessA run bash "$LOCK" acquire --repo o/r
    [ "$status" -eq 0 ]
}

@test "--force overrides an existing lock in the same session" {
    AUTOSPEC_SESSION_ID=sessA bash "$LOCK" acquire --repo o/r
    AUTOSPEC_SESSION_ID=sessA run bash "$LOCK" acquire --repo o/r --force
    [ "$status" -eq 0 ]
}

@test "status reports inactive when free and active when held" {
    AUTOSPEC_SESSION_ID=sessC run bash "$LOCK" status
    [ "$status" -eq 0 ]
    [[ "$output" == *"false"* ]]
    AUTOSPEC_SESSION_ID=sessC bash "$LOCK" acquire --repo o/r
    AUTOSPEC_SESSION_ID=sessC run bash "$LOCK" status
    [[ "$output" == *"sessC"* ]]
}

@test "session token falls back to CLAUDE_CODE_SESSION_ID when AUTOSPEC_SESSION_ID unset" {
    CLAUDE_CODE_SESSION_ID=ccA bash "$LOCK" acquire --repo o/r
    CLAUDE_CODE_SESSION_ID=ccA run bash "$LOCK" acquire --repo o/r
    [ "$status" -eq 3 ]
    CLAUDE_CODE_SESSION_ID=ccB run bash "$LOCK" acquire --repo o/r
    [ "$status" -eq 0 ]
}

@test "release is idempotent (releasing when no lock is held still exits 0)" {
    AUTOSPEC_SESSION_ID=sessA run bash "$LOCK" release
    [ "$status" -eq 0 ]
}

@test "--force is a clean override even when the file is mid-write empty" {
    # Simulate a partially-written (empty) lock body; --force must still acquire
    # deterministically (no rm+recreate race).
    : > "$AUTOSPEC_LOCK_DIR/autospec-run-session-sessF.lock"
    AUTOSPEC_SESSION_ID=sessF run bash "$LOCK" acquire --repo o/r --force
    [ "$status" -eq 0 ]
    [[ "$(cat "$AUTOSPEC_LOCK_DIR/autospec-run-session-sessF.lock")" == *"sessF"* ]]
}
