#!/usr/bin/env bats
# tests/reviewer-vendor.bats — TDD for issue #3347 (guardrails R4):
# a PR reviewer must never share a vendor with the PR author.
#
# The selector is the reviewer role of scripts/verify-voter-vendor.sh:
# given an authoring vendor, it returns a DIFFERENT dispatchable (cloud)
# vendor — or exits 3 so the caller keeps its own TIER_A instead of
# silently proceeding with a same-vendor reviewer.
#
# Rules under test (docs/plans/2026-08-24-local-model-guardrails.md R4):
#   - local-implemented PR  -> reviewer must be cloud
#   - claude-implemented PR -> prefer codex (genuinely different vendor)
#   - any author            -> reviewer vendor != author vendor
#   - no distinct vendor    -> exit 3 (fail closed), never same vendor
#
# Plus the ledger half of the issue: the implementer ledger row records the
# authoring vendor (scripts/routing-ledger.sh `authoring_vendor`, optional
# and append-only-safe: legacy rows without it stay valid).

VOTER="${BATS_TEST_DIRNAME}/../scripts/verify-voter-vendor.sh"
LEDGER="${BATS_TEST_DIRNAME}/../scripts/routing-ledger.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/reviewer-vendor-XXXXXX")"
    RLEDGER="$TMP/routing-ledger.jsonl"
}

teardown() { rm -rf "$TMP"; }

# ledger_row <dispatch_id> <harness> <input> <output>
ledger_row() {
    printf '{"dispatch_id":"%s","harness":"%s","input_tokens":%s,"output_tokens":%s}\n' \
        "$1" "$2" "$3" "$4" >> "$RLEDGER"
}

# rec <id> <kind> <profile> <reasoning> <in> <cached> <ms> <retries> <esc> <outcome>
rec() {
    printf '{"dispatch_id":"%s","ts":"2026-08-05T10:00:00Z","dispatch_kind":"%s","profile":"%s","model":"m","harness":"claude","issue":1,"cell_ctx":"64k","cell_reasoning":"%s","input_tokens":%s,"output_tokens":10,"cached_tokens":%s,"wall_clock_ms":%s,"retries":%s,"escalated":%s,"outcome":"%s","reason":""}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}"
}

# ── argument contract ─────────────────────────────────────────────────────────

@test "--role reviewer without --author is a usage error" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" --role reviewer
    [ "$status" -eq 1 ]
    [[ "$output" == *"--author"* ]]
}

@test "--author with an unknown vendor is rejected, not silently ignored" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" --role reviewer --author gemini
    [ "$status" -eq 1 ]
    [[ "$output" == *"unknown author vendor"* ]]
}

@test "--author in voter (default) mode is a usage error" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" --proposer claude --author codex
    [ "$status" -eq 1 ]
}

@test "an unknown --role is a usage error" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" --role critic --author claude
    [ "$status" -eq 1 ]
}

# ── the independence invariant ────────────────────────────────────────────────

@test "the reviewer is never the author's own vendor (all three cloud authors)" {
    local author out
    for author in claude codex opencode; do
        out="$(env AUTOSPEC_VOTER_VENDORS="claude,codex,opencode" bash "$VOTER" \
            --role reviewer --author "$author" --ledger "$RLEDGER")" || {
            echo "author=$author: expected exit 0" >&2; return 1; }
        [ "$out" != "$author" ] || { echo "author=$author: reviewer=$out is the same vendor" >&2; return 1; }
    done
}

@test "claude-authored PR: codex is preferred even when another vendor spent less" {
    ledger_row d1 codex 900000 400000
    ledger_row d2 opencode 100 50
    run env AUTOSPEC_VOTER_VENDORS="claude,codex,opencode" bash "$VOTER" \
        --role reviewer --author claude --ledger "$RLEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
}

@test "claude-authored PR with codex unavailable: falls to the remaining cloud vendor" {
    ledger_row d1 opencode 100 50
    run env AUTOSPEC_VOTER_VENDORS="claude,codex,opencode" bash "$VOTER" \
        --role reviewer --author claude --unavailable codex --ledger "$RLEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "opencode" ]
}

@test "non-claude author: least ledger spend still tiebreaks among independents" {
    ledger_row d1 claude 900000 400000
    ledger_row d2 opencode 100 50
    run env AUTOSPEC_VOTER_VENDORS="claude,codex,opencode" bash "$VOTER" \
        --role reviewer --author codex --ledger "$RLEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "opencode" ]
}

# ── local author -> cloud reviewer ───────────────────────────────────────────

@test "local is an accepted authoring vendor and the reviewer is cloud" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex,opencode" bash "$VOTER" \
        --role reviewer --author local --ledger "$RLEDGER"
    [ "$status" -eq 0 ]
    case "$output" in
        local|'') echo "local-authored PR got a local/empty reviewer" >&2; return 1 ;;
        claude|codex|opencode) ;;
        *) echo "reviewer '$output' is not a cloud vendor" >&2; return 1 ;;
    esac
}

