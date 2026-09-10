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

# ── §25 telemetry: present fields must be typed, absent fields become unknown ──

@test "--append normalizes absent §25 fields to unknown, never 0" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    [ "$status" -eq 0 ]
    row="$(head -n 1 "$LEDGER")"
    [ "$(printf '%s' "$row" | jq -r '.role')" = "unknown" ]
    [ "$(printf '%s' "$row" | jq -r '.decode_tok_s')" = "unknown" ]
    [ "$(printf '%s' "$row" | jq -r '.merged')" = "unknown" ]
    [ "$(printf '%s' "$row" | jq -r '.ttft_ms')" = "unknown" ]
    # Every one of the 21 new fields must be unknown, and none of them 0.
    [ "$(printf '%s' "$row" | jq '[.role,.model_version,.hardware_fingerprint,.runtime,.quantization,
        .context_requested,.context_reserved,.context_used,.concurrency_at_start,
        .queue_depth_at_start,.prompt_tok_s,.decode_tok_s,.aggregate_decode_tok_s,
        .ttft_ms,.retry_index,.previous_model,.review_outcome,.tests_outcome,
        .merged,.reverted,.stack] | map(select(. != "unknown")) | length')" -eq 0 ]
}

@test "--append accepts a record with fully typed §25 fields" {
    full=$(rec d1 implementer haiku shallow 100 0 10 0 false pending | jq -c \
        '. + {role:"implementer",model_version:"qwen3-32b",hardware_fingerprint:"rtx4090",
              runtime:"ollama",quantization:"q4_k_m",context_requested:8192,
              context_reserved:8192,context_used:1000,concurrency_at_start:2,
              queue_depth_at_start:0,prompt_tok_s:120.5,decode_tok_s:45.5,
              aggregate_decode_tok_s:90.2,ttft_ms:120,retry_index:1,
              previous_model:"qwen3-14b",review_outcome:"lgtm",
              tests_outcome:"passed",merged:true,reverted:false}')
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$full"
    [ "$status" -eq 0 ]
    run bash "$SCRIPT" --ledger "$LEDGER" --validate
    [ "$status" -eq 0 ]
    [ "$(head -n 1 "$LEDGER" | jq -r '.decode_tok_s')" = "45.5" ]
    [ "$(head -n 1 "$LEDGER" | jq -r '.merged')" = "true" ]
}

@test "--append rejects a fabricated string metric" {
    bad=$(rec d1 implementer haiku shallow 100 0 10 0 false pending | jq -c '. + {decode_tok_s:"fast"}')
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$bad"
    [ "$status" -eq 1 ]
    [[ "$output" == *"decode_tok_s"* ]]
    [[ "$output" == *"non-negative number"* ]]
}

@test "--append rejects a negative telemetry number" {
    bad=$(rec d1 implementer haiku shallow 100 0 10 0 false pending | jq -c '. + {ttft_ms:-5}')
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$bad"
    [ "$status" -eq 1 ]
    [[ "$output" == *"ttft_ms"* ]]
}

@test "--append rejects a mistyped telemetry boolean" {
    bad=$(rec d1 implementer haiku shallow 100 0 10 0 false pending | jq -c '. + {merged:"yes"}')
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$bad"
    [ "$status" -eq 1 ]
    [[ "$output" == *"merged must be"* ]]
}

@test "--append rejects an empty string where unknown-or-nonempty is required" {
    bad=$(rec d1 implementer haiku shallow 100 0 10 0 false pending | jq -c '. + {runtime:""}')
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$bad"
    [ "$status" -eq 1 ]
    [[ "$output" == *"runtime"* ]]
    [[ "$output" == *"non-empty string"* ]]
}

@test "--validate accepts a legacy ledger row lacking the §25 keys" {
    printf '%s\n' "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)" > "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --validate
    [ "$status" -eq 0 ]
}

@test "--validate rejects a row with a mistyped §25 field" {
    printf '%s\n' "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)" > "$LEDGER"
    bad=$(rec d2 implementer haiku shallow 100 0 10 0 false merged_clean | jq -c '. + {prompt_tok_s:"lots"}')
    printf '%s\n' "$bad" >> "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --validate
    [ "$status" -eq 1 ]
    [[ "$output" == *":2:"* ]]
    [[ "$output" == *"prompt_tok_s"* ]]
}

@test "--update-outcome normalizes the appended record" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    run bash "$SCRIPT" --ledger "$LEDGER" --update-outcome d1 merged_clean "ok"
    [ "$status" -eq 0 ]
    run grep -c . "$LEDGER"
    [ "$output" = "2" ]
    last="$(tail -n 1 "$LEDGER")"
    [ "$(printf '%s' "$last" | jq -r '.role')" = "unknown" ]
    [ "$(printf '%s' "$last" | jq -r '.outcome')" = "merged_clean" ]
}

