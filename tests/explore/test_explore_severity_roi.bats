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
