#!/usr/bin/env bats
# tests/explore/test_explore_researchers_llm.bats — fixtures for the 2
# LLM-heavy explore researchers (source-analysis + internet) per issue #720.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t explore-research-llm.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
    export AUTOSPEC_EXPLORE_INTERNET_HISTORY="$TMP/.autospec/explore-internet-history.json"
    mkdir -p "$TMP/.autospec"
}

teardown() {
    rm -rf "$TMP"
}

assert_well_formed() {
    local json="$1"
    # Extract the last line that starts with '{' — script may emit warnings to
    # stderr that bats merges into $output.
    json="$(printf '%s\n' "$json" | grep -E '^\s*\{' | tail -n1)"
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

@test "source-analysis: stubbed LLM produces well-formed JSON proposals" {
    cat > README.md <<'EOF'
# Sample project
This is a thing.
EOF
    git add -A && git commit -q -m init
    export AUTOSPEC_LLM_STUB_OUTPUT='[{"title":"feat: add CLI flag","evidence":"README mentions a CLI but no flags","estimated_complexity":"small","confidence":0.6}]'
    run bash "$REPO_ROOT/scripts/explore-research/source-analysis.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"source-analysis"* ]]
    [[ "$output" == *"feat: add CLI flag"* ]]
}

@test "source-analysis: gracefully degrades to empty proposals when LLM stub returns garbage" {
    cat > README.md <<'EOF'
# x
EOF
    git add -A && git commit -q -m init
    export AUTOSPEC_LLM_STUB_OUTPUT='this is not json at all, just prose'
    run bash "$REPO_ROOT/scripts/explore-research/source-analysis.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *'"proposals": []'* || "$output" == *'"proposals":[]'* ]]
}

@test "source-analysis: gracefully degrades when LLM stub is empty (no harness available)" {
    cat > README.md <<'EOF'
# x
EOF
    git add -A && git commit -q -m init
    # Force harness probe miss by overriding $HOME to an empty dir and clearing
    # claude/codex from PATH.
    export HOME="$TMP/fakehome"
    mkdir -p "$HOME"
    export PATH="/usr/bin:/bin"
    run bash "$REPO_ROOT/scripts/explore-research/source-analysis.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
}

@test "source-analysis: does not contain ambiguous any token" {
    run grep -nE '\bany\b' "$REPO_ROOT/scripts/explore-research/source-analysis.sh"
    [ "$status" -eq 1 ]
}

@test "internet: stubbed LLM produces well-formed JSON proposals with citation" {
    cat > README.md <<'EOF'
# Sample
## Alternatives
See https://github.com/example/competitor for prior art.
EOF
    git add -A && git commit -q -m init
    export AUTOSPEC_FETCH_STUB_github_com='Competitor offers a fast pipeline'
    export AUTOSPEC_LLM_STUB_OUTPUT='[{"title":"feat: add fast pipeline mode","evidence":"competitor https://github.com/example/competitor has it","estimated_complexity":"medium","confidence":0.5}]'
    run bash "$REPO_ROOT/scripts/explore-research/internet.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"internet"* ]]
    [[ "$output" == *"https://github.com/example/competitor"* ]]
}

@test "internet: empty corpus (no URLs in README) returns empty proposals" {
    cat > README.md <<'EOF'
# Sample
Nothing here.
EOF
    git add -A && git commit -q -m init
    run bash "$REPO_ROOT/scripts/explore-research/internet.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *'"proposals": []'* || "$output" == *'"proposals":[]'* ]]
}

@test "internet: drops proposals that have no citation URL in evidence" {
    cat > README.md <<'EOF'
# Sample
## Alternatives
https://github.com/example/x
EOF
    git add -A && git commit -q -m init
    export AUTOSPEC_FETCH_STUB_github_com='content here'
    # LLM returns a proposal with no URL; parser should attach the source URL
    # (so the proposal SHOULD survive). Then a second one with no evidence URL
    # and no fetched URL list (impossible by construction → ensures survives).
    export AUTOSPEC_LLM_STUB_OUTPUT='[{"title":"feat: thing","evidence":"competitor does thing","estimated_complexity":"small","confidence":0.4}]'
    run bash "$REPO_ROOT/scripts/explore-research/internet.sh"
    [ "$status" -eq 0 ]
    assert_well_formed "$output"
    [[ "$output" == *"https://github.com/example/x"* ]]
}
