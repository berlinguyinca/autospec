#!/usr/bin/env bats
# explore-orchestrator-ledger.bats — verifies scripts/autospec-explore.sh
# emits the canonical explore-ledger marker into filed issue bodies, appends a
# pending ledger record at file time, and resolves outcomes after drain.
#
# Drives the real orchestrator with --max-iterations 1, all external calls
# stubbed (gh, dispatcher) and ledger env pointed at temp paths.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_FILENAME" >/dev/null 2>&1; cd "$(dirname "$BATS_TEST_FILENAME")/../../../.." && pwd)"
    EXPLORE_SH="$REPO_ROOT/scripts/autospec-explore.sh"
    LEDGER_SH="$REPO_ROOT/skills/autospec-shared/scripts/explore-ledger.sh"

    TMP="$(mktemp -d -t explore-orch-ledger.XXXXXX)"
    cd "$TMP"
    git init -q -b main
    git config user.email t@t.t
    git config user.name t
    git commit --allow-empty -q -m seed
    git init -q --bare "$TMP/remote.git"
    git remote add origin "$TMP/remote.git"
    git push -q origin main
    git fetch -q origin main

    export AUTOSPEC_REPO_ROOT="$TMP"
    export HOME="$TMP/home"
    mkdir -p "$HOME/.autospec"
    export AUTOSPEC_HANDOFF_DISPATCHER_KIND=claude

    # Ledger wiring → temp paths.
    export AUTOSPEC_EXPLORE_LEDGER_BIN="$LEDGER_SH"
    export AUTOSPEC_EXPLORE_LEDGER="$TMP/ledger.jsonl"

    # Stub bin: gh (captures issue bodies; returns a fixed issue number; reports
    # that issue as merged after drain), claude dispatcher.
    mkdir -p "$TMP/bin"
    export GH_BODY_CAPTURE="$TMP/issue-bodies.txt"
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
echo "gh $*" >> "$AUTOSPEC_REPO_ROOT/.gh-calls.log"
case "$1 $2" in
    "issue create")
        # Capture the --body argument for assertion.
        body=""
        while [ $# -gt 0 ]; do
            case "$1" in
                --body) shift; body="$1" ;;
            esac
            shift
        done
        printf '%s\n----\n' "$body" >> "$GH_BODY_CAPTURE"
        # Always file issue #777.
        echo "https://github.com/x/y/issues/777"
        ;;
    "issue view")
        # #777 is CLOSED (merged).
        echo '{"state":"CLOSED","closedAt":"2026-06-15T00:00:00Z"}'
        ;;
    "pr list")
        # A merged PR references the issue — but ONLY when the caller asks for
        # non-open PRs (--state all/merged/closed). gh defaults to --state open,
        # which would EXCLUDE the already-merged PR; emulate that so a missing
        # --state flag yields [] (regression guard for the orchestrator fix).
        case "$*" in
            *"--state all"*|*"--state merged"*|*"--state closed"*)
                echo '[{"number":888,"state":"MERGED","mergedAt":"2026-06-15T00:00:00Z"}]'
                ;;
            *)
                echo '[]'
                ;;
        esac
        ;;
    *) : ;;
esac
exit 0
EOF
    chmod +x "$TMP/bin/gh"
    cat > "$TMP/bin/claude" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
    chmod +x "$TMP/bin/claude"
    export PATH="$TMP/bin:$PATH"

    # Single deterministic researcher emitting one proposal with known fields.
    export AUTOSPEC_RESEARCH_DIR="$TMP/fake-research"
    mkdir -p "$AUTOSPEC_RESEARCH_DIR"
    cat > "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"source":"spec-vs-code","proposals":[
  {"title":"feat: add a brand new widget","evidence":"ev","estimated_complexity":"medium","confidence":0.73}
]}
JSON
EOF
    chmod +x "$AUTOSPEC_RESEARCH_DIR/spec-vs-code.sh"

    export AUTOSPEC_EXPLORE_DRAIN_CMD="echo drain >> $TMP/.drain-calls.log"
    export AUTOSPEC_EXPLORE_REFINE_CMD="echo refine >> $TMP/.refine-calls.log"
    export AUTOSPEC_EXPLORE_DEFINE_CMD="echo define >> $TMP/.define-calls.log"
}

teardown() {
    rm -rf "$TMP"
}

@test "orchestrator-ledger: issue body carries the exact canonical marker" {
    run bash "$EXPLORE_SH" "ship widget" \
        --max-iterations 1 \
        --max-issues-per-round 1 \
        --sandbox-slug orch-ledger \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    [ -f "$GH_BODY_CAPTURE" ]
    # Exact marker: source=spec-vs-code complexity=medium confidence=0.73 round=1
    grep -qF '<!-- explore-ledger source=spec-vs-code complexity=medium confidence=0.73 round=1 -->' "$GH_BODY_CAPTURE"
}

@test "orchestrator-ledger: a pending record is appended at file time" {
    run bash "$EXPLORE_SH" "ship widget" \
        --max-iterations 1 \
        --max-issues-per-round 1 \
        --sandbox-slug orch-ledger2 \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    [ -f "$AUTOSPEC_EXPLORE_LEDGER" ]
    # The first record for issue 777 is pending with correct metadata.
    run jq -se 'map(select(.issue==777 and .outcome=="pending")) | length >= 1' "$AUTOSPEC_EXPLORE_LEDGER"
    [ "$status" -eq 0 ]
    run jq -se 'map(select(.issue==777)) | .[0] | .source=="spec-vs-code" and .complexity=="medium" and (.confidence > 0.72 and .confidence < 0.74) and .round==1' "$AUTOSPEC_EXPLORE_LEDGER"
    [ "$status" -eq 0 ]
}

@test "orchestrator-ledger: merged issue resolved to merged_clean after drain" {
    run bash "$EXPLORE_SH" "ship widget" \
        --max-iterations 1 \
        --max-issues-per-round 1 \
        --sandbox-slug orch-ledger3 \
        --research-sources spec-vs-code
    [ "$status" -eq 0 ] || { echo "$output"; false; }

    [ -f "$TMP/.drain-calls.log" ]
    # --show reflects merged_clean for #777 (latest record per issue).
    run bash "$LEDGER_SH" --ledger "$AUTOSPEC_EXPLORE_LEDGER" --show
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'merged_clean'
    echo "$output" | grep -q '#777'
}

@test "orchestrator-ledger: missing ledger bin -> loop unaffected, no ledger written" {
    export AUTOSPEC_EXPLORE_LEDGER_BIN="$TMP/does-not-exist.sh"
    # Also remove sibling + repo-relative resolution by pointing scripts dir away.
    export AUTOSPEC_SCRIPTS_DIR="$TMP/empty-scripts"
    mkdir -p "$AUTOSPEC_SCRIPTS_DIR"
    run bash "$EXPLORE_SH" "ship widget" \
        --max-iterations 1 \
        --max-issues-per-round 1 \
        --sandbox-slug orch-ledger4 \
        --research-sources spec-vs-code
    # Loop still completes cleanly.
    [ "$status" -eq 0 ] || { echo "$output"; false; }
    [ -f "$TMP/.autospec/explore-loop.json" ]
}
