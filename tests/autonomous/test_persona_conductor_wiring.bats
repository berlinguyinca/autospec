#!/usr/bin/env bats
# tests/autonomous/test_persona_conductor_wiring.bats
# Phase 5.5 integration contract — conductor (scripts/lib/autospec-loop.sh)
# actually WIRES the F3 mining helper and the F6 recalibrate control signal.
#
# These cover cross-PR gaps the per-PR LGTMs missed:
#   - F3 (autonomous-persona-mine.sh) was shipped but never invoked by the
#     conductor → the precedence-3 mined digest was never produced.
#   - F6 wrote a persona-recalibrate.flag but the conductor never consumed it,
#     so autospec:recalibrate-persona never forced a refresh.
#
# Engineering notes:
#   - Conductor helper scripts mocked under CONDUCTOR_SCRIPTS_DIR (real temp dir).
#   - Mocks log invocations / captured args to real temp files (bash 3.2: no
#     [ -f <(...) ]).
#   - HOME redirected to a temp dir so the recalibrate flag path is isolated.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

setup() {
    TMP="$(mktemp -d -t test-persona-wiring.XXXXXX)"

    SCRIPTS_DIR="$TMP/scripts"
    mkdir -p "$SCRIPTS_DIR"

    # mock: waterfall — always Tier 1 / run-backlog
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":1,"action":"run-backlog","reason":"test-default"}\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-waterfall.sh"

    # mock: control-channel — default: no decisions
    cat > "$SCRIPTS_DIR/autonomous-control-channel.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-control-channel.sh"

    # mock: premerge-gate — always merge-ok
    cat > "$SCRIPTS_DIR/autonomous-premerge-gate.sh" <<'EOF'
#!/usr/bin/env bash
printf 'merge-ok\n'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-premerge-gate.sh"

    # mock: resilience — no-op
    cat > "$SCRIPTS_DIR/autonomous-resilience.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-resilience.sh"

    # mock: spend-ledger — always continue
    cat > "$SCRIPTS_DIR/autonomous-spend-ledger.sh" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "check" ]; then printf 'continue\n'; fi
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-spend-ledger.sh"

    # mock: usage-limit — no-op
    cat > "$SCRIPTS_DIR/autospec-usage-limit.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autospec-usage-limit.sh"

    # mock: persona-mine — records each invocation + its args.
    MINE_LOG="$TMP/mine-invocations.log"
    cat > "$SCRIPTS_DIR/autonomous-persona-mine.sh" <<EOF
#!/usr/bin/env bash
printf 'mine-called args=%s\n' "\$*" >> '$MINE_LOG'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-persona-mine.sh"

    # mock: persona-synth — records each invocation + its args.
    SYNTH_LOG="$TMP/synth-invocations.log"
    cat > "$SCRIPTS_DIR/autonomous-persona-synth.sh" <<EOF
#!/usr/bin/env bash
printf 'synth-called args=%s\n' "\$*" >> '$SYNTH_LOG'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-persona-synth.sh"

    RUN_CMD_LOG="$TMP/run-cmd.log"
    touch "$RUN_CMD_LOG"

    export CONDUCTOR_SCRIPTS_DIR="$SCRIPTS_DIR"
    export CONDUCTOR_MAX_CYCLES=1
    export CONDUCTOR_DRY_RUN=0
    export CONDUCTOR_NO_DIGEST=1
    export CONDUCTOR_POLL_INTERVAL=0
    export AUTOSPEC_RUN_CMD="touch $RUN_CMD_LOG"
    export HOME="$TMP/home"
    mkdir -p "$HOME/.autospec"

    export MINE_LOG SYNTH_LOG

    unset CONDUCTOR_PRIORITIES 2>/dev/null || true
    unset AUTOSPEC_ASK_PRIORITIES_CMD 2>/dev/null || true
    unset AUTOSPEC_CONTROL_STATE_DIR 2>/dev/null || true
}

teardown() {
    rm -rf "$TMP"
}

# ── F3 mining wiring ───────────────────────────────────────────────────────

@test "conductor: invokes persona-mine each cycle (F3 wired)" {
    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    [ -f "$MINE_LOG" ]
    grep -q 'mine-called' "$MINE_LOG"
}

@test "conductor: mine runs before synth (digest feeds synthesis)" {
    # A single combined log preserves ordering across the two mocks.
    ORDER_LOG="$TMP/order.log"
    cat > "$SCRIPTS_DIR/autonomous-persona-mine.sh" <<EOF
#!/usr/bin/env bash
printf 'mine\n' >> '$ORDER_LOG'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-persona-mine.sh"
    cat > "$SCRIPTS_DIR/autonomous-persona-synth.sh" <<EOF
#!/usr/bin/env bash
printf 'synth\n' >> '$ORDER_LOG'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-persona-synth.sh"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    [ -f "$ORDER_LOG" ]
    # First line must be mine, then synth.
    run head -2 "$ORDER_LOG"
    [ "${lines[0]}" = "mine" ]
    [ "${lines[1]}" = "synth" ]
}

# ── F6 recalibrate flag consumption ────────────────────────────────────────

@test "conductor: no recalibrate flag → mine/synth NOT forced" {
    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    # Neither helper should receive --force in the steady state.
    [ -f "$MINE_LOG" ]
    run grep -c -- '--force' "$MINE_LOG"
    [ "$output" = "0" ]
    [ -f "$SYNTH_LOG" ]
    run grep -c -- '--force' "$SYNTH_LOG"
    [ "$output" = "0" ]
}

@test "conductor: recalibrate flag forces refresh and is cleared" {
    # Drop the flag the control channel would have written.
    RECAL_FLAG="$HOME/.autospec/persona-recalibrate.flag"
    printf 'recalibrate\n2026-06-26T00:00:00Z\n' > "$RECAL_FLAG"
    [ -f "$RECAL_FLAG" ]

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    # Both helpers must have been forced.
    grep -q -- '--force' "$MINE_LOG"
    grep -q -- '--force' "$SYNTH_LOG"

    # Flag must be consumed (removed) so the refresh happens exactly once.
    [ ! -f "$RECAL_FLAG" ]
}

@test "conductor: recalibrate flag honored under AUTOSPEC_CONTROL_STATE_DIR" {
    STATE_DIR="$TMP/ctrl-state"
    mkdir -p "$STATE_DIR"
    export AUTOSPEC_CONTROL_STATE_DIR="$STATE_DIR"
    RECAL_FLAG="$STATE_DIR/persona-recalibrate.flag"
    printf 'recalibrate\n2026-06-26T00:00:00Z\n' > "$RECAL_FLAG"

    (
        . "$LOOP_LIB"
        autospec_conductor_run
    ) 2>/dev/null || true

    grep -q -- '--force' "$SYNTH_LOG"
    [ ! -f "$RECAL_FLAG" ]
}
