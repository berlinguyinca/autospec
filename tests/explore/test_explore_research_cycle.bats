#!/usr/bin/env bats
# tests/explore/test_explore_research_cycle.bats — aggregator behavior:
# parallel run, dedup by normalized title, weighted ranking, recent-title
# filter, cap.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t explore-cycle.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
    # Override gh; aggregator must still succeed without it.
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"
    # Fake gh that exits non-zero so the recent-title fetch silently produces empty.
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
    chmod +x "$TMP/bin/gh"

    # Build a fake research dir with deterministic mini-researchers we control.
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

@test "aggregator produces well-formed top-level JSON with all expected keys" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: alpha widget","evidence":"e1","estimated_complexity":"small","confidence":0.9}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys
output_json = sys.stdin.read()

import json
d = json.loads(output_json)
for k in ('round','proposals_total','proposals_after_dedup','proposals_after_recent_filter','proposals'):
    assert k in d, f'missing key {k}'
assert d['proposals_total'] == 1
assert d['proposals_after_dedup'] == 1
assert len(d['proposals']) == 1
assert d['proposals'][0]['source'] == 'spec-vs-code'
assert 'score' in d['proposals'][0]
"
}

@test "aggregator dedups by normalized title across researchers" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: Add login flow","evidence":"e1","estimated_complexity":"small","confidence":0.9}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[{"title":"chore: add login flow","evidence":"e2","estimated_complexity":"medium","confidence":0.6}]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys
output_json = sys.stdin.read()

import json
d = json.loads(output_json)
assert d['proposals_total'] == 2
assert d['proposals_after_dedup'] == 1
# Higher-scored should win: spec-vs-code (weight 1.0, conf 0.9, small) > prior-reports.
assert d['proposals'][0]['source'] == 'spec-vs-code', d['proposals'][0]
"
}

@test "aggregator ranks by weighted score and caps to --max-issues-per-round" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: A","evidence":"e","estimated_complexity":"small","confidence":0.9},{"title":"feat: B","evidence":"e","estimated_complexity":"large","confidence":0.5}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[{"title":"feat: C","evidence":"e","estimated_complexity":"small","confidence":0.8}]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[{"title":"feat: D","evidence":"e","estimated_complexity":"small","confidence":0.7}]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[{"title":"feat: E","evidence":"e","estimated_complexity":"small","confidence":0.6}]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 2
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys
output_json = sys.stdin.read()

import json
d = json.loads(output_json)
assert len(d['proposals']) == 2
# Highest weighted: A (1.0*0.9/1.0=0.9) and C (0.9*0.8/1.0=0.72) should be top 2.
titles = [p['title'] for p in d['proposals']]
assert d['proposals'][0]['title'] == 'feat: A', titles
assert d['proposals'][1]['title'] == 'feat: C', titles
"
}

@test "aggregator filters proposals matching recent titles" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: alpha","evidence":"e","estimated_complexity":"small","confidence":0.9},{"title":"feat: beta","evidence":"e","estimated_complexity":"small","confidence":0.8}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    # alpha should be filtered out as a "recent" title.
    export AUTOSPEC_TEST_RECENT_TITLES="feat: alpha"
    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys
output_json = sys.stdin.read()

import json
d = json.loads(output_json)
assert d['proposals_total'] == 2
assert d['proposals_after_recent_filter'] == 1
assert d['proposals'][0]['title'] == 'feat: beta'
"
}

@test "constitution gate drops empty-evidence and below-floor-confidence proposals" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: keep me","evidence":"real evidence","estimated_complexity":"small","confidence":0.9},{"title":"feat: no evidence","evidence":"","estimated_complexity":"small","confidence":0.9},{"title":"feat: low conf","evidence":"e","estimated_complexity":"small","confidence":0.1}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert 'proposals_after_constitution' in d, 'missing proposals_after_constitution key'
assert d['proposals_after_dedup'] == 3, d['proposals_after_dedup']
assert d['proposals_after_constitution'] == 1, d['proposals_after_constitution']
titles = [p['title'] for p in d['proposals']]
assert titles == ['feat: keep me'], titles
"
}

@test "constitution D3 drops bare chore:address marker proposals" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: real work","evidence":"spec gap","estimated_complexity":"medium","confidence":0.8}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[{"title":"chore: address TODO in src/x.py:10","evidence":"TODO at src/x.py:10: TODO: fix it","estimated_complexity":"small","confidence":0.55}]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
titles = [p['title'] for p in d['proposals']]
assert d['proposals_after_dedup'] == 2, d['proposals_after_dedup']
assert d['proposals_after_constitution'] == 1, d['proposals_after_constitution']
assert titles == ['feat: real work'], titles
"
}

@test "constitution floor is overridable via AUTOSPEC_EXPLORE_MIN_CONFIDENCE" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: low conf","evidence":"e","estimated_complexity":"small","confidence":0.1}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run env AUTOSPEC_EXPLORE_MIN_CONFIDENCE=0.05 bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['proposals_after_constitution'] == 1, d['proposals_after_constitution']
"
}
