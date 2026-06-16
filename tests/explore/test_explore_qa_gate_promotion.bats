#!/usr/bin/env bats
# tests/explore/test_explore_qa_gate_promotion.bats
#
# Issue #1114 — --qa-gate loop integration in scripts/autospec-explore.sh.
#
# Drives the real orchestrator end-to-end (no mocks of git) against a synthetic
# repo, terminating immediately via the operator-stop flag so the QA gate runs
# ONCE at loop termination, before the final-summary promotion block. A stub
# gate runner (AUTOSPEC_EXPLORE_QA_GATE_CMD seam) writes a seeded
# .autospec/explore-qa-gate.json verdict; we assert the promotion-readiness
# output in .autospec/explore-summary.md per the verdict table:
#   PASS / PARTIAL(pass-on-partial) / skipped  -> merge block printed, annotated
#   FAIL / PARTIAL(default) / error            -> merge block withheld + discard
#                                                 + code_health:explore_qa_gate_failed
#   no --qa-gate flag                          -> output byte-identical to today

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    ORCH="$REPO_ROOT/scripts/autospec-explore.sh"
    TMP="$(mktemp -d -t explore-qa-gate.XXXXXX)"
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

    # Terminate immediately at the first iteration boundary (operator_stop) so
    # the gate runs once at termination.
    touch "$HOME/.autospec/explore-stop.flag"
}

teardown() {
    cd /
    rm -rf "$TMP"
}

# Install a stub gate runner that writes the given verdict to the gate file.
# The orchestrator invokes it via the AUTOSPEC_EXPLORE_QA_GATE_CMD seam.
_seed_gate() {
    local verdict="$1"
    export AUTOSPEC_EXPLORE_QA_GATE_CMD="\
SANDBOX_HEAD_SHA=\$(git -C '$TMP' rev-parse HEAD); \
jq -n --arg v '$verdict' --arg b sandbox/explore-qg --arg h \"\$SANDBOX_HEAD_SHA\" \
  '{verdict:\$v, sandbox_branch:\$b, sandbox_head_sha:\$h, qa_verdict_path:\".autospec/qa-verdict.json\", blocking_findings:[\"login form 500s\"], ran_at:\"2026-06-16T00:00:00Z\"}' \
  > '$TMP/.autospec/explore-qa-gate.json'"
}

_run_explore() {
    run bash "$ORCH" "ship cool feature" \
        --max-iterations 1 \
        --sandbox-slug explore-qg \
        --research-sources spec-vs-code \
        "$@"
}

@test "qa-gate PASS: prints merge block annotated sandbox QA: PASS" {
    _seed_gate "PASS"
    _run_explore --qa-gate
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    local md="$TMP/.autospec/explore-summary.md"
    grep -q 'To merge sandbox into main' "$md"
    grep -q 'sandbox QA: PASS' "$md"
}

@test "qa-gate FAIL: withholds merge block, prints discard + emits code_health" {
    _seed_gate "FAIL"
    _run_explore --qa-gate
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    local orch_output="$output"
    local md="$TMP/.autospec/explore-summary.md"
    run grep -q 'To merge sandbox into main' "$md"
    [ "$status" -ne 0 ]
    grep -q 'sandbox QA: FAIL' "$md"
    grep -q 'To discard' "$md"
    grep -q '.autospec/qa-verdict.json' "$md"
    echo "$orch_output" | grep -q 'code_health:explore_qa_gate_failed'
}

@test "qa-gate PARTIAL default: withholds merge block + emits code_health" {
    _seed_gate "PARTIAL"
    _run_explore --qa-gate
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    run grep -q 'To merge sandbox into main' "$TMP/.autospec/explore-summary.md"
    [ "$status" -ne 0 ]
    grep -q 'sandbox QA: PARTIAL' "$TMP/.autospec/explore-summary.md"
}

@test "qa-gate PARTIAL with --qa-gate-pass-on-partial: prints merge block" {
    _seed_gate "PARTIAL"
    _run_explore --qa-gate --qa-gate-pass-on-partial
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    grep -q 'To merge sandbox into main' "$TMP/.autospec/explore-summary.md"
    grep -q 'sandbox QA: PASS' "$TMP/.autospec/explore-summary.md"
}

@test "qa-gate skipped: prints merge block with no-config annotation" {
    _seed_gate "skipped"
    _run_explore --qa-gate
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    grep -q 'To merge sandbox into main' "$TMP/.autospec/explore-summary.md"
    grep -q 'sandbox QA: skipped (no QA config)' "$TMP/.autospec/explore-summary.md"
}

@test "qa-gate OFF (no flag): summary byte-identical to today" {
    # Run once WITHOUT the flag and without seeding any gate file.
    unset AUTOSPEC_EXPLORE_QA_GATE_CMD
    _run_explore
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    local got="$TMP/.autospec/explore-summary.md"
    # Reconstruct the expected legacy summary block independently (the exact
    # promotion text the script emitted before #1114).
    local sb
    sb="$(grep -o '"branch"[[:space:]]*:[[:space:]]*"[^"]*"' "$TMP/.autospec/explore-mode.json" \
        | sed 's/.*"branch"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')"
    grep -q "To merge sandbox into main:" "$got"
    grep -q "git checkout main && git merge $sb" "$got"
    grep -q "To discard:" "$got"
    # No QA annotation must appear at all when the gate is off.
    run grep -q 'sandbox QA:' "$got"
    [ "$status" -ne 0 ]
}
