#!/usr/bin/env bats
# tests/autonomous/test_sandbox_routing.bats — F3: sandbox PR-base routing +
# main-merge refusal in autospec_conductor_run().
#
# Covers:
#   1. Tier 2 entry calls explore-sandbox.sh (idempotent sandbox init).
#   2. Tier 3 entry calls explore-sandbox.sh.
#   3. explore-sandbox.sh is called with --base main.
#   4. Tier 2 entry writes .autospec/explore-mode.json.
#   5. Main merge refused (code_health:autonomous_main_merge_refused) when
#      discovery issues are in-flight.
#   6. AUTOSPEC_RUN_CMD not invoked when discovery in-flight.
#   7. Tier-1 backlog drain to main succeeds when no discovery in-flight.
#   8. No refusal emitted when discovery is not in-flight.
#
# Mocking strategy:
#   - All helper scripts mocked via CONDUCTOR_SCRIPTS_DIR (subprocess boundary).
#   - explore-sandbox.sh mocked via AUTOSPEC_SANDBOX_BIN (F3 env seam).
#   - gh is never called (no real network; waterfall uses --backlog-count inject).
#   - bats fixtures written to real temp files (bash 3.2 compat: no process-sub).

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

# ── helpers ──────────────────────────────────────────────────────────────────

setup() {
    TMP="$(mktemp -d -t sandbox-routing.XXXXXX)"
    SCRIPTS_DIR="$TMP/scripts"
    mkdir -p "$SCRIPTS_DIR"

    # Log files — written to real temp files (bash 3.2: no <(...) in [ -f ]).
    SANDBOX_CALL_LOG="$TMP/sandbox-calls.log"
    RUN_CMD_LOG="$TMP/run-cmd.log"
    touch "$SANDBOX_CALL_LOG"
    touch "$RUN_CMD_LOG"

    # explore-mode.json location mirrors what conductor sees:
    # _repo_root = $(cd "${_sdir}/.." && pwd) = TMP.
    MODE_FILE="$TMP/.autospec/explore-mode.json"

    # ── mock: explore-sandbox.sh ─────────────────────────────────────────────
    # Logs its args, then writes explore-mode.json to $TMP/.autospec/.
    cat > "$SCRIPTS_DIR/explore-sandbox.sh" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$SANDBOX_CALL_LOG"
mkdir -p "$(dirname "$MODE_FILE")"
printf '{"branch":"autospec/explore/2026-06-26-test","slug":"test","base":"main","head_sha":"abc","created_at":"2026-06-26T00:00:00Z"}\n' \\
    > "$MODE_FILE"
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/explore-sandbox.sh"

    # ── mock: autonomous-waterfall.sh ─────────────────────────────────────────
    # Default: Tier 1 / run-backlog.
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

    # ── mock: autonomous-premerge-gate.sh ────────────────────────────────────
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

    # ── mock: autonomous-spend-ledger.sh ─────────────────────────────────────
    cat > "$SCRIPTS_DIR/autonomous-spend-ledger.sh" <<'EOF'
#!/usr/bin/env bash
if [ "${1:-}" = "check" ]; then
    printf 'continue\n'
fi
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-spend-ledger.sh"

    # ── mock: explore command (Tier 2/3) — dry by default ────────────────────
    EXPLORE_SCRIPT="$TMP/fake-explore.sh"
    cat > "$EXPLORE_SCRIPT" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":"local","proposals_seen":0,"new_candidates":0,"filed":0,"dry":true,"reason":"test-dry"}\n'
exit 0
EOF
    chmod +x "$EXPLORE_SCRIPT"

    # ── conductor environment ─────────────────────────────────────────────────
    export CONDUCTOR_SCRIPTS_DIR="$SCRIPTS_DIR"
    export CONDUCTOR_MAX_CYCLES=1
    export CONDUCTOR_DRY_RUN=0
    export CONDUCTOR_NO_DIGEST=1
    export CONDUCTOR_POLL_INTERVAL=0
    export AUTOSPEC_RUN_CMD="printf 'run-cmd-invoked\n' >> $RUN_CMD_LOG"
    export AUTOSPEC_EXPLORE_CMD="bash $EXPLORE_SCRIPT"
    export AUTOSPEC_SANDBOX_BIN="$SCRIPTS_DIR/explore-sandbox.sh"
    # No real repo — no lock/heartbeat/notify.
    unset CONDUCTOR_REPO 2>/dev/null || true
    unset AUTOSPEC_REPO  2>/dev/null || true
}

