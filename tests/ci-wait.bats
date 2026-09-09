#!/usr/bin/env bats
# tests/ci-wait.bats — tests for scripts/ci-wait.sh poller state transitions.
#
# Drives the poller's jq logic against fixture rollups with a mocked `gh` on
# PATH. Covers pending/fail/pass/stalled verdicts, including the #3214
# regression: gh 2.67.0 reports in-progress checks with conclusion "" (not
# null), so the poller must count status != COMPLETED as pending and never
# write state=pass while a check is still running.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/ci-wait.sh"
    WORK="$(mktemp -d -t ci-wait-test.XXXXXX)"
    REAL_SLEEP="$(command -v sleep)"

    # Isolate all poller state under a throwaway HOME.
    export HOME="$WORK/home"
    mkdir -p "$HOME"

    # Mock gh: log each call, then emit the fixture rollup (the
    # "--jq .statusCheckRollup // []" result, i.e. a JSON array).
    BIN="$WORK/bin"
    mkdir -p "$BIN"
    GH_LOG="$WORK/gh-calls.log"
    export CI_WAIT_TEST_GH_LOG="$GH_LOG"
    cat > "$BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo x >> "$CI_WAIT_TEST_GH_LOG"
cat "$CI_WAIT_TEST_ROLLUP"
EOF
    chmod +x "$BIN/gh"
    export PATH="$BIN:$PATH"
}

teardown() {
    # Kill the poller (if any) before removing the fake gh/sleep from PATH.
    if [ -n "${PR:-}" ] && [ -f "$HOME/.autospec/ci-state/$PR.pid" ]; then
        kill "$(cat "$HOME/.autospec/ci-state/$PR.pid")" 2>/dev/null || true
    fi
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

# fake_sleep — no-op sleep on PATH so the poller loop spins fast.
fake_sleep() {
    printf '#!/usr/bin/env bash\nexit 0\n' > "$BIN/sleep"
    chmod +x "$BIN/sleep"
}

# wait_for_state <pr> <state> — poll the signal file until it reports <state>.
wait_for_state() {
    local pr="$1" want="$2" i state
    for i in $(seq 1 50); do
        state="$(jq -r '.state' "$HOME/.autospec/ci-state/$pr.signal" 2>/dev/null || true)"
        [ "$state" = "$want" ] && return 0
        "$REAL_SLEEP" 0.2
    done
    return 1
}

# wait_gh_called — poll until the poller has fetched at least one rollup.
wait_gh_called() {
    local i
    for i in $(seq 1 50); do
        [ -s "$CI_WAIT_TEST_GH_LOG" ] && return 0
        "$REAL_SLEEP" 0.1
    done
    return 1
}

# --- #3214 regression: in-progress checks carry conclusion "" (not null) ---

@test "pending: IN_PROGRESS checks with empty-string conclusion mixed with SUCCESS keep state=pending" {
    PR=301
    cat > "$WORK/rollup.json" <<'EOF'
[
  {"name":"build-test","status":"IN_PROGRESS","conclusion":"","startedAt":"2026-08-18T19:30:51Z"},
  {"name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}
]
EOF
    export CI_WAIT_TEST_ROLLUP="$WORK/rollup.json"
    fake_sleep

    run bash "$SCRIPT" "$PR"
    [ "$status" -eq 0 ]

    # Wait for the poller to fetch at least one snapshot, then assert it did
    # NOT settle on pass (the #3214 false-pass) and the signal stays pending.
    wait_gh_called || { echo "poller never fetched"; return 1; }
    "$REAL_SLEEP" 0.3
    local state
    state="$(jq -r '.state' "$HOME/.autospec/ci-state/$PR.signal")"
    [ "$state" = "pending" ]

    # Stop the poller; a pending verdict must survive the kill.
    kill "$(cat "$HOME/.autospec/ci-state/$PR.pid")" 2>/dev/null || true
    state="$(jq -r '.state' "$HOME/.autospec/ci-state/$PR.signal")"
    [ "$state" = "pending" ]
}

# --- fail ---

@test "fail: one FAILURE check settles state=fail" {
    PR=302
    cat > "$WORK/rollup.json" <<'EOF'
[
  {"name":"build-test","status":"COMPLETED","conclusion":"FAILURE"},
  {"name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}
]
EOF
    export CI_WAIT_TEST_ROLLUP="$WORK/rollup.json"

    run bash "$SCRIPT" "$PR"
    [ "$status" -eq 0 ]

    wait_for_state "$PR" fail
    local state
    state="$(jq -r '.state' "$HOME/.autospec/ci-state/$PR.signal")"
    [ "$state" = "fail" ]
}

# --- pass ---

@test "pass: all checks COMPLETED/SUCCESS settle state=pass" {
    PR=303
    cat > "$WORK/rollup.json" <<'EOF'
[
  {"name":"build-test","status":"COMPLETED","conclusion":"SUCCESS"},
  {"name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}
]
EOF
    export CI_WAIT_TEST_ROLLUP="$WORK/rollup.json"

    run bash "$SCRIPT" "$PR"
    [ "$status" -eq 0 ]

    wait_for_state "$PR" pass
    local state
    state="$(jq -r '.state' "$HOME/.autospec/ci-state/$PR.signal")"
    [ "$state" = "pass" ]
}

# --- stalled ---

@test "stalled: timeout expires while a check is still IN_PROGRESS" {
    PR=304
    cat > "$WORK/rollup.json" <<'EOF'
[
  {"name":"build-test","status":"IN_PROGRESS","conclusion":""}
]
EOF
    export CI_WAIT_TEST_ROLLUP="$WORK/rollup.json"
    fake_sleep

    run bash "$SCRIPT" "$PR" --timeout 2
    [ "$status" -eq 0 ]

    wait_for_state "$PR" stalled
    local state
    state="$(jq -r '.state' "$HOME/.autospec/ci-state/$PR.signal")"
    [ "$state" = "stalled" ]
}
