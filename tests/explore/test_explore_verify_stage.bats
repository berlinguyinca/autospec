#!/usr/bin/env bats
# tests/explore/test_explore_verify_stage.bats — adversarial VERIFY stage
# (Issue C) in scripts/explore-research-cycle.sh.
#
# The deterministic aggregator owns the verify *boundary*, not the LLM
# refutation itself (the skeptic is an explore-orchestrator/SKILL-prose
# responsibility). The aggregator:
#   - threads every deduped proposal through a verify boundary,
#   - consumes an OPTIONAL verifier verdict map (normalized title ->
#     {verdict, reason}) supplied via AUTOSPEC_EXPLORE_VERIFY_VERDICTS,
#   - DROPS proposals whose verdict is "refuted",
#   - attaches {verdict, reason} to survivors,
#   - falls back to "all survive" (no-op) when no verdict is supplied,
#   - emits proposals_after_verify and proposals_refuted counters.
#
# No LLM is invoked from bash. The tests inject verdicts directly to exercise
# the deterministic boundary.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t explore-verify.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
    chmod +x "$TMP/bin/gh"
    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR"
}

teardown() {
    rm -rf "$TMP"
}

make_fake_researcher() {
    local name="$1"
    local payload="$2"
    cat > "$AUTOSPEC_RESEARCH_DIR/$name.sh" <<EOF
#!/usr/bin/env bash
cat <<JSON
$payload
JSON
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/$name.sh"
}

empties() {
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'
}

@test "no verdict map supplied -> verify is a no-op; all survive, zero refuted" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: alpha widget","evidence":"e1","estimated_complexity":"small","confidence":0.9}]}'
    empties

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['proposals_after_dedup'] == 1, d
assert d['proposals_after_verify'] == 1, d
assert d['proposals_refuted'] == 0, d
assert len(d['proposals']) == 1, d
p = d['proposals'][0]
# Survivor carries a verdict + reason even in the no-op fallback.
assert p['verdict'] in ('survived', 'unverified'), p
assert 'reason' in p, p
"
}

@test "a refuted proposal is dropped; a real one survives with verdict+reason" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: real fix","evidence":"e1","estimated_complexity":"small","confidence":0.9},{"title":"feat: bogus claim","evidence":"e2","estimated_complexity":"small","confidence":0.9}]}'
    empties

    # Verdict map is keyed by NORMALIZED title (the aggregator strips the
    # conventional-commit prefix before keying).
    cat > "$TMP/verdicts.json" <<'EOF'
{
  "real fix": {"verdict":"survived","reason":"reproduced the gap"},
  "bogus claim": {"verdict":"refuted","reason":"claim does not hold"}
}
EOF
    export AUTOSPEC_EXPLORE_VERIFY_VERDICTS="$TMP/verdicts.json"

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['proposals_after_dedup'] == 2, d
assert d['proposals_after_verify'] == 1, d
assert d['proposals_refuted'] == 1, d
titles = [p['title'] for p in d['proposals']]
assert titles == ['feat: real fix'], titles
p = d['proposals'][0]
assert p['verdict'] == 'survived', p
assert p['reason'] == 'reproduced the gap', p
"
}

@test "refute-by-default on the ambiguous case (verdict map present, proposal absent)" {
    # When a verdict map IS supplied but a given proposal has no entry, the
    # skeptic could not affirm it -> refute by default (drop).
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: affirmed","evidence":"e1","estimated_complexity":"small","confidence":0.9},{"title":"feat: unmentioned","evidence":"e2","estimated_complexity":"small","confidence":0.9}]}'
    empties

    cat > "$TMP/verdicts.json" <<'EOF'
{ "affirmed": {"verdict":"survived","reason":"verified"} }
EOF
    export AUTOSPEC_EXPLORE_VERIFY_VERDICTS="$TMP/verdicts.json"

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['proposals_refuted'] == 1, d
assert d['proposals_after_verify'] == 1, d
titles = [p['title'] for p in d['proposals']]
assert titles == ['feat: affirmed'], titles
"
}

@test "verify_mode=no-op-unverified surfaces the inert-gate state (audit #1086 seam-1)" {
    # Regression for the inert-dead-gate failure mode: with no orchestrator-
    # supplied verdict map the verify stage no-ops, and that MUST be observable
    # in the structured output so an operator (and the orchestrator's
    # code_health warning) can tell "skeptic refuted nothing" from "skeptic
    # never ran". stdout stays pure JSON — the human warning is the
    # orchestrator's job off this field.
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: alpha","evidence":"e1","estimated_complexity":"small","confidence":0.9}]}'
    empties

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['verify_mode'] == 'no-op-unverified', d
"
}

@test "verify_mode=active when a verdict map is supplied (audit #1086 seam-1)" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: real fix","evidence":"e1","estimated_complexity":"small","confidence":0.9}]}'
    empties
    cat > "$TMP/verdicts.json" <<'EOF'
{ "real fix": {"verdict":"survived","reason":"verified"} }
EOF
    export AUTOSPEC_EXPLORE_VERIFY_VERDICTS="$TMP/verdicts.json"

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['verify_mode'] == 'active', d
"
}

@test "all four discovery-enhance counters are present in the iteration log" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: a","evidence":"e1","estimated_complexity":"small","confidence":0.9}]}'
    empties

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
for k in ('proposals_after_verify','proposals_refuted','proposals_after_roi','structural_fixes'):
    assert k in d, ('missing counter', k, d)
# Issue C leaves a clean seam for pattern synthesis (Issue D): zero for now.
assert d['structural_fixes'] == 0, d
"
}