teardown() {
    rm -rf "$TMP"
}

# Run conductor as a sourced function, capturing combined stdout+stderr.
_run_conductor() {
    bash -c "source '$LOOP_LIB'; autospec_conductor_run" 2>&1
}

# ── 1. Tier 2 entry ──────────────────────────────────────────────────────────

@test "Tier 2 entry calls explore-sandbox.sh" {
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":2,"action":"run-explore-once","reason":"tier2-test"}\n'
exit 0
EOF

    _run_conductor
    [ -s "$SANDBOX_CALL_LOG" ]
}

@test "Tier 2 entry writes explore-mode.json" {
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":2,"action":"run-explore-once","reason":"tier2-test"}\n'
exit 0
EOF

    _run_conductor
    [ -f "$MODE_FILE" ]
}

@test "explore-sandbox.sh called with --base main on Tier 2 entry" {
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":2,"action":"run-explore-once","reason":"tier2-test"}\n'
exit 0
EOF

    _run_conductor
    grep -q -- "--base main" "$SANDBOX_CALL_LOG"
}

# ── 2. Tier 3 entry ──────────────────────────────────────────────────────────

@test "Tier 3 entry calls explore-sandbox.sh" {
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":3,"action":"run-explore-once-internet","reason":"tier3-test"}\n'
exit 0
EOF

    _run_conductor
    [ -s "$SANDBOX_CALL_LOG" ]
}

@test "Tier 3 entry writes explore-mode.json" {
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":3,"action":"run-explore-once-internet","reason":"tier3-test"}\n'
exit 0
EOF

    _run_conductor
    [ -f "$MODE_FILE" ]
}

# ── 3. Main-merge refusal while discovery in-flight ──────────────────────────
#
# Two-cycle conductor run:
#   Cycle 1: Tier 2 — explore files 1 discovery issue → _inflight_discovery=1
#   Cycle 2: Tier 1 — gate passes, but refusal fires because in-flight > 0

@test "main merge refused with identifier when discovery in-flight" {
    CYCLE_COUNT_FILE="$TMP/cycle-count.txt"
    printf '0\n' > "$CYCLE_COUNT_FILE"

    # Waterfall: cycle 1 → Tier 2; cycle 2 → Tier 1
    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<EOF
#!/usr/bin/env bash
n=\$(cat "$CYCLE_COUNT_FILE" 2>/dev/null || printf '0')
n=\$((n + 1))
printf '%s\n' "\$n" > "$CYCLE_COUNT_FILE"
if [ "\$n" -le 1 ]; then
    printf '{"tier":2,"action":"run-explore-once","reason":"cycle1-discovery"}\n'
else
    printf '{"tier":1,"action":"run-backlog","reason":"cycle2-tier1"}\n'
fi
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-waterfall.sh"

    # Explore: non-dry, files 1 issue.
    # Use string "false" for dry — jq's // operator treats boolean false as
    # falsy, returning the right-hand side; string "false" is truthy and is
    # passed through unchanged so _explore_dry becomes the string "false".
    cat > "$EXPLORE_SCRIPT" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":"local","proposals_seen":1,"new_candidates":1,"filed":1,"dry":"false"}\n'
exit 0
EOF

    export CONDUCTOR_MAX_CYCLES=2
    run _run_conductor
    [[ "$output" == *"code_health:autonomous_main_merge_refused"* ]]
}

