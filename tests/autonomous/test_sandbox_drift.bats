#!/usr/bin/env bats
# tests/autonomous/test_sandbox_drift.bats — F8 sandbox drift reporting +
# priority reweight.
#
# Covers:
#   1. Digest drift section includes merge-base distance (commits behind).
#   2. Digest drift section includes conflict-risk estimate.
#   3. Digest never runs git rebase.
#   4. Priority proposal score = base * AUTOSPEC_PRIORITY_BOOST (default 1.5).
#   5. Steer-source proposal gets the same boost.
#   6. Boost is bounded: boosted score <= max_non_boosted * multiplier.
#   7. No explore-mode.json → no drift section (graceful).
#   8. Non-priority proposal score unchanged when boost active.
#   9. Custom AUTOSPEC_PRIORITY_BOOST env var is honoured.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"
RESEARCH_CYCLE="$REPO_ROOT/scripts/explore-research-cycle.sh"

# ── helpers ──────────────────────────────────────────────────────────────────

setup() {
    TMP="$(mktemp -d -t sandbox-drift.XXXXXX)"
    export REBASE_LOG="$TMP/rebase.log"
    touch "$REBASE_LOG"
    # Priority-score assertions cover the static DEFAULT_SRC_WEIGHTS table.
    # explore-research-cycle.sh can otherwise pick up an operator-local
    # installed explore-source-weights.sh and learned ledger weights, making this
    # unit suite depend on workstation history.
    export AUTOSPEC_EXPLORE_WEIGHTS_BIN="$TMP/missing-explore-source-weights.sh"

    # Build a fixture git repo with sandbox branch + commits behind main.
    FIXTURE_REPO="$TMP/repo"
    mkdir -p "$FIXTURE_REPO"
    git -C "$FIXTURE_REPO" init -q
    git -C "$FIXTURE_REPO" config user.email t@t.t
    git -C "$FIXTURE_REPO" config user.name t

    # Initial commit on main.
    echo "v0" > "$FIXTURE_REPO/file_a.txt"
    git -C "$FIXTURE_REPO" add file_a.txt
    git -C "$FIXTURE_REPO" commit -q -m "init"

    # Create sandbox branch at this point.
    git -C "$FIXTURE_REPO" branch autospec/explore/2026-06-25-test-sandbox

    # Two more commits on main (sandbox is now 2 commits behind).
    echo "v1" > "$FIXTURE_REPO/file_b.txt"
    git -C "$FIXTURE_REPO" add file_b.txt
    git -C "$FIXTURE_REPO" commit -q -m "main commit 1"

    echo "v2" > "$FIXTURE_REPO/file_a.txt"
    git -C "$FIXTURE_REPO" add file_a.txt
    git -C "$FIXTURE_REPO" commit -q -m "main commit 2 (touches file_a)"

    # Commit on the sandbox branch (also touches file_a → conflict risk).
    git -C "$FIXTURE_REPO" checkout -q autospec/explore/2026-06-25-test-sandbox
    echo "sandbox-change" >> "$FIXTURE_REPO/file_a.txt"
    git -C "$FIXTURE_REPO" add file_a.txt
    git -C "$FIXTURE_REPO" commit -q -m "sandbox commit"

    # Determine the main branch name (git defaults may vary).
    MAIN_BRANCH="$(git -C "$FIXTURE_REPO" branch --list main master \
        | sed 's/[* ]//g' | grep -v '^$' | head -1)"
    MAIN_BRANCH="${MAIN_BRANCH:-main}"

    git -C "$FIXTURE_REPO" checkout -q "$MAIN_BRANCH" 2>/dev/null || true

    # Simulate origin/* refs as local branches (merge-base needs them).
    git -C "$FIXTURE_REPO" branch "origin/$MAIN_BRANCH" HEAD 2>/dev/null || true

    # Write explore-mode.json.
    mkdir -p "$FIXTURE_REPO/.autospec"
    printf '{\n  "branch": "autospec/explore/2026-06-25-test-sandbox",\n  "slug": "test-sandbox",\n  "base": "%s",\n  "head_sha": "abc123",\n  "created_at": "2026-06-25T00:00:00Z"\n}\n' \
        "$MAIN_BRANCH" > "$FIXTURE_REPO/.autospec/explore-mode.json"
}

teardown() {
    rm -rf "$TMP"
}

_call_drift_section() {
    local repo="$1"
    bash -c "source '$LOOP_LIB'; _conductor_sandbox_drift_section '$repo'"
}

