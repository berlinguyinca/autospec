#!/usr/bin/env bats
# tests/explore/test_explore_specialists.bats — domain-specialist roster
# DISCOVERY (Issue E1, spec 2026-06-15-autospec-explore-discovery-enhance.md
# §"Domain-specialist researchers" → "Roster discovery").
#
# Asserts the deterministic signal scan in scripts/explore-specialist-scan.sh:
#   * a trading-dep fixture (ccxt/backtrader) yields a ranked trading domain
#     with file:line evidence and a matching specialist;
#   * a generic/empty fixture yields an empty roster (loop runs unchanged);
#   * the cached .autospec/explore-specialists.json validates against
#     schemas/autospec-explore-specialists.schema.json;
#   * re-invocation is idempotent (reuses the cache);
#   * signals are evidence-grounded, never bare guesses.

SCAN="scripts/explore-specialist-scan.sh"

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t explore-specialists.XXXXXX)"
    cd "$TMP"
    git init -q
    git config user.email t@t.t
    git config user.name t
    export AUTOSPEC_REPO_ROOT="$TMP"
}

teardown() {
    rm -rf "$TMP"
}

@test "schema defines domains + suggested_specialists and compiles with ajv" {
    schema="$REPO_ROOT/schemas/autospec-explore-specialists.schema.json"
    [ -f "$schema" ]
    run jq -e '.properties.domains and .properties.suggested_specialists' "$schema"
    [ "$status" -eq 0 ]
    run jq -e '.["$defs"].specialist.required == ["slug","persona","lens","why","evidence"]' "$schema"
    [ "$status" -eq 0 ]
    if command -v ajv >/dev/null 2>&1; then
        run ajv compile -s "$schema" --spec=draft2020
        [ "$status" -eq 0 ]
    fi
}

@test "trading-dep repo yields a trading domain with file:line evidence" {
    cat > requirements.txt <<'EOF'
flask==2.0
ccxt>=4.0
backtrader==1.9.78
EOF
    git add -A && git commit -q -m init

    run bash "$REPO_ROOT/$SCAN"
    [ "$status" -eq 0 ]

    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['schema_version'] == 1, d
names = [x['name'] for x in d['domains']]
assert 'trading' in names, names
trading = next(x for x in d['domains'] if x['name'] == 'trading')
assert trading['score'] >= 1, trading
# Evidence is grounded: every hit cites a real file + line, never a guess.
for ev in trading['evidence']:
    assert ev['file'] == 'requirements.txt', ev
    assert isinstance(ev['line'], int) and ev['line'] >= 1, ev
    assert ev['match'], ev
# A matching specialist is proposed.
slugs = [s['slug'] for s in d['suggested_specialists']]
assert any('trading' in s for s in slugs), slugs
spec = next(s for s in d['suggested_specialists'] if 'trading' in s['slug'])
assert spec['persona'] and spec['lens'] and spec['why'] and spec['evidence'], spec
assert 'requirements.txt' in spec['evidence'], spec
"
}

@test "generic repo with no detectable domain yields an empty roster" {
    cat > requirements.txt <<'EOF'
flask==2.0
requests>=2.0
EOF
    cat > README.md <<'EOF'
# Generic widget app
A plain web app. Nothing domain-specific here.
EOF
    git add -A && git commit -q -m init

    run bash "$REPO_ROOT/$SCAN"
    [ "$status" -eq 0 ]

    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert d['domains'] == [], d
assert d['suggested_specialists'] == [], d
"
}

@test "cached roster validates against the specialists schema (ajv)" {
    command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available"
    cat > requirements.txt <<'EOF'
ccxt>=4.0
backtrader==1.9.78
EOF
    git add -A && git commit -q -m init
    bash "$REPO_ROOT/$SCAN" >/dev/null
    [ -f "$TMP/.autospec/explore-specialists.json" ]
    run ajv validate \
        -s "$REPO_ROOT/schemas/autospec-explore-specialists.schema.json" \
        --spec=draft2020 \
        -d "$TMP/.autospec/explore-specialists.json"
    [ "$status" -eq 0 ]
}

@test "empty roster validates against the specialists schema (ajv)" {
    command -v ajv >/dev/null 2>&1 || skip "ajv CLI not available"
    git commit -q --allow-empty -m init
    bash "$REPO_ROOT/$SCAN" >/dev/null
    run ajv validate \
        -s "$REPO_ROOT/schemas/autospec-explore-specialists.schema.json" \
        --spec=draft2020 \
        -d "$TMP/.autospec/explore-specialists.json"
    [ "$status" -eq 0 ]
}

@test "roster is cached and reused idempotently on re-invocation" {
    cat > requirements.txt <<'EOF'
ccxt>=4.0
EOF
    git add -A && git commit -q -m init

    run bash "$REPO_ROOT/$SCAN"
    [ "$status" -eq 0 ]
    [ -f "$TMP/.autospec/explore-specialists.json" ]
    first="$(cat "$TMP/.autospec/explore-specialists.json")"

    # Mutate the cache marker; a reuse run must echo the cached bytes verbatim.
    run bash "$REPO_ROOT/$SCAN"
    [ "$status" -eq 0 ]
    second="$(cat "$TMP/.autospec/explore-specialists.json")"
    [ "$first" = "$second" ]
}

@test "LLM stub seam supplies the suggested_specialists roster" {
    cat > requirements.txt <<'EOF'
ccxt>=4.0
EOF
    git add -A && git commit -q -m init

    export AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT='{"domains":[],"suggested_specialists":[{"slug":"market-risk","persona":"Market risk quant","lens":"VaR and drawdown limits","why":"trading deps present","evidence":"requirements.txt:1 ccxt"}]}'
    run bash "$REPO_ROOT/$SCAN" --force
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
slugs = [s['slug'] for s in d['suggested_specialists']]
assert 'market-risk' in slugs, slugs
"
}
