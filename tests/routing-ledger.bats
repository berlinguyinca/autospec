#!/usr/bin/env bats
# tests/routing-ledger.bats — TDD for scripts/routing-ledger.sh
#
# The ledger is a data-integrity tool: a bad row silently poisons every derived
# weight downstream, so validation is tested as hard as the happy path.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/routing-ledger.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/routing-ledger-XXXXXX")"
    LEDGER="$TMP/routing-ledger.jsonl"
}

teardown() { rm -rf "$TMP"; }

# rec <id> <kind> <profile> <reasoning> <in> <cached> <ms> <retries> <esc> <outcome>
rec() {
    printf '{"dispatch_id":"%s","ts":"2026-08-05T10:00:00Z","dispatch_kind":"%s","profile":"%s","model":"m","harness":"claude","issue":1,"cell_ctx":"64k","cell_reasoning":"%s","input_tokens":%s,"output_tokens":10,"cached_tokens":%s,"wall_clock_ms":%s,"retries":%s,"escalated":%s,"outcome":"%s","reason":""}' \
        "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}"
}

@test "routing-ledger.sh is executable" {
    run test -x "$SCRIPT"
    [ "$status" -eq 0 ]
}

@test "--help exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
}

@test "no mode is a usage error" {
    run bash "$SCRIPT" --ledger "$LEDGER"
    [ "$status" -eq 1 ]
}

@test "appends a valid record and creates the ledger directory" {
    LEDGER="$TMP/nested/dir/ledger.jsonl"
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 1000 800 5000 0 false merged_clean)"
    [ "$status" -eq 0 ]
    [ -f "$LEDGER" ]
    run grep -c . "$LEDGER"
    [ "$output" = "1" ]
}

# ── validation: every reject below would otherwise poison derived weights ─────

@test "rejects a record missing a required key" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append '{"dispatch_id":"d1","dispatch_kind":"implementer"}'
    [ "$status" -eq 1 ]
    [[ "$output" == *"missing required key"* ]]
}

@test "rejects a non-object" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append '["not","an","object"]'
    [ "$status" -eq 1 ]
    [[ "$output" == *"not a JSON object"* ]]
}

@test "rejects an unknown outcome" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false banana)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"invalid outcome"* ]]
}

@test "rejects an unknown dispatch_kind" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 wat haiku shallow 100 0 10 0 false merged_clean)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"invalid dispatch_kind"* ]]
}

@test "rejects an off-ordinal cell_reasoning" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku extreme 100 0 10 0 false merged_clean)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"invalid cell_reasoning"* ]]
}

@test "rejects string counters that would poison the cost formula" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append \
        '{"dispatch_id":"d1","ts":"t","dispatch_kind":"implementer","profile":"p","model":"m","harness":"h","issue":1,"cell_ctx":"64k","cell_reasoning":"shallow","input_tokens":"1000","output_tokens":10,"cached_tokens":0,"wall_clock_ms":1,"retries":0,"escalated":false,"outcome":"merged_clean","reason":""}'
    [ "$status" -eq 1 ]
    [[ "$output" == *"non-negative numbers"* ]]
}

@test "rejects negative counters" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 -1 false merged_clean)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"non-negative numbers"* ]]
}

@test "rejects cached_tokens exceeding input_tokens" {
    # A ratio above 1 means double-counting, which would push the cache penalty
    # below its true floor and make the profile look cheaper than it is.
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 500 10 0 false merged_clean)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"cached_tokens may not exceed"* ]]
}

@test "rejects a non-boolean escalated" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append \
        '{"dispatch_id":"d1","ts":"t","dispatch_kind":"implementer","profile":"p","model":"m","harness":"h","issue":1,"cell_ctx":"64k","cell_reasoning":"shallow","input_tokens":10,"output_tokens":10,"cached_tokens":0,"wall_clock_ms":1,"retries":0,"escalated":"yes","outcome":"merged_clean","reason":""}'
    [ "$status" -eq 1 ]
    [[ "$output" == *"escalated must be a boolean"* ]]
}

