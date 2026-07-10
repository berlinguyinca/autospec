#!/usr/bin/env bats
# Run black-box event recording, explanation, and replay.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-run-events.sh"
    TMP="$(mktemp -d -t run-events.XXXXXX)"
    EVENTS="$TMP/run.jsonl"
}

teardown() {
    rm -rf "$TMP"
}

@test "record appends structured run events" {
    run bash "$SCRIPT" record \
        --events "$EVENTS" \
        --repo berlinguyinca/autospec \
        --run-id run-1 \
        --event waterfall \
        --decision run \
        --reason "value queue selected candidate black-box" \
        --issue 1702

    [ "$status" -eq 0 ]
    [ -f "$EVENTS" ]
    [ "$(jq -r '.repo' "$EVENTS")" = "berlinguyinca/autospec" ]
    [ "$(jq -r '.decision' "$EVENTS")" = "run" ]
    [ "$(jq -r '.issue' "$EVENTS")" = "1702" ]
}

@test "explain reports the final decision and why" {
    cat > "$EVENTS" <<'JSONL'
{"ts":"2026-07-10T00:00:00Z","repo":"berlinguyinca/autospec","run_id":"run-1","event":"waterfall","decision":"run","reason":"value queue selected candidate readiness","issue":1703,"pr":null}
{"ts":"2026-07-10T00:01:00Z","repo":"berlinguyinca/autospec","run_id":"run-1","event":"premerge","decision":"blocked","reason":"main-health pending","issue":1703,"pr":null}
JSONL

    run bash "$SCRIPT" explain --events "$EVENTS"

    [ "$status" -eq 0 ]
    [[ "$output" == *"Final decision: blocked"* ]]
    [[ "$output" == *"main-health pending"* ]]
    [[ "$output" == *"#1703"* ]]
}

@test "replay emits deterministic final decision json" {
    cat > "$EVENTS" <<'JSONL'
{"ts":"2026-07-10T00:00:00Z","repo":"berlinguyinca/autospec","run_id":"run-1","event":"waterfall","decision":"run","reason":"selected","issue":1703}
{"ts":"2026-07-10T00:01:00Z","repo":"berlinguyinca/autospec","run_id":"run-1","event":"merge","decision":"merged","reason":"required checks passed","issue":1703,"pr":99}
JSONL

    run bash "$SCRIPT" replay --events "$EVENTS"

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.final_decision')" = "merged" ]
    [ "$(printf '%s' "$output" | jq -r '.issue')" = "1703" ]
    [ "$(printf '%s' "$output" | jq -r '.pr')" = "99" ]
}

