#!/usr/bin/env bats
# tests/unit/test_ci_wait.bats — Exercises ci-wait.sh, ci-wait-poll.sh, ci-wait-cleanup.sh
# Uses stubbed gh commands to test state transitions without real GitHub API calls.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    CI_WAIT="$REPO_ROOT/scripts/ci-wait.sh"
    CI_WAIT_POLL="$REPO_ROOT/scripts/ci-wait-poll.sh"
    CI_WAIT_CLEANUP="$REPO_ROOT/scripts/ci-wait-cleanup.sh"

    # Isolated CI state dir per test
    export HOME_ORIG="$HOME"
    export TMPDIR_HOME="$(mktemp -d)"
    export HOME="$TMPDIR_HOME"
    mkdir -p "$TMPDIR_HOME/.autospec/ci-state"

    # Stub bin dir on PATH
    STUB_BIN="$(mktemp -d)"
    export PATH="$STUB_BIN:$PATH"
}

teardown() {
    rm -rf "$TMPDIR_HOME" "$STUB_BIN" 2>/dev/null || true
    export HOME="$HOME_ORIG"
}

# ── syntax checks ─────────────────────────────────────────────────────────────

@test "ci-wait: bash -n exits 0" {
    run bash -n "$CI_WAIT"
    [ "$status" -eq 0 ]
}

@test "ci-wait-poll: bash -n exits 0" {
    run bash -n "$CI_WAIT_POLL"
    [ "$status" -eq 0 ]
}

@test "ci-wait-cleanup: bash -n exits 0" {
    run bash -n "$CI_WAIT_CLEANUP"
    [ "$status" -eq 0 ]
}

# ── ci-wait.sh: spawns quickly ────────────────────────────────────────────────

@test "ci-wait: returns within 2 seconds" {
    # Stub gh to return empty rollup (keeps poller running in background)
    cat > "$STUB_BIN/gh" <<'EOF'
#!/usr/bin/env bash
printf '[]\n'
exit 0
EOF
    chmod +x "$STUB_BIN/gh"

    start="$(date +%s)"
    run bash "$CI_WAIT" 9999 --timeout 5
    elapsed=$(( $(date +%s) - start ))
    [ "$status" -eq 0 ]
    [ "$elapsed" -lt 3 ]

    # Cleanup background poller
    bash "$CI_WAIT_CLEANUP" 9999 2>/dev/null || true
}

@test "ci-wait: writes initial pending signal file immediately" {
    cat > "$STUB_BIN/gh" <<'EOF'
#!/usr/bin/env bash
printf '[]\n'
exit 0
EOF
    chmod +x "$STUB_BIN/gh"

    run bash "$CI_WAIT" 8888 --timeout 5
    [ "$status" -eq 0 ]
    [ -f "$TMPDIR_HOME/.autospec/ci-state/8888.signal" ]
    grep -q '"state"' "$TMPDIR_HOME/.autospec/ci-state/8888.signal"

    bash "$CI_WAIT_CLEANUP" 8888 2>/dev/null || true
}

@test "ci-wait: writes PID file" {
    cat > "$STUB_BIN/gh" <<'EOF'
#!/usr/bin/env bash
printf '[]\n'
exit 0
EOF
    chmod +x "$STUB_BIN/gh"

    run bash "$CI_WAIT" 7777 --timeout 5
    [ "$status" -eq 0 ]
    [ -f "$TMPDIR_HOME/.autospec/ci-state/7777.pid" ]
    pid="$(cat "$TMPDIR_HOME/.autospec/ci-state/7777.pid")"
    [ -n "$pid" ]

    bash "$CI_WAIT_CLEANUP" 7777 2>/dev/null || true
}

@test "ci-wait: signal JSON has required fields" {
    cat > "$STUB_BIN/gh" <<'EOF'
#!/usr/bin/env bash
printf '[]\n'
exit 0
EOF
    chmod +x "$STUB_BIN/gh"

    run bash "$CI_WAIT" 6666 --timeout 5
    [ "$status" -eq 0 ]

    sig="$TMPDIR_HOME/.autospec/ci-state/6666.signal"
    jq -e '.pr' "$sig" > /dev/null
    jq -e '.state' "$sig" > /dev/null

    bash "$CI_WAIT_CLEANUP" 6666 2>/dev/null || true
}

