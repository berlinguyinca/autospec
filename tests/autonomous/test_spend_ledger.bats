#!/usr/bin/env bats
# tests/autonomous/test_spend_ledger.bats — unit tests for
# scripts/autonomous-spend-ledger.sh (cumulative cost kill-switch).
#
# Covers:
#  - add: increments accumulate across calls
#  - check: returns "continue" while under caps
#  - check: returns "park <reason>" at/over token cap
#  - check: returns "park <reason>" at/over issue cap
#  - park path invokes stubbed notify.sh
#  - ledger file JSON structure is correct
#  - atomic writes: file is valid JSON after each operation
#  - reset zeroes the totals
#  - status prints current ledger JSON
#  - macOS bash 3.2 compatibility: no process substitution in [ -f ]

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    SCRIPT="$REPO_ROOT/scripts/autonomous-spend-ledger.sh"

    # Isolated ledger dir per test (avoids cross-test contamination).
    TEST_DIR="$(mktemp -d)"
    export AUTOSPEC_AUTONOMOUS_SPEND_BASE="$TEST_DIR/spend"
    # Override HOME so ledger writes go to our temp dir.
    export HOME="$TEST_DIR"

    # Stub notify.sh: record invocations to a file so we can assert them.
    NOTIFY_STUB="$TEST_DIR/notify.sh"
    NOTIFY_LOG="$TEST_DIR/notify.log"
    cat > "$NOTIFY_STUB" <<'SH'
#!/usr/bin/env bash
printf '%s\t%s\n' "$1" "$2" >> "$NOTIFY_LOG"
SH
    chmod +x "$NOTIFY_STUB"
    export PATH="$TEST_DIR:$PATH"
    export NOTIFY_LOG

    # Use a fixed fake repo-dir so slug derivation is deterministic.
    REPO_DIR="$TEST_DIR/fake-repo"
    mkdir -p "$REPO_DIR/.git"
    # Create a minimal git repo so git remote get-url works predictably.
    (
        cd "$REPO_DIR"
        git init -q
        git remote add origin "https://github.com/test-owner/test-repo.git"
    )
    export REPO_DIR
}

teardown() {
    rm -rf "$TEST_DIR"
}

run_ledger() {
    run bash "$SCRIPT" "$@" --repo-dir "$REPO_DIR"
}

# ── add ───────────────────────────────────────────────────────────────────────

@test "add: first call creates ledger and sets tokens/issues" {
    run_ledger add --tokens 100 --issues 1
    [ "$status" -eq 0 ]
    tokens="$(printf '%s' "$output" | jq -r '.tokens')"
    issues="$(printf '%s' "$output" | jq -r '.issues')"
    filed_issues="$(printf '%s' "$output" | jq -r '.filed_issues')"
    budget_issues="$(printf '%s' "$output" | jq -r '.budget_issues')"
    [ "$tokens" -eq 100 ]
    [ "$issues" -eq 1 ]
    [ "$filed_issues" -eq 1 ]
    [ "$budget_issues" -eq 1 ]
}

@test "add: supports distinct filed and budget issue counters" {
    run_ledger add --tokens 100 --filed-issues 5 --budget-issues 1
    [ "$status" -eq 0 ]
    tokens="$(printf '%s' "$output" | jq -r '.tokens')"
    issues="$(printf '%s' "$output" | jq -r '.issues')"
    filed_issues="$(printf '%s' "$output" | jq -r '.filed_issues')"
    budget_issues="$(printf '%s' "$output" | jq -r '.budget_issues')"
    [ "$tokens" -eq 100 ]
    [ "$issues" -eq 1 ]
    [ "$filed_issues" -eq 5 ]
    [ "$budget_issues" -eq 1 ]
}

@test "add: repeated calls accumulate totals" {
    run_ledger add --tokens 100 --issues 1
    run_ledger add --tokens 200 --issues 2
    run_ledger add --tokens 50 --issues 0

    tokens="$(printf '%s' "$output" | jq -r '.tokens')"
    issues="$(printf '%s' "$output" | jq -r '.issues')"
    [ "$tokens" -eq 350 ]
    [ "$issues" -eq 3 ]
}

@test "add: ledger file is valid JSON after each call" {
    run_ledger add --tokens 100 --issues 1
    ledger_file="$(find "$HOME/.autospec/autonomous-spend" -name "spend.json" 2>/dev/null | head -1)"
    [ -n "$ledger_file" ]
    jq empty "$ledger_file"
}

@test "add: default --issues is 0 when omitted" {
    run_ledger add --tokens 42
    issues="$(printf '%s' "$output" | jq -r '.issues')"
    [ "$issues" -eq 0 ]
}

@test "add: schema field is present in ledger" {
    run_ledger add --tokens 1
    schema="$(printf '%s' "$output" | jq -r '.schema')"
    [ "$schema" = "1" ]
}

# ── check: continue ────────────────────────────────────────────────────────────

