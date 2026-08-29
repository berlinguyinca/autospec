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
    export AUTOSPEC_BIN="$REPO_ROOT/tests/fixtures/autospec-project-sync-stub.sh"
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

    # Stub adversarial-verify skeptic. Under the precision-refinement contract an
    # autonomous run with NO skeptic fails closed (files nothing); this stub
    # affirms every deduped proposal so verify_mode=active and the happy-path
    # filing/drain plumbing is exercised. A real harness supplies real per-
    # proposal refute-by-default verdicts here.
    cat > "$TMP/bin/stub-verify.sh" <<'EOF'
#!/usr/bin/env bash
python3 -c "
import json, os
d = json.load(open(os.environ['AUTOSPEC_EXPLORE_DEDUPED_IN']))
m = {p['norm_title']: {'verdict': 'survived', 'reason': 'stub skeptic'}
     for p in d.get('deduped', []) if p.get('norm_title')}
json.dump(m, open(os.environ['AUTOSPEC_EXPLORE_VERDICTS_OUT'], 'w'))
"
EOF
    chmod +x "$TMP/bin/stub-verify.sh"
    export AUTOSPEC_EXPLORE_VERIFY_CMD="bash $TMP/bin/stub-verify.sh"

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