@test "--rebuild writes a .rebuilt sidecar without touching the live ledger" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)"
    run bash "$SCRIPT" --ledger "$LEDGER" --update-outcome d1 merged_clean "ok"
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(rec d2 lgtm-reviewer sonnet medium 100 0 10 0 false lgtm_first_pass)"
    before="$(cat "$LEDGER")"
    run bash "$SCRIPT" --ledger "$LEDGER" --rebuild
    [ "$status" -eq 0 ]
    [ -f "$LEDGER.rebuilt" ]
    # The live ledger is untouched: byte-identical after the rebuild.
    [ "$(cat "$LEDGER")" = "$before" ]
    # The sidecar keeps exactly one (latest) row per dispatch_id.
    run grep -c . "$LEDGER.rebuilt"
    [ "$output" = "2" ]
    [ "$(jq -s 'length' "$LEDGER.rebuilt")" -eq 2 ]
    [ "$(jq -s '[.[] | select(.dispatch_id=="d1")][0].outcome' "$LEDGER.rebuilt")" = '"merged_clean"' ]
    run bash "$SCRIPT" --ledger "$LEDGER.rebuilt" --validate
    [ "$status" -eq 0 ]
}

@test "--rebuild on a missing ledger fails" {
    run bash "$SCRIPT" --ledger "$TMP/absent.jsonl" --rebuild
    [ "$status" -eq 1 ]
    [[ "$output" == *"no ledger"* ]]
}

@test "--help mentions --rebuild" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"--rebuild"* ]]
}

# ── Pi execution events (issue #3319): the second record type in one file ────
#
# Event rows are WRITTEN by the Rust normalizer
# (autospec_core::aar::normalize_event / to_ledger_lines in
# crates/autospec-core/src/aar/pi_events.rs), whose side is covered by the
# integration test aar_pi_events_ledger.rs. These cases cover the shell side:
# --append and --validate must accept exactly the rows the Rust side emits, and
# the dispatch readers must keep ignoring them.

RUST_EVENTS="${BATS_TEST_DIRNAME}/../crates/autospec-core/src/aar/pi_events.rs"

# evrec <seq> <event> — a normalized event row with full identity and three
# measured metrics (the fixture metrics from the issue's acceptance criteria).
evrec() {
    printf '{"record_type":"event","schema_version":1,"seq":%s,"event":"%s","timestamp":"2026-08-21T10:00:00Z","session_id":"pi-s1","work_item_id":"3319","agent_role":"implementer","harness":"pi","ttft_ms":412,"decode_tok_s":61.5,"cache_hit_tokens":18944}' "$1" "$2"
}

@test "--append accepts a valid event row" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(evrec 1 model_request)"
    [ "$status" -eq 0 ]
    [ "$(jq -r '.event' "$LEDGER")" = "model_request" ]
}

@test "--append leaves an event row out of the dispatch telemetry contract" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(evrec 1 model_request)"
    [ "$status" -eq 0 ]
    # §25 normalization is dispatch-only: padding an event with profile/outcome
    # would make every event look like a dispatch that measured nothing.
    [ "$(jq -r 'has("profile") or has("outcome") or has("cell_ctx")' "$LEDGER")" = "false" ]
}

@test "--validate accepts a ledger holding both record types" {
    printf '%s\n' "$(rec d1 implementer haiku shallow 100 0 10 0 false merged_clean)" > "$LEDGER"
    printf '%s\n' "$(evrec 1 session_start)" >> "$LEDGER"
    printf '%s\n' "$(evrec 2 model_request)" >> "$LEDGER"
    printf '%s\n' "$(evrec 3 finish)" >> "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --validate
    [ "$status" -eq 0 ]
}

@test "--validate rejects an event row missing an identity key" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(evrec 1 model_request | jq -c 'del(.work_item_id)')"
    [ "$status" -eq 1 ]
    [[ "$output" == *"event missing required key: work_item_id"* ]]
}

@test "--validate rejects an event row with an empty agent_role" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(evrec 1 model_request | jq -c '.agent_role=""')"
    [ "$status" -eq 1 ]
    [[ "$output" == *"agent_role must be a non-empty string"* ]]
}

@test "--validate rejects an event row without a timestamp" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(evrec 1 model_request | jq -c 'del(.timestamp)')"
    [ "$status" -eq 1 ]
    [[ "$output" == *"timestamp"* ]]
}

@test "--validate rejects an event row with seq 0" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(evrec 0 model_request | jq -c '.seq=0')"
    [ "$status" -eq 1 ]
    [[ "$output" == *"event seq must be a number >= 1"* ]]
}

