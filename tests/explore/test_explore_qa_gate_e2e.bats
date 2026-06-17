#!/usr/bin/env bats
# tests/explore/test_explore_qa_gate_e2e.bats
#
# Phase 5.5 cross-script integration guard (issue #1116).
#
# The per-PR suites test the two halves in isolation:
#   - test_explore_qa_gate_runner.bats     drives scripts/explore-qa-gate.sh alone.
#   - test_explore_qa_gate_promotion.bats  drives scripts/autospec-explore.sh but
#     SEEDS the gate file via the AUTOSPEC_EXPLORE_QA_GATE_CMD seam — it never
#     runs the real runner.
#
# Neither composes the REAL runner -> REAL loop. A field/path rename in the
# runner's .autospec/explore-qa-gate.json (e.g. sandbox_head_sha -> head_sha)
# would silently break verdict threading while BOTH isolated suites stay green.
# This test closes that seam: it runs the orchestrator with --qa-gate and NO
# seam override, so the real explore-qa-gate.sh produces the verdict file the
# loop then consumes. It also proves autospec-qa is invoked with --no-heal.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    ORCH="$REPO_ROOT/scripts/autospec-explore.sh"
    TMP="$(mktemp -d -t explore-qag-e2e.XXXXXX)"
    cd "$TMP"
    git init -q -b main
    git config user.email t@t.t
    git config user.name t
    git commit --allow-empty -q -m "seed"
    git init -q --bare "$TMP/remote.git"
    git remote add origin "$TMP/remote.git"
    git push -q origin main
    git fetch -q origin main

    export AUTOSPEC_REPO_ROOT="$TMP"
    export HOME="$TMP/home"
    mkdir -p "$HOME/.autospec"
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude

    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
case "$1 $2" in
    "issue create") echo "https://github.com/x/y/issues/$RANDOM" ;;
    "issue list")   echo '[]' ;;
    *) : ;;
esac
exit 0
EOF
    chmod +x "$TMP/bin/gh"
    cat > "$TMP/bin/claude" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/bin/claude"
    export PATH="$TMP/bin:$PATH"

    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR"
    cat > "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"source":"spec-vs-code","proposals":[
  {"title":"feat: explore proposal one","evidence":"ev1","estimated_complexity":"small","confidence":0.9}
]}
JSON
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh"

    export AUTOSPEC_EXPLORE_DRAIN_CMD="true"
    export AUTOSPEC_EXPLORE_REFINE_CMD="true"
    export AUTOSPEC_EXPLORE_DEFINE_CMD="true"

    # Terminate immediately so the gate runs once at termination.
    touch "$HOME/.autospec/explore-stop.flag"

    # IMPORTANT: do NOT set AUTOSPEC_EXPLORE_QA_GATE_CMD — exercise the real
    # scripts/explore-qa-gate.sh runner.
    unset AUTOSPEC_EXPLORE_QA_GATE_CMD 2>/dev/null || true

    # Confine the runner's throwaway worktree to TMP.
    export AUTOSPEC_QA_GATE_WORKTREE="$TMP/wt-qa-gate"

    # Record where autospec-qa was invoked with which args.
    QA_ARGS_LOG="$TMP/qa-args.log"
    export QA_ARGS_LOG
}

teardown() {
    cd /
    rm -rf "$TMP"
}

# Stub autospec-qa on PATH: records its args (to prove --no-heal) and writes the
# given verdict into the cwd's .autospec/qa-verdict.json (the sandbox worktree
# the real runner cd'd into).
make_qa_stub() {
    local verdict="$1"
    local rc="${2:-0}"
    cat > "$TMP/bin/autospec-qa" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$QA_ARGS_LOG"
mkdir -p .autospec
cat > .autospec/qa-verdict.json <<JSON
{"verdict":"$verdict","findings":[{"category":"smoke","release_blocking":true,"summary":"login 500s","evidence":"trace"}]}
JSON
exit $rc
EOF
    chmod +x "$TMP/bin/autospec-qa"
}

_run_explore() {
    run bash "$ORCH" "ship cool feature" \
        --max-iterations 1 \
        --sandbox-slug explore-qg \
        --research-sources spec-vs-code \
        "$@"
}

@test "e2e FAIL: real runner verdict threads to loop -> merge block WITHHELD" {
    # QA config present so the real runner does NOT skip.
    mkdir -p "$TMP/.autospec"
    printf 'smoke: true\n' > "$TMP/.autospec/test.yml"
    make_qa_stub "FAIL" 1
    _run_explore --qa-gate
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    # Capture orchestrator output before any later `run` overwrites $output.
    local orch_output="$output"

    # The REAL runner wrote the gate file (not a seam).
    local gate="$TMP/.autospec/explore-qa-gate.json"
    [ -f "$gate" ]
    [ "$(jq -r '.verdict' "$gate")" = "FAIL" ]

    # Verdict threaded across scripts -> merge block withheld.
    local md="$TMP/.autospec/explore-summary.md"
    run grep -q 'To merge sandbox into main' "$md"
    [ "$status" -ne 0 ]
    grep -q 'sandbox QA: FAIL' "$md"
    grep -q 'Promotion WITHHELD' "$md"
    echo "$orch_output" | grep -q 'code_health:explore_qa_gate_failed'

    # Proof: autospec-qa was invoked with --no-heal (flag not dropped).
    grep -q -- '--no-heal' "$QA_ARGS_LOG"
}

@test "e2e PASS: real runner verdict threads to loop -> merge block printed" {
    mkdir -p "$TMP/.autospec"
    printf 'smoke: true\n' > "$TMP/.autospec/test.yml"
    make_qa_stub "PASS" 0
    _run_explore --qa-gate
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    local gate="$TMP/.autospec/explore-qa-gate.json"
    [ "$(jq -r '.verdict' "$gate")" = "PASS" ]

    local md="$TMP/.autospec/explore-summary.md"
    grep -q 'To merge sandbox into main' "$md"
    grep -q 'sandbox QA: PASS' "$md"
    grep -q -- '--no-heal' "$QA_ARGS_LOG"
}

@test "e2e skipped: no QA config -> real runner skips -> merge block printed (fail-open)" {
    # No .autospec/test.yml -> runner SKIPs, promotion not penalized.
    make_qa_stub "PASS" 0  # present but never reached
    _run_explore --qa-gate
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    [ "$(jq -r '.verdict' "$TMP/.autospec/explore-qa-gate.json")" = "skipped" ]
    local md="$TMP/.autospec/explore-summary.md"
    grep -q 'To merge sandbox into main' "$md"
    grep -q 'sandbox QA: skipped (no QA config)' "$md"
    # autospec-qa must NOT have run on the skip path.
    [ ! -f "$QA_ARGS_LOG" ]
}

@test "e2e error: QA config present, autospec-qa missing -> real runner errors -> WITHHELD (fail-closed)" {
    mkdir -p "$TMP/.autospec"
    printf 'smoke: true\n' > "$TMP/.autospec/test.yml"
    # Remove the autospec-qa stub so the real runner hits the missing-skill path.
    rm -f "$TMP/bin/autospec-qa"
    _run_explore --qa-gate
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    local orch_output="$output"

    [ "$(jq -r '.verdict' "$TMP/.autospec/explore-qa-gate.json")" = "error" ]
    local md="$TMP/.autospec/explore-summary.md"
    run grep -q 'To merge sandbox into main' "$md"
    [ "$status" -ne 0 ]
    grep -q 'sandbox QA: error' "$md"
    echo "$orch_output" | grep -q 'code_health:explore_qa_gate_failed'
}
