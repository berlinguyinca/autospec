#!/usr/bin/env bats
# tests/explore/test_explore_orchestrator_verify.bats — issue #1095.
#
# The adversarial verify stage must ACTUALLY run end-to-end through the
# orchestrator (scripts/autospec-explore.sh), not just at the aggregator
# boundary with a manually-injected verdict map. The orchestrator runs a
# two-pass research cycle:
#   pass 1 (--stage dedup)    emits deduped proposals with normalized-title keys
#   skeptic dispatch          builds {norm_title -> {verdict, reason}} (refute-by-
#                             default) into a verdict map
#   pass 2 (--stage finalize) consumes the map -> drops refuted -> verify_mode
#                             active, proposals_refuted>0
# and a `refuted` outcome is recorded to the ledger so the refutation-rate
# down-weighting (explore-source-weights.sh) closes the loop.
#
# These tests drive the REAL orchestrator path via the AUTOSPEC_EXPLORE_VERIFY_CMD
# seam (the production seam an LLM harness fills) — no aggregator-boundary
# injection.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t explore-orch-verify.XXXXXX)"
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
echo "gh $*" >> "$AUTOSPEC_REPO_ROOT/.gh-calls.log"
case "$1" in
    issue)
        case "$2" in
            create) echo "https://github.com/x/y/issues/$RANDOM" ;;
            list)   echo '[]' ;;
            view)   echo '{}' ;;
        esac ;;
    pr) echo '[]' ;;
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

    # One researcher emits a real proposal AND a seeded bogus one.
    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR"
    cat > "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"source":"spec-vs-code","proposals":[
  {"title":"feat: real durable fix","evidence":"reproduced gap","estimated_complexity":"small","confidence":0.9},
  {"title":"feat: bogus hallucinated claim","evidence":"none","estimated_complexity":"small","confidence":0.9}
]}
JSON
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh"

    export AUTOSPEC_EXPLORE_DRAIN_CMD="echo drain >> $TMP/.drain.log"
    export AUTOSPEC_EXPLORE_REFINE_CMD="echo refine >> $TMP/.refine.log"
    export AUTOSPEC_EXPLORE_DEFINE_CMD="echo define >> $TMP/.define.log"
    export AUTOSPEC_EXPLORE_LEDGER="$TMP/.autospec/explore-ledger.jsonl"
    # Resolve the ledger bin to the real script (in production REPO_ROOT is the
    # autospec repo and this resolves via the repo path; the synthetic test repo
    # has no skills/ tree, so point at it explicitly).
    export AUTOSPEC_EXPLORE_LEDGER_BIN="$REPO_ROOT/skills/autospec-shared/scripts/explore-ledger.sh"
}

teardown() {
    rm -rf "$TMP"
}

# A skeptic that refutes any proposal whose normalized title contains "bogus"
# and affirms the rest. Reads the pass-1 deduped artifact, writes the verdict
# map — exactly the seam contract the orchestrator hands an LLM harness.
write_skeptic() {
    cat > "$TMP/bin/skeptic.sh" <<'EOF'
#!/usr/bin/env bash
python3 - "$AUTOSPEC_EXPLORE_DEDUPED_IN" "$AUTOSPEC_EXPLORE_VERDICTS_OUT" <<'PY'
import json, sys
dd = json.load(open(sys.argv[1]))
m = {}
for p in dd.get("deduped", []) or []:
    n = p.get("norm_title", "")
    if not n:
        continue
    if "bogus" in n:
        m[n] = {"verdict": "refuted", "reason": "claim does not reproduce"}
    else:
        m[n] = {"verdict": "survived", "reason": "reproduced the gap"}
json.dump(m, open(sys.argv[2], "w"))
PY
EOF
    chmod +x "$TMP/bin/skeptic.sh"
    export AUTOSPEC_EXPLORE_VERIFY_CMD="bash $TMP/bin/skeptic.sh"
}

@test "two-pass: seeded bogus proposal is refuted-and-dropped end-to-end (verify_mode=active, proposals_refuted>0)" {
    write_skeptic
    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 1 \
        --max-issues-per-round 5 \
        --sandbox-slug orch-verify \
        --specialists-mode off \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    research_json="$(ls "$TMP"/.autospec/explore-iter-1/research.json)"
    [ -f "$research_json" ]
    python3 -c "
import json
d = json.load(open('$research_json'))
assert d['verify_mode'] == 'active', d
assert d['proposals_refuted'] >= 1, d
titles = [p['title'] for p in d['proposals']]
assert 'feat: real durable fix' in titles, titles
assert all('bogus' not in t for t in titles), titles
"
}

@test "two-pass: a refuted outcome is recorded to the ledger (closes the down-weight loop)" {
    write_skeptic
    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 1 \
        --max-issues-per-round 5 \
        --sandbox-slug orch-ledger \
        --specialists-mode off \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    [ -f "$AUTOSPEC_EXPLORE_LEDGER" ]
    # At least one ledger line records outcome=refuted for the bogus proposal,
    # attributed to its source (so the source refutation-rate rises).
    run bash "$REPO_ROOT/skills/autospec-shared/scripts/explore-ledger.sh" \
        --ledger "$AUTOSPEC_EXPLORE_LEDGER" --show --json
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "
import sys, json
arr = json.load(sys.stdin)
refuted = [r for r in arr if r.get('outcome') == 'refuted']
assert refuted, ('no refuted ledger record', arr)
assert any('bogus' in r.get('norm_title','') for r in refuted), refuted
assert all(r.get('issue', -1) == 0 for r in refuted), refuted
assert any(r.get('source') == 'spec-vs-code' for r in refuted), refuted
"
    # And the stats put that refutation on the source's refuted tally.
    run bash "$REPO_ROOT/skills/autospec-shared/scripts/explore-ledger.sh" \
        --ledger "$AUTOSPEC_EXPLORE_LEDGER" --stats --json
    [ "$status" -eq 0 ]
    echo "$output" | python3 -c "
import sys, json
s = json.load(sys.stdin)
assert s.get('spec-vs-code',{}).get('refuted',0) >= 1, s
"
}

@test "two-pass: no skeptic capability degrades to the OBSERVABLE no-op (not silent all-survive)" {
    # No AUTOSPEC_EXPLORE_VERIFY_CMD and no harness dispatcher: rung 3. The gate
    # must no-op observably (verify_mode=no-op-unverified + code_health warning),
    # NOT silently let everything survive unverified with verify_mode=active.
    unset AUTOSPEC_EXPLORE_VERIFY_CMD
    # Neutralize harness dispatcher resolution so rung 2 cannot fire.
    export AUTOSPEC_HARNESS_DISPATCHER=""
    rm -f "$TMP/bin/claude"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        "ship cool feature" \
        --max-iterations 1 \
        --max-issues-per-round 5 \
        --sandbox-slug orch-noop \
        --specialists-mode off \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    research_json="$(ls "$TMP"/.autospec/explore-iter-1/research.json)"
    python3 -c "
import json
d = json.load(open('$research_json'))
assert d['verify_mode'] == 'no-op-unverified', d
# Observable no-op: proposals survive but the mode flags the inert gate.
assert d['proposals_refuted'] == 0, d
"
    echo "$output" | grep -q 'code_health:explore_verify_noop'
}
