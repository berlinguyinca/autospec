#!/usr/bin/env bats
# tests/explore/test_explore_e2e.bats — end-to-end orchestrator fixture for
# scripts/autospec-explore.sh (issue #721).
#
# Synthetic repo + mocked researchers + mocked /autospec-run dispatcher
# verifies: sandbox created, 1+ research rounds run, proposals filed as
# issues, drain invoked, summary written, sandbox branch present at end.
# Also exercises termination conditions: operator stop, round cap, budget cap.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t explore-e2e.XXXXXX)"
    cd "$TMP"
    git init -q -b main
    git config user.email t@t.t
    git config user.name t
    git commit --allow-empty -q -m "seed"
    # Fake remote so explore-sandbox can push.
    git init -q --bare "$TMP/remote.git"
    git remote add origin "$TMP/remote.git"
    git push -q origin main
    git fetch -q origin main

    export AUTOSPEC_REPO_ROOT="$TMP"
    export HOME="$TMP/home"
    mkdir -p "$HOME/.autospec"
    # Force harness detect to a known kind without needing real binaries.
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude

    # Stub bin: gh, claude (dispatcher).
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
# Record all gh calls.
echo "gh $*" >> "$AUTOSPEC_REPO_ROOT/.gh-calls.log"
case "$1" in
    issue)
        case "$2" in
            create) echo "https://github.com/x/y/issues/$RANDOM" ;;
            list)   echo '[]' ;;
        esac
        ;;
    *) : ;;
esac
exit 0
EOF
    chmod +x "$TMP/bin/gh"
    cat > "$TMP/bin/claude" <<'EOF'
#!/usr/bin/env bash
echo "claude $*" >> "$AUTOSPEC_REPO_ROOT/.claude-calls.log"
exit 0
EOF
    chmod +x "$TMP/bin/claude"
    export PATH="$TMP/bin:$PATH"

    # Override research dir with a single fast deterministic researcher.
    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR"
    cat > "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"source":"spec-vs-code","proposals":[
  {"title":"feat: explore proposal one","evidence":"ev1","estimated_complexity":"small","confidence":0.9},
  {"title":"feat: explore proposal two","evidence":"ev2","estimated_complexity":"medium","confidence":0.7}
]}
JSON
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh"

    # Bypass the actual /autospec-run handoff — record only.
    export AUTOSPEC_EXPLORE_DRAIN_CMD="echo drain-invoked >> $TMP/.drain-calls.log"
    # Bypass refine handoff.
    export AUTOSPEC_EXPLORE_REFINE_CMD="echo refine-invoked >> $TMP/.refine-calls.log"
    # Bypass decompose handoff.
    export AUTOSPEC_EXPLORE_DEFINE_CMD="echo define-invoked >> $TMP/.define-calls.log"
}

teardown() {
    rm -rf "$TMP"
}

@test "orchestrator: full e2e — sandbox created, 1+ rounds, proposals filed, drain invoked, summary written" {
    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 2 \
        --max-issues-per-round 2 \
        --sandbox-slug e2e-test \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    # Sandbox branch exists locally + remote.
    git -C "$TMP" show-ref --verify --quiet "refs/heads/autospec/explore/$(date -u +%Y-%m-%d)-e2e-test"
    [ -f "$TMP/.autospec/explore-mode.json" ]
    grep -q '"slug": "e2e-test"' "$TMP/.autospec/explore-mode.json"

    # Loop artifacts written.
    [ -f "$TMP/.autospec/explore-summary.md" ]
    [ -f "$TMP/.autospec/explore-loop.json" ]
    grep -q 'autospec-explore' "$TMP/.autospec/explore-summary.md"

    # Drain callback invoked.
    [ -f "$TMP/.drain-calls.log" ]

    # Refine callback invoked at startup.
    [ -f "$TMP/.refine-calls.log" ]

    # gh issue create invoked for proposals.
    grep -q 'issue create' "$TMP/.gh-calls.log"
}

@test "orchestrator: --no-initial-handoff skips startup refine and define dispatch" {
    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --no-initial-handoff \
        --max-iterations 1 \
        --sandbox-slug no-handoff-test \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    [ ! -f "$TMP/.refine-calls.log" ]
    [ ! -f "$TMP/.define-calls.log" ]
}

@test "orchestrator: initial handoff timeout logs diagnostics and continues" {
    unset AUTOSPEC_EXPLORE_REFINE_CMD
    unset AUTOSPEC_EXPLORE_DEFINE_CMD
    export AUTOSPEC_HANDOFF_DISPATCHER=1
    export AUTOSPEC_EXPLORE_HANDOFF_TIMEOUT_SEC=1
    cat > "$TMP/bin/claude" <<'EOF'
#!/usr/bin/env bash
echo "claude-start $*" >> "$AUTOSPEC_REPO_ROOT/.claude-calls.log"
echo "claude-start $*"
sleep 30 >/dev/null 2>&1
EOF
    chmod +x "$TMP/bin/claude"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 1 \
        --sandbox-slug handoff-timeout-test \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    echo "$output" | grep -q 'code_health:explore_handoff_timeout step=refine'
    echo "$output" | grep -q 'code_health:explore_handoff_timeout step=define'
    [ -s "$TMP/.autospec/explore-handoff/refine.log" ]
    [ -s "$TMP/.autospec/explore-handoff/define.log" ]
    grep -q 'claude-start' "$TMP/.autospec/explore-handoff/refine.log"
    grep -q 'claude-start' "$TMP/.autospec/explore-handoff/define.log"
}