# Build a single-researcher cycle environment and run the aggregator.
# Usage: _run_cycle SRC_NAME PAYLOAD_JSON [extra args for research-cycle]
_run_cycle() {
    local src="$1"
    local payload="$2"
    shift 2

    local research_dir="$TMP/research_$$"
    mkdir -p "$research_dir"

    # Researcher file must match the source name.
    local rfile="$research_dir/$src.sh"
    printf '#!/usr/bin/env bash\ncat <<'"'"'JSON'"'"'\n%s\nJSON\n' "$payload" > "$rfile"
    chmod +x "$rfile"

    local fake_bin="$TMP/fakebin_$$"
    mkdir -p "$fake_bin"
    printf '#!/usr/bin/env bash\nexit 1\n' > "$fake_bin/gh"
    chmod +x "$fake_bin/gh"

    local cycle_repo="$TMP/cycle_repo_$$"
    mkdir -p "$cycle_repo"
    git -C "$cycle_repo" init -q
    git -C "$cycle_repo" config user.email t@t.t
    git -C "$cycle_repo" config user.name t

    PATH="$fake_bin:$PATH" \
    AUTOSPEC_REPO_ROOT="$cycle_repo" \
    AUTOSPEC_RESEARCH_DIR="$research_dir" \
    bash "$RESEARCH_CYCLE" \
        --research-sources "$src" \
        --max-issues-per-round 10 \
        "$@"
}

# ── drift section tests ───────────────────────────────────────────────────────

@test "drift section reports commits behind main" {
    run _call_drift_section "$FIXTURE_REPO"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "Commits behind"
    echo "$output" | grep -qE "\| [0-9]+ \|"
}

@test "drift section reports conflict-risk estimate" {
    run _call_drift_section "$FIXTURE_REPO"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "Conflict-risk"
    # file_a.txt changed in both branches → medium or high.
    echo "$output" | grep -qE "medium|high"
}

@test "drift section names the sandbox branch" {
    run _call_drift_section "$FIXTURE_REPO"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "autospec/explore/2026-06-25-test-sandbox"
}

@test "drift section includes no-rebase notice" {
    run _call_drift_section "$FIXTURE_REPO"
    [ "$status" -eq 0 ]
    echo "$output" | grep -qi "no auto-rebase"
}

@test "digest includes drift section when explore-mode.json present" {
    local sdir="$FIXTURE_REPO/scripts"
    mkdir -p "$sdir"
    bash -c "source '$LOOP_LIB'; _conductor_maybe_write_digest 0 '' '$sdir' '' 0" >/dev/null 2>&1 || true
    local digest_file="$FIXTURE_REPO/.autospec/autonomous-digest.md"
    [ -f "$digest_file" ]
    grep -q "Sandbox drift" "$digest_file"
}

@test "no drift section when explore-mode.json is absent" {
    local bare="$TMP/bare_repo"
    mkdir -p "$bare"
    run _call_drift_section "$bare"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "drift section never calls git rebase" {
    local fake_bin="$TMP/rebase_guard_bin"
    mkdir -p "$fake_bin"
    # Write real temp file (bash 3.2 compat: no process substitution for [ -f ]).
    cat > "$fake_bin/git" <<'FAKEGIT'
#!/usr/bin/env bash
if [ "${1:-}" = "rebase" ] || [ "${2:-}" = "rebase" ]; then
    echo "REBASE_DETECTED" >> "$REBASE_LOG"
    exit 1
fi
exec /usr/bin/git "$@"
FAKEGIT
    chmod +x "$fake_bin/git"
    PATH="$fake_bin:$PATH" run _call_drift_section "$FIXTURE_REPO"
    # REBASE_LOG must be empty (or not contain REBASE_DETECTED).
    if [ -s "$REBASE_LOG" ]; then
        ! grep -q "REBASE_DETECTED" "$REBASE_LOG"
    fi
}

# ── priority boost tests ──────────────────────────────────────────────────────

@test "priority proposal score = base * 1.5 (default boost)" {
    # spec-vs-code weight=1.0 (DEFAULT_SRC_WEIGHTS), small→scale=1.0
    # organic:  conf=0.8 → base=0.8
    # priority: conf=0.4 → base=0.4 → boosted=0.6; cap=0.8*1.5=1.2 → 0.6 < cap ✓
    local payload='{"source":"spec-vs-code","proposals":[
        {"title":"fix: cache invalidation on eviction","evidence":"seen in logs","estimated_complexity":"small","confidence":0.8,"severity":"feature","named_consumer":""},
        {"title":"feat: cache warmup operator steer","evidence":"operator request","estimated_complexity":"small","confidence":0.4,"severity":"feature","named_consumer":"","priority":true}
    ]}'
    run _run_cycle "spec-vs-code" "$payload"
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
props = {p['title']: p for p in d['proposals']}
assert 'feat: cache warmup operator steer' in props, list(props.keys())
boosted = props['feat: cache warmup operator steer']['score']
# base=0.4*1.0/1.0=0.4; boosted=0.4*1.5=0.6
assert abs(boosted - 0.6) < 0.01, f'expected ~0.6, got {boosted}'
"
}

