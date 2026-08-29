#!/usr/bin/env bats
# Real-OS process-tree tests for autospec_kill_tree (issue #2751). No overridden
# process commands: ps, pgrep, kill, and sleep all run for real. Every spawned
# family is recorded in PID_LEDGER and force-killed in teardown.

setup() {
    LIB="$BATS_TEST_DIRNAME/../../scripts/lib/autospec-process-tree.sh"
    TMP="$(mktemp -d -t process-tree.XXXXXX)"
    PID_LEDGER="$TMP/pids"
    : > "$PID_LEDGER"
    if [ "$(uname -s)" = Linux ]; then
    cat > "$TMP/family.sh" <<'EOF'
#!/usr/bin/env bash
sleep 60 & echo $! > "$CHILD_PID_FILE"
( sleep 60 & echo $! > "$ORPHAN_PID_FILE" ) &
setsid bash -c 'echo $$ > "$NESTED_PID_FILE"; exec sleep 60' </dev/null >/dev/null 2>&1 &
wait
EOF
    chmod +x "$TMP/family.sh"
    fi
}

teardown() {
    while IFS= read -r p; do
        [ -n "$p" ] || continue
        case "$p" in 0|1) continue ;; esac
        [ "$p" -eq "$$" ] 2>/dev/null && continue
        kill -KILL "$p" 2>/dev/null || true # linter:allow-VACUOUS_OR_TRUE cleanup, not an assertion — pid may already be dead
    done < "$PID_LEDGER"
    rm -rf "$TMP"
}

# Spawns leader + same-group child + same-group orphan + nested-setsid grandchild.
# Sets LEADER_PID, CHILD_PID, ORPHAN_PID, NESTED_PID and records all in the ledger.
spawn_family() {
    [ "$(uname -s)" = Linux ] || skip "real process-group fixture requires Linux setsid semantics"
    export CHILD_PID_FILE="$TMP/child.pid" ORPHAN_PID_FILE="$TMP/orphan.pid" NESTED_PID_FILE="$TMP/nested.pid"
    setsid "$TMP/family.sh" </dev/null >/dev/null 2>&1 &
    LEADER_PID=$!
    echo "$LEADER_PID" >> "$PID_LEDGER"
    for _ in $(seq 1 30); do
        [ -s "$CHILD_PID_FILE" ] && [ -s "$ORPHAN_PID_FILE" ] && [ -s "$NESTED_PID_FILE" ] && break
        sleep 0.1
    done
    CHILD_PID="$(cat "$CHILD_PID_FILE")"
    ORPHAN_PID="$(cat "$ORPHAN_PID_FILE")"
    NESTED_PID="$(cat "$NESTED_PID_FILE")"
    printf '%s\n%s\n%s\n' "$CHILD_PID" "$ORPHAN_PID" "$NESTED_PID" >> "$PID_LEDGER"
    kill -0 "$LEADER_PID" && kill -0 "$CHILD_PID" && kill -0 "$ORPHAN_PID" && kill -0 "$NESTED_PID"
}

assert_dead() {
    for _ in $(seq 1 30); do
        kill -0 "$1" 2>/dev/null || return 0
        sleep 0.1
    done
    return 1
}

@test "policy none kills only the exact pid" {
    spawn_family
    run bash -c "source '$LIB'; autospec_kill_tree $LEADER_PID none"
    [ "$status" -eq 0 ]
    wait "$LEADER_PID" 2>/dev/null || true # linter:allow-VACUOUS_OR_TRUE reaps the zombie; the assertion is the next line
    ! kill -0 "$LEADER_PID" 2>/dev/null
    kill -0 "$CHILD_PID" 2>/dev/null
    kill -0 "$ORPHAN_PID" 2>/dev/null
    kill -0 "$NESTED_PID" 2>/dev/null
}

@test "policy leader kills PPID-chain descendants but not an orphan or nested group" {
    spawn_family
    run bash -c "source '$LIB'; autospec_kill_tree $LEADER_PID leader"
    [ "$status" -eq 0 ]
    assert_dead "$LEADER_PID"
    assert_dead "$CHILD_PID"
    kill -0 "$ORPHAN_PID" 2>/dev/null
    assert_dead "$NESTED_PID"
}

@test "policy separate group-kills same-group members but not a nested group" {
    spawn_family
    run bash -c "source '$LIB'; autospec_kill_tree $LEADER_PID separate"
    [ "$status" -eq 0 ]
    assert_dead "$LEADER_PID"
    assert_dead "$CHILD_PID"
    assert_dead "$ORPHAN_PID"
    kill -0 "$NESTED_PID" 2>/dev/null
}

@test "policy separate-recursive kills the group and nested descendants" {
    spawn_family
    run bash -c "source '$LIB'; autospec_kill_tree $LEADER_PID separate-recursive"
    [ "$status" -eq 0 ]
    assert_dead "$LEADER_PID"
    assert_dead "$CHILD_PID"
    assert_dead "$ORPHAN_PID"
    assert_dead "$NESTED_PID"
}

@test "unknown policy is refused" {
    spawn_family
    run bash -c "source '$LIB'; autospec_kill_tree $LEADER_PID bogus"
    [ "$status" -eq 2 ]
    kill -0 "$LEADER_PID" 2>/dev/null
}

@test "pid 0, pid 1, and self are refused before any signal is sent" {
    run bash -c "source '$LIB'; autospec_kill_tree 0 none"
    [ "$status" -eq 3 ]
    run bash -c "source '$LIB'; autospec_kill_tree 1 none"
    [ "$status" -eq 3 ]
    run bash -c "source '$LIB'; autospec_kill_tree \$\$ none"
    [ "$status" -eq 3 ]
}

@test "separate refuses a pid that is not its own process-group leader" {
    sleep 60 &
    canary=$!
    echo "$canary" >> "$PID_LEDGER"
    run bash -c "source '$LIB'; autospec_kill_tree $canary separate"
    [ "$status" -eq 3 ]
    kill -0 "$canary" 2>/dev/null
}
