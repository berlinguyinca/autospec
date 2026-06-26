#!/usr/bin/env bats
# tests/autonomous/test_resilience.bats — TDD contract for
# scripts/autonomous-resilience.sh (issue #1377)
#
# Covers the four resilience subcommands:
#   1. lock: second conductor blocked by a live heartbeat
#   2. lock: stale heartbeat (age >= 10800s) is reclaimable
#   3. lock: live heartbeat (age < 300s) is NOT reclaimable
#   4. quarantine: failure count past cap labels issue + emits DECISION:quarantine
#   5. main-health: green → DECISION:continue
#   6. main-health: pending → DECISION:wait
#   7. main-health: red/failure → DECISION:halt (exit 1)
#
# Design notes:
#   - gh is stubbed via PATH (real temp files, not process substitutions —
#     macOS bash 3.2 [ -f <(...) ] is false; feedback_bash32_process_sub_test_file)
#   - Stub shebangs use #!/bin/bash (absolute) so they run even on restricted PATH
#   - No real ~/.autospec writes: AUTOSPEC_STATE_DIR is redirected to TMP
#   - AUTOSPEC_HOST and AUTOSPEC_SESSION_ID are injected for determinism
#   - jq must be on the real PATH (tests require it; checked in setup)
#   - notify.sh is stubbed via AUTOSPEC_NOTIFY_SH to avoid desktop popups

bats_require_minimum_version 1.5.0

SCRIPT_DIR="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
RESILIENCE="$SCRIPT_DIR/scripts/autonomous-resilience.sh"
REPO="berlinguyinca/autospec"

setup() {
    # Verify required tools are present (fail fast, clear message)
    command -v jq >/dev/null 2>&1 || skip "jq not available"

    TMP="$(mktemp -d -t resilience.XXXXXX)"
    export AUTOSPEC_STATE_DIR="$TMP/autospec"
    export AUTOSPEC_HOST="test-host"
    export AUTOSPEC_SESSION_ID="test-session-42"
    export AUTOSPEC_NOTIFY=0       # silence real notifications globally
    export AUTOSPEC_REPO="$REPO"

    # Stub directory — prepend to PATH so our gh wins over any real gh
    STUB_DIR="$TMP/bin"
    mkdir -p "$STUB_DIR"
    export PATH="$STUB_DIR:$PATH"

    # Default gh stub: exits 0, emits empty JSON object (safe no-op)
    cat > "$STUB_DIR/gh" <<'EOF'
#!/bin/bash
echo "{}"
EOF
    chmod +x "$STUB_DIR/gh"

    # Stub notify.sh (writes to NOTIFY_LOG so tests can assert it fired)
    NOTIFY_LOG="$TMP/notify.log"
    touch "$NOTIFY_LOG"
    cat > "$STUB_DIR/notify.sh" <<EOF
#!/bin/bash
printf 'notify: %s -- %s\n' "\$1" "\$2" >> "$NOTIFY_LOG"
exit 0
EOF
    chmod +x "$STUB_DIR/notify.sh"
    export AUTOSPEC_NOTIFY_SH="$STUB_DIR/notify.sh"

    export AUTOSPEC_GH_CMD="$STUB_DIR/gh"

    # Helper: write a state.json with explicit heartbeat_at for lock tests
    mkdir -p "$AUTOSPEC_STATE_DIR/autonomous/berlinguyinca__autospec"
    STATE_DIR="$AUTOSPEC_STATE_DIR/autonomous/berlinguyinca__autospec"
}

teardown() {
    rm -rf "$TMP"
}

# ─────────────────────────────────────────────────────────────────────────────
# Helper: write a state.json with a specific heartbeat_at and lock
# ─────────────────────────────────────────────────────────────────────────────
write_locked_state() {
    local heartbeat_at="$1"
    local status="${2:-running}"
    jq -n \
        --arg repo "$REPO" \
        --arg slug "berlinguyinca__autospec" \
        --arg status "$status" \
        --arg host "other-host" \
        --arg session "other-session-99" \
        --argjson ts "$heartbeat_at" \
        --argjson pid 9999 \
        '{
            repo: $repo,
            slug: $slug,
            status: $status,
            host: $host,
            session: $session,
            heartbeat_at: $ts,
            lock_pid: $pid,
            lock_host: $host,
            lock_session: $session,
            lock_acquired_at: $ts
        }' > "$STATE_DIR/state.json"
}

# ─────────────────────────────────────────────────────────────────────────────
# 1. Lock blocks a second conductor when heartbeat is live (age < 300s)
# ─────────────────────────────────────────────────────────────────────────────
@test "lock acquire: blocked when existing lock has live heartbeat (age < 300s)" {
    # Write a state with a heartbeat_at = now (fresh lock held by another session)
    local now
    now="$(date -u +%s)"
    write_locked_state "$now" "running"

    run bash "$RESILIENCE" lock acquire --repo "$REPO"
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "DECISION:lock-held"
}

