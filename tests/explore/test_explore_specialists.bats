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

# ───────────────────────────────────────────────────────────────────────────
# Issue E2 (#1083) — specialist flags + dispatch via scripts/explore-research-
# cycle.sh. The aggregator parses --specialists-mode/--num-specialists/
# --specialists, resolves the roster, and dispatches each as a
# source=specialist:<slug> researcher (default weight 0.6) through the same
# dedup → verify → ROI → synthesis → rank pipeline, under the ≤17 per-round cap.
# ───────────────────────────────────────────────────────────────────────────

CYCLE="scripts/explore-research-cycle.sh"

# Build the deterministic dispatch fixture: a fake research dir with universal
# researchers we control + a fake gh that fails (so recent-title fetch no-ops).
_e2_setup_cycle() {
    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR" "$TMP/bin" "$TMP/.autospec"
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
    chmod +x "$TMP/bin/gh"
    export PATH="$TMP/bin:$PATH"
}

_e2_make_universal() {
    # _e2_make_universal <name> [proposals-json-array]
    local name="$1" props="${2:-[]}"
    cat > "$AUTOSPEC_RESEARCH_DIR/$name.sh" <<EOF
#!/usr/bin/env bash
echo '{"source":"$name","proposals":$props}'
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/$name.sh"
}

@test "E2 --specialists-mode off runs zero specialists (current behavior)" {
    _e2_setup_cycle
    _e2_make_universal spec-vs-code '[{"title":"feat: alpha","evidence":"e","estimated_complexity":"small","confidence":0.9}]'
    # A roster + a proposal seam exist, but off must ignore them entirely.
    cat > "$TMP/.autospec/explore-specialists.json" <<'EOF'
{"schema_version":1,"domains":[],"suggested_specialists":[{"slug":"trading-specialist","persona":"P","lens":"L","why":"W","evidence":"E"}]}
EOF
    export AUTOSPEC_SPECIALIST_PROPOSALS_TRADING_SPECIALIST='[{"title":"feat: should-not-appear","evidence":"e","estimated_complexity":"small","confidence":0.9,"severity":"correctness","named_consumer":"c"}]'

    run bash "$REPO_ROOT/$CYCLE" --specialists-mode off \
        --research-sources spec-vs-code --max-issues-per-round 10
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
srcs = {p['source'] for p in d['proposals']}
assert not any(s.startswith('specialist:') for s in srcs), srcs
assert 'spec-vs-code' in srcs, srcs
"
}

@test "E2 a detected-domain roster dispatches specialist:<slug> through the aggregator" {
    _e2_setup_cycle
    _e2_make_universal spec-vs-code '[]'
    cat > "$TMP/.autospec/explore-specialists.json" <<'EOF'
{"schema_version":1,"domains":[],"suggested_specialists":[{"slug":"market-risk","persona":"Quant","lens":"VaR","why":"trading deps","evidence":"requirements.txt:1"}]}
EOF
    export AUTOSPEC_SPECIALIST_PROPOSALS_MARKET_RISK='[{"title":"feat: VaR drawdown guard","evidence":"order path lacks a stop","estimated_complexity":"small","confidence":0.8,"severity":"correctness","named_consumer":"autospec-run"}]'

    run bash "$REPO_ROOT/$CYCLE" --specialists-mode discover --num-specialists 3 \
        --research-sources spec-vs-code --max-issues-per-round 10
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
sp = [p for p in d['proposals'] if p['source'] == 'specialist:market-risk']
assert len(sp) == 1, d['proposals']
# Flows through verify + ROI like any source, and carries weight-0.6 scoring:
# score = confidence(0.8) * weight(0.6) / complexity(small=1.0) = 0.48.
assert abs(sp[0]['score'] - 0.48) < 1e-6, sp[0]
assert sp[0]['severity'] == 'correctness', sp[0]
"
}

@test "E2 ROI gate drops a specialist proposal with no named_consumer (new-source-gated)" {
    _e2_setup_cycle
    _e2_make_universal spec-vs-code '[]'
    cat > "$TMP/.autospec/explore-specialists.json" <<'EOF'
{"schema_version":1,"domains":[],"suggested_specialists":[{"slug":"market-risk","persona":"Quant","lens":"VaR","why":"deps","evidence":"r:1"}]}
EOF
    # No named_consumer → ROI gate (new-source-only) must drop it.
    export AUTOSPEC_SPECIALIST_PROPOSALS_MARKET_RISK='[{"title":"feat: orphan idea","evidence":"e","estimated_complexity":"small","confidence":0.9,"severity":"feature"}]'

    run bash "$REPO_ROOT/$CYCLE" --specialists-mode discover \
        --research-sources spec-vs-code --max-issues-per-round 10
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
assert not any(p['source'].startswith('specialist:') for p in d['proposals']), d['proposals']
"
}

