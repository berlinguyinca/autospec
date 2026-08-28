#!/usr/bin/env bats
# tests/explore/test_explore_once.bats — --once single-cycle contract (F1).
#
# Verifies:
#   - --once invokes the research cycle exactly 1 time (mocked subprocess)
#   - emits valid JSON with all 6 keys (tier, proposals_seen, new_candidates,
#     filed, dry, reason)
#   - dry=true when new_candidates==0 after dedup
#   - never enters the perpetual loop (no sandbox branch creation)
#   - never calls the drain command (AUTOSPEC_EXPLORE_DRAIN_CMD not invoked)
#   - tier="competitor" when sources include "internet"; "local" otherwise
#   - filed count increments when gh issue create succeeds

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TMP="$(mktemp -d -t explore-once.XXXXXX)"
    cd "$TMP"
    git init -q -b main
    git config user.email t@t.t
    git config user.name t
    git commit --allow-empty -q -m "seed"

    export AUTOSPEC_REPO_ROOT="$TMP"
    export HOME="$TMP/home"
    mkdir -p "$HOME/.autospec" "$TMP/bin" "$TMP/.autospec"

    # Fake gh: records calls + creates issues
    cat > "$TMP/bin/gh" <<'GHEOF'
#!/usr/bin/env bash
echo "gh $*" >> "$AUTOSPEC_REPO_ROOT/.autospec/gh-calls.log"
case "$1" in
    issue)
        case "$2" in
            create) echo "https://github.com/x/y/issues/42" ;;
            list)   echo '[]' ;;
        esac ;;
esac
exit 0
GHEOF
    chmod +x "$TMP/bin/gh"

cat > "$TMP/bin/autospec" <<'AUTOSPECEOF'
#!/usr/bin/env bash
echo "autospec $*" >> "$AUTOSPEC_REPO_ROOT/.autospec/gh-calls.log"
if [ "${1:-}" = "explore" ] && [ "${2:-}" = "verifier-outcome" ]; then
    tier=""; cycle=""; artifact=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --tier) tier="${2:-}"; shift 2 ;;
            --cycle) cycle="${2:-}"; shift 2 ;;
            --artifact) artifact="${2:-}"; shift 2 ;;
            *) shift ;;
        esac
    done
    printf '{"outcome":"NotRun","reason":"missing_AUTOSPEC_EXPLORE_VERIFY_CMD","tier":"%s","cycle":%s,"artifact_path":"%s","sealed":true,"dry":false,"may_mutate_github":false}\n' "$tier" "${cycle:-0}" "$artifact"
    exit 0
fi
if [ "${1:-}" = "project" ] && [ "${2:-}" = "sync" ]; then
    if [ "${AUTOSPEC_SYNC_FAIL:-}" = "hard" ]; then
        echo 'pre-journal failure' >&2
        exit 9
    fi
    exit 0
fi
if [ "${1:-}" != "queue" ] || [ "${2:-}" != "review-safety" ]; then
    exit 41
fi
printf '%s\n' '{"pass":1,"ambiguous":0,"block":0,"stale":0,"conflicted":0,"skipped":0}'
AUTOSPECEOF
    chmod +x "$TMP/bin/autospec"

    # Fake drain: records call if invoked (must NOT be called by --once)
    DRAIN_LOG="$TMP/.autospec/drain-calls.log"
    export DRAIN_LOG
    export AUTOSPEC_EXPLORE_DRAIN_CMD="echo drain-invoked >> \$DRAIN_LOG"

    # Fake sandbox: records call (must NOT be called by --once)
    SANDBOX_LOG="$TMP/.autospec/sandbox-calls.log"
    export SANDBOX_LOG

    export PATH="$TMP/bin:$PATH"
    export GITHUB_REPOSITORY="x/y"
    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR"
}

teardown() {
    rm -rf "$TMP"
}

# ── helper: make a mock cycle command that writes research JSON to $AUTOSPEC_EXPLORE_ONCE_OUT ──
make_cycle_cmd() {
    local proposals_total="$1"
    local proposals_json="$2"   # JSON array string (may be empty '[]')
    local count_file="$TMP/.autospec/cycle-count"

    # Write the cycle mock script to a real temp file (bash 3.2 compat)
    local mock="$TMP/bin/mock-cycle"
    cat > "$mock" <<MEOF
#!/usr/bin/env bash
# Record invocation
echo "cycle-called" >> "$count_file"
# Write research JSON to the output path provided via env
OUT="\${AUTOSPEC_EXPLORE_ONCE_OUT:-}"
[ -n "\$OUT" ] || { echo "mock-cycle: AUTOSPEC_EXPLORE_ONCE_OUT not set" >&2; exit 1; }
cat > "\$OUT" <<'JEOF'
{"round":"2026-01-01","proposals_total":PTOTAL,"proposals_after_dedup":PNEW,"verify_mode":"active","proposals_after_verify":PNEW,"proposals_refuted":0,"proposals_after_roi":PNEW,"structural_fixes":0,"proposals_after_recent_filter":PNEW,"proposals":PARR}
JEOF
exit 0
MEOF
    # Substitute placeholders
    sed -i.bak \
        -e "s/PTOTAL/$proposals_total/g" \
        -e "s/PNEW/$(printf '%s' "$proposals_json" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo 0)/g" \
        -e "s/PARR/$(printf '%s' "$proposals_json" | sed 's/\//\\\//g')/g" \
        "$mock"
    rm -f "$mock.bak"
    chmod +x "$mock"
    echo "bash $mock"
}

