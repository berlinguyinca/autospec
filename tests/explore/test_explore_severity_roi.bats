#!/usr/bin/env bats
# tests/explore/test_explore_severity_roi.bats — proposal contract extension
# (Issue A): the real aggregator must default a legacy proposal that lacks
# `severity` and `named_consumer` to `feature` / "" without error, and must NOT
# silently drop legacy researchers that omit the fields. The proposal schema
# must define both new fields. No mocks of the aggregator itself.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t explore-severity.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
    # Override gh; aggregator must still succeed without it.
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
    chmod +x "$TMP/bin/gh"

    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR"
    for source in spec-vs-code prior-reports codebase-signals open-issues \
        source-analysis dependency-health internet quality-resilience dogfooding \
        self-leverage style-normalization; do
        make_fake_researcher "$source" "{\"source\":\"$source\",\"proposals\":[]}"
    done
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

@test "proposal schema defines severity enum and named_consumer string" {
    schema="$REPO_ROOT/schemas/autospec-explore-proposal.schema.json"
    [ -f "$schema" ]
    run jq -e '.properties.severity.enum and (.properties.named_consumer.type == "string")' "$schema"
    [ "$status" -eq 0 ]
    # Severity enum order is load-bearing: silent-wrong is the highest band.
    run jq -e '.properties.severity.enum == ["silent-wrong","correctness","stability","operability","feature","nicety"]' "$schema"
    [ "$status" -eq 0 ]
}

@test "aggregator defaults a legacy proposal lacking severity/named_consumer" {
    # Legacy researcher emits the base contract only — no severity, no consumer.
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: alpha widget","evidence":"e1","estimated_complexity":"small","confidence":0.9}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
# Legacy researcher is NOT silently muted.
assert len(d['proposals']) == 1, d
p = d['proposals'][0]
assert p['source'] == 'spec-vs-code'
assert p['severity'] == 'feature', p
assert p['named_consumer'] == '', p
"
}

@test "aggregator preserves explicitly-emitted severity/named_consumer" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: silent corruption guard","evidence":"e1","estimated_complexity":"small","confidence":0.9,"severity":"silent-wrong","named_consumer":"autospec-run Phase 4"}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
p = d['proposals'][0]
assert p['severity'] == 'silent-wrong', p
assert p['named_consumer'] == 'autospec-run Phase 4', p
"
}

# --- Issue C: severity-first ranking -------------------------------------

@test "ranking sorts by severity rank first, score second" {
    # A high-severity, LOW-score proposal must out-rank a low-severity,
    # HIGH-score one. Without severity-first the large-complexity silent-wrong
    # item would sink below the small-complexity nicety item.
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[
      {"title":"feat: tiny polish","evidence":"e1","estimated_complexity":"small","confidence":0.99,"severity":"nicety","named_consumer":"x"},
      {"title":"feat: data corruption guard","evidence":"e2","estimated_complexity":"large","confidence":0.5,"severity":"silent-wrong","named_consumer":"y"}
    ]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
titles = [p['title'] for p in d['proposals']]
assert titles[0] == 'feat: data corruption guard', titles
assert titles[1] == 'feat: tiny polish', titles
"
}

@test "within one severity band, score still breaks the tie" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[
      {"title":"feat: low score feature","evidence":"e1","estimated_complexity":"large","confidence":0.4,"severity":"feature","named_consumer":"x"},
      {"title":"feat: high score feature","evidence":"e2","estimated_complexity":"small","confidence":0.95,"severity":"feature","named_consumer":"y"}
    ]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
titles = [p['title'] for p in d['proposals']]
assert titles[0] == 'feat: high score feature', titles
"
}

# --- Issue C: ROI gate ---------------------------------------------------

@test "ROI gate drops a new-source proposal with empty named_consumer; keeps a legacy one" {
    # A NEW researcher (quality-resilience) with empty named_consumer is
    # dropped by the ROI gate. A LEGACY researcher (spec-vs-code) with empty
    # named_consumer is exempt and survives.
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: legacy keeps","evidence":"e1","estimated_complexity":"small","confidence":0.9}]}'
    make_fake_researcher quality-resilience '{"source":"quality-resilience","proposals":[
      {"title":"feat: new no consumer","evidence":"e2","estimated_complexity":"small","confidence":0.9,"severity":"correctness","track":"product"},
      {"title":"feat: new with consumer","evidence":"e3","estimated_complexity":"small","confidence":0.9,"severity":"correctness","named_consumer":"validate.sh","track":"product"}
    ]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" \
        --research-sources spec-vs-code,prior-reports,codebase-signals,open-issues,quality-resilience \
        --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
titles = sorted(p['title'] for p in d['proposals'])
assert 'feat: legacy keeps' in titles, titles          # legacy empty-consumer kept
assert 'feat: new with consumer' in titles, titles      # new + consumer kept
assert 'feat: new no consumer' not in titles, titles     # new empty-consumer dropped
assert d['proposals_after_roi'] == 2, d
"
}

