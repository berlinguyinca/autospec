#!/usr/bin/env bats
# tests/autonomous/test_explore_bridge.bats — explore LLM-bridge wrapper.
#
# The conductor loop (scripts/lib/autospec-loop.sh) resolves the Tier-2/3/4
# discovery command from AUTOSPEC_EXPLORE_CMD and parses the wrapper's stdout
# as the explore yield contract:
#   {"tier":"local|competitor","proposals_seen":N,"new_candidates":N,
#    "filed":N,"dry":<bool>,"reason":"..."}
#
# scripts/autospec-autonomous-explore-drain.sh bridges the explore skill through
# the LLM harness (omx), mirroring autospec-autonomous-run-drain.sh. It derives
# filed/dry from the count of `auto-implement` issues created during the run, and
# degrades to a clean dry (exit 0) on harness absence/failure — a failed explore
# is a clean dry, never a conductor crash.
#
# Mocking: PATH-shim omx + gh; no network. The gh mock reports an issue count
# read from a state file; the omx mock mutates that file to simulate filing.

BRIDGE="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)/scripts/autospec-autonomous-explore-drain.sh"
LAUNCHER="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)/scripts/autospec-autonomous.sh"

setup() {
    TMP="$(mktemp -d -t explore-bridge.XXXXXX)"
    mkdir -p "$TMP/bin"
    export PATH="$TMP/bin:$PATH"

    # gh mock: `gh issue list … --jq length` prints the current issue count.
    # `gh repo view` prints a repo slug. Everything else is a benign no-op.
    COUNT_FILE="$TMP/issue-count"
    printf '0\n' > "$COUNT_FILE"
    cat > "$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
for a in "\$@"; do
    case "\$a" in
        list) MODE=list ;;
        view) MODE=view ;;
    esac
done
if [ "\${MODE:-}" = "view" ]; then
    printf 'owner/repo\n'
    exit 0
fi
if [ "\${MODE:-}" = "list" ]; then
    cat "$COUNT_FILE"
    exit 0
fi
exit 0
EOF
    chmod +x "$TMP/bin/gh"

    # Default omx mock: succeeds, files 1 issue (bumps the count).
    OMX_LOG="$TMP/omx-args.log"
    cat > "$TMP/bin/omx" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$OMX_LOG"
printf '2\n' > "$COUNT_FILE"
exit 0
EOF
    chmod +x "$TMP/bin/omx"

    # Deterministic, fast: no stall watchdog (plain wait).
    export AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS=0
    export CONDUCTOR_REPO="owner/repo"
    export AUTOSPEC_REPO_DIR="$TMP"
}

teardown() {
    rm -rf "$TMP"
}

# ── contract shape ────────────────────────────────────────────────────────────

@test "bridge emits valid contract JSON on a successful explore (filed>0 -> dry=false)" {
    run bash "$BRIDGE" --once
    [ "$status" -eq 0 ]
    # Last stdout line must be the parseable contract with filed>0, dry=false.
    line="$(printf '%s\n' "$output" | grep '"filed"' | tail -1)"
    [ -n "$line" ]
    echo "$line" | jq -e '.dry == false' >/dev/null
    echo "$line" | jq -e '.filed >= 1' >/dev/null
    echo "$line" | jq -e '.tier == "local"' >/dev/null
}

@test "bridge tier is competitor when --research-sources internet is passed" {
    run bash "$BRIDGE" --once --research-sources internet
    [ "$status" -eq 0 ]
    line="$(printf '%s\n' "$output" | grep '"filed"' | tail -1)"
    echo "$line" | jq -e '.tier == "competitor"' >/dev/null
}

@test "bridge forwards the skill invocation to omx" {
    run bash "$BRIDGE" --once
    [ "$status" -eq 0 ]
    grep -q 'autospec-explore --once' "$OMX_LOG"
}