@test "--once reports verifier failure as incomplete discovery" {
    cat > "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' '{"source":"spec-vs-code","proposals":[]}'
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh"
    export AUTOSPEC_EXPLORE_VERIFY_CMD=false

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        --once --research-sources spec-vs-code

    [ "$status" -ne 0 ]
    [[ "$output" == *'"dry":false'* ]]
    [[ "$output" == *'"reason":"research-incomplete"'* ]]
}

@test "--once emits valid JSON with all 6 required keys" {
    local proposals='[{"title":"feat: widget","evidence":"e","estimated_complexity":"small","confidence":0.9,"source":"spec-vs-code","severity":"feature","named_consumer":"test"}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 1 "$proposals")"
    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources spec-vs-code \
        2>/dev/null
    [ "$status" -eq 0 ]

    # Validate JSON has all 6 keys
    printf '%s\n' "$output" | python3 -c "
import sys, json
lines = [l for l in sys.stdin.read().splitlines() if l.strip().startswith('{')]
assert lines, 'no JSON line in output'
d = json.loads(lines[-1])
for k in ('tier','proposals_seen','new_candidates','filed','dry','reason'):
    assert k in d, f'missing key {k}: {d}'
" 2>&1
}

@test "--once with AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD calls cycle exactly 1 time" {
    local proposals='[{"title":"feat: alpha","evidence":"e","estimated_complexity":"small","confidence":0.9,"source":"spec-vs-code","severity":"feature","named_consumer":"test"}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 1 "$proposals")"
    local count_file="$TMP/.autospec/cycle-count"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources spec-vs-code \
        2>/dev/null
    [ "$status" -eq 0 ]

    # Cycle was called exactly once
    local call_count
    call_count="$(wc -l < "$count_file" 2>/dev/null || echo 0)"
    [ "$call_count" -eq 1 ]
}

@test "--once sets dry=true when new_candidates==0" {
    local proposals='[]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 0 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources spec-vs-code \
        2>/dev/null
    [ "$status" -eq 0 ]

    printf '%s\n' "$output" | python3 -c "
import sys, json
lines = [l for l in sys.stdin.read().splitlines() if l.strip().startswith('{')]
assert lines, 'no JSON line in output'
d = json.loads(lines[-1])
assert d['dry'] is True, f'expected dry=true, got: {d}'
assert d['new_candidates'] == 0, f'expected 0 new_candidates, got: {d}'
"
}

@test "--once sets dry=false when new_candidates>0" {
    local proposals='[{"title":"feat: beta","evidence":"e","estimated_complexity":"small","confidence":0.8,"source":"spec-vs-code","severity":"feature","named_consumer":"test"}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 1 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources spec-vs-code \
        2>/dev/null
    [ "$status" -eq 0 ]

    printf '%s\n' "$output" | python3 -c "
import sys, json
lines = [l for l in sys.stdin.read().splitlines() if l.strip().startswith('{')]
assert lines, 'no JSON line in output'
d = json.loads(lines[-1])
assert d['dry'] is False, f'expected dry=false, got: {d}'
assert d['new_candidates'] == 1, f'expected 1 new_candidate, got: {d}'
"
}

@test "--once tier=competitor when internet is in sources" {
    local proposals='[]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 0 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources "spec-vs-code,internet" \
        2>/dev/null
    [ "$status" -eq 0 ]

    printf '%s\n' "$output" | python3 -c "
import sys, json
lines = [l for l in sys.stdin.read().splitlines() if l.strip().startswith('{')]
assert lines, 'no JSON line in output'
d = json.loads(lines[-1])
assert d['tier'] == 'competitor', f'expected tier=competitor, got: {d}'
"
}

@test "--once tier=local when internet is not in sources" {
    local proposals='[]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 0 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources "spec-vs-code,codebase-signals" \
        2>/dev/null
    [ "$status" -eq 0 ]

    printf '%s\n' "$output" | python3 -c "
import sys, json
lines = [l for l in sys.stdin.read().splitlines() if l.strip().startswith('{')]
assert lines, 'no JSON line in output'
d = json.loads(lines[-1])
assert d['tier'] == 'local', f'expected tier=local, got: {d}'
"
}