@test "steer-source proposal gets 1.5x boost" {
    # steer is not a legacy source; named_consumer must be non-empty to pass ROI gate.
    # steer weight defaults to 0.5 (unknown source); conf=0.4, small → base=0.2; boosted=0.3
    local payload='{"source":"steer","proposals":[
        {"title":"feat: retry budget enforcement via steer signal","evidence":"operator directive","estimated_complexity":"small","confidence":0.4,"severity":"feature","named_consumer":"autospec-run Phase 4"}
    ]}'
    run _run_cycle "steer" "$payload"
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
props = {p['title']: p for p in d['proposals']}
assert 'feat: retry budget enforcement via steer signal' in props, list(props.keys())
boosted = props['feat: retry budget enforcement via steer signal']['score']
# base = 0.4 * 0.5 / 1.0 = 0.2; boosted = 0.2 * 1.5 = 0.3
assert abs(boosted - 0.3) < 0.01, f'expected ~0.3, got {boosted}'
"
}

@test "boost is bounded: cannot exceed max_non_boosted * multiplier" {
    # organic:  title=telemetry-flush, conf=0.9 → base=0.72; cap=0.72*1.5=1.08
    # priority: title=heartbeat-interval-steer, conf=1.0 → base=0.80; uncapped=1.2 → clamped to 1.08
    local payload='{"source":"spec-vs-code","proposals":[
        {"title":"fix: telemetry flush on exit","evidence":"data loss observed","estimated_complexity":"small","confidence":0.9,"severity":"feature","named_consumer":""},
        {"title":"feat: heartbeat interval steer override","evidence":"operator request","estimated_complexity":"small","confidence":1.0,"severity":"feature","named_consumer":"","priority":true}
    ]}'
    run _run_cycle "spec-vs-code" "$payload"
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
props = {p['title']: p for p in d['proposals']}
assert 'feat: heartbeat interval steer override' in props, list(props.keys())
assert 'fix: telemetry flush on exit' in props, list(props.keys())
cap = props['fix: telemetry flush on exit']['score'] * 1.5
boosted = props['feat: heartbeat interval steer override']['score']
assert boosted <= cap + 0.001, f'boosted={boosted} exceeded cap={cap}'
base = round(1.0 * 0.8 / 1.0, 4)
assert boosted > base, f'boosted={boosted} not above base={base}'
"
}

@test "non-priority proposal score unchanged when boost active" {
    # organic: conf=0.7, weight=1.0, medium→scale=2.0 → base=0.7/2.0=0.35 (unchanged)
    local payload='{"source":"spec-vs-code","proposals":[
        {"title":"fix: disk quota alert threshold","evidence":"quota breach logs","estimated_complexity":"medium","confidence":0.7,"severity":"feature","named_consumer":""},
        {"title":"feat: sandbox promotion steer hook","evidence":"operator request","estimated_complexity":"small","confidence":0.3,"severity":"feature","named_consumer":"","priority":true}
    ]}'
    run _run_cycle "spec-vs-code" "$payload"
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
props = {p['title']: p for p in d['proposals']}
assert 'fix: disk quota alert threshold' in props, list(props.keys())
score = props['fix: disk quota alert threshold']['score']
# spec-vs-code weight=1.0; 0.7*1.0/2.0=0.35
assert abs(score - 0.35) < 0.01, f'expected ~0.35, got {score}'
"
}

@test "custom AUTOSPEC_PRIORITY_BOOST=2.0 is honoured" {
    # spec-vs-code weight=1.0; conf=0.4, small → base=0.4; boosted=0.4*2.0=0.8
    # cap = organic_base(0.8) * 2.0 = 1.6 → not clamped
    local payload='{"source":"spec-vs-code","proposals":[
        {"title":"fix: session token expiry race","evidence":"race in logs","estimated_complexity":"small","confidence":0.8,"severity":"feature","named_consumer":""},
        {"title":"feat: watchdog restart steer directive","evidence":"operator request","estimated_complexity":"small","confidence":0.4,"severity":"feature","named_consumer":"","priority":true}
    ]}'
    # Export before calling _run_cycle (bats run does not support VAR=val prefix for shell fns).
    export AUTOSPEC_PRIORITY_BOOST=2.0
    run _run_cycle "spec-vs-code" "$payload"
    unset AUTOSPEC_PRIORITY_BOOST
    [ "$status" -eq 0 ]
    printf '%s' "$output" | python3 -c "
import sys, json
d = json.loads(sys.stdin.read())
props = {p['title']: p for p in d['proposals']}
assert 'feat: watchdog restart steer directive' in props, list(props.keys())
boosted = props['feat: watchdog restart steer directive']['score']
# base=0.4*1.0/1.0=0.4; boosted=0.4*2.0=0.8; cap=0.8*2.0=1.6 → not clamped
assert abs(boosted - 0.8) < 0.01, f'expected ~0.8, got {boosted}'
"
}