@test "'local' in the vendor list is never itself returned as a reviewer" {
    local out
    out="$(env AUTOSPEC_VOTER_VENDORS="claude,local" bash "$VOTER" \
        --role reviewer --author local --ledger "$RLEDGER")" || {
        echo "expected exit 0 (claude is an available cloud vendor)" >&2; return 1; }
    [ "$out" = "claude" ]
}

# ── fail closed ───────────────────────────────────────────────────────────────

@test "no distinct vendor available: exit 3 (caller keeps TIER_A), not a silent same-vendor pick" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" \
        --role reviewer --author claude --unavailable codex --ledger "$RLEDGER"
    [ "$status" -eq 3 ]
    [[ "$output" == *"no independent reviewer"* ]]
    [ -z "$output" || ! grep -qx "claude" <<<"$output" ]
}

@test "single-harness host: a same-vendor reviewer is refused with exit 3" {
    run env AUTOSPEC_VOTER_VENDORS="claude" bash "$VOTER" \
        --role reviewer --author claude --ledger "$RLEDGER"
    [ "$status" -eq 3 ]
}

@test "local author with no cloud vendor installed: exit 3, never a local reviewer" {
    run env AUTOSPEC_VOTER_VENDORS="local" bash "$VOTER" \
        --role reviewer --author claude --ledger "$RLEDGER"
    [ "$status" -eq 3 ]
}

# ── explain + stdout hygiene ─────────────────────────────────────────────────

@test "--explain narrates the author independence on stderr; stdout stays clean" {
    local out
    out="$(env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" \
        --role reviewer --author codex --ledger "$RLEDGER" --explain 2>/dev/null)" || {
        echo "expected exit 0" >&2; return 1; }
    [ "$out" = "claude" ]
    [ -n "$out" ]
}

# ── voter mode regression (the pre-existing contract is untouched) ───────────

@test "voter mode is unchanged: --proposer claude still returns a different vendor" {
    run env AUTOSPEC_VOTER_VENDORS="claude,codex" bash "$VOTER" \
        --proposer claude --ledger "$RLEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "codex" ]
}

# ── ledger half: the implementer row records the authoring vendor ───────────

@test "routing-ledger.sh accepts an implementer row with authoring_vendor" {
    local row
    row="$(rec d1 implementer sonnet shallow 100 0 10 0 false merged_clean)"
    row="$(printf '%s' "$row" | jq -c '. + {authoring_vendor:"claude"}')"
    run bash "$LEDGER" --ledger "$RLEDGER" --append "$row"
    [ "$status" -eq 0 ]
    run grep -c '"authoring_vendor":"claude"' "$RLEDGER"
    [ "$output" = "1" ]
}

@test "authoring_vendor 'local' is accepted (local-implemented PR)" {
    local row
    row="$(rec d1 implementer local-4090 shallow 100 0 10 0 false merged_clean)"
    row="$(printf '%s' "$row" | jq -c '. + {authoring_vendor:"local"}')"
    run bash "$LEDGER" --ledger "$RLEDGER" --append "$row"
    [ "$status" -eq 0 ]
}

@test "an unknown authoring_vendor is rejected, not recorded" {
    local row
    row="$(rec d1 implementer sonnet shallow 100 0 10 0 false merged_clean)"
    row="$(printf '%s' "$row" | jq -c '. + {authoring_vendor:"gemini"}')"
    run bash "$LEDGER" --ledger "$RLEDGER" --append "$row"
    [ "$status" -eq 1 ]
    [[ "$output" == *"authoring_vendor"* ]]
    [ ! -f "$RLEDGER" ]
}

@test "a non-string authoring_vendor is rejected" {
    local row
    row="$(rec d1 implementer sonnet shallow 100 0 10 0 false merged_clean)"
    row="$(printf '%s' "$row" | jq -c '. + {authoring_vendor:42}')"
    run bash "$LEDGER" --ledger "$RLEDGER" --append "$row"
    [ "$status" -eq 1 ]
    [[ "$output" == *"authoring_vendor"* ]]
}

@test "legacy rows without authoring_vendor stay valid (append-only)" {
    run bash "$LEDGER" --ledger "$RLEDGER" --append "$(rec d1 implementer sonnet shallow 100 0 10 0 false merged_clean)"
    [ "$status" -eq 0 ]
    run bash "$LEDGER" --ledger "$RLEDGER" --validate
    [ "$status" -eq 0 ]
}

@test "--update-outcome preserves authoring_vendor on the appended row" {
    local row
    row="$(rec d1 implementer sonnet shallow 100 0 10 0 false pending)"
    row="$(printf '%s' "$row" | jq -c '. + {authoring_vendor:"local"}')"
    run bash "$LEDGER" --ledger "$RLEDGER" --append "$row"
    run bash "$LEDGER" --ledger "$RLEDGER" --update-outcome d1 merged_clean
    [ "$status" -eq 0 ]
    run bash "$LEDGER" --ledger "$RLEDGER" --show --json
    [ "$(printf '%s' "$output" | jq -r '.[0].authoring_vendor')" = "local" ]
}