@test "--once never calls drain command" {
    local proposals='[]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 0 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"
    local drain_log="$TMP/.autospec/drain-calls.log"
    rm -f "$drain_log"
    export AUTOSPEC_EXPLORE_DRAIN_CMD="echo drain-invoked >> $drain_log"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources spec-vs-code \
        2>/dev/null
    [ "$status" -eq 0 ]

    # Drain must NOT have been called
    if [ -f "$drain_log" ]; then
        drain_content="$(cat "$drain_log")"
        [ -z "$drain_content" ]
    fi
}

@test "--once never creates a sandbox branch" {
    local proposals='[]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 0 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources spec-vs-code \
        2>/dev/null
    [ "$status" -eq 0 ]

    # No explore-mode.json should be created
    if [ -f "$TMP/.autospec/explore-mode.json" ]; then
        false  # sandbox was unexpectedly created
    fi
}

@test "--once filed count matches gh issue create successes" {
    local proposals='[{"title":"feat: one","evidence":"e","estimated_complexity":"small","confidence":0.9,"source":"spec-vs-code","severity":"feature","named_consumer":"test"},{"title":"feat: two","evidence":"e","estimated_complexity":"small","confidence":0.8,"source":"spec-vs-code","severity":"feature","named_consumer":"test"}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 2 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources spec-vs-code \
        2>/dev/null
    [ "$status" -eq 0 ]

    printf '%s\n' "$output" | python3 -c "
import sys, json
lines = [l for l in sys.stdin.read().splitlines() if l.strip().startswith('{')]
assert lines, 'no JSON line in output'
d = json.loads(lines[-1])
assert d['new_candidates'] == 2, f'expected 2 new_candidates, got: {d}'
assert d['filed'] == 2, f'expected filed=2, got: {d}'
assert d['dry'] is False, f'expected dry=false, got: {d}'
"
}

@test "--once propagates a hard pre-journal Project sync failure" {
    local proposals='[{"title":"feat: one","evidence":"e","estimated_complexity":"small","confidence":0.9,"source":"spec-vs-code","severity":"feature","named_consumer":"test"}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 1 "$proposals")"
    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run env AUTOSPEC_SYNC_FAIL=hard bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once --research-sources spec-vs-code

    [ "$status" -ne 0 ]
    [[ "$output" == *"hard managed Project sync failure"* ]]
}

@test "--once propagates a missing Project sync helper" {
    local proposals='[{"title":"feat: one","evidence":"e","estimated_complexity":"small","confidence":0.9,"source":"spec-vs-code","severity":"feature","named_consumer":"test"}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 1 "$proposals")"
    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"
    mkdir -p "$TMP/missing-scripts"

    run env AUTOSPEC_SCRIPTS_DIR="$TMP/missing-scripts" bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once --research-sources spec-vs-code

    [ "$status" -ne 0 ]
    [[ "$output" == *"sync helper is unavailable"* ]]
}


@test "--once emits machine-readable candidates with evidence, labels, ROI score, and body" {
    local proposals='[{"title":"fix: verified gap","evidence":"scripts/x.sh:12 missing guard","estimated_complexity":"medium","confidence":0.8,"source":"source-analysis","severity":"correctness","named_consumer":"autospec-run","score":0.42}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 1 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources source-analysis \
        2>/dev/null
    [ "$status" -eq 0 ]

    printf '%s\n' "$output" | python3 -c "
import sys, json
lines = [l for l in sys.stdin.read().splitlines() if l.strip().startswith('{')]
assert lines, 'no JSON line in output'
d = json.loads(lines[-1])
c = d.get('candidates')
assert isinstance(c, list) and len(c) == 1, d
cand = c[0]
for k in ('title','body','severity','labels','roi_score','evidence'):
    assert k in cand, (k, cand)
assert cand['title'] == 'fix: verified gap', cand
assert cand['severity'] == 'correctness', cand
assert cand['evidence'] == 'scripts/x.sh:12 missing guard', cand
assert abs(cand['roi_score'] - 0.42) < 0.0001, cand
assert 'auto-implement' in cand['labels'], cand
assert 'ctx:64k' in cand['labels'], cand
assert 'reasoning:deep' in cand['labels'], cand
assert 'scripts/x.sh:12 missing guard' in cand['body'], cand
assert 'Adversarial verify' in cand['body'], cand
"
}

