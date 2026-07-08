#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() { TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER; }

line() { echo "{\"round\":1,\"source\":\"$1\",\"title\":\"t\",\"norm_title\":\"t\",\"channel\":\"seo\",\"kind\":\"$2\",\"issue\":$3,\"outcome\":\"$4\",\"reason\":\"\",\"ts\":\"2026-07-08T00:00:00Z\"}"; }

@test "script exists and is bash -n clean" {
  [ -f "$LG" ]; run bash -n "$LG"; [ "$status" -eq 0 ]
}

@test "append then show returns the line" {
  bash "$LG" --append "$(line keyword-gap artifact 7 pending)"
  run bash "$LG" --show --json
  [ "$status" -eq 0 ]
  [[ "$output" == *"keyword-gap"* ]]
}

@test "update-outcome appends and show reflects latest" {
  bash "$LG" --append "$(line keyword-gap artifact 7 pending)"
  bash "$LG" --update-outcome 7 merged_clean "done"
  # two physical lines, one logical (latest) state
  [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 2 ]
  run bash "$LG" --show --json
  [[ "$output" == *"merged_clean"* ]]
  [[ "$output" != *"pending"* ]]
}

@test "stats: refuted does not count toward filed" {
  bash "$LG" --append "$(line community artifact 8 pending)"
  bash "$LG" --append "$(line community outbound 0 refuted)"
  run bash "$LG" --stats --json
  [ "$status" -eq 0 ]
  # community filed == 1 (issue 8), refuted == 1
  echo "$output" | jq -e '.community.filed == 1'
  echo "$output" | jq -e '.community.refuted == 1'
}

@test "stats: issue:0 published row does not inflate published count" {
  bash "$LG" --append "$(line community artifact 9 pending)"
  bash "$LG" --append "$(line community outbound 0 published)"
  run bash "$LG" --stats --json
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.community.published == 0'
  echo "$output" | jq -e '.community.filed == 1'
}

@test "validate rejects a line missing a required key" {
  echo '{"round":1,"source":"x"}' > "$GROWTH_LEDGER"
  run bash "$LG" --validate
  [ "$status" -ne 0 ]
}

@test "--stats on empty ledger emits valid JSON object {}" {
  # GROWTH_LEDGER env var is set to a temp path that doesn't exist yet
  run bash "$LG" --stats --json
  [ "$status" -eq 0 ]
  # Output must be valid JSON and be an object type
  echo "$output" | jq -e 'type == "object"'
  # Output must be exactly {}
  [ "$output" = "{}" ]
}