# ── ci-wait-poll.sh: exit code mapping ───────────────────────────────────────

@test "ci-wait-poll: exit 3 when no sentinel exists" {
    run bash "$CI_WAIT_POLL" 99999
    [ "$status" -eq 3 ]
}

@test "ci-wait-poll: exit 0 when signal state=pass" {
    PR=5555
    sig="$TMPDIR_HOME/.autospec/ci-state/${PR}.signal"
    printf '{"pr":"%s","state":"pass","checks":[],"settled_at":"2026-05-21T00:00:00Z"}\n' "$PR" > "$sig"
    run bash "$CI_WAIT_POLL" "$PR"
    [ "$status" -eq 0 ]
    [ "$output" = "pass" ]
}

@test "ci-wait-poll: exit 1 when signal state=fail" {
    PR=5556
    sig="$TMPDIR_HOME/.autospec/ci-state/${PR}.signal"
    printf '{"pr":"%s","state":"fail","checks":[],"settled_at":"2026-05-21T00:00:00Z"}\n' "$PR" > "$sig"
    run bash "$CI_WAIT_POLL" "$PR"
    [ "$status" -eq 1 ]
    [ "$output" = "fail" ]
}

@test "ci-wait-poll: exit 1 when signal state=stalled" {
    PR=5557
    sig="$TMPDIR_HOME/.autospec/ci-state/${PR}.signal"
    printf '{"pr":"%s","state":"stalled","checks":[],"settled_at":"2026-05-21T00:00:00Z"}\n' "$PR" > "$sig"
    run bash "$CI_WAIT_POLL" "$PR"
    [ "$status" -eq 1 ]
    [ "$output" = "stalled" ]
}

@test "ci-wait-poll: exit 2 when signal state=pending" {
    PR=5558
    sig="$TMPDIR_HOME/.autospec/ci-state/${PR}.signal"
    printf '{"pr":"%s","state":"pending","checks":[],"settled_at":null}\n' "$PR" > "$sig"
    run bash "$CI_WAIT_POLL" "$PR"
    [ "$status" -eq 2 ]
    [ "$output" = "pending" ]
}

# ── ci-wait-cleanup.sh ────────────────────────────────────────────────────────

@test "ci-wait-cleanup: removes signal, pid, and log files" {
    PR=4444
    dir="$TMPDIR_HOME/.autospec/ci-state"
    touch "$dir/${PR}.signal" "$dir/${PR}.pid" "$dir/${PR}.log"

    run bash "$CI_WAIT_CLEANUP" "$PR"
    [ "$status" -eq 0 ]
    [ ! -f "$dir/${PR}.signal" ]
    [ ! -f "$dir/${PR}.pid" ]
    [ ! -f "$dir/${PR}.log" ]
}

@test "ci-wait-cleanup: exit 0 when no files exist (safe)" {
    run bash "$CI_WAIT_CLEANUP" 33333
    [ "$status" -eq 0 ]
}

@test "ci-wait-cleanup: does not error if only some files exist" {
    PR=4445
    dir="$TMPDIR_HOME/.autospec/ci-state"
    # Only signal file (no pid or log)
    touch "$dir/${PR}.signal"

    run bash "$CI_WAIT_CLEANUP" "$PR"
    [ "$status" -eq 0 ]
    [ ! -f "$dir/${PR}.signal" ]
}

# ── SKILL.md: no blocking gh pr checks --watch ───────────────────────────────

@test "skills/autospec-run/SKILL.md: no synchronous gh pr checks --watch invocation" {
    run grep -n "gh pr checks --watch" "$REPO_ROOT/skills/autospec-run/SKILL.md"
    [ "$status" -ne 0 ]
}

@test "skills/autospec-run/SKILL.md: references ci-wait.sh sentinel" {
    grep -q "ci-wait.sh" "$REPO_ROOT/skills/autospec-run/SKILL.md"
}