@test "orchestrator: zero-proposal successful research round is surfaced loudly" {
    cat > "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"source":"spec-vs-code","proposals":[]}
JSON
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 1 \
        --sandbox-slug no-proposals-test \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    echo "$output" | grep -q 'code_health:explore_no_proposals iter=1'
}

@test "orchestrator: termination — operator stop via explore-stop.flag" {
    touch "$HOME/.autospec/explore-stop.flag"
    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 5 \
        --sandbox-slug stop-test \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    [ -f "$TMP/.autospec/explore-loop.json" ]
    grep -q 'operator_stop' "$TMP/.autospec/explore-loop.json"
}

@test "orchestrator: termination — round cap reached" {
    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 1 \
        --sandbox-slug roundcap-test \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    grep -q '"max_iterations": 1' "$TMP/.autospec/explore-loop.json"
}

@test "orchestrator: termination — budget cap (time) reached" {
    export AUTOSPEC_LOOP_TIME_CAP=0
    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 3 \
        --sandbox-slug budget-test \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    grep -q 'budget_cap_reached\|operator_stop\|round_cap_reached\|oscillation_detected' "$TMP/.autospec/explore-loop.json"
}

@test "orchestrator: refuses to run against main when not in sandbox base mismatch" {
    # Pre-existing explore-mode.json claiming a different base must fail.
    mkdir -p "$TMP/.autospec"
    cat > "$TMP/.autospec/explore-mode.json" <<EOF
{"slug":"conflict-test","base":"other-branch","branch":"autospec/explore/x","head_sha":"deadbeef","created_at":"2026-01-01T00:00:00Z"}
EOF
    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 1 \
        --sandbox-slug conflict-test \
        --research-sources spec-vs-code
    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'explore_sandbox_base_mismatch'
}

# --- Discovery enhancement contract (issues #1084/#1085) ---
#
# Build a sourceable copy of validate.sh with the trailing `main "$@"` removed
# so the new contract function can be invoked in isolation against a mutated
# repo copy. Asserts the gate PASSES on the real (lockstep-clean) trio and
# FAILS on a seeded trio lockstep break.

# Helper: a sourceable validate.sh (no auto-run). The function `cd`s itself via
# the validate.sh header `cd "$REPO_ROOT"`, where REPO_ROOT derives from $0 of
# the *sourcing* shell — so callers must cd to the target repo AFTER sourcing
# and pass an absolute REPO_ROOT override.
_sourceable_validate() {
    local out="$1"
    sed 's/^main "\$@"$/: # main suppressed for sourcing/' \
        "$REPO_ROOT/scripts/validate.sh" > "$out"
}

@test "discovery contract: passes on the real lockstep-clean trio" {
    local vsrc="$TMP/validate-src.sh"
    _sourceable_validate "$vsrc"
    run bash -c '
        set -eu
        source "'"$vsrc"'"
        cd "'"$REPO_ROOT"'"
        check_autospec_explore_discovery_contract
    '
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    echo "$output" | grep -q 'discovery enhancement contract'
}

@test "discovery contract: fails on a seeded trio lockstep break" {
    local repo="$TMP/mutrepo"
    mkdir -p "$repo/skills/autospec-explore/codex"
    cp "$REPO_ROOT/skills/autospec-explore/SKILL.md" "$repo/skills/autospec-explore/SKILL.md"
    cp "$REPO_ROOT/skills/autospec-explore/codex/prompt.md" "$repo/skills/autospec-explore/codex/prompt.md"
    mkdir -p "$repo/skills/autospec-explore/opencode"
    cp "$REPO_ROOT/skills/autospec-explore/opencode/agent.md" "$repo/skills/autospec-explore/opencode/agent.md"
    # Seed a lockstep break: append a divergent line to the codex body only.
    printf '\nSEEDED LOCKSTEP DRIFT LINE (codex only)\n' \
        >> "$repo/skills/autospec-explore/codex/prompt.md"
    local vsrc="$TMP/validate-src2.sh"
    _sourceable_validate "$vsrc"
    run bash -c '
        set -eu
        source "'"$vsrc"'"
        cd "'"$repo"'"
        # The seeded break must be caught by the project lock-step check that the
        # full validate run pairs with this gate.
        check_lockstep skills/autospec-explore
    '
    [ "$status" -ne 0 ]
    echo "$output" | grep -qi 'diverges'
}
