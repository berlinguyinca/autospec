#!/usr/bin/env bats
# tests/sweep/test_sweep_area_dispatch.bats — fixtures for autospec-sweep
# parallel area dispatch (closes #732). Verifies 4 areas dispatch, aggregated
# report covers all 4, and the new dependency-health researcher emits
# well-formed research-cycle JSON.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t sweep-area.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
    # Mirror the sweep dispatcher's expected layout into the temp repo.
    mkdir -p skills/autospec-sweep/areas scripts/explore-research scripts
    for a in spec-vs-code-drift docs-drift code-health dependency-health; do
        echo "# $a" > "skills/autospec-sweep/areas/$a.md"
    done
    # Stub researchers — copy the real ones so reuse is exercised.
    cp "$REPO_ROOT/scripts/explore-research/spec-vs-code.sh" scripts/explore-research/
    cp "$REPO_ROOT/scripts/explore-research/codebase-signals.sh" scripts/explore-research/
    cp "$REPO_ROOT/scripts/explore-research/dependency-health.sh" scripts/explore-research/
    # docs-drift adapter: stub that emits valid research-cycle JSON.
    cat > scripts/dogfood-adapter-doc-drift.sh <<'EOF'
#!/usr/bin/env bash
echo '{"source":"docs-drift","proposals":[]}'
EOF
    chmod +x scripts/dogfood-adapter-doc-drift.sh scripts/explore-research/*.sh
    git add -A && git commit -q -m init
}

teardown() {
    rm -rf "$TMP"
}

@test "sweep area dispatcher emits aggregated report covering all 4 areas" {
    run bash "$REPO_ROOT/scripts/sweep-area-dispatch.sh" --out "$TMP/out.json"
    [ "$status" -eq 0 ]
    [ -f "$TMP/out.json" ]
    python3 - "$TMP/out.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
assert d["schema"] == "autospec-sweep.area-findings.v1"
for a in ("spec-vs-code-drift","docs-drift","code-health","dependency-health"):
    assert a in d["areas"], f"missing area {a}"
assert d["summary"]["area_count"] == 4
PY
}

@test "dependency-health researcher emits well-formed research-cycle JSON" {
    cat > package.json <<'EOF'
{"name":"x","version":"0.0.1","dependencies":{"left-pad":"^1.0.0"}}
EOF
    cat > npm-outdated.json <<'EOF'
{"left-pad":{"current":"1.0.0","latest":"1.3.0","wanted":"1.3.0"}}
EOF
    export AUTOSPEC_TEST_NPM_OUTDATED="$TMP/npm-outdated.json"
    git add -A && git commit -q -m deps
    run bash "$REPO_ROOT/scripts/explore-research/dependency-health.sh"
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c '
import json,sys
d=json.load(sys.stdin)
assert d["source"]=="dependency-health"
assert isinstance(d["proposals"],list)
assert any("left-pad" in p["title"] for p in d["proposals"]), d
for p in d["proposals"]:
    for k in ("title","evidence","estimated_complexity","confidence"):
        assert k in p
    assert p["estimated_complexity"] in ("small","medium","large")
    assert 0.0 <= float(p["confidence"]) <= 1.0
'
}

@test "dispatcher fails fast when an area definition file is missing" {
    rm skills/autospec-sweep/areas/dependency-health.md
    run bash "$REPO_ROOT/scripts/sweep-area-dispatch.sh" --out "$TMP/out.json"
    [ "$status" -ne 0 ]
    [[ "$output" == *"missing area definition"* ]]
}

@test "dependency-health emits zero-proposal JSON when no manifests present" {
    run bash "$REPO_ROOT/scripts/explore-research/dependency-health.sh"
    [ "$status" -eq 0 ]
    [[ "$output" == *"dependency-health"* ]]
    [[ "$output" == *"\"proposals\": []"* || "$output" == *'"proposals":[]'* ]]
}