@test "--once syncs an interim candidate before exact Rust safety review" {
    local proposals='[{"title":"fix: label body gap","evidence":"lib/y.sh:7 failing path","estimated_complexity":"small","confidence":0.7,"source":"spec-vs-code","severity":"feature","named_consumer":"autospec-run","score":0.7}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 1 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once \
        --research-sources spec-vs-code \
        2>/dev/null
    [ "$status" -eq 0 ]

    grep -q -- '--label auto-implement' "$TMP/.autospec/gh-calls.log"
    grep -q 'autospec project sync --repo-dir' "$TMP/.autospec/gh-calls.log"
    grep -q 'autospec queue review-safety --repo x/y --limit 1 --issue 42' "$TMP/.autospec/gh-calls.log"
    create_line="$(grep -n 'gh issue create' "$TMP/.autospec/gh-calls.log" | head -1 | cut -d: -f1)"
    sync_line="$(grep -n 'autospec project sync' "$TMP/.autospec/gh-calls.log" | head -1 | cut -d: -f1)"
    review_line="$(grep -n 'autospec queue review-safety' "$TMP/.autospec/gh-calls.log" | head -1 | cut -d: -f1)"
    [ "$create_line" -lt "$sync_line" ]
    [ "$sync_line" -lt "$review_line" ]
    grep -q -- '--label ctx:32k' "$TMP/.autospec/gh-calls.log"
    grep -q -- '--label reasoning:medium' "$TMP/.autospec/gh-calls.log"
    grep -q 'lib/y.sh:7 failing path' "$TMP/.autospec/gh-calls.log"
    grep -q 'Adversarial verify: passed' "$TMP/.autospec/gh-calls.log"
    ! grep -q -- '--label safety:reviewed' "$TMP/.autospec/gh-calls.log"
    ! grep -q 'autospec-safety:begin' "$TMP/.autospec/gh-calls.log"
}


@test "--once fails closed (autonomous, no skeptic): files 0 and reports verify-unavailable-failclosed" {
    # Run the REAL cycle (no AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD mock) with a fixture
    # researcher. --once forces autonomous; with no verdict map the cycle fails
    # closed -> 0 filed, and --once must surface it distinctly from a dry well.
    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR"
    cat > "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"source":"spec-vs-code","proposals":[{"title":"feat: would ship if verified","evidence":"ev","estimated_complexity":"small","confidence":0.9}]}
JSON
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "test prompt" \
        --once --research-sources spec-vs-code
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | python3 -c "
import sys, json
lines = [l for l in sys.stdin.read().splitlines() if l.strip().startswith('{')]
assert lines, 'no JSON line'
d = json.loads(lines[-1])
assert d['filed'] == 0, d
assert d['dry'] is True, d
assert d['reason'] == 'verify-unavailable-failclosed', d
assert d['verifier_outcome']['outcome'] == 'NotRun', d
assert d['verifier_outcome']['reason'] == 'missing_AUTOSPEC_EXPLORE_VERIFY_CMD', d
assert d['verifier_outcome']['tier'] == 'local', d
assert d['verifier_outcome']['cycle'] == 1, d
assert d['verifier_outcome']['artifact_path'].endswith('/research.json'), d
assert d['verifier_outcome']['sealed'] is True, d
assert d['verifier_outcome']['dry'] is False, d
assert d['verifier_outcome']['may_mutate_github'] is False, d
"
    # No issue was created (fail-closed filed nothing).
    ! grep -q 'issue create' "$TMP/.autospec/gh-calls.log" 2>/dev/null
}

@test "--once with no initial prompt does not exit 2 (issue #1625)" {
    # The conductor's Tier 2/4 discovery block invokes '--once' with no
    # positional prompt. --once must default to a generic discovery seed
    # instead of hard-failing with the usage error (exit 2).
    local proposals='[]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 0 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" \
        --once \
        --research-sources spec-vs-code \
        2>&1
    [ "$status" -eq 0 ]
    [[ "$output" != *"missing initial prompt"* ]]

    printf '%s\n' "$output" | python3 -c "
import sys, json
lines = [l for l in sys.stdin.read().splitlines() if l.strip().startswith('{')]
assert lines, f'no JSON line in output: {sys.stdin}'
d = json.loads(lines[-1])
assert d['dry'] is True, f'expected dry=true, got: {d}'
"
}

@test "--once with initial prompt still works unchanged (no regression)" {
    local proposals='[{"title":"feat: gamma","evidence":"e","estimated_complexity":"small","confidence":0.8,"source":"spec-vs-code","severity":"feature","named_consumer":"test"}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 1 "$proposals")"

    export AUTOSPEC_EXPLORE_ONCE_CYCLE_CMD="$cycle_cmd"

    run bash "$REPO_ROOT/scripts/autospec-explore.sh" "explicit prompt" \
        --once \
        --research-sources spec-vs-code \
        2>/dev/null
    [ "$status" -eq 0 ]
}

@test "missing initial prompt without --once still exits 2 (unchanged contract)" {
    run bash "$REPO_ROOT/scripts/autospec-explore.sh" --research-sources spec-vs-code
    [ "$status" -eq 2 ]
    [[ "$output" == *"missing initial prompt"* ]]
}