@test "ROI gate exempts ALL seven legacy universal sources" {
    # Each legacy universal source emitting an empty-consumer proposal must
    # survive the ROI gate. Guards against silently muting the existing 7.
    # source-analysis is also a gap-claiming source, so its proposal carries a
    # confirmable gap_check (a present-claim against a real repo file) to pass
    # the gap-confirmation stage that runs before ROI; the other six are not
    # gap-claiming and pass through unchanged.
    echo "roi-probe-marker" > roi_probe.txt
    git add roi_probe.txt && git commit -q -m "roi probe"
    for s in spec-vs-code prior-reports codebase-signals open-issues dependency-health internet; do
        make_fake_researcher "$s" "{\"source\":\"$s\",\"proposals\":[{\"title\":\"feat: $s item\",\"evidence\":\"e\",\"estimated_complexity\":\"small\",\"confidence\":0.9}]}"
    done
    make_fake_researcher source-analysis '{"source":"source-analysis","proposals":[{"title":"feat: source-analysis item","evidence":"e","estimated_complexity":"small","confidence":0.9,"gap_check":{"kind":"present","needle":"roi-probe-marker","haystack":"roi_probe.txt"}}]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" \
        --research-sources spec-vs-code,prior-reports,codebase-signals,open-issues,source-analysis,dependency-health,internet \
        --max-issues-per-round 20
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['proposals_after_roi'] == 7, d
"
}

# --- Issue D: pattern synthesis ------------------------------------------

@test "pattern synthesis collapses a 3-instance class to one structural-fix" {
    # Three survivors share a coarse class key (same severity band + leading
    # subject tokens). They must collapse to ONE structural-fix proposal whose
    # evidence lists all three instances, and structural_fixes must count it.
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[
      {"title":"fix: missing error handling in alpha module","evidence":"alpha lacks try/except","estimated_complexity":"small","confidence":0.9,"severity":"correctness","named_consumer":"x"},
      {"title":"fix: missing error handling in beta module","evidence":"beta lacks try/except","estimated_complexity":"small","confidence":0.9,"severity":"correctness","named_consumer":"x"},
      {"title":"fix: missing error handling in gamma module","evidence":"gamma lacks try/except","estimated_complexity":"small","confidence":0.9,"severity":"correctness","named_consumer":"x"}
    ]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
# The three same-class instances collapse to one structural-fix.
assert d['structural_fixes'] == 1, d
sfix = [p for p in d['proposals'] if p.get('proposal_kind') == 'structural-fix']
assert len(sfix) == 1, d['proposals']
p = sfix[0]
# Evidence lists all three instances.
for tok in ('alpha', 'beta', 'gamma'):
    assert tok in p['evidence'], p['evidence']
# The 3 original members are gone; only the structural-fix remains for them.
titles = [q['title'] for q in d['proposals']]
assert not any('alpha module' in t and t != p['title'] for t in titles), titles
"
}

@test "pattern synthesis cannot disguise raw marker chores from constitution D3" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[
      {"title":"chore: address TODO in src/alpha.py:10","evidence":"TODO at src/alpha.py:10: TODO: implement","estimated_complexity":"small","confidence":0.55},
      {"title":"chore: address TODO in src/beta.py:20","evidence":"TODO at src/beta.py:20: TODO: implement","estimated_complexity":"small","confidence":0.55},
      {"title":"chore: address TODO in src/gamma.py:30","evidence":"TODO at src/gamma.py:30: TODO: implement","estimated_complexity":"small","confidence":0.55}
    ]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['structural_fixes'] == 0, d
assert d['proposals_after_constitution'] == 0, d
assert d['proposals'] == [], d['proposals']
"
}

@test "pattern synthesis does NOT over-collapse unrelated proposals" {
    # Two survivors in different severity bands / unrelated subjects must each
    # stay a distinct proposal — no structural-fix, structural_fixes == 0.
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[
      {"title":"fix: data corruption in writer","evidence":"e1","estimated_complexity":"small","confidence":0.9,"severity":"silent-wrong","named_consumer":"x"},
      {"title":"feat: add dark mode toggle","evidence":"e2","estimated_complexity":"small","confidence":0.9,"severity":"nicety","named_consumer":"y"}
    ]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['structural_fixes'] == 0, d
assert not any(p.get('proposal_kind') == 'structural-fix' for p in d['proposals']), d['proposals']
titles = sorted(p['title'] for p in d['proposals'])
assert 'fix: data corruption in writer' in titles, titles
assert 'feat: add dark mode toggle' in titles, titles
"
}

@test "structural_fixes counter is 0 with no clustering" {
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[{"title":"feat: lone item","evidence":"e","estimated_complexity":"small","confidence":0.9}]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['structural_fixes'] == 0, d
"
}

@test "ROI gate drops empty-consumer specialist:<slug> proposals (new source)" {
    make_fake_researcher 'specialist:market-risk' '{"source":"specialist:market-risk","proposals":[{"title":"feat: risk control gap","evidence":"e","estimated_complexity":"small","confidence":0.9,"severity":"correctness"}]}'
    make_fake_researcher spec-vs-code '{"source":"spec-vs-code","proposals":[]}'
    make_fake_researcher prior-reports '{"source":"prior-reports","proposals":[]}'
    make_fake_researcher codebase-signals '{"source":"codebase-signals","proposals":[]}'
    make_fake_researcher open-issues '{"source":"open-issues","proposals":[]}'

    run bash "$REPO_ROOT/scripts/explore-research-cycle.sh" \
        --research-sources spec-vs-code,prior-reports,codebase-signals,open-issues,specialist:market-risk \
        --max-issues-per-round 5
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['proposals_after_roi'] == 0, d
assert len(d['proposals']) == 0, d
"
}