@test "lock acquire: HOLDER_SESSION is reported when blocked" {
    local now
    now="$(date -u +%s)"
    write_locked_state "$now" "running"

    run bash "$RESILIENCE" lock acquire --repo "$REPO"
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "HOLDER_SESSION:other-session-99"
}

# ─────────────────────────────────────────────────────────────────────────────
# 2. Stale heartbeat (age >= RECLAIM_SECS=10800) is reclaimable
# ─────────────────────────────────────────────────────────────────────────────
@test "lock acquire: reclaimable when heartbeat is stale (age >= 10800s)" {
    local now
    now="$(date -u +%s)"
    # heartbeat_at = now - 11000 (past the 10800s reclaim threshold)
    local stale_ts=$(( now - 11000 ))
    write_locked_state "$stale_ts" "running"

    run bash "$RESILIENCE" lock acquire --repo "$REPO"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "DECISION:lock-acquired"
}

@test "lock acquire: state.json updated with new session after reclaim" {
    local now
    now="$(date -u +%s)"
    local stale_ts=$(( now - 11000 ))
    write_locked_state "$stale_ts" "running"

    bash "$RESILIENCE" lock acquire --repo "$REPO" --session "new-session-77"

    # Verify the new lock session is written to state.json
    local new_session
    new_session="$(jq -r '.lock_session' "$STATE_DIR/state.json")"
    [ "$new_session" = "new-session-77" ]
}

# ─────────────────────────────────────────────────────────────────────────────
# 3. Claimed-state stale threshold (300s): only blocks when age < 300
# ─────────────────────────────────────────────────────────────────────────────
@test "lock acquire: blocked when status=claimed and age < 300s" {
    local now
    now="$(date -u +%s)"
    # age = 100s, status=claimed → still live
    local fresh_ts=$(( now - 100 ))
    write_locked_state "$fresh_ts" "claimed"

    run bash "$RESILIENCE" lock acquire --repo "$REPO"
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "DECISION:lock-held"
}

@test "lock acquire: reclaimable when status=claimed and age >= 300s (but < 10800s)" {
    local now
    now="$(date -u +%s)"
    # age = 400s, status=claimed → stale per claimed threshold
    local stale_ts=$(( now - 400 ))
    write_locked_state "$stale_ts" "claimed"

    run bash "$RESILIENCE" lock acquire --repo "$REPO"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "DECISION:lock-acquired"
}

# ─────────────────────────────────────────────────────────────────────────────
# 4. Lock release clears the lock so a fresh process can acquire
# ─────────────────────────────────────────────────────────────────────────────
@test "lock release: clears lock and allows subsequent acquire" {
    local now
    now="$(date -u +%s)"
    # Write a live lock
    write_locked_state "$now" "running"

    # First acquire should be blocked
    run bash "$RESILIENCE" lock acquire --repo "$REPO"
    [ "$status" -eq 1 ]

    # Release the lock
    bash "$RESILIENCE" lock release --repo "$REPO"

    # Now acquire should succeed
    run bash "$RESILIENCE" lock acquire --repo "$REPO"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "DECISION:lock-acquired"
}

@test "lock release: emits DECISION:lock-released" {
    local now
    now="$(date -u +%s)"
    write_locked_state "$now" "running"

    run bash "$RESILIENCE" lock release --repo "$REPO"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "DECISION:lock-released"
}

# ─────────────────────────────────────────────────────────────────────────────
# 5. Quarantine: failure count below cap → DECISION:continue
# ─────────────────────────────────────────────────────────────────────────────
@test "quarantine: below cap emits DECISION:continue" {
    # Cap defaults to 3; pass --failures 2 → below cap
    run bash "$RESILIENCE" quarantine --repo "$REPO" --issue 999 --failures 2
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "DECISION:continue"
}

# ─────────────────────────────────────────────────────────────────────────────
# 6. Quarantine: failure count past cap → label + notify + DECISION:quarantine
# ─────────────────────────────────────────────────────────────────────────────
@test "quarantine: at or past cap emits DECISION:quarantine and exits 1" {
    # Cap defaults to 3; pass --failures 3 → at cap
    run bash "$RESILIENCE" quarantine --repo "$REPO" --issue 42 --failures 3
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "DECISION:quarantine"
}

@test "quarantine: past cap invokes gh to label issue autospec:needs-human" {
    # Stub gh to record calls
    GH_LOG="$TMP/gh.log"
    touch "$GH_LOG"
    cat > "$STUB_DIR/gh" <<EOF
#!/bin/bash
printf 'gh %s\n' "\$*" >> "$GH_LOG"
exit 0
EOF
    chmod +x "$STUB_DIR/gh"

    run bash "$RESILIENCE" quarantine --repo "$REPO" --issue 77 --failures 3
    [ "$status" -eq 1 ]

    # gh issue edit should have been called with --add-label autospec:needs-human
    grep -q "add-label" "$GH_LOG"
    grep -q "needs-human" "$GH_LOG"
}