@test "check: returns 'continue' with empty ledger (no file yet)" {
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=100 \
    run_ledger check
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

@test "check: returns 'continue' while under both caps" {
    run_ledger add --tokens 100 --issues 1
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

@test "check: issue cap uses budget_issues rather than filed_issues" {
    run_ledger add --tokens 0 --filed-issues 5 --budget-issues 2
    [ "$status" -eq 0 ]
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=3 \
    run_ledger check
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

@test "check: returns 'continue' with tokens at cap minus 1" {
    run_ledger add --tokens 999 --issues 0
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

# ── check: park on token cap ──────────────────────────────────────────────────

@test "check: returns 'park' when tokens exactly equal the token cap" {
    run_ledger add --tokens 1000 --issues 0
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    [ "$status" -eq 0 ]
    [[ "$output" == park* ]]
    [[ "$output" == *"token cap"* ]]
}

@test "check: returns 'park' when tokens exceed the token cap" {
    run_ledger add --tokens 2000 --issues 0
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    [ "$status" -eq 0 ]
    [[ "$output" == park* ]]
}

@test "check: park on token cap calls stubbed notify.sh" {
    run_ledger add --tokens 1000 --issues 0
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    [ -f "$NOTIFY_LOG" ]
    grep -q "autospec-autonomous parked" "$NOTIFY_LOG"
}

@test "check: park writes parked=true to ledger JSON" {
    run_ledger add --tokens 1000 --issues 0
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    ledger_file="$(find "$HOME/.autospec/autonomous-spend" -name "spend.json" 2>/dev/null | head -1)"
    parked="$(jq -r '.parked' "$ledger_file")"
    [ "$parked" = "true" ]
}

# ── check: park on issue cap ──────────────────────────────────────────────────

@test "check: returns 'park' when issues exactly equal the issue cap" {
    run_ledger add --tokens 0 --issues 10
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    [ "$status" -eq 0 ]
    [[ "$output" == park* ]]
    [[ "$output" == *"issue cap"* ]]
}

@test "check: returns 'park' when issues exceed the issue cap" {
    run_ledger add --tokens 0 --issues 20
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    [ "$status" -eq 0 ]
    [[ "$output" == park* ]]
}

@test "check: park on issue cap calls stubbed notify.sh" {
    run_ledger add --tokens 0 --issues 10
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    [ -f "$NOTIFY_LOG" ]
    grep -q "autospec-autonomous parked" "$NOTIFY_LOG"
}

# ── check: disabled caps (0 means no cap) ─────────────────────────────────────

@test "check: token cap of 0 means disabled (no park even at very high tokens)" {
    run_ledger add --tokens 99999999 --issues 0
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=0 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=500 \
    run_ledger check
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

@test "check: issue cap of 0 means disabled (no park even at very high issues)" {
    run_ledger add --tokens 0 --issues 99999
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=0 \
    run_ledger check
    [ "$status" -eq 0 ]
    [ "$output" = "continue" ]
}

# ── reset ─────────────────────────────────────────────────────────────────────

@test "reset: zeroes tokens and issue counters" {
    run_ledger add --tokens 500 --filed-issues 7 --budget-issues 5
    run_ledger reset
    run_ledger status
    tokens="$(printf '%s' "$output" | jq -r '.tokens')"
    issues="$(printf '%s' "$output" | jq -r '.issues')"
    filed_issues="$(printf '%s' "$output" | jq -r '.filed_issues')"
    budget_issues="$(printf '%s' "$output" | jq -r '.budget_issues')"
    [ "$tokens" -eq 0 ]
    [ "$issues" -eq 0 ]
    [ "$filed_issues" -eq 0 ]
    [ "$budget_issues" -eq 0 ]
}

@test "reset: after reset, check returns continue" {
    run_ledger add --tokens 9999999 --issues 9999
    run_ledger reset
    AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS=1000 \
    AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES=10 \
    run_ledger check
    [ "$output" = "continue" ]
}

# ── status ────────────────────────────────────────────────────────────────────

@test "status: prints valid JSON with tokens and issues fields" {
    run_ledger add --tokens 77 --issues 3
    run_ledger status
    [ "$status" -eq 0 ]
    tokens="$(printf '%s' "$output" | jq -r '.tokens')"
    issues="$(printf '%s' "$output" | jq -r '.issues')"
    [ "$tokens" -eq 77 ]
    [ "$issues" -eq 3 ]
}

@test "status: returns zero-state when no ledger exists yet" {
    run_ledger status
    [ "$status" -eq 0 ]
    tokens="$(printf '%s' "$output" | jq -r '.tokens')"
    [ "$tokens" -eq 0 ]
}

# ── path scoping ──────────────────────────────────────────────────────────────

@test "path-scoped: different repo dirs produce different ledger files" {
    REPO_DIR2="$TEST_DIR/fake-repo-2"
    mkdir -p "$REPO_DIR2/.git"
    (
        cd "$REPO_DIR2"
        git init -q
        git remote add origin "https://github.com/test-owner/other-repo.git"
    )

    run bash "$SCRIPT" add --tokens 100 --issues 1 --repo-dir "$REPO_DIR"
    run bash "$SCRIPT" add --tokens 999 --issues 9 --repo-dir "$REPO_DIR2"

    # Check that the two repos have independent totals.
    run bash "$SCRIPT" status --repo-dir "$REPO_DIR"
    t1="$(printf '%s' "$output" | jq -r '.tokens')"
    run bash "$SCRIPT" status --repo-dir "$REPO_DIR2"
    t2="$(printf '%s' "$output" | jq -r '.tokens')"

    [ "$t1" -eq 100 ]
    [ "$t2" -eq 999 ]
}

# ── macOS bash 3.2 / real temp file check ─────────────────────────────────────

@test "ledger file is a real file on disk (not process substitution artifact)" {
    run_ledger add --tokens 10 --issues 1
    # Write ledger path to a real temp file before testing [ -f ]
    # (feedback_bash32_process_sub_test_file: [ -f <(...) ] is false on macOS bash 3.2)
    tmp_path="$(mktemp)"
    find "$HOME/.autospec/autonomous-spend" -name "spend.json" 2>/dev/null > "$tmp_path"
    ledger_file="$(cat "$tmp_path")"
    rm -f "$tmp_path"
    [ -n "$ledger_file" ]
    [ -f "$ledger_file" ]
}