@test "drain still runs onto sandbox while discovery in-flight (phase4 refuses main merges)" {
    # Regression for the Phase 5.5 deadlock finding: the conductor must NOT skip
    # the Tier-1 drain while discovery is in-flight. The phase4 implementer
    # enforces the main-merge refusal per-PR via explore-mode.json (PRs target
    # the sandbox). Skipping the drain here would deadlock, because the only path
    # that decrements the in-flight counter lives inside the drain branch.
    CYCLE_COUNT_FILE="$TMP/cycle-count.txt"
    printf '0\n' > "$CYCLE_COUNT_FILE"

    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<EOF
#!/usr/bin/env bash
n=\$(cat "$CYCLE_COUNT_FILE" 2>/dev/null || printf '0')
n=\$((n + 1))
printf '%s\n' "\$n" > "$CYCLE_COUNT_FILE"
if [ "\$n" -le 1 ]; then
    printf '{"tier":2,"action":"run-explore-once","reason":"cycle1-discovery"}\n'
else
    printf '{"tier":1,"action":"run-backlog","reason":"cycle2-tier1"}\n'
fi
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-waterfall.sh"

    # String "false" — see comment in prior test for why.
    cat > "$EXPLORE_SCRIPT" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":"local","proposals_seen":1,"new_candidates":1,"filed":1,"dry":"false"}\n'
exit 0
EOF

    export CONDUCTOR_MAX_CYCLES=2
    _run_conductor
    # The drain MUST run on cycle 2 even though discovery is in-flight.
    [ -s "$RUN_CMD_LOG" ]
    grep -q "run-cmd-invoked" "$RUN_CMD_LOG"
}

@test "in-flight counter clears once a discovery outcome is consumed (no permanent deadlock)" {
    # Three cycles: cycle 1 files a discovery issue (in-flight=1); cycles 2-3 are
    # Tier-1 drains. The drain mock writes a discovery outcome file, which the
    # conductor consumes to decrement the counter. By the time the counter hits 0
    # the main-merge refusal must stop firing — proving the counter can clear.
    CYCLE_COUNT_FILE="$TMP/cycle-count.txt"
    printf '0\n' > "$CYCLE_COUNT_FILE"

    OUTCOME_FILE="$TMP/.autospec/last-outcome.json"
    export AUTOSPEC_LAST_OUTCOME_FILE="$OUTCOME_FILE"

    cat > "$SCRIPTS_DIR/autonomous-waterfall.sh" <<EOF
#!/usr/bin/env bash
n=\$(cat "$CYCLE_COUNT_FILE" 2>/dev/null || printf '0')
n=\$((n + 1))
printf '%s\n' "\$n" > "$CYCLE_COUNT_FILE"
if [ "\$n" -le 1 ]; then
    printf '{"tier":2,"action":"run-explore-once","reason":"cycle1-discovery"}\n'
else
    printf '{"tier":1,"action":"run-backlog","reason":"tier1-drain"}\n'
fi
exit 0
EOF
    chmod +x "$SCRIPTS_DIR/autonomous-waterfall.sh"

    cat > "$EXPLORE_SCRIPT" <<'EOF'
#!/usr/bin/env bash
printf '{"tier":"local","proposals_seen":1,"new_candidates":1,"filed":1,"dry":"false"}\n'
exit 0
EOF

    # Drain mock writes a discovery outcome file so the conductor decrements.
    export AUTOSPEC_RUN_CMD="printf 'run-cmd-invoked\n' >> $RUN_CMD_LOG; mkdir -p '$TMP/.autospec'; printf '{\"is_discovery\":true,\"issue\":4242,\"source\":\"spec-vs-code\",\"outcome\":\"shipped\"}\n' > '$OUTCOME_FILE'"

    export CONDUCTOR_MAX_CYCLES=3
    run _run_conductor
    # The decrement path must be reachable: counter returns to 0.
    [[ "$output" == *"in-flight=0"* ]]
}

# ── 4. Tier-1 backlog drain unaffected when no discovery in-flight ───────────

@test "Tier-1 backlog drain to main succeeds when no discovery in-flight" {
    # Default waterfall: Tier 1 / run-backlog.
    # No prior Tier 2/3 → _inflight_discovery=0 → drain proceeds.
    _run_conductor
    [ -s "$RUN_CMD_LOG" ]
    grep -q "run-cmd-invoked" "$RUN_CMD_LOG"
}

@test "no main-merge-refused emitted when discovery is not in-flight" {
    run _run_conductor
    [[ "$output" != *"code_health:autonomous_main_merge_refused"* ]]
}
