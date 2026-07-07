#!/usr/bin/env bats
# tests/autonomous/test_outcome_ledger.bats — outcome-ledger wiring unit tests
# for autospec_conductor_run() (F5, issue #1397).
#
# Covers:
#  - conductor invokes explore-source-weights.sh before a Tier 2+ discovery cycle
#  - a shipped discovery issue (is_discovery=true last-outcome.json) triggers
#    exactly 1 explore-ledger.sh --update-outcome call with the correct issue
#    and outcome (source is resolved internally by the ledger, not passed)
#  - a ledger call that returns non-zero does not abort the cycle (fail-open)
#  - Tier-1 backlog issues (no last-outcome.json) do not trigger ledger recording
#
# All external scripts (explore-ledger.sh, explore-source-weights.sh, gh,
# autonomous-premerge-gate.sh, autonomous-waterfall.sh, autonomous-resilience.sh,
# autonomous-spend-ledger.sh) are stubbed via PATH injection so no network or
# disk side-effects occur.
#
# Bash 3.2 compat: no [[ ]], no process substitution in [ -f ].
# set -eu + if/then/fi; jq capture()/== (no interpolated test()).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"

    # Isolated temp dir per test.
    TMP="$(mktemp -d -t conductor_ledger.XXXXXX)"
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin" "$TMP/scripts" "$TMP/repo/.autospec"

    # ── Stubs ──────────────────────────────────────────────────────────────────

    # autonomous-waterfall.sh: default → Tier 1 run-backlog.
    cat > "$TMP/bin/autonomous-waterfall.sh" <<'SH'
#!/usr/bin/env bash
printf '{"tier":1,"action":"run-backlog","reason":"stub"}\n'
exit 0
SH

    # autonomous-premerge-gate.sh: default → merge-ok.
    cat > "$TMP/bin/autonomous-premerge-gate.sh" <<'SH'
#!/usr/bin/env bash
printf 'merge-ok\n'
exit 0
SH

    # autonomous-resilience.sh: no-op (fail-open).
    cat > "$TMP/bin/autonomous-resilience.sh" <<'SH'
#!/usr/bin/env bash
exit 0
SH

    # autonomous-spend-ledger.sh: always "continue".
    cat > "$TMP/bin/autonomous-spend-ledger.sh" <<'SH'
#!/usr/bin/env bash
printf 'continue\n'
exit 0
SH

    # autonomous-control-channel.sh: no control signal.
    cat > "$TMP/bin/autonomous-control-channel.sh" <<'SH'
#!/usr/bin/env bash
exit 0
SH

    # gh: no-op stub.
    cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
exit 0
SH

    # explore-source-weights.sh: records call, emits empty JSON.
    WEIGHTS_LOG="$TMP/weights.log"
    export WEIGHTS_LOG
    cat > "$TMP/bin/explore-source-weights.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$WEIGHTS_LOG"
printf '{}\n'
exit 0
SH

    # explore-ledger.sh: records call, exits 0 by default.
    LEDGER_LOG="$TMP/ledger.log"
    export LEDGER_LOG
    cat > "$TMP/bin/explore-ledger.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$LEDGER_LOG"
exit 0
SH

    chmod +x "$TMP/bin/"*

    # Export overrides so the conductor resolves our stubs.
    export AUTOSPEC_EXPLORE_WEIGHTS_BIN="$TMP/bin/explore-source-weights.sh"
    export AUTOSPEC_EXPLORE_LEDGER_BIN="$TMP/bin/explore-ledger.sh"
    export AUTOSPEC_EXPLORE_LEDGER="$TMP/repo/.autospec/explore-ledger.jsonl"
    export AUTOSPEC_LAST_OUTCOME_FILE="$TMP/repo/.autospec/last-outcome.json"
    export AUTOSPEC_ENABLE_DISCOVERY_TIERS=1

    # Use $TMP/bin as CONDUCTOR_SCRIPTS_DIR so all helpers resolve to our stubs.
    export CONDUCTOR_SCRIPTS_DIR="$TMP/bin"
    export CONDUCTOR_REPO=""
    export CONDUCTOR_NO_DIGEST=1
    export CONDUCTOR_POLL_INTERVAL=0
}

teardown() {
    rm -rf "$TMP"
}

# Helper: source the lib and run conductor with 1 cycle, then return.
_run_conductor_1() {
    # We source the lib so autospec_conductor_run is available, then call it
    # with CONDUCTOR_MAX_CYCLES=1. AUTOSPEC_RUN_CMD is set by each test.
    CONDUCTOR_MAX_CYCLES=1 bash -c "
        . '$LIB'
        autospec_conductor_run
    " 2>&1
}

# ─── Tier 2+ weights consultation ─────────────────────────────────────────────

@test "conductor consults explore-source-weights.sh when Tier 2 waterfall fires" {
    # Override waterfall stub to return Tier 2 (not-enabled → weights consult + park).
    cat > "$TMP/bin/autonomous-waterfall.sh" <<'SH'
#!/usr/bin/env bash
printf '{"tier":2,"action":"run-explore-once","reason":"stub"}\n'
exit 0
SH

    export AUTOSPEC_RUN_CMD=""   # no drain needed
    run _run_conductor_1
    # Weights script must have been invoked.
    [ -f "$WEIGHTS_LOG" ]
    weights_calls="$(wc -l < "$WEIGHTS_LOG" | tr -d ' ')"
    [ "$weights_calls" -ge 1 ]
}

