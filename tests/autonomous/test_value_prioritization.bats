#!/usr/bin/env bats
# Value-gated prioritization, anti-thrash, and idle-floor coverage for issue #1542.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    PRIORITIZE="$REPO_ROOT/scripts/autonomous-prioritize.sh"
    WATERFALL="$REPO_ROOT/scripts/autonomous-waterfall.sh"
    TMP="$(mktemp -d -t value-priority.XXXXXX)"
}

teardown() {
    rm -rf "$TMP"
}

write_jsonl() {
    local file="$1"
    shift
    : > "$file"
    for row in "$@"; do
        printf '%s\n' "$row" >> "$file"
    done
}

@test "scorer ranks candidates from all workstreams into one queue" {
    candidates="$TMP/candidates.jsonl"
    write_jsonl "$candidates" \
      '{"id":"perf-1","workstream":"performance","severity":3,"value":3,"confidence":0.8,"reversibility":1,"effort":1,"blast_radius":1,"files":["crates/perf.rs"]}' \
      '{"id":"sec-1","workstream":"security","severity":4,"value":5,"confidence":0.9,"reversibility":1,"effort":2,"blast_radius":1,"files":["scripts/auth.sh"]}' \
      '{"id":"ux-1","workstream":"ux-ui","severity":1,"value":2,"confidence":0.7,"reversibility":1,"effort":1,"blast_radius":1,"files":["docs/ui.md"]}'

    run bash "$PRIORITIZE" score --candidates "$candidates" --value-floor 1
    [ "$status" -eq 0 ]
    ids="$(printf '%s' "$output" | jq -r '[.ranked[].id] | join(",")')"
    [ "$ids" = "sec-1,perf-1,ux-1" ]
    [ "$(printf '%s' "$output" | jq -r '.decision')" = "run" ]
}

@test "below value floor emits idle-rescan rather than park/churn" {
    candidates="$TMP/low.jsonl"
    write_jsonl "$candidates" \
      '{"id":"cosmetic-1","workstream":"polish","severity":1,"value":1,"confidence":0.4,"reversibility":1,"effort":8,"blast_radius":2,"files":["README.md"]}'

    run bash "$PRIORITIZE" score --candidates "$candidates" --value-floor 1
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.decision')" = "idle" ]

    run bash "$WATERFALL" --candidate-file "$candidates" --value-floor 1 --backlog-count 0 --open-issue-count 0 --dry-cycles 99 --tier15-dry-cycles 99 --tier2-dry-cycles 99 --tier3-dry-cycles 99 --tier4-dry-cycles 99
    [ "$status" -eq 0 ]
    [[ "$output" == *'"action":"run-backlog"'* ]]
    [[ "$output" == *'idle-rescan:'* ]]
    [[ "$output" != *'"action":"park"'* ]]
}

@test "recently touched decay prevents A to B to A thrash" {
    candidates="$TMP/thrash.jsonl"
    recent="$TMP/recent.jsonl"
    write_jsonl "$candidates" \
      '{"id":"A","workstream":"technical-debt","severity":5,"value":5,"confidence":1,"reversibility":1,"effort":5,"blast_radius":1,"files":["src/a.py"]}' \
      '{"id":"B","workstream":"security","severity":3,"value":3,"confidence":1,"reversibility":1,"effort":2,"blast_radius":1,"files":["src/b.py"]}'
    write_jsonl "$recent" '{"path":"src/a.py","timestamp":"2026-07-07T00:00:00Z"}'

    run bash "$PRIORITIZE" score --candidates "$candidates" --recent-touches "$recent" --recent-decay 0.2 --value-floor 1
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.top.id')" = "B" ]
    [ "$(printf '%s' "$output" | jq -r '.ranked[] | select(.id=="A") | .decay_applied')" = "true" ]
}

@test "fenced or high blast-radius candidates route to human gate" {
    candidates="$TMP/high-risk.jsonl"
    write_jsonl "$candidates" \
      '{"id":"schema-1","workstream":"security","severity":5,"value":5,"confidence":1,"reversibility":1,"effort":1,"blast_radius":5,"fenced":true,"files":["migrations/001.sql"]}'

    run bash "$PRIORITIZE" score --candidates "$candidates" --value-floor 1 --human-gate-blast-radius 4
    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.decision')" = "human_gate" ]
    [ "$(printf '%s' "$output" | jq -r '.top.route')" = "human_gate" ]

    run bash "$WATERFALL" --candidate-file "$candidates" --value-floor 1 --human-gate-blast-radius 4 --backlog-count 0
    [ "$status" -eq 0 ]
    [[ "$output" == *'"action":"control"'* ]]
    [[ "$output" == *'human-gate:'* ]]
}

@test "design doc cites SAFe WSJF and Cost of Delay" {
    grep -q 'SAFe WSJF' "$REPO_ROOT/docs/specs/2026-06-25-autospec-autonomous-design.md"
    grep -q 'Cost of Delay' "$REPO_ROOT/docs/specs/2026-06-25-autospec-autonomous-design.md"
}
