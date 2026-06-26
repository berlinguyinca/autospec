#!/usr/bin/env bats
# tests/autonomous/test_conductor_governor.bats — F6: the usage-governor is
# actually wired into autospec_conductor_run() and parks the loop at the soft
# ceiling.
#
# Regression for the Phase 5.5 integration finding: autonomous-usage-governor.sh
# existed and was unit-tested in isolation, but the conductor never invoked it,
# so the 90% soft-park never fired in production.
#
# Mocking strategy:
#   - All helper scripts mocked via CONDUCTOR_SCRIPTS_DIR (subprocess boundary).
#   - The governor mock records that it was called (real temp file) and emits a
#     configurable verdict.
#   - gh is never called (waterfall mock returns a fixed decision).

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

setup() {
    TMP="$(mktemp -d -t conductor-governor.XXXXXX)"
    SCRIPTS_DIR="$TMP/scripts"
    mkdir -p "$SCRIPTS_DIR"

    GOVERNOR_CALL_LOG="$TMP/governor-calls.log"
    RUN_CMD_LOG="$TMP/run-cmd.log"
    touch "$GOVERNOR_CALL_LOG"
    touch "$RUN_CMD_LOG"

    # ── mock: autonomous-waterfall.sh — Tier 1 / run-backlog ──────────────────
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":1,"action":"run-backlog","reason":"test-default"}\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-waterfall.sh"

    # ── mock: autonomous-control-channel.sh ──────────────────────────────────
    cat > "$SCRIPTS_DIR/autonomous-control-channel.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-control-channel.sh"

    # ── mock: autonomous-premerge-gate.sh — merge-ok ─────────────────────────
    cat > "$SCRIPTS_DIR/autonomous-premerge-gate.sh" <<'EOF'
#!/usr/bin/env bash
printf 'merge-ok\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-premerge-gate.sh"

    # ── mock: autonomous-resilience.sh ───────────────────────────────────────
    cat > "$SCRIPTS_DIR/autonomous-resilience.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-resilience.sh"

    # ── mock: autonomous-spend-ledger.sh — never the hard cap (continue) ──────
    cat > "$SCRIPTS_DIR/autonomous-spend-ledger.sh" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "check" ]; then
    printf 'continue\n'
fi
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-spend-ledger.sh"

    # ── mock: autospec-usage-limit.sh — record arm calls (no daemon) ──────────
    cat > "$SCRIPTS_DIR/autospec-usage-limit.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autospec-usage-limit.sh"

    export CONDUCTOR_SCRIPTS_DIR="$SCRIPTS_DIR"
    export CONDUCTOR_MAX_CYCLES=1
    export CONDUCTOR_DRY_RUN=0
    export CONDUCTOR_NO_DIGEST=1
    export CONDUCTOR_POLL_INTERVAL=0
    export AUTOSPEC_RUN_CMD="printf 'run-cmd-invoked\n' >> $RUN_CMD_LOG"
    unset CONDUCTOR_REPO 2>/dev/null || true
    unset AUTOSPEC_REPO  2>/dev/null || true
}

teardown() {
    rm -rf "$TMP"
}

_write_governor_mock() {
    # $1 = verdict line the mock prints (e.g. "continue" or "park <at>")
    local verdict="$1"
    cat > "$SCRIPTS_DIR/autonomous-usage-governor.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$GOVERNOR_CALL_LOG"
printf '${verdict}\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-usage-governor.sh"
}

_run_conductor() {
    bash -c "source '$LOOP_LIB'; autospec_conductor_run" 2>&1
}

@test "conductor invokes the usage-governor each cycle" {
    _write_governor_mock "continue"
    _run_conductor
    [ -s "$GOVERNOR_CALL_LOG" ]
}

@test "conductor passes a valid harness to the governor" {
    _write_governor_mock "continue"
    _run_conductor
    # First field of the recorded args must be a valid harness keyword.
    grep -Eq '^(claude|codex|opencode)\b' "$GOVERNOR_CALL_LOG"
}

@test "governor park stops the conductor with usage-governor:park" {
    _write_governor_mock "park 2026-06-26T12:00:00Z"
    run _run_conductor
    [[ "$output" == *"usage-governor:park"* ]]
}

@test "governor continue lets the loop proceed (no park)" {
    _write_governor_mock "continue"
    run _run_conductor
    [[ "$output" != *"usage-governor:park"* ]]
}

@test "AUTOSPEC_GOVERNOR_HARNESS override is honored and validated" {
    _write_governor_mock "continue"
    export AUTOSPEC_GOVERNOR_HARNESS="codex"
    _run_conductor
    grep -Eq '^codex\b' "$GOVERNOR_CALL_LOG"
}

@test "invalid harness override falls back to claude" {
    _write_governor_mock "continue"
    export AUTOSPEC_GOVERNOR_HARNESS="bogus-harness"
    _run_conductor
    grep -Eq '^claude\b' "$GOVERNOR_CALL_LOG"
}