@test "E2 --specialists-mode explicit uses the verbatim roster" {
    _e2_setup_cycle
    _e2_make_universal spec-vs-code '[]'
    # No cache file at all — explicit must not need it.
    export AUTOSPEC_SPECIALIST_PROPOSALS_PAYMENTS_REVIEWER='[{"title":"feat: idempotent retry key","evidence":"double-charge risk","estimated_complexity":"small","confidence":0.7,"severity":"stability","named_consumer":"autospec-run"}]'

    run bash "$REPO_ROOT/$CYCLE" --specialists-mode explicit \
        --specialists "payments-reviewer:Payments reliability,unused-slug:Persona" \
        --num-specialists 1 --research-sources spec-vs-code --max-issues-per-round 10
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
srcs = [p['source'] for p in d['proposals'] if p['source'].startswith('specialist:')]
assert srcs == ['specialist:payments-reviewer'], srcs
"
}

@test "E2 --num-specialists caps at 6 and selects the top-N" {
    _e2_setup_cycle
    _e2_make_universal spec-vs-code '[]'
    # Roster of 4; ask for top 2.
    cat > "$TMP/.autospec/explore-specialists.json" <<'EOF'
{"schema_version":1,"domains":[],"suggested_specialists":[
{"slug":"sp-a","persona":"P","lens":"L","why":"W","evidence":"E"},
{"slug":"sp-b","persona":"P","lens":"L","why":"W","evidence":"E"},
{"slug":"sp-c","persona":"P","lens":"L","why":"W","evidence":"E"},
{"slug":"sp-d","persona":"P","lens":"L","why":"W","evidence":"E"}]}
EOF
    # Distinct multi-word subjects so pattern-synthesis does not collapse them.
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_A='[{"title":"feat: trading order-execution guard","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_B='[{"title":"feat: payments idempotency key","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_C='[{"title":"feat: healthcare phi redaction","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_D='[{"title":"feat: ml reproducibility seed","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'

    run bash "$REPO_ROOT/$CYCLE" --specialists-mode discover --num-specialists 2 \
        --research-sources spec-vs-code --max-issues-per-round 50
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
sp = sorted(p['source'] for p in d['proposals'] if p['source'].startswith('specialist:'))
assert sp == ['specialist:sp-a','specialist:sp-b'], sp
"
}

@test "E2 the per-round researcher count is clamped to 17" {
    _e2_setup_cycle
    # 14 universal researchers selected.
    local us=""
    for i in $(seq 1 14); do
        _e2_make_universal "u$i" '[]'
        us="$us,u$i"
    done
    us="${us#,}"
    # Roster of 6; ask for 6, but only 17-14=3 slots remain.
    {
        printf '{"schema_version":1,"domains":[],"suggested_specialists":['
        printf '{"slug":"sp-a","persona":"P","lens":"L","why":"W","evidence":"E"},'
        printf '{"slug":"sp-b","persona":"P","lens":"L","why":"W","evidence":"E"},'
        printf '{"slug":"sp-c","persona":"P","lens":"L","why":"W","evidence":"E"},'
        printf '{"slug":"sp-d","persona":"P","lens":"L","why":"W","evidence":"E"},'
        printf '{"slug":"sp-e","persona":"P","lens":"L","why":"W","evidence":"E"},'
        printf '{"slug":"sp-f","persona":"P","lens":"L","why":"W","evidence":"E"}]}\n'
    } > "$TMP/.autospec/explore-specialists.json"
    # Distinct multi-word subjects so pattern-synthesis does not collapse them.
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_A='[{"title":"feat: trading order-execution guard","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_B='[{"title":"feat: payments idempotency key","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_C='[{"title":"feat: healthcare phi redaction","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_D='[{"title":"feat: ml reproducibility seed","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_E='[{"title":"feat: infra rollout blast radius","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'
    export AUTOSPEC_SPECIALIST_PROPOSALS_SP_F='[{"title":"feat: security authz boundary","evidence":"e","estimated_complexity":"small","confidence":0.7,"severity":"feature","named_consumer":"c"}]'

    run bash "$REPO_ROOT/$CYCLE" --specialists-mode discover --num-specialists 6 \
        --research-sources "$us" --max-issues-per-round 50
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
sp = sorted(p['source'] for p in d['proposals'] if p['source'].startswith('specialist:'))
assert len(sp) == 3, sp  # 14 universal + 3 specialists = 17 cap
"
}

@test "E2 invalid --specialists-mode is rejected" {
    _e2_setup_cycle
    _e2_make_universal spec-vs-code '[]'
    run bash "$REPO_ROOT/$CYCLE" --specialists-mode bogus --research-sources spec-vs-code
    [ "$status" -eq 2 ]
}