@test "conductor passes ledger path to explore-source-weights.sh" {
    cat > "$TMP/bin/autonomous-waterfall.sh" <<'SH'
#!/usr/bin/env bash
printf '{"tier":2,"action":"run-explore-once","reason":"stub"}\n'
exit 0
SH

    export AUTOSPEC_RUN_CMD=""
    run _run_conductor_1
    [ -f "$WEIGHTS_LOG" ]
    # The call must include --ledger.
    grep -q -- '--ledger' "$WEIGHTS_LOG"
}

# ─── Discovery-issue outcome recording ────────────────────────────────────────

@test "shipped discovery issue triggers exactly 1 explore-ledger --update-outcome call" {
    # AUTOSPEC_RUN_CMD writes a discovery outcome file then exits.
    cat > "$TMP/run-cmd.sh" <<SH
#!/usr/bin/env bash
printf '{"is_discovery":true,"issue":42,"source":"spec-vs-code","outcome":"merged_clean"}' \
    > "$AUTOSPEC_LAST_OUTCOME_FILE"
exit 0
SH
    chmod +x "$TMP/run-cmd.sh"
    export AUTOSPEC_RUN_CMD="bash $TMP/run-cmd.sh"

    run _run_conductor_1
    [ -f "$LEDGER_LOG" ]
    ledger_calls="$(wc -l < "$LEDGER_LOG" | tr -d ' ')"
    [ "$ledger_calls" -eq 1 ]
    grep -q -- '--update-outcome' "$LEDGER_LOG"
}

@test "outcome record call includes the issue number from last-outcome.json" {
    cat > "$TMP/run-cmd.sh" <<SH
#!/usr/bin/env bash
printf '{"is_discovery":true,"issue":99,"source":"prior-reports","outcome":"merged_clean"}' \
    > "$AUTOSPEC_LAST_OUTCOME_FILE"
exit 0
SH
    chmod +x "$TMP/run-cmd.sh"
    export AUTOSPEC_RUN_CMD="bash $TMP/run-cmd.sh"

    run _run_conductor_1
    [ -f "$LEDGER_LOG" ]
    grep -q '99' "$LEDGER_LOG"
}

@test "outcome record call includes the outcome value from last-outcome.json" {
    cat > "$TMP/run-cmd.sh" <<SH
#!/usr/bin/env bash
printf '{"is_discovery":true,"issue":7,"source":"codebase-signals","outcome":"qa_failed"}' \
    > "$AUTOSPEC_LAST_OUTCOME_FILE"
exit 0
SH
    chmod +x "$TMP/run-cmd.sh"
    export AUTOSPEC_RUN_CMD="bash $TMP/run-cmd.sh"

    run _run_conductor_1
    [ -f "$LEDGER_LOG" ]
    grep -q 'qa_failed' "$LEDGER_LOG"
}

# ─── Fail-open: ledger errors never abort the cycle ───────────────────────────

@test "failing explore-ledger.sh does not abort the conductor cycle" {
    # Override ledger stub to return non-zero.
    cat > "$TMP/bin/explore-ledger.sh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$LEDGER_LOG"
exit 2
SH
    chmod +x "$TMP/bin/explore-ledger.sh"

    cat > "$TMP/run-cmd.sh" <<SH
#!/usr/bin/env bash
printf '{"is_discovery":true,"issue":5,"source":"open-issues","outcome":"merged_clean"}' \
    > "$AUTOSPEC_LAST_OUTCOME_FILE"
exit 0
SH
    chmod +x "$TMP/run-cmd.sh"
    export AUTOSPEC_RUN_CMD="bash $TMP/run-cmd.sh"

    # Conductor must exit 0 even though the ledger call fails.
    run _run_conductor_1
    [ "$status" -eq 0 ]
}

@test "failing explore-source-weights.sh does not abort the conductor cycle" {
    cat > "$TMP/bin/autonomous-waterfall.sh" <<'SH'
#!/usr/bin/env bash
printf '{"tier":2,"action":"run-explore-once","reason":"stub"}\n'
exit 0
SH
    # Override weights stub to return non-zero.
    cat > "$TMP/bin/explore-source-weights.sh" <<SH
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$WEIGHTS_LOG"
exit 1
SH
    chmod +x "$TMP/bin/explore-source-weights.sh"

    export AUTOSPEC_RUN_CMD=""
    run _run_conductor_1
    [ "$status" -eq 0 ]
}

# ─── Tier-1 backlog issues skip ledger recording ──────────────────────────────

@test "Tier-1 backlog issue (no last-outcome.json) does not trigger ledger recording" {
    # AUTOSPEC_RUN_CMD does NOT write a last-outcome.json.
    cat > "$TMP/run-cmd.sh" <<'SH'
#!/usr/bin/env bash
exit 0
SH
    chmod +x "$TMP/run-cmd.sh"
    export AUTOSPEC_RUN_CMD="bash $TMP/run-cmd.sh"

    run _run_conductor_1
    # ledger.log must not exist (or be empty — no update-outcome call).
    if [ -f "$LEDGER_LOG" ]; then
        grep -v '^$' "$LEDGER_LOG" | grep -c '--update-outcome' | grep -q '^0$'
    fi
}

@test "last-outcome.json with is_discovery=false is silently skipped" {
    cat > "$TMP/run-cmd.sh" <<SH
#!/usr/bin/env bash
printf '{"is_discovery":false,"issue":3,"source":"spec-vs-code","outcome":"merged_clean"}' \
    > "$AUTOSPEC_LAST_OUTCOME_FILE"
exit 0
SH
    chmod +x "$TMP/run-cmd.sh"
    export AUTOSPEC_RUN_CMD="bash $TMP/run-cmd.sh"

    run _run_conductor_1
    if [ -f "$LEDGER_LOG" ]; then
        count="$(grep -c -- '--update-outcome' "$LEDGER_LOG" || true)"
        [ "$count" -eq 0 ]
    fi
}