# ── append-only audit trail ───────────────────────────────────────────────────

@test "--update-outcome appends rather than rewriting" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    run bash "$SCRIPT" --ledger "$LEDGER" --update-outcome d1 merged_clean "ok"
    [ "$status" -eq 0 ]
    run grep -c . "$LEDGER"
    [ "$output" = "2" ]
}

@test "readers take the latest line per dispatch_id" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    run bash "$SCRIPT" --ledger "$LEDGER" --update-outcome d1 reverted "flaky"
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" --ledger "$LEDGER" --show --json
    [ "$(printf '%s' "$output" | jq 'length')" -eq 1 ]
    [ "$(printf '%s' "$output" | jq -r '.[0].outcome')" = "reverted" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].reason')" = "flaky" ]
}

@test "--update-outcome on an unknown dispatch_id fails" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    run bash "$SCRIPT" --ledger "$LEDGER" --update-outcome nope merged_clean
    [ "$status" -eq 1 ]
    [[ "$output" == *"not found"* ]]
}

@test "--update-outcome rejects an unknown outcome" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    run bash "$SCRIPT" --ledger "$LEDGER" --update-outcome d1 banana
    [ "$status" -eq 1 ]
    [[ "$output" == *"invalid outcome"* ]]
}

# ── stats: the contract routing-cost.sh consumes ──────────────────────────────

@test "stats aggregate per (dispatch_kind, profile, cell)" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 1000 800 5000 0 false merged_clean)"
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d2 implementer haiku shallow 1000 0 9000 2 true qa_failed)"
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d3 lgtm-reviewer sonnet medium 500 100 3000 0 false lgtm_first_pass)"
    run bash "$SCRIPT" --ledger "$LEDGER" --stats --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq 'length')" -eq 2 ]
    row="$(printf '%s' "$output" | jq -c '.[]|select(.dispatch_kind=="implementer")')"
    [ "$(printf '%s' "$row" | jq '.dispatches')" -eq 2 ]
    [ "$(printf '%s' "$row" | jq '.first_pass_rate')" = "0.5" ]
    [ "$(printf '%s' "$row" | jq '.escalation_rate')" = "0.5" ]
    [ "$(printf '%s' "$row" | jq '.mean_retries')" = "1" ]
}

@test "cache_hit_ratio is cached over input tokens" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 1000 800 100 0 false merged_clean)"
    run bash "$SCRIPT" --ledger "$LEDGER" --stats --json
    [ "$(printf '%s' "$output" | jq -r '.[0].cache_hit_ratio')" = "0.8" ]
}

@test "pending dispatches are excluded from stats" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    run bash "$SCRIPT" --ledger "$LEDGER" --stats --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq 'length')" -eq 0 ]
}

@test "a missing ledger yields empty stats rather than an error" {
    run bash "$SCRIPT" --ledger "$TMP/absent.jsonl" --stats --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq 'length')" -eq 0 ]
}

@test "--show filters by profile and by kind" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)"
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d2 lgtm-reviewer sonnet medium 100 0 10 0 false merged_clean)"
    run bash "$SCRIPT" --ledger "$LEDGER" --show --kind implementer --json
    [ "$(printf '%s' "$output" | jq 'length')" -eq 1 ]
    run bash "$SCRIPT" --ledger "$LEDGER" --show --profile sonnet --json
    [ "$(printf '%s' "$output" | jq -r '.[0].dispatch_kind')" = "lgtm-reviewer" ]
}

# ── validate ──────────────────────────────────────────────────────────────────

@test "--validate accepts a clean ledger and a missing one" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)"
    run bash "$SCRIPT" --ledger "$LEDGER" --validate
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" --ledger "$TMP/absent.jsonl" --validate
    [ "$status" -eq 0 ]
}

@test "--validate reports the offending line number" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)"
    printf '{"dispatch_id":"bad"}\n' >> "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --validate
    [ "$status" -eq 1 ]
    [[ "$output" == *":2:"* ]]
}
