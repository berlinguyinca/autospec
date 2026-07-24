#!/usr/bin/env bats
# tests/autonomous/test_idle_rescan.bats — conductor idle-rescan vs park.
#
# Never-idle contract (docs/specs/2026-07-06-autospec-autonomous-platform-design.md,
# R1/R5, F1): an all-tiers-dry cascade must enter idle-rescan — arm resume context
# and re-scan after AUTOSPEC_RESCAN_INTERVAL — and CONTINUE the loop, never
# convergence-stop. Genuine resource/control park (spend-ledger, usage-governor,
# control labels) must still EXIT. --max-cycles must not be bypassed.
#
# Mocking mirrors test_sandbox_routing.bats: all helper scripts mocked via
# CONDUCTOR_SCRIPTS_DIR; no network. The waterfall mock counts its invocations
# via a state file so we can prove the loop continued (idle-rescan) or stopped
# (park) after cycle 1.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

setup() {
    TMP="$(mktemp -d -t idle-rescan.XXXXXX)"
    SCRIPTS_DIR="$TMP/scripts"
    mkdir -p "$SCRIPTS_DIR"

    WF_CALL_LOG="$TMP/wf-calls.log"
    touch "$WF_CALL_LOG"

    # ── mock helper scripts (subprocess boundary) ─────────────────────────────
    cat > "$SCRIPTS_DIR/autonomous-control-channel.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    cat > "$SCRIPTS_DIR/autonomous-premerge-gate.sh" <<'EOF'
#!/usr/bin/env bash
printf 'merge-ok\n'
exit 0
EOF
    cat > "$SCRIPTS_DIR/autonomous-resilience.sh" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    cat > "$SCRIPTS_DIR/autonomous-spend-ledger.sh" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "check" ]; then printf 'continue\n'; fi
exit 0
EOF
    chmod +x "$SCRIPTS_DIR"/*.sh

    export CONDUCTOR_SCRIPTS_DIR="$SCRIPTS_DIR"
    export CONDUCTOR_MAX_CYCLES=2
    export CONDUCTOR_DRY_RUN=0
    export CONDUCTOR_NO_DIGEST=1
    export CONDUCTOR_POLL_INTERVAL=0
    # No real sleep during idle.
    export AUTOSPEC_RESCAN_INTERVAL=0
    export AUTOSPEC_RUN_CMD="true"
    unset CONDUCTOR_REPO 2>/dev/null || true
    unset AUTOSPEC_REPO  2>/dev/null || true
}

teardown() {
    rm -rf "$TMP"
}

_run_conductor() {
    bash -c "source '$LOOP_LIB'; autospec_conductor_run" 2>&1
}

# Run under `set -eu`, matching the real autospec-autonomous.sh contract (it
# sources the lib under `set -eu`). This seam catches errexit aborts inside the
# idle-rescan branch that the plain _run_conductor silently tolerates (cf. the
# #1625 set -e capture bug guarded the same way in test_sandbox_routing.bats).
_run_conductor_set_e() {
    bash -c "set -eu; source '$LOOP_LIB'; autospec_conductor_run" 2>&1
}

# A waterfall mock that logs each invocation, then emits the given JSON.
_waterfall_emitting() {
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<EOF
#!/usr/bin/env bash
printf 'wf-call\n' >> "$WF_CALL_LOG"
printf '%s\n' '$1'
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-waterfall.sh"
}

# ── idle-rescan continues the loop and arms resume ───────────────────────────

@test "idle-rescan arms resume context and does NOT terminate the loop" {
    _waterfall_emitting '{"tier":4,"action":"idle-rescan","reason":"all tiers dry; idle and re-scan"}'

    run _run_conductor
    [ "$status" -eq 0 ]
    # Resume context armed for the idle-rescan.
    [[ "$output" == *"arming resume context"* ]]
    [[ "$output" == *"idle-rescan"* ]]
    # Loop continued: the waterfall was consulted on BOTH cycles (not stopped
    # after cycle 1) and it stopped only at the cycle cap.
    [ "$(grep -c 'wf-call' "$WF_CALL_LOG")" -eq 2 ]
    [[ "$output" != *"parking:"* ]]
}

@test "idle-rescan continues the loop under set -eu (no errexit abort)" {
    _waterfall_emitting '{"tier":4,"action":"idle-rescan","reason":"all tiers dry; idle and re-scan"}'

    run _run_conductor_set_e
    [ "$status" -eq 0 ]
    [[ "$output" == *"arming resume context"* ]]
    # Loop survived both cycles under errexit — the branch never aborted.
    [ "$(grep -c 'wf-call' "$WF_CALL_LOG")" -eq 2 ]
    [[ "$output" != *"parking:"* ]]
}

@test "idle-rescan resets exhausted dry counters before the next waterfall selection" {
    # This guards the actual rescan contract: a dry cascade must re-enter the
    # waterfall, not remain permanently at its exhausted Tier-4 state.
    run grep -F '_dry_cycles=0' "$LOOP_LIB"
    [ "$status" -eq 0 ]
    run grep -F '_tier2_dry_cycles=0' "$LOOP_LIB"
    [ "$status" -eq 0 ]
    run grep -F '_tier4_dry_cycles=0' "$LOOP_LIB"
    [ "$status" -eq 0 ]
}

# ── resource / control park still EXITS ──────────────────────────────────────

@test "waterfall park (convergence-park verb) still exits the loop after one cycle" {
    _waterfall_emitting '{"tier":4,"action":"park","reason":"legacy convergence park"}'

    run _run_conductor
    [ "$status" -eq 0 ]
    [[ "$output" == *"parking:"* ]]
    # Loop broke on cycle 1 — the waterfall was consulted exactly once.
    [ "$(grep -c 'wf-call' "$WF_CALL_LOG")" -eq 1 ]
}

@test "control park (Tier 0) still exits the loop after one cycle" {
    _waterfall_emitting '{"tier":0,"action":"park","reason":"autospec:pause"}'

    run _run_conductor
    [ "$status" -eq 0 ]
    [[ "$output" == *"parking:"* ]]
    [ "$(grep -c 'wf-call' "$WF_CALL_LOG")" -eq 1 ]
}
