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
            create) echo "https://github.com/x/y/issues/$RANDOM" ;;
            list)   echo '[]' ;;
        esac ;;
esac
exit 0
GHEOF
    chmod +x "$TMP/bin/gh"

    # Fake drain: records call if invoked (must NOT be called by --once)
    DRAIN_LOG="$TMP/.autospec/drain-calls.log"
    export DRAIN_LOG
    export AUTOSPEC_EXPLORE_DRAIN_CMD="echo drain-invoked >> \$DRAIN_LOG"

    # Fake sandbox: records call (must NOT be called by --once)
    SANDBOX_LOG="$TMP/.autospec/sandbox-calls.log"
    export SANDBOX_LOG

    export PATH="$TMP/bin:$PATH"
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

@test "--once emits valid JSON with all 6 required keys" {
    local proposals='[{"title":"feat: widget","evidence":"e","estimated_complexity":"small","confidence":0.9,"source":"spec-vs-code","severity":"feature","named_consumer":"test"}]'
    local cycle_cmd
    cycle_cmd="$(make_cycle_cmd 1 "$proposals")"

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
