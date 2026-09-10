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

# ── §25 per-dispatch telemetry ───────────────────────────────────────────────
# Optional at rest (legacy rows predate them) but typed when present, and every
# omitted field normalizes to "unknown" — never a fabricated 0.

TEL_KEYS='["role","model_version","hardware_fingerprint","runtime","quantization","previous_model","review_outcome","tests_outcome","context_requested","context_reserved","context_used","concurrency_at_start","queue_depth_at_start","prompt_tok_s","decode_tok_s","aggregate_decode_tok_s","ttft_ms","retry_index","merged","reverted"]'

TEL_FULL='{"role":"implementer","model_version":"1.2.3","hardware_fingerprint":"rtx4090-node7","runtime":"vllm","quantization":"q4_k_m","context_requested":131072,"context_reserved":96000,"context_used":81234,"concurrency_at_start":2,"queue_depth_at_start":1,"prompt_tok_s":812.5,"decode_tok_s":64.2,"aggregate_decode_tok_s":118.4,"ttft_ms":240,"retry_index":0,"previous_model":"qwen3-coder","review_outcome":"approved","tests_outcome":"passed","merged":true,"reverted":false}'

# rec_tel <telemetry-json> <rec args...> — base record merged with telemetry.
rec_tel() {
    rec "$2" "$3" "$4" "$5" "$6" "$7" "$8" "$9" "${10}" "${11}" \
        | jq -c --argjson t "$1" '. + $t'
}

@test "append accepts all 20 §25 telemetry fields verbatim" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel "$TEL_FULL" d1 implementer haiku shallow 100 0 10 0 false merged_clean)"
    [ "$status" -eq 0 ]
    [ "$(jq '. | length' "$LEDGER")" -eq 38 ]
    [ "$(jq -r '.role' "$LEDGER")" = "implementer" ]
    [ "$(jq -r '.decode_tok_s' "$LEDGER")" = "64.2" ]
    [ "$(jq -r '.merged' "$LEDGER")" = "true" ]
}

@test "every omitted §25 field normalizes to unknown, never 0 or null" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    [ "$status" -eq 0 ]
    for k in $(jq -r '.[]' <<<"$TEL_KEYS"); do
        [ "$(jq -r --arg k "$k" '.[$k]' "$LEDGER")" = "unknown" ]
    done
}

@test "explicit unknown is accepted for every §25 field" {
    tel="$(jq -cn --argjson keys "$TEL_KEYS" '[ $keys[] | {key:., value:"unknown"} ] | from_entries')"
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel "$tel" d1 implementer haiku shallow 100 0 10 0 false pending)"
    [ "$status" -eq 0 ]
    [ "$(jq -r '.ttft_ms' "$LEDGER")" = "unknown" ]
    [ "$(jq -r '.merged' "$LEDGER")" = "unknown" ]
}

@test "rejects a stringly-typed §25 number" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel '{"decode_tok_s":"64.2"}' d1 implementer haiku shallow 100 0 10 0 false pending)"
    [ "$status" -eq 1 ]
    [[ "$output" == *'decode_tok_s must be a non-negative number or "unknown"'* ]]
    [ ! -f "$LEDGER" ]
}

@test "rejects a negative §25 number" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel '{"ttft_ms":-5}' d1 implementer haiku shallow 100 0 10 0 false pending)"
    [ "$status" -eq 1 ]
    [[ "$output" == *'ttft_ms must be a non-negative number or "unknown"'* ]]
}

@test "rejects a truthy-string boolean §25 field" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel '{"merged":"yes"}' d1 implementer haiku shallow 100 0 10 0 false pending)"
    [ "$status" -eq 1 ]
    [[ "$output" == *'merged must be a boolean or "unknown"'* ]]
}

@test "rejects an off-vocabulary role" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel '{"role":"teamlead"}' d1 implementer haiku shallow 100 0 10 0 false pending)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"invalid role: teamlead"* ]]
}

@test "rejects an off-vocabulary runtime" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel '{"runtime":"llama.cpp"}' d1 implementer haiku shallow 100 0 10 0 false pending)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"invalid runtime: llama.cpp"* ]]
}

@test "rejects an empty §25 string field" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel '{"model_version":""}' d1 implementer haiku shallow 100 0 10 0 false pending)"
    [ "$status" -eq 1 ]
    [[ "$output" == *'model_version must be a non-empty string'* ]]
}

@test "legacy rows without telemetry pass --validate" {
    printf '%s\n' "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)" > "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --validate
    [ "$status" -eq 0 ]
}

@test "a fully extended row passes --validate" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel "$TEL_FULL" d1 implementer haiku shallow 100 0 10 0 false merged_clean)"
    run bash "$SCRIPT" --ledger "$LEDGER" --validate
    [ "$status" -eq 0 ]
}

@test "--update-outcome preserves §25 telemetry on the appended copy" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec_tel "$TEL_FULL" d1 implementer haiku shallow 100 0 10 0 false pending)"
    run bash "$SCRIPT" --ledger "$LEDGER" --update-outcome d1 merged_clean "ok"
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" --ledger "$LEDGER" --show --json
    [ "$(printf '%s' "$output" | jq 'length')" -eq 1 ]
    [ "$(printf '%s' "$output" | jq -r '.[0].outcome')" = "merged_clean" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].role')" = "implementer" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].decode_tok_s')" = "64.2" ]
}

@test "--show reports unknown for §25 fields on rows written before telemetry existed" {
    printf '%s\n' "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)" > "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --show --json
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.[0].role')" = "unknown" ]
    [ "$(printf '%s' "$output" | jq -r '.[0].reverted')" = "unknown" ]
}

@test "--rebuild keeps the latest record per dispatch_id, one row each" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    run bash "$SCRIPT" --ledger "$LEDGER" --update-outcome d1 merged_clean "ok"
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d2 implementer sonnet medium 100 0 10 0 false reverted)"
    run bash "$SCRIPT" --ledger "$LEDGER" --rebuild
    [ "$status" -eq 0 ]
    [[ "$output" == *"rebuilt 2 records"* ]]
    run grep -c . "$LEDGER"
    [ "$output" = "2" ]
    run bash "$SCRIPT" --ledger "$LEDGER" --show --json
    [ "$(printf '%s' "$output" | jq -r '.[0].outcome')" = "merged_clean" ]
}

@test "--rebuild normalizes legacy rows in place" {
    printf '%s\n' "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)" > "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --rebuild
    [ "$status" -eq 0 ]
    [ "$(jq -r '.role' "$LEDGER")" = "unknown" ]
    [ "$(jq -r '.reverted' "$LEDGER")" = "unknown" ]
    run bash "$SCRIPT" --ledger "$LEDGER" --validate
    [ "$status" -eq 0 ]
}

@test "--rebuild on a missing ledger creates an empty one" {
    run bash "$SCRIPT" --ledger "$LEDGER" --rebuild
    [ "$status" -eq 0 ]
    [[ "$output" == *"rebuilt 0 records"* ]]
    [ -f "$LEDGER" ]
    [ ! -s "$LEDGER" ]
}

@test "--rebuild fails closed on an invalid row and leaves the ledger untouched" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)"
    printf '{"dispatch_id":"bad"}\n' >> "$LEDGER"
    cp "$LEDGER" "$TMP/ledger.before"
    run bash "$SCRIPT" --ledger "$LEDGER" --rebuild
    [ "$status" -eq 1 ]
    [[ "$output" == *"invalid rows"* ]]
    run diff "$TMP/ledger.before" "$LEDGER"
    [ "$status" -eq 0 ]
}
