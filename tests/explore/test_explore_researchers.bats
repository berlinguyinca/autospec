#!/usr/bin/env bats
# tests/explore/test_explore_researchers.bats — fixtures for each of the 4
# deterministic explore researchers. Each test asserts well-formed JSON
# matching the contract in docs/specs/2026-05-29-autospec-explore-design.md
# (Research cycle contract).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t explore-research.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
}

teardown() {
    rm -rf "$TMP"
}

assert_well_formed() {
    local json="$1"
    printf '%s' "$json" | python3 -c '
import json,sys
d = json.load(sys.stdin)
assert isinstance(d, dict), "top-level must be object"
assert "source" in d, "missing source"
assert "proposals" in d, "missing proposals"
assert isinstance(d["proposals"], list), "proposals must be list"
for p in d["proposals"]:
    for k in ("title","evidence","estimated_complexity","confidence"):
        assert k in p, f"proposal missing {k}"
    assert p["estimated_complexity"] in ("small","medium","large"), "bad complexity"
    assert 0.0 <= float(p["confidence"]) <= 1.0, "bad confidence"
'
}

@test "spec-vs-code emits well-formed JSON from a spec with unmatched AC" {
    mkdir -p docs/specs
    cat > docs/specs/example.md <<'EOF'
# Example spec
## Acceptance criteria
- [ ] Implement the zzzunique-xyzqqq-marker widget for users
- [x] Already done item
EOF
    git add -A && git commit -q -m init
    run bash "$REPO_ROOT/scripts/explore-research/spec-vs-code.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"spec-vs-code"* ]]
    [[ "$output" == *"zzzunique-xyzqqq-marker"* ]]
}

@test "prior-reports emits well-formed JSON harvesting Next steps section" {
    mkdir -p .autospec
    cat > .autospec/run-summary.md <<'EOF'
# Run summary
Result: PASS

## Next steps
- fix the broken loader handler in service xyz
- add coverage for the streaming endpoint module
EOF
    git add -A && git commit -q -m init
    run bash "$REPO_ROOT/scripts/explore-research/prior-reports.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"prior-reports"* ]]
    [[ "$output" == *"broken loader handler"* || "$output" == *"streaming endpoint"* ]]
}

@test "codebase-signals emits well-formed JSON from TODO comments" {
    mkdir -p src
    cat > src/foo.py <<'EOF'
def foo():
    # TODO: implement properly
    return 1
EOF
    git add -A && git commit -q -m init
    run bash "$REPO_ROOT/scripts/explore-research/codebase-signals.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"codebase-signals"* ]]
    [[ "$output" == *"TODO"* ]]
}

@test "open-issues emits well-formed JSON from injected fake gh output" {
    cat > issues.json <<'EOF'
[
  {"number":1,"title":"Fix login redirect","url":"https://example.com/1","labels":[]},
  {"number":2,"title":"v2 flow refactor","url":"https://example.com/2","labels":[{"name":"autospec:v2-flow"}]}
]
EOF
    export AUTOSPEC_TEST_ISSUES_JSON="$TMP/issues.json"
    run bash "$REPO_ROOT/scripts/explore-research/open-issues.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"open-issues"* ]]
    [[ "$output" == *"Fix login redirect"* ]]
    # v2-flow labeled issue must be excluded
    [[ "$output" != *"v2 flow refactor"* ]]
}
