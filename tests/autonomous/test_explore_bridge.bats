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
# the active LLM harness. It derives
# filed/dry from the count of `auto-implement` issues created during the run, and
# returns non-zero on harness absence/failure so a failed explore cannot be
# confused with a completed empty repository scan.
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

    # Default codex mock: succeeds, files 2 issues (bumps the count).
    HARNESS_LOG="$TMP/harness-args.log"
    cat > "$TMP/bin/codex" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$HARNESS_LOG"
printf '2\n' > "$COUNT_FILE"
exit 0
EOF
    chmod +x "$TMP/bin/codex"
    cat > "$TMP/bin/omx" <<'EOF'
#!/usr/bin/env bash
echo "unexpected omx invocation" >&2
exit 99
EOF
    chmod +x "$TMP/bin/omx"

    # Deterministic, fast: no stall watchdog (plain wait).
    export AUTOSPEC_AUTONOMOUS_EXPLORE_STALL_SECS=0
    export CONDUCTOR_REPO="owner/repo"
    export AUTOSPEC_REPO_DIR="$TMP"
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=codex
    export AUTOSPEC_HANDOFF_DISPATCHER=1
    export AUTOSPEC_HARNESS_PROBE_ROOT="$TMP/probes"
    export AUTOSPEC_HARNESS_RUNTIME_ALIASES="$TMP/aliases.tsv"
    printf 'claude\tclaude\t--dangerously-skip-permissions\tClaude Code\ncodex\tcodex\t--yolo\tCodex CLI\nopencode\topencode\t\tOpenCode\n' \
        > "$AUTOSPEC_HARNESS_RUNTIME_ALIASES"
}

teardown() {
    rm -rf "$TMP"
}

write_codex_failure() {
    local code="$1"
    printf '%s\n' '#!/usr/bin/env bash' 'echo "codex: boom" >&2' "exit $code" \
        > "$TMP/bin/codex"
    chmod +x "$TMP/bin/codex"
}

write_codex_success_noop() {
    printf '%s\n' '#!/usr/bin/env bash' 'exit 0' > "$TMP/bin/codex"
    chmod +x "$TMP/bin/codex"
}

# ── contract shape ────────────────────────────────────────────────────────────

@test "bridge emits valid contract JSON on a successful explore (filed>0 -> dry=false)" {
    run /bin/bash "$BRIDGE" --once
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

@test "bridge forwards the skill invocation to the active harness" {
    run bash "$BRIDGE" --once
    [ "$status" -eq 0 ]
    grep -q 'autospec-explore --once' "$HARNESS_LOG"
}

@test "bridge forwards --research-sources to the active harness" {
    run bash "$BRIDGE" --once --research-sources internet
    [ "$status" -eq 0 ]
    grep -q 'research-sources internet' "$HARNESS_LOG"
}

@test "bridge uses Claude slash-skill syntax when Claude owns the session" {
    cat > "$TMP/bin/claude" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$HARNESS_LOG"
printf '1\n' > "$COUNT_FILE"
EOF
    chmod +x "$TMP/bin/claude"
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude

    run bash "$BRIDGE" --once
    [ "$status" -eq 0 ]
    grep -q '/autospec-explore --once' "$HARNESS_LOG"
    ! grep -q '\$autospec-explore' "$HARNESS_LOG"
}

@test "bridge uses OpenCode slash-skill syntax when OpenCode owns the session" {
    cat > "$TMP/bin/opencode" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$HARNESS_LOG"
printf '1\n' > "$COUNT_FILE"
EOF
    chmod +x "$TMP/bin/opencode"
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=opencode

    run bash "$BRIDGE" --once
    [ "$status" -eq 0 ]
    grep -q '/autospec-explore --once' "$HARNESS_LOG"
    ! grep -q '\$autospec-explore' "$HARNESS_LOG"
}

# ── dry explore ───────────────────────────────────────────────────────────────

@test "bridge reports a clean dry (filed=0 -> dry=true) when explore files nothing" {
    # Harness mock leaves the issue count unchanged -> filed=0.
    write_codex_success_noop

    run /bin/bash "$BRIDGE" --once
    [ "$status" -eq 0 ]
    line="$(printf '%s\n' "$output" | grep '"filed"' | tail -1)"
    echo "$line" | jq -e '.dry == true' >/dev/null
    echo "$line" | jq -e '.filed == 0' >/dev/null
}

# ── failure / harness absence -> non-zero, never clean dry ────────────────────

@test "bridge fails visibly when the active harness exits non-zero" {
    write_codex_failure 3

    run bash "$BRIDGE" --once
    [ "$status" -ne 0 ]
    [[ "$output" == *"explore-error"* ]]
    [[ "$output" != *'"dry":true'* ]]
}

@test "bridge fails visibly when the selected harness is absent from PATH" {
    rm -f "$TMP/bin/codex"
    ln -sf /usr/bin/dirname "$TMP/bin/dirname"
    run /usr/bin/env PATH="$TMP/bin" /bin/bash "$BRIDGE" --once
    [ "$status" -ne 0 ]
    [[ "$output" != *'"dry":true'* ]]
}

@test "bridge preserves a non-zero harness result under set -eu" {
    write_codex_failure 5

    run /bin/bash -c "set -eu; bash '$BRIDGE' --once"
    [ "$status" -ne 0 ]
    [[ "$output" != *'"dry":true'* ]]
}

@test "bridge bounds the direct verifier fallback" {
    run bash -n "$BRIDGE"
    [ "$status" -eq 0 ]
    grep -q 'direct fallback max runtime' "$BRIDGE"
    grep -q 'autospec_kill_tree "\$direct_pid" separate' "$BRIDGE"
}

@test "bridge preserves a non-zero direct verifier fallback result" {
    local direct_root="$TMP/direct"
    mkdir -p "$direct_root/scripts/lib"
    cp "$BRIDGE" "$direct_root/scripts/autospec-autonomous-explore-drain.sh"
    cp "$(dirname "$BRIDGE")/lib/autospec-harness-detect.sh" "$direct_root/scripts/lib/"
    cp "$(dirname "$BRIDGE")/lib/autospec-process-tree.sh" "$direct_root/scripts/lib/"
    cat > "$direct_root/scripts/autospec-explore.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' '{"tier":"local","proposals_seen":0,"new_candidates":0,"filed":0,"dry":false,"reason":"research-incomplete"}'
exit 4
EOF
    chmod +x "$direct_root/scripts/"*.sh
    cat > "$TMP/bin/codex" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' 'AUTOSPEC_EXPLORE_VERIFY_CMD_not_executed'
exit 0
EOF
    chmod +x "$TMP/bin/codex"

    run bash "$direct_root/scripts/autospec-autonomous-explore-drain.sh" --once
    [ "$status" -eq 4 ]
    [[ "$output" == *'"reason":"research-incomplete"'* ]]
    [[ "$output" != *'"dry":true'* ]]
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
