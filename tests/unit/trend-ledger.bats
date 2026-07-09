#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
TL="$REPO_ROOT/skills/autospec-shared/scripts/trend-ledger.sh"

setup() { TMP="$(mktemp -d)"; export AUTOSPEC_TREND_LEDGER="$TMP/ledger.jsonl"; }
teardown() { rm -rf "$TMP"; unset AUTOSPEC_TREND_LEDGER; }

sig() {
  # sig <source> <norm_key> <recurrence>
  echo "{\"source\":\"$1\",\"kind\":\"complaint\",\"summary\":\"s\",\"norm_key\":\"$2\",\"evidence_ref\":\"https://example.com\",\"first_seen\":\"2026-07-08T00:00:00Z\",\"recurrence\":$3,\"sanitized_excerpt\":\"e\",\"ts\":\"2026-07-08T00:00:00Z\"}"
}

@test "script exists and is bash -n clean" {
  [ -f "$TL" ]; run bash -n "$TL"; [ "$status" -eq 0 ]
}

@test "append then show returns the line" {
  run bash "$TL" --append "$(sig internet-forums foo-bar 1)"
  [ "$status" -eq 0 ]
  run bash "$TL" --show
  [ "$status" -eq 0 ]
  [[ "$output" == *"foo-bar"* ]]
}

@test "append rejects a record that fails validate-trend-signal.sh" {
  run bash "$TL" --append '{"source":"internet-forums"}'
  [ "$status" -ne 0 ]
  [ ! -f "$AUTOSPEC_TREND_LEDGER" ]
}

@test "show returns latest entry per norm_key" {
  bash "$TL" --append "$(sig internet-forums dup-key 1)"
  bash "$TL" --append "$(sig internet-forums dup-key 2)"
  run bash "$TL" --show --json
  [ "$status" -eq 0 ]
  # only one logical row for dup-key, and it is the latest (recurrence 2)
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="dup-key")] | length')" -eq 1 ]
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="dup-key")][0].recurrence')" -eq 2 ]
}

@test "bump-recurrence increments recurrence for the matching norm_key" {
  bash "$TL" --append "$(sig internet-forums bump-me 1)"
  run bash "$TL" --bump-recurrence bump-me
  [ "$status" -eq 0 ]
  run bash "$TL" --show --json
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="bump-me")][0].recurrence')" -eq 2 ]
}

@test "bump-recurrence matches norm_key literally, not as a regex" {
  bash "$TL" --append "$(sig internet-forums 'a.b' 1)"
  run bash "$TL" --bump-recurrence 'aXb'
  [ "$status" -ne 0 ]
  run bash "$TL" --show --json
  [ "$(echo "$output" | jq '[.[] | select(.norm_key=="a.b")][0].recurrence')" -eq 1 ]
}

@test "show --source filters by source" {
  bash "$TL" --append "$(sig internet-forums key-a 1)"
  bash "$TL" --append "$(sig userspace-usage key-b 1)"
  run bash "$TL" --show --json --source userspace-usage
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq 'length')" -eq 1 ]
  [ "$(echo "$output" | jq -r '.[0].source')" = "userspace-usage" ]
}

@test "stats counts per source on latest rows" {
  bash "$TL" --append "$(sig internet-forums key-c 1)"
  bash "$TL" --append "$(sig internet-forums key-c 2)"
  bash "$TL" --append "$(sig userspace-usage key-d 1)"
  run bash "$TL" --stats
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '.["internet-forums"]')" -eq 1 ]
  [ "$(echo "$output" | jq '.["userspace-usage"]')" -eq 1 ]
}

@test "validate passes on a well-formed ledger" {
  bash "$TL" --append "$(sig internet-forums key-e 1)"
  run bash "$TL" --validate
  [ "$status" -eq 0 ]
}

@test "validate fails closed on a malformed ledger line" {
  echo '{"source":"internet-forums"}' > "$AUTOSPEC_TREND_LEDGER"
  run bash "$TL" --validate
  [ "$status" -ne 0 ]
}

@test "--show on empty/absent ledger is clean" {
  run bash "$TL" --show --json
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq 'length')" -eq 0 ]
}

@test "--stats on empty ledger emits valid JSON object {}" {
  run bash "$TL" --stats
  [ "$status" -eq 0 ]
  echo "$output" | jq -e 'type == "object"'
  [ "$output" = "{}" ]
}

@test "--validate on absent ledger is clean" {
  run bash "$TL" --validate
  [ "$status" -eq 0 ]
}