@test "--validate rejects an unmapped event kind" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(evrec 1 tool_invocation)"
    [ "$status" -eq 1 ]
    [[ "$output" == *"invalid event: tool_invocation"* ]]
}

@test "--validate rejects a null metric on an event row" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(evrec 1 model_request | jq -c '. + {ttft_ms:null}')"
    [ "$status" -eq 1 ]
    [[ "$output" == *"ttft_ms"* ]]
    [[ "$output" == *"invalid type"* ]]
}

@test "--validate accepts the unknown sentinel on an event metric" {
    # Pi reports nothing for a streamed request's prefill: "unknown", never 0.
    run bash "$SCRIPT" --ledger "$LEDGER" --append \
        "$(evrec 1 model_request | jq -c '. + {prefill_ms:"unknown", success:"unknown"}')"
    [ "$status" -eq 0 ]
}

@test "--validate rejects an unknown record_type" {
    run bash "$SCRIPT" --ledger "$LEDGER" --append "$(evrec 1 model_request | jq -c '.record_type="telemetry"')"
    [ "$status" -eq 1 ]
    [[ "$output" == *"unknown record_type: telemetry"* ]]
}

@test "--stats ignores an event row sharing a dispatch_id" {
    printf '%s\n' "$(rec d1 implementer haiku shallow 1000 800 5000 0 false merged_clean)" > "$LEDGER"
    # The event arrives after the dispatch and carries the same dispatch_id: if
    # it entered the aggregate, its null cell would poison the derived weights.
    printf '%s\n' "$(evrec 1 model_request | jq -c '. + {dispatch_id:"d1"}')" >> "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --stats --json
    [ "$status" -eq 0 ]
    [ "$(jq -r 'length' <<<"$output")" -eq 1 ]
    [ "$(jq -r '.[0].dispatches' <<<"$output")" = "1" ]
    [ "$(jq -r '.[0].input_tokens' <<<"$output")" = "1000" ]
    [ "$(jq -r '.[0].cell_ctx' <<<"$output")" = "64k" ]
}

@test "--show ignores event rows" {
    printf '%s\n' "$(rec d1 implementer haiku shallow 1000 800 5000 0 false merged_clean)" > "$LEDGER"
    printf '%s\n' "$(evrec 1 session_start)" >> "$LEDGER"
    printf '%s\n' "$(evrec 2 model_request)" >> "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --show --json
    [ "$status" -eq 0 ]
    [ "$(jq -r 'length' <<<"$output")" -eq 1 ]
    [ "$(jq -r '.[0].dispatch_id' <<<"$output")" = "d1" ]
}

@test "--update-outcome cannot target an event row" {
    printf '%s\n' "$(evrec 1 model_request | jq -c '. + {dispatch_id:"d1"}')" > "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --update-outcome d1 merged_clean
    [ "$status" -eq 1 ]
    [[ "$output" == *"not found"* ]]
}

@test "--rebuild sidecar holds dispatch rows only" {
    printf '%s\n' "$(rec d1 implementer haiku shallow 100 0 10 0 false pending)" > "$LEDGER"
    printf '%s\n' "$(evrec 1 model_request | jq -c '. + {dispatch_id:"d1"}')" >> "$LEDGER"
    printf '%s\n' "$(rec d2 lgtm-reviewer sonnet medium 100 0 10 0 false lgtm_first_pass)" >> "$LEDGER"
    run bash "$SCRIPT" --ledger "$LEDGER" --rebuild
    [ "$status" -eq 0 ]
    [ "$(jq -s '[.[] | select(.record_type == "event")] | length' "$LEDGER.rebuilt")" -eq 0 ]
    [ "$(jq -s 'length' "$LEDGER.rebuilt")" -eq 2 ]
}

@test "the shell event vocabulary mirrors the Rust canonical kinds" {
    shell_kinds="$(sed -n 's/^ALLOWED_EVENT_KINDS="\(.*\)"$/\1/p' "$SCRIPT" | tr ' ' '\n' | sort)"
    # EventKind is serialized with rename_all = "snake_case", so the Rust
    # variant names are the wire names: SessionStart -> session_start.
    rust_kinds="$(sed -n '/^pub enum EventKind/,/^}/p' "$RUST_EVENTS" \
        | grep -oE '^    [A-Z][A-Za-z]+,$' | tr -d ' ,' \
        | sed -E 's/([a-z0-9])([A-Z])/\1_\2/' | tr 'A-Z' 'a-z' | sort)"
    [ -n "$rust_kinds" ]
    [ "$shell_kinds" = "$rust_kinds" ]
}
