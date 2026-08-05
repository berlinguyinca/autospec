#!/usr/bin/env bats
# tests/verify-voter-vendor.bats — vendor selection for the verify voter.
#
# The property under test is INDEPENDENCE, not cost. A voter drawn from the
# proposer's own vendor shares its failure modes and tends to be wrong with it,
# which is the single case a verify pass exists to catch. Every test below either
# pins that independence or pins that the script fails closed rather than
# pretending to have it.

VOTER="${BATS_TEST_DIRNAME}/../scripts/verify-voter-vendor.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/voter-vendor-XXXXXX")"
    LEDGER="$TMP/ledger.jsonl"
    : > "$LEDGER"
}

teardown() { rm -rf "$TMP"; }

# ledger_row <dispatch_id> <harness> <in> <out>
ledger_row() {
    printf '{"dispatch_id":"%s","harness":"%s","input_tokens":%s,"output_tokens":%s}\n' \
        "$1" "$2" "$3" "$4" >> "$LEDGER"
}

@test "verify-voter-vendor.sh is executable" {
    run test -x "$VOTER"
    [ "$status" -eq 0 ]
}

@test "--proposer is required" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" --ledger "$LEDGER"
    [ "$status" -eq 1 ]
    [[ "$output" == *"--proposer is required"* ]]
}

@test "an unknown vendor is rejected rather than silently ignored" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" --proposer gemini --ledger "$LEDGER"
    [ "$status" -eq 1 ]
    [[ "$output" == *"unknown vendor: gemini"* ]]
}

@test "a typo in AUTOSPEC_VOTER_VENDORS is an error, not a smaller fleet" {
    # Silently dropping it would read as "that harness is not installed" and
    # could collapse the candidate set to the proposer alone.
    run env AUTOSPEC_VOTER_VENDORS="claude,codx" bash "$VOTER" --proposer claude --ledger "$LEDGER"
    [ "$status" -eq 1 ]
    [[ "$output" == *"AUTOSPEC_VOTER_VENDORS"* ]]
}

# ── the independence invariant ─────────────────────────────────────────────────

@test "the voter is never the proposer's own vendor" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" --proposer claude --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" --proposer codex --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "claude" ]
}

@test "a single-vendor host exits 3 instead of naming the proposer" {
    # Fail closed: printing "claude" here would claim an independence the host
    # cannot provide. Exit 3 means the caller keeps its same-vendor TIER_B voter.
    run env AUTOSPEC_VOTER_VENDORS="claude" bash "$VOTER" --proposer claude --ledger "$LEDGER"
    [ "$status" -eq 3 ]
    [ -z "$output" ]
}

@test "failover exhausting the independent vendors exits 3, never the proposer" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" \
        --proposer claude --unavailable codex --ledger "$LEDGER"
    [ "$status" -eq 3 ]
    [ "$output" != "claude" ]
}

# ── reactive failover is the load-bearing mechanism ───────────────────────────

@test "an unavailable vendor is skipped even when it has the least spend" {
    # A 429 is ground truth; ledger spend is an estimate. Availability must win.
    ledger_row d1 opencode 10 5
    ledger_row d2 codex 900000 400000
    run env AUTOSPEC_VOTER_VENDORS="claude,codex,opencode" bash "$VOTER" \
        --proposer claude --unavailable opencode --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
}

@test "several unavailable vendors can be named" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex,opencode" bash "$VOTER" \
        --proposer opencode --unavailable claude --unavailable codex --ledger "$LEDGER"
    [ "$status" -eq 3 ]
}

# ── spend is a tiebreak, and only a tiebreak ──────────────────────────────────

@test "the least-spent independent vendor wins" {
    ledger_row d1 codex 900000 400000
    ledger_row d2 opencode 100 50
    run env AUTOSPEC_VOTER_VENDORS="claude,codex,opencode" bash "$VOTER" \
        --proposer claude --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "opencode" ]
}

@test "spend counts every dispatch kind, not just verify-voter rows" {
    # Quota is consumed per harness: an implementer dispatch spends the same
    # budget a voter would, so filtering to voter rows would understate it.
    printf '{"dispatch_id":"d1","dispatch_kind":"implementer","harness":"codex","input_tokens":900000,"output_tokens":400000}\n' >> "$LEDGER"
    printf '{"dispatch_id":"d2","dispatch_kind":"verify-voter","harness":"opencode","input_tokens":10,"output_tokens":5}\n' >> "$LEDGER"
    run env AUTOSPEC_VOTER_VENDORS="claude,codex,opencode" bash "$VOTER" \
        --proposer claude --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "opencode" ]
}

@test "only the latest row per dispatch_id is counted" {
    # The ledger is append-only, so a corrected row supersedes rather than adds.
    ledger_row d1 codex 900000 400000
    ledger_row d1 codex 1 1
    run env AUTOSPEC_VOTER_VENDORS="codex,opencode" bash "$VOTER" \
        --proposer opencode --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
}

@test "an equal-spend tie resolves deterministically to the first candidate" {
    ledger_row d1 codex 100 50
    ledger_row d2 opencode 100 50
    run env AUTOSPEC_VOTER_VENDORS="codex,opencode" bash "$VOTER" \
        --proposer claude --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    first="$output"
    run env AUTOSPEC_VOTER_VENDORS="codex,opencode" bash "$VOTER" \
        --proposer claude --ledger "$LEDGER"
    [ "$output" = "$first" ]
    [ "$output" = "codex" ]
}

# ── missing or unreadable signal must not break the choice ────────────────────

@test "a missing ledger still yields an independent vendor" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" \
        --proposer claude --ledger "$TMP/does-not-exist.jsonl"
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
}

@test "a malformed ledger line does not abort the decision" {
    printf 'not json at all\n' >> "$LEDGER"
    ledger_row d1 codex 100 50
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" \
        --proposer claude --ledger "$LEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
}

@test "--explain names each narrowing step on stderr, leaving stdout clean" {
    ledger_row d1 codex 100 50
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" \
        --proposer claude --ledger "$LEDGER" --explain
    [ "$status" -eq 0 ]
    [[ "$output" == *"after failover"* ]]
    [[ "$output" == *"independent of proposer=claude"* ]]
    # stdout alone must still be exactly the vendor, since callers capture it.
    stdout_only="$(env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" \
        --proposer claude --ledger "$LEDGER" --explain 2>/dev/null)"
    [ "$stdout_only" = "codex" ]
}
