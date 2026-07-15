#!/usr/bin/env bats
# tests/explore/test_explore_specialists.bats — domain-specialist dispatch
#
# Rust core/CLI tests own deterministic roster discovery. This suite keeps the
# shell explore-research-cycle behavior covered once a Rust-produced
# .autospec/explore-specialists.json roster already exists.

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