@test "quarantine: past cap fires notify via AUTOSPEC_NOTIFY_SH stub" {
    run bash "$RESILIENCE" quarantine --repo "$REPO" --issue 55 --failures 3
    [ "$status" -eq 1 ]
    # Notify log should contain an entry
    grep -q "notify" "$NOTIFY_LOG" || grep -q "quarantine" "$NOTIFY_LOG"
}

@test "quarantine: auto-increments failure count when --failures not provided" {
    # First call: no --failures, count starts at 0 → increments to 1
    run bash "$RESILIENCE" quarantine --repo "$REPO" --issue 100
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "FAILURES:1"

    # Second call: count 1 → 2
    run bash "$RESILIENCE" quarantine --repo "$REPO" --issue 100
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "FAILURES:2"

    # Third call: count 2 → 3 → at cap (default 3)
    run bash "$RESILIENCE" quarantine --repo "$REPO" --issue 100
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "DECISION:quarantine"
}

# ─────────────────────────────────────────────────────────────────────────────
# 7. main-health: green (success) → DECISION:continue
# ─────────────────────────────────────────────────────────────────────────────
@test "main-health: green CI state → DECISION:continue and exit 0" {
    STATUS_JSON='{"state":"success","statuses":[]}'
    STATUS_FILE="$TMP/status.json"
    printf '%s\n' "$STATUS_JSON" > "$STATUS_FILE"
    cat > "$STUB_DIR/gh" <<EOF
#!/bin/bash
cat "$STATUS_FILE"
EOF
    chmod +x "$STUB_DIR/gh"

    run bash "$RESILIENCE" main-health --repo "$REPO"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "DECISION:continue"
}

# ─────────────────────────────────────────────────────────────────────────────
# 8. main-health: pending → DECISION:wait
# ─────────────────────────────────────────────────────────────────────────────
@test "main-health: pending CI state → DECISION:wait and exit 0" {
    STATUS_FILE="$TMP/status.json"
    printf '{"state":"pending","statuses":[]}\n' > "$STATUS_FILE"
    cat > "$STUB_DIR/gh" <<EOF
#!/bin/bash
cat "$STATUS_FILE"
EOF
    chmod +x "$STUB_DIR/gh"

    run bash "$RESILIENCE" main-health --repo "$REPO"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "DECISION:wait"
}

# ─────────────────────────────────────────────────────────────────────────────
# 9. main-health: failure → DECISION:halt and exit 1
# ─────────────────────────────────────────────────────────────────────────────
@test "main-health: failure CI state → DECISION:halt and exit 1" {
    STATUS_FILE="$TMP/status.json"
    printf '{"state":"failure","statuses":[]}\n' > "$STATUS_FILE"
    cat > "$STUB_DIR/gh" <<EOF
#!/bin/bash
cat "$STATUS_FILE"
EOF
    chmod +x "$STUB_DIR/gh"

    run bash "$RESILIENCE" main-health --repo "$REPO"
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "DECISION:halt"
}

@test "main-health: error CI state → DECISION:halt and exit 1" {
    STATUS_FILE="$TMP/status.json"
    printf '{"state":"error","statuses":[]}\n' > "$STATUS_FILE"
    cat > "$STUB_DIR/gh" <<EOF
#!/bin/bash
cat "$STATUS_FILE"
EOF
    chmod +x "$STUB_DIR/gh"

    run bash "$RESILIENCE" main-health --repo "$REPO"
    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "DECISION:halt"
}

@test "main-health: gh api failure → DECISION:wait (conservative fallback)" {
    cat > "$STUB_DIR/gh" <<'EOF'
#!/bin/bash
exit 1
EOF
    chmod +x "$STUB_DIR/gh"

    run bash "$RESILIENCE" main-health --repo "$REPO"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "DECISION:wait"
}

# ─────────────────────────────────────────────────────────────────────────────
# 10. state write/read round-trip
# ─────────────────────────────────────────────────────────────────────────────
@test "state write: emits DECISION:state-written" {
    run bash "$RESILIENCE" state write --repo "$REPO" --status running
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "DECISION:state-written"
}

@test "state read: returns written state as JSON" {
    bash "$RESILIENCE" state write --repo "$REPO" --status running

    run bash "$RESILIENCE" state read --repo "$REPO"
    [ "$status" -eq 0 ]
    # Output should be valid JSON with status=running
    local status_field
    status_field="$(printf '%s\n' "$output" | jq -r '.status' 2>/dev/null)"
    [ "$status_field" = "running" ]
}

@test "state write: path-scoped to canonical slug (owner__name directory)" {
    bash "$RESILIENCE" state write --repo "$REPO" --status idle

    # State should be in the canonical slug directory
    local expected_dir="$AUTOSPEC_STATE_DIR/autonomous/berlinguyinca__autospec"
    [ -f "$expected_dir/state.json" ]
}