@test "bridge forwards --research-sources to the omx skill invocation" {
    run bash "$BRIDGE" --once --research-sources internet
    [ "$status" -eq 0 ]
    grep -q 'research-sources internet' "$OMX_LOG"
}

# ── dry explore ───────────────────────────────────────────────────────────────

@test "bridge reports a clean dry (filed=0 -> dry=true) when explore files nothing" {
    # omx mock leaves the issue count unchanged -> filed=0.
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/bin/omx"

    run bash "$BRIDGE" --once
    [ "$status" -eq 0 ]
    line="$(printf '%s\n' "$output" | grep '"filed"' | tail -1)"
    echo "$line" | jq -e '.dry == true' >/dev/null
    echo "$line" | jq -e '.filed == 0' >/dev/null
}

# ── failure / harness absence -> clean dry, exit 0, never crash ───────────────

@test "bridge degrades to a clean dry (exit 0) when omx exits non-zero" {
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
echo "omx: boom" >&2
exit 3
EOF
    chmod +x "$TMP/bin/omx"

    run bash "$BRIDGE" --once
    [ "$status" -eq 0 ]
    line="$(printf '%s\n' "$output" | grep '"filed"' | tail -1)"
    echo "$line" | jq -e '.dry == true' >/dev/null
    echo "$line" | jq -e '.filed == 0' >/dev/null
    echo "$line" | jq -e '.reason == "explore-error"' >/dev/null
}

@test "bridge degrades to a clean dry (exit 0) when omx is absent from PATH" {
    rm -f "$TMP/bin/omx"
    ln -sf /usr/bin/dirname "$TMP/bin/dirname"
    # Ensure omx is genuinely unavailable even on developer hosts with /usr/bin/omx or /bin/omx.
    run env PATH="$TMP/bin" /bin/bash "$BRIDGE" --once
    [ "$status" -eq 0 ]
    line="$(printf '%s\n' "$output" | grep '"filed"' | tail -1)"
    echo "$line" | jq -e '.dry == true' >/dev/null
    echo "$line" | jq -e '.filed == 0' >/dev/null
}

@test "bridge runs safely under set -eu (no errexit abort on a failing explore)" {
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
exit 5
EOF
    chmod +x "$TMP/bin/omx"

    run bash -c "set -eu; bash '$BRIDGE' --once"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '"dry":true'
}

@test "bridge bounds the direct verifier fallback" {
    run bash -n "$BRIDGE"
    [ "$status" -eq 0 ]
    grep -q 'direct fallback max runtime' "$BRIDGE"
    grep -q 'autospec_kill_process_tree "\$direct_pid"' "$BRIDGE"
}

@test "researcher timeout polling handles exited and zombie children" {
    cycle="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)/scripts/explore-research-cycle.sh"
    run bash -n "$cycle"
    [ "$status" -eq 0 ]
    grep -q 'case "\$_state" in' "$cycle"
    grep -q "Z\*|'') break" "$cycle"
}

@test "once explore path wires the verifier command" {
    explore="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)/scripts/autospec-explore.sh"
    run bash -n "$explore"
    [ "$status" -eq 0 ]
    grep -q 'AUTOSPEC_EXPLORE_DEDUPED_IN=' "$explore"
    grep -q 'data\["verify_mode"\] = "active"' "$explore"
}

# ── launcher wiring ───────────────────────────────────────────────────────────

@test "launcher exports AUTOSPEC_EXPLORE_CMD to the bridge path by default" {
    grep -q 'AUTOSPEC_EXPLORE_CMD="\${AUTOSPEC_EXPLORE_CMD:-\$SCRIPT_DIR/autospec-autonomous-explore-drain.sh}"' \
        "$LAUNCHER"
}

@test "launcher AUTOSPEC_EXPLORE_CMD default is overridable (guarded with :-)" {
    # The :- guard means a pre-set AUTOSPEC_EXPLORE_CMD wins.
    grep -q 'AUTOSPEC_EXPLORE_CMD:-' "$LAUNCHER"
}
